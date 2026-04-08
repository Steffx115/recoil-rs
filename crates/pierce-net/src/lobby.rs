//! Simple lobby / game browser for matchmaking.
//!
//! This module provides in-memory data structures and logic for hosting,
//! joining, and listing games. No actual networking is performed here —
//! these types are intended to be driven by a future network transport.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single game visible in the game browser.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GameListing {
    pub game_id: u64,
    pub host_name: String,
    pub map_name: String,
    pub max_players: u8,
    pub current_players: u8,
    pub state: GameState,
}

/// Lifecycle state of a hosted game.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// Waiting for players to join.
    Lobby,
    /// Simulation is running.
    InProgress,
    /// Game has ended.
    Finished,
}

/// Messages exchanged between a client and the lobby server.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LobbyMessage {
    HostGame {
        host_name: String,
        map_name: String,
        max_players: u8,
    },
    JoinGame {
        game_id: u64,
    },
    LeaveGame {
        game_id: u64,
    },
    StartGame {
        game_id: u64,
    },
    ListGames,
    GameList(Vec<GameListing>),
    Error(String),
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur when mutating lobby state.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum LobbyError {
    #[error("game {0} not found")]
    GameNotFound(u64),
    #[error("game {0} is full")]
    GameFull(u64),
    #[error("game {0} is not in the Lobby state")]
    NotInLobby(u64),
}

// ---------------------------------------------------------------------------
// LobbyServer
// ---------------------------------------------------------------------------

/// In-memory lobby that tracks hosted games.
///
/// Uses a [`BTreeMap`] for deterministic iteration order (per project rules).
pub struct LobbyServer {
    games: BTreeMap<u64, GameListing>,
    next_game_id: u64,
}

impl LobbyServer {
    /// Create an empty lobby.
    pub fn new() -> Self {
        Self {
            games: BTreeMap::new(),
            next_game_id: 1,
        }
    }

    /// Host a new game and return its `game_id`.
    pub fn host_game(&mut self, host_name: String, map_name: String, max_players: u8) -> u64 {
        let game_id = self.next_game_id;
        self.next_game_id += 1;

        let listing = GameListing {
            game_id,
            host_name,
            map_name,
            max_players,
            current_players: 1, // host counts as player 1
            state: GameState::Lobby,
        };
        self.games.insert(game_id, listing);
        game_id
    }

    /// Join an existing game. Fails if full or not in `Lobby` state.
    pub fn join_game(&mut self, game_id: u64) -> Result<(), LobbyError> {
        let game = self
            .games
            .get_mut(&game_id)
            .ok_or(LobbyError::GameNotFound(game_id))?;

        if game.state != GameState::Lobby {
            return Err(LobbyError::NotInLobby(game_id));
        }
        if game.current_players >= game.max_players {
            return Err(LobbyError::GameFull(game_id));
        }

        game.current_players += 1;
        Ok(())
    }

    /// Leave a game. If the last player leaves, the game is removed.
    pub fn leave_game(&mut self, game_id: u64) -> Result<(), LobbyError> {
        let game = self
            .games
            .get_mut(&game_id)
            .ok_or(LobbyError::GameNotFound(game_id))?;

        game.current_players = game.current_players.saturating_sub(1);

        if game.current_players == 0 {
            self.games.remove(&game_id);
        }

        Ok(())
    }

    /// Transition a game from `Lobby` to `InProgress`.
    pub fn start_game(&mut self, game_id: u64) -> Result<(), LobbyError> {
        let game = self
            .games
            .get_mut(&game_id)
            .ok_or(LobbyError::GameNotFound(game_id))?;

        if game.state != GameState::Lobby {
            return Err(LobbyError::NotInLobby(game_id));
        }

        game.state = GameState::InProgress;
        Ok(())
    }

    /// List all games currently in the `Lobby` state.
    pub fn list_games(&self) -> Vec<&GameListing> {
        self.games
            .values()
            .filter(|g| g.state == GameState::Lobby)
            .collect()
    }

    /// Look up a game by id.
    pub fn get_game(&self, game_id: u64) -> Option<&GameListing> {
        self.games.get(&game_id)
    }
}

impl Default for LobbyServer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "lobby_tests.rs"]
mod tests;
