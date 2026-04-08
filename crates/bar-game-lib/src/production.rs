//! Factory production: queuing units by name.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use pierce_sim::factory::BuildQueue;
use pierce_sim::unit_defs::UnitDefRegistry;

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
#[path = "production_tests.rs"]
mod tests;
