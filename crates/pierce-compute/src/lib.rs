//! GPU compute backends for fog-of-war and targeting.
//!
//! Provides both CPU fallback and (with feature `gpu`) wgpu-backed
//! implementations of the [`FogCompute`] and [`TargetCompute`] traits
//! defined in `pierce-sim::compute`.

pub mod cpu_fog;
pub mod cpu_targeting;

#[cfg(feature = "gpu")]
pub mod buffers;
#[cfg(feature = "gpu")]
pub mod gpu_context;
#[cfg(feature = "gpu")]
pub mod gpu_fog;

pub use cpu_fog::CpuFogCompute;
pub use cpu_targeting::CpuTargetCompute;

#[cfg(feature = "gpu")]
pub use gpu_context::create_headless_device;
#[cfg(feature = "gpu")]
pub use gpu_fog::GpuFogCompute;
