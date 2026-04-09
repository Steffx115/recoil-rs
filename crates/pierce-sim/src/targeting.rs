//! Weapon targeting and firing systems.
//!
//! Provides [`targeting_system`] (acquires the closest enemy target) and
//! [`reload_system`] (counts down weapon cooldowns and emits [`FireEvent`]s).
//!
//! Features:
//! - Terrain line-of-sight raycasting (RR-75)
//! - Fire modes: hold fire / return fire / fire at will (RR-76)
//! - Weapon priority targets and threat levels (RR-76)
//! - Manual target override and attack-move (RR-76)
//! - Minimum weapon range (RR-134)
//! - Turret rotation and firing arcs (RR-135)
//! - Overkill avoidance and fire coordination (RR-136)

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::combat_data::{WeaponCategory, WeaponDef, WeaponSet};
use crate::components::{
    Allegiance, AttackMove, Dead, FireMode, Health, Heading, LastAttacker, ManualTarget, Position,
    SimId, Target, TurretFacings,
};
use crate::fog::{is_entity_visible, FogOfWar};
use crate::map::HeightmapData;
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

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

/// Tracks pending (in-flight) damage per target entity for overkill avoidance.
///
/// Uses `BTreeMap` for deterministic iteration (no HashMap in sim code).
#[derive(Resource, Debug, Clone, Default)]
pub struct PendingDamage {
    /// Map from target entity bits to total pending damage.
    pub damage: BTreeMap<u64, SimFloat>,
}

impl PendingDamage {
    /// Record pending damage against a target.
    pub fn add(&mut self, target: Entity, amount: SimFloat) {
        let entry = self.damage.entry(target.to_bits()).or_insert(SimFloat::ZERO);
        *entry += amount;
    }

    /// Get total pending damage against a target.
    pub fn get(&self, target: Entity) -> SimFloat {
        self.damage
            .get(&target.to_bits())
            .copied()
            .unwrap_or(SimFloat::ZERO)
    }

    /// Clear all pending damage (call at start of each tick).
    pub fn clear(&mut self) {
        self.damage.clear();
    }
}

// ---------------------------------------------------------------------------
// Threat scoring (RR-76)
// ---------------------------------------------------------------------------

/// Threat level for target prioritization. Higher = more threatening.
fn threat_score(has_weapons: bool, is_building: bool) -> SimFloat {
    if has_weapons && !is_building {
        // Armed mobile unit: highest threat.
        SimFloat::from_int(3)
    } else if has_weapons && is_building {
        // Armed building (turret): medium threat.
        SimFloat::from_int(2)
    } else {
        // Unarmed / economy: lowest threat.
        SimFloat::ONE
    }
}

// ---------------------------------------------------------------------------
// Angle utilities
// ---------------------------------------------------------------------------

/// Normalize an angle to [-PI, PI].
fn normalize_angle(angle: SimFloat) -> SimFloat {
    let mut a = angle;
    // Iterative normalization (handles angles up to a few multiples of TAU).
    while a > SimFloat::PI {
        a -= SimFloat::TAU;
    }
    while a < SimFloat::ZERO - SimFloat::PI {
        a += SimFloat::TAU;
    }
    a
}

/// Calculate the angle from `from` to `to` position on the XZ plane.
fn angle_to_target(from_x: SimFloat, from_z: SimFloat, to_x: SimFloat, to_z: SimFloat) -> SimFloat {
    SimFloat::atan2(to_x - from_x, to_z - from_z)
}

// ---------------------------------------------------------------------------
// targeting_system
// ---------------------------------------------------------------------------

