//! Derives a Claude Code usage indicator from the local session transcripts Claude Code
//! writes to `~/.claude/projects/**/*.jsonl`. There is no official quota API, so — like
//! codenotch labels its non-official sources — these numbers are *derived* from local logs,
//! not an authoritative usage/limit figure from Anthropic.

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Claude Code (and the underlying Claude subscription plans) track usage in rolling
/// 5-hour windows that start on the first message after the previous window expired.
pub const USAGE_WINDOW: Duration = Duration::hours(5);

pub fn default_projects_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".claude").join("projects"))
}

#[derive(Debug, Clone, Copy)]
pub struct TokenEvent {
    pub at: DateTime<Utc>,
    pub tokens: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct UsageBlock {
    pub started_at: DateTime<Utc>,
    pub tokens: u64,
}

impl UsageBlock {
    pub fn resets_at(&self) -> DateTime<Utc> {
        self.started_at + USAGE_WINDOW
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptLine {
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    message: Option<MessagePayload>,
}

#[derive(Debug, Deserialize)]
struct MessagePayload {
    #[serde(default)]
    usage: Option<UsagePayload>,
}

#[derive(Debug, Deserialize, Default)]
struct UsagePayload {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

impl UsagePayload {
    fn total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
    }
}

/// Parses one `.jsonl` transcript's contents into token-usage events, skipping any line
/// that isn't a well-formed assistant message with a usage block (tool calls, user
/// messages, and malformed lines are silently ignored — this is best-effort telemetry,
/// not a source of truth).
pub fn parse_transcript(contents: &str) -> Vec<TokenEvent> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parsed: TranscriptLine = serde_json::from_str(line).ok()?;
            let at = parsed.timestamp?;
            let usage = parsed.message?.usage?;
            let tokens = usage.total();
            (tokens > 0).then_some(TokenEvent { at, tokens })
        })
        .collect()
}

/// Recursively scans every `.jsonl` file under `projects_dir` and collects token events
/// from all of them. Missing/unreadable files and directories are skipped rather than
/// treated as a hard error, so a partially-broken install still yields the data it can.
pub fn scan_token_events(projects_dir: &Path) -> Vec<TokenEvent> {
    if !projects_dir.is_dir() {
        return Vec::new();
    }
    walkdir::WalkDir::new(projects_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .flat_map(|contents| parse_transcript(&contents))
        .collect()
}

/// Reconstructs the sequence of non-overlapping 5-hour usage blocks from `events` and
/// returns the currently active one (relative to `now`), or `None` if the most recent
/// block has already expired (i.e. the next message would start a fresh window).
pub fn compute_current_block(events: &[TokenEvent], now: DateTime<Utc>) -> Option<UsageBlock> {
    let mut sorted: Vec<&TokenEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.at);

    let mut block_start: Option<DateTime<Utc>> = None;
    let mut block_tokens: u64 = 0;

    for event in sorted {
        match block_start {
            Some(start) if event.at < start + USAGE_WINDOW => {
                block_tokens += event.tokens;
            }
            _ => {
                block_start = Some(event.at);
                block_tokens = event.tokens;
            }
        }
    }

    let started_at = block_start?;
    if now >= started_at + USAGE_WINDOW {
        return None;
    }
    Some(UsageBlock {
        started_at,
        tokens: block_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn parses_assistant_usage_lines_and_skips_the_rest() {
        let contents = [
            r#"{"type":"user","timestamp":"2025-01-01T00:00:00Z","message":{"role":"user"}}"#,
            r#"{"type":"assistant","timestamp":"2025-01-01T00:00:05Z","message":{"role":"assistant","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
            "not even json",
            r#"{"type":"assistant","timestamp":"2025-01-01T00:01:00Z","message":{"role":"assistant","usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":2,"cache_read_input_tokens":3}}}"#,
        ]
        .join("\n");

        let events = parse_transcript(&contents);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].tokens, 15);
        assert_eq!(events[1].tokens, 7);
    }

    #[test]
    fn single_event_starts_an_active_block() {
        let events = vec![TokenEvent {
            at: ts("2025-01-01T00:00:00Z"),
            tokens: 100,
        }];
        let now = ts("2025-01-01T01:00:00Z");
        let block = compute_current_block(&events, now).unwrap();
        assert_eq!(block.tokens, 100);
        assert_eq!(block.started_at, ts("2025-01-01T00:00:00Z"));
    }

    #[test]
    fn events_within_five_hours_accumulate_into_one_block() {
        let events = vec![
            TokenEvent {
                at: ts("2025-01-01T00:00:00Z"),
                tokens: 100,
            },
            TokenEvent {
                at: ts("2025-01-01T02:00:00Z"),
                tokens: 50,
            },
            TokenEvent {
                at: ts("2025-01-01T04:59:00Z"),
                tokens: 25,
            },
        ];
        let now = ts("2025-01-01T04:59:30Z");
        let block = compute_current_block(&events, now).unwrap();
        assert_eq!(block.tokens, 175);
        assert_eq!(block.started_at, ts("2025-01-01T00:00:00Z"));
    }

    #[test]
    fn gap_past_five_hours_starts_a_new_block() {
        let events = vec![
            TokenEvent {
                at: ts("2025-01-01T00:00:00Z"),
                tokens: 100,
            },
            TokenEvent {
                at: ts("2025-01-01T08:00:00Z"),
                tokens: 50,
            },
        ];
        let now = ts("2025-01-01T08:30:00Z");
        let block = compute_current_block(&events, now).unwrap();
        assert_eq!(block.tokens, 50);
        assert_eq!(block.started_at, ts("2025-01-01T08:00:00Z"));
    }

    #[test]
    fn expired_block_yields_no_active_window() {
        let events = vec![TokenEvent {
            at: ts("2025-01-01T00:00:00Z"),
            tokens: 100,
        }];
        let now = ts("2025-01-01T06:00:00Z");
        assert!(compute_current_block(&events, now).is_none());
    }

    #[test]
    fn no_events_yields_no_active_window() {
        assert!(compute_current_block(&[], Utc::now()).is_none());
    }
}
