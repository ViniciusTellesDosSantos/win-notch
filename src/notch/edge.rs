use serde::{Deserialize, Serialize};

/// Which screen edge the notch is currently anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Collapsed (pill) and expanded (panel) sizes of the notch window, in logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct NotchSize {
    pub collapsed: egui::Vec2,
    pub expanded: egui::Vec2,
}

impl Default for NotchSize {
    fn default() -> Self {
        Self {
            collapsed: egui::vec2(120.0, 28.0),
            expanded: egui::vec2(320.0, 168.0),
        }
    }
}

/// How close (in logical pixels) the cursor must be to a screen edge while dragging
/// before the notch snaps to that edge.
const SNAP_MARGIN: f32 = 48.0;

/// Computes the top-left position (screen logical coordinates) of the notch window for the
/// given monitor bounds, anchored edge, offset along that edge, and current window size.
/// The offset is measured from the edge's start corner (left for Top/Bottom, top for
/// Left/Right) and is clamped so the window never leaves the monitor bounds.
pub fn window_position(
    monitor: egui::Rect,
    edge: Edge,
    offset_along_edge: f32,
    size: egui::Vec2,
) -> egui::Pos2 {
    match edge {
        Edge::Top => {
            let x = clamp_offset(monitor.left(), monitor.right(), offset_along_edge, size.x);
            egui::pos2(x, monitor.top())
        }
        Edge::Bottom => {
            let x = clamp_offset(monitor.left(), monitor.right(), offset_along_edge, size.x);
            egui::pos2(x, monitor.bottom() - size.y)
        }
        Edge::Left => {
            let y = clamp_offset(monitor.top(), monitor.bottom(), offset_along_edge, size.y);
            egui::pos2(monitor.left(), y)
        }
        Edge::Right => {
            let y = clamp_offset(monitor.top(), monitor.bottom(), offset_along_edge, size.y);
            egui::pos2(monitor.right() - size.x, y)
        }
    }
}

fn clamp_offset(min: f32, max: f32, offset: f32, size_along: f32) -> f32 {
    let available = (max - min - size_along).max(0.0);
    (min + offset).clamp(min, min + available)
}

/// Given a cursor position in screen coordinates while dragging, returns the nearest edge
/// of the monitor if the cursor is within `SNAP_MARGIN` of it, or `None` if it's not close
/// enough to any edge (in which case the notch should keep its current edge).
pub fn snap_edge_for_cursor(monitor: egui::Rect, cursor: egui::Pos2) -> Option<Edge> {
    let candidates = [
        (Edge::Top, (cursor.y - monitor.top()).abs()),
        (Edge::Bottom, (cursor.y - monitor.bottom()).abs()),
        (Edge::Left, (cursor.x - monitor.left()).abs()),
        (Edge::Right, (cursor.x - monitor.right()).abs()),
    ];

    candidates
        .into_iter()
        .filter(|(_, dist)| *dist <= SNAP_MARGIN)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(edge, _)| edge)
}

/// Converts a cursor position into an offset-along-edge value, centering the notch on the
/// cursor for the given edge and window size.
pub fn offset_for_cursor(
    monitor: egui::Rect,
    edge: Edge,
    cursor: egui::Pos2,
    size: egui::Vec2,
) -> f32 {
    match edge {
        Edge::Top | Edge::Bottom => cursor.x - monitor.left() - size.x / 2.0,
        Edge::Left | Edge::Right => cursor.y - monitor.top() - size.y / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0))
    }

    #[test]
    fn top_edge_position_uses_monitor_top() {
        let size = egui::vec2(120.0, 28.0);
        let pos = window_position(monitor(), Edge::Top, 100.0, size);
        assert_eq!(pos.y, 0.0);
        assert_eq!(pos.x, 100.0);
    }

    #[test]
    fn bottom_edge_position_uses_monitor_bottom_minus_height() {
        let size = egui::vec2(120.0, 28.0);
        let pos = window_position(monitor(), Edge::Bottom, 0.0, size);
        assert_eq!(pos.y, 1080.0 - 28.0);
    }

    #[test]
    fn left_edge_position_uses_monitor_left() {
        let size = egui::vec2(28.0, 120.0);
        let pos = window_position(monitor(), Edge::Left, 50.0, size);
        assert_eq!(pos.x, 0.0);
        assert_eq!(pos.y, 50.0);
    }

    #[test]
    fn right_edge_position_uses_monitor_right_minus_width() {
        let size = egui::vec2(28.0, 120.0);
        let pos = window_position(monitor(), Edge::Right, 0.0, size);
        assert_eq!(pos.x, 1920.0 - 28.0);
    }

    #[test]
    fn offset_is_clamped_within_monitor_bounds() {
        let size = egui::vec2(120.0, 28.0);
        let pos = window_position(monitor(), Edge::Top, 100_000.0, size);
        assert_eq!(pos.x, 1920.0 - 120.0);
        let pos = window_position(monitor(), Edge::Top, -100_000.0, size);
        assert_eq!(pos.x, 0.0);
    }

    #[test]
    fn snap_edge_picks_nearest_within_margin() {
        assert_eq!(
            snap_edge_for_cursor(monitor(), egui::pos2(500.0, 10.0)),
            Some(Edge::Top)
        );
        assert_eq!(
            snap_edge_for_cursor(monitor(), egui::pos2(10.0, 500.0)),
            Some(Edge::Left)
        );
        assert_eq!(
            snap_edge_for_cursor(monitor(), egui::pos2(500.0, 500.0)),
            None
        );
    }

    #[test]
    fn offset_for_cursor_centers_window_on_cursor() {
        let size = egui::vec2(120.0, 28.0);
        let offset = offset_for_cursor(monitor(), Edge::Top, egui::pos2(500.0, 0.0), size);
        assert_eq!(offset, 500.0 - 60.0);
    }
}
