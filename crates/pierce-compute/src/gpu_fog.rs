//! GPU implementation of fog-of-war computation.
//!
//! Uses a WGSL compute shader with integer-only arithmetic for determinism.

use std::collections::BTreeMap;

use pierce_sim::compute::{FogCompute, FogGridParams, FogUnitInput};

use crate::buffers::{GpuFogParams, GpuFogUnit, i64_to_pair};

/// GPU fog-of-war backend.
pub struct GpuFogCompute {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
}

impl GpuFogCompute {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader_src = include_str!("shaders/fog.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog compute shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fog bind group layout"),
                entries: &[
                    // 0: params uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 1: units storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 2: grid storage (read-write, atomic)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fog pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fog compute pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("fog_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog params"),
            size: std::mem::size_of::<GpuFogParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            params_buffer,
        }
    }
}

impl FogCompute for GpuFogCompute {
    fn compute_fog(
        &mut self,
        params: &FogGridParams,
        units: &[FogUnitInput],
        previous_grids: &BTreeMap<u8, Vec<u8>>,
    ) -> BTreeMap<u8, Vec<u8>> {
        let w = params.width;
        let h = params.height;
        let cell_count = (w as usize) * (h as usize);

        if units.is_empty() {
            // No units: just downgrade Visible -> Explored.
            let mut grids = BTreeMap::new();
            for &team in &params.teams {
                let grid = if let Some(prev) = previous_grids.get(&team) {
                    prev.iter()
                        .map(|&v| if v == 2 { 1 } else { v })
                        .collect()
                } else {
                    vec![0u8; cell_count]
                };
                grids.insert(team, grid);
            }
            return grids;
        }

        // Process each team separately (each team has its own grid).
        let mut result_grids = BTreeMap::new();

        for &team in &params.teams {
            let team_units: Vec<&FogUnitInput> =
                units.iter().filter(|u| u.team == team).collect();

            // Initialize grid: downgrade previous Visible -> Explored.
            let mut grid_data: Vec<u32> = if let Some(prev) = previous_grids.get(&team) {
                prev.iter()
                    .map(|&v| if v == 2 { 1u32 } else { v as u32 })
                    .collect()
            } else {
                vec![0u32; cell_count]
            };

            if team_units.is_empty() {
                result_grids.insert(
                    team,
                    grid_data.iter().map(|&v| v as u8).collect(),
                );
                continue;
            }

            // Upload unit data.
            let gpu_units: Vec<GpuFogUnit> = team_units
                .iter()
                .map(|u| GpuFogUnit::from_raw(u.pos_x_raw, u.pos_z_raw, u.range_raw, u.team))
                .collect();

            let unit_buffer = self.device.create_buffer_init_desc(
                "fog units",
                bytemuck::cast_slice(&gpu_units),
                wgpu::BufferUsages::STORAGE,
            );

            // Upload grid (for read-write).
            let grid_buffer = self.device.create_buffer_init_desc(
                "fog grid",
                bytemuck::cast_slice(&grid_data),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );

            // Readback buffer.
            let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog readback"),
                size: (cell_count * 4) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // Upload params.
            let gpu_params = GpuFogParams {
                width: w,
                height: h,
                cell_size: i64_to_pair(params.cell_size_raw),
                half_cell: i64_to_pair(params.cell_size_raw >> 1),
                unit_count: team_units.len() as u32,
                team_count: params.teams.len() as u32,
            };
            self.queue.write_buffer(
                &self.params_buffer,
                0,
                bytemuck::bytes_of(&gpu_params),
            );

            // Bind group.
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog bind group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: unit_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: grid_buffer.as_entire_binding(),
                    },
                ],
            });

            // Dispatch: one workgroup per unit, 64 threads per workgroup.
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("fog compute"),
                });

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fog pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                // x = rows per unit (up to 2*range_cells+1, capped by workgroup size 64)
                // y = unit index
                let workgroups_y = team_units.len() as u32;
                pass.dispatch_workgroups(1, workgroups_y, 1);
            }

            encoder.copy_buffer_to_buffer(
                &grid_buffer,
                0,
                &readback_buffer,
                0,
                (cell_count * 4) as u64,
            );

            self.queue.submit(Some(encoder.finish()));

            // Readback.
            let slice = readback_buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::Maintain::Wait);

            let data = slice.get_mapped_range();
            let result_u32: &[u32] = bytemuck::cast_slice(&data);
            let result_u8: Vec<u8> = result_u32.iter().map(|&v| v as u8).collect();
            drop(data);
            readback_buffer.unmap();

            result_grids.insert(team, result_u8);
        }

        result_grids
    }
}

/// Extension trait to create initialized buffers (wgpu 24 API).
trait DeviceBufferInitExt {
    fn create_buffer_init_desc(
        &self,
        label: &str,
        data: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer;
}

impl DeviceBufferInitExt for wgpu::Device {
    fn create_buffer_init_desc(
        &self,
        label: &str,
        data: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        use wgpu::util::{BufferInitDescriptor, DeviceExt};
        self.create_buffer_init(&BufferInitDescriptor {
            label: Some(label),
            contents: data,
            usage,
        })
    }
}
