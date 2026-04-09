use super::*;
use bevy_ecs::world::World;

/// Helper: spawn an entity with the given bundle, then read back the
/// component and run an assertion closure on it.
fn roundtrip<C: Component + std::fmt::Debug + Clone>(component: C) -> C {
    let mut world = World::new();
    let entity = world.spawn(component).id();
    world.get::<C>(entity).unwrap().clone()
}

#[test]
fn sim_id_roundtrip() {
    let c = roundtrip(SimId { id: 42 });
    assert_eq!(c.id, 42);
}

#[test]
fn position_roundtrip() {
    let c = roundtrip(Position { pos: SimVec3::ZERO });
    assert_eq!(c.pos, SimVec3::ZERO);
}

#[test]
fn velocity_roundtrip() {
    let v = SimVec3::new(SimFloat::ONE, SimFloat::TWO, SimFloat::ZERO);
    let c = roundtrip(Velocity { vel: v });
    assert_eq!(c.vel, v);
}

#[test]
fn heading_roundtrip() {
    let c = roundtrip(Heading {
        angle: pierce_math::Angle::HALF,
    });
    assert_eq!(c.angle, pierce_math::Angle::HALF);
}

#[test]
fn health_roundtrip() {
    let c = roundtrip(Health {
        current: 80,
        max: 100,
    });
    assert_eq!(c.current, 80);
    assert_eq!(c.max, 100);
}

#[test]
fn dead_marker() {
    let mut world = World::new();
    let entity = world.spawn(Dead).id();
    assert!(world.get::<Dead>(entity).is_some());
}

#[test]
fn unit_type_roundtrip() {
    let c = roundtrip(UnitType { id: 7 });
    assert_eq!(c.id, 7);
}

#[test]
fn allegiance_roundtrip() {
    let c = roundtrip(Allegiance { team: 3 });
    assert_eq!(c.team, 3);
}

#[test]
fn build_progress_roundtrip() {
    let c = roundtrip(BuildProgress {
        progress: SimFloat::ZERO,
        total_cost: SimFloat::from_int(500),
    });
    assert_eq!(c.progress, SimFloat::ZERO);
    assert_eq!(c.total_cost, SimFloat::from_int(500));
}

#[test]
fn cloaked_roundtrip() {
    let c = roundtrip(Cloaked {
        cloak_cost: SimFloat::ONE,
    });
    assert_eq!(c.cloak_cost, SimFloat::ONE);
}

#[test]
fn transport_roundtrip() {
    let c = roundtrip(Transport {
        capacity: 8,
        passengers: Vec::new(),
    });
    assert_eq!(c.capacity, 8);
    assert!(c.passengers.is_empty());
}

#[test]
fn spawn_all_components_on_one_entity() {
    let mut world = World::new();
    let passenger = world.spawn_empty().id();

    let entity = world
        .spawn((
            SimId { id: 1 },
            Position { pos: SimVec3::ZERO },
            Velocity { vel: SimVec3::ZERO },
            Heading {
                angle: pierce_math::Angle::ZERO,
            },
            Health {
                current: 100,
                max: 100,
            },
            UnitType { id: 5 },
            Allegiance { team: 1 },
            BuildProgress {
                progress: SimFloat::ZERO,
                total_cost: SimFloat::from_int(200),
            },
            Cloaked {
                cloak_cost: SimFloat::HALF,
            },
            Transport {
                capacity: 4,
                passengers: vec![passenger],
            },
        ))
        .id();

    assert!(world.get::<SimId>(entity).is_some());
    assert!(world.get::<Position>(entity).is_some());
    assert!(world.get::<Velocity>(entity).is_some());
    assert!(world.get::<Heading>(entity).is_some());
    assert!(world.get::<Health>(entity).is_some());
    assert!(world.get::<UnitType>(entity).is_some());
    assert!(world.get::<Allegiance>(entity).is_some());
    assert!(world.get::<BuildProgress>(entity).is_some());
    assert!(world.get::<Cloaked>(entity).is_some());

    let transport = world.get::<Transport>(entity).unwrap();
    assert_eq!(transport.capacity, 4);
    assert_eq!(transport.passengers.len(), 1);
    assert_eq!(transport.passengers[0], passenger);
}
