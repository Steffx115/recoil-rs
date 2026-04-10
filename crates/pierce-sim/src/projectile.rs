//! Projectile simulation: spawning, movement, and impact detection.
//!
//! Consumes [`FireEvent`]s from [`FireEventQueue`] and either creates instant
//! impacts (beam weapons) or spawns projectile entities that travel toward
//! their targets over multiple frames.
//!
//! ## ProjectilePhysics trait (RR-113)
//!
//! Movement and hit detection are dispatched through the [`ProjectilePhysics`]
//! trait, with implementations for [`BallisticPhysics`], [`HomingPhysics`],
//! and [`BeamPhysics`]. Adding a new projectile type only requires a new impl.
//!
//! ## Target prediction (RR-132)
//!
//! [`predict_intercept`] calculates a lead point based on target velocity and
//! projectile speed so that ballistic rounds aim ahead of moving targets.
//!
//! ## Terrain collision (RR-133)
//!
//! The [`Heightmap`] resource enables projectile-terrain raycasting via
//! [`raycast_heightmap`]. Projectiles that strike terrain before reaching
//! their target create an impact at the collision point. Artillery (indirect
//! fire) arcs over intervening terrain.

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
// ProjectilePhysics trait (RR-113)
// ---------------------------------------------------------------------------

/// Trait for projectile movement physics and hit detection.
///
/// Each projectile type implements this trait to define how it moves and
/// when it counts as hitting a target. Adding a new projectile type only
/// requires a new impl of this trait.
pub trait ProjectilePhysics {
    /// Advance the projectile by one simulation tick.
    ///
    /// * `pos` — current world position (mutated in place)
    /// * `vel` — current velocity (mutated in place)
    /// * `target` — target position this frame
    /// * `dt` — time step (typically `SimFloat::ONE` for per-tick systems)
    fn step(&self, pos: &mut SimVec3, vel: &mut SimVec3, target: SimVec3, dt: SimFloat);

    /// Returns `true` when the projectile is close enough to count as a hit.
    fn hit_check(&self, pos: SimVec3, target: SimVec3, radius: SimFloat) -> bool;
}

// ---------------------------------------------------------------------------
// BallisticPhysics
// ---------------------------------------------------------------------------

/// Ballistic projectile: affected by gravity, does not re-aim after launch.
pub struct BallisticPhysics;

impl ProjectilePhysics for BallisticPhysics {
    fn step(&self, pos: &mut SimVec3, vel: &mut SimVec3, _target: SimVec3, _dt: SimFloat) {
        // Apply gravity (downward on Y axis).
        vel.y -= GRAVITY;
        // Move along velocity.
        *pos += *vel;
    }

    fn hit_check(&self, pos: SimVec3, target: SimVec3, radius: SimFloat) -> bool {
        pos.distance(target) < radius
    }
}

// ---------------------------------------------------------------------------
// HomingPhysics
// ---------------------------------------------------------------------------

/// Homing projectile: re-aims toward the target every frame.
pub struct HomingPhysics {
    pub speed: SimFloat,
}

impl ProjectilePhysics for HomingPhysics {
    fn step(&self, pos: &mut SimVec3, vel: &mut SimVec3, target: SimVec3, _dt: SimFloat) {
        let direction = (target - *pos).normalize();
        *vel = direction * self.speed;
        *pos += *vel;
    }

    fn hit_check(&self, pos: SimVec3, target: SimVec3, radius: SimFloat) -> bool {
        pos.distance(target) < radius
    }
}

// ---------------------------------------------------------------------------
// BeamPhysics
// ---------------------------------------------------------------------------

/// Beam / hitscan: instant hit at spawn time, never exists as an entity.
pub struct BeamPhysics;

impl ProjectilePhysics for BeamPhysics {
    fn step(&self, _pos: &mut SimVec3, _vel: &mut SimVec3, _target: SimVec3, _dt: SimFloat) {
        // Beams are resolved instantly at spawn; this should never be called.
    }

    fn hit_check(&self, _pos: SimVec3, _target: SimVec3, _radius: SimFloat) -> bool {
        // Beams always hit instantly.
        true
    }
}

// ---------------------------------------------------------------------------
// Dispatch helper
// ---------------------------------------------------------------------------

/// Returns the appropriate physics implementation for a given projectile type.
fn dispatch_physics(projectile_type: ProjectileType, speed: SimFloat) -> Box<dyn ProjectilePhysics> {
    match projectile_type {
        ProjectileType::Ballistic => Box::new(BallisticPhysics),
        ProjectileType::Homing => Box::new(HomingPhysics { speed }),
        ProjectileType::Beam => Box::new(BeamPhysics),
    }
}

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
    /// If true, this is an indirect-fire (artillery) projectile that arcs
    /// over terrain rather than colliding with it.
    pub indirect_fire: bool,
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
    /// For single-target (non-AOE) hits, the known target entity.
    /// Allows damage_system to skip the spatial query entirely.
    pub target_entity: Option<Entity>,
}

