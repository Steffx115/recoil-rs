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
        ..Default::default()
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
        ..Default::default()
    }
}

fn setup_world(defs: Vec<WeaponDef>) -> World {
    let mut world = World::new();
    world.insert_resource(WeaponRegistry { defs });
    world.insert_resource(FireEventQueue::default());
    world.insert_resource(ImpactEventQueue::default());
    world
}

/// Builder for spawning test projectiles with sensible defaults.
#[allow(dead_code)]
struct TestProjectile {
    projectile_type: ProjectileType,
    start: SimVec3,
    target_entity: Entity,
    target_pos: SimVec3,
    speed: SimFloat,
    damage: SimFloat,
    lifetime: u32,
}

impl TestProjectile {
    fn new(projectile_type: ProjectileType, target_entity: Entity, target_pos: SimVec3) -> Self {
        Self {
            projectile_type,
            start: SimVec3::ZERO,
            target_entity,
            target_pos,
            speed: sf(5),
            damage: sf(10),
            lifetime: 300,
        }
    }

    #[allow(dead_code)]
    fn start(mut self, pos: SimVec3) -> Self {
        self.start = pos;
        self
    }

    fn speed(mut self, s: SimFloat) -> Self {
        self.speed = s;
        self
    }

    #[allow(dead_code)]
    fn lifetime(mut self, l: u32) -> Self {
        self.lifetime = l;
        self
    }