/// Acquires the best valid enemy target for every armed entity.
///
/// For each entity with (`Position`, `Allegiance`, `WeaponSet`, `Target`):
/// 1. Check fire mode: HoldFire skips auto-targeting, ReturnFire only targets attacker.
/// 2. Check manual target override first.
/// 3. Compute the maximum weapon range from its `WeaponSet`.
/// 4. Query the `SpatialGrid` for entities within that radius.
/// 5. Filter out allies, dead entities, entities with zero health.
/// 6. Check terrain LOS (unless weapon has indirect fire).
/// 7. Check min_range for each weapon.
/// 8. Score by weapon priority, threat level, then distance (ties broken by `SimId`).
/// 9. Apply overkill avoidance: skip targets with enough pending damage.
/// Entry point called by sim_tick_with with pre-cached capabilities.
pub fn targeting_system_with_caps(world: &mut World, caps: &crate::sim_runner::SimCapabilities) {
    if caps.has_compute_backends {
        targeting_system_with_backend(world);
    } else {
        targeting_system(world);
    }
}

pub fn targeting_system(world: &mut World) {

    // Use Arc snapshot from SimFrameData if available.
    let grid: std::sync::Arc<SpatialGrid> =
        if let Some(frame) = world.get_resource::<crate::frame_data::SimFrameData>() {
            if let Some(ref snap) = frame.grid_snapshot {
                snap.clone()
            } else {
                std::sync::Arc::new(world.resource::<SpatialGrid>().clone())
            }
        } else {
            std::sync::Arc::new(world.resource::<SpatialGrid>().clone())
        };
    let registry = world.resource::<WeaponRegistry>().clone();
    let heightmap = world.get_resource::<HeightmapData>().cloned();
    let fog = world.get_resource::<FogOfWar>().cloned();

    // Gather shooter info.
    struct ShooterInfo {
        entity: Entity,
        pos_x: SimFloat,
        pos_y: SimFloat,
        pos_z: SimFloat,
        pos_xz: SimVec2,
        team: u8,
        max_range: SimFloat,
        fire_mode: FireMode,
        last_attacker: Option<Entity>,
        #[allow(dead_code)]
        manual_target_pos: Option<(SimFloat, SimFloat, SimFloat)>,
        manual_target_entity: Option<Entity>,
        #[allow(dead_code)]
        has_attack_move: bool,
        /// Whether any weapon has indirect fire.
        has_indirect: bool,
        /// Weapon categories on this unit.
        weapon_categories: Vec<WeaponCategory>,
        /// Min range of each weapon.
        weapon_min_ranges: Vec<SimFloat>,
    }

    let mut shooters: Vec<ShooterInfo> = Vec::new();

    {
        let mut query_state = world.query::<(
            Entity,
            &Position,
            &Allegiance,
            &WeaponSet,
            &Target,
            Option<&FireMode>,
            Option<&LastAttacker>,
            Option<&ManualTarget>,
            Option<&AttackMove>,
        )>();
        for (entity, position, allegiance, weapon_set, _target, fire_mode, last_attacker, manual_target, attack_move) in query_state.iter(world) {
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

            let has_indirect = weapon_set.weapons.iter().any(|w| {
                registry
                    .defs
                    .get(w.def_id as usize)
                    .is_some_and(|def| def.indirect_fire)
            });

            let weapon_categories: Vec<WeaponCategory> = weapon_set
                .weapons
                .iter()
                .map(|w| {
                    registry
                        .defs
                        .get(w.def_id as usize)
                        .map_or(WeaponCategory::General, |def| def.category)
                })
                .collect();

            let weapon_min_ranges: Vec<SimFloat> = weapon_set
                .weapons
                .iter()
                .map(|w| {
                    registry
                        .defs
                        .get(w.def_id as usize)
                        .map_or(SimFloat::ZERO, |def| def.min_range)
                })
                .collect();

            let (mt_pos, mt_entity) = if let Some(mt) = manual_target {
                (mt.position.map(|p| (p.x, p.y, p.z)), mt.forced_entity)
            } else {
                (None, None)
            };

            shooters.push(ShooterInfo {
                entity,
                pos_x: position.pos.x,
                pos_y: position.pos.y,
                pos_z: position.pos.z,
                pos_xz: SimVec2::new(position.pos.x, position.pos.z),
                team: allegiance.team,
                max_range,
                fire_mode: fire_mode.copied().unwrap_or(FireMode::FireAtWill),
                last_attacker: last_attacker.and_then(|la| la.entity),
                manual_target_pos: mt_pos,
                manual_target_entity: mt_entity,
                has_attack_move: attack_move.is_some(),
                has_indirect,
                weapon_categories,
                weapon_min_ranges,
            });
        }
    }

    // Batch-collect candidate data upfront.
    struct CandidateInfo {
        team: u8,
        is_dead: bool,
        health_current: SimFloat,
        health_positive: bool,
        pos_x: SimFloat,
        pos_y: SimFloat,
        pos_z: SimFloat,
        pos_xz: SimVec2,
        sim_id: u64,
        has_weapons: bool,
        is_building: bool,
    }

    let mut candidate_data: BTreeMap<u64, CandidateInfo> = BTreeMap::new();

    {
        let mut cand_query = world.query::<(
            Entity,
            &Position,
            &Allegiance,
            Option<&Dead>,
            Option<&Health>,
            Option<&SimId>,
            Option<&WeaponSet>,
            Option<&crate::components::BuildingFootprint>,
        )>();
        for (entity, pos, allegiance, dead, health, sim_id, weapon_set, footprint) in cand_query.iter(world) {
            candidate_data.insert(
                entity.to_bits(),
                CandidateInfo {
                    team: allegiance.team,
                    is_dead: dead.is_some(),
                    health_current: SimFloat::from_int(health.map_or(0, |h| h.current)),
                    health_positive: health.is_some_and(|h| h.current > 0),
                    pos_x: pos.pos.x,
                    pos_y: pos.pos.y,
                    pos_z: pos.pos.z,
                    pos_xz: SimVec2::new(pos.pos.x, pos.pos.z),
                    sim_id: sim_id.map_or(u64::MAX, |s| s.id),
                    has_weapons: weapon_set.is_some_and(|ws| !ws.weapons.is_empty()),
                    is_building: footprint.is_some(),
                },
            );
        }
    }

    // Read pending damage for overkill avoidance.
    let pending = world
        .get_resource::<PendingDamage>()
        .cloned()
        .unwrap_or_default();

    // For each shooter, find the best target.
    let mut assignments: Vec<(Entity, Option<Entity>)> = Vec::with_capacity(shooters.len());

    for shooter in &shooters {
        // HoldFire: never auto-target (manual override checked by reload_system).
        if shooter.fire_mode == FireMode::HoldFire && shooter.manual_target_entity.is_none() {
            assignments.push((shooter.entity, None));
            continue;
        }

        // Manual target override: forced entity.
        if let Some(forced) = shooter.manual_target_entity {
            assignments.push((shooter.entity, Some(forced)));
            continue;
        }

        // ReturnFire: only target the last attacker if it's alive and in range.
        if shooter.fire_mode == FireMode::ReturnFire {
            if let Some(attacker) = shooter.last_attacker {
                if let Some(info) = candidate_data.get(&attacker.to_bits()) {
                    if !info.is_dead && info.health_positive && info.team != shooter.team {
                        let dist_sq = shooter.pos_xz.distance_squared(info.pos_xz);
                        if dist_sq <= shooter.max_range * shooter.max_range {
                            assignments.push((shooter.entity, Some(attacker)));
                            continue;
                        }
                    }
                }
            }
            assignments.push((shooter.entity, None));
            continue;
        }

        // FireAtWill (or AttackMove): find best target.
        let candidates = grid.units_in_radius(shooter.pos_xz, shooter.max_range);

        // Score: (priority_bonus, threat, -distance_sq, -sim_id) -- higher is better.
        let mut best: Option<(SimFloat, SimFloat, SimFloat, u64, Entity)> = None;

        for candidate in &candidates {
            if *candidate == shooter.entity {
                continue;
            }

            let Some(info) = candidate_data.get(&candidate.to_bits()) else {
                continue;
            };

            if info.team == shooter.team {
                continue;
            }
            if info.is_dead {
                continue;
            }
            if !info.health_positive {
                continue;
            }

            // Must be visible in fog of war (if fog exists).
            if let Some(ref fog) = fog {
                let cand_pos = SimVec3::new(info.pos_x, info.pos_y, info.pos_z);
                if !is_entity_visible(fog, shooter.team, cand_pos, SimFloat::ONE) {
                    continue;
                }
            }

            let dist_sq = shooter.pos_xz.distance_squared(info.pos_xz);

            // Check min_range: at least one weapon must be able to fire at this distance.
            let any_weapon_in_range = shooter
                .weapon_min_ranges
                .iter()
                .enumerate()
                .any(|(i, min_r)| {
                    let def_range = shooter
                        .weapon_categories
                        .get(i)
                        .map_or(shooter.max_range, |_| {
                            // Get actual range for this weapon from the category lookup.
                            // We need the actual range, not the category.
                            shooter.max_range
                        });
                    dist_sq >= *min_r * *min_r && dist_sq <= def_range * def_range
                });
            if !any_weapon_in_range {
                continue;
            }

            // Terrain LOS check (skip for indirect fire weapons).
            if !shooter.has_indirect {
                if let Some(ref hm) = heightmap {
                    if !hm.has_line_of_sight(
                        shooter.pos_x,
                        shooter.pos_y,
                        shooter.pos_z,
                        info.pos_x,
                        info.pos_y,
                        info.pos_z,
                    ) {
                        continue;
                    }
                }
            }

            // Overkill avoidance: skip if pending damage >= current health.
            let pending_dmg = pending.get(*candidate);
            if pending_dmg >= info.health_current {
                continue;
            }

            // Weapon category priority bonus.
            let priority_bonus = if shooter.weapon_categories.contains(&WeaponCategory::AntiAir)
                && !info.is_building
            {
                // Anti-air bonus for non-building targets (simplified: treat non-building as potential air).
                SimFloat::from_int(10)
            } else if shooter.weapon_categories.contains(&WeaponCategory::AntiArmor)
                && info.is_building
            {
                SimFloat::from_int(5)
            } else {
                SimFloat::ZERO
            };

            let threat = threat_score(info.has_weapons, info.is_building);

            // Negate dist_sq so closer targets score higher.
            let neg_dist = SimFloat::ZERO - dist_sq;

            match &best {
                Some((best_prio, best_threat, best_neg_dist, best_id, _)) => {
                    if priority_bonus > *best_prio
                        || (priority_bonus == *best_prio && threat > *best_threat)
                        || (priority_bonus == *best_prio
                            && threat == *best_threat
                            && neg_dist > *best_neg_dist)
                        || (priority_bonus == *best_prio
                            && threat == *best_threat
                            && neg_dist == *best_neg_dist
                            && info.sim_id < *best_id)
                    {
                        best = Some((priority_bonus, threat, neg_dist, info.sim_id, *candidate));
                    }
                }
                None => {
                    best = Some((priority_bonus, threat, neg_dist, info.sim_id, *candidate));
                }
            }
        }

        assignments.push((shooter.entity, best.map(|(_, _, _, _, e)| e)));
    }

    // Write back targets.
    for (entity, target_entity) in assignments {
        if let Some(mut target) = world.get_mut::<Target>(entity) {
            target.entity = target_entity;
        }
    }
}

