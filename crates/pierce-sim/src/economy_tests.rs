use super::*;
use proptest::prelude::*;

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

// ==================================================================
// Property-based tests (proptest)
// ==================================================================

fn arb_resource_rate() -> impl Strategy<Value = SimFloat> {
    (0..500i32).prop_map(SimFloat::from_int)
}

proptest! {
    // ------------------------------------------------------------------
    // P1. Resources never go negative after economy_system tick
    // ------------------------------------------------------------------
    #[test]
    fn prop_resources_never_negative(
        income_m in arb_resource_rate(),
        income_e in arb_resource_rate(),
        expense_m in arb_resource_rate(),
        expense_e in arb_resource_rate(),
    ) {
        let mut world = setup(&[1]);
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: income_m,
                energy_per_tick: income_e,
            },
        ));
        world.spawn((
            Allegiance { team: 1 },
            ResourceConsumer {
                metal_per_tick: expense_m,
                energy_per_tick: expense_e,
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        prop_assert!(r.metal >= SimFloat::ZERO,
            "metal went negative: {}", r.metal.to_f64());
        prop_assert!(r.energy >= SimFloat::ZERO,
            "energy went negative: {}", r.energy.to_f64());
    }

    // ------------------------------------------------------------------
    // P2. Stall ratio is always in [0, 1]
    // ------------------------------------------------------------------
    #[test]
    fn prop_stall_ratio_bounded(
        income_m in arb_resource_rate(),
        income_e in arb_resource_rate(),
        expense_m in arb_resource_rate(),
        expense_e in arb_resource_rate(),
        initial_m in (0..2000i32).prop_map(SimFloat::from_int),
        initial_e in (0..2000i32).prop_map(SimFloat::from_int),
    ) {
        let mut world = setup(&[1]);
        {
            let mut state = world.resource_mut::<EconomyState>();
            let r = state.teams.get_mut(&1).unwrap();
            r.metal = initial_m;
            r.energy = initial_e;
        }
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: income_m,
                energy_per_tick: income_e,
            },
        ));
        world.spawn((
            Allegiance { team: 1 },
            ResourceConsumer {
                metal_per_tick: expense_m,
                energy_per_tick: expense_e,
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        prop_assert!(r.stall_ratio_metal >= SimFloat::ZERO && r.stall_ratio_metal <= SimFloat::ONE,
            "metal stall ratio out of bounds: {}", r.stall_ratio_metal.to_f64());
        prop_assert!(r.stall_ratio_energy >= SimFloat::ZERO && r.stall_ratio_energy <= SimFloat::ONE,
            "energy stall ratio out of bounds: {}", r.stall_ratio_energy.to_f64());
    }

    // ------------------------------------------------------------------
    // P3. Resources never exceed storage capacity
    // ------------------------------------------------------------------
    #[test]
    fn prop_resources_capped_at_storage(
        income_m in arb_resource_rate(),
        income_e in arb_resource_rate(),
    ) {
        let mut world = setup(&[1]);
        world.spawn((
            Allegiance { team: 1 },
            ResourceProducer {
                metal_per_tick: income_m,
                energy_per_tick: income_e,
            },
        ));

        economy_system(&mut world);

        let state = world.resource::<EconomyState>();
        let r = &state.teams[&1];
        prop_assert!(r.metal <= r.metal_storage,
            "metal {} exceeds storage {}", r.metal.to_f64(), r.metal_storage.to_f64());
        prop_assert!(r.energy <= r.energy_storage,
            "energy {} exceeds storage {}", r.energy.to_f64(), r.energy_storage.to_f64());
    }
}