    fn spawn(self, world: &mut World) -> Entity {
        let direction = (self.target_pos - self.start).normalize();
        let vel = direction * self.speed;
        world
            .spawn((
                Projectile {
                    projectile_type: self.projectile_type,
                    target_entity: self.target_entity,
                    target_pos: self.target_pos,
                    damage: self.damage,
                    damage_type: DamageType::Normal,
                    area_of_effect: SimFloat::ZERO,
                    speed: self.speed,
                    is_paralyzer: false,
                    lifetime: self.lifetime,
                    indirect_fire: false,
                },
                Position { pos: self.start },
                Velocity { vel },
            ))
            .id()
    }
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
            indirect_fire: false,
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
                indirect_fire: false,
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
                indirect_fire: false,
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

// =======================================================================
// RR-113: ProjectilePhysics trait tests
// =======================================================================

#[test]
fn trait_ballistic_applies_gravity() {
    let physics = BallisticPhysics;
    let mut pos = SimVec3::new(sf(0), sf(100), sf(0));
    let mut vel = SimVec3::new(sf(10), SimFloat::ZERO, SimFloat::ZERO);
    let target = SimVec3::new(sf(200), sf(0), sf(0));

    physics.step(&mut pos, &mut vel, target, SimFloat::ONE);

    // Gravity should reduce Y velocity.
    assert_eq!(vel.y, SimFloat::ZERO - GRAVITY);
    // Position advanced by velocity (after gravity).
    assert_eq!(pos.x, sf(10));
    assert_eq!(pos.y, sf(100) - GRAVITY);
}

#[test]
fn trait_homing_re_aims_toward_target() {
    let physics = HomingPhysics { speed: sf(5) };
    let mut pos = SimVec3::ZERO;
    let mut vel = SimVec3::new(sf(5), SimFloat::ZERO, SimFloat::ZERO);
    let target = SimVec3::new(sf(0), sf(0), sf(100));

    physics.step(&mut pos, &mut vel, target, SimFloat::ONE);

    // After step, velocity should point toward +Z target.
    assert!(vel.z > SimFloat::ZERO, "Should re-aim toward Z target");
    assert_eq!(vel.length().to_f64().round() as i32, 5, "Speed preserved");
}

#[test]
fn trait_beam_always_hits() {
    let physics = BeamPhysics;
    assert!(physics.hit_check(SimVec3::ZERO, SimVec3::new(sf(1000), sf(0), sf(0)), sf(1)));
}

#[test]
fn trait_dispatch_returns_correct_impl() {
    // Ballistic: gravity applied
    let physics = dispatch_physics(ProjectileType::Ballistic, sf(10));
    let mut pos = SimVec3::new(sf(0), sf(50), sf(0));
    let mut vel = SimVec3::new(sf(10), SimFloat::ZERO, SimFloat::ZERO);
    physics.step(&mut pos, &mut vel, SimVec3::ZERO, SimFloat::ONE);
    assert_eq!(vel.y, SimFloat::ZERO - GRAVITY);

    // Homing: re-aims
    let physics = dispatch_physics(ProjectileType::Homing, sf(5));
    let mut pos = SimVec3::ZERO;
    let mut vel = SimVec3::new(sf(5), SimFloat::ZERO, SimFloat::ZERO);
    physics.step(&mut pos, &mut vel, SimVec3::new(sf(0), sf(100), sf(0)), SimFloat::ONE);
    assert!(vel.y > SimFloat::ZERO);
}

// =======================================================================
// RR-117: Beam-specific tests
// =======================================================================

#[test]
fn beam_hits_instantly_same_frame() {
    // Beam weapons create an impact in the same frame they fire —
    // no projectile entity ever exists in the world.
    let mut world = setup_world(vec![beam_weapon_def()]);

    let shooter = world.spawn(pos3(0, 0, 0)).id();
    let target = world.spawn(pos3(500, 0, 0)).id();

    world
        .resource_mut::<FireEventQueue>()
        .events
        .push(FireEvent {
            shooter,
            target,
            weapon_def_id: 0,
        });

    spawn_projectile_system(&mut world);

    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert_eq!(impacts.len(), 1, "Beam should create exactly one impact");

    // Impact position should be at the target, regardless of distance.
    assert_eq!(impacts[0].position, SimVec3::new(sf(500), sf(0), sf(0)));
}

#[test]
fn beam_damage_applied_in_single_tick() {
    // Fire two beam weapons at the same target in a single frame.
    let mut world = setup_world(vec![beam_weapon_def(), beam_weapon_def()]);

    let shooter = world.spawn(pos3(0, 0, 0)).id();
    let target = world.spawn(pos3(10, 0, 0)).id();

    let queue = &mut world.resource_mut::<FireEventQueue>().events;
    queue.push(FireEvent {
        shooter,
        target,
        weapon_def_id: 0,
    });
    queue.push(FireEvent {
        shooter,
        target,
        weapon_def_id: 1,
    });

    spawn_projectile_system(&mut world);

    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert_eq!(impacts.len(), 2, "Both beams should produce impacts");
    // Total damage = 50 + 50 = 100 across two events.
    let total_damage = impacts[0].damage + impacts[1].damage;
    assert_eq!(total_damage, sf(100));
}

#[test]
fn beam_creates_no_projectile_entity() {
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

    // Absolutely no projectile entities should exist after spawn.
    let mut q = world.query::<&Projectile>();
    assert_eq!(q.iter(&world).count(), 0, "Beam must not spawn entities");
}

// =======================================================================
// RR-117: Homing-specific tests
// =======================================================================

#[test]
fn homing_hits_even_if_target_changes_direction() {
    let mut world = setup_world(vec![]);

    // Target starts at (50, 0, 0).
    let target = world.spawn(pos3(50, 0, 0)).id();
    let speed = sf(10);
    let proj = TestProjectile::new(ProjectileType::Homing, target, SimVec3::new(sf(50), sf(0), sf(0)))
        .speed(speed)
        .spawn(&mut world);

    // Run a few ticks, moving the target in different directions.
    for i in 0..3 {
        let new_z = if i % 2 == 0 { sf(30) } else { sf(-30) };
        world.get_mut::<Position>(target).unwrap().pos =
            SimVec3::new(sf(50), sf(0), new_z);
        projectile_movement_system(&mut world);
    }

    // Now put target close enough to ensure eventual hit.
    world.get_mut::<Position>(target).unwrap().pos =
        SimVec3::new(sf(20), sf(0), sf(0));

    // Run until impact or max iterations.
    let mut hit = false;
    for _ in 0..100 {
        projectile_movement_system(&mut world);
        if world.get::<Projectile>(proj).is_none() {
            hit = true;
            break;
        }
    }
    assert!(hit, "Homing projectile should eventually hit despite direction changes");
}

#[test]
fn homing_dead_target_continues_on_last_trajectory() {
    let mut world = setup_world(vec![]);

    let target = world.spawn(pos3(100, 0, 0)).id();
    let speed = sf(5);
    let proj = TestProjectile::new(ProjectileType::Homing, target, SimVec3::new(sf(100), sf(0), sf(0)))
        .speed(speed)
        .spawn(&mut world);

    // Tick once so projectile starts moving.
    projectile_movement_system(&mut world);
    let vel_before = world.get::<Velocity>(proj).unwrap().vel;

    // Despawn the target (simulates death).
    world.despawn(target);

    // Tick again — homing falls back to last-known target_pos.
    projectile_movement_system(&mut world);

    // Projectile should still exist (continuing toward last-known pos).
    assert!(
        world.get::<Projectile>(proj).is_some(),
        "Projectile should continue when target is dead"
    );
    let vel_after = world.get::<Velocity>(proj).unwrap().vel;
    // Should still be heading roughly the same direction (+X).
    assert!(vel_after.x > SimFloat::ZERO);
    // Speed should be preserved.
    let speed_before = vel_before.length();
    let speed_after = vel_after.length();
    assert_eq!(speed_before, speed_after, "Speed should be preserved after target death");
}

#[test]
fn homing_tracks_target_moving_perpendicular() {
    let mut world = setup_world(vec![]);

    // Target moves along Z axis while projectile chases.
    let target = world.spawn(pos3(80, 0, 0)).id();
    let speed = sf(5);
    let proj = TestProjectile::new(ProjectileType::Homing, target, SimVec3::new(sf(80), sf(0), sf(0)))
        .speed(speed)
        .spawn(&mut world);

    // Each tick, move target along +Z.
    for i in 0..5 {
        world.get_mut::<Position>(target).unwrap().pos =
            SimVec3::new(sf(80), sf(0), sf((i + 1) * 10));
        projectile_movement_system(&mut world);
    }

    let vel = world.get::<Velocity>(proj).unwrap().vel;
    assert!(
        vel.z > SimFloat::ZERO,
        "Homing projectile must track perpendicular target movement"
    );
}

// =======================================================================
// RR-117: Ballistic-specific tests
// =======================================================================

#[test]
fn ballistic_parabolic_arc_gravity() {
    let mut world = setup_world(vec![]);

    let target_pos = SimVec3::new(sf(200), sf(0), sf(0));
    let speed = sf(10);
    // Launch at 45 degrees: vx = vy = speed / sqrt(2) ~ 7
    let vx = SimFloat::from_ratio(7, 1);
    let vy = SimFloat::from_ratio(7, 1);

    let proj = world
        .spawn((
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
                indirect_fire: false,
            },
            Position { pos: SimVec3::ZERO },
            Velocity {
                vel: SimVec3::new(vx, vy, SimFloat::ZERO),
            },
        ))
        .id();

