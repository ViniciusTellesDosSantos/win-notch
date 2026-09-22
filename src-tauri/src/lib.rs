mod autostart;
mod commands;
mod config;
mod screenshot;
mod tray;
mod usage;

use std::sync::Mutex;
use tauri::Manager;

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

            reposition_notch(app, &settings);

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

/// Moves the "notch" window to its saved edge/offset on startup. `tauri.conf.json` gives
/// it a reasonable fallback position, but the real one depends on the user's saved
/// settings and the actual monitor geometry, both only known once the window exists.
fn reposition_notch(app: &tauri::App, settings: &Settings) {
    let Some(window) = app.get_webview_window("notch") else {
        return;
    };

    if let Ok(Some(monitor)) = window.current_monitor() {
        // Monitor geometry from the OS is always physical; convert to logical so it lines
        // up with COLLAPSED_SIZE (declared in logical pixels, matching tauri.conf.json)
        // and with the same-unit math ui/notch.js does at runtime.
        let scale = monitor.scale_factor();
        let physical_pos = monitor.position();
        let physical_size = monitor.size();
        let monitor_pos = (
            (physical_pos.x as f64 / scale) as i32,
            (physical_pos.y as f64 / scale) as i32,
        );
        let monitor_size = (
            (physical_size.width as f64 / scale) as u32,
            (physical_size.height as f64 / scale) as u32,
        );

        let (x, y) = settings.window_position(monitor_pos, monitor_size, config::COLLAPSED_SIZE);
        let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    }

    // The window starts hidden (visible: false in tauri.conf.json) precisely so any
    // repositioning above happens before the user ever sees it, instead of flashing at
    // the config's fallback spot first. Show it regardless of whether repositioning
    // above succeeded — falling back to the conf's default spot beats staying invisible.
    let _ = window.show();
}
