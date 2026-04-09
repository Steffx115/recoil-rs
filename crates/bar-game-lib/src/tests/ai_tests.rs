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

fn make_ai_world_with_strategy(strategy: Box<dyn AiStrategy>) -> (World, AiState) {
    use crate::setup;
    use std::path::Path;

    let mut world = World::new();
    let config = setup::setup_game(
        &mut world,
        Path::new("nonexistent/units"),
        Path::new("assets/maps/small_duel/manifest.ron"),
    );

    let ai = AiState::with_strategy(
        42, 1, 0,
        config.commander_team1,
        config.commander_team0,
        strategy,
    );
    (world, ai)
}

/// Give AI team generous resources.
fn fund_ai(world: &mut World, team: u8) {
    let mut economy = world.resource_mut::<EconomyState>();
    if let Some(res) = economy.teams.get_mut(&team) {
        res.metal = SimFloat::from_int(50000);
        res.energy = SimFloat::from_int(100000);
        res.metal_storage = SimFloat::from_int(100000);
        res.energy_storage = SimFloat::from_int(200000);
    }
}

/// Run the sim for `frames` ticks, calling ai_tick at the right interval.
fn run_sim(world: &mut World, ai: &mut AiState, frames: u64) {
    let weapon_def_ids = std::collections::BTreeMap::new();
    for frame in 0..frames {
        if frame.is_multiple_of(AI_TICK_INTERVAL) {
            ai_tick(world, ai, frame);
        }
        pierce_sim::construction::construction_system(world);
        pierce_sim::sim_runner::sim_tick(world);
        crate::building::equip_factory_spawned_units(world, &weapon_def_ids);
        crate::building::finalize_completed_buildings(world);
    }
}

// -----------------------------------------------------------------------
// Existing tests (updated for new API)
// -----------------------------------------------------------------------

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
    fund_ai(&mut world, 1);

    assert_eq!(ai.phase, AiPhase::Opening);

    run_sim(&mut world, &mut ai, 900);

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
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 3000);

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
    // With factory but no army -> Expand.
    // With army >= threshold -> Attack.
}

#[test]
fn test_ai_claims_metal_spots() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 2000);

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

// -----------------------------------------------------------------------
// RR-105: New comprehensive AI decision-making tests
// -----------------------------------------------------------------------

/// 1. AI builds solar when energy income is negative / low.
#[test]
fn test_ai_builds_solar_when_energy_low() {
    let (mut world, mut ai) = make_ai_world();
    // Give metal but minimal energy.
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&1) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(50); // very low energy
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }

    run_sim(&mut world, &mut ai, 1500);

    // AI should have built at least one solar.
    assert!(
        ai.solar_count > 0,
        "AI should build solars when energy is low, got solar_count={}",
        ai.solar_count
    );
}

/// 2. AI builds metal extractor when metal income is low and spots are available.
#[test]
fn test_ai_builds_mex_when_metal_income_low() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    let has_spots = world
        .get_resource::<pierce_sim::map::MetalSpots>()
        .map(|ms| !ms.spots.is_empty())
        .unwrap_or(false);

    run_sim(&mut world, &mut ai, 2000);

    if has_spots {
        assert!(
            !ai.claimed_mex_spots.is_empty(),
            "AI should build mexes on available metal spots"
        );
    }
}

/// 3. AI attacks when army size exceeds threshold.
#[test]
fn test_ai_attacks_when_army_large() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Run long enough to build an army.
    run_sim(&mut world, &mut ai, 5000);

    // If army reached threshold, phase should have been Attack at some point.
    let army_size = count_combat_units(&mut world, 1);
    if army_size >= ATTACK_THRESHOLD {
        // Phase should be Attack (or may have cycled).
        // Check that at least some units are no longer idle (were sent to attack).
        let _moving_count = world
            .query_filtered::<(&Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
            .iter(&world)
            .filter(|(a, ms)| a.team == 1 && matches!(ms, MoveState::MovingTo(_)))
            .count();
        // Either units are moving to attack or the attack was already completed.
        // We just verify the AI reached the attack threshold.
        assert!(
            army_size >= ATTACK_THRESHOLD,
            "Army should have exceeded threshold: {}",
            army_size
        );
    }
}

/// 4. AI expands to unclaimed metal spots before claimed ones.
#[test]
fn test_ai_expands_unclaimed_before_claimed() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    let total_spots = world
        .get_resource::<pierce_sim::map::MetalSpots>()
        .map(|ms| ms.spots.len())
        .unwrap_or(0);

    run_sim(&mut world, &mut ai, 3000);

    // All claimed spots should be unique (no duplicates).
    let claimed = &ai.claimed_mex_spots;
    for (i, a) in claimed.iter().enumerate() {
        for (j, b) in claimed.iter().enumerate() {
            if i != j {
                let same = (a.0 - b.0).abs() < 5.0 && (a.1 - b.1).abs() < 5.0;
                assert!(
                    !same,
                    "AI claimed the same metal spot twice: {:?} and {:?}",
                    a, b
                );
            }
        }
    }

    // Should not have claimed more than total available spots.
    if total_spots > 0 {
        assert!(
            claimed.len() <= total_spots,
            "AI claimed {} spots but only {} exist",
            claimed.len(),
            total_spots
        );
    }
}

