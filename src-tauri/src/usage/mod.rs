pub mod anthropic_oauth;
pub mod claude_code;

use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the background thread re-scans the transcript files.
const POLL_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub enum UsageStatus {
    /// First scan hasn't completed yet.
    Loading,
    /// Official percentage of the 5h plan limit used, straight from the same endpoint the
    /// `claude` CLI's own `/usage` reads — not an estimate.
    ActiveOfficial {
        percent: f64,
        resets_at: Option<DateTime<Utc>>,
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

fn run_loop(snapshot: Arc<Mutex<UsageSnapshot>>, projects_dir: Option<PathBuf>) {
    let Some(dir) = projects_dir else {
        publish(
            &snapshot,
            UsageStatus::Unavailable("não foi possível localizar a pasta do usuário".into()),
        );
        return;
    };

    loop {
        let status = match anthropic_oauth::fetch_official_usage() {
            Ok(window) => UsageStatus::ActiveOfficial {
                percent: window.percent,
                resets_at: window.resets_at,
            },
            Err(err) => {
                log::debug!("uso oficial indisponível, caindo pra estimativa local: {err}");
                fallback_status(&dir, err)
            }
        };

        publish(&snapshot, status);
        std::thread::sleep(POLL_INTERVAL);
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
