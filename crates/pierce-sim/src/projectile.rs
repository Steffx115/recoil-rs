//! Projectile simulation: spawning, movement, and impact detection.
//!
//! Consumes [`FireEvent`]s from [`FireEventQueue`] and either creates instant
//! impacts (beam weapons) or spawns projectile entities that travel toward
//! their targets over multiple frames.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::combat_data::DamageType;
use crate::components::{Position, Velocity};
use crate::targeting::{FireEventQueue, WeaponRegistry};
use crate::{SimFloat, SimVec3};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Gravity applied per frame to ballistic projectiles (negative Y).
const GRAVITY: SimFloat = SimFloat::from_ratio(1, 10);

/// Distance threshold below which a projectile counts as hitting its target.
/// Must be >= projectile speed to prevent overshoot oscillation.
const HIT_DISTANCE: SimFloat = SimFloat::from_int(10);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How a projectile travels through the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileType {
    /// Affected by gravity, does not track target after launch.
    Ballistic,
    /// Re-aims toward the target every frame.
    Homing,
    /// Instant hit (zero travel time). Handled at spawn time and never
    /// actually placed in the world as an entity.
    Beam,
}

/// Component attached to in-flight projectile entities.
#[derive(Component, Debug, Clone)]
pub struct Projectile {
    pub projectile_type: ProjectileType,
    pub target_entity: Entity,
    /// Last-known target position (used by ballistic projectiles that do not
    /// re-aim).
    pub target_pos: SimVec3,
    pub damage: SimFloat,
    pub damage_type: DamageType,
    pub area_of_effect: SimFloat,
    pub speed: SimFloat,
    pub is_paralyzer: bool,
    /// Frames remaining before the projectile self-destructs.
    pub lifetime: u32,
}

// ---------------------------------------------------------------------------
// Impact events
// ---------------------------------------------------------------------------

/// Produced when a projectile (or beam) hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactEvent {
    pub position: SimVec3,
    pub damage: SimFloat,
    pub damage_type: DamageType,
    pub area_of_effect: SimFloat,
    pub is_paralyzer: bool,
}

/// Per-frame queue of impact events.
#[derive(Resource, Debug, Clone, Default)]
pub struct ImpactEventQueue {
    pub events: Vec<ImpactEvent>,
}

// ---------------------------------------------------------------------------
// spawn_projectile_system
// ---------------------------------------------------------------------------

/// Reads [`FireEventQueue`], looks up weapon defs, and either creates an
/// instant [`ImpactEvent`] (beam / hitscan) or spawns a [`Projectile`] entity.
pub fn spawn_projectile_system(world: &mut World) {
    let fire_events: Vec<_> = world
        .resource_mut::<FireEventQueue>()
        .events
        .drain(..)
        .collect();

    let registry = world.resource::<WeaponRegistry>().clone();

    // Collect spawn data before mutating the world.
    struct SpawnInfo {
        shooter_pos: SimVec3,
        target_pos: SimVec3,
        target_entity: Entity,
        damage: SimFloat,
        damage_type: DamageType,
        area_of_effect: SimFloat,
        speed: SimFloat,
        is_paralyzer: bool,
    }

    let mut beams: Vec<ImpactEvent> = Vec::new();
    let mut spawns: Vec<SpawnInfo> = Vec::new();

    for event in &fire_events {
        let Some(def) = registry.defs.get(event.weapon_def_id as usize) else {
            continue;
        };

        let Some(shooter_pos) = world.get::<Position>(event.shooter) else {
            continue;
        };
        let shooter_pos = shooter_pos.pos;

        let Some(target_pos) = world.get::<Position>(event.target) else {
            continue;
        };
        let target_pos = target_pos.pos;

        if def.projectile_speed == SimFloat::ZERO {
            // Beam / hitscan: instant impact at target position.
            beams.push(ImpactEvent {
                position: target_pos,
                damage: def.damage,
                damage_type: def.damage_type,
                area_of_effect: def.area_of_effect,
                is_paralyzer: def.is_paralyzer,
            });
        } else {
            spawns.push(SpawnInfo {
                shooter_pos,
                target_pos,
                target_entity: event.target,
                damage: def.damage,
                damage_type: def.damage_type,
                area_of_effect: def.area_of_effect,
                speed: def.projectile_speed,
                is_paralyzer: def.is_paralyzer,
            });
        }
    }

    // Append beam impacts.
    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .extend(beams);

    // Spawn projectile entities.
    for info in spawns {
        let direction = (info.target_pos - info.shooter_pos).normalize();
        let vel = direction * info.speed;

        // Homing if speed > 0 (all non-beam projectiles default to homing;
        // a real game might derive this from weapon data, but the spec says
        // we only need Ballistic and Homing variants tested, so we use
        // Homing by default and let tests override via direct construction).
        let projectile_type = ProjectileType::Homing;

        world.spawn((
            Projectile {
                projectile_type,
                target_entity: info.target_entity,
                target_pos: info.target_pos,
                damage: info.damage,
                damage_type: info.damage_type,
                area_of_effect: info.area_of_effect,
                speed: info.speed,
                lifetime: 300,
                is_paralyzer: info.is_paralyzer,
            },
            Position {
                pos: info.shooter_pos,
            },
            Velocity { vel },
        ));
    }
}

