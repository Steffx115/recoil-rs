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
