use image::RgbaImage;
use xcap::Monitor;

use super::selection::SelectionRect;

pub struct MonitorShot {
    pub x: i32,
    pub y: i32,
    pub image: RgbaImage,
}

/// Grabs a still frame of every connected monitor. Must be called *before* any capture
/// overlay is shown, so the overlay itself never ends up in the screenshot.
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

/// Crops the union of monitor screenshots down to a screen-space selection rect, stitching
/// across monitor boundaries if the selection spans more than one. Pixels outside every
/// captured monitor are left transparent (e.g. a selection dragged into empty desktop
/// space between two differently-sized monitors).
pub fn crop_selection(shots: &[MonitorShot], sel: SelectionRect) -> Option<RgbaImage> {
    let mut out = RgbaImage::new(sel.width, sel.height);
    let mut wrote_any = false;

    for shot in shots {
        let (iw, ih) = shot.image.dimensions();
        for out_y in 0..sel.height {
            let src_y = sel.y + out_y as i32 - shot.y;
            if src_y < 0 || src_y as u32 >= ih {
                continue;
            }
            for out_x in 0..sel.width {
                let src_x = sel.x + out_x as i32 - shot.x;
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
