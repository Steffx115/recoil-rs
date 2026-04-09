//! World initialization: economy, fog, registries, commander spawning.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};
use pierce_sim::commands::CommandQueue;
use pierce_sim::construction::Builder;
use pierce_sim::economy::{init_economy, ResourceProducer};
use pierce_sim::factory::{UnitBlueprint, UnitRegistry};
use pierce_sim::fog::FogOfWar;
use pierce_sim::lifecycle::spawn_unit;
use pierce_sim::map::MapManifest;
use pierce_sim::targeting::WeaponRegistry;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SightRange,
    Target, UnitType, Velocity,
};

use super::map_loader::MapData;
use super::GameConfig;

/// Initialize the world with economy, fog, registries, and commanders.
///
/// Consumes the loaded `UnitDefRegistry` and `MapData` and populates the
/// ECS world with all resources and starting entities.
/// Options for world initialization.
pub struct InitOptions {
    /// Enable fog of war. Default: `true`.
    pub fog_of_war: bool,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self { fog_of_war: true }
    }
}

pub fn init_world_with_options(
    world: &mut World,
    unit_def_registry: UnitDefRegistry,
    map_data: &MapData,
    options: InitOptions,
) -> GameConfig {
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

    init_economy(world, &[0, 1]);

    // Metal spots resource (for linking mex buildings to map spots)
    {
        let spots = if let Some(ref manifest) = map_data.manifest {
            manifest
                .metal_spots
                .iter()
                .map(|ms| pierce_sim::map::MetalSpot {
                    x: ms.x,
                    z: ms.z,
                    metal_per_tick: ms.metal_per_tick,
                })
                .collect()
        } else {
            Vec::new()
        };
        world.insert_resource(pierce_sim::map::MetalSpots { spots });
    }

    // Fog of War — size to match map extent. sim_tick skips fog when the
    // resource is absent, so omitting it disables fog entirely.
    if options.fog_of_war {
        let (fog_w, fog_h) = if let Some(ref manifest) = map_data.manifest {
            let w = manifest.width as u32 * manifest.cell_size as u32;
            let h = manifest.height as u32 * manifest.cell_size as u32;
            (w.max(1024), h.max(1024))
        } else {
            (1024, 1024)
        };
        let fog = FogOfWar::new(fog_w, fog_h, &[0, 1]);
        world.insert_resource(fog);
    }

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
    let (start_pos_0, start_pos_1) = start_positions(map_data.manifest.as_ref());

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
        metal_spots: map_data.metal_spots.clone(),
        commander_team0,
        commander_team1,
    }
}

/// Determine team start positions from a map manifest (or use defaults).
fn start_positions(manifest: Option<&MapManifest>) -> ((f32, f32), (f32, f32)) {
    if let Some(manifest) = manifest {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pierce_sim::sim_runner;

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
