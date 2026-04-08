//! COB animation bytecode parser and VM for the Pierce RTS engine.
//!
//! Provides parsing of Spring COB (compiled BOS) files and a stack-based
//! virtual machine for executing animation scripts.

pub mod loader;
pub mod vm;

pub use loader::{parse_cob, CobError, CobFunction, CobScript};
pub use vm::{CobThread, CobVm, PieceAnimState, WaitCondition};
