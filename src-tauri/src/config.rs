use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Collapsed tab size for `edge`, in *logical* pixels: upright on the side edges, flat on
/// the top/bottom ones (body plus a concave corner on each side along the edge). Kept in
/// sync by hand with `TAB_VERTICAL`/`TAB_HORIZONTAL` in `ui/notch.js`, since there's no
/// build step sharing constants between the two.
pub fn collapsed_size(edge: Edge) -> (u32, u32) {
    match edge {
        Edge::Top | Edge::Bottom => (136, 56),
        Edge::Left | Edge::Right => (64, 128),
    }
}

/// Which screen edge the notch is anchored to. Matches the strings used on the JS side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub edge: Edge,
    /// Position of the notch's *center* along the edge, in physical pixels, measured from
    /// the edge's start corner (left for Top/Bottom, top for Left/Right). Center-based
    /// (rather than top-left-based) so that switching between the collapsed pill and the
    /// expanded panel size keeps the same visual anchor point instead of jumping.
    pub offset_along_edge: f64,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            // A reasonable guess at "top-center" for a first run — it's clamped against
            // the real monitor width as soon as it's used, so this never overflows even
            // on a narrower screen.
            offset_along_edge: 640.0,
            start_with_windows: false,
        }
    }
}

impl Settings {
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "win-notch", "win-notch")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| toml::from_str(&contents).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::config_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, contents)
    }

    /// Top-left position for a window of `win_size` anchored to this settings' edge on a
    /// monitor at `monitor_pos` with `monitor_size`. Unit-agnostic (just arithmetic), but
    /// callers should pass *logical* pixels throughout so the notch looks the same size on
    /// every display regardless of DPI scaling — mirrors the (center-based) geometry the
    /// JS side uses while dragging and resizing, so the window lands in the same place on
    /// startup that it was left in, and stays visually anchored when it grows from the
    /// collapsed pill to the expanded panel.
    pub fn window_position(
        &self,
        monitor_pos: (i32, i32),
        monitor_size: (u32, u32),
        win_size: (u32, u32),
    ) -> (i32, i32) {
        let (mx, my) = monitor_pos;
        let (mw, mh) = (monitor_size.0 as i32, monitor_size.1 as i32);
        let (w, h) = (win_size.0 as i32, win_size.1 as i32);
        let center = self.offset_along_edge.round() as i32;

        match self.edge {
            Edge::Top => (mx + clamp_center(mw, center, w), my),
            Edge::Bottom => (mx + clamp_center(mw, center, w), my + mh - h),
            Edge::Left => (mx, my + clamp_center(mh, center, h)),
            Edge::Right => (mx + mw - w, my + clamp_center(mh, center, h)),
        }
    }
}

/// Converts a center position along an axis into a clamped top-left position, given the
/// axis's total length and the window's size along that same axis.
fn clamp_center(total: i32, center: i32, size_along: i32) -> i32 {
    let available = (total - size_along).max(0);
    (center - size_along / 2).clamp(0, available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let settings = Settings {
            edge: Edge::Right,
            offset_along_edge: 123.5,
            start_with_windows: true,
        };
        let text = toml::to_string_pretty(&settings).unwrap();
        let parsed: Settings = toml::from_str(&text).unwrap();
        assert_eq!(parsed.edge, Edge::Right);
        assert_eq!(parsed.offset_along_edge, 123.5);
        assert!(parsed.start_with_windows);
    }

    #[test]
    fn collapsed_tab_is_flat_on_top_bottom_and_upright_on_sides() {
        for edge in [Edge::Top, Edge::Bottom] {
            let (w, h) = collapsed_size(edge);
            assert!(w > h, "{edge:?} tab should be wider than tall");
        }
        for edge in [Edge::Left, Edge::Right] {
            let (w, h) = collapsed_size(edge);
            assert!(h > w, "{edge:?} tab should be taller than wide");
        }
    }

    #[test]
    fn top_edge_hugs_monitor_top_and_centers_on_offset() {
        let settings = Settings {
            edge: Edge::Top,
            offset_along_edge: 160.0,
            ..Settings::default()
        };
        let pos = settings.window_position((0, 0), (1920, 1080), (120, 28));
        assert_eq!(pos, (100, 0));
    }

    #[test]
    fn right_edge_hugs_monitor_right() {
        let settings = Settings {
            edge: Edge::Right,
            offset_along_edge: 0.0,
            ..Settings::default()
        };
        let pos = settings.window_position((0, 0), (1920, 1080), (28, 120));
        assert_eq!(pos, (1920 - 28, 0));
    }

    #[test]
    fn offset_clamps_within_monitor_and_respects_monitor_origin() {
        let settings = Settings {
            edge: Edge::Top,
            offset_along_edge: 100_000.0,
            ..Settings::default()
        };
        let pos = settings.window_position((1920, 0), (1920, 1080), (120, 28));
        assert_eq!(pos, (1920 + 1920 - 120, 0));
    }

    #[test]
    fn center_stays_fixed_when_window_grows_from_collapsed_to_expanded() {
        let settings = Settings {
            edge: Edge::Top,
            offset_along_edge: 500.0,
            ..Settings::default()
        };
        let collapsed = settings.window_position((0, 0), (1920, 1080), (120, 28));
        let expanded = settings.window_position((0, 0), (1920, 1080), (340, 180));
        let collapsed_center = collapsed.0 + 120 / 2;
        let expanded_center = expanded.0 + 340 / 2;
        assert_eq!(collapsed_center, expanded_center);
    }
}
