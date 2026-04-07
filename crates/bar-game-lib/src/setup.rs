//! Game initialization: loading unit defs, map manifest, spawning commanders.

use std::collections::BTreeMap;
use std::path::Path;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use recoil_math::{SimFloat, SimVec3};
use recoil_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};
use recoil_sim::commands::CommandQueue;
use recoil_sim::construction::Builder;
use recoil_sim::economy::{init_economy, ResourceProducer};
use recoil_sim::factory::{UnitBlueprint, UnitRegistry};
use recoil_sim::fog::FogOfWar;
use recoil_sim::lifecycle::spawn_unit;
use recoil_sim::map::load_map_manifest;
use recoil_sim::sim_runner;
use recoil_sim::targeting::WeaponRegistry;
use recoil_sim::unit_defs::UnitDefRegistry;
use recoil_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SightRange,
    Target, UnitType, Velocity,
};

/// Configuration produced by game setup, containing IDs needed later.
pub struct GameConfig {
    /// Weapon def IDs per unit type ID.
    pub weapon_def_ids: BTreeMap<u32, Vec<u32>>,
    /// Metal spot positions from the map manifest.
    pub metal_spots: Vec<(f64, f64)>,
    /// Commander entity for team 0.
    pub commander_team0: Option<Entity>,
    /// Commander entity for team 1.
    pub commander_team1: Option<Entity>,
}

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
        tracing::info!("Registered {} fallback unit defs", unit_def_registry.defs.len());
    }

    unit_def_registry
}

/// Full game setup: init world, load defs, register weapons, spawn commanders.
///
/// Returns a `GameConfig` with entity handles and cached data.
pub fn setup_game(
    world: &mut World,
    bar_units_path: &Path,
    map_manifest_path: &Path,
) -> GameConfig {
    sim_runner::init_sim_world(world);

    let unit_def_registry = load_unit_defs(bar_units_path);

    // Register weapon defs
    let mut weapon_def_ids: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    {
        let mut registry = world.resource_mut::<WeaponRegistry>();
        for (unit_type_id, unit_def) in &unit_def_registry.defs {
            let mut ids = Vec::new();
            for weapon_def in unit_def.to_weapon_defs() {
                let id = registry.defs.len() as u32;
                registry.defs.push(weapon_def);
                ids.push(id);
            }
            weapon_def_ids.insert(*unit_type_id, ids);
        }
    }

    // Load map manifest
    let map_manifest = load_map_manifest(map_manifest_path).ok();
    let metal_spots = if let Some(ref manifest) = map_manifest {
        tracing::info!(
            "Loaded map '{}' with {} start positions, {} metal spots",
            manifest.name,
            manifest.start_positions.len(),
            manifest.metal_spots.len(),
        );
        manifest.metal_spots.iter().map(|ms| (ms.x, ms.z)).collect()
    } else {
        Vec::new()
    };

    init_economy(world, &[0, 1]);

    // Metal spots resource (for linking mex buildings to map spots)
    {
        let spots = if let Some(ref manifest) = map_manifest {
            manifest
                .metal_spots
                .iter()
                .map(|ms| recoil_sim::map::MetalSpot {
                    x: ms.x,
                    z: ms.z,
                    metal_per_tick: ms.metal_per_tick,
                })
                .collect()
        } else {
            Vec::new()
        };
        world.insert_resource(recoil_sim::map::MetalSpots { spots });
    }

    // Fog of War
    let fog = FogOfWar::new(64, 64, &[0, 1]);
    world.insert_resource(fog);

    // Build UnitRegistry for factory_system from loaded UnitDefs
    let mut unit_registry = UnitRegistry::default();
    for def in unit_def_registry.defs.values() {
        unit_registry.blueprints.push(UnitBlueprint {
            unit_type_id: def.unit_type_id,
            metal_cost: SimFloat::from_f64(def.metal_cost),
            energy_cost: SimFloat::from_f64(def.energy_cost),
            build_time: if def.build_time > 0 {
                def.build_time
            } else {
                60
            },
            max_health: SimFloat::from_f64(def.max_health),
        });
    }
    world.insert_resource(unit_registry);

    // Determine start positions
    let (start_pos_0, start_pos_1) = if let Some(ref manifest) = map_manifest {
        let sp0 = manifest.start_positions.iter().find(|sp| sp.team == 0);
        let sp1 = manifest.start_positions.iter().find(|sp| sp.team == 1);
        (
            sp0.map(|sp| (sp.x as f32, sp.z as f32))
                .unwrap_or((200.0, 200.0)),
            sp1.map(|sp| (sp.x as f32, sp.z as f32))
                .unwrap_or((824.0, 824.0)),
        )
    } else {
        ((200.0, 200.0), (824.0, 824.0))
    };

    // Spawn commanders
    let commander_team0 = Some(spawn_commander(
        world,
        &unit_def_registry,
        &weapon_def_ids,
        start_pos_0,
        0,
    ));
    let commander_team1 = Some(spawn_commander(
        world,
        &unit_def_registry,
        &weapon_def_ids,
        start_pos_1,
        1,
    ));

    // Store unit def registry
    world.insert_resource(unit_def_registry);

    GameConfig {
        weapon_def_ids,
        metal_spots,
        commander_team0,
        commander_team1,
    }
}

