//! Building placement and finalization logic.
//!
//! All building costs, stats, and capabilities are now driven by [`UnitDef`]
//! data loaded from BAR Lua files or fallback RON assets.  The synthetic
//! building ID constants are retained only as convenience aliases for the
//! fallback RON defs (which use sequential IDs).

use std::collections::BTreeMap;
use std::collections::VecDeque;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};
use pierce_sim::commands::CommandQueue;
use pierce_sim::construction::{BuildSite, BuildTarget, Builder};
use pierce_sim::economy::{EconomyState, ResourceProducer};
use pierce_sim::factory::BuildQueue;
use pierce_sim::footprint::{footprint_cells, mark_building_footprint};
use pierce_sim::pathfinding::TerrainGrid;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{
    Allegiance, CollisionRadius, Dead, Heading, Health, MoveState, MovementParams, Position,
    SightRange, SimVec2, Target, UnitType, Velocity,
};

// ---------------------------------------------------------------------------
// Legacy building ID constants (for fallback RON defs)
// ---------------------------------------------------------------------------

/// UnitType ID for solar (FNV hash of "armsolar").
pub const BUILDING_SOLAR_ID: u32 = 789_715_014;
/// UnitType ID for mex (FNV hash of "armmex").
pub const BUILDING_MEX_ID: u32 = 3_027_733_053;
/// UnitType ID for bot factory (FNV hash of "armlab").
pub const BUILDING_FACTORY_ID: u32 = 357_525_636;

// ---------------------------------------------------------------------------
// PlacementType — now data-driven (wraps a unit_type_id)
// ---------------------------------------------------------------------------

/// A building type that can be placed.  Wraps the `unit_type_id` from the
/// [`UnitDefRegistry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementType(pub u32);

impl PlacementType {
    /// Human-readable label for this building type.
    pub fn label(self, registry: &UnitDefRegistry) -> String {
        registry
            .get(self.0)
            .map(|d| format!("Build {}", d.name))
            .unwrap_or_else(|| format!("Build #{}", self.0))
    }
}

// ---------------------------------------------------------------------------
// Building placement (data-driven)
// ---------------------------------------------------------------------------

