//! Cascaded shadow mapping for directional light.
//!
//! Provides a 2-cascade shadow map with PCF soft filtering.
//! Shadow maps are 2048x2048 `Depth32Float` textures stored as a 2D array.

use bytemuck::{Pod, Zeroable};

use crate::camera::Camera;
use crate::gpu::GpuContext;

/// Resolution of each shadow map cascade.
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// Number of cascades.
pub const CASCADE_COUNT: u32 = 2;

// ---------------------------------------------------------------------------
// GPU uniform
// ---------------------------------------------------------------------------

/// Shadow uniform data uploaded to the GPU.
///
/// Contains the light view-projection matrices for each cascade and the
/// cascade split distances (in view-space depth).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShadowUniforms {
    /// Light VP matrix for cascade 0 (near).
    pub light_vp_0: [[f32; 4]; 4],
    /// Light VP matrix for cascade 1 (far).
    pub light_vp_1: [[f32; 4]; 4],
    /// Cascade splits: [near, split, far, pad].
    pub cascade_splits: [f32; 4],
}

// ---------------------------------------------------------------------------
// WGSL snippet for sampling (shared by terrain & unit shaders)
// ---------------------------------------------------------------------------

/// WGSL code for shadow sampling, intended to be prepended to shaders that
/// need shadow support. Declares group(1) bindings and a `shadow_factor` fn.
pub const SHADOW_WGSL: &str = r#"
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
"#;

// ---------------------------------------------------------------------------
// Depth-only shader for shadow pass
// ---------------------------------------------------------------------------

/// Minimal depth-only vertex shader for terrain shadow pass.
const SHADOW_TERRAIN_SHADER: &str = r#"
struct LightUniforms {
    light_vp: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> light_uniforms: LightUniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return light_uniforms.light_vp * vec4<f32>(position, 1.0);
}
"#;

