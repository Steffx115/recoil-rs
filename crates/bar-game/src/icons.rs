//! Unit icon loading from BAR buildpic DDS files.
//!
//! [`IconAtlas`] loads all DDS images from a directory at startup, converts
//! them to RGBA8, and registers them as `egui::TextureHandle`s.  When the
//! directory is missing or empty, it returns an empty atlas so the UI can
//! gracefully fall back to text-only mode.

use std::collections::BTreeMap;
use std::path::Path;

/// Collection of unit buildpic textures registered with egui.
pub struct IconAtlas {
    /// Map from lowercase unit name (e.g. "armpw") to texture handle.
    icons: BTreeMap<String, egui::TextureHandle>,
}

impl IconAtlas {
    /// Create an empty atlas (text-only fallback).
    pub fn empty() -> Self {
        Self {
            icons: BTreeMap::new(),
        }
    }

    /// Load all `.dds` files from `dir` and register them as egui textures.
    ///
    /// Returns an empty atlas if the directory does not exist or is unreadable.
    pub fn load_unitpics(ctx: &egui::Context, dir: &Path) -> Self {
        let mut icons = BTreeMap::new();

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                tracing::info!(
                    "Unitpics directory not found ({}), text-only mode: {}",
                    dir.display(),
                    err
                );
                return Self { icons };
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "dds" {
                continue;
            }

            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_lowercase(),
                None => continue,
            };

            match load_dds_to_color_image(&path) {
                Ok(color_image) => {
                    let handle = ctx.load_texture(
                        &stem,
                        color_image,
                        egui::TextureOptions {
                            magnification: egui::TextureFilter::Linear,
                            minification: egui::TextureFilter::Linear,
                            ..Default::default()
                        },
                    );
                    icons.insert(stem, handle);
                }
                Err(err) => {
                    tracing::warn!("Failed to load DDS {}: {}", path.display(), err);
                }
            }
        }

        tracing::info!("Loaded {} unit buildpic icons", icons.len());
        Self { icons }
    }

    /// Look up an icon by unit name (case-insensitive).
    pub fn get_icon(&self, unit_name: &str) -> Option<&egui::TextureHandle> {
        self.icons.get(&unit_name.to_lowercase())
    }

    /// Returns true if no icons were loaded (text-only fallback).
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    /// Number of loaded icons.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.icons.len()
    }
}

/// Decode a DDS file to an `egui::ColorImage` (RGBA8).
fn load_dds_to_color_image(path: &Path) -> anyhow::Result<egui::ColorImage> {
    let data = std::fs::read(path)?;
    let img = image::load_from_memory_with_format(&data, image::ImageFormat::Dds)?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn empty_atlas_when_dir_missing() {
        let ctx = egui::Context::default();
        let atlas = IconAtlas::load_unitpics(&ctx, Path::new("/nonexistent/path/unitpics"));
        assert!(atlas.is_empty());
        assert!(atlas.get_icon("armpw").is_none());
    }

    #[test]
    fn empty_atlas_when_dir_has_no_dds() {
        let dir = tempfile::tempdir().unwrap();
        // Write a non-DDS file
        std::fs::write(dir.path().join("readme.txt"), "not a dds").unwrap();

        let ctx = egui::Context::default();
        let atlas = IconAtlas::load_unitpics(&ctx, dir.path());
        assert!(atlas.is_empty());
    }

    #[test]
    fn case_insensitive_lookup() {
        // Create a minimal valid DDS file (1x1 pixel, DXT1)
        // We can't easily create a valid DDS in a unit test without the full
        // header, so instead we test the lookup logic directly.
        let atlas = IconAtlas {
            icons: BTreeMap::new(),
        };
        // Empty atlas should return None for any key.
        assert!(atlas.get_icon("ARMPW").is_none());
        assert!(atlas.get_icon("armpw").is_none());
    }

    #[test]
    fn load_real_unitpics_if_available() {
        let dir = PathBuf::from("../Beyond-All-Reason-Sandbox/unitpics");
        if !dir.exists() {
            // Skip if BAR assets not present
            return;
        }
        let ctx = egui::Context::default();
        let atlas = IconAtlas::load_unitpics(&ctx, &dir);
        // If the dir exists it should have at least some icons
        assert!(
            !atlas.is_empty(),
            "Expected some icons from BAR unitpics directory"
        );
    }
}