/// Targeting via compute backend (CPU or GPU).
fn targeting_system_with_backend(world: &mut World) {
    use crate::compute::{ComputeBackends, TargetingCandidateInput, TargetingShooterInput};
    use crate::fog::FogOfWar;

    let registry = world.resource::<WeaponRegistry>().clone();
    let fog = world.get_resource::<FogOfWar>().cloned();
    let pending = world
        .get_resource::<PendingDamage>()
        .cloned()
        .unwrap_or_default();

    // Build candidate list: all entities with Position + Allegiance.
    let mut candidate_entities: Vec<Entity> = Vec::new();
    let mut candidates: Vec<TargetingCandidateInput> = Vec::new();

    {
        let mut q = world.query::<(
            Entity,
            &Position,
            &Allegiance,
            Option<&Dead>,
            Option<&Health>,
            Option<&SimId>,
            Option<&WeaponSet>,
            Option<&crate::components::BuildingFootprint>,
        )>();
        for (entity, pos, allegiance, dead, health, sim_id, weapon_set, footprint) in q.iter(world)
        {
            candidate_entities.push(entity);
            candidates.push(TargetingCandidateInput {
                pos_x_raw: pos.pos.x.raw(),
                pos_y_raw: pos.pos.y.raw(),
                pos_z_raw: pos.pos.z.raw(),
                team: allegiance.team,
                is_dead: dead.is_some(),
                health_raw: health.map_or(0, |h| (h.current as i64) << 32),
                sim_id: sim_id.map_or(u64::MAX, |s| s.id),
                has_weapons: weapon_set.is_some_and(|ws| !ws.weapons.is_empty()),
                is_building: footprint.is_some(),
                pending_damage_raw: pending.get(entity).raw(),
            });
        }
    }

    // Build entity-to-candidate-index map for manual target / last attacker lookup.
    let entity_to_idx: std::collections::BTreeMap<u64, i32> = candidate_entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.to_bits(), i as i32))
        .collect();

    // Build shooter list.
    let mut shooter_entities: Vec<Entity> = Vec::new();
    let mut shooters: Vec<TargetingShooterInput> = Vec::new();

    {
        let mut q = world.query::<(
            Entity,
            &Position,
            &Allegiance,
            &WeaponSet,
            &Target,
            Option<&crate::components::FireMode>,
            Option<&crate::components::LastAttacker>,
            Option<&crate::components::ManualTarget>,
        )>();
        for (entity, pos, allegiance, weapon_set, _target, fire_mode, last_attacker, manual_target)
            in q.iter(world)
        {
            let fm = fire_mode.copied().unwrap_or(crate::components::FireMode::FireAtWill);
            let fire_mode_u8 = match fm {
                crate::components::FireMode::FireAtWill => 0,
                crate::components::FireMode::ReturnFire => 1,
                crate::components::FireMode::HoldFire => 2,
            };

            let mut max_range_raw = 0i64;
            let mut has_indirect = false;
            let mut weapon_min_ranges = [0i64; 4];
            let mut weapon_count = 0u8;

            for (i, w) in weapon_set.weapons.iter().enumerate() {
                if let Some(def) = registry.defs.get(w.def_id as usize) {
                    if def.range.raw() > max_range_raw {
                        max_range_raw = def.range.raw();
                    }
                    if def.indirect_fire {
                        has_indirect = true;
                    }
                    if i < 4 {
                        weapon_min_ranges[i] = def.min_range.raw();
                        weapon_count = (i + 1) as u8;
                    }
                }
            }

            let manual_idx = manual_target
                .and_then(|mt| mt.forced_entity)
                .and_then(|e| entity_to_idx.get(&e.to_bits()).copied())
                .unwrap_or(-1);

            let attacker_idx = last_attacker
                .and_then(|la| la.entity)
                .and_then(|e| entity_to_idx.get(&e.to_bits()).copied())
                .unwrap_or(-1);

            shooter_entities.push(entity);
            shooters.push(TargetingShooterInput {
                index: shooter_entities.len() as u32 - 1,
                pos_x_raw: pos.pos.x.raw(),
                pos_y_raw: pos.pos.y.raw(),
                pos_z_raw: pos.pos.z.raw(),
                team: allegiance.team,
                max_range_raw,
                fire_mode: fire_mode_u8,
                has_indirect,
                manual_target_idx: manual_idx,
                last_attacker_idx: attacker_idx,
                weapon_min_ranges,
                weapon_count,
            });
        }
    }

    // Get fog data for compute backend.
    let (fog_grids, fog_width, fog_height, fog_cell_raw) = if let Some(ref f) = fog {
        (Some(f.grids_as_u8()), f.width(), f.height(), SimFloat::ONE.raw())
    } else {
        (None, 0, 0, 0)
    };

    // Dispatch to backend via resource_scope (avoids remove/re-insert).
    let results = world.resource_scope(|_world, mut backends: bevy_ecs::prelude::Mut<crate::compute::ComputeBackends>| {
        backends.targeting.compute_targets(
            &shooters,
            &candidates,
            fog_grids.as_ref(),
            fog_width,
            fog_height,
            fog_cell_raw,
        )
    });

    // Apply results: map candidate indices back to entities.
    for (i, &target_idx) in results.iter().enumerate() {
        let shooter_entity = shooter_entities[i];
        let target_entity = if target_idx >= 0 && (target_idx as usize) < candidate_entities.len() {
            Some(candidate_entities[target_idx as usize])
        } else {
            None
        };
        if let Some(mut target) = world.get_mut::<Target>(shooter_entity) {
            target.entity = target_entity;
        }
    }
}

