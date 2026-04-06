//! In-game UI framework for the Recoil RTS engine (egui-based).

pub mod command_panel;
pub mod resource_bar;
pub mod selection_panel;
pub mod status_bar;

pub use command_panel::{draw_command_panel, UiCommand};
pub use resource_bar::draw_resource_bar;
pub use selection_panel::{draw_selection_panel, SelectedUnitInfo};
pub use status_bar::draw_status_bar;
