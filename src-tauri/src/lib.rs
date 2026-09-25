mod autostart;
mod captures;
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
        // Must be registered first: on Windows this needs to be able to hand off to an
        // already-running instance and exit before any other plugin/setup work happens.
        // Without it, nothing stops the app from being launched twice — two "notch" windows
        // fighting over the same screen edge, two tray icons, two threads racing to read the
        // same OAuth credentials file and overwrite the same saved position.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("notch") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
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
            commands::preview_selection,
            commands::finish_annotated,
            commands::cancel_selection,
            commands::list_captures,
            commands::copy_capture,
            commands::open_captures_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Logical size of `monitor`. Monitor geometry from the OS is always physical; converting
/// to logical here is what lines it up with `config::collapsed_size` (declared in logical
/// pixels, matching `tauri.conf.json`) and with the same-unit math `ui/notch.js` does.
fn logical_monitor_size(monitor: &tauri::Monitor) -> (u32, u32) {
    let scale = monitor.scale_factor();
    let size = monitor.size();
    (
        (size.width as f64 / scale) as u32,
        (size.height as f64 / scale) as u32,
    )
}

/// The monitor saved in `settings`, if it's still connected; otherwise whichever one the
/// window is on, then the primary one.
fn target_monitor(window: &tauri::WebviewWindow, settings: &Settings) -> Option<tauri::Monitor> {
    if let Some(name) = &settings.monitor {
        let saved = window
            .available_monitors()
            .ok()
            .and_then(|all| all.into_iter().find(|m| m.name() == Some(name)));
        if saved.is_some() {
            return saved;
        }
    }
    window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
}

/// Moves the "notch" window to match `settings`' edge/offset/monitor. Used both at startup
/// (before the window is ever shown) and by the tray's "reset position" action (on an
/// already-visible, already-running window).
fn apply_notch_position(app: &AppHandle, settings: &Settings) {
    let Some(window) = app.get_webview_window("notch") else {
        return;
    };
    let Some(monitor) = target_monitor(&window, settings) else {
        return;
    };

    // Resized too, not just moved: the collapsed tab's orientation depends on the edge, so
    // a saved left/right edge needs the upright size tauri.conf.json can't know about.
    let size = config::collapsed_size(settings.edge);
    let (x, y) = settings.window_position((0, 0), logical_monitor_size(&monitor), size);
    // The position goes in as *physical* pixels using the target monitor's own scale: a
    // logical position would be converted with the scale of whichever monitor the window
    // is on right now, which is wrong when the two monitors are scaled differently. Size
    // first, position second, so the window lands on the target monitor already at its
    // final size (Windows then keeps its logical size if the scale differs there).
    let scale = monitor.scale_factor();
    let origin = monitor.position();
    let _ = window.set_size(tauri::LogicalSize::new(size.0 as f64, size.1 as f64));
    let _ = window.set_position(tauri::PhysicalPosition::new(
        origin.x + (x as f64 * scale).round() as i32,
        origin.y + (y as f64 * scale).round() as i32,
    ));
}

/// Resets the notch to top-center of its current monitor and persists it — the safety net
/// for "dragged it somewhere and now can't find it": collapsed, it's just a small
/// unlabeled icon, easy to lose track of on a border nobody thought to check. Moves the
/// live window immediately and tells the frontend via an event, so `ui/notch.js`'s own
/// `edge`/`offsetCenter` (used for the next hover-expand or drag) don't go stale.
pub fn reset_notch_position(app: &AppHandle, settings_state: &SettingsState) {
    let mut settings = settings_state.0.lock().expect("settings mutex poisoned");
    settings.edge = config::Edge::Top;

    if let Some(monitor) = app
        .get_webview_window("notch")
        .and_then(|window| window.current_monitor().ok().flatten())
    {
        settings.offset_along_edge = logical_monitor_size(&monitor).0 as f64 / 2.0;
        settings.monitor = monitor.name().cloned();
    }

    if let Err(err) = settings.save() {
        log::warn!("falha ao salvar posição resetada: {err}");
    }

    apply_notch_position(app, &settings);
    let _ = app.emit("notch-position-reset", settings.clone());
}
