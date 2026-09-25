//! Screen-capture flow: grab every monitor and spawn a fullscreen selection overlay window
//! (`ui/selection.html`). Once the user picks a rectangle, `preview_selection` crops it and
//! hands the crop back for the overlay to show frozen in place while the user annotates it;
//! `finish_annotated` then lays the annotation layer over the crop, saves it as a PNG (see
//! `captures.rs`) and copies it to the clipboard.
//!
//! Windows-only (xcap/arboard); stubbed out elsewhere so the crate still builds and can be
//! type-checked cross-platform.

use image::RgbaImage;
use std::sync::Mutex;
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Manager;

/// The capture in flight between opening the overlay and finishing/cancelling it — there's
/// only ever one at a time. Starts as the raw monitor frames, becomes the cropped region
/// once the user has picked one (and is annotating it).
pub enum Pending {
    Shots(Vec<win::MonitorShot>),
    Cropped(RgbaImage),
}

#[derive(Default)]
pub struct PendingCapture(pub Mutex<Option<Pending>>);

/// Lays the frontend's annotation layer (transparent except where something was drawn)
/// over the captured region. The screenshot's own pixels never round-trip through the
/// webview — only the layer does — so everything outside the annotations stays exact.
pub fn compose(base: &RgbaImage, layer: &RgbaImage) -> Result<RgbaImage, String> {
    if base.dimensions() != layer.dimensions() {
        return Err(format!(
            "camada de anotação com tamanho {:?}, esperado {:?}",
            layer.dimensions(),
            base.dimensions()
        ));
    }
    let mut out = base.clone();
    image::imageops::overlay(&mut out, layer, 0, 0);
    Ok(out)
}

/// PNG-encodes the crop for the overlay's preview. Fast compression: it's shown once and
/// thrown away, so encode speed matters far more than size.
fn encode_png_fast(image: &RgbaImage) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::NoFilter)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

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
        .replace(Pending::Shots(shots));

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

/// What happened to a finished capture. It's always on the clipboard by the time this is
/// returned; saving the file is best-effort on top of that, so a save failure is reported
/// alongside instead of failing the whole capture.
pub struct FinishedCapture {
    pub saved_to: Result<std::path::PathBuf, String>,
}

/// Crops the chosen region out of the captured monitors, keeps it as the pending capture
/// and returns it PNG-encoded for the overlay to show frozen while it's being annotated.
#[cfg(windows)]
pub fn preview_selection(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let state = app.state::<PendingCapture>();
    let mut pending = state.0.lock().unwrap();
    let Some(Pending::Shots(shots)) = pending.as_ref() else {
        return Err("nenhuma captura pendente".into());
    };
    if width == 0 || height == 0 {
        return Err("seleção vazia".into());
    }
    let cropped =
        win::crop_selection(shots, x, y, width, height).ok_or("seleção fora da área capturada")?;
    let png = encode_png_fast(&cropped)?;
    *pending = Some(Pending::Cropped(cropped));
    Ok(png)
}

/// Finishes the capture with the annotation layer drawn over it (`None`: nothing was
/// drawn). The result is always on the clipboard by the time this returns `Ok`.
#[cfg(windows)]
pub fn finish_annotated(
    app: AppHandle,
    layer_png: Option<&[u8]>,
) -> Result<FinishedCapture, String> {
    let pending = app.state::<PendingCapture>().0.lock().unwrap().take();

    if let Some(window) = app.get_webview_window("selection") {
        let _ = window.close();
    }

    let Some(Pending::Cropped(cropped)) = pending else {
        return Err("nenhuma captura pendente".into());
    };
    let image = match layer_png {
        Some(bytes) => {
            let layer = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .map_err(|e| format!("camada de anotação inválida: {e}"))?
                .to_rgba8();
            compose(&cropped, &layer)?
        }
        None => cropped,
    };

    let saved_to = crate::captures::save_capture(&image);
    copy_image_to_clipboard(image)?;
    Ok(FinishedCapture { saved_to })
}

#[cfg(windows)]
pub fn copy_image_to_clipboard(image: image::RgbaImage) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Owned(image.into_raw()),
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
pub fn preview_selection(
    _app: AppHandle,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
) -> Result<Vec<u8>, String> {
    Err("captura de tela só é suportada no Windows".to_string())
}

#[cfg(not(windows))]
pub fn finish_annotated(
    _app: AppHandle,
    _layer_png: Option<&[u8]>,
) -> Result<FinishedCapture, String> {
    Err("captura de tela só é suportada no Windows".to_string())
}

#[cfg(not(windows))]
pub fn copy_image_to_clipboard(_image: image::RgbaImage) -> Result<(), String> {
    Err("área de transferência só é suportada no Windows".to_string())
}

#[cfg(not(windows))]
pub fn cancel_selection(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn annotations_cover_the_capture_only_where_drawn() {
        let base = RgbaImage::from_pixel(10, 8, Rgba([10, 20, 30, 255]));
        let mut layer = RgbaImage::new(10, 8);
        layer.put_pixel(2, 3, Rgba([255, 45, 45, 255]));
        // Half-transparent edge pixel (antialiasing): blended, not replaced.
        layer.put_pixel(3, 3, Rgba([255, 45, 45, 128]));

        let out = compose(&base, &layer).unwrap();
        assert_eq!(*out.get_pixel(2, 3), Rgba([255, 45, 45, 255]));
        let blended = out.get_pixel(3, 3);
        assert!(blended[0] > 10 && blended[0] < 255, "{blended:?}");
        for (x, y, pixel) in out.enumerate_pixels() {
            if (x, y) != (2, 3) && (x, y) != (3, 3) {
                assert_eq!(*pixel, Rgba([10, 20, 30, 255]), "pixel {x},{y} changed");
            }
        }
    }

    #[test]
    fn layer_of_the_wrong_size_is_rejected() {
        let base = RgbaImage::new(10, 8);
        let layer = RgbaImage::new(9, 8);
        assert!(compose(&base, &layer).is_err());
    }

    #[test]
    fn preview_png_round_trips() {
        let image = RgbaImage::from_pixel(5, 4, Rgba([1, 2, 3, 255]));
        let bytes = encode_png_fast(&image).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded, image);
    }
}