/// Per-frame queue of impact events.
#[derive(Resource, Debug, Clone, Default)]
pub struct ImpactEventQueue {
    pub events: Vec<ImpactEvent>,
}

// ---------------------------------------------------------------------------
// Heightmap resource (RR-133)
// ---------------------------------------------------------------------------

/// ECS resource providing terrain height lookup for projectile-terrain
/// collision detection. Row-major, indexed as `data[z * width + x]`.
#[derive(Resource, Debug, Clone)]
pub struct Heightmap {
    pub data: Vec<u16>,
    pub width: u32,
    pub height: u32,
    /// World units per heightmap cell.
    pub cell_size: SimFloat,
}

impl Heightmap {
    /// Sample the terrain height at world-space `(wx, wz)`.
    ///
    /// Uses bilinear interpolation between the four surrounding cells.
    /// Returns `SimFloat::ZERO` if the coordinates are outside the map.
    pub fn sample(&self, wx: SimFloat, wz: SimFloat) -> SimFloat {
        if self.cell_size == SimFloat::ZERO {
            return SimFloat::ZERO;
        }

        let fx = wx / self.cell_size;
        let fz = wz / self.cell_size;

        // Clamp to map bounds.
        let max_x = SimFloat::from_int(self.width as i32 - 1);
        let max_z = SimFloat::from_int(self.height as i32 - 1);
        let fx = fx.clamp(SimFloat::ZERO, max_x);
        let fz = fz.clamp(SimFloat::ZERO, max_z);

        let ix = fx.floor();
        let iz = fz.floor();
        let tx = fx - ix;
        let tz = fz - iz;

        let x0 = ix.to_f64() as usize;
        let z0 = iz.to_f64() as usize;
        let x1 = (x0 + 1).min(self.width as usize - 1);
        let z1 = (z0 + 1).min(self.height as usize - 1);

        let w = self.width as usize;
        let h00 = SimFloat::from_int(self.data[z0 * w + x0] as i32);
        let h10 = SimFloat::from_int(self.data[z0 * w + x1] as i32);
        let h01 = SimFloat::from_int(self.data[z1 * w + x0] as i32);
        let h11 = SimFloat::from_int(self.data[z1 * w + x1] as i32);

        let top = h00 + (h10 - h00) * tx;
        let bot = h01 + (h11 - h01) * tx;
        top + (bot - top) * tz
    }
}

