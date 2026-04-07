//! Factory production: queuing units by name.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use recoil_sim::factory::BuildQueue;
use recoil_sim::unit_defs::UnitDefRegistry;

/// Queue a unit for production in a factory, looked up by name.
pub fn queue_unit_by_name(world: &mut World, factory_entity: Entity, unit_name: &str) {
    let unit_type_id = {
        let registry = world.resource::<UnitDefRegistry>();
        registry
            .defs
            .values()
            .find(|d| d.name.to_lowercase() == unit_name)
            .map(|d| d.unit_type_id)
    };

    if let Some(type_id) = unit_type_id {
        queue_unit(world, factory_entity, type_id);
        tracing::info!("Queued {} (id={}) in factory", unit_name, type_id);
    } else {
        tracing::warn!("Unit def '{}' not found", unit_name);
    }
}

/// Queue a unit for production in a factory by type ID.
pub fn queue_unit(world: &mut World, factory_entity: Entity, unit_type_id: u32) {
    if let Some(mut bq) = world.get_mut::<BuildQueue>(factory_entity) {
        bq.queue.push_back(unit_type_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recoil_sim::factory::{UnitBlueprint, UnitRegistry};
    use recoil_sim::unit_defs::UnitDef;
    use std::collections::VecDeque;

    use recoil_math::SimFloat;
    use recoil_sim::{Allegiance, Health, Position, UnitType};

    #[test]
    fn test_queue_unit_by_id() {
        let mut world = bevy_ecs::world::World::new();

        // Create a factory entity with BuildQueue
        let factory = world
            .spawn((
                recoil_sim::factory::BuildQueue {
                    queue: VecDeque::new(),
                    current_progress: SimFloat::ZERO,
                    rally_point: recoil_math::SimVec3::ZERO,
                    repeat: false,
                },
                Position {
                    pos: recoil_math::SimVec3::ZERO,
                },
                Allegiance { team: 0 },
                UnitType { id: 50002 },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
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
            max_health: SimFloat::from_int(200),
        });
        world.insert_resource(unit_reg);

        let factory = world
            .spawn((
                BuildQueue {
                    queue: VecDeque::new(),
                    current_progress: SimFloat::ZERO,
                    rally_point: recoil_math::SimVec3::ZERO,
                    repeat: false,
                },
                Position {
                    pos: recoil_math::SimVec3::ZERO,
                },
                Allegiance { team: 0 },
                UnitType { id: 50002 },
                Health {
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
            ))
            .id();

        queue_unit_by_name(&mut world, factory, "testbot");

        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.queue.len(), 1);
        assert_eq!(bq.queue[0], 77);
    }
}
