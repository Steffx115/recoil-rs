use super::*;
use crate::combat_data::DamageType;
use crate::components::{Position, Velocity};
use crate::projectile::{ImpactEventQueue, Projectile, ProjectileType};
use crate::targeting::{FireEventQueue, WeaponRegistry};
use crate::{SimFloat, SimVec3};
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

fn sf(n: i32) -> SimFloat {
    SimFloat::from_int(n)
}

fn pos3(x: i32, y: i32, z: i32) -> Position {
    Position {
        pos: SimVec3::new(sf(x), sf(y), sf(z)),
    }
}

fn setup_world() -> World {
    let mut world = World::new();
    world.insert_resource(WeaponRegistry { defs: Vec::new() });
    world.insert_resource(FireEventQueue::default());
    world.insert_resource(ImpactEventQueue::default());
    world
}

fn spawn_test_projectile(world: &mut World, pos: SimVec3, damage: SimFloat) -> Entity {
    world
        .spawn((
            Projectile {
                projectile_type: ProjectileType::Homing,
                target_entity: Entity::PLACEHOLDER,
                target_pos: pos,
                damage,
                damage_type: DamageType::Normal,
                area_of_effect: SimFloat::ZERO,
                speed: sf(5),
                is_paralyzer: false,
                lifetime: 300,
                indirect_fire: false,
            },
            Position { pos },
            Velocity {
                vel: SimVec3::new(sf(5), SimFloat::ZERO, SimFloat::ZERO),
            },
        ))
        .id()
}

// -----------------------------------------------------------------------
// Shield component unit tests
// -----------------------------------------------------------------------

#[test]
fn shield_absorb_full_damage() {
    let mut shield = Shield {
        capacity: sf(100),
        current: sf(100),
        regen_rate: sf(1),
        radius: sf(50),
    };

    let excess = shield.absorb(sf(30));
    assert_eq!(excess, SimFloat::ZERO);
    assert_eq!(shield.current, sf(70));
}

#[test]
fn shield_absorb_exact_capacity() {
    let mut shield = Shield {
        capacity: sf(100),
        current: sf(50),
        regen_rate: sf(1),
        radius: sf(50),
    };

    let excess = shield.absorb(sf(50));
    assert_eq!(excess, SimFloat::ZERO);
    assert_eq!(shield.current, SimFloat::ZERO);
    assert!(!shield.is_active());
}

#[test]
fn shield_absorb_overflow_returns_excess() {
    let mut shield = Shield {
        capacity: sf(100),
        current: sf(30),
        regen_rate: sf(1),
        radius: sf(50),
    };

    let excess = shield.absorb(sf(50));
    assert_eq!(excess, sf(20));
    assert_eq!(shield.current, SimFloat::ZERO);
}

#[test]
fn shield_collapsed_does_not_absorb() {
    let mut shield = Shield {
        capacity: sf(100),
        current: SimFloat::ZERO,
        regen_rate: sf(1),
        radius: sf(50),
    };

    assert!(!shield.is_active());
    let excess = shield.absorb(sf(10));
    assert_eq!(excess, sf(10));
}

#[test]
fn shield_regenerates_up_to_capacity() {
    let mut shield = Shield {
        capacity: sf(100),
        current: sf(95),
        regen_rate: sf(10),
        radius: sf(50),
    };

    shield.regenerate();
    assert_eq!(shield.current, sf(100)); // Clamped to capacity.
}

#[test]
fn shield_regenerates_from_zero() {
    let mut shield = Shield {
        capacity: sf(100),
        current: SimFloat::ZERO,
        regen_rate: sf(5),
        radius: sf(50),
    };

    shield.regenerate();
    assert_eq!(shield.current, sf(5));
    assert!(shield.is_active());
}

// -----------------------------------------------------------------------
// shield_regen_system tests
// -----------------------------------------------------------------------

#[test]
fn regen_system_restores_shield_energy() {
    let mut world = setup_world();

    let entity = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(100),
                current: sf(50),
                regen_rate: sf(10),
                radius: sf(50),
            },
        ))
        .id();

    shield_regen_system(&mut world);

    let shield = world.get::<Shield>(entity).unwrap();
    assert_eq!(shield.current, sf(60));
}

// -----------------------------------------------------------------------
// shield_absorb_system tests
// -----------------------------------------------------------------------

#[test]
fn absorb_system_destroys_projectile_inside_shield() {
    let mut world = setup_world();

    // Shielded entity at origin with radius 50.
    world.spawn((
        pos3(0, 0, 0),
        Shield {
            capacity: sf(200),
            current: sf(200),
            regen_rate: sf(1),
            radius: sf(50),
        },
    ));

    // Projectile inside the shield radius.
    let proj = spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(30));

    shield_absorb_system(&mut world);

    // Projectile should be despawned.
    assert!(
        world.get::<Projectile>(proj).is_none(),
        "Projectile should be despawned after full absorption"
    );
}