/// Minimal depth-only vertex shader for unit shadow pass (instanced).
const SHADOW_UNIT_SHADER: &str = r#"
struct LightUniforms {
    light_vp: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> light_uniforms: LightUniforms;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) _normal: vec3<f32>,
    @location(2) _color: vec3<f32>,
    @location(3) inst_position: vec3<f32>,
    @location(4) heading: f32,
    @location(5) _team_color: vec3<f32>,
) -> @builtin(position) vec4<f32> {
    let c = cos(heading);
    let s = sin(heading);
    let rotated = vec3<f32>(
        position.x * c + position.z * s,
        position.y,
        -position.x * s + position.z * c,
    );
    let world_pos = rotated + inst_position;
    return light_uniforms.light_vp * vec4<f32>(world_pos, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// ShadowResources
// ---------------------------------------------------------------------------

/// Owns all GPU resources for cascaded shadow mapping.
pub struct ShadowResources {
    /// Shadow map texture (2D array, one layer per cascade).
    pub shadow_texture: wgpu::Texture,
    /// Per-layer views used as depth attachments during shadow pass.
    pub cascade_views: Vec<wgpu::TextureView>,
    /// Full-array view used for sampling in the main pass.
    pub shadow_view: wgpu::TextureView,
    /// Comparison sampler for PCF filtering.
    pub shadow_sampler: wgpu::Sampler,
    /// Shadow uniform buffer.
    pub uniform_buffer: wgpu::Buffer,
    /// Bind group layout for group 1 (used by terrain + unit pipelines).
    shadow_bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group for group 1.
    shadow_bind_group: wgpu::BindGroup,

    /// Light VP bind group layout (group 0 in shadow pass).
    #[allow(dead_code)]
    light_bind_group_layout: wgpu::BindGroupLayout,
    /// Per-cascade light VP buffers.
    light_vp_buffers: Vec<wgpu::Buffer>,
    /// Per-cascade light VP bind groups.
    light_vp_bind_groups: Vec<wgpu::BindGroup>,

    /// Depth-only terrain pipeline.
    pub terrain_shadow_pipeline: wgpu::RenderPipeline,
    /// Depth-only unit pipeline.
    pub unit_shadow_pipeline: wgpu::RenderPipeline,

    /// Current light direction (normalised).
    pub light_dir: [f32; 3],
}

impl ShadowResources {
    /// Create shadow mapping resources.
    pub fn new(device: &wgpu::Device) -> Self {
        // --- Shadow map texture (2D array) ---
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_map"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: CASCADE_COUNT,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GpuContext::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // Per-layer views for rendering into.
        let cascade_views: Vec<wgpu::TextureView> = (0..CASCADE_COUNT)
            .map(|i| {
                shadow_texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("shadow_cascade_{i}_view")),
                    format: Some(GpuContext::DEPTH_FORMAT),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    aspect: wgpu::TextureAspect::DepthOnly,
                    base_mip_level: 0,
                    mip_level_count: None,
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        // Full array view for sampling.
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow_map_array_view"),
            format: Some(GpuContext::DEPTH_FORMAT),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::DepthOnly,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(CASCADE_COUNT),
            ..Default::default()
        });

        // --- Comparison sampler ---
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // --- Shadow uniform buffer ---
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_uniform_buffer"),
            size: std::mem::size_of::<ShadowUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Bind group layout (group 1 in main pass) ---
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow_bind_group_layout"),
                entries: &[
                    // binding 0: shadow map texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 1: comparison sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                    // binding 2: shadow uniforms
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_bind_group"),
            layout: &shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // --- Light VP bind group layout (group 0 in shadow pass) ---
        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("light_vp_bind_group_layout"),
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

        // Per-cascade buffers and bind groups.
        let mut light_vp_buffers = Vec::with_capacity(CASCADE_COUNT as usize);
        let mut light_vp_bind_groups = Vec::with_capacity(CASCADE_COUNT as usize);
        for i in 0..CASCADE_COUNT {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("light_vp_buffer_{i}")),
                size: 64, // mat4x4<f32>
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("light_vp_bind_group_{i}")),
                layout: &light_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            });
            light_vp_buffers.push(buf);
            light_vp_bind_groups.push(bg);
        }

        // --- Shadow pass pipeline layout ---
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow_pipeline_layout"),
                bind_group_layouts: &[&light_bind_group_layout],
                push_constant_ranges: &[],
            });

        // --- Terrain shadow pipeline ---
        let terrain_shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow_terrain_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_TERRAIN_SHADER.into()),
        });

        // Terrain vertex: only position is needed (location 0), but we include the
        // full vertex layout so wgpu can skip the normal/uv attributes.
        let terrain_shadow_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("terrain_shadow_pipeline"),
                layout: Some(&shadow_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &terrain_shadow_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[crate::terrain::TerrainVertex::LAYOUT],
                    compilation_options: Default::default(),
                },
                fragment: None,
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
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        // --- Unit shadow pipeline ---
        let unit_shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow_unit_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_UNIT_SHADER.into()),
        });

        let unit_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("unit_shadow_pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &unit_shadow_shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    crate::unit_mesh::UnitVertex::LAYOUT,
                    crate::unit_renderer::UnitInstance::LAYOUT,
                ],
                compilation_options: Default::default(),
            },
            fragment: None,
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
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let light_dir = normalize3([0.4, 0.8, 0.3]);

        Self {
            shadow_texture,
            cascade_views,
            shadow_view,
            shadow_sampler,
            uniform_buffer,
            shadow_bind_group_layout,
            shadow_bind_group,
            light_bind_group_layout,
            light_vp_buffers,
            light_vp_bind_groups,
            terrain_shadow_pipeline,
            unit_shadow_pipeline,
            light_dir,
        }
    }

    /// The bind group layout for shadow data (group 1 in main pass).
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.shadow_bind_group_layout
    }

    /// The bind group for shadow data (group 1 in main pass).
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.shadow_bind_group
    }

    /// The per-cascade light VP bind group (group 0 in shadow pass).
    pub fn light_vp_bind_group(&self, cascade: usize) -> &wgpu::BindGroup {
        &self.light_vp_bind_groups[cascade]
    }

    /// Set the light direction (will be normalised).
    pub fn set_light_direction(&mut self, dir: [f32; 3]) {
        self.light_dir = normalize3(dir);
    }

    /// Recompute cascade matrices and upload to the GPU.
    pub fn update(&self, queue: &wgpu::Queue, camera: &Camera) {
        let uniforms = compute_cascade_matrices(camera, self.light_dir);

        // Upload combined uniform.
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Upload per-cascade light VP matrices.
        queue.write_buffer(
            &self.light_vp_buffers[0],
            0,
            bytemuck::cast_slice(&uniforms.light_vp_0),
        );
        queue.write_buffer(
            &self.light_vp_buffers[1],
            0,
            bytemuck::cast_slice(&uniforms.light_vp_1),
        );
    }
}

// ---------------------------------------------------------------------------
// Cascade computation
// ---------------------------------------------------------------------------

/// Cascade split distances (view-space depth).
const CASCADE_NEAR: f32 = 0.0;
const CASCADE_SPLIT: f32 = 200.0;
const CASCADE_FAR: f32 = 800.0;

