use bytemuck::{Pod, Zeroable};

/// Model vertex: position + normal + color.
///
/// This is the common vertex format used by all model loaders (S3O, OBJ, etc.).
/// It has the same memory layout as the GPU vertex but does not depend on wgpu.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}
