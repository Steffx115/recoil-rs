use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use kira::manager::backend::DefaultBackend;
use kira::manager::{AudioManager, AudioManagerSettings};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::tween::Tween;
use tracing::debug;

use crate::events::SoundCategory;

/// Entry in the sound library: the loaded data plus its category.
struct SoundEntry {
    data: StaticSoundData,
    category: SoundCategory,
}

/// The main audio engine. Owns the kira [`AudioManager`] and manages loaded
/// sounds, per-category concurrency limits, and volume levels.
pub struct AudioEngine {
    manager: AudioManager,
    sounds: BTreeMap<String, SoundEntry>,
    active_counts: BTreeMap<SoundCategory, u32>,
    max_per_category: u32,
    master_volume: f64,
    category_volumes: BTreeMap<SoundCategory, f64>,
}

impl AudioEngine {
    /// Create a new `AudioEngine` backed by the default audio device.
    pub fn new() -> Result<Self> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .context("failed to initialise kira AudioManager")?;

        Ok(Self {
            manager,
            sounds: BTreeMap::new(),
            active_counts: BTreeMap::new(),
            max_per_category: 8,
            master_volume: 1.0,
            category_volumes: BTreeMap::new(),
        })
    }

    /// Load a sound from disk and register it under `name`.
    pub fn load_sound(&mut self, name: &str, path: &Path, category: SoundCategory) -> Result<()> {
        let data = StaticSoundData::from_file(path)
            .with_context(|| format!("failed to load sound file: {}", path.display()))?;
        self.sounds
            .insert(name.to_owned(), SoundEntry { data, category });
        debug!(name, ?category, "loaded sound");
        Ok(())
    }

    /// Play a previously loaded sound.
    ///
    /// * `position` — world-space position of the sound source. `None` for
    ///   non-positional (e.g. UI) sounds.
    /// * `listener_pos` — the camera / listener position used for attenuation.
    pub fn play(
        &mut self,
        name: &str,
        position: Option<[f32; 3]>,
        listener_pos: [f32; 3],
    ) -> Result<()> {
        let entry = self
            .sounds
            .get(name)
            .with_context(|| format!("unknown sound: {name}"))?;

        let category = entry.category;

        // Concurrency limit per category.
        let count = self.active_counts.entry(category).or_insert(0);
        if *count >= self.max_per_category {
            debug!(name, ?category, "skipped — category limit reached");
            return Ok(());
        }

        // Volume: master * category * distance attenuation.
        let cat_vol = self.category_volumes.get(&category).copied().unwrap_or(1.0);
        let attenuation = position
            .map(|pos| compute_attenuation(pos, listener_pos))
            .unwrap_or(1.0);
        let final_volume = self.master_volume * cat_vol * attenuation;

        let settings = StaticSoundSettings::new().volume(final_volume);
        let data = entry.data.with_settings(settings);

        self.manager.play(data).context("failed to play sound")?;

        *self.active_counts.entry(category).or_insert(0) += 1;
        Ok(())
    }

    /// Set the master volume (0.0 – 1.0).
    pub fn set_master_volume(&mut self, vol: f64) {
        self.master_volume = vol.clamp(0.0, 1.0);
        // Push to the kira main track so already-playing sounds are affected.
        self.manager
            .main_track()
            .set_volume(self.master_volume, Tween::default());
    }

    /// Set volume for a specific category (0.0 – 1.0).
    pub fn set_category_volume(&mut self, cat: SoundCategory, vol: f64) {
        self.category_volumes.insert(cat, vol.clamp(0.0, 1.0));
    }

    /// Call once per simulation tick. Resets per-category active counts so new
    /// sounds can be played next tick.
    pub fn tick(&mut self) {
        for count in self.active_counts.values_mut() {
            *count = 0;
        }
    }
}

/// Compute distance-based volume attenuation: `1 / (1 + dist / 100)`.
pub fn compute_attenuation(source: [f32; 3], listener: [f32; 3]) -> f64 {
    let dx = (source[0] - listener[0]) as f64;
    let dy = (source[1] - listener[1]) as f64;
    let dz = (source[2] - listener[2]) as f64;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    1.0 / (1.0 + dist / 100.0)
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
