use egui::{Color32, Painter, Rect, Rounding, Stroke};

pub const BG_COLOR: Color32 = Color32::from_rgba_premultiplied(16, 16, 18, 235);
pub const BORDER_COLOR: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 22);
pub const ACCENT_COLOR: Color32 = Color32::from_rgb(122, 162, 255);
pub const MUTED_TEXT: Color32 = Color32::from_rgb(160, 160, 168);

/// Paints the notch's rounded background panel (used for both the collapsed pill and the
/// expanded panel — only `rect` and `corner_radius` differ between the two states).
pub fn paint_panel(painter: &Painter, rect: Rect, corner_radius: f32) {
    let rounding = Rounding::same(corner_radius);
    painter.rect_filled(rect, rounding, BG_COLOR);
    painter.rect_stroke(rect, rounding, Stroke::new(1.0, BORDER_COLOR));
}

/// The corner radius that makes a rect of this size look like a fully-rounded pill.
pub fn pill_corner_radius(size: egui::Vec2) -> f32 {
    size.x.min(size.y) / 2.0
}
