//! Fetches the *official* Claude Code plan-usage percentages (5h and 7d windows) by reading
//! the OAuth credentials the `claude` CLI already maintains locally and calling the same
//! endpoint `claude`'s own `/usage` command uses. This endpoint isn't publicly documented by
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
    five_hour: Option<RawWindow>,
    #[serde(default)]
    seven_day: Option<RawWindow>,
}

#[derive(Debug, Deserialize)]
struct RawWindow {
    utilization: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct PercentWindow {
    pub percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Both usage windows the endpoint reports. `seven_day` is `None` if the response simply
/// didn't include it (older API versions, a plan without weekly limits, etc) — that's not
/// treated as a failure, since `five_hour` is the information the rest of the app actually
/// depends on.
#[derive(Debug, Clone, Copy)]
pub struct OfficialUsage {
    pub five_hour: PercentWindow,
    pub seven_day: Option<PercentWindow>,
}

fn to_percent_window(raw: RawWindow) -> PercentWindow {
    let resets_at = raw
        .resets_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    PercentWindow {
        percent: raw.utilization.clamp(0.0, 100.0),
        resets_at,
    }
}

fn parse_usage_response(body: &str) -> Result<OfficialUsage, String> {
    let parsed: UsageResponse =
        serde_json::from_str(body).map_err(|e| format!("resposta inesperada: {e}"))?;
    let five_hour = parsed.five_hour.ok_or("resposta sem o campo five_hour")?;

    Ok(OfficialUsage {
        five_hour: to_percent_window(five_hour),
        seven_day: parsed.seven_day.map(to_percent_window),
    })
}

/// Distinguishes "the endpoint is rate-limiting us" from everything else, since callers
/// need to back off specifically on that (see `usage/mod.rs`'s `RATE_LIMIT_BACKOFF`) rather
/// than just retrying at the normal poll interval, which would just draw another 429.
#[derive(Debug)]
pub enum FetchError {
    RateLimited,
    Other(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::RateLimited => write!(f, "limite de requisições atingido (429)"),
            FetchError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl From<String> for FetchError {
    fn from(msg: String) -> Self {
        FetchError::Other(msg)
    }
}

impl From<&str> for FetchError {
    fn from(msg: &str) -> Self {
        FetchError::Other(msg.to_string())
    }
}

#[cfg(windows)]
pub fn fetch_official_usage() -> Result<OfficialUsage, FetchError> {
    let creds = read_credentials().ok_or("credenciais do Claude Code não encontradas")?;
    if creds.is_expired(Utc::now()) {
        return Err("token OAuth expirado".into());
    }

    let response = ureq::get(USAGE_ENDPOINT)
        .set("Authorization", &format!("Bearer {}", creds.access_token))
        .set("anthropic-beta", ANTHROPIC_BETA)
        .set("User-Agent", USER_AGENT)
        .set("Content-Type", "application/json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(429, _) => FetchError::RateLimited,
            other => FetchError::Other(format!("falha na requisição: {other}")),
        })?;

    let body = response
        .into_string()
        .map_err(|e| FetchError::Other(format!("falha ao ler resposta: {e}")))?;
    parse_usage_response(&body).map_err(FetchError::Other)
}

#[cfg(not(windows))]
pub fn fetch_official_usage() -> Result<OfficialUsage, FetchError> {
    Err("uso oficial só é buscado no Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_and_other_errors_display_differently() {
        assert_eq!(
            FetchError::RateLimited.to_string(),
            "limite de requisições atingido (429)"
        );
        assert_eq!(FetchError::Other("deu ruim".into()).to_string(), "deu ruim");
    }

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
        let body = r#"{"five_hour":{"utilization":42,"resets_at":"2025-01-01T12:00:00Z"}}"#;
        let usage = parse_usage_response(body).unwrap();
        assert_eq!(usage.five_hour.percent, 42.0);
        assert!(usage.five_hour.resets_at.is_some());
    }

    #[test]
    fn missing_five_hour_field_is_an_error() {
        let body = r#"{"seven_day":{"utilization":10}}"#;
        assert!(parse_usage_response(body).is_err());
    }

    #[test]
    fn percent_is_clamped_to_valid_range() {
        let body = r#"{"five_hour":{"utilization":142}}"#;
        let usage = parse_usage_response(body).unwrap();
        assert_eq!(usage.five_hour.percent, 100.0);
    }

    #[test]
    fn missing_resets_at_still_parses() {
        let body = r#"{"five_hour":{"utilization":7}}"#;
        let usage = parse_usage_response(body).unwrap();
        assert_eq!(usage.five_hour.percent, 7.0);
        assert!(usage.five_hour.resets_at.is_none());
    }

    #[test]
    fn parses_seven_day_alongside_five_hour() {
        let body = r#"{"five_hour":{"utilization":42},"seven_day":{"utilization":12,"resets_at":"2025-01-08T00:00:00Z"}}"#;
        let usage = parse_usage_response(body).unwrap();
        let weekly = usage.seven_day.expect("seven_day should be present");
        assert_eq!(weekly.percent, 12.0);
        assert!(weekly.resets_at.is_some());
    }

    #[test]
    fn missing_seven_day_is_none_not_an_error() {
        let body = r#"{"five_hour":{"utilization":42}}"#;
        let usage = parse_usage_response(body).unwrap();
        assert!(usage.seven_day.is_none());
    }
}
