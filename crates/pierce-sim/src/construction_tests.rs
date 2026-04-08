use super::*;
use crate::economy::init_economy;

/// Create a world with economy for the given teams.
fn setup(teams: &[u8]) -> World {
    let mut world = World::new();
    init_economy(&mut world, teams);
    world
}

#[test]
fn builder_completes_build_site() {
    let mut world = setup(&[1]);

    // Spawn a nanoframe with BuildSite.
    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(10),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
        ))
        .id();

    // Spawn a builder targeting the nanoframe.
    // build_power = 10, total_build_time = 10 => progress += 1.0 per tick.
    world.spawn((
        Builder {
            build_power: SimFloat::from_int(10),
        },
        BuildTarget { target: nanoframe },
        Allegiance { team: 1 },
    ));

    construction_system(&mut world);

    // Should be complete in one tick (10/10 = 1.0 progress).
    assert!(
        world.get::<BuildSite>(nanoframe).is_none(),
        "BuildSite should be removed after completion"
    );
    let health = world.get::<Health>(nanoframe).unwrap();
    assert_eq!(health.current, health.max);
}

#[test]
fn builder_stops_and_clears_target_after_construction() {
    use crate::components::MoveState;
    use pierce_math::{SimFloat, SimVec3};

    let mut world = setup(&[1]);

    let building_pos = SimVec3::new(
        SimFloat::from_int(100),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );

    // Spawn a nanoframe with BuildSite.
    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(10),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
            crate::components::Position { pos: building_pos },
        ))
        .id();

    // Spawn a builder that is still moving toward the building.
    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(10),
            },
            BuildTarget { target: nanoframe },
            Allegiance { team: 1 },
            crate::components::Position { pos: building_pos },
            MoveState::MovingTo(building_pos),
        ))
        .id();

    construction_system(&mut world);

    // Building should be complete.
    assert!(world.get::<BuildSite>(nanoframe).is_none());

    // Builder's BuildTarget should be removed.
    assert!(
        world.get::<BuildTarget>(builder).is_none(),
        "BuildTarget should be removed after construction completes"
    );

    // Builder's MoveState should be Idle.
    let ms = world.get::<MoveState>(builder).unwrap();
    assert_eq!(
        *ms,
        MoveState::Idle,
        "Builder should stop moving after construction completes"
    );
}

#[test]
fn multiple_builders_speed_up_construction() {
    let mut world = setup(&[1]);

    // total_build_time = 10, each builder has build_power = 2.
    // One builder: progress = 2/10 = 0.2 per tick.
    // Two builders: progress = 0.2 + 0.2 = 0.4 per tick.
    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(10),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
        ))
        .id();

    // Spawn two builders.
    for _ in 0..2 {
        world.spawn((
            Builder {
                build_power: SimFloat::from_int(2),
            },
            BuildTarget { target: nanoframe },
            Allegiance { team: 1 },
        ));
    }

    // After one tick, progress should be 0.4.
    construction_system(&mut world);
    let site = world.get::<BuildSite>(nanoframe).unwrap();
    assert_eq!(
        site.progress,
        SimFloat::from_ratio(2, 5),
        "Two builders should contribute 0.4 progress"
    );

    // After two more ticks (total 3), progress = 0.4 * 3 = 1.2 >= 1.0.
    construction_system(&mut world);
    construction_system(&mut world);
    assert!(
        world.get::<BuildSite>(nanoframe).is_none(),
        "BuildSite should be removed after 3 ticks with two builders"
    );
}

#[test]
fn reclaim_returns_metal() {
    let mut world = setup(&[1]);

    let metal_before = {
        let state = world.resource::<EconomyState>();
        state.teams[&1].metal
    };

    // Spawn a reclaimable wreck with 50 metal value.
    let wreck = world
        .spawn(Reclaimable {
            metal_value: SimFloat::from_int(50),
            reclaim_progress: SimFloat::ZERO,
        })
        .id();

    // Builder with build_power = 50 => completes in 1 tick (50/50 = 1.0).
    world.spawn((
        Builder {
            build_power: SimFloat::from_int(50),
        },
        BuildTarget { target: wreck },
        Allegiance { team: 1 },
    ));

    construction_system(&mut world);

    // Wreck should be marked dead.
    assert!(
        world.get::<Dead>(wreck).is_some(),
        "Reclaimable should be marked Dead after completion"
    );

    // Team should have gained 50 metal.
    let state = world.resource::<EconomyState>();
    assert_eq!(state.teams[&1].metal, metal_before + SimFloat::from_int(50),);
}

#[test]
fn stalled_economy_slows_construction() {
    let mut world = setup(&[1]);

    // Set team resources very low so stall kicks in.
    {
        let mut state = world.resource_mut::<EconomyState>();
        let res = state.teams.get_mut(&1).unwrap();
        res.metal = SimFloat::from_int(10);
        res.energy = SimFloat::from_int(10);
        // Set stall ratios to 0.5 (simulating a prior economy tick stall).
        res.stall_ratio_metal = SimFloat::HALF;
        res.stall_ratio_energy = SimFloat::HALF;
    }

    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(10),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
        ))
        .id();

    // Builder with build_power = 5 => nominal progress = 5/10 = 0.5.
    // With stall 0.5 => effective progress = 0.25.
    world.spawn((
        Builder {
            build_power: SimFloat::from_int(5),
        },
        BuildTarget { target: nanoframe },
        Allegiance { team: 1 },
    ));

    construction_system(&mut world);

    let site = world.get::<BuildSite>(nanoframe).unwrap();
    assert_eq!(
        site.progress,
        SimFloat::from_ratio(1, 4),
        "Stalled economy should halve effective build rate"
    );
}

#[test]
fn repair_restores_health() {
    let mut world = setup(&[1]);

    // Spawn a damaged unit (no BuildSite).
    let unit = world
        .spawn((
            Health {
                current: SimFloat::from_int(50),
                max: SimFloat::from_int(100),
            },
            Allegiance { team: 1 },
        ))
        .id();

    // Builder targeting the damaged unit with build_power = 20.
    world.spawn((
        Builder {
            build_power: SimFloat::from_int(20),
        },
        BuildTarget { target: unit },
        Allegiance { team: 1 },
    ));

    repair_system(&mut world);

    let health = world.get::<Health>(unit).unwrap();
    assert_eq!(
        health.current,
        SimFloat::from_int(70),
        "Repair should restore 20 HP"
    );

    // Run again — should cap at max.
    repair_system(&mut world);
    repair_system(&mut world);
    let health = world.get::<Health>(unit).unwrap();
    assert_eq!(
        health.current,
        SimFloat::from_int(100),
        "Repair should not exceed max health"
    );
}
