use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::notch::edge::Edge;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub edge: Edge,
    /// Offset along the edge, in logical pixels, from the edge's start corner.
    pub offset_along_edge: f32,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            offset_along_edge: 0.0,
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
}