/// Place a building: spawn BuildSite and assign a builder entity.
///
/// Looks up costs and stats from the [`UnitDefRegistry`].
/// Returns the build site entity, or `None` if the def is missing or the
/// team cannot afford it.
pub fn place_building(
    world: &mut World,
    builder_entity: Option<Entity>,
    unit_type_id: u32,
    x: f32,
    z: f32,
    team: u8,
) -> Option<Entity> {
    // Look up costs from the UnitDefRegistry.
    let (metal_cost, energy_cost, build_time, max_health, collision_r) = {
        let registry = world.resource::<UnitDefRegistry>();
        let def = registry.get(unit_type_id)?;
        (
            SimFloat::from_f64(def.metal_cost),
            SimFloat::from_f64(def.energy_cost),
            SimFloat::from_f64(def.build_time as f64),
            SimFloat::from_f64(def.max_health),
            SimFloat::from_f64(def.collision_radius),
        )
    };

    // Check if team can afford it.
    let can_afford = {
        let economy = world.resource::<EconomyState>();
        if let Some(res) = economy.teams.get(&team) {
            res.metal >= metal_cost && res.energy >= energy_cost
        } else {
            false
        }
    };

    if !can_afford {
        tracing::info!("Cannot afford building type {}", unit_type_id);
        return None;
    }

    // Check if the footprint location is free (no overlapping buildings or
    // impassable terrain).  Only validate cells that fall within the terrain
    // grid — positions outside the grid are unconstrained.
    {
        let building_pos = SimVec2::new(SimFloat::from_f32(x), SimFloat::from_f32(z));
        let grid = world.resource::<TerrainGrid>();
        let cells = footprint_cells(grid, building_pos, collision_r);
        for &(cx, cy) in &cells {
            if !grid.is_passable(cx, cy) {
                tracing::info!(
                    "Cannot place building type {} at ({:.0}, {:.0}): cell ({}, {}) blocked",
                    unit_type_id,
                    x,
                    z,
                    cx,
                    cy,
                );
                return None;
            }
        }
    }

    // Spawn the build site entity.
    let build_site_entity = world
        .spawn((
            Position {
                pos: SimVec3::new(SimFloat::from_f32(x), SimFloat::ZERO, SimFloat::from_f32(z)),
            },
            BuildSite {
                metal_cost,
                energy_cost,
                total_build_time: build_time,
                progress: SimFloat::ZERO,
            },
            Health {
                current: SimFloat::from_int(1),
                max: max_health,
            },
            Allegiance { team },
            UnitType { id: unit_type_id },
            CollisionRadius {
                radius: collision_r.max(SimFloat::from_int(8)),
            },
        ))
        .id();

    // Mark building footprint on the terrain grid.
    {
        let building_pos = SimVec2::new(SimFloat::from_f32(x), SimFloat::from_f32(z));
        let mut grid = world.resource_mut::<TerrainGrid>();
        let footprint = mark_building_footprint(&mut grid, building_pos, collision_r);
        world.entity_mut(build_site_entity).insert(footprint);
    }

    // Assign the builder.
    if let Some(cmd) = builder_entity {
        if world.get_entity(cmd).is_ok() {
            if world.get::<BuildTarget>(cmd).is_some() {
                *world.get_mut::<BuildTarget>(cmd).unwrap() = BuildTarget {
                    target: build_site_entity,
                };
            } else {
                world.entity_mut(cmd).insert(BuildTarget {
                    target: build_site_entity,
                });
            }

            if world.get::<MoveState>(cmd).is_some() {
                *world.get_mut::<MoveState>(cmd).unwrap() = MoveState::MovingTo(SimVec3::new(
                    SimFloat::from_f32(x),
                    SimFloat::ZERO,
                    SimFloat::from_f32(z),
                ));
            }
        }
    }

    tracing::info!(
        "Team {} placed building type {} at ({:.0}, {:.0})",
        team,
        unit_type_id,
        x,
        z
    );
    Some(build_site_entity)
}

// ---------------------------------------------------------------------------
// Building finalization (data-driven)
// ---------------------------------------------------------------------------

