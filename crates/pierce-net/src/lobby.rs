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
mod tests {
    use super::*;

    #[test]
    fn host_game_appears_in_list() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);

        let games = lobby.list_games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id, id);
        assert_eq!(games[0].host_name, "Alice");
        assert_eq!(games[0].current_players, 1);
        assert_eq!(games[0].state, GameState::Lobby);
    }

    #[test]
    fn join_game_increments_players() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);

        lobby.join_game(id).unwrap();
        assert_eq!(lobby.get_game(id).unwrap().current_players, 2);
    }

    #[test]
    fn leave_game_decrements_players() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);
        lobby.join_game(id).unwrap();

        lobby.leave_game(id).unwrap();
        assert_eq!(lobby.get_game(id).unwrap().current_players, 1);
    }

    #[test]
    fn leave_game_removes_when_empty() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);

        lobby.leave_game(id).unwrap();
        assert!(lobby.get_game(id).is_none());
        assert!(lobby.list_games().is_empty());
    }

    #[test]
    fn start_game_changes_state() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);

        lobby.start_game(id).unwrap();
        assert_eq!(lobby.get_game(id).unwrap().state, GameState::InProgress);
    }

    #[test]
    fn started_game_not_in_list() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);

        lobby.start_game(id).unwrap();
        assert!(lobby.list_games().is_empty());
    }

    #[test]
    fn cannot_join_full_game() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 2);
        lobby.join_game(id).unwrap(); // 2/2

        let err = lobby.join_game(id).unwrap_err();
        assert_eq!(err, LobbyError::GameFull(id));
    }

    #[test]
    fn cannot_join_in_progress_game() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);
        lobby.start_game(id).unwrap();

        let err = lobby.join_game(id).unwrap_err();
        assert_eq!(err, LobbyError::NotInLobby(id));
    }

    #[test]
    fn cannot_start_already_started_game() {
        let mut lobby = LobbyServer::new();
        let id = lobby.host_game("Alice".into(), "DeltaSiege".into(), 4);
        lobby.start_game(id).unwrap();

        let err = lobby.start_game(id).unwrap_err();
        assert_eq!(err, LobbyError::NotInLobby(id));
    }

    #[test]
    fn join_nonexistent_game_errors() {
        let mut lobby = LobbyServer::new();
        let err = lobby.join_game(999).unwrap_err();
        assert_eq!(err, LobbyError::GameNotFound(999));
    }

    #[test]
    fn leave_nonexistent_game_errors() {
        let mut lobby = LobbyServer::new();
        let err = lobby.leave_game(999).unwrap_err();
        assert_eq!(err, LobbyError::GameNotFound(999));
    }

    #[test]
    fn game_ids_are_unique() {
        let mut lobby = LobbyServer::new();
        let id1 = lobby.host_game("A".into(), "Map1".into(), 2);
        let id2 = lobby.host_game("B".into(), "Map2".into(), 4);
        assert_ne!(id1, id2);
        assert_eq!(lobby.list_games().len(), 2);
    }
}
