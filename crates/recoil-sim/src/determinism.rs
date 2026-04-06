//! Determinism test harness.
//!
//! Runs the same simulation twice with identical inputs and asserts
//! identical output. This is the single most important test in the
//! project — if it fails, nothing else matters.

use std::hash::{Hash, Hasher};

/// A frame-by-frame checksum of simulation state, used to detect desyncs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimChecksum {
    pub frame: u64,
    pub hash: u64,
}

/// Compute a deterministic hash for any hashable sim state.
pub fn checksum<T: Hash>(frame: u64, state: &T) -> SimChecksum {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.hash(&mut hasher);
    SimChecksum {
        frame,
        hash: hasher.finish(),
    }
}

/// Compare two checksum traces and return the first frame where they diverge.
pub fn find_divergence_frame(a: &[SimChecksum], b: &[SimChecksum]) -> Option<u64> {
    let len = a.len().min(b.len());
    for i in 0..len {
        if a[i].hash != b[i].hash {
            return Some(a[i].frame);
        }
    }
    if a.len() != b.len() {
        return Some(len as u64);
    }
    None
}

/// Run a simulation function twice with the same inputs, assert checksums match.
///
/// `run_sim` takes a clone of the commands and returns per-frame checksums.
/// This is the core determinism assertion used throughout the test suite.
pub fn assert_deterministic<F, C>(commands: &C, run_sim: F)
where
    F: Fn(&C) -> Vec<SimChecksum>,
    C: Clone,
{
    let trace_a = run_sim(commands);
    let trace_b = run_sim(&commands.clone());

    assert_eq!(
        trace_a.len(),
        trace_b.len(),
        "Simulation produced different number of frames"
    );

    if let Some(frame) = find_divergence_frame(&trace_a, &trace_b) {
        panic!("Determinism violation: desync at frame {frame}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_traces_pass() {
        let trace = vec![
            SimChecksum { frame: 0, hash: 1 },
            SimChecksum { frame: 1, hash: 2 },
        ];
        assert_eq!(find_divergence_frame(&trace, &trace), None);
    }

    #[test]
    fn divergent_traces_detected() {
        let a = vec![
            SimChecksum { frame: 0, hash: 1 },
            SimChecksum { frame: 1, hash: 2 },
        ];
        let b = vec![
            SimChecksum { frame: 0, hash: 1 },
            SimChecksum { frame: 1, hash: 99 },
        ];
        assert_eq!(find_divergence_frame(&a, &b), Some(1));
    }

    #[test]
    fn trivial_determinism() {
        // A trivial "simulation" that just hashes integers — must be deterministic.
        assert_deterministic(&10u64, |&frames| {
            (0..frames).map(|f| checksum(f, &(f * 42))).collect()
        });
    }
}
