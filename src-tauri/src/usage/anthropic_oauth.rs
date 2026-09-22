//! Fetches the *official* Claude Code plan-usage percentage by reading the OAuth
//! credentials the `claude` CLI already maintains locally and calling the same endpoint
//! `claude`'s own `/usage` command uses. This endpoint isn't publicly documented by
//! Anthropic, but the request shape here (path, headers, response fields) is confirmed
//! against two independent open-source implementations that read it the same way:
//! [codenotch](https://github.com/vinzdg/codenotch) and
//! [ai-usagebar](https://github.com/akitaonrails/ai-usagebar/blob/main/src/anthropic/fetch.rs).
//!
//! Read-only and best-effort: never logs the token, never writes back to the credentials
//! file, and doesn't attempt OAuth refresh — if the token is expired, [`fetch_official_usage`]
//! just errors, and callers should fall back to the JSONL-derived estimate in
//! `claude_code.rs` instead (this module never had network access validated against a real
//! account in development, so treat a fallback path as load-bearing, not optional).

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;

const USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.1.183";

pub fn credentials_path() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".claude").join(".credentials.json"))
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OauthCredentials,
}

#[derive(Debug, Deserialize)]
pub struct OauthCredentials {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "expiresAt")]
    pub expires_at_ms: i64,
}

impl OauthCredentials {
    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match DateTime::<Utc>::from_timestamp_millis(self.expires_at_ms) {
            Some(expires_at) => now >= expires_at,
            // Can't make sense of the expiry — safer to treat it as unusable than to send
            // a token we can't confirm is still valid.
            None => true,
        }
    }
}

/// Reads `~/.claude/.credentials.json` (`%USERPROFILE%\.claude\.credentials.json` on
/// Windows) — the same file the `claude` CLI itself writes and refreshes. Returns `None`
/// on any failure (missing file, unreadable, unexpected shape); this is a best-effort
/// local read, not something worth surfacing a detailed error for.
pub fn read_credentials() -> Option<OauthCredentials> {
    let path = credentials_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    let parsed: CredentialsFile = serde_json::from_str(&contents).ok()?;
    Some(parsed.claude_ai_oauth)
}

#[derive(Debug, Deserialize, Default)]
struct UsageResponse {
    #[serde(default)]
    five_hour: Option<Window>,
}

#[derive(Debug, Deserialize)]
struct Window {
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

pub struct FiveHourWindow {
    pub percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

fn parse_usage_response(body: &str) -> Result<FiveHourWindow, String> {
    let parsed: UsageResponse =
        serde_json::from_str(body).map_err(|e| format!("resposta inesperada: {e}"))?;
    let window = parsed.five_hour.ok_or("resposta sem o campo five_hour")?;
    let resets_at = window
        .resets_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Ok(FiveHourWindow {
        percent: window.utilization.clamp(0.0, 100.0),
        resets_at,
    })
}

#[cfg(windows)]
pub fn fetch_official_usage() -> Result<FiveHourWindow, String> {
    let creds = read_credentials().ok_or("credenciais do Claude Code não encontradas")?;
    if creds.is_expired(Utc::now()) {
        return Err("token OAuth expirado".to_string());
    }

    let response = ureq::get(USAGE_ENDPOINT)
        .set("Authorization", &format!("Bearer {}", creds.access_token))
        .set("anthropic-beta", ANTHROPIC_BETA)
        .set("User-Agent", USER_AGENT)
        .set("Content-Type", "application/json")
        .call()
        .map_err(|e| format!("falha na requisição: {e}"))?;

    let body = response
        .into_string()
        .map_err(|e| format!("falha ao ler resposta: {e}"))?;
    parse_usage_response(&body)
}

#[cfg(not(windows))]
pub fn fetch_official_usage() -> Result<FiveHourWindow, String> {
    Err("uso oficial só é buscado no Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_token_is_detected() {
        let creds = OauthCredentials {
            access_token: "x".into(),
            expires_at_ms: 1000,
        };
        let now = DateTime::<Utc>::from_timestamp_millis(2000).unwrap();
        assert!(creds.is_expired(now));
    }

    #[test]
    fn valid_token_is_not_expired() {
        let creds = OauthCredentials {
            access_token: "x".into(),
            expires_at_ms: 5000,
        };
        let now = DateTime::<Utc>::from_timestamp_millis(2000).unwrap();
        assert!(!creds.is_expired(now));
    }

    #[test]
    fn malformed_expiry_is_treated_as_expired() {
        let creds = OauthCredentials {
            access_token: "x".into(),
            expires_at_ms: i64::MAX,
        };
        assert!(creds.is_expired(Utc::now()));
    }

    #[test]
    fn parses_five_hour_utilization_and_reset() {
        let body = r#"{"five_hour":{"utilization":42,"resets_at":"2025-01-01T12:00:00Z"},"seven_day":{"utilization":10}}"#;
        let window = parse_usage_response(body).unwrap();
        assert_eq!(window.percent, 42.0);
        assert!(window.resets_at.is_some());
    }

    #[test]
    fn missing_five_hour_field_is_an_error() {
        let body = r#"{"seven_day":{"utilization":10}}"#;
        assert!(parse_usage_response(body).is_err());
    }

    #[test]
    fn percent_is_clamped_to_valid_range() {
        let body = r#"{"five_hour":{"utilization":142}}"#;
        let window = parse_usage_response(body).unwrap();
        assert_eq!(window.percent, 100.0);
    }

    #[test]
    fn missing_resets_at_still_parses() {
        let body = r#"{"five_hour":{"utilization":7}}"#;
        let window = parse_usage_response(body).unwrap();
        assert_eq!(window.percent, 7.0);
        assert!(window.resets_at.is_none());
    }
}
