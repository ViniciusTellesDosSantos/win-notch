//! Toggling "start win-notch with Windows". A no-op stub outside Windows so the rest of
//! the app can call this unconditionally regardless of target platform.

#[cfg(windows)]
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let auto = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("win-notch")
        .set_app_path(&exe.to_string_lossy())
        .set_args(&[] as &[&str])
        .build()
        .map_err(|e| e.to_string())?;

    if enabled {
        auto.enable().map_err(|e| e.to_string())
    } else {
        auto.disable().map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
pub fn set_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}
