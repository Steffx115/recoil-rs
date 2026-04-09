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
        is_paralyzer: false, ..Default::default()
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 100,
                max: 100,
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
                current: 0,
                max: 100,
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
                current: 50,
                max: 100,
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
