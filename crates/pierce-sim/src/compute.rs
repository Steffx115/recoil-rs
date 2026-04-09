//! Compute backend traits for GPU-offloadable systems.
//!
//! `pierce-sim` defines these traits with no wgpu dependency.
//! Implementations live in `pierce-compute` (CPU fallback and GPU backends).
//! When the `ComputeBackends` resource is present in the ECS world,
//! fog and targeting systems dispatch to it instead of running inline.

use std::collections::BTreeMap;

use bevy_ecs::system::Resource;

// ---------------------------------------------------------------------------
// Fog compute
// ---------------------------------------------------------------------------

/// Input data for a single unit in the fog computation.
#[derive(Debug, Clone, Copy)]
pub struct FogUnitInput {
    /// `Position.pos.x.raw()` — 32.32 fixed-point as i64.
    pub pos_x_raw: i64,
    /// `Position.pos.z.raw()` — 32.32 fixed-point as i64.
    pub pos_z_raw: i64,
    /// `SightRange.range.raw()` — 32.32 fixed-point as i64.
    pub range_raw: i64,
    /// Team allegiance.
    pub team: u8,
}

/// Parameters for the fog grid.
#[derive(Debug, Clone)]
pub struct FogGridParams {
    pub width: u32,
    pub height: u32,
    /// `cell_size.raw()` — 32.32 fixed-point as i64.
    pub cell_size_raw: i64,
    /// Sorted team IDs.
    pub teams: Vec<u8>,
}

/// Trait for fog-of-war computation backend.
///
/// Takes unit inputs and the previous frame's grids, returns updated grids.
/// Each grid is `width * height` bytes: 0=Unexplored, 1=Explored, 2=Visible.
pub trait FogCompute: Send + Sync {
    fn compute_fog(
        &mut self,
        params: &FogGridParams,
        units: &[FogUnitInput],
        previous_grids: &BTreeMap<u8, Vec<u8>>,
    ) -> BTreeMap<u8, Vec<u8>>;
}

// ---------------------------------------------------------------------------
// Targeting compute
// ---------------------------------------------------------------------------

/// Input data for a shooter in targeting computation.
#[derive(Debug, Clone, Copy)]
pub struct TargetingShooterInput {
    /// Index of this shooter (for output mapping).
    pub index: u32,
    pub pos_x_raw: i64,
    pub pos_y_raw: i64,
    pub pos_z_raw: i64,
    pub team: u8,
    pub max_range_raw: i64,
    /// 0=FireAtWill, 1=ReturnFire, 2=HoldFire.
    pub fire_mode: u8,
    /// Whether any weapon has indirect fire.
    pub has_indirect: bool,
    /// Index into candidates array for manual target, or -1.
    pub manual_target_idx: i32,
    /// Index into candidates array for last attacker, or -1.
    pub last_attacker_idx: i32,
    /// Per-weapon min ranges (raw i64). Up to 4 weapons.
    pub weapon_min_ranges: [i64; 4],
    /// Number of weapons (0..4).
    pub weapon_count: u8,
}

/// Input data for a candidate target.
#[derive(Debug, Clone, Copy)]
pub struct TargetingCandidateInput {
    pub pos_x_raw: i64,
    pub pos_y_raw: i64,
    pub pos_z_raw: i64,
    pub team: u8,
    pub is_dead: bool,
    /// `Health.current.raw()`.
    pub health_raw: i64,
    /// `SimId.id`.
    pub sim_id: u64,
    pub has_weapons: bool,
    pub is_building: bool,
    /// Pending incoming damage (raw i64).
    pub pending_damage_raw: i64,
}

/// Trait for targeting computation backend.
///
/// For each shooter, finds the best target index into the candidates array.
/// Returns -1 for no target.
pub trait TargetCompute: Send + Sync {
    fn compute_targets(
        &mut self,
        shooters: &[TargetingShooterInput],
        candidates: &[TargetingCandidateInput],
        fog_grids: Option<&BTreeMap<u8, Vec<u8>>>,
        fog_width: u32,
        fog_height: u32,
        fog_cell_size_raw: i64,
    ) -> Vec<i32>;
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// ECS resource holding compute backends. When present, fog and targeting
/// systems dispatch to these instead of running inline CPU code.
#[derive(Resource)]
pub struct ComputeBackends {
    pub fog: Box<dyn FogCompute>,
    pub targeting: Box<dyn TargetCompute>,
}

/// ECS resource holding the batch math backend. When present, systems
/// can offload bulk fixed-point operations to CPU/GPU.
#[derive(Resource)]
pub struct BatchMathBackend {
    pub ops: Box<dyn pierce_math::BatchMathOps>,
}
