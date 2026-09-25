use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::config::{Edge, Settings};
use crate::usage::{UsageStatus, UsageWatcher};
use crate::{autostart, captures, screenshot};

pub struct SettingsState(pub Mutex<Settings>);

/// Emitted on the "screenshot-result" event so the notch window (which isn't the one that
/// ran the capture — that's the transient "selection" window) can show the outcome.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScreenshotResult {
    pub ok: bool,
    /// `None` with `ok: false` means the user cancelled — nothing to show. `Some` means an
    /// actual error message to display. With `ok: true`, why saving the file failed (the
    /// image still made it to the clipboard).
    pub message: Option<String>,
    /// Where the capture was saved, when that worked.
    pub saved: Option<String>,
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
pub fn save_position(
    state: State<SettingsState>,
    edge: Edge,
    offset: f64,
    monitor: Option<String>,
) {
    let mut settings = state.0.lock().unwrap();
    settings.edge = edge;
    settings.offset_along_edge = offset;
    settings.monitor = monitor;
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

/// Crops the chosen region and returns it as PNG bytes (raw, not JSON) for the overlay to
/// show frozen while the user annotates it.
#[tauri::command]
pub async fn preview_selection(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<tauri::ipc::Response, String> {
    screenshot::preview_selection(app, x, y, width, height).map(tauri::ipc::Response::new)
}

/// Finishes the capture. The request body is the annotation layer as raw PNG bytes — empty
/// when nothing was drawn.
#[tauri::command]
pub async fn finish_annotated(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let layer = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) if !bytes.is_empty() => Some(bytes.as_slice()),
        _ => None,
    };
    let outcome = screenshot::finish_annotated(app.clone(), layer);
    let payload = match &outcome {
        Ok(finished) => match &finished.saved_to {
            Ok(path) => ScreenshotResult {
                ok: true,
                message: None,
                saved: Some(path.to_string_lossy().into_owned()),
            },
            Err(err) => ScreenshotResult {
                ok: true,
                message: Some(err.clone()),
                saved: None,
            },
        },
        Err(err) => ScreenshotResult {
            ok: false,
            message: Some(err.clone()),
            saved: None,
        },
    };
    let _ = app.emit("screenshot-result", payload);
    outcome.map(|_| ())
}

/// Newest saved captures with thumbnails, for the popover. Async so decoding a thumbnail
/// for the first time doesn't run on the main thread.
#[tauri::command]
pub async fn list_captures() -> Vec<captures::CaptureEntry> {
    captures::recent()
}

/// Copies a saved capture (one from `list_captures`) back to the clipboard.
#[tauri::command]
pub async fn copy_capture(path: String) -> Result<(), String> {
    let file = captures::resolve_capture(&path)?;
    let image = image::open(&file)
        .map_err(|e| format!("não foi possível abrir a captura: {e}"))?
        .to_rgba8();
    screenshot::copy_image_to_clipboard(image)
}

#[tauri::command]
pub fn open_captures_folder() -> Result<(), String> {
    let dir = captures::captures_dir().ok_or("pasta de capturas não encontrada")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("não foi possível abrir o Explorer: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_selection(app: AppHandle) -> Result<(), String> {
    let outcome = screenshot::cancel_selection(app.clone());
    let _ = app.emit(
        "screenshot-result",
        ScreenshotResult {
            ok: false,
            message: None,
            saved: None,
        },
    );
    outcome
}
