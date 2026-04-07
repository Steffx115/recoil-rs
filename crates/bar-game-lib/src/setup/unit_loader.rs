//! Unit definition loading: BAR Lua directory or fallback RON assets.

use std::path::Path;

use recoil_sim::unit_defs::{UnitDef, UnitDefRegistry};

/// Load unit defs from BAR Lua directory or fallback RON assets.
///
/// When the BAR sandbox repo is present, loads all units and buildings
/// recursively (including T2 subdirectories), then resolves `buildoptions`
/// names to unit_type_ids and computes derived flags.
pub fn load_unit_defs(bar_units_path: &Path) -> UnitDefRegistry {
    let mut unit_def_registry = UnitDefRegistry::default();

    if bar_units_path.exists() {
        // Scan all faction/category directories recursively (handles T2 subdirs).
        let bar_dirs = [
            "ArmBots",
            "ArmVehicles",
            "ArmBuildings",
            "ArmAircraft",
            "ArmHovercraft",
            "ArmShips",
            "ArmSeaplanes",
            "ArmGantry",
            "CorBots",
            "CorVehicles",
            "CorBuildings",
            "CorAircraft",
            "CorHovercraft",
            "CorShips",
            "CorSeaplanes",
            "CorGantry",
        ];
        for dir in &bar_dirs {
            let path = bar_units_path.join(dir);
            if path.exists() {
                if let Ok(reg) = recoil_sim::lua_unitdefs::load_bar_unitdefs_recursive(&path) {
                    for (_id, def) in reg.defs {
                        unit_def_registry.register(def);
                    }
                }
            }
        }
        // Also load top-level .lua files (commanders, etc.)
        for e in std::fs::read_dir(bar_units_path)
            .into_iter()
            .flatten()
            .flatten()
        {
            if e.path().extension().is_some_and(|ext| ext == "lua") {
                if let Ok(def) = recoil_sim::lua_unitdefs::load_bar_unitdef(&e.path()) {
                    unit_def_registry.register(def);
                }
            }
        }

        // Second pass: resolve buildoptions names → unit_type_ids.
        unit_def_registry.resolve_build_options();
        unit_def_registry.compute_derived_flags();

        let building_count = unit_def_registry
            .defs
            .values()
            .filter(|d| d.is_building)
            .count();
        let factory_count = unit_def_registry
            .defs
            .values()
            .filter(|d| d.is_factory())
            .count();
        tracing::info!(
            "Loaded {} BAR unit defs ({} buildings, {} factories)",
            unit_def_registry.defs.len(),
            building_count,
            factory_count,
        );
    } else {
        unit_def_registry = UnitDefRegistry::load_directory(Path::new("assets/unitdefs/armada"))
            .unwrap_or_default();
        if let Ok(cortex) = UnitDefRegistry::load_directory(Path::new("assets/unitdefs/cortex")) {
            for (_id, def) in cortex.defs {
                unit_def_registry.register(def);
            }
        }
        // Compute derived flags for RON-loaded defs too.
        unit_def_registry.compute_derived_flags();
        tracing::info!(
            "Loaded {} RON unit defs (BAR repo not found)",
            unit_def_registry.defs.len()
        );
    }

    // Ensure we always have at least the core building defs, even if
    // neither the BAR repo nor the RON assets are available (e.g., in tests).
    if unit_def_registry.defs.is_empty() {
        register_fallback_defs(&mut unit_def_registry);
        tracing::info!(
            "Registered {} fallback unit defs",
            unit_def_registry.defs.len()
        );
    }

    unit_def_registry
}

