use super::*;
use crate::economy::init_economy;

/// Create a world with economy for the given teams.
fn setup(teams: &[u8]) -> World {
    let mut world = World::new();
    init_economy(&mut world, teams);
    world
}

/// Create a world with economy and a terrain grid for footprint tests.
fn setup_with_grid(teams: &[u8]) -> World {
    let mut world = World::new();
    init_economy(&mut world, teams);
    world.insert_resource(crate::pathfinding::TerrainGrid::new(
        128,
        128,
        SimFloat::ONE,
    ));
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

// ---------------------------------------------------------------------------
// RR-125: cancel_build_site
// ---------------------------------------------------------------------------

#[test]
fn cancel_build_site_refunds_proportional_resources() {
    let mut world = setup(&[1]);

    let initial_metal = {
        let state = world.resource::<EconomyState>();
        state.teams[&1].metal
    };

    // Spawn a nanoframe at 50% progress.
    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::from_int(50),
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(200),
                total_build_time: SimFloat::from_int(10),
                progress: SimFloat::HALF,
            },
            Allegiance { team: 1 },
        ))
        .id();

    // Spawn a builder targeting the nanoframe.
    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(5),
            },
            BuildTarget { target: nanoframe },
            Allegiance { team: 1 },
        ))
        .id();

    cancel_build_site(&mut world, nanoframe);

    // Nanoframe should be marked Dead.
    assert!(
        world.get::<Dead>(nanoframe).is_some(),
        "Cancelled BuildSite should be marked Dead"
    );

    // Refund: 50% remaining * 100 metal = 50 metal refunded.
    let state = world.resource::<EconomyState>();
    let expected_metal = initial_metal + SimFloat::from_int(50);
    assert_eq!(
        state.teams[&1].metal, expected_metal,
        "Should refund 50% of remaining metal cost"
    );

    // Refund: 50% remaining * 200 energy = 100 energy refunded.
    let expected_energy = SimFloat::from_int(1000) + SimFloat::from_int(100);
    assert_eq!(
        state.teams[&1].energy, expected_energy,
        "Should refund 50% of remaining energy cost"
    );

    // Builder should have its BuildTarget cleared.
    assert!(
        world.get::<BuildTarget>(builder).is_none(),
        "Builder's BuildTarget should be removed after cancel"
    );
}

#[test]
fn cancel_build_site_full_refund_at_zero_progress() {
    let mut world = setup(&[1]);

    let initial_metal = {
        let state = world.resource::<EconomyState>();
        state.teams[&1].metal
    };

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

    cancel_build_site(&mut world, nanoframe);

    // Full refund: 100% remaining * 100 = 100 metal.
    let state = world.resource::<EconomyState>();
    assert_eq!(
        state.teams[&1].metal,
        initial_metal + SimFloat::from_int(100),
        "Should refund full cost at zero progress"
    );
}

#[test]
fn cancel_build_site_unmarks_footprint() {
    let mut world = setup_with_grid(&[1]);

    // Mark footprint manually.
    let footprint = {
        let pos = pierce_math::SimVec2::new(SimFloat::from_int(50), SimFloat::from_int(50));
        let radius = SimFloat::from_int(5);
        let mut grid = world.resource_mut::<crate::pathfinding::TerrainGrid>();
        crate::footprint::mark_building_footprint(&mut grid, pos, radius)
    };

    // Verify cells are impassable.
    {
        let grid = world.resource::<crate::pathfinding::TerrainGrid>();
        for &(cx, cy) in &footprint.cells {
            assert!(!grid.is_passable(cx, cy), "Cell should be impassable");
        }
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
            footprint.clone(),
        ))
        .id();

    cancel_build_site(&mut world, nanoframe);

    // Cells should be passable again.
    let grid = world.resource::<crate::pathfinding::TerrainGrid>();
    for &(cx, cy) in &footprint.cells {
        assert!(
            grid.is_passable(cx, cy),
            "Cell ({cx}, {cy}) should be passable after cancel"
        );
    }
}

#[test]
fn cancel_nonexistent_entity_is_noop() {
    let mut world = setup(&[1]);

    // Despawn an entity so it becomes invalid.
    let e = world.spawn_empty().id();
    world.despawn(e);

    // Should not panic.
    cancel_build_site(&mut world, e);
}

// ---------------------------------------------------------------------------
// RR-127: auto-resume construction
// ---------------------------------------------------------------------------

