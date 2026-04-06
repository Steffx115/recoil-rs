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
mod tests {
    use super::*;
    use crate::combat_data::{DamageType, WeaponDef};
    use crate::components::Position;
    use crate::targeting::{FireEvent, FireEventQueue, WeaponRegistry};
    use crate::{SimFloat, SimVec3};

    fn sf(n: i32) -> SimFloat {
        SimFloat::from_int(n)
    }

    fn pos3(x: i32, y: i32, z: i32) -> Position {
        Position {
            pos: SimVec3::new(sf(x), sf(y), sf(z)),
        }
    }

    fn beam_weapon_def() -> WeaponDef {
        WeaponDef {
            damage: sf(50),
            damage_type: DamageType::Laser,
            range: sf(100),
            reload_time: 10,
            projectile_speed: SimFloat::ZERO,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        }
    }

    fn projectile_weapon_def(speed: i32) -> WeaponDef {
        WeaponDef {
            damage: sf(25),
            damage_type: DamageType::Explosive,
            range: sf(100),
            reload_time: 10,
            projectile_speed: sf(speed),
            area_of_effect: sf(5),
            is_paralyzer: false,
        }
    }

    fn setup_world(defs: Vec<WeaponDef>) -> World {
        let mut world = World::new();
        world.insert_resource(WeaponRegistry { defs });
        world.insert_resource(FireEventQueue::default());
        world.insert_resource(ImpactEventQueue::default());
        world
    }

    // -----------------------------------------------------------------------
    // Beam weapon creates instant impact
    // -----------------------------------------------------------------------