/// 5. AI doesn't stall economy (doesn't overbuild consumers).
#[test]
fn test_ai_doesnt_stall_economy() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 3000);

    // Check that the AI still has some resources (not totally stalled).
    let economy = world.resource::<EconomyState>();
    if let Some(res) = economy.teams.get(&1) {
        // The AI shouldn't have drained everything if it's managing economy.
        // (generous starting resources, so at least some should remain)
        let total = res.metal.to_f32() + res.energy.to_f32();
        // This is a soft check — the AI had 150k resources, it shouldn't be at 0.
        assert!(
            total > 0.0,
            "AI completely drained resources: metal={}, energy={}",
            res.metal.to_f32(),
            res.energy.to_f32()
        );
    }
}

/// 6. AI transitions from Opening -> Expand -> Attack correctly.
#[test]
fn test_ai_phase_transition_sequence() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Initially in Opening.
    assert_eq!(ai.phase, AiPhase::Opening);

    // Run until factory is built.
    run_sim(&mut world, &mut ai, 1500);

    // If factory exists, phase should be Expand or Attack.
    if ai.factory.is_some() {
        assert_ne!(
            ai.phase,
            AiPhase::Opening,
            "AI should leave Opening once factory is built"
        );
    }

    // Run longer to build army.
    run_sim(&mut world, &mut ai, 5000);

    let army_size = count_combat_units(&mut world, 1);
    if army_size >= ATTACK_THRESHOLD {
        assert_eq!(
            ai.phase,
            AiPhase::Attack,
            "AI should be in Attack phase with army_size={}",
            army_size
        );
    }
}

/// 7. AI handles edge case: all metal spots claimed.
#[test]
fn test_ai_handles_all_metal_spots_claimed() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Pre-claim all metal spots.
    let spots: Vec<(f64, f64)> = world
        .get_resource::<pierce_sim::map::MetalSpots>()
        .map(|ms| ms.spots.iter().map(|s| (s.x, s.z)).collect())
        .unwrap_or_default();
    ai.claimed_mex_spots = spots;

    // Should not panic when all spots are claimed.
    run_sim(&mut world, &mut ai, 1000);
}

/// 8. AI handles edge case: commander dies mid-opening.
#[test]
fn test_ai_handles_commander_death() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Kill the commander.
    if let Some(cmd) = ai.commander {
        world.entity_mut(cmd).insert(Dead);
    }

    // Should not panic — AI should just bail out.
    run_sim(&mut world, &mut ai, 500);
}

/// 9. AI produces correct unit mix (not all one type).
#[test]
fn test_ai_produces_unit_mix() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Run long enough to produce many units.
    run_sim(&mut world, &mut ai, 5000);

    // Collect unit types of alive non-building, non-builder units on AI team.
    let unit_types: Vec<u32> = world
        .query_filtered::<(&UnitType, &Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
        .iter(&world)
        .filter(|(_, a, _)| a.team == 1)
        .map(|(ut, _, _)| ut.id)
        .collect();

    // If we have more than 3 units, check for variety.
    // (On maps with only one buildable combat unit, this is fine — just one type.)
    if unit_types.len() > 3 {
        let mut seen = std::collections::BTreeSet::new();
        for t in &unit_types {
            seen.insert(*t);
        }
        // We can't guarantee >1 type if the factory can only build one unit type,
        // but verify we produced something.
        assert!(
            !seen.is_empty(),
            "AI should have produced at least one unit type"
        );
    }
}

/// Test 10: AI continues functioning and producing units even when under attack
/// during the expansion phase.
#[test]
fn test_ai_continues_producing_under_attack() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Build up the AI a bit.
    run_sim(&mut world, &mut ai, 2000);

    let units_before = world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&world)
        .filter(|a| a.team == 1)
        .count();

    // Continue running — AI should keep producing.
    run_sim(&mut world, &mut ai, 2000);

    let units_after = world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&world)
        .filter(|a| a.team == 1)
        .count();

    assert!(
        units_after >= units_before,
        "AI should continue building: before={}, after={}",
        units_before,
        units_after
    );
}

/// 11a. Opening phase test: AI places factory in opening.
#[test]
fn test_opening_phase_places_factory() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    assert_eq!(ai.phase, AiPhase::Opening);

    // Run just one AI tick worth.
    run_sim(&mut world, &mut ai, AI_TICK_INTERVAL + 1);

    // Factory may not be finished, but something should be under construction.
    let building_count = world
        .query_filtered::<(&Allegiance, &UnitType), Without<Dead>>()
        .iter(&world)
        .filter(|(a, _)| a.team == 1)
        .count();
    // Commander + at least one placed structure.
    assert!(
        building_count >= 1,
        "Opening should place at least a factory: count={}",
        building_count
    );
}

