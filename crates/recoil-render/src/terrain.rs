use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::gpu::GpuContext;

// ---------------------------------------------------------------------------
// Vertex type
// ---------------------------------------------------------------------------

/// Terrain vertex: position + normal + texture coordinates.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl TerrainVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TerrainVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            // position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            // normal
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            // uv
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x2,
            },
        ],
    };
}

// ---------------------------------------------------------------------------
// WGSL shaders (inline)
// ---------------------------------------------------------------------------

const TERRAIN_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_y: f32,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.pos = uniforms.view_proj * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.uv = uv;
    out.world_y = position.y;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Height-based coloring
    let h = clamp(in.world_y / 40.0, 0.0, 1.0);
    let grass = vec3<f32>(0.15, 0.35, 0.1);
    let rock = vec3<f32>(0.35, 0.3, 0.25);
    let snow = vec3<f32>(0.7, 0.7, 0.7);
    var base_color = mix(grass, rock, smoothstep(0.2, 0.5, h));
    base_color = mix(base_color, snow, smoothstep(0.7, 0.95, h));

    // Slope coloring (steeper = more rock)
    let slope = 1.0 - max(dot(normalize(in.normal), vec3<f32>(0.0, 1.0, 0.0)), 0.0);
    base_color = mix(base_color, rock, smoothstep(0.3, 0.6, slope));

    // Subtle grid lines
    let grid = abs(fract(in.uv * 16.0) - 0.5);
    let line = 1.0 - smoothstep(0.45, 0.5, min(grid.x, grid.y));
    base_color = mix(base_color, base_color * 0.8, line * 0.15);

    // Directional lighting
    let light_dir = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let n = normalize(in.normal);
    let ndl = max(dot(n, light_dir), 0.0);
    let lighting = 0.3 + ndl * 0.7;

    return vec4<f32>(base_color * lighting, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// Mesh generation
// ---------------------------------------------------------------------------

/// Generate a flat terrain grid mesh.
///
/// * `cells` -- number of cells along each axis (e.g. 64 produces a 64x64 grid).
/// * `cell_size` -- world-space size of each cell.
///
/// Returns `(vertices, indices)`.
pub fn generate_grid(cells: u32, cell_size: f32) -> (Vec<TerrainVertex>, Vec<u32>) {
    let verts_per_side = cells + 1;
    let mut vertices = Vec::with_capacity((verts_per_side * verts_per_side) as usize);

    for z in 0..verts_per_side {
        for x in 0..verts_per_side {
            let px = x as f32 * cell_size;
            let pz = z as f32 * cell_size;
            let u = x as f32 / cells as f32;
            let v = z as f32 / cells as f32;
            vertices.push(TerrainVertex {
                position: [px, 0.0, pz],
                normal: [0.0, 1.0, 0.0],
                uv: [u, v],
            });
        }
    }

    let mut indices = Vec::with_capacity((cells * cells * 6) as usize);
    for z in 0..cells {
        for x in 0..cells {
            let tl = z * verts_per_side + x;
            let tr = tl + 1;
            let bl = (z + 1) * verts_per_side + x;
            let br = bl + 1;
            // Two triangles per cell (counter-clockwise winding when viewed from above).
            indices.push(tl);
            indices.push(bl);
            indices.push(tr);
            indices.push(tr);
            indices.push(bl);
            indices.push(br);
        }
    }

    (vertices, indices)
}

/// Generate a terrain grid from a heightmap.
///
/// * `width`, `height` — grid dimensions in cells.
/// * `cell_size` — world-space size per cell.
/// * `heights` — row-major height values (`(width+1) * (height+1)` entries), in world Y units.
pub fn generate_heightmap_grid(
    width: u32,
    height: u32,
    cell_size: f32,
    heights: &[f32],
) -> (Vec<TerrainVertex>, Vec<u32>) {
    let vw = width + 1;
    let vh = height + 1;
    let mut vertices = Vec::with_capacity((vw * vh) as usize);

    let get_h = |x: u32, z: u32| -> f32 {
        let idx = (z * vw + x) as usize;
        heights.get(idx).copied().unwrap_or(0.0)
    };

    for z in 0..vh {
        for x in 0..vw {
            let px = x as f32 * cell_size;
            let pz = z as f32 * cell_size;
            let py = get_h(x, z);
            let u = x as f32 / width as f32;
            let v = z as f32 / height as f32;

            // Compute normal from height differences (central difference).
            let hx_l = if x > 0 { get_h(x - 1, z) } else { py };
            let hx_r = if x < width { get_h(x + 1, z) } else { py };
            let hz_u = if z > 0 { get_h(x, z - 1) } else { py };
            let hz_d = if z < height { get_h(x, z + 1) } else { py };
            let dx = (hx_r - hx_l) / (2.0 * cell_size);
            let dz = (hz_d - hz_u) / (2.0 * cell_size);
            let len = (dx * dx + 1.0 + dz * dz).sqrt();
            let normal = [-dx / len, 1.0 / len, -dz / len];

            vertices.push(TerrainVertex {
                position: [px, py, pz],
                normal,
                uv: [u, v],
            });
        }
    }

    let mut indices = Vec::with_capacity((width * height * 6) as usize);
    for z in 0..height {
        for x in 0..width {
            let tl = z * vw + x;
            let tr = tl + 1;
            let bl = (z + 1) * vw + x;
            let br = bl + 1;
            indices.push(tl);
            indices.push(bl);
            indices.push(tr);
            indices.push(tr);
            indices.push(bl);
            indices.push(br);
        }
    }

    (vertices, indices)
}

// ---------------------------------------------------------------------------
// Terrain renderer resources
// ---------------------------------------------------------------------------

/// GPU resources for terrain rendering.
pub struct TerrainResources {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub camera_uniform: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl TerrainResources {
    /// Create terrain GPU resources (pipeline, buffers, bind groups).
    pub fn new(gpu: &GpuContext, camera: &Camera) -> Result<Self> {
        let device = &gpu.device;

        // --- Shader module ---
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain_shader"),
            source: wgpu::ShaderSource::Wgsl(TERRAIN_SHADER.into()),
        });

        // --- Mesh ---
        let (vertices, indices) = generate_grid(128, 8.0); // 128*8=1024 world units (matches Small Duel map)
        let index_count = indices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // --- Camera uniform ---
        let vp = camera.view_projection();
        let camera_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform"),
            contents: bytemuck::cast_slice(&vp),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // --- Bind group ---
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform.as_entire_binding(),
            }],
        });

        // --- Pipeline ---
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[TerrainVertex::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.config.format,
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

        Ok(Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count,
            camera_uniform,
            camera_bind_group,
            bind_group_layout,
        })
    }

    /// Upload a new view-projection matrix to the camera uniform buffer.
    pub fn update_camera(&self, queue: &wgpu::Queue, camera: &Camera) {
        let vp = camera.view_projection();
        queue.write_buffer(&self.camera_uniform, 0, bytemuck::cast_slice(&vp));
    }

    /// Replace the terrain mesh (e.g. from heightmap data).
    pub fn set_mesh(&mut self, device: &wgpu::Device, vertices: &[TerrainVertex], indices: &[u32]) {
        self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_vertex_buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_index_buffer"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.index_count = indices.len() as u32;
    }

    /// Access the bind group layout (needed if other passes want the same layout).
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_dimensions() {
        let (verts, indices) = generate_grid(4, 1.0);
        // 5x5 = 25 vertices for a 4x4 grid
        assert_eq!(verts.len(), 25);
        // 4*4*6 = 96 indices
        assert_eq!(indices.len(), 96);
    }

    #[test]
    fn grid_normals_point_up() {
        let (verts, _) = generate_grid(2, 1.0);
        for v in &verts {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
        }
    }

    #[test]
    fn grid_uvs_in_unit_range() {
        let (verts, _) = generate_grid(8, 2.0);
        for v in &verts {
            assert!(v.uv[0] >= 0.0 && v.uv[0] <= 1.0);
            assert!(v.uv[1] >= 0.0 && v.uv[1] <= 1.0);
        }
    }
}
