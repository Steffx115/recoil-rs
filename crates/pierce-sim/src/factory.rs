//! Factory production and build queues.
//!
//! Factories are entities with a [`BuildQueue`] component that produce units
//! over time, consuming metal and energy from the team's economy.  The
//! [`factory_system`] drives production each tick, respecting economy stall
//! ratios so that resource shortages proportionally slow build speed.

use std::collections::VecDeque;

use bevy_ecs::prelude::Component;
use bevy_ecs::system::Resource;
use bevy_ecs::world::World;

use crate::components::{Allegiance, Health, Position, UnitType};
use crate::economy::EconomyState;
use crate::lifecycle::spawn_unit;
use crate::{SimFloat, SimVec3};

// ---------------------------------------------------------------------------
// Data definitions
// ---------------------------------------------------------------------------

/// Blueprint describing a unit type's cost and build parameters.
///
/// Stored in the [`UnitRegistry`] resource, indexed by `unit_type_id`.
#[derive(Debug, Clone)]
pub struct UnitBlueprint {
    pub unit_type_id: u32,
    pub metal_cost: SimFloat,
    pub energy_cost: SimFloat,
    /// Frames to complete at full build power (no stall).
    pub build_time: u32,
    pub max_health: i32,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Registry of all unit blueprints, indexed by `unit_type_id`.
#[derive(Resource, Debug, Clone, Default)]
pub struct UnitRegistry {
    pub blueprints: Vec<UnitBlueprint>,
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A factory's build queue and current production state.
#[derive(Component, Debug, Clone)]
pub struct BuildQueue {
    /// Unit type IDs waiting to be built (front = current).
    pub queue: VecDeque<u32>,
    /// Progress on the current item, in the range `[0, 1]`.
    pub current_progress: SimFloat,
    /// Where newly built units are spawned.
    pub rally_point: SimVec3,
    /// When true, completed items are re-appended to the back of the queue.
    pub repeat: bool,
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Advance factory production for one tick.
///
/// For each entity with (`BuildQueue`, `Allegiance`, `Position`):
/// 1. Skip if the queue is empty.
/// 2. Look up the current item's [`UnitBlueprint`] from [`UnitRegistry`].
/// 3. Calculate build rate as `1 / build_time` (progress per frame).
/// 4. Multiply by the team's `stall_ratio_metal` from [`EconomyState`].
/// 5. Deduct per-frame metal and energy costs from the team's resources.
/// 6. Accumulate `current_progress`.
/// 7. When progress reaches 1.0, spawn the unit and advance the queue.
pub fn factory_system(world: &mut World) {
    // Collect factory entities and their relevant data so we can mutate the
    // world freely afterwards.
    struct FactoryWork {
        queue_front: u32,
        team: u8,
        rally_point: SimVec3,
    }

    let mut work: Vec<(bevy_ecs::entity::Entity, FactoryWork)> = Vec::new();

    {
        let mut query = world.query::<(
            bevy_ecs::entity::Entity,
            &BuildQueue,
            &Allegiance,
            &Position,
        )>();
        for (entity, bq, allegiance, _pos) in query.iter(world) {
            if let Some(&front) = bq.queue.front() {
                work.push((
                    entity,
                    FactoryWork {
                        queue_front: front,
                        team: allegiance.team,
                        rally_point: bq.rally_point,
                    },
                ));
            }
        }
    }

    // Process each factory.
    for (entity, fw) in work {
        // Look up blueprint.
        let blueprint = {
            let registry = world.resource::<UnitRegistry>();
            registry
                .blueprints
                .iter()
                .find(|bp| bp.unit_type_id == fw.queue_front)
                .cloned()
        };
        let Some(blueprint) = blueprint else {
            continue;
        };

        // Calculate build rate using ceiling division so that
        // build_time * rate >= ONE even when SCALE isn't evenly divisible.
        let bt = blueprint.build_time as i64;
        let base_rate = SimFloat::from_raw((SimFloat::ONE.raw() + bt - 1) / bt);

        // Get stall ratio for this team.
        let stall_ratio = {
            let economy = world.resource::<EconomyState>();
            economy
                .teams
                .get(&fw.team)
                .map(|t| t.stall_ratio_metal)
                .unwrap_or(SimFloat::ZERO)
        };

        let effective_rate = base_rate * stall_ratio;

        // Deduct resources from the team.
        let metal_per_frame = blueprint.metal_cost * effective_rate;
        let energy_per_frame = blueprint.energy_cost * effective_rate;

        {
            let mut economy = world.resource_mut::<EconomyState>();
            if let Some(team_res) = economy.teams.get_mut(&fw.team) {
                team_res.metal = (team_res.metal - metal_per_frame).max(SimFloat::ZERO);
                team_res.energy = (team_res.energy - energy_per_frame).max(SimFloat::ZERO);
            }
        }

        // Advance progress.
        let mut bq = world.get_mut::<BuildQueue>(entity).unwrap();
        bq.current_progress += effective_rate;

        if bq.current_progress >= SimFloat::ONE {
            let completed = bq.queue.pop_front();
            if bq.repeat {
                if let Some(id) = completed {
                    bq.queue.push_back(id);
                }
            }
            bq.current_progress = SimFloat::ZERO;

            // Spawn the completed unit at the rally point.
            let rally = fw.rally_point;
            spawn_unit(
                world,
                Position { pos: rally },
                UnitType {
                    id: blueprint.unit_type_id,
                },
                Allegiance { team: fw.team },
                Health {
                    current: blueprint.max_health,
                    max: blueprint.max_health,
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/factory_tests.rs"]
mod tests;
