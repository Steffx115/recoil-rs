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

/// Per-shooter targeting input (128 bytes, aligned).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuShooter {
    pub pos_x: [i32; 2],
    pub pos_y: [i32; 2],
    pub pos_z: [i32; 2],
    pub max_range: [i32; 2],
    pub min_range_0: [i32; 2],
    pub min_range_1: [i32; 2],
    pub min_range_2: [i32; 2],
    pub min_range_3: [i32; 2],
    pub team: u32,
    pub fire_mode: u32,
    pub has_indirect: u32,
    pub weapon_count: u32,
    pub manual_target_idx: i32,
    pub last_attacker_idx: i32,
    pub _pad0: u32,
    pub _pad1: u32,
}

/// Per-candidate targeting input (64 bytes, aligned).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuCandidate {
    pub pos_x: [i32; 2],
    pub pos_y: [i32; 2],
    pub pos_z: [i32; 2],
    pub health: [i32; 2],
    pub pending_damage: [i32; 2],
    pub sim_id_lo: u32,
    pub sim_id_hi: u32,
    pub team: u32,
    pub flags: u32, // bit 0: is_dead, bit 1: has_weapons, bit 2: is_building
    pub _pad0: u32,
    pub _pad1: u32,
}

/// Targeting uniform parameters.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuTargetingParams {
    pub shooter_count: u32,
    pub candidate_count: u32,
    pub fog_width: u32,
    pub fog_height: u32,
    pub fog_cell_size: [i32; 2],
    pub has_fog: u32,
    pub _pad: u32,
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
