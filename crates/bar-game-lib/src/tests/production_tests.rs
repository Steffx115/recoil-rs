use super::*;
use pierce_sim::factory::{UnitBlueprint, UnitRegistry};
use pierce_sim::unit_defs::UnitDef;
use std::collections::VecDeque;

use pierce_math::SimFloat;
use pierce_sim::{Allegiance, Health, Position, UnitType};

#[test]
fn test_queue_unit_by_id() {
    let mut world = bevy_ecs::world::World::new();

    // Create a factory entity with BuildQueue
    let factory = world
        .spawn((
            pierce_sim::factory::BuildQueue {
                queue: VecDeque::new(),
                current_progress: SimFloat::ZERO,
                rally_point: pierce_math::SimVec3::ZERO,
                repeat: false,
            },
            Position {
                pos: pierce_math::SimVec3::ZERO,
            },
            Allegiance { team: 0 },
            UnitType { id: 50002 },
            Health {
                current: 500,
                max: 500,
            },
        ))
        .id();

    queue_unit(&mut world, factory, 42);

    let bq = world.get::<BuildQueue>(factory).unwrap();
    assert_eq!(bq.queue.len(), 1);
    assert_eq!(bq.queue[0], 42);
}

#[test]
fn test_queue_unit_by_name() {
    let mut world = bevy_ecs::world::World::new();

    // Insert a UnitDefRegistry with a test unit
    let mut registry = UnitDefRegistry::default();
    let def = UnitDef {
        name: "testbot".to_string(),
        unit_type_id: 77,
        max_health: 200.0,
        armor_class: "Light".to_string(),
        sight_range: 500.0,
        collision_radius: 10.0,
        max_speed: 2.0,
        acceleration: 1.0,
        turn_rate: 1.0,
        metal_cost: 50.0,
        energy_cost: 100.0,
        build_time: 60,
        weapons: vec![],
        model_path: None,
        icon_path: None,
        categories: vec![],
        can_build: vec![],
        can_build_names: vec![],
        build_power: None,
        metal_production: None,
        energy_production: None,
        is_building: false,
        is_builder: false,
    };
    registry.defs.insert(77, def);
    world.insert_resource(registry);

    // Insert a UnitRegistry (needed for factory)
    let mut unit_reg = UnitRegistry::default();
    unit_reg.blueprints.push(UnitBlueprint {
        unit_type_id: 77,
        metal_cost: SimFloat::from_int(50),
        energy_cost: SimFloat::from_int(100),
        build_time: 60,
        max_health: 200,
    });
    world.insert_resource(unit_reg);

    let factory = world
        .spawn((
            BuildQueue {
                queue: VecDeque::new(),
                current_progress: SimFloat::ZERO,
                rally_point: pierce_math::SimVec3::ZERO,
                repeat: false,
            },
            Position {
                pos: pierce_math::SimVec3::ZERO,
            },
            Allegiance { team: 0 },
            UnitType { id: 50002 },
            Health {
                current: 500,
                max: 500,
            },
        ))
        .id();

    queue_unit_by_name(&mut world, factory, "testbot");

    let bq = world.get::<BuildQueue>(factory).unwrap();
    assert_eq!(bq.queue.len(), 1);
    assert_eq!(bq.queue[0], 77);
}