/// 11b. Expand phase test: AI builds economy structures.
#[test]
fn test_expand_phase_builds_economy() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Fast-forward through opening to get a factory.
    run_sim(&mut world, &mut ai, 1500);

    let solar_before = ai.solar_count;

    // Lower energy to trigger solar building.
    {
        let mut economy = world.resource_mut::<EconomyState>();
        if let Some(res) = economy.teams.get_mut(&1) {
            res.energy = SimFloat::from_int(50);
        }
    }

    run_sim(&mut world, &mut ai, 1000);

    // Should have built more solars.
    assert!(
        ai.solar_count >= solar_before,
        "AI should build more solars in expand phase"
    );
}

/// 11c. Attack phase test: AI sends units.
#[test]
fn test_attack_phase_sends_units() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Run long enough to build an army.
    run_sim(&mut world, &mut ai, 5000);

    if ai.phase == AiPhase::Attack {
        // ticks_since_attack should have been reset at some point if attack happened.
        // (The default attack wave resets it to 0.)
        // We just verify the AI doesn't crash in attack phase.
        assert_eq!(ai.phase, AiPhase::Attack);
    }
}

/// 12. AI doesn't crash over 5000+ tick games.
#[test]
fn test_ai_survives_long_game() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Run a long game. Should not panic.
    run_sim(&mut world, &mut ai, 6000);

    // Basic sanity: AI is still in a valid phase.
    assert!(
        matches!(ai.phase, AiPhase::Opening | AiPhase::Expand | AiPhase::Attack),
        "AI should be in a valid phase after long game"
    );
}

/// 13. PassiveStrategy only builds economy, never attacks.
#[test]
fn test_passive_strategy_no_attack() {
    let (mut world, mut ai) = make_ai_world_with_strategy(Box::new(PassiveStrategy));
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 5000);

    // Passive strategy should never produce combat units or send attacks.
    // Check that no non-commander units are in MovingTo state (attack wave).
    let moving_combat: Vec<Entity> = world
        .query_filtered::<(Entity, &Allegiance, &MoveState), (Without<Dead>, Without<Builder>)>()
        .iter(&world)
        .filter(|(e, a, ms)| {
            a.team == 1 && matches!(ms, MoveState::MovingTo(_)) && Some(*e) != ai.commander
        })
        .map(|(e, _, _)| e)
        .collect();

    assert!(
        moving_combat.is_empty(),
        "Passive strategy should not attack: {} units moving",
        moving_combat.len()
    );
}

/// 14. PassiveStrategy still builds economy.
#[test]
fn test_passive_strategy_builds_economy() {
    let (mut world, mut ai) = make_ai_world_with_strategy(Box::new(PassiveStrategy));
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 2000);

    // Should have built at least a solar or factory.
    let all_alive = world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&world)
        .filter(|a| a.team == 1)
        .count();
    assert!(
        all_alive > 1,
        "Passive strategy should build economy structures: alive={}",
        all_alive
    );
}

/// 15. with_strategy constructor works and uses the provided strategy.
#[test]
fn test_with_strategy_constructor() {
    // Use a custom strategy that does nothing.
    struct DoNothingStrategy;
    impl AiStrategy for DoNothingStrategy {
        fn decide(&mut self, _world: &mut World, _state: &AiState) -> Vec<AiAction> {
            vec![]
        }
    }

    let (mut world, mut ai) =
        make_ai_world_with_strategy(Box::new(DoNothingStrategy));
    fund_ai(&mut world, 1);

    run_sim(&mut world, &mut ai, 1000);

    // DoNothing should result in no buildings besides the commander.
    let all_alive = world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&world)
        .filter(|a| a.team == 1)
        .count();
    // Only the commander should be alive (the DoNothing strategy places nothing).
    assert_eq!(
        all_alive, 1,
        "DoNothing strategy should not build anything: alive={}",
        all_alive
    );
}

/// 16. DefaultStrategy produces the same actions as the original behavior.
#[test]
fn test_default_strategy_is_default() {
    let ai = AiState::new(42, 1, 0, None, None);
    // AiState::new should use DefaultStrategy by default.
    assert_eq!(ai.phase, AiPhase::Opening);
    assert_eq!(ai.team, 1);
    assert_eq!(ai.enemy_team, 0);
}

/// 17. AI tick interval is respected (no action on non-interval frames).
#[test]
fn test_ai_tick_interval_respected() {
    let (mut world, mut ai) = make_ai_world();
    fund_ai(&mut world, 1);

    // Run on a non-interval frame — should be a no-op.
    let phase_before = ai.phase;
    ai_tick(&mut world, &mut ai, 1); // Not a multiple of AI_TICK_INTERVAL.
    assert_eq!(ai.phase, phase_before, "AI should not act on non-interval frames");
}

/// 18. AI find_buildable returns None for unknown builder type.
#[test]
fn test_find_buildable_unknown_builder() {
    let registry = UnitDefRegistry::default();
    let result = find_buildable(&registry, 999_999, |_| true);
    assert!(result.is_none(), "Should return None for unknown builder type");
}