    // Track Y positions over time to verify parabolic arc.
    // Gravity = 0.1/tick, initial vy = 7. Apex at tick ~70, so run 140 ticks.
    let mut y_values: Vec<SimFloat> = Vec::new();
    for _ in 0..140 {
        projectile_movement_system(&mut world);
        if let Some(pos) = world.get::<Position>(proj) {
            y_values.push(pos.pos.y);
        } else {
            break; // despawned
        }
    }

    assert!(!y_values.is_empty(), "Projectile should have survived at least one tick");

    // Y should rise then fall (parabolic).
    let max_y = y_values.iter().max().unwrap();
    let last_y = y_values.last().unwrap();
    assert!(
        *max_y > SimFloat::ZERO,
        "Ballistic should arc upward initially"
    );
    assert!(
        *last_y < *max_y,
        "Ballistic should come back down under gravity"
    );
}

#[test]
fn ballistic_can_miss_moving_target() {
    let mut world = setup_world(vec![]);

    // Target starts at (100, 0, 0), projectile aimed there.
    let target = world.spawn(pos3(100, 0, 0)).id();
    let speed = sf(5);
    world.spawn((
        Projectile {
            projectile_type: ProjectileType::Ballistic,
            target_entity: target,
            target_pos: SimVec3::new(sf(100), sf(0), sf(0)),
            damage: sf(10),
            damage_type: DamageType::Normal,
            area_of_effect: SimFloat::ZERO,
            speed,
            is_paralyzer: false,
            lifetime: 50,
            indirect_fire: false,
        },
        Position { pos: SimVec3::ZERO },
        Velocity {
            vel: SimVec3::new(speed, SimFloat::ZERO, SimFloat::ZERO),
        },
    ));

    // Move target far away each tick — ballistic cannot re-aim.
    for _ in 0..50 {
        if let Some(mut tpos) = world.get_mut::<Position>(target) {
            tpos.pos.z += sf(50);
        }
        projectile_movement_system(&mut world);
    }

    // If target moved away enough, the projectile should have expired
    // without hitting — verify no impact was generated.
    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert!(
        impacts.is_empty(),
        "Ballistic projectile should miss if target moves away"
    );
}

