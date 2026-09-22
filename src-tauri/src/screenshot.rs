//! Screen-capture flow: grab every monitor, spawn a fullscreen selection overlay window
//! (`ui/selection.html` drives the drag-rectangle UI), then crop and copy the result to
//! the clipboard once the frontend reports the chosen rectangle.
//!
//! Windows-only (xcap/arboard); stubbed out elsewhere so the crate still builds and can be
//! type-checked cross-platform.

use std::sync::Mutex;
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Manager;

/// Holds the just-captured monitor frames between `open_selection_overlay` and
/// `finish_selection`/`cancel_selection`, keyed by nothing in particular — there's only
/// ever one capture in flight at a time.
#[derive(Default)]
pub struct PendingCapture(pub Mutex<Option<Vec<win::MonitorShot>>>);

#[cfg(windows)]
pub mod win {
    use image::RgbaImage;
    use xcap::Monitor;

    pub struct MonitorShot {
        pub x: i32,
        pub y: i32,
        pub image: RgbaImage,
    }

    pub fn capture_all_monitors() -> Result<Vec<MonitorShot>, String> {
        let monitors = Monitor::all().map_err(|e| e.to_string())?;
        monitors
            .into_iter()
            .map(|m| {
                let image = m.capture_image().map_err(|e| e.to_string())?;
                Ok(MonitorShot {
                    x: m.x(),
                    y: m.y(),
                    image,
                })
            })
            .collect()
    }

    /// Bounding box (screen physical pixels) covering every captured monitor: (x, y, width, height).
    pub fn virtual_bounds(shots: &[MonitorShot]) -> Option<(i32, i32, u32, u32)> {
        if shots.is_empty() {
            return None;
        }
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for shot in shots {
            let (w, h) = shot.image.dimensions();
            min_x = min_x.min(shot.x);
            min_y = min_y.min(shot.y);
            max_x = max_x.max(shot.x + w as i32);
            max_y = max_y.max(shot.y + h as i32);
        }
        Some((min_x, min_y, (max_x - min_x) as u32, (max_y - min_y) as u32))
    }

    /// Crops the union of monitor screenshots down to a screen-space selection rect,
    /// stitching across monitor boundaries if the selection spans more than one.
    pub fn crop_selection(
        shots: &[MonitorShot],
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Option<RgbaImage> {
        let mut out = RgbaImage::new(width, height);
        let mut wrote_any = false;

        for shot in shots {
            let (iw, ih) = shot.image.dimensions();
            for out_y in 0..height {
                let src_y = y + out_y as i32 - shot.y;
                if src_y < 0 || src_y as u32 >= ih {
                    continue;
                }
                for out_x in 0..width {
                    let src_x = x + out_x as i32 - shot.x;
                    if src_x < 0 || src_x as u32 >= iw {
                        continue;
                    }
                    out.put_pixel(
                        out_x,
                        out_y,
                        *shot.image.get_pixel(src_x as u32, src_y as u32),
                    );
                    wrote_any = true;
                }
            }
        }

        wrote_any.then_some(out)
    }
}

#[cfg(not(windows))]
pub mod win {
    pub struct MonitorShot;
}

#[cfg(windows)]
pub fn open_selection_overlay(app: AppHandle) -> Result<(), String> {
    let shots = win::capture_all_monitors()?;
    let (x, y, width, height) = win::virtual_bounds(&shots).ok_or("nenhum monitor encontrado")?;

    app.state::<PendingCapture>()
        .0
        .lock()
        .unwrap()
        .replace(shots);

    if let Some(existing) = app.get_webview_window("selection") {
        let _ = existing.close();
    }

    // WebviewWindowBuilder::position/inner_size take *logical* pixels, but `x/y/width/
    // height` here are physical (straight from xcap's monitor capture) — passing them
    // through unconverted would misalign the overlay from the actual screen content on any
    // scaled display. Build with a throwaway size instead, then set the real physical
    // bounds via the runtime setters, which — unlike the builder — take a `Position`/`Size`
    // that can be explicitly physical.
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "selection",
        tauri::WebviewUrl::App("selection.html".into()),
    )
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focused(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    window
        .set_size(tauri::PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(windows)]
pub fn finish_selection(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let shots = app.state::<PendingCapture>().0.lock().unwrap().take();

    if let Some(window) = app.get_webview_window("selection") {
        let _ = window.close();
    }

    let Some(shots) = shots else {
        return Err("nenhuma captura pendente".into());
    };
    if width == 0 || height == 0 {
        return Ok(());
    }

    let cropped =
        win::crop_selection(&shots, x, y, width, height).ok_or("seleção fora da área capturada")?;

    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image_data = arboard::ImageData {
        width: cropped.width() as usize,
        height: cropped.height() as usize,
        bytes: std::borrow::Cow::Owned(cropped.into_raw()),
    };
    clipboard.set_image(image_data).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn cancel_selection(app: AppHandle) -> Result<(), String> {
    app.state::<PendingCapture>().0.lock().unwrap().take();
    if let Some(window) = app.get_webview_window("selection") {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_selection_overlay(_app: AppHandle) -> Result<(), String> {
    Err("captura de tela só é suportada no Windows".to_string())
}

#[cfg(not(windows))]
pub fn finish_selection(
    _app: AppHandle,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn cancel_selection(_app: AppHandle) -> Result<(), String> {
    Ok(())
}