/// After construction_system completes a building (removes BuildSite),
/// add the appropriate functional components based on the [`UnitDef`].
///
/// Returns the entity of the first factory finalized for team 1 (for AI
/// tracking), if any.
pub fn finalize_completed_buildings(world: &mut World) -> Option<Entity> {
    // Find entities that look like completed buildings: have UnitType but
    // no BuildSite, no MoveState, no ResourceProducer, no BuildQueue, not
    // Dead, and not a Builder already.
    let candidates: Vec<(Entity, u32, u8, f32, f32)> = world
        .query_filtered::<(Entity, &UnitType, &Allegiance, &Position), (
            Without<BuildSite>,
            Without<MoveState>,
            Without<ResourceProducer>,
            Without<BuildQueue>,
            Without<Dead>,
            Without<Builder>,
        )>()
        .iter(world)
        .map(|(e, ut, a, p)| (e, ut.id, a.team, p.pos.x.to_f32(), p.pos.z.to_f32()))
        .collect();

    // Filter to only buildings using the registry.
    let building_candidates: Vec<_> = {
        let registry = world.resource::<UnitDefRegistry>();
        candidates
            .into_iter()
            .filter(|(_, id, _, _, _)| registry.get(*id).map(|d| d.is_building).unwrap_or(false))
            .collect()
    };

    let mut new_team1_factory = None;

    for (entity, unit_type_id, team, x, z) in building_candidates {
        let (energy_prod, metal_prod, can_build, build_power) = {
            let registry = world.resource::<UnitDefRegistry>();
            if let Some(def) = registry.get(unit_type_id) {
                (
                    def.energy_production,
                    def.metal_production,
                    def.can_build.clone(),
                    def.build_power,
                )
            } else {
                continue;
            }
        };

        // Add ResourceProducer if this building produces resources.
        let has_production = energy_prod.is_some() || metal_prod.is_some();
        if has_production {
            // For metal extractors, use the nearest metal spot's value instead
            // of the flat rate from the UnitDef.
            let actual_metal = if metal_prod.is_some() && energy_prod.is_none() {
                // Looks like a pure metal producer (mex) — link to nearest spot.
                world
                    .get_resource::<pierce_sim::map::MetalSpots>()
                    .and_then(|spots| spots.nearest(x as f64, z as f64, 30.0))
                    .map(|spot| spot.metal_per_tick)
                    .unwrap_or(metal_prod.unwrap_or(0.0))
            } else {
                metal_prod.unwrap_or(0.0)
            };

            world.entity_mut(entity).insert(ResourceProducer {
                metal_per_tick: SimFloat::from_f64(actual_metal),
                energy_per_tick: SimFloat::from_f64(energy_prod.unwrap_or(0.0)),
            });
            tracing::info!(
                "Team {} economy building (type {}) completed at ({:.0}, {:.0}), metal={:.1}/tick",
                team,
                unit_type_id,
                x,
                z,
                actual_metal
            );
        }

        // Add BuildQueue if this is a factory (has can_build entries).
        if !can_build.is_empty() {
            let rally = SimVec3::new(
                SimFloat::from_f32(x + 30.0),
                SimFloat::ZERO,
                SimFloat::from_f32(z),
            );
            world.entity_mut(entity).insert(BuildQueue {
                queue: VecDeque::new(),
                current_progress: SimFloat::ZERO,
                rally_point: rally,
                repeat: false,
            });
            tracing::info!(
                "Team {} factory (type {}) completed at ({:.0}, {:.0})",
                team,
                unit_type_id,
                x,
                z
            );

            if team == 1 {
                new_team1_factory = Some(entity);
            }
        }

        // Add Builder if this building has build_power (e.g., nano towers).
        if let Some(bp) = build_power {
            if can_build.is_empty() {
                // Only add Builder if it's not a factory (factories get BuildQueue).
                world.entity_mut(entity).insert(Builder {
                    build_power: SimFloat::from_f64(bp),
                });
            }
        }
    }

    new_team1_factory
}

// ---------------------------------------------------------------------------
// Equip factory-spawned units
// ---------------------------------------------------------------------------

