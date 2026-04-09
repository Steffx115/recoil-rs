//! GPU implementation of targeting computation.
//!
//! One thread per shooter, brute-forces all candidates. On GPU this is
//! efficient — 5000 threads × 5000 candidates = 25M comparisons is
//! trivial at GPU throughput. No spatial grid needed.

use std::collections::BTreeMap;

use bytemuck::Zeroable;
use pierce_sim::compute::{TargetCompute, TargetingCandidateInput, TargetingShooterInput};

use crate::buffers::{i64_to_pair, GpuCandidate, GpuShooter, GpuTargetingParams};

/// GPU targeting backend.
pub struct GpuTargetingCompute {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
}

impl GpuTargetingCompute {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader_src = include_str!("shaders/targeting.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("targeting compute shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("targeting bind group layout"),
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
                    // 1: shooters storage (read)
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
                    // 2: candidates storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 3: fog grid storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 4: results storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
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
            label: Some("targeting pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("targeting compute pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("targeting_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("targeting params"),
            size: std::mem::size_of::<GpuTargetingParams>() as u64,
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

    fn create_buffer(&self, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        use wgpu::util::{BufferInitDescriptor, DeviceExt};
        self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some(label),
            contents: data,
            usage,
        })
    }
}

impl TargetCompute for GpuTargetingCompute {
    fn compute_targets(
        &mut self,
        shooters: &[TargetingShooterInput],
        candidates: &[TargetingCandidateInput],
        fog_grids: Option<&BTreeMap<u8, Vec<u8>>>,
        fog_width: u32,
        fog_height: u32,
        fog_cell_size_raw: i64,
    ) -> Vec<i32> {
        if shooters.is_empty() {
            return Vec::new();
        }

        // Convert shooters to GPU format.
        let gpu_shooters: Vec<GpuShooter> = shooters
            .iter()
            .map(|s| GpuShooter {
                pos_x: i64_to_pair(s.pos_x_raw),
                pos_y: i64_to_pair(s.pos_y_raw),
                pos_z: i64_to_pair(s.pos_z_raw),
                max_range: i64_to_pair(s.max_range_raw),
                min_range_0: i64_to_pair(s.weapon_min_ranges[0]),
                min_range_1: i64_to_pair(if s.weapon_count > 1 { s.weapon_min_ranges[1] } else { 0 }),
                min_range_2: i64_to_pair(if s.weapon_count > 2 { s.weapon_min_ranges[2] } else { 0 }),
                min_range_3: i64_to_pair(if s.weapon_count > 3 { s.weapon_min_ranges[3] } else { 0 }),
                team: s.team as u32,
                fire_mode: s.fire_mode as u32,
                has_indirect: if s.has_indirect { 1 } else { 0 },
                weapon_count: s.weapon_count as u32,
                manual_target_idx: s.manual_target_idx,
                last_attacker_idx: s.last_attacker_idx,
                _pad0: 0,
                _pad1: 0,
            })
            .collect();

        // Convert candidates to GPU format.
        let gpu_candidates: Vec<GpuCandidate> = candidates
            .iter()
            .map(|c| {
                let mut flags = 0u32;
                if c.is_dead { flags |= 1; }
                if c.has_weapons { flags |= 2; }
                if c.is_building { flags |= 4; }
                GpuCandidate {
                    pos_x: i64_to_pair(c.pos_x_raw),
                    pos_y: i64_to_pair(c.pos_y_raw),
                    pos_z: i64_to_pair(c.pos_z_raw),
                    health: i64_to_pair(c.health_raw),
                    pending_damage: i64_to_pair(c.pending_damage_raw),
                    sim_id_lo: c.sim_id as u32,
                    sim_id_hi: (c.sim_id >> 32) as u32,
                    team: c.team as u32,
                    flags,
                    _pad0: 0,
                    _pad1: 0,
                }
            })
            .collect();

        // Build fog grid buffer. If fog exists, flatten all team grids into one buffer.
        // Layout: team_idx * width * height + z * width + x (u32 per cell).
        let (fog_data, has_fog) = if let Some(grids) = fog_grids {
            let cell_count = (fog_width as usize) * (fog_height as usize);
            // We need team indices to match shooter team IDs.
            // Build a flat buffer where team N's grid starts at N * cell_count.
            let max_team = grids.keys().max().copied().unwrap_or(0) as usize;
            let total = (max_team + 1) * cell_count;
            let mut flat = vec![0u32; total];
            for (&team, grid) in grids {
                let offset = (team as usize) * cell_count;
                for (i, &v) in grid.iter().enumerate() {
                    if offset + i < flat.len() {
                        flat[offset + i] = v as u32;
                    }
                }
            }
            (flat, 1u32)
        } else {
            (vec![0u32; 4], 0u32) // Dummy buffer (wgpu requires non-empty)
        };

        // Upload params.
        let gpu_params = GpuTargetingParams {
            shooter_count: shooters.len() as u32,
            candidate_count: candidates.len() as u32,
            fog_width,
            fog_height,
            fog_cell_size: i64_to_pair(fog_cell_size_raw),
            has_fog,
            _pad: 0,
        };
        self.queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&gpu_params));

        // Create buffers.
        let shooter_buffer = self.create_buffer(
            "targeting shooters",
            bytemuck::cast_slice(&gpu_shooters),
            wgpu::BufferUsages::STORAGE,
        );

        // Ensure non-empty candidate buffer.
        let candidate_data = if gpu_candidates.is_empty() {
            vec![GpuCandidate::zeroed()]
        } else {
            gpu_candidates
        };
        let candidate_buffer = self.create_buffer(
            "targeting candidates",
            bytemuck::cast_slice(&candidate_data),
            wgpu::BufferUsages::STORAGE,
        );

        let fog_buffer = self.create_buffer(
            "targeting fog",
            bytemuck::cast_slice(&fog_data),
            wgpu::BufferUsages::STORAGE,
        );

        // Results buffer: one i32 per shooter.
        let results_size = (shooters.len() * 4) as u64;
        let results_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("targeting results"),
            size: results_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("targeting readback"),
            size: results_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Bind group.
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("targeting bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shooter_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: candidate_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fog_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: results_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch.
        let workgroups_x = (shooters.len() as u32 + 63) / 64;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("targeting compute"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("targeting pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(workgroups_x, 1, 1);
        }

        encoder.copy_buffer_to_buffer(&results_buffer, 0, &readback_buffer, 0, results_size);
        self.queue.submit(Some(encoder.finish()));

        // Readback.
        let slice = readback_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);

        let data = slice.get_mapped_range();
        let result: Vec<i32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        readback_buffer.unmap();

        result
    }
}
