pub mod anthropic_oauth;
pub mod claude_code;

use anthropic_oauth::{FetchError, OfficialUsage};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the background thread checks for fresh data — matches the 60s cache TTL
/// `ai-usagebar` uses for this same endpoint, since polling it much faster than that is
/// what was drawing 429s in the first place.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// After a 429, how long to go without even trying the official endpoint again — mirrors
/// `ai-usagebar`'s own rate-limit backoff for this endpoint.
const RATE_LIMIT_BACKOFF: chrono::Duration = chrono::Duration::minutes(5);

/// How long a cached successful reading is still worth showing over the token-count
/// fallback when a fetch fails. Deliberately much shorter than `ai-usagebar`'s 7-day
/// staleness ceiling (that's a display widget; showing week-old numbers here as if current
/// felt actively misleading) — 24h means a truly broken token still surfaces the honest
/// fallback well within the same day.
const MAX_CACHE_AGE: chrono::Duration = chrono::Duration::hours(24);

#[derive(Debug, Clone)]
pub enum UsageStatus {
    /// First scan hasn't completed yet.
    Loading,
    /// Official percentage of the 5h and 7d plan limits used, straight from the same
    /// endpoint the `claude` CLI's own `/usage` reads — not an estimate. `weekly_*` is
    /// `None` if the endpoint's response simply didn't include a `seven_day` window.
    ActiveOfficial {
        percent: f64,
        resets_at: Option<DateTime<Utc>>,
        weekly_percent: Option<f64>,
        weekly_resets_at: Option<DateTime<Utc>>,
    },
    /// There's an active 5h usage window with `tokens` consumed so far. Fallback used when
    /// the official source (`anthropic_oauth`) isn't available for any reason — `official_error`
    /// carries that reason so it's visible somewhere (the app has no console in release
    /// builds, so a log line alone is not enough to ever debug this from the field).
    Active {
        tokens: u64,
        started_at: DateTime<Utc>,
        resets_at: DateTime<Utc>,
        official_error: String,
    },
    /// No active window right now (no recent activity, or the last window expired).
    Idle,
    /// Couldn't read local Claude Code data at all (not installed, no sessions yet, ...).
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct UsageSnapshot {
    pub status: UsageStatus,
    pub last_updated: DateTime<Utc>,
}

/// Runs the periodic scan of `~/.claude/projects` on a background thread and exposes the
/// latest result through a shared snapshot the UI reads every frame (no locking needed on
/// the UI side beyond a quick mutex lock — the background thread never blocks on the UI).
pub struct UsageWatcher {
    snapshot: Arc<Mutex<UsageSnapshot>>,
}

impl UsageWatcher {
    pub fn spawn(projects_dir: Option<PathBuf>) -> Self {
        let snapshot = Arc::new(Mutex::new(UsageSnapshot {
            status: UsageStatus::Loading,
            last_updated: Utc::now(),
        }));

        let bg_snapshot = Arc::clone(&snapshot);
        std::thread::Builder::new()
            .name("usage-watcher".into())
            .spawn(move || run_loop(bg_snapshot, projects_dir))
            .expect("failed to spawn usage-watcher thread");

        Self { snapshot }
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.snapshot
            .lock()
            .expect("usage snapshot mutex poisoned")
            .clone()
    }
}

/// The last successful official reading, kept around so a transient failure (rate limit,
/// a dropped connection, ...) shows slightly-stale real numbers instead of immediately
/// flipping to the token-count estimate.
struct OfficialCache {
    usage: OfficialUsage,
    fetched_at: DateTime<Utc>,
}

fn run_loop(snapshot: Arc<Mutex<UsageSnapshot>>, projects_dir: Option<PathBuf>) {
    let Some(dir) = projects_dir else {
        publish(
            &snapshot,
            UsageStatus::Unavailable("não foi possível localizar a pasta do usuário".into()),
        );
        return;
    };

    let mut cache: Option<OfficialCache> = None;
    let mut rate_limited_until: Option<DateTime<Utc>> = None;

    loop {
        let now = Utc::now();
        let skip_fetch = rate_limited_until.is_some_and(|until| now < until);

        let status = if skip_fetch {
            cache_or_fallback(
                &cache,
                &dir,
                "aguardando o limite de requisições (429) liberar".into(),
            )
        } else {
            match anthropic_oauth::fetch_official_usage() {
                Ok(usage) => {
                    rate_limited_until = None;
                    let status = to_active_official(&usage);
                    cache = Some(OfficialCache {
                        usage,
                        fetched_at: now,
                    });
                    status
                }
                Err(FetchError::RateLimited) => {
                    rate_limited_until = Some(now + RATE_LIMIT_BACKOFF);
                    log::debug!(
                        "uso oficial retornou 429, pausando por {}min",
                        RATE_LIMIT_BACKOFF.num_minutes()
                    );
                    cache_or_fallback(&cache, &dir, FetchError::RateLimited.to_string())
                }
                Err(FetchError::Other(err)) => {
                    log::debug!("uso oficial indisponível, caindo pra estimativa local: {err}");
                    cache_or_fallback(&cache, &dir, err)
                }
            }
        };

        publish(&snapshot, status);
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn to_active_official(usage: &OfficialUsage) -> UsageStatus {
    UsageStatus::ActiveOfficial {
        percent: usage.five_hour.percent,
        resets_at: usage.five_hour.resets_at,
        weekly_percent: usage.seven_day.map(|w| w.percent),
        weekly_resets_at: usage.seven_day.and_then(|w| w.resets_at),
    }
}

/// Serves the cached official reading if it's still fresh enough, otherwise falls back to
/// the JSONL-derived estimate — the point of the cache is exactly to absorb a run of
/// failures (like the 429 backoff window) without the UI flapping between the real
/// percentage and the estimate every time a fetch fails.
fn cache_or_fallback(
    cache: &Option<OfficialCache>,
    dir: &std::path::Path,
    official_error: String,
) -> UsageStatus {
    match cache {
        Some(entry) if Utc::now() - entry.fetched_at < MAX_CACHE_AGE => {
            to_active_official(&entry.usage)
        }
        _ => fallback_status(dir, official_error),
    }
}

/// The JSONL-derived estimate used whenever the official source (`anthropic_oauth`) isn't
/// available — `official_error` is why it wasn't, carried through so `Active` can surface it
/// (see the module docs on `anthropic_oauth` for why that's expected to happen at least some
/// of the time: no refresh-token handling, network issues, etc).
fn fallback_status(dir: &std::path::Path, official_error: String) -> UsageStatus {
    if !dir.is_dir() {
        return UsageStatus::Unavailable(
            "Claude Code ainda não gerou dados locais nesta máquina".into(),
        );
    }

    let events = claude_code::scan_token_events(dir);
    match claude_code::compute_current_block(&events, Utc::now()) {
        Some(block) => UsageStatus::Active {
            tokens: block.tokens,
            started_at: block.started_at,
            resets_at: block.resets_at(),
            official_error,
        },
        None => UsageStatus::Idle,
    }
}

fn publish(snapshot: &Arc<Mutex<UsageSnapshot>>, status: UsageStatus) {
    let mut guard = snapshot.lock().expect("usage snapshot mutex poisoned");
    *guard = UsageSnapshot {
        status,
        last_updated: Utc::now(),
    };
}
