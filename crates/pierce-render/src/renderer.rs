use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

use crate::camera::Camera;
use crate::gpu::GpuContext;
use crate::projectile_renderer::{ProjectileInstance, ProjectileRenderer};
use crate::shadow::ShadowResources;
use crate::terrain::TerrainResources;
use crate::unit_renderer::{UnitInstance, UnitRenderer};

/// Top-level renderer that owns GPU state and all render sub-systems.
pub struct Renderer {
    pub gpu: GpuContext,
    pub camera: Camera,
    shadow: ShadowResources,
    terrain: TerrainResources,
    unit_renderer: UnitRenderer,
    projectile_renderer: ProjectileRenderer,
}

impl Renderer {
    /// Create a new renderer for the given window.
    pub async fn new(window: Arc<Window>) -> Result<Self> {
        Self::with_map_size(window, 1024.0).await
    }

    pub async fn with_map_size(window: Arc<Window>, map_world_size: f32) -> Result<Self> {
        let gpu = GpuContext::new(window).await?;

        let camera = Camera {
            aspect: gpu.config.width as f32 / gpu.config.height as f32,
            ..Camera::default()
        };

        let shadow = ShadowResources::new(&gpu.device);

        let terrain = TerrainResources::with_map_size(&gpu, &camera, shadow.bind_group_layout(), map_world_size)
            .context("failed to create terrain resources")?;

        let unit_renderer = UnitRenderer::new(
            &gpu.device,
            gpu.config.format,
            terrain.bind_group_layout(),
            shadow.bind_group_layout(),
        );

        let projectile_renderer =
            ProjectileRenderer::new(&gpu.device, gpu.config.format, terrain.bind_group_layout());

        Ok(Self {
            gpu,
            camera,
            shadow,
            terrain,
            unit_renderer,
            projectile_renderer,
        })
    }

    /// Render one frame: update camera uniform, draw terrain, present.
    pub fn render(&mut self) -> Result<()> {
        let (output, _view) = self.render_no_present()?;
        output.present();
        Ok(())
    }

    /// Render the 3D scene but do NOT present. Returns the surface texture and
    /// view so the caller can add additional passes (e.g. egui overlay) before
    /// presenting.
    pub fn render_no_present(&mut self) -> Result<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        // Upload latest camera matrix.
        self.terrain.update_camera(&self.gpu.queue, &self.camera);

        // Update shadow cascade matrices.
        self.shadow.update(&self.gpu.queue, &self.camera);

        let output = self
            .gpu
            .surface
            .get_current_texture()
            .context("failed to acquire next swap-chain texture")?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_encoder"),
            });

        // --- Shadow pass: render depth from light's perspective for each cascade ---
        for cascade in 0..crate::shadow::CASCADE_COUNT as usize {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow.cascade_views[cascade],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Terrain shadow
            shadow_pass.set_pipeline(&self.shadow.terrain_shadow_pipeline);
            shadow_pass.set_bind_group(0, self.shadow.light_vp_bind_group(cascade), &[]);
            shadow_pass.set_vertex_buffer(0, self.terrain.vertex_buffer.slice(..));
            shadow_pass.set_index_buffer(
                self.terrain.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            shadow_pass.draw_indexed(0..self.terrain.index_count, 0, 0..1);

            // Unit shadow
            shadow_pass.set_pipeline(&self.shadow.unit_shadow_pipeline);
            shadow_pass.set_bind_group(0, self.shadow.light_vp_bind_group(cascade), &[]);
            self.unit_renderer.render_shadow(&mut shadow_pass);
        }

        // --- Main render pass ---
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.08,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Terrain
            pass.set_pipeline(&self.terrain.pipeline);
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            pass.set_bind_group(1, self.shadow.bind_group(), &[]);
            pass.set_vertex_buffer(0, self.terrain.vertex_buffer.slice(..));
            pass.set_index_buffer(
                self.terrain.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.terrain.index_count, 0, 0..1);

            // Units: reuse the same camera bind group (group 0) + shadow (group 1).
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            pass.set_bind_group(1, self.shadow.bind_group(), &[]);
            self.unit_renderer.render(&mut pass);

            // Projectiles / particles: draw after units (alpha-blended).
            // Projectiles don't use shadows, only bind group 0.
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            self.projectile_renderer.render(&mut pass);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));

        Ok((output, view))
    }

    /// Handle a window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        self.camera.aspect = width as f32 / height as f32;
    }

    /// Update the camera and upload the new VP matrix to the GPU.
    pub fn update_camera(&mut self, camera: &Camera) {
        self.camera = Camera {
            eye: camera.eye,
            target: camera.target,
            up: camera.up,
            fov_y: camera.fov_y,
            aspect: camera.aspect,
            near: camera.near,
            far: camera.far,
        };
        self.terrain.update_camera(&self.gpu.queue, &self.camera);
    }

    /// Replace the placeholder unit mesh (mesh_id=0).
    pub fn set_unit_mesh(&mut self, vertices: &[crate::unit_mesh::UnitVertex], indices: &[u16]) {
        self.unit_renderer
            .set_mesh(&self.gpu.device, vertices, indices);
    }

    /// Register a mesh for a specific mesh_id (e.g. a unit_type_id).
    pub fn register_unit_mesh(
        &mut self,
        mesh_id: u32,
        vertices: &[crate::unit_mesh::UnitVertex],
        indices: &[u16],
    ) {
        self.unit_renderer
            .register_mesh(&self.gpu.device, mesh_id, vertices, indices);
    }

    /// Upload unit instance data for the next frame.
    pub fn update_units(&mut self, instances: &[UnitInstance]) {
        self.unit_renderer
            .prepare(&self.gpu.device, &self.gpu.queue, instances);
    }

    /// Upload projectile/particle instance data for the next frame.
    pub fn update_projectiles(&mut self, instances: &[ProjectileInstance]) {
        self.projectile_renderer
            .prepare(&self.gpu.device, &self.gpu.queue, instances);
    }

    /// Replace the terrain mesh with heightmap data.
    pub fn set_terrain_mesh(
        &mut self,
        vertices: &[crate::terrain::TerrainVertex],
        indices: &[u32],
    ) {
        self.terrain.set_mesh(&self.gpu.device, vertices, indices);
    }

    /// Set the directional light direction for shadow casting.
    pub fn set_light_direction(&mut self, dir: [f32; 3]) {
        self.shadow.set_light_direction(dir);
    }

    /// Access terrain resources (e.g. for custom draw calls).
    pub fn terrain(&self) -> &TerrainResources {
        &self.terrain
    }
}
