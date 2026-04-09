pub mod ai;
pub mod building;
pub mod game;
pub mod production;
pub mod setup;

// ---------------------------------------------------------------------------
// Seeded LCG (no rand crate) — deterministic RNG for game logic
// ---------------------------------------------------------------------------

/// A simple linear congruential generator for deterministic random numbers.
/// Used by AI and game setup. No external dependencies.
pub struct Lcg {
    state: u64,
}

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.state >> 33) as u32
    }

    pub fn next_f32(&mut self, max: f32) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * max
    }
}

// Re-export key types for convenience
pub use building::PlacementType;
pub use game::{GameOver, GameState};
pub use setup::{GameConfig, InitOptions};
