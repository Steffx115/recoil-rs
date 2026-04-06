use std::fmt;

/// Categories of sounds for volume control and concurrency limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SoundCategory {
    WeaponFire,
    Explosion,
    UnitAcknowledge,
    Ambient,
    UI,
}

impl fmt::Display for SoundCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SoundCategory::WeaponFire => write!(f, "WeaponFire"),
            SoundCategory::Explosion => write!(f, "Explosion"),
            SoundCategory::UnitAcknowledge => write!(f, "UnitAcknowledge"),
            SoundCategory::Ambient => write!(f, "Ambient"),
            SoundCategory::UI => write!(f, "UI"),
        }
    }
}

/// A request to play a sound. Systems push these into [`SoundEventQueue`].
pub struct SoundEvent {
    pub name: String,
    pub category: SoundCategory,
    /// World-space position. `None` for non-positional sounds (e.g. UI clicks).
    pub position: Option<[f32; 3]>,
}

/// Resource that collects [`SoundEvent`]s each tick for the audio engine to drain.
#[derive(Default)]
pub struct SoundEventQueue {
    pub events: Vec<SoundEvent>,
}

impl SoundEventQueue {
    pub fn push(&mut self, event: SoundEvent) {
        self.events.push(event);
    }

    pub fn drain(&mut self) -> Vec<SoundEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_ordering_is_deterministic() {
        // Variants are ordered by discriminant: WeaponFire < Explosion < ... < UI
        assert!(SoundCategory::WeaponFire < SoundCategory::Explosion);
        assert!(SoundCategory::Explosion < SoundCategory::UnitAcknowledge);
        assert!(SoundCategory::UnitAcknowledge < SoundCategory::Ambient);
        assert!(SoundCategory::Ambient < SoundCategory::UI);
    }

    #[test]
    fn event_queue_push_and_drain() {
        let mut queue = SoundEventQueue::default();
        assert!(queue.events.is_empty());

        queue.push(SoundEvent {
            name: "boom".into(),
            category: SoundCategory::Explosion,
            position: Some([1.0, 2.0, 3.0]),
        });
        queue.push(SoundEvent {
            name: "click".into(),
            category: SoundCategory::UI,
            position: None,
        });

        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert!(queue.events.is_empty());
    }
}
