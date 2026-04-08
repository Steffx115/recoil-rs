use super::*;
use std::path::Path;

#[test]
fn test_load_unit_defs_fallback() {
    // Should not panic even if no BAR or RON paths exist
    let registry = load_unit_defs(Path::new("nonexistent/units"));
    // May be empty or populated depending on assets; should not panic
    let _ = registry.defs.len();
}

#[test]
fn test_load_bar_unit_defs() {
    // Resolve relative to workspace root (2 levels up from this crate's manifest).
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap();
    let bar_path = workspace_root.join("../Beyond-All-Reason-Sandbox/units");
    if !bar_path.exists() {
        eprintln!(
            "Skipping test_load_bar_unit_defs: BAR sandbox repo not found at {}",
            bar_path.display()
        );
        return;
    }

    let registry = load_unit_defs(&bar_path);

    // Should have loaded hundreds of defs
    assert!(
        registry.defs.len() > 200,
        "Expected >200 defs, got {}",
        registry.defs.len()
    );

    // Verify buildings exist and have correct flags
    let building_count = registry.defs.values().filter(|d| d.is_building).count();
    assert!(
        building_count > 50,
        "Expected >50 buildings, got {}",
        building_count
    );

    let factory_count = registry.defs.values().filter(|d| d.is_factory()).count();
    assert!(
        factory_count > 10,
        "Expected >10 factories, got {}",
        factory_count
    );

    // Verify armcom commander has can_build populated
    let armcom = registry.get_by_name("armcom");
    assert!(armcom.is_some(), "armcom should be loaded");
    let armcom = armcom.unwrap();
    assert!(armcom.is_builder, "armcom should be a builder");
    assert!(
        !armcom.can_build.is_empty(),
        "armcom should have can_build entries (buildoptions)"
    );

    // Verify armsolar has energy production
    let solar = registry.get_by_name("armsolar");
    assert!(solar.is_some(), "armsolar should be loaded");
    let solar = solar.unwrap();
    assert!(solar.is_building, "armsolar should be a building");
    assert!(
        solar.energy_production.is_some(),
        "armsolar should produce energy"
    );

    // Verify armlab factory has can_build populated
    let armlab = registry.get_by_name("armlab");
    assert!(armlab.is_some(), "armlab should be loaded");
    let armlab = armlab.unwrap();
    assert!(armlab.is_factory(), "armlab should be a factory");
    assert!(
        !armlab.can_build.is_empty(),
        "armlab should have can_build entries"
    );

    // Verify armpw is a combat unit (not building, not builder)
    let armpw = registry.get_by_name("armpw");
    assert!(armpw.is_some(), "armpw should be loaded");
    let armpw = armpw.unwrap();
    assert!(!armpw.is_building);
    assert!(!armpw.is_builder);
    assert!(!armpw.weapons.is_empty(), "armpw should have weapons");

    // Verify armamex has metal production
    let mex = registry.get_by_name("armamex");
    if let Some(mex) = mex {
        assert!(mex.is_building);
        assert!(
            mex.metal_production.is_some(),
            "armamex should produce metal"
        );
    }

    // Print summary
    eprintln!(
        "BAR loading: {} total defs, {} buildings, {} factories, {} builders",
        registry.defs.len(),
        building_count,
        factory_count,
        registry.defs.values().filter(|d| d.is_builder).count()
    );
}