// ---------------------------------------------------------------------------
// projectile_movement_system
// ---------------------------------------------------------------------------

/// Moves all in-flight projectiles and checks for impacts.
///
/// - **Ballistic**: moves along current velocity, applies gravity each frame.
/// - **Homing**: re-aims velocity toward the target's current position.
/// - If within [`HIT_DISTANCE`] of the target, creates an [`ImpactEvent`] and
///   despawns the projectile.
pub fn projectile_movement_system(world: &mut World) {
    // Gather projectile data.
    struct ProjInfo {
        entity: Entity,
        projectile_type: ProjectileType,
        target_entity: Entity,
        target_pos: SimVec3,
        damage: SimFloat,
        damage_type: DamageType,
        area_of_effect: SimFloat,
        speed: SimFloat,
        is_paralyzer: bool,
        lifetime: u32,
        pos: SimVec3,
        vel: SimVec3,
    }

    let mut infos: Vec<ProjInfo> = Vec::new();

    let mut query = world.query::<(Entity, &Projectile, &Position, &Velocity)>();
    for (entity, proj, pos, vel) in query.iter(world) {
        infos.push(ProjInfo {
            entity,
            projectile_type: proj.projectile_type,
            target_entity: proj.target_entity,
            target_pos: proj.target_pos,
            damage: proj.damage,
            damage_type: proj.damage_type,
            area_of_effect: proj.area_of_effect,
            speed: proj.speed,
            is_paralyzer: proj.is_paralyzer,
            lifetime: proj.lifetime,
            pos: pos.pos,
            vel: vel.vel,
        });
    }

    let mut impacts: Vec<ImpactEvent> = Vec::new();
    let mut despawns: Vec<Entity> = Vec::new();

    for info in &mut infos {
        match info.projectile_type {
            ProjectileType::Ballistic => {
                // Apply gravity (downward on Y axis).
                info.vel.y -= GRAVITY;
                // Move along velocity.
                info.pos += info.vel;
            }
            ProjectileType::Homing => {
                // Re-aim toward target's current position.
                let current_target_pos = world
                    .get::<Position>(info.target_entity)
                    .map(|p| p.pos)
                    .unwrap_or(info.target_pos);

                let direction = (current_target_pos - info.pos).normalize();
                info.vel = direction * info.speed;
                info.pos += info.vel;
                // Update target_pos for the check below.
                info.target_pos = current_target_pos;
            }
            ProjectileType::Beam => {
                // Beams are handled at spawn time; should never appear here.
                unreachable!("Beam projectiles should not exist as entities");
            }
        }

        // Decrement lifetime — despawn if expired (prevents infinite orbit).
        info.lifetime = info.lifetime.saturating_sub(1);
        if info.lifetime == 0 {
            despawns.push(info.entity);
            continue;
        }

        // Check hit distance against target position.
        let dist = info.pos.distance(info.target_pos);
        if dist < HIT_DISTANCE {
            impacts.push(ImpactEvent {
                position: info.pos,
                damage: info.damage,
                damage_type: info.damage_type,
                area_of_effect: info.area_of_effect,
                is_paralyzer: info.is_paralyzer,
            });
            despawns.push(info.entity);
        }
    }

    // Write back updated positions and velocities (only for non-despawned).
    for info in &infos {
        if despawns.contains(&info.entity) {
            continue;
        }
        if let Some(mut pos) = world.get_mut::<Position>(info.entity) {
            pos.pos = info.pos;
        }
        if let Some(mut vel) = world.get_mut::<Velocity>(info.entity) {
            vel.vel = info.vel;
        }
        if let Some(mut proj) = world.get_mut::<Projectile>(info.entity) {
            proj.lifetime = info.lifetime;
        }
    }

    // Append impacts.
    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .extend(impacts);

    // Despawn hit projectiles.
    for entity in despawns {
        world.despawn(entity);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "projectile_tests.rs"]
mod tests;
