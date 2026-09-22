use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Manager};

use crate::commands::SettingsState;

pub fn setup(app: &App) -> tauri::Result<()> {
    let autostart_enabled = app
        .state::<SettingsState>()
        .0
        .lock()
        .unwrap()
        .start_with_windows;

    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Iniciar com o Windows",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&autostart_item, &quit_item])?;

    let icon = app
        .default_window_icon()
        .map(|icon| icon.clone().to_owned())
        .unwrap_or_else(fallback_icon);

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("win-notch")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "autostart" => {
                let state = app.state::<SettingsState>();
                let enabled = !state.0.lock().unwrap().start_with_windows;
                match crate::autostart::set_enabled(enabled) {
                    Ok(()) => {
                        state.0.lock().unwrap().start_with_windows = enabled;
                        if let Err(err) = state.0.lock().unwrap().save() {
                            log::warn!("falha ao salvar autostart: {err}");
                        }
                        autostart_item.set_checked(enabled).ok();
                    }
                    Err(err) => log::warn!("falha ao alternar autostart: {err}"),
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Small solid-color placeholder used only if the app icon somehow failed to embed.
fn fallback_icon() -> tauri::image::Image<'static> {
    let size = 16u32;
    let rgba = [122u8, 162, 255, 255].repeat((size * size) as usize);
    tauri::image::Image::new_owned(rgba, size, size)
}
