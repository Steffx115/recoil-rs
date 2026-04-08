//! wgpu rendering pipeline for the Recoil RTS engine.
//!
//! Provides GPU initialization, a perspective camera, terrain rendering,
//! and a top-level [`Renderer`] that ties everything together.

pub mod backend;
pub mod camera;
pub mod gpu;
pub mod model_registry;
pub mod obj_loader;
pub mod particles;
pub mod projectile_renderer;
pub mod renderer;
pub mod shadow;
pub mod terrain;
pub mod unit_mesh;
pub mod unit_renderer;

pub use backend::{MeshId, NullBackend, RenderBackend, RenderError, TextureId, WgpuBackend};
pub use camera::Camera;
pub use gpu::GpuContext;
pub use model_registry::{LoadedModel, ModelRegistry};
pub use obj_loader::{load_obj, load_obj_file};
pub use particles::{Particle, ParticleSystem};
pub use projectile_renderer::{ProjectileInstance, ProjectileRenderer};
pub use renderer::Renderer;
pub use shadow::{ShadowResources, ShadowUniforms};
pub use terrain::{generate_heightmap_grid, TerrainResources, TerrainVertex};
pub use unit_mesh::{unit_vertex_layout, UnitVertex};
pub use unit_renderer::{UnitInstance, UnitRenderer};

// Re-exports from extracted crates for backwards compatibility.
pub use pierce_cob::{parse_cob, CobScript, CobVm};
pub use pierce_model::{
    flatten_with_transforms, ModelVertex, PieceNode, PieceTransform, PieceTree,
};
pub use pierce_s3o::{load_s3o, load_s3o_file, load_s3o_tree};

/// Backwards-compatible alias for [`PieceTree`].
pub type S3oPieceTree = pierce_model::PieceTree;
