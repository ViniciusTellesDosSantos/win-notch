//! Auto-update from this repo's GitHub Releases, via the official `tauri-plugin-updater`
//! (endpoint and signing public key in `tauri.conf.json` under `plugins.updater`). The
//! release workflow (`.github/workflows/release.yml`) builds the installer and signs it;
//! the plugin refuses anything whose signature doesn't match the public key.
//!
//! Checks shortly after startup and then every few hours. When a newer version exists the
//! tray item turns into "Atualizar para vX" and the popover shows an "Atualizar" button;
//! either one downloads, verifies and installs it. On Windows the plugin then launches the
//! installer (passive mode: just a progress bar) and exits the app, and the installer
//! starts the new version when it's done.

use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_updater::UpdaterExt;

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Version of the update waiting to be installed, if any.
#[derive(Default)]
pub struct UpdateState(pub Mutex<Option<String>>);

/// The tray's update item, kept so its label can follow the update state.
pub struct UpdateMenuItem(pub MenuItem<Wry>);

#[derive(Clone, serde::Serialize)]
struct UpdateAvailable {
    version: String,
}

pub fn spawn_periodic_check(app: AppHandle) {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                let _ = tauri::async_runtime::block_on(check(&app));
                std::thread::sleep(CHECK_INTERVAL);
            }
        })
        .expect("failed to spawn update-check thread");
}

pub fn idle_label(app: &AppHandle) -> String {
    format!("Procurar atualizações (v{})", app.package_info().version)
}

fn set_tray_label(app: &AppHandle, text: &str) {
    if let Some(item) = app.try_state::<UpdateMenuItem>() {
        let _ = item.0.set_text(text);
    }
}

/// Asks the release endpoint whether there's a newer version and updates the tray/popover
/// to match. Returns the available version, if any.
pub async fn check(app: &AppHandle) -> Result<Option<String>, String> {
    let result = async {
        let updater = app.updater().map_err(|e| e.to_string())?;
        updater.check().await.map_err(|e| e.to_string())
    }
    .await;

    match result {
        Ok(Some(update)) => {
            let version = update.version.clone();
            *app.state::<UpdateState>().0.lock().unwrap() = Some(version.clone());
            set_tray_label(app, &format!("Atualizar para v{version}"));
            let _ = app.emit(
                "update-available",
                UpdateAvailable {
                    version: version.clone(),
                },
            );
            Ok(Some(version))
        }
        Ok(None) => {
            *app.state::<UpdateState>().0.lock().unwrap() = None;
            set_tray_label(app, &idle_label(app));
            Ok(None)
        }
        Err(err) => {
            log::warn!("falha ao procurar atualização: {err}");
            Err(err)
        }
    }
}

/// Downloads, verifies and installs the latest release, then restarts into it. On Windows
/// the plugin exits the app itself once the installer is launched, so this only returns if
/// something failed.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    set_tray_label(app, "Baixando atualização…");
    let outcome = async {
        let updater = app.updater().map_err(|e| e.to_string())?;
        let update = updater
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("nenhuma atualização disponível")?;
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())
    }
    .await;

    match outcome {
        Ok(()) => app.restart(),
        Err(err) => {
            log::warn!("falha ao instalar atualização: {err}");
            // Back to whatever the current state really is.
            let _ = check(app).await;
            Err(err)
        }
    }
}
