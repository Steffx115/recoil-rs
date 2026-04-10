use super::*;
use crate::combat_data::{ArmorClass, DamageTable, DamageType};
use crate::components::{Dead, Health, Position, Stunned};
use crate::construction::Reclaimable;
use crate::damage::DamageableCache;
use crate::projectile::{ImpactEvent, ImpactEventQueue};
use crate::spatial::SpatialGrid;
use crate::{SimFloat, SimVec2, SimVec3};

fn sf(n: i32) -> SimFloat {
    SimFloat::from_int(n)
}

fn pos3(x: i32, y: i32, z: i32) -> Position {
    Position {
        pos: SimVec3::new(sf(x), sf(y), sf(z)),
    }
}

/// Set up a world with the required resources for the damage system.
fn setup_world() -> World {
    let mut world = World::new();
    world.insert_resource(ImpactEventQueue::default());
    world.insert_resource(DamageTable::default());
    world.insert_resource(DamageableCache::default());
    // Grid: cell_size=10, 100x100 map.
    world.insert_resource(SpatialGrid::new(sf(10), 100, 100));
    world
}

/// Insert an entity into the spatial grid at its XZ position.
fn register_in_grid(world: &mut World, entity: Entity, pos: &Position) {
    let xz = SimVec2::new(pos.pos.x, pos.pos.z);
    world.resource_mut::<SpatialGrid>().insert(entity, xz);
}

// -----------------------------------------------------------------------
// 1. Single target takes damage
// -----------------------------------------------------------------------

#[test]
fn single_target_takes_damage() {
    let mut world = setup_world();

    let pos = pos3(10, 0, 10);
    let entity = world
        .spawn((
            pos.clone(),
            Health {
                current: 100,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, entity, &pos);

    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(30),
            damage_type: DamageType::Normal,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });

    damage_system(&mut world);

    let health = world.get::<Health>(entity).unwrap();
    assert_eq!(health.current, 70, "Should have taken 30 damage");
}

// -----------------------------------------------------------------------
// 2. AOE damages multiple targets
// -----------------------------------------------------------------------

#[test]
fn aoe_damages_multiple_targets() {
    let mut world = setup_world();

    // Spawn two units close together.
    let pos_a = pos3(10, 0, 10);
    let a = world
        .spawn((
            pos_a.clone(),
            Health {
                current: 100,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, a, &pos_a);

    let pos_b = pos3(12, 0, 10);
    let b = world
        .spawn((
            pos_b.clone(),
            Health {
                current: 100,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, b, &pos_b);

    // Spawn one unit far away (should NOT be hit).
    let pos_c = pos3(90, 0, 90);
    let c = world
        .spawn((
            pos_c.clone(),
            Health {
                current: 100,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, c, &pos_c);

    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(11), sf(0), sf(10)),
            damage: sf(20),
            damage_type: DamageType::Normal,
            area_of_effect: sf(5),
            is_paralyzer: false,
        });

    damage_system(&mut world);

    let ha = world.get::<Health>(a).unwrap();
    let hb = world.get::<Health>(b).unwrap();
    let hc = world.get::<Health>(c).unwrap();

    assert!(
        ha.current < 100,
        "Unit A should be damaged by AOE, got {:?}",
        ha.current
    );
    assert!(
        hb.current < 100,
        "Unit B should be damaged by AOE, got {:?}",
        hb.current
    );
    assert_eq!(hc.current, 100, "Unit C should be unaffected");
}

// -----------------------------------------------------------------------
// 3. Unit dies and spawns wreckage
// -----------------------------------------------------------------------

#[test]
fn unit_dies_and_spawns_wreckage() {
    let mut world = setup_world();

    let pos = pos3(10, 0, 10);
    let entity = world
        .spawn((
            pos.clone(),
            Health {
                current: 50,
                max: 200,
            },
        ))
        .id();
    register_in_grid(&mut world, entity, &pos);

    // Deal enough damage to kill.
    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(60),
            damage_type: DamageType::Normal,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });

    damage_system(&mut world);

    // Entity should be marked Dead.
    assert!(
        world.get::<Dead>(entity).is_some(),
        "Unit should be marked Dead"
    );

    // A wreckage entity should exist with Reclaimable.
    let mut query = world.query::<(&Reclaimable, &Position)>();
    let wrecks: Vec<_> = query.iter(&world).collect();
    assert_eq!(wrecks.len(), 1, "Should have spawned exactly one wreckage");

    let (reclaimable, wreck_pos) = wrecks[0];
    // metal_value = 50% of max health (200) = 100.
    assert_eq!(reclaimable.metal_value, sf(100));
    assert_eq!(reclaimable.reclaim_progress, SimFloat::ZERO);
    assert_eq!(wreck_pos.pos, pos.pos, "Wreckage at same position");
}

// -----------------------------------------------------------------------
// 4. Paralyzer stuns instead of killing
// -----------------------------------------------------------------------

#[test]
fn paralyzer_stuns_instead_of_killing() {
    let mut world = setup_world();

    let pos = pos3(10, 0, 10);
    let entity = world
        .spawn((
            pos.clone(),
            Health {
                current: 50,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, entity, &pos);

    // Fire paralyzer damage — should NOT reduce health.
    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(999),
            damage_type: DamageType::Paralyzer,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: true,
        });

    damage_system(&mut world);

    let health = world.get::<Health>(entity).unwrap();
    assert_eq!(health.current, 50, "Paralyzer should NOT reduce health");
    assert!(
        world.get::<Dead>(entity).is_none(),
        "Paralyzer should NOT kill"
    );
    assert!(world.get::<Stunned>(entity).is_some(), "Should be stunned");
    assert_eq!(
        world.get::<Stunned>(entity).unwrap().remaining_frames,
        PARALYZER_STUN_FRAMES
    );
}

// -----------------------------------------------------------------------
// 5. Armor multipliers apply correctly
// -----------------------------------------------------------------------

#[test]
fn armor_multipliers_apply() {
    let mut world = setup_world();

    // Heavy armor vs Explosive: multiplier = 1.5
    let pos = pos3(10, 0, 10);
    let entity = world
        .spawn((
            pos.clone(),
            Health {
                current: 100,
                max: 100,
            },
            ArmorClass::Heavy,
        ))
        .id();
    register_in_grid(&mut world, entity, &pos);

    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(20),
            damage_type: DamageType::Explosive,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });

    damage_system(&mut world);

    let health = world.get::<Health>(entity).unwrap();
    // 20 * 1.5 = 30 damage => 100 - 30 = 70
    assert_eq!(
        health.current,
        70,
        "Explosive vs Heavy should deal 1.5x damage"
    );
}

