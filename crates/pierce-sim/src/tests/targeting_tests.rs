use super::*;
use crate::combat_data::{DamageType, WeaponInstance, WeaponSet};
use crate::components::{
    Dead, FireMode, Heading, Health, LastAttacker, ManualTarget, Position, SimId, Target,
    TurretFacings, TurretState,
};
use crate::map::HeightmapData;
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
        ..Default::default()
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

// =======================================================================
// RR-75: Terrain LOS tests
// =======================================================================

#[test]
fn terrain_los_blocked_by_hill() {
    // Heightmap: 5x1 strip. Middle cell has a tall hill.
    let heightmap = HeightmapData {
        width: 5,
        height: 1,
        cell_size: sf(10),
        heights: vec![0, 0, 5000, 0, 0], // middle cell = height 50
    };

    // Shooter at x=5, y=0. Target at x=35, y=0. Hill at x=20..30, height=50.
    // The sight line from y=0 to y=0 passes through height 50 terrain.
    assert!(!heightmap.has_line_of_sight(
        sf(5),
        SimFloat::ZERO,
        SimFloat::ZERO,
        sf(35),
        SimFloat::ZERO,
        SimFloat::ZERO,
    ));
}

#[test]
fn terrain_los_clear_when_flat() {
    let heightmap = HeightmapData {
        width: 5,
        height: 1,
        cell_size: sf(10),
        heights: vec![0, 0, 0, 0, 0],
    };

    assert!(heightmap.has_line_of_sight(
        sf(5),
        SimFloat::ZERO,
        SimFloat::ZERO,
        sf(35),
        SimFloat::ZERO,
        SimFloat::ZERO,
    ));
}

#[test]
fn terrain_los_high_ground_advantage() {
    // Shooter on a hill (y=200) can see over a lower hill (height=50).
    let heightmap = HeightmapData {
        width: 5,
        height: 1,
        cell_size: sf(10),
        heights: vec![0, 0, 5000, 0, 0], // middle = height 50
    };

    // Shooter at y=200. At the hill sample point (t=2/3), sight height
    // is 200 * 1/3 = 66.7, which clears the 50-height terrain.
    assert!(heightmap.has_line_of_sight(
        sf(5),
        sf(200),
        SimFloat::ZERO,
        sf(35),
        SimFloat::ZERO,
        SimFloat::ZERO,
    ));
}

