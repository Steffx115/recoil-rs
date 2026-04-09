//! GPU buffer layout types for compute shaders.
//!
//! All structs are `#[repr(C)]` and derive `bytemuck::Pod` for zero-copy
//! upload to GPU buffers. SimFloat i64 values are stored as `[i32; 2]`
//! (lo, hi) matching the little-endian memory layout.

use bytemuck::{Pod, Zeroable};

/// Per-unit fog input (32 bytes, aligned).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuFogUnit {
    /// Position X as i64 split into [lo, hi].
    pub pos_x: [i32; 2],
    /// Position Z as i64 split into [lo, hi].
    pub pos_z: [i32; 2],
    /// Sight range as i64 split into [lo, hi].
    pub range: [i32; 2],
    /// Team ID.
    pub team: u32,
    pub _pad: u32,
}

/// Fog uniform parameters (constant per dispatch).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuFogParams {
    pub width: u32,
    pub height: u32,
    pub cell_size: [i32; 2],
    pub half_cell: [i32; 2],
    pub unit_count: u32,
    pub team_count: u32,
}

impl GpuFogUnit {
    pub fn from_raw(pos_x: i64, pos_z: i64, range: i64, team: u8) -> Self {
        Self {
            pos_x: i64_to_pair(pos_x),
            pos_z: i64_to_pair(pos_z),
            range: i64_to_pair(range),
            team: team as u32,
            _pad: 0,
        }
    }
}

/// Split an i64 into [lo, hi] i32 pair (little-endian).
#[inline]
pub fn i64_to_pair(v: i64) -> [i32; 2] {
    [v as i32, (v >> 32) as i32]
}

/// Reconstruct i64 from [lo, hi] i32 pair.
#[inline]
pub fn pair_to_i64(p: [i32; 2]) -> i64 {
    (p[0] as u32 as i64) | ((p[1] as i64) << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_i64() {
        for v in [0i64, 1, -1, i64::MAX, i64::MIN, 42 << 32, -(100 << 32)] {
            assert_eq!(pair_to_i64(i64_to_pair(v)), v);
        }
    }
}
