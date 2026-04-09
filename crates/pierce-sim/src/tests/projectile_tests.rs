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