/// Spawn a commander entity with full components (builder, resource producer, weapons).
pub fn spawn_commander(
    world: &mut World,
    unit_def_registry: &UnitDefRegistry,
    weapon_def_ids: &BTreeMap<u32, Vec<u32>>,
    pos: (f32, f32),
    team: u8,
) -> Entity {
    let cmd_name = if team == 0 { "armcom" } else { "corcom" };
    let found_def = unit_def_registry
        .defs
        .values()
        .find(|d| d.name.to_lowercase() == cmd_name);

    let (hp, max_speed, accel, turn_rate, collision_r, sight_r, armor_class, unit_type_id) =
        if let Some(def) = found_def {
            (
                SimFloat::from_f64(def.max_health),
                SimFloat::from_f64(def.max_speed),
                SimFloat::from_f64(def.acceleration),
                SimFloat::from_f64(def.turn_rate),
                SimFloat::from_f64(def.collision_radius),
                SimFloat::from_f64(def.sight_range),
                def.parse_armor_class(),
                def.unit_type_id,
            )
        } else {
            // Fallback commander stats
            (
                SimFloat::from_int(3000),
                SimFloat::from_ratio(3, 2),
                SimFloat::ONE,
                SimFloat::PI / SimFloat::from_int(30),
                SimFloat::from_int(12),
                SimFloat::from_int(300),
                ArmorClass::Heavy,
                9999u32,
            )
        };

    let weapon_ids = weapon_def_ids
        .get(&unit_type_id)
        .cloned()
        .unwrap_or_default();

    let entity = spawn_unit(
        world,
        Position {
            pos: SimVec3::new(
                SimFloat::from_f32(pos.0),
                SimFloat::ZERO,
                SimFloat::from_f32(pos.1),
            ),
        },
        UnitType { id: unit_type_id },
        Allegiance { team },
        Health {
            current: hp,
            max: hp,
        },
    );

    let weapons: Vec<WeaponInstance> = weapon_ids
        .iter()
        .map(|&def_id| WeaponInstance {
            def_id,
            reload_remaining: 0,
        })
        .collect();

    world.entity_mut(entity).insert((
        MoveState::Idle,
        MovementParams {
            max_speed,
            acceleration: accel,
            turn_rate,
        },
        CollisionRadius {
            radius: collision_r,
        },
        Heading {
            angle: SimFloat::ZERO,
        },
        Velocity { vel: SimVec3::ZERO },
        armor_class,
        Target { entity: None },
        WeaponSet { weapons },
        CommandQueue::default(),
        SightRange { range: sight_r },
        // Commander is a builder
        Builder {
            build_power: SimFloat::from_int(300),
        },
        // Commander produces a small trickle of resources
        ResourceProducer {
            metal_per_tick: SimFloat::from_ratio(1, 2),
            energy_per_tick: SimFloat::from_int(20),
        },
    ));

    entity
}