/// Equip newly factory-spawned units with movement and combat components.
///
/// Factory system only spawns with (SimId, Position, UnitType, Allegiance, Health).
/// This function adds MoveState, MovementParams, weapons, etc.
/// Only equips non-building units (mobile units).
pub fn equip_factory_spawned_units(world: &mut World, weapon_def_ids: &BTreeMap<u32, Vec<u32>>) {
    let to_equip: Vec<(Entity, u32, u8)> = world
        .query_filtered::<(Entity, &UnitType, &Allegiance), (
            Without<MoveState>,
            Without<BuildSite>,
            Without<Dead>,
            Without<Builder>,
            Without<BuildQueue>,
            Without<ResourceProducer>,
        )>()
        .iter(world)
        .map(|(e, ut, a)| (e, ut.id, a.team))
        .collect();

    // Filter out buildings using the registry.
    let to_equip: Vec<_> = {
        let registry = world.resource::<UnitDefRegistry>();
        to_equip
            .into_iter()
            .filter(|(_, id, _)| {
                registry.get(*id).map(|d| !d.is_building).unwrap_or(false) // skip unknown units
            })
            .collect()
    };

    for (entity, unit_type_id, _team) in to_equip {
        let (stats, builder_info) = {
            let registry = world.resource::<UnitDefRegistry>();
            let s = registry.defs.get(&unit_type_id).map(|def| {
                (
                    SimFloat::from_f64(def.max_speed),
                    SimFloat::from_f64(def.acceleration),
                    SimFloat::from_f64(def.turn_rate),
                    SimFloat::from_f64(def.collision_radius),
                    SimFloat::from_f64(def.sight_range),
                    def.parse_armor_class(),
                )
            });
            // Extract builder info if this is a construction bot.
            let b = registry.defs.get(&unit_type_id).and_then(|def| {
                if def.is_builder {
                    Some(SimFloat::from_f64(def.build_power.unwrap_or(100.0)))
                } else {
                    None
                }
            });
            (s, b)
        };

        let (max_speed, accel, turn_rate, collision_r, sight_r, armor_class) = stats.unwrap_or((
            SimFloat::from_int(2),
            SimFloat::ONE,
            SimFloat::PI / SimFloat::from_int(30),
            SimFloat::from_int(8),
            SimFloat::from_int(80),
            ArmorClass::Light,
        ));

        let weapon_ids = weapon_def_ids
            .get(&unit_type_id)
            .cloned()
            .unwrap_or_default();

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
        ));

        // Construction bots get the Builder component so they can build.
        if let Some(build_power) = builder_info {
            world.entity_mut(entity).insert(Builder { build_power });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pierce_sim::economy::init_economy;
    use pierce_sim::sim_runner;

    fn setup_world_with_economy() -> World {
        let mut world = World::new();
        sim_runner::init_sim_world(&mut world);
        // Override the default 64x64 grid with a larger one so test
        // coordinates (up to ~200) are within bounds.
        world.insert_resource(pierce_sim::pathfinding::TerrainGrid::new(
            256,
            256,
            SimFloat::ONE,
        ));
        init_economy(&mut world, &[0, 1]);
        // Give team 0 plenty of resources for tests
        {
            let mut economy = world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::from_int(10000);
                res.energy = SimFloat::from_int(50000);
            }
        }
        // Insert a minimal UnitDefRegistry with a solar building def
        let mut registry = UnitDefRegistry::default();
        registry.register(pierce_sim::unit_defs::UnitDef {
            name: "testsolar".into(),
            unit_type_id: BUILDING_SOLAR_ID,
            max_health: 500.0,
            armor_class: "Building".into(),
            sight_range: 200.0,
            collision_radius: 32.0,
            max_speed: 0.0,
            acceleration: 0.0,
            turn_rate: 0.0,
            metal_cost: 150.0,
            energy_cost: 0.0,
            build_time: 120,
            weapons: vec![],
            model_path: None,
            icon_path: None,
            categories: vec![],
            can_build: vec![],
            can_build_names: vec![],
            build_power: None,
            metal_production: None,
            energy_production: Some(20.0),
            is_building: true,
            is_builder: false,
        });
        world.insert_resource(registry);
        world
    }

    /// Assert the terrain grid covers a building at `(x, z)` with `radius`.
    fn assert_world_grid_covers(world: &World, x: f32, z: f32, radius: f32) {
        let grid = world.resource::<TerrainGrid>();
        let max_x = (x + radius).ceil() as usize;
        let max_z = (z + radius).ceil() as usize;
        assert!(
            max_x < grid.width() && max_z < grid.height(),
            "Terrain grid {}x{} too small for ({x}, {z}) r={radius} (needs {}x{})",
            grid.width(),
            grid.height(),
            max_x + 1,
            max_z + 1,
        );
    }

    #[test]
    fn test_place_building_solar() {
        let mut world = setup_world_with_economy();
        assert_world_grid_covers(&world, 100.0, 100.0, 32.0);
        let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(result.is_some());
        let entity = result.unwrap();
        assert!(world.get::<BuildSite>(entity).is_some());
        assert_eq!(world.get::<UnitType>(entity).unwrap().id, BUILDING_SOLAR_ID);
    }

    #[test]
    fn test_place_building_cannot_afford() {
        let mut world = World::new();
        sim_runner::init_sim_world(&mut world);
        init_economy(&mut world, &[0, 1]);
        // Drain resources
        {
            let mut economy = world.resource_mut::<EconomyState>();
            if let Some(res) = economy.teams.get_mut(&0) {
                res.metal = SimFloat::ZERO;
                res.energy = SimFloat::ZERO;
            }
        }
        // Insert a minimal registry
        let mut registry = UnitDefRegistry::default();
        registry.register(pierce_sim::unit_defs::UnitDef {
            name: "testsolar".into(),
            unit_type_id: BUILDING_SOLAR_ID,
            max_health: 500.0,
            armor_class: "Building".into(),
            sight_range: 200.0,
            collision_radius: 32.0,
            max_speed: 0.0,
            acceleration: 0.0,
            turn_rate: 0.0,
            metal_cost: 150.0,
            energy_cost: 0.0,
            build_time: 120,
            weapons: vec![],
            model_path: None,
            icon_path: None,
            categories: vec![],
            can_build: vec![],
            can_build_names: vec![],
            build_power: None,
            metal_production: None,
            energy_production: Some(20.0),
            is_building: true,
            is_builder: false,
        });
        world.insert_resource(registry);

        let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_place_building_blocked_by_existing_building() {
        let mut world = setup_world_with_economy();
        assert_world_grid_covers(&world, 100.0, 100.0, 32.0);

        // Place first building — should succeed.
        let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(first.is_some(), "first building should succeed");

        // Place second building at the same spot — should fail (footprint overlap).
        let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(second.is_none(), "overlapping building should be rejected");
    }

    #[test]
    fn test_place_building_blocked_by_partial_overlap() {
        let mut world = setup_world_with_economy();
        assert_world_grid_covers(&world, 110.0, 110.0, 32.0);

        // Place first building at (100, 100) with collision_radius=32.
        let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(first.is_some());

        // Place second building slightly offset — still overlapping the footprint.
        let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 110.0, 110.0, 0);
        assert!(
            second.is_none(),
            "partially overlapping building should be rejected"
        );
    }

    #[test]
    fn test_place_building_adjacent_succeeds() {
        let mut world = setup_world_with_economy();
        assert_world_grid_covers(&world, 200.0, 200.0, 32.0);

        // Place first building at (100, 100) with collision_radius=32.
        // Footprint covers roughly (68..132, 68..132).
        let first = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(first.is_some());

        // Place second building far enough away that footprints don't overlap.
        let second = place_building(&mut world, None, BUILDING_SOLAR_ID, 200.0, 200.0, 0);
        assert!(second.is_some(), "non-overlapping building should succeed");
    }

    #[test]
    fn test_place_building_blocked_by_impassable_terrain() {
        let mut world = setup_world_with_economy();
        assert_world_grid_covers(&world, 100.0, 100.0, 32.0);

        // Manually mark a region as impassable (cliff).
        {
            let mut grid = world.resource_mut::<TerrainGrid>();
            for y in 95..=105 {
                for x in 95..=105 {
                    grid.set(x, y, SimFloat::ZERO);
                }
            }
        }

        // Try to place building on the cliff — should fail.
        let result = place_building(&mut world, None, BUILDING_SOLAR_ID, 100.0, 100.0, 0);
        assert!(
            result.is_none(),
            "building on impassable terrain should be rejected"
        );
    }

    #[test]
    fn test_finalize_completed_solar() {
        let mut world = setup_world_with_economy();

        // Manually spawn a "completed" solar: has building UnitType but no BuildSite
        world.spawn((
            Position {
                pos: SimVec3::new(
                    SimFloat::from_f32(50.0),
                    SimFloat::ZERO,
                    SimFloat::from_f32(50.0),
                ),
            },
            Health {
                current: SimFloat::from_int(500),
                max: SimFloat::from_int(500),
            },
            Allegiance { team: 0 },
            UnitType {
                id: BUILDING_SOLAR_ID,
            },
            CollisionRadius {
                radius: SimFloat::from_int(16),
            },
        ));

        finalize_completed_buildings(&mut world);

        // Verify it now has a ResourceProducer
        let count = world
            .query_filtered::<&ResourceProducer, Without<Dead>>()
            .iter(&world)
            .count();
        assert!(
            count >= 1,
            "Expected at least 1 ResourceProducer after finalization"
        );
    }
}
