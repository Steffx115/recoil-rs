use super::*;
use pierce_sim::{SimFloat, SimVec3};

// -----------------------------------------------------------------------
// SyncValidator tests
// -----------------------------------------------------------------------

#[test]
fn matching_checksums_returns_in_sync() {
    let mut v = SyncValidator::new(1);
    v.record_local(0, 0xABCD);
    v.record_remote(0, 1, 0xABCD);
    assert_eq!(v.check(0), SyncStatus::InSync);
}

#[test]
fn mismatched_checksums_returns_desync() {
    let mut v = SyncValidator::new(1);
    v.record_local(0, 0xABCD);
    v.record_remote(0, 1, 0xDEAD);
    assert_eq!(
        v.check(0),
        SyncStatus::Desync {
            frame: 0,
            local_hash: 0xABCD,
            remote_hash: 0xDEAD,
            player_id: 1,
        }
    );
}

#[test]
fn missing_local_returns_pending() {
    let mut v = SyncValidator::new(1);
    v.record_remote(0, 1, 0xABCD);
    assert_eq!(v.check(0), SyncStatus::Pending);
}

#[test]
fn missing_remote_returns_pending() {
    let mut v = SyncValidator::new(1);
    v.record_local(0, 0xABCD);
    assert_eq!(v.check(0), SyncStatus::Pending);
}

#[test]
fn partial_remotes_returns_pending() {
    let mut v = SyncValidator::new(2);
    v.record_local(0, 0xABCD);
    v.record_remote(0, 1, 0xABCD);
    // Still waiting for player 2.
    assert_eq!(v.check(0), SyncStatus::Pending);
}

#[test]
fn desync_frame_tracks_first_mismatch() {
    let mut v = SyncValidator::new(1);

    // Frame 0: in sync.
    v.record_local(0, 100);
    v.record_remote(0, 1, 100);
    assert_eq!(v.check(0), SyncStatus::InSync);
    assert_eq!(v.desync_frame(), None);

    // Frame 5: desync.
    v.record_local(5, 200);
    v.record_remote(5, 1, 999);
    assert!(matches!(v.check(5), SyncStatus::Desync { .. }));
    assert_eq!(v.desync_frame(), Some(5));

    // Frame 10: another desync — first mismatch should still be 5.
    v.record_local(10, 300);
    v.record_remote(10, 1, 888);
    assert!(matches!(v.check(10), SyncStatus::Desync { .. }));
    assert_eq!(v.desync_frame(), Some(5));
}

#[test]
fn earlier_desync_overrides_later() {
    let mut v = SyncValidator::new(1);

    // Detect desync at frame 10 first.
    v.record_local(10, 1);
    v.record_remote(10, 1, 2);
    v.check(10);
    assert_eq!(v.desync_frame(), Some(10));

    // Now detect desync at frame 3 (earlier).
    v.record_local(3, 1);
    v.record_remote(3, 1, 2);
    v.check(3);
    assert_eq!(v.desync_frame(), Some(3));
}

#[test]
fn multiple_remote_players_all_match() {
    let mut v = SyncValidator::new(3);
    v.record_local(0, 42);
    v.record_remote(0, 1, 42);
    v.record_remote(0, 2, 42);
    v.record_remote(0, 3, 42);
    assert_eq!(v.check(0), SyncStatus::InSync);
}

#[test]
fn multiple_remote_players_one_mismatches() {
    let mut v = SyncValidator::new(3);
    v.record_local(0, 42);
    v.record_remote(0, 1, 42);
    v.record_remote(0, 2, 99);
    v.record_remote(0, 3, 42);
    let status = v.check(0);
    assert!(matches!(
        status,
        SyncStatus::Desync {
            player_id: 2,
            remote_hash: 99,
            ..
        }
    ));
}

// -----------------------------------------------------------------------
// ComponentHashes tests
// -----------------------------------------------------------------------

