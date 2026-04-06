//! Metal/energy economy system.
//!
//! Each team tracks metal and energy resources.  [`ResourceProducer`] and
//! [`ResourceConsumer`] components on entities generate income and expense
//! respectively, aggregated per team by [`economy_system`].  When a team
//! cannot afford its total expense the system applies a proportional stall.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;

use crate::components::Allegiance;
use crate::SimFloat;

// ---------------------------------------------------------------------------
// Resource bookkeeping (stored as a World resource, not a component)
// ---------------------------------------------------------------------------

/// Per-team resource state.
#[derive(Debug, Clone)]
pub struct TeamResources {
    pub metal: SimFloat,
    pub metal_storage: SimFloat,
    pub metal_income: SimFloat,
    pub metal_expense: SimFloat,
    pub energy: SimFloat,
    pub energy_storage: SimFloat,
    pub energy_income: SimFloat,
    pub energy_expense: SimFloat,
    pub stall_ratio_metal: SimFloat,
    pub stall_ratio_energy: SimFloat,
}

impl Default for TeamResources {
    fn default() -> Self {
        Self {
            metal: SimFloat::ZERO,
            metal_storage: SimFloat::ZERO,
            metal_income: SimFloat::ZERO,
            metal_expense: SimFloat::ZERO,
            energy: SimFloat::ZERO,
            energy_storage: SimFloat::ZERO,
            energy_income: SimFloat::ZERO,
            energy_expense: SimFloat::ZERO,
            stall_ratio_metal: SimFloat::ONE,
            stall_ratio_energy: SimFloat::ONE,
        }
    }
}

