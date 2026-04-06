use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::GpuContext;

// ---------------------------------------------------------------------------
// Per-instance data
// ---------------------------------------------------------------------------

/// Per-instance data for a projectile or particle: world position, velocity
/// direction (used for optional orientation), color, and size.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ProjectileInstance {
    pub position: [f32; 3],
    pub size: f32,
    pub velocity_dir: [f32; 3],
    pub _pad: f32,
    pub color: [f32; 3],
    pub _pad2: f32,
}

impl ProjectileInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ProjectileInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            // position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            // size
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32,
            },
            // velocity_dir
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x3,
            },
            // color
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    };
}

// ---------------------------------------------------------------------------
// WGSL shader — billboard quad, round shape via distance discard
// ---------------------------------------------------------------------------

const PROJECTILE_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
}

// Quad vertices: two triangles forming a [-0.5, 0.5] quad.
// We generate them from vertex_index (0..5).
@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    // Per-instance
    @location(0) inst_position: vec3<f32>,
    @location(1) size: f32,
    @location(2) velocity_dir: vec3<f32>,
    @location(3) color: vec3<f32>,
) -> VertexOutput {
    // Quad corner offsets (two triangles: 0-1-2, 2-1-3)
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
    );

    let corner = corners[vi];

    // Billboard: extract camera right and up from the view_proj matrix.
    // For a proper billboard we need the inverse view rotation. We can
    // approximate by extracting the first two column vectors of the VP
    // matrix (which encode right and up in clip space before projection).
    // A simpler approach: compute clip-space center, then offset in clip
    // space by the corner scaled by size.
    let center_clip = uniforms.view_proj * vec4<f32>(inst_position, 1.0);

    // Scale the offset in clip space. Divide by w to keep screen-space
    // size roughly constant, then multiply back.
    let offset = corner * size * 0.004;
    let clip_pos = vec4<f32>(
        center_clip.x + offset.x * center_clip.w,
        center_clip.y + offset.y * center_clip.w,
        center_clip.z,
        center_clip.w,
    );

    var out: VertexOutput;
    out.pos = clip_pos;
    out.uv = corner + vec2<f32>(0.5, 0.5); // [0,1] range
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Round shape: discard fragments outside a circle.
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(in.uv, center);
    if dist > 0.5 {
        discard;
    }

    // Soft edge fade
    let alpha = smoothstep(0.5, 0.35, dist);

    // Slight glow: brighter at center
    let glow = 1.0 + (1.0 - dist * 2.0) * 0.5;
    return vec4<f32>(in.color * glow, alpha);
}
"#;

// ---------------------------------------------------------------------------
// ProjectileRenderer
// ---------------------------------------------------------------------------

/// Manages GPU resources for instanced projectile/particle rendering.
pub struct ProjectileRenderer {
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

impl ProjectileRenderer {
    /// Create the projectile render pipeline.
    ///
    /// `camera_bind_group_layout` must match the camera bind group layout used
    /// by other renderers so they can share the same camera uniform.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("projectile_shader"),
            source: wgpu::ShaderSource::Wgsl(PROJECTILE_SHADER.into()),
        });

        // Empty instance buffer (will be resized on prepare)
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("projectile_instance_buffer"),
            size: 256,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("projectile_pipeline_layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("projectile_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[ProjectileInstance::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // billboards face both ways
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: GpuContext::DEPTH_FORMAT,
                depth_write_enabled: false, // transparent; read but don't write depth
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            instance_buffer,
            instance_count: 0,
        }
    }

    /// Upload instance data for this frame. Recreates the instance buffer if
    /// the current one is too small.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[ProjectileInstance],
    ) {
        self.instance_count = instances.len() as u32;
        if instances.is_empty() {
            return;
        }

        let data = bytemuck::cast_slice(instances);
        let required = data.len() as u64;

        if required > self.instance_buffer.size() {
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("projectile_instance_buffer"),
                contents: data,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.instance_buffer, 0, data);
        }
    }

    /// Record draw commands into an existing render pass.
    ///
    /// The caller must have already set bind group 0 to the camera bind group.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        // 6 vertices per quad (two triangles), one quad per instance.
        pass.draw(0..6, 0..self.instance_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projectile_instance_size() {
        // 3+1+3+1+3+1 = 12 floats = 48 bytes
        assert_eq!(std::mem::size_of::<ProjectileInstance>(), 48);
    }

    #[test]
    fn projectile_instance_is_pod() {
        let inst = ProjectileInstance::zeroed();
        assert_eq!(inst.position, [0.0, 0.0, 0.0]);
        assert_eq!(inst.size, 0.0);
        assert_eq!(inst.color, [0.0, 0.0, 0.0]);
    }
}
