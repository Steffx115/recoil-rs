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
#[path = "tests/sync_tests.rs"]
mod tests;