/// World resource holding per-team economies, keyed by team id.
#[derive(Debug, Clone, Default, bevy_ecs::system::Resource)]
pub struct EconomyState {
    pub teams: BTreeMap<u8, TeamResources>,
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Marks an entity that produces resources each tick.
#[derive(Component, Debug, Clone)]
pub struct ResourceProducer {
    pub metal_per_tick: SimFloat,
    pub energy_per_tick: SimFloat,
}

/// Marks an entity that consumes resources each tick.
#[derive(Component, Debug, Clone)]
pub struct ResourceConsumer {
    pub metal_per_tick: SimFloat,
    pub energy_per_tick: SimFloat,
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Runs one tick of the economy: accumulates income/expense per team,
/// applies stall when expenses exceed available resources, and clamps
/// totals to storage capacity.
pub fn economy_system(world: &mut World) {
    // --- 1. Reset income / expense / stall for every team ---------------
    {
        let mut state = world.resource_mut::<EconomyState>();
        for res in state.teams.values_mut() {
            res.metal_income = SimFloat::ZERO;
            res.metal_expense = SimFloat::ZERO;
            res.energy_income = SimFloat::ZERO;
            res.energy_expense = SimFloat::ZERO;
            res.stall_ratio_metal = SimFloat::ONE;
            res.stall_ratio_energy = SimFloat::ONE;
        }
    }

    // --- 2. Accumulate producer income per team -------------------------
    let mut income_map: BTreeMap<u8, (SimFloat, SimFloat)> = BTreeMap::new();
    let mut query = world.query::<(&Allegiance, &ResourceProducer)>();
    for (allegiance, producer) in query.iter(world) {
        let entry = income_map
            .entry(allegiance.team)
            .or_insert((SimFloat::ZERO, SimFloat::ZERO));
        entry.0 += producer.metal_per_tick;
        entry.1 += producer.energy_per_tick;
    }

    // --- 3. Accumulate consumer expense per team ------------------------
    let mut expense_map: BTreeMap<u8, (SimFloat, SimFloat)> = BTreeMap::new();
    let mut query = world.query::<(&Allegiance, &ResourceConsumer)>();
    for (allegiance, consumer) in query.iter(world) {
        let entry = expense_map
            .entry(allegiance.team)
            .or_insert((SimFloat::ZERO, SimFloat::ZERO));
        entry.0 += consumer.metal_per_tick;
        entry.1 += consumer.energy_per_tick;
    }

    // --- 4. Apply to each team ------------------------------------------
    let mut state = world.resource_mut::<EconomyState>();
    for (team, res) in state.teams.iter_mut() {
        if let Some(&(m, e)) = income_map.get(team) {
            res.metal_income = m;
            res.energy_income = e;
        }
        if let Some(&(m, e)) = expense_map.get(team) {
            res.metal_expense = m;
            res.energy_expense = e;
        }

        // Stall ratio: if expense > available, scale down proportionally.
        // available = current + income  (resources that could be spent this tick)
        let metal_available = res.metal + res.metal_income;
        if res.metal_expense > SimFloat::ZERO && res.metal_expense > metal_available {
            res.stall_ratio_metal =
                (metal_available / res.metal_expense).clamp(SimFloat::ZERO, SimFloat::ONE);
        }

        let energy_available = res.energy + res.energy_income;
        if res.energy_expense > SimFloat::ZERO && res.energy_expense > energy_available {
            res.stall_ratio_energy =
                (energy_available / res.energy_expense).clamp(SimFloat::ZERO, SimFloat::ONE);
        }

        // Update totals: resource += income - expense * stall_ratio, clamped
        res.metal = (res.metal + res.metal_income - res.metal_expense * res.stall_ratio_metal)
            .clamp(SimFloat::ZERO, res.metal_storage);
        res.energy = (res.energy + res.energy_income - res.energy_expense * res.stall_ratio_energy)
            .clamp(SimFloat::ZERO, res.energy_storage);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Initialises the [`EconomyState`] resource with default values for the
/// given teams (1000 metal, 1000 energy, 2000 storage each).
pub fn init_economy(world: &mut World, teams: &[u8]) {
    let mut state = EconomyState::default();
    for &team in teams {
        state.teams.insert(
            team,
            TeamResources {
                metal: SimFloat::from_int(1000),
                metal_storage: SimFloat::from_int(2000),
                energy: SimFloat::from_int(1000),
                energy_storage: SimFloat::from_int(2000),
                ..Default::default()
            },
        );
    }
    world.insert_resource(state);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a world with economy initialised for the given teams.
    fn setup(teams: &[u8]) -> World {
        let mut world = World::new();
        init_economy(&mut world, teams);
        world
    }

    #[test]
    fn single_producer_income() {
        let mut world = setup(&[1]);
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: SimFloat::from_int(10),
                energy_per_tick: SimFloat::from_int(5),
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        assert_eq!(r.metal_income, SimFloat::from_int(10));
        assert_eq!(r.energy_income, SimFloat::from_int(5));
        assert_eq!(r.metal, SimFloat::from_int(1010));
        assert_eq!(r.energy, SimFloat::from_int(1005));
        assert_eq!(r.stall_ratio_metal, SimFloat::ONE);
        assert_eq!(r.stall_ratio_energy, SimFloat::ONE);
    }

    #[test]
    fn consumer_expense() {
        let mut world = setup(&[1]);
        world.spawn((
            Allegiance { team: 1 },
            ResourceConsumer {
                metal_per_tick: SimFloat::from_int(100),
                energy_per_tick: SimFloat::from_int(50),
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        assert_eq!(r.metal_expense, SimFloat::from_int(100));
        assert_eq!(r.energy_expense, SimFloat::from_int(50));
        // 1000 - 100 = 900, 1000 - 50 = 950
        assert_eq!(r.metal, SimFloat::from_int(900));
        assert_eq!(r.energy, SimFloat::from_int(950));
        assert_eq!(r.stall_ratio_metal, SimFloat::ONE);
        assert_eq!(r.stall_ratio_energy, SimFloat::ONE);
    }

    #[test]
    fn stall_insufficient_resources() {
        let mut world = setup(&[1]);
        // Set metal to 100, energy to 50 so expense exceeds available.
        {
            let mut state = world.resource_mut::<EconomyState>();
            let r = state.teams.get_mut(&1).unwrap();
            r.metal = SimFloat::from_int(100);
            r.energy = SimFloat::from_int(50);
        }
        world.spawn((
            Allegiance { team: 1 },
            ResourceConsumer {
                metal_per_tick: SimFloat::from_int(200),
                energy_per_tick: SimFloat::from_int(200),
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        // Stall ratio metal = 100 / 200 = 0.5
        assert_eq!(r.stall_ratio_metal, SimFloat::HALF);
        // Stall ratio energy = 50 / 200 = 0.25
        assert_eq!(r.stall_ratio_energy, SimFloat::from_ratio(1, 4));
        // metal = 100 + 0 - 200 * 0.5 = 0
        assert_eq!(r.metal, SimFloat::ZERO);
        // energy = 50 + 0 - 200 * 0.25 = 0
        assert_eq!(r.energy, SimFloat::ZERO);
    }

    #[test]
    fn multiple_teams_independent() {
        let mut world = setup(&[1, 2]);
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: SimFloat::from_int(20),
                energy_per_tick: SimFloat::ZERO,
            },
        ));
        world.spawn((
            Allegiance { team: 2 },
            ResourceConsumer {
                metal_per_tick: SimFloat::from_int(50),
                energy_per_tick: SimFloat::ZERO,
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        // Team 1: gained 20 metal
        assert_eq!(state.teams[&1].metal, SimFloat::from_int(1020));
        // Team 2: lost 50 metal
        assert_eq!(state.teams[&2].metal, SimFloat::from_int(950));
        // Ensure they did not interfere
        assert_eq!(state.teams[&1].metal_expense, SimFloat::ZERO);
        assert_eq!(state.teams[&2].metal_income, SimFloat::ZERO);
    }

    #[test]
    fn storage_cap() {
        let mut world = setup(&[1]);
        // Set metal near storage cap (2000).
        {
            let mut state = world.resource_mut::<EconomyState>();
            let r = state.teams.get_mut(&1).unwrap();
            r.metal = SimFloat::from_int(1995);
        }
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: SimFloat::from_int(100),
                energy_per_tick: SimFloat::ZERO,
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        // Should be clamped to storage (2000), not 2095.
        assert_eq!(state.teams[&1].metal, SimFloat::from_int(2000));
    }
}
