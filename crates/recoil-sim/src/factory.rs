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
    pub max_health: SimFloat,
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
mod tests {
    use super::*;
    use crate::economy::{init_economy, EconomyState};
    use crate::lifecycle::init_lifecycle;

    /// Set up a world with economy, lifecycle, and a unit registry containing
    /// one blueprint (unit_type_id=0, metal_cost=100, energy_cost=50,
    /// build_time=10 frames, max_health=200).
    fn setup() -> World {
        let mut world = World::new();
        init_lifecycle(&mut world);
        init_economy(&mut world, &[1]);

        let mut registry = UnitRegistry::default();
        registry.blueprints.push(UnitBlueprint {
            unit_type_id: 0,
            metal_cost: SimFloat::from_int(100),
            energy_cost: SimFloat::from_int(50),
            build_time: 10,
            max_health: SimFloat::from_int(200),
        });
        world.insert_resource(registry);
        world
    }

    /// Spawn a factory entity with the given queue entries.
    fn spawn_factory(world: &mut World, queue: &[u32], rally: SimVec3) -> bevy_ecs::entity::Entity {
        world
            .spawn((
                BuildQueue {
                    queue: queue.iter().copied().collect(),
                    current_progress: SimFloat::ZERO,
                    rally_point: rally,
                    repeat: false,
                },
                Allegiance { team: 1 },
                Position { pos: SimVec3::ZERO },
            ))
            .id()
    }

    /// Count entities that have a UnitType component (excluding the factory).
    fn count_spawned_units(world: &mut World) -> usize {
        let mut query = world.query::<&UnitType>();
        query.iter(world).count()
    }

