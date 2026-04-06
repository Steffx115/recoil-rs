//! Lobby and game setup screen for multiplayer match configuration.

use egui::{ComboBox, Context};

/// The hosting mode for the lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyMode {
    Host,
    Join,
}

/// Faction selection for a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Armada,
    Cortex,
}

impl std::fmt::Display for Faction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Faction::Armada => write!(f, "Armada"),
            Faction::Cortex => write!(f, "Cortex"),
        }
    }
}

/// A player entry in the lobby.
#[derive(Debug, Clone)]
pub struct LobbyPlayer {
    pub name: String,
    pub faction: Faction,
    pub team: u8,
    pub ready: bool,
}

/// Full lobby state for the game setup screen.
pub struct LobbyState {
    pub mode: LobbyMode,
    pub host_ip: String,
    pub join_ip: String,
    pub player_name: String,
    pub selected_map: usize,
    pub available_maps: Vec<String>,
    pub faction: Faction,
    pub players: Vec<LobbyPlayer>,
    pub ready: bool,
}

impl Default for LobbyState {
    fn default() -> Self {
        Self {
            mode: LobbyMode::Host,
            host_ip: "0.0.0.0:7777".to_string(),
            join_ip: String::new(),
            player_name: "Player".to_string(),
            selected_map: 0,
            available_maps: vec!["Default Map".to_string()],
            faction: Faction::Armada,
            players: Vec::new(),
            ready: false,
        }
    }
}

/// Actions that can be triggered from the lobby UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyAction {
    None,
    StartGame,
    ReadyToggle,
    Disconnect,
}

/// Draw the lobby/game setup screen. Returns a [`LobbyAction`] indicating
/// what the caller should do next.
pub fn draw_lobby(ctx: &Context, state: &mut LobbyState) -> LobbyAction {
    let mut action = LobbyAction::None;

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Game Lobby");
        ui.separator();

        // Mode selection: Host / Join
        ui.horizontal(|ui| {
            ui.label("Mode:");
            ui.radio_value(&mut state.mode, LobbyMode::Host, "Host");
            ui.radio_value(&mut state.mode, LobbyMode::Join, "Join");
        });

        // IP input
        match state.mode {
            LobbyMode::Host => {
                ui.horizontal(|ui| {
                    ui.label("Listen address:");
                    ui.text_edit_singleline(&mut state.host_ip);
                });
            }
            LobbyMode::Join => {
                ui.horizontal(|ui| {
                    ui.label("Server IP:");
                    ui.text_edit_singleline(&mut state.join_ip);
                });
            }
        }

        // Player name
        ui.horizontal(|ui| {
            ui.label("Player name:");
            ui.text_edit_singleline(&mut state.player_name);
        });

        // Map selection
        if !state.available_maps.is_empty() {
            let selected_label = state
                .available_maps
                .get(state.selected_map)
                .cloned()
                .unwrap_or_default();

            ComboBox::from_label("Map")
                .selected_text(&selected_label)
                .show_ui(ui, |ui| {
                    for (i, map_name) in state.available_maps.iter().enumerate() {
                        ui.selectable_value(&mut state.selected_map, i, map_name);
                    }
                });
        }

        // Faction selection
        ui.horizontal(|ui| {
            ui.label("Faction:");
            ui.radio_value(&mut state.faction, Faction::Armada, "Armada");
            ui.radio_value(&mut state.faction, Faction::Cortex, "Cortex");
        });

        ui.separator();

        // Player list table
        ui.heading("Players");
        egui::Grid::new("lobby_player_grid")
            .striped(true)
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("Name");
                ui.label("Faction");
                ui.label("Team");
                ui.label("Ready");
                ui.end_row();

                for player in &state.players {
                    ui.label(&player.name);
                    ui.label(player.faction.to_string());
                    ui.label(format!("{}", player.team));
                    ui.label(if player.ready { "Yes" } else { "No" });
                    ui.end_row();
                }
            });

        ui.separator();

        // Action buttons
        ui.horizontal(|ui| {
            let ready_label = if state.ready { "Unready" } else { "Ready" };
            if ui.button(ready_label).clicked() {
                action = LobbyAction::ReadyToggle;
            }

            // Start Game: only for host, only when all players are ready
            if state.mode == LobbyMode::Host {
                let all_ready = !state.players.is_empty() && state.players.iter().all(|p| p.ready);
                let start_btn = ui.add_enabled(all_ready, egui::Button::new("Start Game"));
                if start_btn.clicked() {
                    action = LobbyAction::StartGame;
                }
            }

            if ui.button("Disconnect").clicked() {
                action = LobbyAction::Disconnect;
            }
        });
    });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_renders_without_panic() {
        let ctx = egui::Context::default();
        let mut state = LobbyState::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            draw_lobby(ctx, &mut state);
        });
    }

    #[test]
    fn default_state_is_valid() {
        let state = LobbyState::default();
        assert_eq!(state.mode, LobbyMode::Host);
        assert!(!state.player_name.is_empty());
        assert!(!state.available_maps.is_empty());
        assert_eq!(state.selected_map, 0);
        assert_eq!(state.faction, Faction::Armada);
        assert!(!state.ready);
        assert!(state.players.is_empty());
    }

    #[test]
    fn lobby_action_default_is_none() {
        let ctx = egui::Context::default();
        let mut state = LobbyState::default();
        let mut action = LobbyAction::None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            action = draw_lobby(ctx, &mut state);
        });
        assert_eq!(action, LobbyAction::None);
    }

    #[test]
    fn faction_display() {
        assert_eq!(Faction::Armada.to_string(), "Armada");
        assert_eq!(Faction::Cortex.to_string(), "Cortex");
    }
}