/// Register a minimal set of unit/building defs for use when neither the BAR
/// sandbox repo nor RON asset files are available (e.g., in headless tests).
fn register_fallback_defs(registry: &mut UnitDefRegistry) {
    let make = |name: &str,
                max_health: f64,
                armor_class: &str,
                metal_cost: f64,
                energy_cost: f64,
                build_time: u32,
                max_speed: f64,
                build_power: Option<f64>,
                metal_production: Option<f64>,
                energy_production: Option<f64>,
                can_build_names: Vec<String>,
                weapons: Vec<recoil_sim::unit_defs::WeaponDefData>| {
        let mut def = UnitDef {
            name: name.to_string(),
            unit_type_id: recoil_sim::lua_unitdefs::hash_unit_name(name),
            max_health,
            armor_class: armor_class.to_string(),
            sight_range: 300.0,
            collision_radius: if max_speed == 0.0 { 32.0 } else { 10.0 },
            max_speed,
            acceleration: if max_speed > 0.0 { 1.0 } else { 0.0 },
            turn_rate: if max_speed > 0.0 { 0.1 } else { 0.0 },
            metal_cost,
            energy_cost,
            build_time,
            weapons,
            model_path: None,
            icon_path: None,
            categories: vec![],
            can_build: vec![],
            can_build_names,
            build_power,
            metal_production,
            energy_production,
            is_building: false,
            is_builder: false,
        };
        def.compute_derived_flags();
        def
    };

    // Commanders
    registry.register(make(
        "armcom",
        3000.0,
        "Heavy",
        0.0,
        0.0,
        0,
        1.2,
        Some(300.0),
        Some(0.5),
        Some(20.0),
        vec!["armsolar".into(), "armmex".into(), "armlab".into()],
        vec![recoil_sim::unit_defs::WeaponDefData {
            name: "Lightning Gun".into(),
            damage: 75.0,
            damage_type: "Laser".into(),
            range: 300.0,
            reload_time: 30,
            projectile_speed: 0.0,
            area_of_effect: 0.0,
        }],
    ));
    registry.register(make(
        "corcom",
        3000.0,
        "Heavy",
        0.0,
        0.0,
        0,
        1.2,
        Some(300.0),
        Some(0.5),
        Some(20.0),
        vec!["corsolar".into(), "cormex".into(), "corlab".into()],
        vec![recoil_sim::unit_defs::WeaponDefData {
            name: "Lightning Gun".into(),
            damage: 75.0,
            damage_type: "Laser".into(),
            range: 300.0,
            reload_time: 30,
            projectile_speed: 0.0,
            area_of_effect: 0.0,
        }],
    ));

    // Buildings — economy
    registry.register(make(
        "armsolar",
        500.0,
        "Building",
        150.0,
        0.0,
        120,
        0.0,
        None,
        None,
        Some(20.0),
        vec![],
        vec![],
    ));
    registry.register(make(
        "corsolar",
        500.0,
        "Building",
        150.0,
        0.0,
        120,
        0.0,
        None,
        None,
        Some(20.0),
        vec![],
        vec![],
    ));
    registry.register(make(
        "armmex",
        600.0,
        "Building",
        50.0,
        500.0,
        120,
        0.0,
        None,
        Some(3.0),
        None,
        vec![],
        vec![],
    ));
    registry.register(make(
        "cormex",
        600.0,
        "Building",
        50.0,
        500.0,
        120,
        0.0,
        None,
        Some(3.0),
        None,
        vec![],
        vec![],
    ));

    // Factories
    registry.register(make(
        "armlab",
        4000.0,
        "Building",
        650.0,
        2800.0,
        450,
        0.0,
        Some(100.0),
        None,
        None,
        vec![
            "armpw".into(),
            "armrock".into(),
            "armham".into(),
            "armwar".into(),
        ],
        vec![],
    ));
    registry.register(make(
        "corlab",
        4000.0,
        "Building",
        650.0,
        2800.0,
        450,
        0.0,
        Some(100.0),
        None,
        None,
        vec!["corak".into(), "corthud".into(), "corgator".into()],
        vec![],
    ));

    // Combat units
    for (name, hp, speed, metal, energy, bt, dmg, range, reload, proj_speed) in [
        ("armpw", 370.0, 2.9, 55.0, 900.0, 55, 9.0, 180.0, 9, 20.0),
        (
            "armrock", 680.0, 1.5, 130.0, 1500.0, 100, 120.0, 400.0, 60, 7.0,
        ),
        (
            "armham", 560.0, 1.5, 150.0, 2200.0, 120, 60.0, 600.0, 45, 5.0,
        ),
        (
            "armwar", 1000.0, 2.0, 200.0, 2800.0, 140, 15.0, 220.0, 10, 15.0,
        ),
        ("corak", 480.0, 2.7, 75.0, 800.0, 55, 11.0, 190.0, 10, 18.0),
        (
            "corthud", 600.0, 1.4, 120.0, 1600.0, 100, 110.0, 380.0, 60, 7.0,
        ),
        (
            "corgator", 760.0, 1.3, 170.0, 2500.0, 130, 70.0, 550.0, 50, 5.0,
        ),
    ] {
        registry.register(make(
            name,
            hp,
            "Light",
            metal,
            energy,
            bt,
            speed,
            None,
            None,
            None,
            vec![],
            vec![recoil_sim::unit_defs::WeaponDefData {
                name: "weapon".into(),
                damage: dmg,
                damage_type: "Normal".into(),
                range,
                reload_time: reload,
                projectile_speed: proj_speed,
                area_of_effect: 0.0,
            }],
        ));
    }

    // Resolve cross-references (commander can_build → factory → units)
    registry.resolve_build_options();
}

#[cfg(test)]
mod tests {
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
}
