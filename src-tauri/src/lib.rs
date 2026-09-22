mod autostart;
mod commands;
mod config;
mod screenshot;
mod tray;
mod usage;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use commands::SettingsState;
use config::Settings;
use screenshot::PendingCapture;
use usage::UsageWatcher;

pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .setup(|app| {
            let settings = Settings::load();
            let usage = UsageWatcher::spawn(usage::claude_code::default_projects_dir());

            apply_notch_position(app.handle(), &settings);
            // The window starts hidden (visible: false in tauri.conf.json) precisely so
            // the positioning above happens before the user ever sees it, instead of
            // flashing at the config's fallback spot first.
            if let Some(window) = app.get_webview_window("notch") {
                let _ = window.show();
            }

            app.manage(SettingsState(Mutex::new(settings)));
            app.manage(usage);
            app.manage(PendingCapture::default());

            tray::setup(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_usage,
            commands::get_settings,
            commands::save_position,
            commands::set_autostart,
            commands::capture_region,
            commands::finish_selection,
            commands::cancel_selection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Logical-pixel `(position, size)` of the monitor the "notch" window is currently on.
/// Monitor geometry from the OS is always physical; converting to logical here is what
/// lines it up with `COLLAPSED_SIZE` (declared in logical pixels, matching
/// `tauri.conf.json`) and with the same-unit math `ui/notch.js` does at runtime.
fn logical_monitor_geometry(window: &tauri::WebviewWindow) -> Option<((i32, i32), (u32, u32))> {
    let monitor = window.current_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let physical_pos = monitor.position();
    let physical_size = monitor.size();
    Some((
        (
            (physical_pos.x as f64 / scale) as i32,
            (physical_pos.y as f64 / scale) as i32,
        ),
        (
            (physical_size.width as f64 / scale) as u32,
            (physical_size.height as f64 / scale) as u32,
        ),
    ))
}

/// Moves the "notch" window to match `settings`' edge/offset. Used both at startup (before
/// the window is ever shown) and by the tray's "reset position" action (on an already-
/// visible, already-running window).
fn apply_notch_position(app: &AppHandle, settings: &Settings) {
    let Some(window) = app.get_webview_window("notch") else {
        return;
    };
    let Some((monitor_pos, monitor_size)) = logical_monitor_geometry(&window) else {
        return;
    };

    let (x, y) = settings.window_position(monitor_pos, monitor_size, config::COLLAPSED_SIZE);
    let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
}

/// Resets the notch to top-center of its current monitor and persists it — the safety net
/// for "dragged it somewhere and now can't find it": collapsed, it's just a small
/// unlabeled icon, easy to lose track of on a border nobody thought to check. Moves the
/// live window immediately and tells the frontend via an event, so `ui/notch.js`'s own
/// `edge`/`offsetCenter` (used for the next hover-expand or drag) don't go stale.
pub fn reset_notch_position(app: &AppHandle, settings_state: &SettingsState) {
    let mut settings = settings_state.0.lock().expect("settings mutex poisoned");
    settings.edge = config::Edge::Top;

    if let Some(window) = app.get_webview_window("notch") {
        if let Some((_, (monitor_width, _))) = logical_monitor_geometry(&window) {
            settings.offset_along_edge = monitor_width as f64 / 2.0;
        }
    }

    if let Err(err) = settings.save() {
        log::warn!("falha ao salvar posição resetada: {err}");
    }

    apply_notch_position(app, &settings);
    let _ = app.emit("notch-position-reset", settings.clone());
}
