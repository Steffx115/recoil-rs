use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

use crate::camera::Camera;
use crate::gpu::GpuContext;
use crate::terrain::TerrainResources;

/// Top-level renderer that owns GPU state and all render sub-systems.
pub struct Renderer {
    pub gpu: GpuContext,
    pub camera: Camera,
    terrain: TerrainResources,
}

impl Renderer {
    /// Create a new renderer for the given window.
    pub async fn new(window: Arc<Window>) -> Result<Self> {
        let gpu = GpuContext::new(window).await?;

        let camera = Camera {
            aspect: gpu.config.width as f32 / gpu.config.height as f32,
            ..Camera::default()
        };

        let terrain =
            TerrainResources::new(&gpu, &camera).context("failed to create terrain resources")?;

        Ok(Self {
            gpu,
            camera,
            terrain,
        })
    }

    /// Render one frame: update camera uniform, draw terrain, present.
    pub fn render(&mut self) -> Result<()> {
        // Upload latest camera matrix.
        self.terrain.update_camera(&self.gpu.queue, &self.camera);

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

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain_pass"),
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

            pass.set_pipeline(&self.terrain.pipeline);
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.terrain.vertex_buffer.slice(..));
            pass.set_index_buffer(
                self.terrain.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.terrain.index_count, 0, 0..1);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
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

    /// Access terrain resources (e.g. for custom draw calls).
    pub fn terrain(&self) -> &TerrainResources {
        &self.terrain
    }
}