/// Register a minimal set of unit/building defs for use when neither the BAR
/// sandbox repo nor RON asset files are available (e.g., in headless tests).
fn register_fallback_defs(registry: &mut UnitDefRegistry) {
    use recoil_sim::unit_defs::UnitDef;

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
        "armcom", 3000.0, "Heavy", 0.0, 0.0, 0, 1.2, Some(300.0),
        Some(0.5), Some(20.0),
        vec!["armsolar".into(), "armmex".into(), "armlab".into()],
        vec![recoil_sim::unit_defs::WeaponDefData {
            name: "Lightning Gun".into(), damage: 75.0, damage_type: "Laser".into(),
            range: 300.0, reload_time: 30, projectile_speed: 0.0, area_of_effect: 0.0,
        }],
    ));
    registry.register(make(
        "corcom", 3000.0, "Heavy", 0.0, 0.0, 0, 1.2, Some(300.0),
        Some(0.5), Some(20.0),
        vec!["corsolar".into(), "cormex".into(), "corlab".into()],
        vec![recoil_sim::unit_defs::WeaponDefData {
            name: "Lightning Gun".into(), damage: 75.0, damage_type: "Laser".into(),
            range: 300.0, reload_time: 30, projectile_speed: 0.0, area_of_effect: 0.0,
        }],
    ));

    // Buildings — economy
    registry.register(make(
        "armsolar", 500.0, "Building", 150.0, 0.0, 120, 0.0,
        None, None, Some(20.0), vec![], vec![],
    ));
    registry.register(make(
        "corsolar", 500.0, "Building", 150.0, 0.0, 120, 0.0,
        None, None, Some(20.0), vec![], vec![],
    ));
    registry.register(make(
        "armmex", 600.0, "Building", 50.0, 500.0, 120, 0.0,
        None, Some(3.0), None, vec![], vec![],
    ));
    registry.register(make(
        "cormex", 600.0, "Building", 50.0, 500.0, 120, 0.0,
        None, Some(3.0), None, vec![], vec![],
    ));

    // Factories
    registry.register(make(
        "armlab", 4000.0, "Building", 650.0, 2800.0, 450, 0.0,
        Some(100.0), None, None,
        vec!["armpw".into(), "armrock".into(), "armham".into(), "armwar".into()],
        vec![],
    ));
    registry.register(make(
        "corlab", 4000.0, "Building", 650.0, 2800.0, 450, 0.0,
        Some(100.0), None, None,
        vec!["corak".into(), "corthud".into(), "corgator".into()],
        vec![],
    ));

    // Combat units
    for (name, hp, speed, metal, energy, bt, dmg, range, reload, proj_speed) in [
        ("armpw", 370.0, 2.9, 55.0, 900.0, 55, 9.0, 180.0, 9, 20.0),
        ("armrock", 680.0, 1.5, 130.0, 1500.0, 100, 120.0, 400.0, 60, 7.0),
        ("armham", 560.0, 1.5, 150.0, 2200.0, 120, 60.0, 600.0, 45, 5.0),
        ("armwar", 1000.0, 2.0, 200.0, 2800.0, 140, 15.0, 220.0, 10, 15.0),
        ("corak", 480.0, 2.7, 75.0, 800.0, 55, 11.0, 190.0, 10, 18.0),
        ("corthud", 600.0, 1.4, 120.0, 1600.0, 100, 110.0, 380.0, 60, 7.0),
        ("corgator", 760.0, 1.3, 170.0, 2500.0, 130, 70.0, 550.0, 50, 5.0),
    ] {
        registry.register(make(
            name, hp, "Light", metal, energy, bt, speed, None, None, None, vec![],
            vec![recoil_sim::unit_defs::WeaponDefData {
                name: "weapon".into(), damage: dmg, damage_type: "Normal".into(),
                range, reload_time: reload, projectile_speed: proj_speed, area_of_effect: 0.0,
            }],
        ));
    }

    // Resolve cross-references (commander can_build → factory → units)
    registry.resolve_build_options();
}

#[cfg(test)]
mod tests {
    use super::*;

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
            eprintln!("Skipping test_load_bar_unit_defs: BAR sandbox repo not found at {}", bar_path.display());
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

    #[test]
    fn test_spawn_commander_fallback() {
        let mut world = World::new();
        sim_runner::init_sim_world(&mut world);
        let registry = UnitDefRegistry::default();
        let weapon_ids = BTreeMap::new();

        let entity = spawn_commander(&mut world, &registry, &weapon_ids, (100.0, 100.0), 0);
        assert!(world.get::<Health>(entity).is_some());
        assert!(world.get::<Builder>(entity).is_some());
        assert!(world.get::<ResourceProducer>(entity).is_some());
        assert!(world.get::<MoveState>(entity).is_some());
    }
}