#[test]
fn ballistic_aoe_at_impact_point() {
    // Verify that when a ballistic projectile hits, the impact event
    // carries the area_of_effect for splash damage.
    let mut world = setup_world(vec![]);

    let target_pos = SimVec3::new(sf(8), sf(0), sf(0));
    let target = world.spawn(Position { pos: target_pos }).id();

    world.spawn((
        Projectile {
            projectile_type: ProjectileType::Ballistic,
            target_entity: target,
            target_pos,
            damage: sf(30),
            damage_type: DamageType::Explosive,
            area_of_effect: sf(15),
            speed: sf(5),
            is_paralyzer: false,
            lifetime: 300,
            indirect_fire: false,
        },
        Position { pos: SimVec3::ZERO },
        Velocity {
            vel: SimVec3::new(sf(5), SimFloat::ZERO, SimFloat::ZERO),
        },
    ));

    // Run until impact.
    for _ in 0..50 {
        projectile_movement_system(&mut world);
    }

    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert_eq!(impacts.len(), 1, "Should hit target");
    assert_eq!(impacts[0].area_of_effect, sf(15), "AOE should be carried to impact");
    assert_eq!(impacts[0].damage, sf(30));
}

// =======================================================================
// RR-132: Target prediction tests
// =======================================================================

#[test]
fn predict_intercept_stationary_target() {
    let shooter = SimVec3::ZERO;
    let target = SimVec3::new(sf(100), sf(0), sf(0));
    let target_vel = SimVec3::ZERO;

    let predicted = predict_intercept(shooter, target, target_vel, sf(10));
    assert_eq!(predicted, target, "Stationary target: prediction equals current position");
}

#[test]
fn predict_intercept_moving_target() {
    let shooter = SimVec3::ZERO;
    let target = SimVec3::new(sf(100), sf(0), sf(0));
    let target_vel = SimVec3::new(SimFloat::ZERO, SimFloat::ZERO, sf(5));

    let predicted = predict_intercept(shooter, target, target_vel, sf(10));

    // Distance = 100, speed = 10, time = 10 ticks.
    // Predicted Z = 0 + 5 * 10 = 50.
    assert_eq!(predicted.x, sf(100));
    assert_eq!(predicted.z, sf(50));
}

