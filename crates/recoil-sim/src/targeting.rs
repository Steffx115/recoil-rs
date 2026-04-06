//! Weapon targeting and firing systems.
//!
//! Provides [`targeting_system`] (acquires the closest enemy target) and
//! [`reload_system`] (counts down weapon cooldowns and emits [`FireEvent`]s).

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::combat_data::{WeaponDef, WeaponSet};
use crate::components::{Allegiance, Dead, Health, Position, SimId, Target};
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Registry of all weapon definitions, indexed by `WeaponInstance::def_id`.
#[derive(Resource, Debug, Clone)]
pub struct WeaponRegistry {
    pub defs: Vec<WeaponDef>,
}

/// A single fire event emitted when a weapon fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireEvent {
    pub shooter: Entity,
    pub target: Entity,
    pub weapon_def_id: u32,
}

/// Per-frame queue of fire events. Cleared before each targeting pass.
#[derive(Resource, Debug, Clone, Default)]
pub struct FireEventQueue {
    pub events: Vec<FireEvent>,
}

// ---------------------------------------------------------------------------
// targeting_system
// ---------------------------------------------------------------------------

/// Acquires the closest valid enemy target for every armed entity.
///
/// For each entity with (`Position`, `Allegiance`, `WeaponSet`, `Target`):
/// 1. Compute the maximum weapon range from its `WeaponSet`.
/// 2. Query the `SpatialGrid` for entities within that radius.
/// 3. Filter out allies, dead entities, and entities with zero health.
/// 4. Pick the closest candidate (by `distance_squared`; ties broken by `SimId`).
pub fn targeting_system(world: &mut World) {
    // Collect query data so we can mutate Target afterwards.
    let grid = world.resource::<SpatialGrid>().clone();
    let registry = world.resource::<WeaponRegistry>().clone();

    // Gather shooter info.
    struct ShooterInfo {
        entity: Entity,
        pos_xz: SimVec2,
        team: u8,
        max_range: SimFloat,
    }

    let mut shooters: Vec<ShooterInfo> = Vec::new();

    // First pass: read shooter data.
    let mut query_state = world.query::<(Entity, &Position, &Allegiance, &WeaponSet, &Target)>();
    for (entity, position, allegiance, weapon_set, _target) in query_state.iter(world) {
        let max_range = weapon_set
            .weapons
            .iter()
            .map(|w| {
                registry
                    .defs
                    .get(w.def_id as usize)
                    .map_or(SimFloat::ZERO, |def| def.range)
            })
            .max()
            .unwrap_or(SimFloat::ZERO);

        shooters.push(ShooterInfo {
            entity,
            pos_xz: SimVec2::new(position.pos.x, position.pos.z),
            team: allegiance.team,
            max_range,
        });
    }

    // For each shooter, find the best target.
    let mut assignments: Vec<(Entity, Option<Entity>)> = Vec::with_capacity(shooters.len());

    for shooter in &shooters {
        let candidates = grid.units_in_radius(shooter.pos_xz, shooter.max_range);

        let mut best: Option<(SimFloat, u64, Entity)> = None;

        for candidate in &candidates {
            // Skip self.
            if *candidate == shooter.entity {
                continue;
            }

            // Must have Allegiance and be enemy.
            let Some(allegiance) = world.get::<Allegiance>(*candidate) else {
                continue;
            };
            if allegiance.team == shooter.team {
                continue;
            }

            // Must not be Dead.
            if world.get::<Dead>(*candidate).is_some() {
                continue;
            }

            // Must be alive (Health.current > 0).
            let Some(health) = world.get::<Health>(*candidate) else {
                continue;
            };
            if health.current <= SimFloat::ZERO {
                continue;
            }

            // Distance for sorting.
            let Some(cand_pos) = world.get::<Position>(*candidate) else {
                continue;
            };
            let cand_xz = SimVec2::new(cand_pos.pos.x, cand_pos.pos.z);
            let dist_sq = shooter.pos_xz.distance_squared(cand_xz);

            // Tie-break by SimId (lower id wins for determinism).
            let sim_id = world.get::<SimId>(*candidate).map_or(u64::MAX, |s| s.id);

            match &best {
                Some((best_dist, best_id, _)) => {
                    if dist_sq < *best_dist || (dist_sq == *best_dist && sim_id < *best_id) {
                        best = Some((dist_sq, sim_id, *candidate));
                    }
                }
                None => {
                    best = Some((dist_sq, sim_id, *candidate));
                }
            }
        }

        assignments.push((shooter.entity, best.map(|(_, _, e)| e)));
    }

    // Write back targets.
    for (entity, target_entity) in assignments {
        if let Some(mut target) = world.get_mut::<Target>(entity) {
            target.entity = target_entity;
        }
    }
}

