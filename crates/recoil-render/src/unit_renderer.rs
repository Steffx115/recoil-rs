use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::GpuContext;
use crate::unit_mesh::{generate_unit_mesh, UnitVertex};

// ---------------------------------------------------------------------------
// Per-instance data
// ---------------------------------------------------------------------------

/// Per-instance data for a unit.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct UnitInstance {
    pub position: [f32; 3],
    pub heading: f32,
    pub team_color: [f32; 3],
    /// Which mesh to render (0 = placeholder). Set from UnitType.id or a mesh table index.
    pub mesh_id: u32,
}

impl UnitInstance {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<UnitInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x3,
            },
            // mesh_id not passed to shader — used CPU-side for draw grouping only
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

// Shadow bindings (group 1)
struct ShadowUniforms {
    light_vp_0: mat4x4<f32>,
    light_vp_1: mat4x4<f32>,
    cascade_splits: vec4<f32>,
};

@group(1) @binding(0) var shadow_map: texture_depth_2d_array;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;
@group(1) @binding(2) var<uniform> shadow_uniforms: ShadowUniforms;

fn shadow_factor(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    var light_vp: mat4x4<f32>;
    var cascade: u32;
    if (view_depth < shadow_uniforms.cascade_splits.y) {
        light_vp = shadow_uniforms.light_vp_0;
        cascade = 0u;
    } else {
        light_vp = shadow_uniforms.light_vp_1;
        cascade = 1u;
    }
    let light_pos = light_vp * vec4<f32>(world_pos, 1.0);
    let proj = light_pos.xyz / light_pos.w;
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, 1.0 - (proj.y * 0.5 + 0.5));
    let depth = proj.z;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) { return 1.0; }
    let texel_size = 1.0 / 2048.0;
    var total = 0.0;
    for (var x = -1i; x <= 1i; x += 2i) {
        for (var y = -1i; y <= 1i; y += 2i) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            total += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, cascade, depth - 0.005);
        }
    }
    return total / 4.0;
}

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) team_color: vec3<f32>,
    @location(2) base_color: vec3<f32>,
    @location(3) world_pos: vec3<f32>,
    @location(4) view_depth: f32,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) inst_position: vec3<f32>,
    @location(4) heading: f32,
    @location(5) team_color: vec3<f32>,
) -> VertexOutput {
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
    let clip_pos = uniforms.view_proj * vec4<f32>(world_pos, 1.0);
    var out: VertexOutput;
    out.pos = clip_pos;
    out.normal = rotated_normal;
    out.team_color = team_color;
    out.base_color = color;
    out.world_pos = world_pos;
    out.view_depth = clip_pos.w;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(0.5, 0.8, 0.3));
    let n = normalize(in.normal);
    let ndl = max(dot(n, light_dir), 0.0);
    let shadow = shadow_factor(in.world_pos, in.view_depth);
    let ambient = 0.25;
    let diffuse = ndl * 0.75 * shadow;
    let lighting = ambient + diffuse;
    let color = in.base_color * 0.3 + in.team_color * 0.7;
    return vec4<f32>(color * lighting, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// Mesh storage
// ---------------------------------------------------------------------------

struct MeshData {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

// ---------------------------------------------------------------------------
// UnitRenderer
// ---------------------------------------------------------------------------

/// Manages GPU resources for instanced unit rendering with multiple meshes.
pub struct UnitRenderer {
    pipeline: wgpu::RenderPipeline,
    /// Meshes indexed by mesh_id. 0 = placeholder.
    meshes: BTreeMap<u32, MeshData>,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    /// Per-mesh instance ranges for the current frame.
    draw_groups: Vec<(u32, u32, u32)>, // (mesh_id, instance_start, instance_count)
}

impl UnitRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shadow_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unit_shader"),
            source: wgpu::ShaderSource::Wgsl(UNIT_SHADER.into()),
        });

        // Placeholder mesh (id=0)
        let (vertices, indices) = generate_unit_mesh();
        let mut meshes = BTreeMap::new();
        meshes.insert(
            0,
            MeshData {
                vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("unit_mesh_0_vb"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("unit_mesh_0_ib"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: indices.len() as u32,
            },
        );

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unit_instance_buffer"),
            size: 256,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("unit_pipeline_layout"),
            bind_group_layouts: &[camera_bind_group_layout, shadow_bind_group_layout],
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
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
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
            meshes,
            instance_buffer,
            instance_count: 0,
            draw_groups: Vec::new(),
        }
    }

    /// Register a mesh for a given mesh_id. Overwrites if already present.
    pub fn register_mesh(
        &mut self,
        device: &wgpu::Device,
        mesh_id: u32,
        vertices: &[UnitVertex],
        indices: &[u16],
    ) {
        self.meshes.insert(
            mesh_id,
            MeshData {
                vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("unit_mesh_{}_vb", mesh_id)),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("unit_mesh_{}_ib", mesh_id)),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: indices.len() as u32,
            },
        );
    }

    /// Replace the placeholder mesh (id=0). Backwards-compatible with old API.
    pub fn set_mesh(&mut self, device: &wgpu::Device, vertices: &[UnitVertex], indices: &[u16]) {
        self.register_mesh(device, 0, vertices, indices);
    }

    /// Upload instance data and compute draw groups (sorted by mesh_id).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[UnitInstance],
    ) {
        self.instance_count = instances.len() as u32;
        self.draw_groups.clear();

        if instances.is_empty() {
            return;
        }

        // Sort instances by mesh_id for grouped drawing.
        let mut sorted: Vec<UnitInstance> = instances.to_vec();
        sorted.sort_by_key(|i| i.mesh_id);

        // Compute draw groups.
        let mut start = 0u32;
        let mut current_mesh = sorted[0].mesh_id;
        for (i, inst) in sorted.iter().enumerate() {
            if inst.mesh_id != current_mesh {
                let count = i as u32 - start;
                if count > 0 {
                    // Fall back to placeholder (0) if mesh_id not registered.
                    let mid = if self.meshes.contains_key(&current_mesh) {
                        current_mesh
                    } else {
                        0
                    };
                    self.draw_groups.push((mid, start, count));
                }
                start = i as u32;
                current_mesh = inst.mesh_id;
            }
        }
        // Last group.
        let count = sorted.len() as u32 - start;
        if count > 0 {
            let mid = if self.meshes.contains_key(&current_mesh) {
                current_mesh
            } else {
                0
            };
            self.draw_groups.push((mid, start, count));
        }

        // Upload sorted instances.
        let data = bytemuck::cast_slice(&sorted);
        let required = data.len() as u64;
        if required > self.instance_buffer.size() {
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("unit_instance_buffer"),
                contents: data,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.instance_buffer, 0, data);
        }
    }

    /// Record draw commands — one draw call per mesh group.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));

        for &(mesh_id, inst_start, inst_count) in &self.draw_groups {
            if let Some(mesh) = self.meshes.get(&mesh_id) {
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..mesh.index_count, 0, inst_start..inst_start + inst_count);
            }
        }
    }

    /// Record draw commands into a shadow (depth-only) render pass.
    ///
    /// The caller must have already set the shadow pipeline and bind group 0
    /// (light VP). This method only sets vertex/index buffers and issues draws.
    pub fn render_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));

        for &(mesh_id, inst_start, inst_count) in &self.draw_groups {
            if let Some(mesh) = self.meshes.get(&mesh_id) {
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..mesh.index_count, 0, inst_start..inst_start + inst_count);
            }
        }
    }

    /// Number of registered meshes.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_instance_size() {
        // 3 position + 1 heading + 3 team_color + 1 mesh_id = 8 * 4 = 32 bytes
        assert_eq!(std::mem::size_of::<UnitInstance>(), 32);
    }

    #[test]
    fn unit_instance_is_pod() {
        let inst = UnitInstance::zeroed();
        assert_eq!(inst.heading, 0.0);
        assert_eq!(inst.mesh_id, 0);
    }
}