// ---------------------------------------------------------------------------
// turret_system (RR-135)
// ---------------------------------------------------------------------------

/// Updates turret facings each tick, rotating toward the current target.
///
/// For each entity with `TurretFacings` and a valid `Target`, rotates each
/// weapon turret toward the target at the weapon's `turret_turn_rate`.
pub fn turret_system(world: &mut World) {
    let registry = world.resource::<WeaponRegistry>().clone();

    struct TurretInfo {
        entity: Entity,
        pos_x: SimFloat,
        pos_z: SimFloat,
        target_entity: Option<Entity>,
        /// Per-weapon def_id (pre-collected to avoid borrow conflicts).
        weapon_def_ids: Vec<u32>,
        facings_count: usize,
    }

    let mut infos: Vec<TurretInfo> = Vec::new();

    {
        let mut query_state =
            world.query::<(Entity, &Position, &WeaponSet, Option<&Target>, Option<&TurretFacings>)>();
        for (entity, pos, weapon_set, target, turret_facings) in query_state.iter(world) {
            if turret_facings.is_none() {
                continue;
            }
            infos.push(TurretInfo {
                entity,
                pos_x: pos.pos.x,
                pos_z: pos.pos.z,
                target_entity: target.and_then(|t| t.entity),
                weapon_def_ids: weapon_set.weapons.iter().map(|w| w.def_id).collect(),
                facings_count: turret_facings.map_or(0, |tf| tf.facings.len()),
            });
        }
    }

    // Collect target positions.
    let mut target_positions: BTreeMap<u64, (SimFloat, SimFloat)> = BTreeMap::new();
    {
        let mut pos_query = world.query::<(Entity, &Position)>();
        for (entity, pos) in pos_query.iter(world) {
            target_positions.insert(entity.to_bits(), (pos.pos.x, pos.pos.z));
        }
    }

    for info in &infos {
        let target_pos = info
            .target_entity
            .and_then(|e| target_positions.get(&e.to_bits()).copied());

        let Some(mut turret_facings) = world.get_mut::<TurretFacings>(info.entity) else {
            continue;
        };

        let count = info.weapon_def_ids.len().min(info.facings_count);
        for i in 0..count {
            let def_id = info.weapon_def_ids[i];
            let def = registry.defs.get(def_id as usize);
            let turn_rate = def.map_or(SimFloat::ZERO, |d| d.turret_turn_rate);

            if turn_rate == SimFloat::ZERO {
                // Instant aim: snap to target.
                if let Some((tx, tz)) = target_pos {
                    turret_facings.facings[i].facing =
                        angle_to_target(info.pos_x, info.pos_z, tx, tz);
                }
                continue;
            }

            if let Some((tx, tz)) = target_pos {
                let desired = angle_to_target(info.pos_x, info.pos_z, tx, tz);
                let current = turret_facings.facings[i].facing;
                let diff = normalize_angle(desired - current);

                if diff.abs() <= turn_rate {
                    turret_facings.facings[i].facing = desired;
                } else if diff > SimFloat::ZERO {
                    turret_facings.facings[i].facing =
                        normalize_angle(current + turn_rate);
                } else {
                    turret_facings.facings[i].facing =
                        normalize_angle(current - turn_rate);
                }
            }
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
///   check min_range, turret aim, firing arc before firing.
///   Reset the cooldown and push a [`FireEvent`] to [`FireEventQueue`].
/// - Records pending damage for overkill avoidance.
pub fn reload_system(world: &mut World) {
    let registry = world.resource::<WeaponRegistry>().clone();

    struct ReloadInfo {
        entity: Entity,
        target: Option<Entity>,
        weapon_count: usize,
        pos_x: SimFloat,
        pos_z: SimFloat,
        heading: pierce_math::Angle,
        /// Pre-collected turret facings (cloned to avoid borrow conflicts).
        turret_facings: Option<Vec<SimFloat>>,
    }

    let mut infos: Vec<ReloadInfo> = Vec::new();

    {
        let mut query_state =
            world.query::<(Entity, &WeaponSet, Option<&Target>, &Position, Option<&Heading>, Option<&TurretFacings>)>();
        for (entity, weapon_set, target, pos, heading, turret_facings) in query_state.iter(world) {
            infos.push(ReloadInfo {
                entity,
                target: target.and_then(|t| t.entity),
                weapon_count: weapon_set.weapons.len(),
                pos_x: pos.pos.x,
                pos_z: pos.pos.z,
                heading: heading.map_or(pierce_math::Angle::ZERO, |h| h.angle),
                turret_facings: turret_facings.map(|tf| tf.facings.iter().map(|f| f.facing).collect()),
            });
        }
    }

    // Collect target positions for distance/angle checks.
    let mut target_positions: BTreeMap<u64, (SimFloat, SimFloat)> = BTreeMap::new();
    {
        let mut pos_query = world.query::<(Entity, &Position)>();
        for (entity, pos) in pos_query.iter(world) {
            target_positions.insert(entity.to_bits(), (pos.pos.x, pos.pos.z));
        }
    }

    let mut fire_events: Vec<FireEvent> = Vec::new();
    let mut new_pending: Vec<(Entity, SimFloat)> = Vec::new();

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
                    let def = registry.defs.get(def_id as usize);

                    // Check min_range.
                    if let (Some(def), Some(&(tx, tz))) =
                        (def, target_positions.get(&target_entity.to_bits()))
                    {
                        let dx = tx - info.pos_x;
                        let dz = tz - info.pos_z;
                        let dist_sq = dx * dx + dz * dz;

                        // Min range check.
                        if def.min_range > SimFloat::ZERO
                            && dist_sq < def.min_range * def.min_range
                        {
                            continue;
                        }

                        // Firing arc check (RR-135).
                        if def.firing_arc > SimFloat::ZERO {
                            let angle_to =
                                angle_to_target(info.pos_x, info.pos_z, tx, tz);
                            let diff = normalize_angle(angle_to - info.heading.to_radians());
                            if diff.abs() > def.firing_arc {
                                continue;
                            }
                        }

                        // Turret aim check (RR-135).
                        if def.turret_turn_rate > SimFloat::ZERO {
                            if let Some(ref facings) = info.turret_facings {
                                if let Some(&turret_facing) = facings.get(i) {
                                    let angle_to =
                                        angle_to_target(info.pos_x, info.pos_z, tx, tz);
                                    let diff = normalize_angle(angle_to - turret_facing);
                                    let tolerance = if def.aim_tolerance > SimFloat::ZERO {
                                        def.aim_tolerance
                                    } else {
                                        // Default ~3 degrees.
                                        SimFloat::from_ratio(52, 1000)
                                    };
                                    if diff.abs() > tolerance {
                                        continue;
                                    }
                                }
                            }
                        }

                        let reload_time = def.reload_time;
                        weapon.reload_remaining = reload_time;

                        fire_events.push(FireEvent {
                            shooter: info.entity,
                            target: target_entity,
                            weapon_def_id: def_id,
                        });

                        // Record pending damage for overkill avoidance.
                        new_pending.push((target_entity, def.damage));
                    } else {
                        // No def found or no target position, fire anyway (legacy behavior).
                        let reload_time = def.map_or(1, |d| d.reload_time);
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
    }

    // Update pending damage resource.
    if let Some(mut pending) = world.get_resource_mut::<PendingDamage>() {
        for (target, dmg) in &new_pending {
            pending.add(*target, *dmg);
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
