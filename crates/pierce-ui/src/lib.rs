//! In-game UI framework for the Pierce RTS engine (egui-based).

pub mod command_panel;
pub mod lobby;
pub mod minimap;
pub mod replay_browser;
pub mod resource_bar;
pub mod selection_panel;
pub mod status_bar;

pub use command_panel::{draw_command_panel, UiCommand};
pub use lobby::{draw_lobby, Faction, LobbyAction, LobbyMode, LobbyPlayer, LobbyState};
pub use minimap::{draw_minimap, MinimapData, MinimapResponse, MinimapUnit};
pub use replay_browser::{
    draw_replay_browser, draw_replay_controls, ReplayBrowserAction, ReplayControlAction,
    ReplayEntry,
};
pub use resource_bar::draw_resource_bar;
pub use selection_panel::{draw_selection_panel, SelectedUnitInfo};
pub use status_bar::draw_status_bar;