fn spawn_unit(world: &mut World, id: u64, x: i32, z: i32) {
    world.spawn((
        SimId { id },
        Position {
            pos: SimVec3::new(SimFloat::from_int(x), SimFloat::ZERO, SimFloat::from_int(z)),
        },
        Velocity { vel: SimVec3::ZERO },
        Heading {
            angle: SimFloat::ZERO,
        },
        Health {
            current: 100,
            max: 100,
        },
        MoveState::Idle,
    ));
}

#[test]
fn identical_worlds_produce_identical_component_hashes() {
    let mut world_a = World::new();
    spawn_unit(&mut world_a, 1, 10, 20);
    spawn_unit(&mut world_a, 2, 30, 40);

    let mut world_b = World::new();
    spawn_unit(&mut world_b, 1, 10, 20);
    spawn_unit(&mut world_b, 2, 30, 40);

    let hashes_a = compute_component_hashes(&mut world_a);
    let hashes_b = compute_component_hashes(&mut world_b);
    assert_eq!(hashes_a, hashes_b);
}

#[test]
fn different_position_produces_different_position_hash() {
    let mut world_a = World::new();
    spawn_unit(&mut world_a, 1, 10, 20);

    let mut world_b = World::new();
    spawn_unit(&mut world_b, 1, 99, 20);

    let hashes_a = compute_component_hashes(&mut world_a);
    let hashes_b = compute_component_hashes(&mut world_b);

    assert_ne!(hashes_a.position_hash, hashes_b.position_hash);
    // Other components should still match.
    assert_eq!(hashes_a.velocity_hash, hashes_b.velocity_hash);
    assert_eq!(hashes_a.heading_hash, hashes_b.heading_hash);
    assert_eq!(hashes_a.health_hash, hashes_b.health_hash);
    assert_eq!(hashes_a.move_state_hash, hashes_b.move_state_hash);
}

#[test]
fn different_health_produces_different_health_hash() {
    let mut world_a = World::new();
    spawn_unit(&mut world_a, 1, 10, 20);

    let mut world_b = World::new();
    world_b.spawn((
        SimId { id: 1 },
        Position {
            pos: SimVec3::new(
                SimFloat::from_int(10),
                SimFloat::ZERO,
                SimFloat::from_int(20),
            ),
        },
        Velocity { vel: SimVec3::ZERO },
        Heading {
            angle: SimFloat::ZERO,
        },
        Health {
            current: 50,
            max: 100,
        },
        MoveState::Idle,
    ));

    let hashes_a = compute_component_hashes(&mut world_a);
    let hashes_b = compute_component_hashes(&mut world_b);

    assert_ne!(hashes_a.health_hash, hashes_b.health_hash);
    // Other components should still match.
    assert_eq!(hashes_a.position_hash, hashes_b.position_hash);
    assert_eq!(hashes_a.velocity_hash, hashes_b.velocity_hash);
}

#[test]
fn different_move_state_produces_different_hash() {
    let mut world_a = World::new();
    spawn_unit(&mut world_a, 1, 10, 20);

    let mut world_b = World::new();
    world_b.spawn((
        SimId { id: 1 },
        Position {
            pos: SimVec3::new(
                SimFloat::from_int(10),
                SimFloat::ZERO,
                SimFloat::from_int(20),
            ),
        },
        Velocity { vel: SimVec3::ZERO },
        Heading {
            angle: SimFloat::ZERO,
        },
        Health {
            current: 100,
            max: 100,
        },
        MoveState::MovingTo(SimVec3::new(
            SimFloat::from_int(50),
            SimFloat::ZERO,
            SimFloat::from_int(50),
        )),
    ));

    let hashes_a = compute_component_hashes(&mut world_a);
    let hashes_b = compute_component_hashes(&mut world_b);

    assert_ne!(hashes_a.move_state_hash, hashes_b.move_state_hash);
    assert_eq!(hashes_a.position_hash, hashes_b.position_hash);
}
