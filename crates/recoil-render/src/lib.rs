//! wgpu rendering pipeline for the Recoil RTS engine.
//!
//! Provides GPU initialization, a perspective camera, terrain rendering,
//! and a top-level [`Renderer`] that ties everything together.

pub mod camera;
pub mod gpu;
pub mod renderer;
pub mod terrain;

pub use camera::Camera;
pub use gpu::GpuContext;
pub use renderer::Renderer;
pub use terrain::{TerrainResources, TerrainVertex};
