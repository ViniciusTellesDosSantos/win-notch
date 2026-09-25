//! Saved screenshots: every capture is written as a PNG into `Pictures\win-notch` (the
//! folder itself is the history — nothing else to keep in sync, and deleting a file in
//! Explorer removes it from the popover too), and the popover lists the newest few with
//! small thumbnails cached under the app's cache dir.

use base64::Engine;
use chrono::{DateTime, Local};
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// How many recent captures the popover shows.
pub const RECENT_LIMIT: usize = 4;

/// Thumbnails are generated to fit this box — 2x the size they're displayed at in the
/// popover, so they stay sharp on scaled displays.
const THUMB_MAX: (u32, u32) = (128, 88);

#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureEntry {
    pub path: String,
    pub name: String,
    pub taken_at: String,
    /// `data:image/png;base64,...` — `None` if the file couldn't be decoded.
    pub thumb: Option<String>,
}

pub fn captures_dir() -> Option<PathBuf> {
    let dirs = directories::UserDirs::new()?;
    let pictures = dirs
        .picture_dir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dirs.home_dir().join("Pictures"));
    Some(pictures.join("win-notch"))
}

fn thumbs_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "win-notch", "win-notch")
        .map(|dirs| dirs.cache_dir().join("thumbs"))
}

fn capture_file_name(at: DateTime<Local>) -> String {
    format!("captura-{}.png", at.format("%Y-%m-%d_%H-%M-%S"))
}

/// Writes `image` into `dir` under a timestamped name, adding a numeric suffix if two
/// captures land in the same second.
fn save_into(dir: &Path, image: &RgbaImage, at: DateTime<Local>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("não foi possível criar {}: {e}", dir.display()))?;
    let base = capture_file_name(at);
    let mut path = dir.join(&base);
    let mut n = 2;
    while path.exists() {
        path = dir.join(base.replace(".png", &format!("-{n}.png")));
        n += 1;
    }
    image
        .save(&path)
        .map_err(|e| format!("não foi possível salvar a captura: {e}"))?;
    Ok(path)
}

pub fn save_capture(image: &RgbaImage) -> Result<PathBuf, String> {
    let dir = captures_dir().ok_or("pasta de imagens do usuário não encontrada")?;
    save_into(&dir, image, Local::now())
}

pub fn recent() -> Vec<CaptureEntry> {
    match (captures_dir(), thumbs_dir()) {
        (Some(dir), Some(thumbs)) => recent_in(&dir, &thumbs, RECENT_LIMIT),
        _ => Vec::new(),
    }
}

fn recent_in(dir: &Path, thumbs: &Path, limit: usize) -> Vec<CaptureEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = read
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));

    files
        .into_iter()
        .take(limit)
        .map(|(path, modified)| CaptureEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            taken_at: DateTime::<Local>::from(modified).to_rfc3339(),
            thumb: thumbnail_data_url(&path, thumbs),
            path: path.to_string_lossy().into_owned(),
        })
        .collect()
}

/// Thumbnail as a data URL, generated once per capture and cached (decoding a full-screen
/// PNG every time the popover opens would be needlessly slow).
fn thumbnail_data_url(path: &Path, thumbs: &Path) -> Option<String> {
    let cached = thumbs.join(path.file_name()?);
    let is_fresh = match (std::fs::metadata(&cached), std::fs::metadata(path)) {
        (Ok(thumb), Ok(source)) => match (thumb.modified(), source.modified()) {
            (Ok(t), Ok(s)) => t >= s,
            _ => false,
        },
        _ => false,
    };

    if !is_fresh {
        let thumb = image::open(path).ok()?.thumbnail(THUMB_MAX.0, THUMB_MAX.1);
        std::fs::create_dir_all(thumbs).ok()?;
        thumb.save(&cached).ok()?;
    }

    let bytes = std::fs::read(&cached).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Turns a path the frontend sent back into a real file, but only if it's a capture: the
/// command taking it can read and copy the file, so it must not work for arbitrary paths.
pub fn resolve_capture(path: &str) -> Result<PathBuf, String> {
    let dir = captures_dir().ok_or("pasta de capturas não encontrada")?;
    resolve_in(&dir, path)
}

fn resolve_in(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let not_a_capture = || "arquivo não é uma captura do win-notch".to_string();
    let dir = dir.canonicalize().map_err(|_| not_a_capture())?;
    let file = Path::new(path)
        .canonicalize()
        .map_err(|_| not_a_capture())?;
    if file.parent() == Some(dir.as_path()) && file.is_file() {
        Ok(file)
    } else {
        Err(not_a_capture())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("win-notch-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_image() -> RgbaImage {
        RgbaImage::from_pixel(400, 300, image::Rgba([200, 60, 20, 255]))
    }

    #[test]
    fn file_name_is_timestamped() {
        let at = Local.with_ymd_and_hms(2026, 9, 25, 14, 3, 7).unwrap();
        assert_eq!(capture_file_name(at), "captura-2026-09-25_14-03-07.png");
    }

    #[test]
    fn same_second_captures_get_distinct_files() {
        let dir = temp_dir("collide");
        let at = Local.with_ymd_and_hms(2026, 9, 25, 14, 3, 7).unwrap();
        let first = save_into(&dir, &sample_image(), at).unwrap();
        let second = save_into(&dir, &sample_image(), at).unwrap();
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-2.png"));
    }

    #[test]
    fn recent_lists_newest_first_with_thumbnails_and_respects_limit() {
        let dir = temp_dir("recent");
        let thumbs = temp_dir("recent-thumbs");
        for second in 0..3 {
            let at = Local.with_ymd_and_hms(2026, 9, 25, 14, 3, second).unwrap();
            save_into(&dir, &sample_image(), at).unwrap();
            // Distinct mtimes, since that's what "newest" is based on.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::write(dir.join("notas.txt"), "not a capture").unwrap();

        let entries = recent_in(&dir, &thumbs, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "captura-2026-09-25_14-03-02.png");
        assert_eq!(entries[1].name, "captura-2026-09-25_14-03-01.png");
        let thumb = entries[0].thumb.as_deref().unwrap();
        assert!(thumb.starts_with("data:image/png;base64,"));

        let cached = image::open(thumbs.join(&entries[0].name)).unwrap();
        assert!(cached.width() <= THUMB_MAX.0 && cached.height() <= THUMB_MAX.1);
    }

    #[test]
    fn only_files_inside_the_captures_folder_resolve() {
        let dir = temp_dir("resolve");
        let at = Local.with_ymd_and_hms(2026, 9, 25, 14, 3, 7).unwrap();
        let saved = save_into(&dir, &sample_image(), at).unwrap();
        assert!(resolve_in(&dir, &saved.to_string_lossy()).is_ok());

        let outside = temp_dir("resolve-outside").join("x.png");
        std::fs::write(&outside, "x").unwrap();
        assert!(resolve_in(&dir, &outside.to_string_lossy()).is_err());

        let sneaky = dir.join("..").join(outside.file_name().unwrap());
        assert!(resolve_in(&dir, &sneaky.to_string_lossy()).is_err());
        assert!(resolve_in(&dir, "C:\\Windows\\win.ini").is_err());
    }
}
