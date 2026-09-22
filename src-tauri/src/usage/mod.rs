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
    /// There's an active 5h usage window with `tokens` consumed so far.
    Active {
        tokens: u64,
        started_at: DateTime<Utc>,
        resets_at: DateTime<Utc>,
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
        let status = if dir.is_dir() {
            let events = claude_code::scan_token_events(&dir);
            match claude_code::compute_current_block(&events, Utc::now()) {
                Some(block) => UsageStatus::Active {
                    tokens: block.tokens,
                    started_at: block.started_at,
                    resets_at: block.resets_at(),
                },
                None => UsageStatus::Idle,
            }
        } else {
            UsageStatus::Unavailable(
                "Claude Code ainda não gerou dados locais nesta máquina".into(),
            )
        };

        publish(&snapshot, status);
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn publish(snapshot: &Arc<Mutex<UsageSnapshot>>, status: UsageStatus) {
    let mut guard = snapshot.lock().expect("usage snapshot mutex poisoned");
    *guard = UsageSnapshot {
        status,
        last_updated: Utc::now(),
    };
}