#[test]
fn targeting_respects_terrain_los() {
    // Weapon with direct fire.
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        reload_time: 10,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    // Add heightmap with a hill blocking LOS.
    world.insert_resource(HeightmapData {
        width: 10,
        height: 10,
        cell_size: sf(10),
        heights: {
            let mut h = vec![0u16; 100];
            // Create a wall at column 5.
            for row in 0..10 {
                h[row * 10 + 5] = 10000; // Very tall hill.
            }
            h
        },
    });

    // Shooter at (10, 0, 50), team 1.
    let shooter = world
        .spawn((
            pos3(10, 0, 50),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 1, 5);

    // Enemy behind the hill at (70, 0, 50), team 2.
    let enemy_behind_hill = world
        .spawn((
            pos3(70, 0, 50),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy_behind_hill, 7, 5);

    targeting_system(&mut world);

    // Should NOT acquire the target because terrain blocks LOS.
    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(target.entity, None);
}

#[test]
fn indirect_fire_ignores_terrain_los() {
    // Artillery weapon with indirect fire.
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        reload_time: 10,
        indirect_fire: true,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    // Add heightmap with a blocking hill.
    world.insert_resource(HeightmapData {
        width: 10,
        height: 10,
        cell_size: sf(10),
        heights: {
            let mut h = vec![0u16; 100];
            for row in 0..10 {
                h[row * 10 + 5] = 10000;
            }
            h
        },
    });

    let shooter = world
        .spawn((
            pos3(10, 0, 50),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 1, 5);

    let enemy = world
        .spawn((
            pos3(70, 0, 50),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy, 7, 5);

    targeting_system(&mut world);

    // Artillery CAN fire over the hill.
    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(target.entity, Some(enemy));
}

// =======================================================================
// RR-76: Fire modes tests
// =======================================================================

#[test]
fn hold_fire_prevents_targeting() {
    let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

    let shooter = world
        .spawn((
            pos3(10, 0, 10),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            FireMode::HoldFire,
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 10, 10);

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

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(target.entity, None, "HoldFire should not auto-target");
}

#[test]
fn return_fire_targets_attacker_only() {
    let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

    let attacker = world
        .spawn((
            pos3(15, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, attacker, 15, 10);

    // Another enemy, closer, but not the attacker.
    let _closer_enemy = world
        .spawn((
            pos3(12, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 3 },
        ))
        .id();
    grid_insert(&mut world, _closer_enemy, 12, 10);

    let shooter = world
        .spawn((
            pos3(10, 0, 10),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            FireMode::ReturnFire,
            LastAttacker {
                entity: Some(attacker),
            },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 10, 10);

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity,
        Some(attacker),
        "ReturnFire should target the attacker"
    );
}

#[test]
fn return_fire_no_target_without_attacker() {
    let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

    let _enemy = world
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
    grid_insert(&mut world, _enemy, 12, 10);

    let shooter = world
        .spawn((
            pos3(10, 0, 10),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            FireMode::ReturnFire,
            LastAttacker { entity: None },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 10, 10);

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity, None,
        "ReturnFire without attacker should have no target"
    );
}

#[test]
fn manual_target_override() {
    let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

    let far_enemy = world
        .spawn((
            pos3(50, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 3 },
        ))
        .id();
    grid_insert(&mut world, far_enemy, 50, 10);

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

    // Shooter with manual target override on the far enemy.
    let shooter = world
        .spawn((
            pos3(10, 0, 10),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target { entity: None },
            ManualTarget {
                position: None,
                forced_entity: Some(far_enemy),
            },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 10, 10);

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity,
        Some(far_enemy),
        "Manual override should force target"
    );
}

#[test]
fn threat_priority_armed_over_unarmed() {
    let mut world = setup_world(vec![simple_weapon_def(100, 10)]);

    // Unarmed enemy (closer).
    let unarmed = world
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
    grid_insert(&mut world, unarmed, 12, 10);

    // Armed enemy (same distance but different axis).
    let armed = world
        .spawn((
            pos3(10, 0, 12),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            SimId { id: 3 },
        ))
        .id();
    grid_insert(&mut world, armed, 10, 12);

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

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity,
        Some(armed),
        "Armed targets should be prioritized over unarmed"
    );
}

// =======================================================================
// RR-134: Minimum range tests
// =======================================================================

#[test]
fn min_range_prevents_firing() {
    // Weapon with min_range 5, max range 100.
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        min_range: sf(5),
        reload_time: 1,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    // Enemy at distance 2 (within min_range).
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

    targeting_system(&mut world);

    // Target should NOT be acquired because enemy is inside min_range.
    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity, None,
        "Enemy within min_range should not be targeted"
    );
}

#[test]
fn min_range_allows_targets_beyond() {
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        min_range: sf(5),
        reload_time: 1,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    // Enemy at distance ~14 (beyond min_range).
    let enemy = world
        .spawn((
            pos3(24, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy, 24, 10);

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

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity,
        Some(enemy),
        "Enemy beyond min_range should be targeted"
    );
}

#[test]
fn min_range_prevents_reload_firing() {
    // Test that reload_system also respects min_range.
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        min_range: sf(20),
        reload_time: 1,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

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

    // Shooter with weapon ready and target set, but enemy within min_range.
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
            Target {
                entity: Some(enemy),
            },
            SimId { id: 1 },
        ))
        .id();

    reload_system(&mut world);

    assert!(
        world.resource::<FireEventQueue>().events.is_empty(),
        "Should not fire at target within min_range"
    );
}

// =======================================================================
// RR-135: Turret rotation and firing arcs tests
// =======================================================================

#[test]
fn turret_rotates_toward_target() {
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        reload_time: 10,
        turret_turn_rate: SimFloat::from_ratio(1, 10), // 0.1 rad/tick
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    let enemy = world
        .spawn((
            pos3(20, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy, 20, 10);

    let shooter = world
        .spawn((
            pos3(10, 0, 10),
            Allegiance { team: 1 },
            WeaponSet {
                weapons: vec![weapon_instance(0)],
            },
            Target {
                entity: Some(enemy),
            },
            TurretFacings {
                facings: vec![TurretState {
                    facing: SimFloat::ZERO,
                }],
            },
            SimId { id: 1 },
        ))
        .id();
    grid_insert(&mut world, shooter, 10, 10);

    // Run turret system a few times.
    turret_system(&mut world);

    // Turret should have rotated toward the target.
    let facings = world.get::<TurretFacings>(shooter).unwrap();
    assert_ne!(
        facings.facings[0].facing,
        SimFloat::ZERO,
        "Turret should have rotated"
    );
}

#[test]
fn turret_not_aimed_prevents_firing() {
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        reload_time: 1,
        turret_turn_rate: SimFloat::from_ratio(1, 100), // Very slow turn.
        aim_tolerance: SimFloat::from_ratio(52, 1000),   // ~3 degrees.
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    let enemy = world
        .spawn((
            pos3(20, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy, 20, 10);

    // Shooter with turret pointed the wrong way.
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
            Target {
                entity: Some(enemy),
            },
            TurretFacings {
                facings: vec![TurretState {
                    facing: SimFloat::PI, // Pointing backwards.
                }],
            },
            SimId { id: 1 },
        ))
        .id();

    reload_system(&mut world);

    assert!(
        world.resource::<FireEventQueue>().events.is_empty(),
        "Should not fire when turret is not aimed at target"
    );
}

#[test]
fn firing_arc_restricts_targeting() {
    // Weapon with a narrow forward-only firing arc.
    let def = WeaponDef {
        damage: sf(10),
        range: sf(100),
        reload_time: 1,
        firing_arc: SimFloat::FRAC_PI_4, // 45-degree half-arc (90 total).
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);

    // Enemy directly behind the shooter (angle ~PI from heading 0).
    let enemy_behind = world
        .spawn((
            pos3(10, 0, 20),
            Allegiance { team: 2 },
            Health {
                current: sf(100),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, enemy_behind, 10, 20);

    // Shooter at origin, heading = 0 (facing +Z... but enemy is behind).
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
            Target {
                entity: Some(enemy_behind),
            },
            Heading {
                angle: SimFloat::PI, // Facing opposite direction.
            },
            SimId { id: 1 },
        ))
        .id();

    reload_system(&mut world);

    assert!(
        world.resource::<FireEventQueue>().events.is_empty(),
        "Should not fire at target outside firing arc"
    );
}

// =======================================================================
// RR-136: Overkill avoidance tests
// =======================================================================

#[test]
fn overkill_avoidance_skips_over_damaged_target() {
    // Weapon does 100 damage.
    let def = WeaponDef {
        damage: sf(100),
        range: sf(100),
        reload_time: 10,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);
    world.insert_resource(PendingDamage::default());

    // Enemy with 50 HP.
    let weak_enemy = world
        .spawn((
            pos3(12, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(50),
                max: sf(100),
            },
            SimId { id: 2 },
        ))
        .id();
    grid_insert(&mut world, weak_enemy, 12, 10);

    // Second enemy with 200 HP (further away).
    let strong_enemy = world
        .spawn((
            pos3(20, 0, 10),
            Allegiance { team: 2 },
            Health {
                current: sf(200),
                max: sf(200),
            },
            SimId { id: 3 },
        ))
        .id();
    grid_insert(&mut world, strong_enemy, 20, 10);

    // Pre-load pending damage: weak_enemy already has 50 pending damage (enough to kill).
    world
        .resource_mut::<PendingDamage>()
        .add(weak_enemy, sf(50));

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

    targeting_system(&mut world);

    let target = world.get::<Target>(shooter).unwrap();
    assert_eq!(
        target.entity,
        Some(strong_enemy),
        "Should skip overkill target and pick the next one"
    );
}

#[test]
fn pending_damage_tracks_fire_events() {
    let def = WeaponDef {
        damage: sf(25),
        range: sf(100),
        reload_time: 1,
        ..Default::default()
    };
    let mut world = setup_world(vec![def]);
    world.insert_resource(PendingDamage::default());

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
            Target {
                entity: Some(enemy),
            },
            SimId { id: 1 },
        ))
        .id();

    reload_system(&mut world);

    // Fire event should be emitted.
    assert_eq!(world.resource::<FireEventQueue>().events.len(), 1);

    // Pending damage should be recorded.
    let pending = world.resource::<PendingDamage>();
    assert_eq!(
        pending.get(enemy),
        sf(25),
        "Pending damage should track fired shots"
    );
}

#[test]
fn pending_damage_clear_resets() {
    let mut pd = PendingDamage::default();
    let entity = Entity::from_raw(42);
    pd.add(entity, sf(100));
    assert_eq!(pd.get(entity), sf(100));
    pd.clear();
    assert_eq!(pd.get(entity), SimFloat::ZERO);
}
