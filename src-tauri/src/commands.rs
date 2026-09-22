use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::config::{Edge, Settings};
use crate::usage::{UsageStatus, UsageWatcher};
use crate::{autostart, screenshot};

pub struct SettingsState(pub Mutex<Settings>);

/// Emitted on the "screenshot-result" event so the notch window (which isn't the one that
/// ran the capture — that's the transient "selection" window) can show the outcome.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScreenshotResult {
    pub ok: bool,
    /// `None` with `ok: false` means the user cancelled — nothing to show. `Some` means an
    /// actual error message to display.
    pub message: Option<String>,
}

/// Flat DTO for the frontend — easier to consume in plain JS than a tagged Rust enum.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageDto {
    pub status: &'static str,
    /// For `"unavailable"`, why. For `"active"` (the token-count fallback), why the
    /// official percentage wasn't used instead — surfaced so a failure here is ever
    /// diagnosable without a console, which release builds don't have.
    pub reason: Option<String>,
    /// Official percentage of the 5h plan limit used (only set for `"active_official"`).
    pub percent: Option<f64>,
    /// Official percentage of the 7d plan limit used (only set for `"active_official"`,
    /// and even then only if the endpoint's response included a weekly window).
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<String>,
    pub tokens: Option<u64>,
    pub started_at: Option<String>,
    pub resets_at: Option<String>,
    pub last_updated: String,
}

#[tauri::command]
pub fn get_usage(state: State<UsageWatcher>) -> UsageDto {
    let snapshot = state.snapshot();
    let last_updated = snapshot.last_updated.to_rfc3339();

    match snapshot.status {
        UsageStatus::Loading => UsageDto {
            status: "loading",
            reason: None,
            percent: None,
            weekly_percent: None,
            weekly_resets_at: None,
            tokens: None,
            started_at: None,
            resets_at: None,
            last_updated,
        },
        UsageStatus::Unavailable(reason) => UsageDto {
            status: "unavailable",
            reason: Some(reason),
            percent: None,
            weekly_percent: None,
            weekly_resets_at: None,
            tokens: None,
            started_at: None,
            resets_at: None,
            last_updated,
        },
        UsageStatus::Idle => UsageDto {
            status: "idle",
            reason: None,
            percent: None,
            weekly_percent: None,
            weekly_resets_at: None,
            tokens: None,
            started_at: None,
            resets_at: None,
            last_updated,
        },
        UsageStatus::ActiveOfficial {
            percent,
            resets_at,
            weekly_percent,
            weekly_resets_at,
        } => UsageDto {
            status: "active_official",
            reason: None,
            percent: Some(percent),
            weekly_percent,
            weekly_resets_at: weekly_resets_at.map(|dt| dt.to_rfc3339()),
            tokens: None,
            started_at: None,
            resets_at: resets_at.map(|dt| dt.to_rfc3339()),
            last_updated,
        },
        UsageStatus::Active {
            tokens,
            started_at,
            resets_at,
            official_error,
        } => UsageDto {
            status: "active",
            reason: Some(official_error),
            percent: None,
            weekly_percent: None,
            weekly_resets_at: None,
            tokens: Some(tokens),
            started_at: Some(started_at.to_rfc3339()),
            resets_at: Some(resets_at.to_rfc3339()),
            last_updated,
        },
    }
}

#[tauri::command]
pub fn get_settings(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_position(state: State<SettingsState>, edge: Edge, offset: f64) {
    let mut settings = state.0.lock().unwrap();
    settings.edge = edge;
    settings.offset_along_edge = offset;
    if let Err(err) = settings.save() {
        log::warn!("falha ao salvar posição do notch: {err}");
    }
}

#[tauri::command]
pub fn set_autostart(state: State<SettingsState>, enabled: bool) -> Result<(), String> {
    autostart::set_enabled(enabled)?;
    let mut settings = state.0.lock().unwrap();
    settings.start_with_windows = enabled;
    settings.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn capture_region(app: AppHandle) -> Result<(), String> {
    screenshot::open_selection_overlay(app)
}

#[tauri::command]
pub fn finish_selection(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let outcome = screenshot::finish_selection(app.clone(), x, y, width, height);
    let payload = match &outcome {
        Ok(()) => ScreenshotResult {
            ok: true,
            message: None,
        },
        Err(err) => ScreenshotResult {
            ok: false,
            message: Some(err.clone()),
        },
    };
    let _ = app.emit("screenshot-result", payload);
    outcome
}

#[tauri::command]
pub fn cancel_selection(app: AppHandle) -> Result<(), String> {
    let outcome = screenshot::cancel_selection(app.clone());
    let _ = app.emit(
        "screenshot-result",
        ScreenshotResult {
            ok: false,
            message: None,
        },
    );
    outcome
}