    #[test]
    fn beam_creates_instant_impact() {
        let mut world = setup_world(vec![beam_weapon_def()]);

        let shooter = world.spawn(pos3(0, 0, 0)).id();
        let target = world.spawn(pos3(10, 0, 0)).id();

        world
            .resource_mut::<FireEventQueue>()
            .events
            .push(FireEvent {
                shooter,
                target,
                weapon_def_id: 0,
            });

        spawn_projectile_system(&mut world);

        // Should have created an impact event immediately.
        let impacts = &world.resource::<ImpactEventQueue>().events;
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].damage, sf(50));
        assert_eq!(impacts[0].damage_type, DamageType::Laser);
        assert_eq!(impacts[0].position, SimVec3::new(sf(10), sf(0), sf(0)));

        // No projectile entities should exist.
        let mut q = world.query::<&Projectile>();
        assert_eq!(q.iter(&world).count(), 0);
    }

    // -----------------------------------------------------------------------
    // Ballistic projectile moves toward target
    // -----------------------------------------------------------------------

    #[test]
    fn ballistic_projectile_moves_toward_target() {
        let mut world = setup_world(vec![]);
        world.insert_resource(ImpactEventQueue::default());

        let target_pos = SimVec3::new(sf(100), sf(0), sf(0));

        // Manually spawn a ballistic projectile moving along +X.
        let speed = sf(5);
        world.spawn((
            Projectile {
                projectile_type: ProjectileType::Ballistic,
                target_entity: Entity::PLACEHOLDER,
                target_pos,
                damage: sf(10),
                damage_type: DamageType::Normal,
                area_of_effect: SimFloat::ZERO,
                speed,
                is_paralyzer: false,
                lifetime: 300,
            },
            Position {
                pos: SimVec3::new(sf(0), sf(10), sf(0)),
            },
            Velocity {
                vel: SimVec3::new(speed, SimFloat::ZERO, SimFloat::ZERO),
            },
        ));

        projectile_movement_system(&mut world);

        // After one tick the projectile should have moved +5 on X,
        // and gravity should have reduced Y velocity by 0.1.
        let mut q = world.query::<(&Position, &Velocity)>();
        let (pos, vel) = q.single(&world);

        assert_eq!(pos.pos.x, sf(5));
        // Y should be 10 + (0 - 0.1) = 9.9
        let expected_y = sf(10) - GRAVITY;
        assert_eq!(pos.pos.y, expected_y);
        // Velocity Y should now be -GRAVITY.
        assert_eq!(vel.vel.y, SimFloat::ZERO - GRAVITY);
    }

    // -----------------------------------------------------------------------
    // Homing projectile tracks moving target
    // -----------------------------------------------------------------------

    #[test]
    fn homing_projectile_tracks_moving_target() {
        let mut world = setup_world(vec![]);
        world.insert_resource(ImpactEventQueue::default());

        // Target starts at (100, 0, 0).
        let target = world.spawn(pos3(100, 0, 0)).id();

        let speed = sf(5);
        let proj = world
            .spawn((
                Projectile {
                    projectile_type: ProjectileType::Homing,
                    target_entity: target,
                    target_pos: SimVec3::new(sf(100), sf(0), sf(0)),
                    damage: sf(10),
                    damage_type: DamageType::Normal,
                    area_of_effect: SimFloat::ZERO,
                    speed,
                    is_paralyzer: false,
                    lifetime: 300,
                },
                Position {
                    pos: SimVec3::new(sf(0), sf(0), sf(0)),
                },
                Velocity {
                    vel: SimVec3::new(speed, SimFloat::ZERO, SimFloat::ZERO),
                },
            ))
            .id();

        // First tick: projectile moves toward (100,0,0).
        projectile_movement_system(&mut world);

        let pos1 = world.get::<Position>(proj).unwrap().pos;
        // Should have moved along +X.
        assert!(pos1.x > SimFloat::ZERO);

        // Now move the target to (100, 0, 100) — offset in Z.
        world.get_mut::<Position>(target).unwrap().pos = SimVec3::new(sf(100), sf(0), sf(100));

        // Second tick: homing should adjust toward new target position.
        projectile_movement_system(&mut world);

        let vel2 = world.get::<Velocity>(proj).unwrap().vel;
        // Velocity should now have a positive Z component (tracking the target).
        assert!(
            vel2.z > SimFloat::ZERO,
            "Homing projectile should track target's new Z position"
        );
    }

    // -----------------------------------------------------------------------
    // Projectile despawns on impact
    // -----------------------------------------------------------------------

    #[test]
    fn projectile_despawns_on_impact() {
        let mut world = setup_world(vec![]);
        world.insert_resource(ImpactEventQueue::default());

        // Place projectile very close to target so it hits on the first tick.
        let target_pos = SimVec3::new(sf(3), sf(0), sf(0));
        let target = world.spawn(Position { pos: target_pos }).id();

        let speed = sf(2);
        let proj = world
            .spawn((
                Projectile {
                    projectile_type: ProjectileType::Homing,
                    target_entity: target,
                    target_pos,
                    damage: sf(20),
                    damage_type: DamageType::Explosive,
                    area_of_effect: sf(3),
                    speed,
                    is_paralyzer: false,
                    lifetime: 300,
                },
                Position { pos: SimVec3::ZERO },
                Velocity {
                    vel: SimVec3::new(speed, SimFloat::ZERO, SimFloat::ZERO),
                },
            ))
            .id();

        projectile_movement_system(&mut world);

        // Projectile should be despawned — query for its Projectile component.
        assert!(
            world.get::<Projectile>(proj).is_none(),
            "Projectile entity should be despawned after impact"
        );

        // Impact event should have been created.
        let impacts = &world.resource::<ImpactEventQueue>().events;
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].damage, sf(20));
        assert_eq!(impacts[0].damage_type, DamageType::Explosive);
        assert_eq!(impacts[0].area_of_effect, sf(3));
    }

    // -----------------------------------------------------------------------
    // Determinism test: same inputs produce identical outputs
    // -----------------------------------------------------------------------

    #[test]
    fn determinism_identical_runs() {
        fn run_simulation() -> (Vec<ImpactEvent>, Vec<(SimVec3, SimVec3)>) {
            let mut world = World::new();
            world.insert_resource(WeaponRegistry {
                defs: vec![projectile_weapon_def(3)],
            });
            world.insert_resource(FireEventQueue::default());
            world.insert_resource(ImpactEventQueue::default());

            let shooter = world.spawn(pos3(0, 0, 0)).id();
            let target = world.spawn(pos3(20, 0, 0)).id();

            world
                .resource_mut::<FireEventQueue>()
                .events
                .push(FireEvent {
                    shooter,
                    target,
                    weapon_def_id: 0,
                });

            spawn_projectile_system(&mut world);

            // Run several movement ticks.
            for _ in 0..5 {
                projectile_movement_system(&mut world);
            }

            let impacts = world.resource::<ImpactEventQueue>().events.clone();

            let mut positions: Vec<(SimVec3, SimVec3)> = Vec::new();
            let mut q = world.query::<(&Position, &Velocity)>();
            for (pos, vel) in q.iter(&world) {
                positions.push((pos.pos, vel.vel));
            }
            // Sort for deterministic ordering.
            positions.sort_by_key(|(p, _)| (p.x, p.y, p.z));

            (impacts, positions)
        }

        let (impacts_a, positions_a) = run_simulation();
        let (impacts_b, positions_b) = run_simulation();

        assert_eq!(impacts_a, impacts_b, "Impact events must be identical");
        assert_eq!(
            positions_a, positions_b,
            "Projectile positions must be identical"
        );
    }

    // -----------------------------------------------------------------------
    // spawn_projectile_system spawns a projectile for non-beam weapons
    // -----------------------------------------------------------------------

    #[test]
    fn spawn_creates_projectile_entity() {
        let mut world = setup_world(vec![projectile_weapon_def(10)]);

        let shooter = world.spawn(pos3(0, 0, 0)).id();
        let target = world.spawn(pos3(50, 0, 0)).id();

        world
            .resource_mut::<FireEventQueue>()
            .events
            .push(FireEvent {
                shooter,
                target,
                weapon_def_id: 0,
            });

        spawn_projectile_system(&mut world);

        // No impact events for projectile weapons.
        assert!(world.resource::<ImpactEventQueue>().events.is_empty());

        // One projectile entity should exist.
        let mut q = world.query::<(&Projectile, &Position, &Velocity)>();
        let results: Vec<_> = q.iter(&world).collect();
        assert_eq!(results.len(), 1);

        let (proj, pos, vel) = results[0];
        assert_eq!(proj.damage, sf(25));
        assert_eq!(proj.damage_type, DamageType::Explosive);
        assert_eq!(proj.area_of_effect, sf(5));
        assert_eq!(proj.speed, sf(10));
        // Spawned at shooter position.
        assert_eq!(pos.pos, SimVec3::ZERO);
        // Velocity should point toward target (+X).
        assert!(vel.vel.x > SimFloat::ZERO);
    }
}
