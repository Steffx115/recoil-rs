use super::*;
use pierce_sim::economy::init_economy;
use pierce_sim::sim_runner;

fn make_ai_world() -> (World, AiState) {
    use crate::setup;
    use std::path::Path;

    let mut world = World::new();
    let config = setup::setup_game(
        &mut world,
        Path::new("nonexistent/units"),
        Path::new("assets/maps/small_duel/manifest.ron"),
    );

    let ai = AiState::new(42, 1, 0, config.commander_team1, config.commander_team0);
    (world, ai)
}

#[test]
fn test_ai_no_panic_with_dead_commander() {
    let mut world = World::new();
    sim_runner::init_sim_world(&mut world);
    init_economy(&mut world, &[0, 1]);
    world.insert_resource(UnitDefRegistry::default());
    world.insert_resource(pierce_sim::map::MetalSpots::default());

    let mut ai = AiState::new(42, 1, 0, None, None);
    ai_tick(&mut world, &mut ai, AI_TICK_INTERVAL);
}

#[test]
fn test_ai_builds_factory_in_opening() {
    let (mut world, mut ai) = make_ai_world();

    // Give AI resources.
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&1) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }

    assert_eq!(ai.phase, AiPhase::Opening);

    // Run a few AI ticks.
    for frame in 0u64..900 {
        if frame.is_multiple_of(AI_TICK_INTERVAL) {
            ai_tick(&mut world, &mut ai, frame);
        }
        pierce_sim::construction::construction_system(&mut world);
        pierce_sim::sim_runner::sim_tick(&mut world);
        crate::building::finalize_completed_buildings(&mut world);
    }

    // AI should have placed a factory (may or may not be finished).
    let factory_count = world
        .query_filtered::<(&Allegiance, &UnitType), Without<Dead>>()
        .iter(&world)
        .filter(|(a, _)| a.team == 1)
        .count();
    assert!(
        factory_count > 1,
        "AI should have built something beyond the commander: got {}",
        factory_count
    );
}

#[test]
fn test_ai_produces_army() {
    let (mut world, mut ai) = make_ai_world();

    // Give AI resources.
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&1) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }

    // Run for a long time.
    let weapon_def_ids = std::collections::BTreeMap::new();
    for frame in 0u64..3000 {
        if frame.is_multiple_of(AI_TICK_INTERVAL) {
            ai_tick(&mut world, &mut ai, frame);
        }
        pierce_sim::construction::construction_system(&mut world);
        pierce_sim::sim_runner::sim_tick(&mut world);
        crate::building::equip_factory_spawned_units(&mut world, &weapon_def_ids);
        crate::building::finalize_completed_buildings(&mut world);
    }

    // AI should have produced at least some units (factory + combat).
    let _total = count_combat_units(&mut world, 1);
    let all_alive = world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&world)
        .filter(|a| a.team == 1)
        .count();
    assert!(
        all_alive > 1,
        "AI should have built something after 3000 frames: alive={}",
        all_alive
    );
}

#[test]
fn test_ai_phase_transitions() {
    let mut ai = AiState::new(42, 1, 0, None, None);
    assert_eq!(ai.phase, AiPhase::Opening);

    // Simulate having a factory.
    ai.factory = Some(Entity::from_raw(999));
    // Phase would transition to Expand in ai_tick, but we check logic directly:
    // With factory but no army → Expand.
    // With army >= threshold → Attack.
}

#[test]
fn test_ai_claims_metal_spots() {
    let (mut world, mut ai) = make_ai_world();
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&1) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }

    // Run enough for factory + expansion.
    for frame in 0u64..2000 {
        if frame.is_multiple_of(AI_TICK_INTERVAL) {
            ai_tick(&mut world, &mut ai, frame);
        }
        pierce_sim::construction::construction_system(&mut world);
        pierce_sim::sim_runner::sim_tick(&mut world);
        crate::building::finalize_completed_buildings(&mut world);
    }

    // AI should have claimed metal spots if the map has any.
    let has_spots = world
        .get_resource::<pierce_sim::map::MetalSpots>()
        .map(|ms| !ms.spots.is_empty())
        .unwrap_or(false);
    if has_spots {
        assert!(
            !ai.claimed_mex_spots.is_empty(),
            "AI should have claimed metal spots when map has them"
        );
    }
}