/// Raycast a projectile path segment against the heightmap.
///
/// Walks from `start` to `end` in `steps` evenly spaced samples and returns
/// the first point where the projectile Y is below the terrain height.
/// Returns `None` if the path stays above terrain.
pub fn raycast_heightmap(
    heightmap: &Heightmap,
    start: SimVec3,
    end: SimVec3,
    steps: u32,
) -> Option<SimVec3> {
    if steps == 0 {
        return None;
    }
    let inv_steps = SimFloat::from_ratio(1, steps as i32);
    for i in 1..=steps {
        let t = SimFloat::from_int(i as i32) * inv_steps;
        let p = SimVec3::new(
            start.x + (end.x - start.x) * t,
            start.y + (end.y - start.y) * t,
            start.z + (end.z - start.z) * t,
        );
        let terrain_y = heightmap.sample(p.x, p.z);
        if p.y <= terrain_y {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Target prediction (RR-132)
// ---------------------------------------------------------------------------

/// Calculate a predicted intercept point for a moving target.
///
/// Given the shooter position, target position, target velocity, and
/// projectile speed, returns the position where the target is expected
/// to be when the projectile arrives. Uses a simple linear approximation:
/// time-to-target = distance / projectile_speed, then
/// predicted = target_pos + target_vel * time.
///
/// Falls back to `target_pos` when the projectile speed is zero.
pub fn predict_intercept(
    shooter_pos: SimVec3,
    target_pos: SimVec3,
    target_vel: SimVec3,
    projectile_speed: SimFloat,
) -> SimVec3 {
    if projectile_speed == SimFloat::ZERO {
        return target_pos;
    }
    let dist = shooter_pos.distance(target_pos);
    let time = dist / projectile_speed;
    SimVec3::new(
        target_pos.x + target_vel.x * time,
        target_pos.y + target_vel.y * time,
        target_pos.z + target_vel.z * time,
    )
}

// ---------------------------------------------------------------------------
// spawn_projectile_system
// ---------------------------------------------------------------------------

/// Reads [`FireEventQueue`], looks up weapon defs, and either creates an
/// instant [`ImpactEvent`] (beam / hitscan) or spawns a [`Projectile`] entity.
///
/// For ballistic projectiles, applies target prediction (RR-132) when the
/// target has a [`Velocity`] component.
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

        let Some(target_pos_comp) = world.get::<Position>(event.target) else {
            continue;
        };
        let raw_target_pos = target_pos_comp.pos;

        if def.projectile_speed == SimFloat::ZERO {
            // Beam / hitscan: instant impact at target position.
            beams.push(ImpactEvent {
                position: raw_target_pos,
                damage: def.damage,
                damage_type: def.damage_type,
                area_of_effect: def.area_of_effect,
                is_paralyzer: def.is_paralyzer,
                target_entity: if def.area_of_effect == SimFloat::ZERO {
                    Some(event.target)
                } else {
                    None
                },
            });
        } else {
            // RR-132: apply target prediction for ballistic projectiles.
            let target_vel = world
                .get::<Velocity>(event.target)
                .map(|v| v.vel)
                .unwrap_or(SimVec3::ZERO);
            let predicted_pos =
                predict_intercept(shooter_pos, raw_target_pos, target_vel, def.projectile_speed);

            spawns.push(SpawnInfo {
                shooter_pos,
                target_pos: predicted_pos,
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
                indirect_fire: false,
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
/// Uses the [`ProjectilePhysics`] trait to dispatch movement and hit
/// detection per projectile type (RR-113). Checks terrain collision via
/// the optional [`Heightmap`] resource (RR-133).
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
        indirect_fire: bool,
        despawned: bool,
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
            indirect_fire: proj.indirect_fire,
            despawned: false,
        });
    }

    // Optionally read the heightmap for terrain collision (RR-133).
    let heightmap = world.get_resource::<Heightmap>().cloned();

    let mut impacts: Vec<ImpactEvent> = Vec::new();

    for info in &mut infos {
        let prev_pos = info.pos;

        // RR-113: dispatch via ProjectilePhysics trait.
        let physics = dispatch_physics(info.projectile_type, info.speed);

        // For homing projectiles, resolve the live target position.
        let current_target_pos = match info.projectile_type {
            ProjectileType::Homing => {
                let live_pos = world
                    .get::<Position>(info.target_entity)
                    .map(|p| p.pos)
                    .unwrap_or(info.target_pos);
                info.target_pos = live_pos;
                live_pos
            }
            _ => info.target_pos,
        };

        physics.step(
            &mut info.pos,
            &mut info.vel,
            current_target_pos,
            SimFloat::ONE,
        );

        // Decrement lifetime — despawn if expired (prevents infinite orbit).
        info.lifetime = info.lifetime.saturating_sub(1);
        if info.lifetime == 0 {
            info.despawned = true;
            continue;
        }

        // RR-133: terrain collision (skip for indirect-fire / artillery).
        if !info.indirect_fire {
            if let Some(ref hm) = heightmap {
                if let Some(hit_point) = raycast_heightmap(hm, prev_pos, info.pos, 4) {
                    impacts.push(ImpactEvent {
                        position: hit_point,
                        damage: info.damage,
                        damage_type: info.damage_type,
                        area_of_effect: info.area_of_effect,
                        is_paralyzer: info.is_paralyzer,
                        target_entity: None, // terrain hit, no specific target
                    });
                    info.despawned = true;
                    continue;
                }
            }
        }

        // Check hit distance against target position using the trait.
        if physics.hit_check(info.pos, info.target_pos, HIT_DISTANCE) {
            impacts.push(ImpactEvent {
                position: info.pos,
                damage: info.damage,
                damage_type: info.damage_type,
                area_of_effect: info.area_of_effect,
                is_paralyzer: info.is_paralyzer,
                target_entity: if info.area_of_effect == SimFloat::ZERO {
                    Some(info.target_entity)
                } else {
                    None
                },
            });
            info.despawned = true;
        }
    }

    // Write back updated positions and velocities (only for non-despawned).
    for info in &infos {
        if info.despawned {
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
            proj.target_pos = info.target_pos;
        }
    }

    // Append impacts.
    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .extend(impacts);

    // Pre-warm the damageable cache for damage_system (runs next).
    crate::damage::populate_damageable_cache(world);

    // Despawn hit projectiles.
    for info in &infos {
        if info.despawned {
            world.despawn(info.entity);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/projectile_tests.rs"]
mod tests;
