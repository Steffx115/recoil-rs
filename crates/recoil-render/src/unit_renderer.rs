use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::GpuContext;
use crate::unit_mesh::{generate_unit_mesh, UnitVertex};

// ---------------------------------------------------------------------------
// Per-instance data
// ---------------------------------------------------------------------------

/// Per-instance data for a unit: world position, heading (Y-axis rotation),
/// and team color. Team color is applied in the fragment shader.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct UnitInstance {
    pub position: [f32; 3],
    pub heading: f32,
    pub team_color: [f32; 3],
    /// Padding to align to 16-byte boundary (required by some GPU drivers).
    pub _pad: f32,
}

impl UnitInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<UnitInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            // instance position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x3,
            },
            // heading
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32,
            },
            // team_color
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    };
}

// ---------------------------------------------------------------------------
// WGSL shader
// ---------------------------------------------------------------------------

const UNIT_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) team_color: vec3<f32>,
    @location(2) base_color: vec3<f32>,
}

@vertex
fn vs_main(
    // Per-vertex
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // Per-instance
    @location(3) inst_position: vec3<f32>,
    @location(4) heading: f32,
    @location(5) team_color: vec3<f32>,
) -> VertexOutput {
    // Y-axis rotation by heading
    let c = cos(heading);
    let s = sin(heading);

    let rotated = vec3<f32>(
        position.x * c + position.z * s,
        position.y,
        -position.x * s + position.z * c,
    );

    let rotated_normal = vec3<f32>(
        normal.x * c + normal.z * s,
        normal.y,
        -normal.x * s + normal.z * c,
    );

    let world_pos = rotated + inst_position;

    var out: VertexOutput;
    out.pos = uniforms.view_proj * vec4<f32>(world_pos, 1.0);
    out.normal = rotated_normal;
    out.team_color = team_color;
    out.base_color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Simple directional light from upper-right
    let light_dir = normalize(vec3<f32>(0.5, 0.8, 0.3));
    let n = normalize(in.normal);
    let ndl = max(dot(n, light_dir), 0.0);

    // Ambient + diffuse
    let ambient = 0.25;
    let diffuse = ndl * 0.75;
    let lighting = ambient + diffuse;

    // Mix base mesh color with team color
    let color = in.base_color * 0.3 + in.team_color * 0.7;
    return vec4<f32>(color * lighting, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// UnitRenderer
// ---------------------------------------------------------------------------

/// Manages GPU resources for instanced unit rendering.
pub struct UnitRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

impl UnitRenderer {
    /// Create the unit render pipeline and upload the placeholder mesh.
    ///
    /// `camera_bind_group_layout` must match the terrain's camera bind group
    /// layout so both pipelines can share the same camera uniform.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unit_shader"),
            source: wgpu::ShaderSource::Wgsl(UNIT_SHADER.into()),
        });

        // Mesh buffers
        let (vertices, indices) = generate_unit_mesh();
        let index_count = indices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit_index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Empty instance buffer (will be resized on prepare)
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unit_instance_buffer"),
            size: 256, // small initial allocation
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Pipeline
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("unit_pipeline_layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("unit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[UnitVertex::LAYOUT, UnitInstance::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: GpuContext::DEPTH_FORMAT,
                depth_write_enabled: true,
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
            vertex_buffer,
            index_buffer,
            index_count,
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
        instances: &[UnitInstance],
    ) {
        self.instance_count = instances.len() as u32;
        if instances.is_empty() {
            return;
        }

        let data = bytemuck::cast_slice(instances);
        let required = data.len() as u64;

        if required > self.instance_buffer.size() {
            // Grow with some headroom to avoid frequent re-allocs.
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("unit_instance_buffer"),
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
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_instance_size() {
        // 3 floats position + 1 heading + 3 team_color + 1 pad = 8 * 4 = 32 bytes
        assert_eq!(std::mem::size_of::<UnitInstance>(), 32);
    }

    #[test]
    fn unit_instance_is_pod() {
        // Compile-time check: Pod + Zeroable are derived; this just exercises it.
        let inst = UnitInstance::zeroed();
        assert_eq!(inst.heading, 0.0);
        assert_eq!(inst.team_color, [0.0, 0.0, 0.0]);
    }
}
