//! Sync validation layer for deterministic lockstep multiplayer.
//!
//! Compares local and remote checksums each frame to detect desyncs
//! as early as possible. Also provides per-component hashing to help
//! pinpoint which component type diverged.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use bevy_ecs::prelude::*;

use pierce_sim::{Heading, Health, MoveState, Position, SimId, Velocity};

/// Result of comparing checksums for a given frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// All received checksums match the local one.
    InSync,
    /// A mismatch was detected.
    Desync {
        frame: u64,
        local_hash: u64,
        remote_hash: u64,
        player_id: u8,
    },
    /// Not all checksums have been received yet.
    Pending,
}

/// Per-component hash breakdown to help locate the source of a desync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentHashes {
    pub position_hash: u64,
    pub velocity_hash: u64,
    pub heading_hash: u64,
    pub health_hash: u64,
    pub move_state_hash: u64,
}

/// Tracks local and remote checksums and detects the first frame where
/// a desync occurred.
pub struct SyncValidator {
    local_checksums: BTreeMap<u64, u64>,
    remote_checksums: BTreeMap<u64, BTreeMap<u8, u64>>,
    desync_frame: Option<u64>,
    num_remote_players: u8,
}

impl SyncValidator {
    /// Create a new validator expecting `num_remote_players` remote peers.
    pub fn new(num_remote_players: u8) -> Self {
        Self {
            local_checksums: BTreeMap::new(),
            remote_checksums: BTreeMap::new(),
            desync_frame: None,
            num_remote_players,
        }
    }

    /// Store the local checksum for a frame.
    pub fn record_local(&mut self, frame: u64, checksum: u64) {
        self.local_checksums.insert(frame, checksum);
    }

    /// Store a remote player's checksum for a frame.
    pub fn record_remote(&mut self, frame: u64, player_id: u8, checksum: u64) {
        self.remote_checksums
            .entry(frame)
            .or_default()
            .insert(player_id, checksum);
    }

    /// Compare all available checksums for the given frame.
    ///
    /// Returns [`SyncStatus::Pending`] if the local checksum or any remote
    /// checksum is still missing. Returns [`SyncStatus::Desync`] on the
    /// first mismatched remote player found. Otherwise returns
    /// [`SyncStatus::InSync`].
    pub fn check(&mut self, frame: u64) -> SyncStatus {
        let local_hash = match self.local_checksums.get(&frame) {
            Some(&h) => h,
            None => return SyncStatus::Pending,
        };

        let remotes = match self.remote_checksums.get(&frame) {
            Some(r) => r,
            None => return SyncStatus::Pending,
        };

        if (remotes.len() as u8) < self.num_remote_players {
            return SyncStatus::Pending;
        }

        for (&player_id, &remote_hash) in remotes {
            if remote_hash != local_hash {
                // Record the first desync frame.
                if self.desync_frame.is_none() || frame < self.desync_frame.unwrap() {
                    self.desync_frame = Some(frame);
                }
                return SyncStatus::Desync {
                    frame,
                    local_hash,
                    remote_hash,
                    player_id,
                };
            }
        }

        SyncStatus::InSync
    }

    /// The first frame where a desync was detected, if any.
    pub fn desync_frame(&self) -> Option<u64> {
        self.desync_frame
    }
}

/// Hash a single component type across all entities, sorted by [`SimId`].
fn hash_component<C: Component + Hash>(world: &mut World) -> u64 {
    let mut entries: Vec<(u64, Entity)> = world
        .query::<(Entity, &SimId)>()
        .iter(world)
        .map(|(e, sid)| (sid.id, e))
        .collect();
    entries.sort_by_key(|&(id, _)| id);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (_, entity) in entries {
        if let Some(c) = world.get::<C>(entity) {
            c.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Compute per-component hashes over all entities in the world.
///
/// Each component type is hashed independently so that when a desync
/// occurs you can compare component hashes to narrow down which
/// component diverged.
pub fn compute_component_hashes(world: &mut World) -> ComponentHashes {
    ComponentHashes {
        position_hash: hash_component::<Position>(world),
        velocity_hash: hash_component::<Velocity>(world),
        heading_hash: hash_component::<Heading>(world),
        health_hash: hash_component::<Health>(world),
        move_state_hash: hash_component::<MoveState>(world),
    }
}

#[cfg(test)]
mod tests {
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
                current: SimFloat::from_int(100),
                max: SimFloat::from_int(100),
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
                current: SimFloat::from_int(50),
                max: SimFloat::from_int(100),
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
                current: SimFloat::from_int(100),
                max: SimFloat::from_int(100),
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
}