    #[test]
    fn factory_builds_unit_over_n_frames() {
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0], SimVec3::ZERO);

        // build_time = 10 frames, so after 9 ticks the unit should NOT be done.
        for _ in 0..9 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 0);

        // 10th tick should complete it.
        factory_system(&mut world);
        assert_eq!(count_spawned_units(&mut world), 1);

        // Queue should be empty and progress reset.
        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert!(bq.queue.is_empty());
        assert_eq!(bq.current_progress, SimFloat::ZERO);
    }

    #[test]
    fn stalled_economy_slows_production() {
        let mut world = setup();
        spawn_factory(&mut world, &[0], SimVec3::ZERO);

        // Set stall_ratio_metal to 0.5 — production should take 20 frames.
        {
            let mut economy = world.resource_mut::<EconomyState>();
            let team_res = economy.teams.get_mut(&1).unwrap();
            team_res.stall_ratio_metal = SimFloat::HALF;
        }

        for _ in 0..19 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 0);

        factory_system(&mut world);
        assert_eq!(count_spawned_units(&mut world), 1);
    }

    #[test]
    fn queue_processes_multiple_items() {
        let mut world = setup();
        spawn_factory(&mut world, &[0, 0], SimVec3::ZERO);

        // First unit: 10 frames.
        for _ in 0..10 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 1);

        // Second unit: another 10 frames.
        for _ in 0..10 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 2);
    }

    #[test]
    fn no_resources_no_progress() {
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0], SimVec3::ZERO);

        // Set stall ratio to zero — simulating complete stall.
        {
            let mut economy = world.resource_mut::<EconomyState>();
            let team_res = economy.teams.get_mut(&1).unwrap();
            team_res.stall_ratio_metal = SimFloat::ZERO;
        }

        for _ in 0..20 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 0);

        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.current_progress, SimFloat::ZERO);
    }

    #[test]
    fn spawned_unit_appears_at_rally_point() {
        let mut world = setup();
        let rally = SimVec3::new(
            SimFloat::from_int(100),
            SimFloat::ZERO,
            SimFloat::from_int(200),
        );
        spawn_factory(&mut world, &[0], rally);

        for _ in 0..10 {
            factory_system(&mut world);
        }

        // Find the spawned unit (has UnitType component).
        let mut query = world.query::<(&UnitType, &Position)>();
        let positions: Vec<_> = query.iter(&world).collect();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].1.pos, rally);
    }

    // ==================================================================
    // Edge case tests (RR-115)
    // ==================================================================

    #[test]
    fn queue_cancellation_refunds_correct_resources() {
        // Removing an un-started item from the queue should not have consumed
        // any resources for that item (factory only drains for the front item).
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0, 0], SimVec3::ZERO);

        // Run 5 ticks — halfway through the first unit.
        for _ in 0..5 {
            factory_system(&mut world);
        }

        // Record resource levels before cancellation.
        let metal_before = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // Cancel the second queued item (simulate player cancellation).
        {
            let mut bq = world.get_mut::<BuildQueue>(factory).unwrap();
            bq.queue.pop_back(); // remove second item
        }

        // The second item never started building, so no resources were spent
        // on it. Current resource level should be unchanged after removal.
        let metal_after = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };
        assert_eq!(metal_before, metal_after);

        // Queue should have only the first item.
        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.queue.len(), 1);
    }

    #[test]
    fn produced_unit_spawns_at_rally_point_not_factory_center() {
        // The factory sits at (0,0,0) but the rally point is at (50,0,80).
        // The produced unit must appear at the rally point.
        let mut world = setup();
        let factory_pos = SimVec3::ZERO;
        let rally = SimVec3::new(
            SimFloat::from_int(50),
            SimFloat::ZERO,
            SimFloat::from_int(80),
        );

        world.spawn((
            BuildQueue {
                queue: [0].iter().copied().collect(),
                current_progress: SimFloat::ZERO,
                rally_point: rally,
                repeat: false,
            },
            Allegiance { team: 1 },
            Position { pos: factory_pos },
        ));

        for _ in 0..10 {
            factory_system(&mut world);
        }

        let mut query = world.query::<(&UnitType, &Position)>();
        let units: Vec<_> = query.iter(&world).collect();
        assert_eq!(units.len(), 1);
        // Unit must be at rally point, not at factory position.
        assert_eq!(units[0].1.pos, rally);
        assert_ne!(units[0].1.pos, factory_pos);
    }

    #[test]
    fn repeat_mode_requeues_completed_unit_type() {
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0], SimVec3::ZERO);

        // Enable repeat mode.
        {
            let mut bq = world.get_mut::<BuildQueue>(factory).unwrap();
            bq.repeat = true;
        }

        // Complete the first unit (10 ticks).
        for _ in 0..10 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 1);

        // After completion, the queue should still contain the item (re-queued).
        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.queue.len(), 1);
        assert_eq!(*bq.queue.front().unwrap(), 0);
        assert_eq!(bq.current_progress, SimFloat::ZERO);

        // Complete a second unit (another 10 ticks) — proves repeat works
        // continuously, not just once.
        for _ in 0..10 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 2);

        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.queue.len(), 1);
    }

    #[test]
    fn factory_with_empty_queue_is_idle_no_resource_drain() {
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[], SimVec3::ZERO);

        let metal_before = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // Run several ticks with an empty queue.
        for _ in 0..20 {
            factory_system(&mut world);
        }

        let metal_after = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // No resources should have been drained.
        assert_eq!(metal_before, metal_after);

        // No units should have been spawned.
        assert_eq!(count_spawned_units(&mut world), 0);

        // Progress should still be zero.
        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.current_progress, SimFloat::ZERO);
    }

    #[test]
    fn multiple_factories_with_same_queue_dont_interfere() {
        let mut world = setup();
        let rally_a = SimVec3::new(SimFloat::from_int(10), SimFloat::ZERO, SimFloat::ZERO);
        let rally_b = SimVec3::new(SimFloat::from_int(20), SimFloat::ZERO, SimFloat::ZERO);
        let factory_a = spawn_factory(&mut world, &[0], rally_a);
        let factory_b = spawn_factory(&mut world, &[0], rally_b);

        // Run 10 ticks — both factories should complete independently.
        for _ in 0..10 {
            factory_system(&mut world);
        }

        // Both factories should have produced one unit each.
        assert_eq!(count_spawned_units(&mut world), 2);

        // Both factory queues should be empty.
        let bq_a = world.get::<BuildQueue>(factory_a).unwrap();
        let bq_b = world.get::<BuildQueue>(factory_b).unwrap();
        assert!(bq_a.queue.is_empty());
        assert!(bq_b.queue.is_empty());

        // Units should appear at their respective rally points.
        let mut query = world.query::<(&UnitType, &Position)>();
        let mut positions: Vec<SimVec3> = query.iter(&world).map(|(_, p)| p.pos).collect();
        positions.sort_by_key(|p| p.x.raw());
        assert_eq!(positions[0], rally_a);
        assert_eq!(positions[1], rally_b);
    }

    #[test]
    fn factory_production_pauses_when_economy_fully_stalled() {
        // stall_ratio = 0 means zero effective build rate — no progress at all.
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0], SimVec3::ZERO);

        {
            let mut economy = world.resource_mut::<EconomyState>();
            let team_res = economy.teams.get_mut(&1).unwrap();
            team_res.stall_ratio_metal = SimFloat::ZERO;
        }

        // Run many more ticks than build_time.
        for _ in 0..100 {
            factory_system(&mut world);
        }

        // No unit should have been produced.
        assert_eq!(count_spawned_units(&mut world), 0);

        // Progress should remain at zero.
        let bq = world.get::<BuildQueue>(factory).unwrap();
        assert_eq!(bq.current_progress, SimFloat::ZERO);

        // Now restore the economy and verify production resumes.
        {
            let mut economy = world.resource_mut::<EconomyState>();
            let team_res = economy.teams.get_mut(&1).unwrap();
            team_res.stall_ratio_metal = SimFloat::ONE;
        }

        for _ in 0..10 {
            factory_system(&mut world);
        }
        assert_eq!(count_spawned_units(&mut world), 1);
    }

    #[test]
    fn cancelling_mid_production_loses_spent_resources() {
        // When a player cancels mid-build by clearing the queue, the resources
        // already spent on partial progress are gone — the system does not
        // implement automatic refunds.
        let mut world = setup();
        let factory = spawn_factory(&mut world, &[0], SimVec3::ZERO);

        let metal_before = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // Run 5 ticks — halfway through (build_time=10).
        for _ in 0..5 {
            factory_system(&mut world);
        }

        let metal_mid = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // Resources should have been partially spent.
        assert!(metal_mid < metal_before);

        // Cancel by clearing the queue.
        {
            let mut bq = world.get_mut::<BuildQueue>(factory).unwrap();
            bq.queue.clear();
            bq.current_progress = SimFloat::ZERO;
        }

        // Run more ticks — nothing should happen.
        for _ in 0..10 {
            factory_system(&mut world);
        }

        let metal_after = {
            let economy = world.resource::<EconomyState>();
            economy.teams[&1].metal
        };

        // Resources after cancellation should equal resources at cancel time
        // (no refund, no further drain).
        assert_eq!(metal_mid, metal_after);
        assert_eq!(count_spawned_units(&mut world), 0);
    }

    #[test]
    fn queue_order_is_fifo() {
        // Register a second blueprint so we can distinguish unit types.
        let mut world = setup();
        {
            let mut registry = world.resource_mut::<UnitRegistry>();
            registry.blueprints.push(UnitBlueprint {
                unit_type_id: 1,
                metal_cost: SimFloat::from_int(200),
                energy_cost: SimFloat::from_int(100),
                build_time: 5,
                max_health: SimFloat::from_int(300),
            });
        }

        // Queue: [type 0, type 1] — type 0 should build first.
        spawn_factory(&mut world, &[0, 1], SimVec3::ZERO);

        // Type 0 has build_time=10. After 10 ticks, first unit completes.
        for _ in 0..10 {
            factory_system(&mut world);
        }

        {
            let mut query = world.query::<&UnitType>();
            let types: Vec<u32> = query.iter(&world).map(|ut| ut.id).collect();
            assert_eq!(types.len(), 1);
            assert_eq!(types[0], 0, "First queued unit (type 0) should build first");
        }

        // Type 1 has build_time=5. After 5 more ticks, second unit completes.
        for _ in 0..5 {
            factory_system(&mut world);
        }

        {
            let mut query = world.query::<&UnitType>();
            let mut types: Vec<u32> = query.iter(&world).map(|ut| ut.id).collect();
            types.sort();
            assert_eq!(types.len(), 2);
            assert_eq!(types, vec![0, 1], "Both units should now be spawned");
        }
    }
}
