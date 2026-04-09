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

    // Batch-collect candidate data upfront to avoid per-candidate world queries.
    // This is a flat Vec indexed by Entity bits via a BTreeMap for O(log n) lookup,
    // but populated once instead of once-per-shooter.
    struct CandidateInfo {
        team: u8,
        is_dead: bool,
        health_positive: bool,
        pos_xz: SimVec2,
        sim_id: u64,
    }

    let mut candidate_data: std::collections::BTreeMap<u64, CandidateInfo> =
        std::collections::BTreeMap::new();

    let mut cand_query = world.query::<(
        Entity,
        &Position,
        &Allegiance,
        Option<&Dead>,
        Option<&Health>,
        Option<&SimId>,
    )>();
    for (entity, pos, allegiance, dead, health, sim_id) in cand_query.iter(world) {
        candidate_data.insert(
            entity.to_bits(),
            CandidateInfo {
                team: allegiance.team,
                is_dead: dead.is_some(),
                health_positive: health.is_some_and(|h| h.current > 0),
                pos_xz: SimVec2::new(pos.pos.x, pos.pos.z),
                sim_id: sim_id.map_or(u64::MAX, |s| s.id),
            },
        );
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

            let Some(info) = candidate_data.get(&candidate.to_bits()) else {
                continue;
            };

            // Must be enemy.
            if info.team == shooter.team {
                continue;
            }

            // Must not be Dead.
            if info.is_dead {
                continue;
            }

            // Must be alive (Health.current > 0).
            if !info.health_positive {
                continue;
            }

            let dist_sq = shooter.pos_xz.distance_squared(info.pos_xz);

            match &best {
                Some((best_dist, best_id, _)) => {
                    if dist_sq < *best_dist || (dist_sq == *best_dist && info.sim_id < *best_id) {
                        best = Some((dist_sq, info.sim_id, *candidate));
                    }
                }
                None => {
                    best = Some((dist_sq, info.sim_id, *candidate));
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
#[path = "tests/targeting_tests.rs"]
mod tests;
