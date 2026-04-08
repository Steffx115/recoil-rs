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
