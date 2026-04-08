//! Render abstraction layer for GPU backend portability.
//!
//! Defines the [`RenderBackend`] trait that decouples game code from wgpu
//! specifics, enabling future migration to other GPU backends. Includes a
//! [`WgpuBackend`] wrapping the existing renderer and a [`NullBackend`] for
//! headless testing without a GPU.

use std::collections::HashMap;
use std::hash::Hash;

use crate::unit_mesh::UnitVertex;

// ---------------------------------------------------------------------------
// Opaque handle types
// ---------------------------------------------------------------------------

/// Opaque handle to a registered mesh. Game code uses this instead of
/// backend-specific buffer types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MeshId(pub u32);

/// Opaque handle to a registered texture.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during render backend operations.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The surface was lost or outdated and needs reconfiguration.
    #[error("surface lost")]
    SurfaceLost,

    /// An internal backend error occurred.
    #[error("backend error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over a GPU rendering backend.
///
/// Game code programs against this trait so it is not coupled to any specific
/// graphics API. The associated handle types let each backend use its own
/// representation while exposing only opaque identifiers to callers.
pub trait RenderBackend {
    /// Handle type returned when a mesh is registered.
    type MeshHandle: Copy + Eq + Hash;

    /// Handle type returned when a texture is registered.
    type TextureHandle: Copy + Eq + Hash;

    /// Upload a mesh (vertices + indices) and return a handle.
    fn register_mesh(&mut self, vertices: &[UnitVertex], indices: &[u16]) -> Self::MeshHandle;

    /// Replace the data of a previously registered mesh.
    fn update_mesh(&mut self, handle: Self::MeshHandle, vertices: &[UnitVertex], indices: &[u16]);

    /// Upload a 2D RGBA texture and return a handle.
    fn register_texture(&mut self, width: u32, height: u32, data: &[u8]) -> Self::TextureHandle;

    /// Begin a new frame. Must be called before any draw commands.
    fn begin_frame(&mut self) -> Result<(), RenderError>;

    /// Finish the current frame and present.
    fn end_frame(&mut self);

    /// Notify the backend that the output surface was resized.
    fn resize(&mut self, width: u32, height: u32);

    /// Upload the combined view-projection matrix for the current frame.
    fn set_camera(&mut self, view_proj: [[f32; 4]; 4]);
}

// ---------------------------------------------------------------------------
// WgpuBackend — wraps the existing Renderer
// ---------------------------------------------------------------------------

/// A [`RenderBackend`] implementation backed by the existing wgpu
/// [`Renderer`](crate::renderer::Renderer).
///
/// This is a thin adapter: mesh/texture registration is tracked locally with
/// opaque IDs while the heavy lifting stays in the original renderer code.
/// Migration of individual sub-systems to the trait is future work.
pub struct WgpuBackend {
    /// The existing top-level wgpu renderer.
    pub renderer: crate::renderer::Renderer,
    next_mesh_id: u32,
    next_texture_id: u32,
}

impl WgpuBackend {
    /// Wrap an already-initialised [`Renderer`](crate::renderer::Renderer).
    pub fn new(renderer: crate::renderer::Renderer) -> Self {
        Self {
            renderer,
            next_mesh_id: 0,
            next_texture_id: 0,
        }
    }
}

impl RenderBackend for WgpuBackend {
    type MeshHandle = MeshId;
    type TextureHandle = TextureId;

    fn register_mesh(&mut self, vertices: &[UnitVertex], indices: &[u16]) -> MeshId {
        let id = self.next_mesh_id;
        self.next_mesh_id += 1;
        self.renderer.register_unit_mesh(id, vertices, indices);
        MeshId(id)
    }

    fn update_mesh(&mut self, handle: MeshId, vertices: &[UnitVertex], indices: &[u16]) {
        self.renderer
            .register_unit_mesh(handle.0, vertices, indices);
    }

    fn register_texture(&mut self, _width: u32, _height: u32, _data: &[u8]) -> TextureId {
        // Texture registration is not yet wired through the existing renderer.
        // Allocate an ID so the API contract holds; actual GPU upload is future
        // work.
        let id = self.next_texture_id;
        self.next_texture_id += 1;
        TextureId(id)
    }

    fn begin_frame(&mut self) -> Result<(), RenderError> {
        // The existing renderer acquires the surface texture inside `render()`.
        // For now this is a no-op; a future refactor will split acquire/present.
        Ok(())
    }

    fn end_frame(&mut self) {
        // Presentation is handled by the caller via `Renderer::render()`.
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    fn set_camera(&mut self, view_proj: [[f32; 4]; 4]) {
        // The existing renderer manages camera via its own Camera struct.
        // Store the raw matrix for potential future use; camera sync is
        // currently done through `Renderer::update_camera`.
        let _ = view_proj;
    }
}

// ---------------------------------------------------------------------------
// NullBackend — headless no-op backend for testing
// ---------------------------------------------------------------------------

/// A headless no-op [`RenderBackend`] for tests that need render API calls
/// without a GPU. Every operation succeeds but performs no actual work.
pub struct NullBackend {
    next_mesh_id: u32,
    next_texture_id: u32,
    meshes: HashMap<MeshId, (Vec<UnitVertex>, Vec<u16>)>,
    textures: HashMap<TextureId, (u32, u32, Vec<u8>)>,
    current_camera: [[f32; 4]; 4],
    width: u32,
    height: u32,
    frame_active: bool,
}

impl NullBackend {
    /// Create a new headless backend with the given initial dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            next_mesh_id: 0,
            next_texture_id: 0,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            current_camera: [[0.0; 4]; 4],
            width,
            height,
            frame_active: false,
        }
    }

    /// Number of meshes currently registered.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Number of textures currently registered.
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Current surface dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Whether a frame is currently in progress (between begin/end).
    pub fn frame_active(&self) -> bool {
        self.frame_active
    }

    /// The last camera matrix set via [`RenderBackend::set_camera`].
    pub fn current_camera(&self) -> [[f32; 4]; 4] {
        self.current_camera
    }
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new(800, 600)
    }
}

impl RenderBackend for NullBackend {
    type MeshHandle = MeshId;
    type TextureHandle = TextureId;

    fn register_mesh(&mut self, vertices: &[UnitVertex], indices: &[u16]) -> MeshId {
        let id = MeshId(self.next_mesh_id);
        self.next_mesh_id += 1;
        self.meshes
            .insert(id, (vertices.to_vec(), indices.to_vec()));
        id
    }

    fn update_mesh(&mut self, handle: MeshId, vertices: &[UnitVertex], indices: &[u16]) {
        self.meshes
            .insert(handle, (vertices.to_vec(), indices.to_vec()));
    }

    fn register_texture(&mut self, width: u32, height: u32, data: &[u8]) -> TextureId {
        let id = TextureId(self.next_texture_id);
        self.next_texture_id += 1;
        self.textures.insert(id, (width, height, data.to_vec()));
        id
    }

    fn begin_frame(&mut self) -> Result<(), RenderError> {
        self.frame_active = true;
        Ok(())
    }

    fn end_frame(&mut self) {
        self.frame_active = false;
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn set_camera(&mut self, view_proj: [[f32; 4]; 4]) {
        self.current_camera = view_proj;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