#[test]
fn armor_default_to_light_when_missing() {
    let mut world = setup_world();

    // No ArmorClass component => defaults to Light.
    // Laser vs Light: multiplier = 1.5
    let pos = pos3(10, 0, 10);
    let entity = world
        .spawn((
            pos.clone(),
            Health {
                current: 100,
                max: 100,
            },
        ))
        .id();
    register_in_grid(&mut world, entity, &pos);

    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(20),
            damage_type: DamageType::Laser,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });

    damage_system(&mut world);

    let health = world.get::<Health>(entity).unwrap();
    // 20 * 1.5 = 30 damage => 100 - 30 = 70
    assert_eq!(
        health.current,
        70,
        "No ArmorClass should default to Light (Laser 1.5x)"
    );
}

// -----------------------------------------------------------------------
// 6. Stun system ticks down and removes
// -----------------------------------------------------------------------

#[test]
fn stun_system_decrements_and_removes() {
    let mut world = World::new();

    let entity = world
        .spawn(Stunned {
            remaining_frames: 3,
        })
        .id();

    stun_system(&mut world);
    assert_eq!(world.get::<Stunned>(entity).unwrap().remaining_frames, 2);

    stun_system(&mut world);
    assert_eq!(world.get::<Stunned>(entity).unwrap().remaining_frames, 1);

    stun_system(&mut world);
    assert!(
        world.get::<Stunned>(entity).is_none(),
        "Stunned should be removed when remaining_frames reaches 0"
    );
}

// -----------------------------------------------------------------------
// 7. Determinism: same inputs produce same outputs
// -----------------------------------------------------------------------

#[test]
fn determinism_identical_runs() {
    fn run() -> Vec<(i32, bool, bool)> {
        let mut world = setup_world();

        // Spawn three units at known positions.
        let positions = [pos3(10, 0, 10), pos3(12, 0, 10), pos3(50, 0, 50)];
        let mut entities = Vec::new();
        for pos in &positions {
            let e = world
                .spawn((
                    pos.clone(),
                    Health {
                        current: 100,
                        max: 100,
                    },
                ))
                .id();
            register_in_grid(&mut world, e, pos);
            entities.push(e);
        }

        // AOE impact near first two, direct hit on third.
        world.resource_mut::<ImpactEventQueue>().events.extend([
            ImpactEvent {
                position: SimVec3::new(sf(11), sf(0), sf(10)),
                damage: sf(60),
                damage_type: DamageType::Normal,
                area_of_effect: sf(5),
                is_paralyzer: false,
            },
            ImpactEvent {
                position: SimVec3::new(sf(50), sf(0), sf(50)),
                damage: sf(120),
                damage_type: DamageType::Normal,
                area_of_effect: SimFloat::ZERO,
                is_paralyzer: false,
            },
        ]);

        damage_system(&mut world);

        // Collect results in entity-spawn order.
        entities
            .iter()
            .map(|&e| {
                let hp = world.get::<Health>(e).unwrap().current;
                let dead = world.get::<Dead>(e).is_some();
                let stunned = world.get::<Stunned>(e).is_some();
                (hp, dead, stunned)
            })
            .collect()
    }

    let a = run();
    let b = run();
    assert_eq!(a, b, "Damage system must be deterministic");
}

// -----------------------------------------------------------------------
// 8. Impact queue is cleared after processing
// -----------------------------------------------------------------------

#[test]
fn impact_queue_cleared() {
    let mut world = setup_world();

    world
        .resource_mut::<ImpactEventQueue>()
        .events
        .push(ImpactEvent {
            position: SimVec3::new(sf(10), sf(0), sf(10)),
            damage: sf(10),
            damage_type: DamageType::Normal,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });

    damage_system(&mut world);

    assert!(
        world.resource::<ImpactEventQueue>().events.is_empty(),
        "ImpactEventQueue should be cleared after damage_system"
    );
}
