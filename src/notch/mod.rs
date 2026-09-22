pub mod edge;
pub mod shape;

use edge::{offset_for_cursor, snap_edge_for_cursor, window_position, Edge, NotchSize};
use std::time::{Duration, Instant};

/// How long the pointer must hover before the pill expands.
const HOVER_EXPAND_DELAY: Duration = Duration::from_millis(120);
/// How long the pointer must be away before the panel collapses back to a pill.
const HOVER_COLLAPSE_DELAY: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotchState {
    Collapsed,
    Expanded,
}

/// Owns the notch's expand/collapse and drag-to-reposition state. Pure state machine —
/// it has no knowledge of the actual OS window; `app.rs` reads `window_position`/
/// `current_size` each frame and applies them via `ViewportCommand`.
pub struct Notch {
    pub state: NotchState,
    pub edge: Edge,
    pub offset_along_edge: f32,
    pub sizes: NotchSize,
    hover_started: Option<Instant>,
    unhover_started: Option<Instant>,
    dragging: bool,
}

impl Notch {
    pub fn new(edge: Edge, offset_along_edge: f32) -> Self {
        Self {
            state: NotchState::Collapsed,
            edge,
            offset_along_edge,
            sizes: NotchSize::default(),
            hover_started: None,
            unhover_started: None,
            dragging: false,
        }
    }

    pub fn current_size(&self) -> egui::Vec2 {
        match self.state {
            NotchState::Collapsed => self.sizes.collapsed,
            NotchState::Expanded => self.sizes.expanded,
        }
    }

    /// Advances the hover-driven expand/collapse state machine. `hovered` is whether the
    /// pointer is currently over the notch window this frame. Returns `true` if `state`
    /// changed as a result (callers use this to know the OS window needs to be resized).
    pub fn update_hover(&mut self, hovered: bool) -> bool {
        self.update_hover_at(hovered, Instant::now())
    }

    fn update_hover_at(&mut self, hovered: bool, now: Instant) -> bool {
        if hovered {
            self.unhover_started = None;
            self.hover_started.get_or_insert(now);
        } else {
            self.hover_started = None;
            self.unhover_started.get_or_insert(now);
        }

        let before = self.state;
        match self.state {
            NotchState::Collapsed => {
                if let Some(started) = self.hover_started {
                    if now.duration_since(started) >= HOVER_EXPAND_DELAY {
                        self.state = NotchState::Expanded;
                    }
                }
            }
            NotchState::Expanded => {
                if !self.dragging {
                    if let Some(started) = self.unhover_started {
                        if now.duration_since(started) >= HOVER_COLLAPSE_DELAY {
                            self.state = NotchState::Collapsed;
                        }
                    }
                }
            }
        }
        before != self.state
    }

    pub fn begin_drag(&mut self) {
        self.dragging = true;
    }

    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// Applies a drag step: given the cursor's current screen position and the monitor
    /// bounds, snaps to a new edge if the cursor is close enough to one, and updates the
    /// offset along the (possibly new) edge so the notch tracks the cursor.
    pub fn drag_to(&mut self, monitor: egui::Rect, cursor: egui::Pos2) {
        if let Some(new_edge) = snap_edge_for_cursor(monitor, cursor) {
            self.edge = new_edge;
        }
        self.offset_along_edge = offset_for_cursor(monitor, self.edge, cursor, self.current_size());
    }

    pub fn window_position(&self, monitor: egui::Rect) -> egui::Pos2 {
        window_position(
            monitor,
            self.edge,
            self.offset_along_edge,
            self.current_size(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_after_hover_delay() {
        let mut notch = Notch::new(Edge::Top, 0.0);
        let base = Instant::now();
        assert!(!notch.update_hover_at(true, base));
        assert_eq!(notch.state, NotchState::Collapsed);
        assert!(notch.update_hover_at(true, base + HOVER_EXPAND_DELAY));
        assert_eq!(notch.state, NotchState::Expanded);
    }

    #[test]
    fn collapses_after_unhover_delay() {
        let mut notch = Notch::new(Edge::Top, 0.0);
        let base = Instant::now();
        notch.update_hover_at(true, base);
        notch.update_hover_at(true, base + HOVER_EXPAND_DELAY);
        assert_eq!(notch.state, NotchState::Expanded);

        notch.update_hover_at(false, base + HOVER_EXPAND_DELAY);
        assert_eq!(notch.state, NotchState::Expanded);
        assert!(notch.update_hover_at(false, base + HOVER_EXPAND_DELAY + HOVER_COLLAPSE_DELAY));
        assert_eq!(notch.state, NotchState::Collapsed);
    }

    #[test]
    fn stays_expanded_while_dragging_even_without_hover() {
        let mut notch = Notch::new(Edge::Top, 0.0);
        let base = Instant::now();
        notch.update_hover_at(true, base);
        notch.update_hover_at(true, base + HOVER_EXPAND_DELAY);
        notch.begin_drag();

        notch.update_hover_at(false, base + HOVER_EXPAND_DELAY);
        let changed =
            notch.update_hover_at(false, base + HOVER_EXPAND_DELAY + HOVER_COLLAPSE_DELAY * 10);
        assert!(!changed);
        assert_eq!(notch.state, NotchState::Expanded);
    }

    #[test]
    fn drag_snaps_edge_and_tracks_cursor() {
        let mut notch = Notch::new(Edge::Top, 0.0);
        let monitor = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0));
        notch.drag_to(monitor, egui::pos2(10.0, 500.0));
        assert_eq!(notch.edge, Edge::Left);
    }
}