#[test]
fn absorb_system_reduces_shield_energy() {
    let mut world = setup_world();

    let shielded = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(200),
                current: sf(200),
                regen_rate: sf(1),
                radius: sf(50),
            },
        ))
        .id();

    // Projectile inside shield radius with 80 damage.
    spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(80));

    shield_absorb_system(&mut world);

    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, sf(120));
}

#[test]
fn absorb_system_partial_absorb_reduces_projectile_damage() {
    let mut world = setup_world();

    world.spawn((
        pos3(0, 0, 0),
        Shield {
            capacity: sf(200),
            current: sf(30),
            regen_rate: sf(1),
            radius: sf(50),
        },
    ));

    // Projectile with 80 damage -- shield can only absorb 30.
    let proj = spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(80));

    shield_absorb_system(&mut world);

    // Projectile should still exist with reduced damage.
    let proj_data = world.get::<Projectile>(proj).unwrap();
    assert_eq!(proj_data.damage, sf(50));
}

#[test]
fn absorb_system_ignores_projectile_outside_radius() {
    let mut world = setup_world();

    let shielded = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(200),
                current: sf(200),
                regen_rate: sf(1),
                radius: sf(50),
            },
        ))
        .id();

    // Projectile far outside shield radius.
    let proj = spawn_test_projectile(&mut world, SimVec3::new(sf(100), sf(0), sf(0)), sf(80));

    shield_absorb_system(&mut world);

    // Projectile should still exist with original damage.
    let proj_data = world.get::<Projectile>(proj).unwrap();
    assert_eq!(proj_data.damage, sf(80));

    // Shield should be unchanged.
    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, sf(200));
}

#[test]
fn absorb_system_collapsed_shield_does_not_absorb() {
    let mut world = setup_world();

    world.spawn((
        pos3(0, 0, 0),
        Shield {
            capacity: sf(200),
            current: SimFloat::ZERO,
            regen_rate: sf(1),
            radius: sf(50),
        },
    ));

    // Projectile inside radius but shield is collapsed.
    let proj = spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(80));

    shield_absorb_system(&mut world);

    // Projectile should pass through.
    let proj_data = world.get::<Projectile>(proj).unwrap();
    assert_eq!(proj_data.damage, sf(80));
}

#[test]
fn shield_collapses_when_energy_depleted() {
    let mut world = setup_world();

    let shielded = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(100),
                current: sf(20),
                regen_rate: sf(5),
                radius: sf(50),
            },
        ))
        .id();

    // Hit with exactly 20 damage -- shield should collapse.
    spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(20));

    shield_absorb_system(&mut world);

    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, SimFloat::ZERO);
    assert!(!shield.is_active());
}

#[test]
fn shield_regens_after_collapse_and_absorbs_again() {
    let mut world = setup_world();

    let shielded = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(100),
                current: SimFloat::ZERO,
                regen_rate: sf(25),
                radius: sf(50),
            },
        ))
        .id();

    // Regen for a few ticks.
    for _ in 0..4 {
        shield_regen_system(&mut world);
    }

    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, sf(100));
    assert!(shield.is_active());

    // Now a projectile should be absorbed.
    let proj = spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(40));

    shield_absorb_system(&mut world);

    assert!(
        world.get::<Projectile>(proj).is_none(),
        "Projectile should be absorbed after shield regens"
    );
    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, sf(60));
}

#[test]
fn multiple_projectiles_drain_shield_sequentially() {
    let mut world = setup_world();

    let shielded = world
        .spawn((
            pos3(0, 0, 0),
            Shield {
                capacity: sf(100),
                current: sf(100),
                regen_rate: sf(1),
                radius: sf(50),
            },
        ))
        .id();

    // Three projectiles inside radius.
    let p1 = spawn_test_projectile(&mut world, SimVec3::new(sf(5), sf(0), sf(0)), sf(30));
    let p2 = spawn_test_projectile(&mut world, SimVec3::new(sf(10), sf(0), sf(0)), sf(30));
    let p3 = spawn_test_projectile(&mut world, SimVec3::new(sf(15), sf(0), sf(0)), sf(30));

    shield_absorb_system(&mut world);

    // Shield started at 100, absorbed 30+30+30=90, should have 10 left.
    let shield = world.get::<Shield>(shielded).unwrap();
    assert_eq!(shield.current, sf(10));

    // All three projectiles should be absorbed.
    assert!(world.get::<Projectile>(p1).is_none());
    assert!(world.get::<Projectile>(p2).is_none());
    assert!(world.get::<Projectile>(p3).is_none());
}