/// Compute cascade light view-projection matrices by fitting an orthographic
/// projection around the camera frustum slice for each cascade.
pub fn compute_cascade_matrices(camera: &Camera, light_dir: [f32; 3]) -> ShadowUniforms {
    let view = camera.view_matrix();
    let inv_view = mat4_inverse(view);
    let light_dir = normalize3(light_dir);

    let cam_near = camera.near;
    let cam_far = camera.far;

    // Map cascade boundaries from world-unit ranges to camera near/far fractions.
    let splits = [
        cam_near.max(CASCADE_NEAR),
        cam_far.min(CASCADE_SPLIT).max(cam_near),
        cam_far.min(CASCADE_FAR),
    ];

    let light_vp_0 = cascade_matrix(camera, &inv_view, light_dir, splits[0], splits[1]);
    let light_vp_1 = cascade_matrix(camera, &inv_view, light_dir, splits[1], splits[2]);

    ShadowUniforms {
        light_vp_0,
        light_vp_1,
        cascade_splits: [splits[0], splits[1], splits[2], 0.0],
    }
}

/// Compute a tight ortho light VP matrix for a single cascade slice.
fn cascade_matrix(
    camera: &Camera,
    inv_view: &[[f32; 4]; 4],
    light_dir: [f32; 3],
    near: f32,
    far: f32,
) -> [[f32; 4]; 4] {
    // Get frustum corners in NDC then transform to world space.
    let corners = frustum_corners_world(camera, inv_view, near, far);

    // Build light view matrix (looking along -light_dir).
    let light_view = light_look_at(light_dir);

    // Transform corners to light space and compute AABB.
    let mut min_ls = [f32::MAX; 3];
    let mut max_ls = [f32::MIN; 3];
    for c in &corners {
        let ls = mat4_transform_point(&light_view, *c);
        for i in 0..3 {
            min_ls[i] = min_ls[i].min(ls[i]);
            max_ls[i] = max_ls[i].max(ls[i]);
        }
    }

    // Pad the Z range slightly to avoid clipping.
    let z_pad = (max_ls[2] - min_ls[2]) * 0.1 + 10.0;
    min_ls[2] -= z_pad;
    max_ls[2] += z_pad;

    // Build orthographic projection.
    let ortho = ortho_projection(
        min_ls[0], max_ls[0], min_ls[1], max_ls[1], min_ls[2], max_ls[2],
    );

    mat4_mul(ortho, light_view)
}

/// Compute the 8 frustum corners in world space for a sub-frustum
/// between `near` and `far` (in view-space depth).
fn frustum_corners_world(
    camera: &Camera,
    inv_view: &[[f32; 4]; 4],
    near: f32,
    far: f32,
) -> [[f32; 3]; 8] {
    let proj = camera.projection_matrix();
    let inv_proj = mat4_inverse(proj);

    // NDC corners at z=0 (near) and z=1 (far).
    let ndc_corners = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

    let mut corners = [[0.0f32; 3]; 8];

    for (i, &[x, y]) in ndc_corners.iter().enumerate() {
        // Unproject at ndc z=0 and z=1 to get view-space points.
        let near_ndc = [x, y, 0.0, 1.0];
        let far_ndc = [x, y, 1.0, 1.0];

        let near_vs = mat4_transform_point4(&inv_proj, near_ndc);
        let far_vs = mat4_transform_point4(&inv_proj, far_ndc);

        // Interpolate along the view-space ray to the desired near/far range.
        let cam_near = camera.near;
        let cam_far = camera.far;
        let t_near = (near - cam_near) / (cam_far - cam_near);
        let t_far = (far - cam_near) / (cam_far - cam_near);

        let p_near = lerp3(near_vs, far_vs, t_near);
        let p_far = lerp3(near_vs, far_vs, t_far);

        // Transform to world space.
        corners[i] = mat4_transform_point(inv_view, p_near);
        corners[i + 4] = mat4_transform_point(inv_view, p_far);
    }

    corners
}

/// Build a view matrix looking along `-light_dir` centred at the origin.
fn light_look_at(light_dir: [f32; 3]) -> [[f32; 4]; 4] {
    let dir = normalize3(light_dir);
    // "eye" at some point along the light direction.
    let eye = [0.0, 0.0, 0.0];
    let target = [-dir[0], -dir[1], -dir[2]];
    // Choose an up vector that isn't parallel to dir.
    let up = if dir[1].abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    look_at(eye, target, up)
}