#[test]
fn predict_intercept_zero_speed_returns_target() {
    let predicted = predict_intercept(
        SimVec3::ZERO,
        SimVec3::new(sf(50), sf(0), sf(0)),
        SimVec3::new(sf(10), sf(0), sf(0)),
        SimFloat::ZERO,
    );
    assert_eq!(predicted.x, sf(50), "Zero speed should return raw target pos");
}

#[test]
fn spawn_with_prediction_leads_moving_target() {
    // A target moving along +Z should cause the spawned projectile
    // to aim ahead of the current position.
    let mut world = setup_world(vec![projectile_weapon_def(10)]);

    let shooter = world.spawn(pos3(0, 0, 0)).id();
    let target = world
        .spawn((
            pos3(100, 0, 0),
            Velocity {
                vel: SimVec3::new(SimFloat::ZERO, SimFloat::ZERO, sf(5)),
            },
        ))
        .id();

    world
        .resource_mut::<FireEventQueue>()
        .events
        .push(FireEvent {
            shooter,
            target,
            weapon_def_id: 0,
        });

    spawn_projectile_system(&mut world);

    let mut q = world.query::<(&Projectile, &Velocity)>();
    let (proj, vel) = q.single(&world);

    // The target_pos stored on the projectile should have Z > 0
    // (leading the target).
    assert!(
        proj.target_pos.z > SimFloat::ZERO,
        "Projectile should lead target in Z"
    );
    // Velocity should have a Z component pointing toward predicted position.
    assert!(
        vel.vel.z > SimFloat::ZERO,
        "Velocity should aim at predicted intercept"
    );
}

// =======================================================================
// RR-133: Terrain collision tests
// =======================================================================

fn flat_heightmap(height: u16, width: u32, depth: u32) -> Heightmap {
    Heightmap {
        data: vec![height; (width * depth) as usize],
        width,
        height: depth,
        cell_size: SimFloat::ONE,
    }
}

fn ridge_heightmap() -> Heightmap {
    // 10x1 heightmap: flat (0) except cells 4-5 which are a tall ridge (100).
    let mut data = vec![0u16; 10];
    data[4] = 100;
    data[5] = 100;
    Heightmap {
        data,
        width: 10,
        height: 1,
        cell_size: SimFloat::ONE,
    }
}

#[test]
fn heightmap_sample_flat() {
    let hm = flat_heightmap(50, 8, 8);
    let h = hm.sample(sf(4), sf(4));
    assert_eq!(h, sf(50));
}

#[test]
fn heightmap_sample_clamps_out_of_bounds() {
    let hm = flat_heightmap(10, 4, 4);
    // Out of bounds should clamp, not panic.
    let h = hm.sample(sf(-5), sf(-5));
    assert_eq!(h, sf(10));
}

#[test]
fn raycast_hits_terrain() {
    // Projectile flies at Y=5 over terrain at Y=50 — should hit.
    let hm = flat_heightmap(50, 8, 8);
    let start = SimVec3::new(sf(1), sf(5), sf(1));
    let end = SimVec3::new(sf(6), sf(5), sf(1));
    let hit = raycast_heightmap(&hm, start, end, 8);
    assert!(hit.is_some(), "Projectile below terrain should register hit");
}

#[test]
fn raycast_misses_above_terrain() {
    // Projectile flies at Y=100 over terrain at Y=50 — should not hit.
    let hm = flat_heightmap(50, 8, 8);
    let start = SimVec3::new(sf(1), sf(100), sf(1));
    let end = SimVec3::new(sf(6), sf(100), sf(1));
    let hit = raycast_heightmap(&hm, start, end, 8);
    assert!(hit.is_none(), "Projectile above terrain should not hit");
}

