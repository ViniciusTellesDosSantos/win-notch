use egui::{Color32, Context, Key, Pos2, Rect, Stroke, ViewportBuilder, ViewportId};

use super::capture::MonitorShot;

#[derive(Debug, Clone, Copy)]
pub struct SelectionRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The bounding box (screen-space) covering every captured monitor.
pub fn virtual_screen_bounds(shots: &[MonitorShot]) -> Option<Rect> {
    shots.iter().fold(None, |acc, shot| {
        let (w, h) = shot.image.dimensions();
        let rect = Rect::from_min_size(
            Pos2::new(shot.x as f32, shot.y as f32),
            egui::vec2(w as f32, h as f32),
        );
        Some(match acc {
            Some(existing) => existing.union(rect),
            None => rect,
        })
    })
}

fn rect_to_selection(rect: Rect) -> SelectionRect {
    SelectionRect {
        x: rect.min.x.round() as i32,
        y: rect.min.y.round() as i32,
        width: rect.width().round().max(1.0) as u32,
        height: rect.height().round().max(1.0) as u32,
    }
}

/// Opens a fullscreen, transparent, borderless overlay spanning every monitor and lets the
/// user drag out a rectangle to select. Returns the selection in screen coordinates, or
/// `None` if the user cancelled (Escape) or there was nothing to show an overlay over.
///
/// Uses egui's immediate-viewport mechanism, so this call blocks (pumping its own nested
/// event/paint loop) until the user finishes or cancels the selection.
pub fn run_selection_overlay(ctx: &Context, shots: &[MonitorShot]) -> Option<SelectionRect> {
    let bounds = virtual_screen_bounds(shots)?;

    let mut drag_start: Option<Pos2> = None;
    let mut result: Option<SelectionRect> = None;
    let mut cancelled = false;

    ctx.show_viewport_immediate(
        ViewportId::from_hash_of("win-notch-selection-overlay"),
        ViewportBuilder::default()
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_position(bounds.min)
            .with_inner_size(bounds.size()),
        |ctx, _class| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::from_black_alpha(90)))
                .show(ctx, |ui| {
                    if ctx.input(|i| i.key_pressed(Key::Escape)) {
                        cancelled = true;
                    }

                    let pointer = ctx.input(|i| i.pointer.clone());
                    if pointer.primary_pressed() {
                        drag_start = pointer.interact_pos();
                    }

                    if let Some(start) = drag_start {
                        if let Some(current) =
                            pointer.interact_pos().or_else(|| pointer.hover_pos())
                        {
                            let rect = Rect::from_two_pos(start, current);
                            let painter = ui.painter();
                            painter.rect_filled(rect, 0.0, Color32::from_white_alpha(24));
                            painter.rect_stroke(rect, 0.0, Stroke::new(1.5, Color32::WHITE));

                            if pointer.primary_released() {
                                result = Some(rect_to_selection(rect));
                            }
                        }
                    }
                });

            ctx.request_repaint();

            if result.is_some() || cancelled {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        },
    );

    if cancelled {
        None
    } else {
        result
    }
}
