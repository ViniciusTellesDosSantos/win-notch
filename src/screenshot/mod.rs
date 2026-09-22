#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod selection;

/// Runs the full "screenshot a region" flow: capture every monitor, show the drag-select
/// overlay, crop, and copy the result to the clipboard. Returns `Ok(true)` on a successful
/// capture, `Ok(false)` if the user cancelled, or `Err` with a user-facing message.
#[cfg(windows)]
pub fn capture_region_to_clipboard(ctx: &egui::Context) -> Result<bool, String> {
    let shots = capture::capture_all_monitors()?;
    let Some(selection) = selection::run_selection_overlay(ctx, &shots) else {
        return Ok(false);
    };
    let cropped = capture::crop_selection(&shots, selection)
        .ok_or_else(|| "seleção fora da área capturada".to_string())?;

    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image_data = arboard::ImageData {
        width: cropped.width() as usize,
        height: cropped.height() as usize,
        bytes: std::borrow::Cow::Owned(cropped.into_raw()),
    };
    clipboard.set_image(image_data).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(not(windows))]
pub fn capture_region_to_clipboard(_ctx: &egui::Context) -> Result<bool, String> {
    Err("captura de tela só é suportada no Windows".to_string())
}
