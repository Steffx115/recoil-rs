//! Server-authoritative fixed-rate tick with command deduplication.
//!
//! The server runs the simulation at a fixed tick rate (default 30 Hz).
//! Clients send commands at any time; the server buffers them and applies
//! only the latest command per (player, unit, command-kind) tuple each tick.

use std::collections::BTreeMap;

use crate::protocol::{CommandFrame, PlayerCommand};

/// Discriminant for command deduplication.
///
/// Two commands with the same `CommandKind` for the same unit replace
/// each other — only the most recent one is kept per tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum CommandKind {
    Move,
    Attack,
    Patrol,
    Guard,
    Stop,
    HoldPosition,
    Build,
    Reclaim,
    Repair,
}

impl CommandKind {
    fn from_command(cmd: &pierce_sim::Command) -> Self {
        match cmd {
            pierce_sim::Command::Move(_) => Self::Move,
            pierce_sim::Command::Attack(_) => Self::Attack,
            pierce_sim::Command::Patrol(_) => Self::Patrol,
            pierce_sim::Command::Guard(_) => Self::Guard,
            pierce_sim::Command::Stop => Self::Stop,
            pierce_sim::Command::HoldPosition => Self::HoldPosition,
            pierce_sim::Command::Build { .. } => Self::Build,
            pierce_sim::Command::Reclaim(_) => Self::Reclaim,
            pierce_sim::Command::Repair(_) => Self::Repair,
        }
    }
}

/// Dedup key: (player_id, target_unit, command_kind).
type DedupeKey = (u8, u64, CommandKind);

/// Server-side tick state. Buffers incoming commands and produces a
/// deduplicated command list each tick.
pub struct ServerTick {
    /// Current simulation frame.
    pub current_frame: u64,
    /// Number of players in the session.
    pub num_players: u8,
    /// Pending commands, deduplicated by (player, unit, kind).
    /// Latest command wins.
    pending: BTreeMap<DedupeKey, PlayerCommand>,
    /// Track which players have pending commands (for building CommandFrames).
    player_commands: BTreeMap<u8, Vec<PlayerCommand>>,
}

impl ServerTick {
    /// Create a new server tick state.
    pub fn new(num_players: u8) -> Self {
        Self {
            current_frame: 0,
            num_players,
            pending: BTreeMap::new(),
            player_commands: BTreeMap::new(),
        }
    }

    /// Buffer a player's commands. Can be called multiple times per tick;
    /// later calls for the same (player, unit, kind) replace earlier ones.
    pub fn receive_commands(&mut self, player_id: u8, commands: &[PlayerCommand]) {
        for cmd in commands {
            let kind = CommandKind::from_command(&cmd.command);
            let key = (player_id, cmd.target_sim_id, kind);
            self.pending.insert(key, cmd.clone());
        }
    }

    /// Advance one tick: drain all pending commands, group by player,
    /// return deduplicated command frames, and increment the frame counter.
    ///
    /// Always advances — does not wait for any player. Missing players
    /// simply have empty command frames.
    pub fn advance(&mut self) -> Vec<CommandFrame> {
        // Drain pending into per-player buckets.
        self.player_commands.clear();
        let pending = std::mem::take(&mut self.pending);
        for ((player_id, _, _), cmd) in pending {
            self.player_commands
                .entry(player_id)
                .or_default()
                .push(cmd);
        }

        // Build one CommandFrame per player (sorted by player_id via BTreeMap).
        let frame = self.current_frame;
        let result: Vec<CommandFrame> = (0..self.num_players)
            .map(|pid| CommandFrame {
                frame,
                player_id: pid,
                commands: self.player_commands.remove(&pid).unwrap_or_default(),
            })
            .collect();

        self.current_frame += 1;
        result
    }

    /// Number of pending (not yet consumed) commands across all players.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
#[path = "tests/server_tick_tests.rs"]
mod tests;
