//! wgpu rendering pipeline for the Recoil RTS engine.
//!
//! Provides GPU initialization, a perspective camera, terrain rendering,
//! and a top-level [`Renderer`] that ties everything together.

pub mod camera;
pub mod gpu;
pub mod model_registry;
pub mod obj_loader;
pub mod particles;
pub mod projectile_renderer;
pub mod renderer;
pub mod s3o_loader;
pub mod terrain;
pub mod unit_mesh;
pub mod unit_renderer;

pub use camera::Camera;
pub use gpu::GpuContext;
pub use model_registry::{LoadedModel, ModelRegistry};
pub use obj_loader::{load_obj, load_obj_file};
pub use particles::{Particle, ParticleSystem};
pub use projectile_renderer::{ProjectileInstance, ProjectileRenderer};
pub use renderer::Renderer;
pub use s3o_loader::{load_s3o, load_s3o_file};
pub use terrain::{TerrainResources, TerrainVertex};
pub use unit_mesh::UnitVertex;
pub use unit_renderer::{UnitInstance, UnitRenderer};
