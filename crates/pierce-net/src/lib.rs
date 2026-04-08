//! Lockstep networking protocol and replay system for the Pierce RTS engine.

pub mod codec;
pub mod lobby;
pub mod lockstep;
pub mod protocol;
pub mod replay;
pub mod sync;

pub use codec::{decode, decode_framed, encode, encode_framed};
pub use lobby::{GameListing, GameState, LobbyError, LobbyMessage, LobbyServer};
pub use lockstep::LockstepState;
pub use protocol::{CommandFrame, NetMessage, PlayerCommand};
pub use replay::{
    load_replay, save_replay, Replay, ReplayError, ReplayHeader, ReplayPlayer, ReplayRecorder,
};
pub use sync::{compute_component_hashes, ComponentHashes, SyncStatus, SyncValidator};