// ---------------------------------------------------------------------------
// Math helpers (render-side f32, same conventions as camera.rs)
// ---------------------------------------------------------------------------

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 {
        return [0.0; 3];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Right-handed look-at (column-major).
fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize3(sub3(target, eye));
    let s = normalize3(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot3(s, eye), -dot3(u, eye), dot3(f, eye), 1.0],
    ]
}

/// Column-major mat4 multiply: result = a * b.
fn mat4_mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

/// Transform a 3D point by a column-major mat4 (w=1, perspective divide).
fn mat4_transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let x = m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0];
    let y = m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1];
    let z = m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2];
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
    if w.abs() < 1e-10 {
        return [x, y, z];
    }
    [x / w, y / w, z / w]
}

/// Transform a 4D point by a column-major mat4 with perspective divide, returning xyz.
fn mat4_transform_point4(m: &[[f32; 4]; 4], p: [f32; 4]) -> [f32; 3] {
    let x = m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0] * p[3];
    let y = m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1] * p[3];
    let z = m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2] * p[3];
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3] * p[3];
    if w.abs() < 1e-10 {
        return [x, y, z];
    }
    [x / w, y / w, z / w]
}

/// Right-handed orthographic projection with depth [0, 1] (wgpu convention).
fn ortho_projection(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> [[f32; 4]; 4] {
    let rml = right - left;
    let tmb = top - bottom;
    let fmn = far - near;
    [
        [2.0 / rml, 0.0, 0.0, 0.0],
        [0.0, 2.0 / tmb, 0.0, 0.0],
        [0.0, 0.0, -1.0 / fmn, 0.0],
        [
            -(right + left) / rml,
            -(top + bottom) / tmb,
            -near / fmn,
            1.0,
        ],
    ]
}

/// Invert a 4x4 column-major matrix. Uses cofactor expansion.
fn mat4_inverse(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Flatten to row-major for easier indexing of elements.
    // m[col][row] in column-major. We index as e(row, col).
    let e = |r: usize, c: usize| -> f32 { m[c][r] };

    let mut inv = [[0.0f32; 4]; 4];

    let s0 = e(0, 0) * e(1, 1) - e(1, 0) * e(0, 1);
    let s1 = e(0, 0) * e(1, 2) - e(1, 0) * e(0, 2);
    let s2 = e(0, 0) * e(1, 3) - e(1, 0) * e(0, 3);
    let s3 = e(0, 1) * e(1, 2) - e(1, 1) * e(0, 2);
    let s4 = e(0, 1) * e(1, 3) - e(1, 1) * e(0, 3);
    let s5 = e(0, 2) * e(1, 3) - e(1, 2) * e(0, 3);

    let c5 = e(2, 2) * e(3, 3) - e(3, 2) * e(2, 3);
    let c4 = e(2, 1) * e(3, 3) - e(3, 1) * e(2, 3);
    let c3 = e(2, 1) * e(3, 2) - e(3, 1) * e(2, 2);
    let c2 = e(2, 0) * e(3, 3) - e(3, 0) * e(2, 3);
    let c1 = e(2, 0) * e(3, 2) - e(3, 0) * e(2, 2);
    let c0 = e(2, 0) * e(3, 1) - e(3, 0) * e(2, 1);

    let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    if det.abs() < 1e-20 {
        return [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
    }
    let inv_det = 1.0 / det;

    // Row 0
    inv[0][0] = (e(1, 1) * c5 - e(1, 2) * c4 + e(1, 3) * c3) * inv_det;
    inv[1][0] = (-e(0, 1) * c5 + e(0, 2) * c4 - e(0, 3) * c3) * inv_det;
    inv[2][0] = (e(3, 1) * s5 - e(3, 2) * s4 + e(3, 3) * s3) * inv_det;
    inv[3][0] = (-e(2, 1) * s5 + e(2, 2) * s4 - e(2, 3) * s3) * inv_det;
    // Row 1
    inv[0][1] = (-e(1, 0) * c5 + e(1, 2) * c2 - e(1, 3) * c1) * inv_det;
    inv[1][1] = (e(0, 0) * c5 - e(0, 2) * c2 + e(0, 3) * c1) * inv_det;
    inv[2][1] = (-e(3, 0) * s5 + e(3, 2) * s2 - e(3, 3) * s1) * inv_det;
    inv[3][1] = (e(2, 0) * s5 - e(2, 2) * s2 + e(2, 3) * s1) * inv_det;
    // Row 2
    inv[0][2] = (e(1, 0) * c4 - e(1, 1) * c2 + e(1, 3) * c0) * inv_det;
    inv[1][2] = (-e(0, 0) * c4 + e(0, 1) * c2 - e(0, 3) * c0) * inv_det;
    inv[2][2] = (e(3, 0) * s4 - e(3, 1) * s2 + e(3, 3) * s0) * inv_det;
    inv[3][2] = (-e(2, 0) * s4 + e(2, 1) * s2 - e(2, 3) * s0) * inv_det;
    // Row 3
    inv[0][3] = (-e(1, 0) * c3 + e(1, 1) * c1 - e(1, 2) * c0) * inv_det;
    inv[1][3] = (e(0, 0) * c3 - e(0, 1) * c1 + e(0, 2) * c0) * inv_det;
    inv[2][3] = (-e(3, 0) * s3 + e(3, 1) * s1 - e(3, 2) * s0) * inv_det;
    inv[3][3] = (e(2, 0) * s3 - e(2, 1) * s1 + e(2, 2) * s0) * inv_det;

    inv
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_uniforms_size() {
        // 2 * 64 bytes (mat4x4) + 16 bytes (vec4) = 144 bytes
        assert_eq!(std::mem::size_of::<ShadowUniforms>(), 144);
    }

    #[test]
    fn shadow_uniforms_is_pod() {
        let u = ShadowUniforms::zeroed();
        assert_eq!(u.cascade_splits, [0.0; 4]);
    }

    #[test]
    fn cascade_matrices_produce_valid_output() {
        let camera = Camera::default();
        let light_dir = [0.4, 0.8, 0.3];
        let uniforms = compute_cascade_matrices(&camera, light_dir);

        // Splits should match expected values (clamped to camera range).
        assert!(uniforms.cascade_splits[0] >= 0.0);
        assert!(uniforms.cascade_splits[1] > uniforms.cascade_splits[0]);
        assert!(uniforms.cascade_splits[2] > uniforms.cascade_splits[1]);

        // Matrices should not be zero.
        let zero_mat = [[0.0f32; 4]; 4];
        assert_ne!(uniforms.light_vp_0, zero_mat);
        assert_ne!(uniforms.light_vp_1, zero_mat);
    }

    #[test]
    fn cascade_splits_clamp_to_camera_range() {
        let mut camera = Camera::default();
        camera.near = 1.0;
        camera.far = 100.0; // Far < CASCADE_SPLIT

        let uniforms = compute_cascade_matrices(&camera, [0.4, 0.8, 0.3]);

        // With far=100, split should be clamped to 100.
        assert!((uniforms.cascade_splits[1] - 100.0).abs() < 1e-5);
        assert!((uniforms.cascade_splits[2] - 100.0).abs() < 1e-5);
    }

    #[test]
    fn mat4_inverse_identity() {
        let id = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let inv = mat4_inverse(id);
        for c in 0..4 {
            for r in 0..4 {
                let expected = if c == r { 1.0 } else { 0.0 };
                assert!(
                    (inv[c][r] - expected).abs() < 1e-5,
                    "mismatch at [{c}][{r}]: {} vs {expected}",
                    inv[c][r]
                );
            }
        }
    }

    #[test]
    fn mat4_inverse_roundtrip() {
        let camera = Camera::default();
        let view = camera.view_matrix();
        let inv = mat4_inverse(view);
        let product = mat4_mul(view, inv);
        for c in 0..4 {
            for r in 0..4 {
                let expected = if c == r { 1.0 } else { 0.0 };
                assert!(
                    (product[c][r] - expected).abs() < 1e-4,
                    "mismatch at [{c}][{r}]: {} vs {expected}",
                    product[c][r]
                );
            }
        }
    }

    #[test]
    fn ortho_projection_maps_center_to_origin() {
        let p = ortho_projection(-10.0, 10.0, -10.0, 10.0, 0.0, 100.0);
        // Center point (0, 0, -50) should map near the center of NDC.
        let pt = mat4_transform_point(&p, [0.0, 0.0, -50.0]);
        assert!(pt[0].abs() < 1e-5, "x: {}", pt[0]);
        assert!(pt[1].abs() < 1e-5, "y: {}", pt[1]);
    }

    #[test]
    fn normalize3_unit_length() {
        let v = normalize3([3.0, 4.0, 0.0]);
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }

    #[test]
    fn light_look_at_produces_valid_view() {
        let view = light_look_at([0.4, 0.8, 0.3]);
        // The view matrix should be orthonormal (columns have unit length).
        for col in 0..3 {
            let len = (view[col][0].powi(2) + view[col][1].powi(2) + view[col][2].powi(2)).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "column {col} length = {len}");
        }
    }
}