#[test]
fn builder_out_of_range_saves_previous_target() {
    use crate::components::{MoveState, Position};
    use pierce_math::SimVec3;

    let mut world = setup(&[1]);

    let building_pos = SimVec3::new(
        SimFloat::from_int(100),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );
    let far_pos = SimVec3::new(
        SimFloat::from_int(200),
        SimFloat::ZERO,
        SimFloat::from_int(200),
    );

    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(100),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
            Position { pos: building_pos },
        ))
        .id();

    // Builder far from the building.
    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(5),
            },
            BuildTarget { target: nanoframe },
            Allegiance { team: 1 },
            Position { pos: far_pos },
            MoveState::Idle,
        ))
        .id();

    construction_system(&mut world);

    // BuildTarget should be removed (out of range).
    assert!(
        world.get::<BuildTarget>(builder).is_none(),
        "BuildTarget should be removed when out of range"
    );

    // PreviousBuildTarget should be saved.
    let pbt = world.get::<PreviousBuildTarget>(builder);
    assert!(
        pbt.is_some(),
        "PreviousBuildTarget should be saved when builder goes out of range"
    );
    assert_eq!(pbt.unwrap().target, nanoframe);

    // Build progress should not have advanced.
    let site = world.get::<BuildSite>(nanoframe).unwrap();
    assert_eq!(
        site.progress,
        SimFloat::ZERO,
        "No progress should be made when builder is out of range"
    );
}

#[test]
fn auto_resume_reassigns_idle_builder_near_site() {
    use crate::components::{MoveState, Position};
    use pierce_math::SimVec3;

    let mut world = setup(&[1]);

    let building_pos = SimVec3::new(
        SimFloat::from_int(100),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );

    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(100),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
            Position { pos: building_pos },
        ))
        .id();

    // Builder near the site, idle, with PreviousBuildTarget.
    let near_pos = SimVec3::new(
        SimFloat::from_int(110),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );
    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(5),
            },
            PreviousBuildTarget { target: nanoframe },
            Allegiance { team: 1 },
            Position { pos: near_pos },
            MoveState::Idle,
        ))
        .id();

    auto_resume_construction_system(&mut world);

    // Builder should now have BuildTarget re-assigned.
    let bt = world.get::<BuildTarget>(builder);
    assert!(
        bt.is_some(),
        "Idle builder near site should auto-resume construction"
    );
    assert_eq!(bt.unwrap().target, nanoframe);

    // PreviousBuildTarget should be removed.
    assert!(
        world.get::<PreviousBuildTarget>(builder).is_none(),
        "PreviousBuildTarget should be cleared after resume"
    );

    // Builder should be moving toward the target.
    let ms = world.get::<MoveState>(builder).unwrap();
    assert!(
        matches!(ms, MoveState::MovingTo(_)),
        "Builder should be moving toward resumed target"
    );
}

#[test]
fn auto_resume_ignores_builder_too_far_from_site() {
    use crate::components::{MoveState, Position};
    use pierce_math::SimVec3;

    let mut world = setup(&[1]);

    let building_pos = SimVec3::new(
        SimFloat::from_int(100),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );

    let nanoframe = world
        .spawn((
            Health {
                current: SimFloat::ZERO,
                max: SimFloat::from_int(100),
            },
            BuildSite {
                metal_cost: SimFloat::from_int(100),
                energy_cost: SimFloat::from_int(100),
                total_build_time: SimFloat::from_int(100),
                progress: SimFloat::ZERO,
            },
            Allegiance { team: 1 },
            Position { pos: building_pos },
        ))
        .id();

    // Builder far from the site.
    let far_pos = SimVec3::new(
        SimFloat::from_int(500),
        SimFloat::ZERO,
        SimFloat::from_int(500),
    );
    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(5),
            },
            PreviousBuildTarget { target: nanoframe },
            Allegiance { team: 1 },
            Position { pos: far_pos },
            MoveState::Idle,
        ))
        .id();

    auto_resume_construction_system(&mut world);

    // Builder should NOT have BuildTarget (too far).
    assert!(
        world.get::<BuildTarget>(builder).is_none(),
        "Builder too far away should not auto-resume"
    );

    // PreviousBuildTarget should still be there.
    assert!(
        world.get::<PreviousBuildTarget>(builder).is_some(),
        "PreviousBuildTarget should persist when builder is too far"
    );
}

#[test]
fn auto_resume_clears_previous_if_site_completed() {
    use crate::components::{MoveState, Position};
    use pierce_math::SimVec3;

    let mut world = setup(&[1]);

    let building_pos = SimVec3::new(
        SimFloat::from_int(100),
        SimFloat::ZERO,
        SimFloat::from_int(100),
    );

    // Completed building: no BuildSite component.
    let completed = world
        .spawn((
            Health {
                current: SimFloat::from_int(100),
                max: SimFloat::from_int(100),
            },
            Allegiance { team: 1 },
            Position { pos: building_pos },
        ))
        .id();

    let builder = world
        .spawn((
            Builder {
                build_power: SimFloat::from_int(5),
            },
            PreviousBuildTarget { target: completed },
            Allegiance { team: 1 },
            Position { pos: building_pos },
            MoveState::Idle,
        ))
        .id();

    auto_resume_construction_system(&mut world);

    // PreviousBuildTarget should be cleared (site is complete).
    assert!(
        world.get::<PreviousBuildTarget>(builder).is_none(),
        "PreviousBuildTarget should be cleared when site is completed"
    );

    // Should NOT have a new BuildTarget.
    assert!(
        world.get::<BuildTarget>(builder).is_none(),
        "Builder should not resume a completed site"
    );
}