// ---------------------------------------------------------------------------
// reload_system
// ---------------------------------------------------------------------------

/// Decrements weapon cooldowns and fires ready weapons at valid targets.
///
/// For each entity with a `WeaponSet`:
/// - Decrement `reload_remaining` by 1 (saturating).
/// - If `reload_remaining == 0` **and** the entity has a valid `Target`,
///   reset the cooldown and push a [`FireEvent`] to [`FireEventQueue`].
pub fn reload_system(world: &mut World) {
    let registry = world.resource::<WeaponRegistry>().clone();

    // Gather entities that have weapons and a target.
    struct ReloadInfo {
        entity: Entity,
        target: Option<Entity>,
        weapon_count: usize,
    }

    let mut infos: Vec<ReloadInfo> = Vec::new();

    let mut query_state = world.query::<(Entity, &WeaponSet, Option<&Target>)>();
    for (entity, weapon_set, target) in query_state.iter(world) {
        infos.push(ReloadInfo {
            entity,
            target: target.and_then(|t| t.entity),
            weapon_count: weapon_set.weapons.len(),
        });
    }

    let mut fire_events: Vec<FireEvent> = Vec::new();

    for info in &infos {
        let Some(mut weapon_set) = world.get_mut::<WeaponSet>(info.entity) else {
            continue;
        };
        for i in 0..info.weapon_count {
            let weapon = &mut weapon_set.weapons[i];
            weapon.reload_remaining = weapon.reload_remaining.saturating_sub(1);

            if weapon.reload_remaining == 0 {
                if let Some(target_entity) = info.target {
                    let def_id = weapon.def_id;
                    let reload_time = registry
                        .defs
                        .get(def_id as usize)
                        .map_or(1, |d| d.reload_time);
                    weapon.reload_remaining = reload_time;
                    fire_events.push(FireEvent {
                        shooter: info.entity,
                        target: target_entity,
                        weapon_def_id: def_id,
                    });
                }
            }
        }
    }

    // Append events to the queue resource.
    world
        .resource_mut::<FireEventQueue>()
        .events
        .extend(fire_events);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat_data::{DamageType, WeaponInstance, WeaponSet};
    use crate::components::{Dead, Health, Position, SimId, Target};
    use crate::{SimFloat, SimVec2, SimVec3};

    fn sf(n: i32) -> SimFloat {
        SimFloat::from_int(n)
    }

    fn pos3(x: i32, y: i32, z: i32) -> Position {
        Position {
            pos: SimVec3::new(sf(x), sf(y), sf(z)),
        }
    }

    fn simple_weapon_def(range: i32, reload: u32) -> WeaponDef {
        WeaponDef {
            damage: sf(10),
            damage_type: DamageType::Normal,
            range: sf(range),
            reload_time: reload,
            projectile_speed: SimFloat::ZERO,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        }
    }

    fn weapon_instance(def_id: u32) -> WeaponInstance {
        WeaponInstance {
            def_id,
            reload_remaining: 0,
        }
    }

    /// Build a world with a spatial grid and weapon registry.
    fn setup_world(defs: Vec<WeaponDef>) -> World {
        let mut world = World::new();
        world.insert_resource(SpatialGrid::new(sf(10), 20, 20));
        world.insert_resource(WeaponRegistry { defs });
        world.insert_resource(FireEventQueue::default());
        world
    }

    /// Insert an entity into the spatial grid (XZ plane).
    fn grid_insert(world: &mut World, entity: Entity, x: i32, z: i32) {
        world
            .resource_mut::<SpatialGrid>()
            .insert(entity, SimVec2::new(sf(x), sf(z)));
    }

    // -----------------------------------------------------------------------
    // targeting_system tests
    // -----------------------------------------------------------------------

    #[test]
    fn targets_closest_enemy() {
        let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

        // Shooter at (10, 0, 10), team 1.
        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![weapon_instance(0)],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Close enemy at (12, 0, 10), team 2.
        let close_enemy = world
            .spawn((
                pos3(12, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 2 },
            ))
            .id();
        grid_insert(&mut world, close_enemy, 12, 10);

        // Far enemy at (30, 0, 10), team 2.
        let far_enemy = world
            .spawn((
                pos3(30, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 3 },
            ))
            .id();
        grid_insert(&mut world, far_enemy, 30, 10);

        targeting_system(&mut world);

        let target = world.get::<Target>(shooter).unwrap();
        assert_eq!(target.entity, Some(close_enemy));
    }

    #[test]
    fn ignores_allies() {
        let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![weapon_instance(0)],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Ally right next to shooter.
        let ally = world
            .spawn((
                pos3(11, 0, 10),
                Allegiance { team: 1 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 2 },
            ))
            .id();
        grid_insert(&mut world, ally, 11, 10);

        // Enemy further away.
        let enemy = world
            .spawn((
                pos3(20, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 3 },
            ))
            .id();
        grid_insert(&mut world, enemy, 20, 10);

        targeting_system(&mut world);

        let target = world.get::<Target>(shooter).unwrap();
        assert_eq!(target.entity, Some(enemy));
    }

    #[test]
    fn no_target_when_no_enemies_in_range() {
        let mut world = setup_world(vec![simple_weapon_def(5, 10)]);

        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![weapon_instance(0)],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Enemy way out of range.
        let enemy = world
            .spawn((
                pos3(90, 0, 90),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 2 },
            ))
            .id();
        grid_insert(&mut world, enemy, 90, 90);

        targeting_system(&mut world);

        let target = world.get::<Target>(shooter).unwrap();
        assert_eq!(target.entity, None);
    }

    #[test]
    fn weapons_reload_and_fire() {
        let mut world = setup_world(vec![simple_weapon_def(100, 3)]);

        // Enemy for the shooter to target.
        let enemy = world
            .spawn((
                pos3(12, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 2 },
            ))
            .id();
        grid_insert(&mut world, enemy, 12, 10);

        // Shooter with reload_remaining = 2 (will need 2 ticks to be ready).
        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![WeaponInstance {
                        def_id: 0,
                        reload_remaining: 2,
                    }],
                },
                Target {
                    entity: Some(enemy),
                },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Tick 1: 2 -> 1, should not fire.
        reload_system(&mut world);
        assert!(world.resource::<FireEventQueue>().events.is_empty());
        assert_eq!(
            world.get::<WeaponSet>(shooter).unwrap().weapons[0].reload_remaining,
            1
        );

        // Tick 2: 1 -> 0, should fire and reset to 3.
        reload_system(&mut world);
        let events = &world.resource::<FireEventQueue>().events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].shooter, shooter);
        assert_eq!(events[0].target, enemy);
        assert_eq!(events[0].weapon_def_id, 0);
        assert_eq!(
            world.get::<WeaponSet>(shooter).unwrap().weapons[0].reload_remaining,
            3
        );
    }

    #[test]
    fn determinism_same_distance_sorted_by_sim_id() {
        let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![weapon_instance(0)],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Two enemies at exactly the same distance but different SimIds.
        let enemy_a = world
            .spawn((
                pos3(15, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 100 },
            ))
            .id();
        grid_insert(&mut world, enemy_a, 15, 10);

        let enemy_b = world
            .spawn((
                pos3(10, 0, 15),
                Allegiance { team: 2 },
                Health {
                    current: sf(100),
                    max: sf(100),
                },
                SimId { id: 50 },
            ))
            .id();
        grid_insert(&mut world, enemy_b, 10, 15);

        // Both are distance 5 away. enemy_b has lower SimId (50 < 100).
        targeting_system(&mut world);

        let target = world.get::<Target>(shooter).unwrap();
        assert_eq!(target.entity, Some(enemy_b));

        // Run again to verify same result (determinism).
        targeting_system(&mut world);
        let target2 = world.get::<Target>(shooter).unwrap();
        assert_eq!(target2.entity, Some(enemy_b));
    }

    #[test]
    fn ignores_dead_entities() {
        let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

        let shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![weapon_instance(0)],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();
        grid_insert(&mut world, shooter, 10, 10);

        // Dead enemy (closer).
        let dead_enemy = world
            .spawn((
                pos3(11, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(0),
                    max: sf(100),
                },
                Dead,
                SimId { id: 2 },
            ))
            .id();
        grid_insert(&mut world, dead_enemy, 11, 10);

        // Live enemy (further).
        let live_enemy = world
            .spawn((
                pos3(20, 0, 10),
                Allegiance { team: 2 },
                Health {
                    current: sf(50),
                    max: sf(100),
                },
                SimId { id: 3 },
            ))
            .id();
        grid_insert(&mut world, live_enemy, 20, 10);

        targeting_system(&mut world);

        let target = world.get::<Target>(shooter).unwrap();
        assert_eq!(target.entity, Some(live_enemy));
    }

    #[test]
    fn no_fire_without_target() {
        let mut world = setup_world(vec![simple_weapon_def(100, 1)]);

        // Shooter with no target and a ready weapon.
        let _shooter = world
            .spawn((
                pos3(10, 0, 10),
                Allegiance { team: 1 },
                WeaponSet {
                    weapons: vec![WeaponInstance {
                        def_id: 0,
                        reload_remaining: 0,
                    }],
                },
                Target { entity: None },
                SimId { id: 1 },
            ))
            .id();

        reload_system(&mut world);

        assert!(world.resource::<FireEventQueue>().events.is_empty());
    }
}