#[test]
fn projectile_terrain_collision_in_movement_system() {
    // Place a heightmap with tall terrain at Y=200.
    // Fire a projectile at Y=10 — it should collide with terrain.
    let mut world = setup_world(vec![]);
    world.insert_resource(flat_heightmap(200, 20, 20));

    let target_pos = SimVec3::new(sf(15), sf(10), sf(5));
    let target = world.spawn(Position { pos: target_pos }).id();

    world.spawn((
        Projectile {
            projectile_type: ProjectileType::Ballistic,
            target_entity: target,
            target_pos,
            damage: sf(10),
            damage_type: DamageType::Explosive,
            area_of_effect: sf(5),
            speed: sf(3),
            is_paralyzer: false,
            lifetime: 300,
            indirect_fire: false,
        },
        Position {
            pos: SimVec3::new(sf(1), sf(10), sf(5)),
        },
        Velocity {
            vel: SimVec3::new(sf(3), SimFloat::ZERO, SimFloat::ZERO),
        },
    ));

    projectile_movement_system(&mut world);

    // Should have created a terrain-collision impact.
    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert_eq!(impacts.len(), 1, "Projectile should hit terrain");

    // No projectile entities should remain.
    let mut q = world.query::<&Projectile>();
    assert_eq!(q.iter(&world).count(), 0, "Projectile despawned on terrain hit");
}

#[test]
fn artillery_indirect_fire_arcs_over_terrain() {
    // An indirect_fire projectile should NOT collide with terrain.
    let mut world = setup_world(vec![]);
    world.insert_resource(flat_heightmap(200, 20, 20));

    let target_pos = SimVec3::new(sf(15), sf(10), sf(5));
    let target = world.spawn(Position { pos: target_pos }).id();

    world.spawn((
        Projectile {
            projectile_type: ProjectileType::Ballistic,
            target_entity: target,
            target_pos,
            damage: sf(10),
            damage_type: DamageType::Explosive,
            area_of_effect: sf(5),
            speed: sf(3),
            is_paralyzer: false,
            lifetime: 300,
            indirect_fire: true, // artillery
        },
        Position {
            pos: SimVec3::new(sf(1), sf(10), sf(5)),
        },
        Velocity {
            vel: SimVec3::new(sf(3), SimFloat::ZERO, SimFloat::ZERO),
        },
    ));

    projectile_movement_system(&mut world);

    // Indirect fire should skip terrain collision — no impact yet.
    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert!(
        impacts.is_empty(),
        "Artillery (indirect fire) should arc over terrain"
    );

    // Projectile should still exist.
    let mut q = world.query::<&Projectile>();
    assert_eq!(q.iter(&world).count(), 1, "Artillery projectile should survive");
}

#[test]
fn projectile_hits_ridge_between_shooter_and_target() {
    // Ridge at cells 4-5, projectile at Y=50 flying from x=0 to x=9.
    // Ridge height is 100, projectile at Y=50 should hit the ridge.
    let mut world = setup_world(vec![]);
    world.insert_resource(ridge_heightmap());

    let target_pos = SimVec3::new(sf(9), sf(0), sf(0));
    let target = world.spawn(Position { pos: target_pos }).id();

    world.spawn((
        Projectile {
            projectile_type: ProjectileType::Ballistic,
            target_entity: target,
            target_pos,
            damage: sf(10),
            damage_type: DamageType::Explosive,
            area_of_effect: sf(5),
            speed: sf(3),
            is_paralyzer: false,
            lifetime: 300,
            indirect_fire: false,
        },
        Position {
            pos: SimVec3::new(sf(1), sf(50), sf(0)),
        },
        Velocity {
            vel: SimVec3::new(sf(3), SimFloat::ZERO, SimFloat::ZERO),
        },
    ));

    // Run a few ticks — projectile should hit the ridge.
    for _ in 0..10 {
        projectile_movement_system(&mut world);
    }

    let impacts = &world.resource::<ImpactEventQueue>().events;
    assert!(
        !impacts.is_empty(),
        "Projectile should collide with ridge between shooter and target"
    );
    // Impact should be near the ridge (x ~ 4-5).
    let impact_x = impacts[0].position.x;
    assert!(
        impact_x >= sf(2) && impact_x <= sf(7),
        "Impact should be near the ridge"
    );
}
