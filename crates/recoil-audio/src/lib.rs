//! Spatial audio for the Recoil RTS engine, powered by [kira](https://docs.rs/kira).
//!
//! Game systems push [`SoundEvent`]s into the [`SoundEventQueue`] resource each
//! tick. The presentation layer drains the queue and feeds events into the
//! [`AudioEngine`], which handles positional attenuation, per-category volume,
//! and concurrency limiting.

pub mod engine;
pub mod events;

pub use engine::{compute_attenuation, AudioEngine};
pub use events::{SoundCategory, SoundEvent, SoundEventQueue};
