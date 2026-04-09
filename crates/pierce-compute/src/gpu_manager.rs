//! Double-buffered GPU compute manager.
//!
//! Combines fog + targeting into a single command encoder submission.
//! Uses async readback: while GPU computes frame N, CPU processes
//! frame N-1's results. Pre-allocates buffers with headroom.

use std::collections::BTreeMap;

use pierce_sim::compute::{
    FogCompute, FogGridParams, FogUnitInput, TargetCompute, TargetingCandidateInput,
    TargetingShooterInput,
};

use crate::buffers::{i64_to_pair, GpuCandidate, GpuFogParams, GpuFogUnit, GpuShooter, GpuTargetingParams};

/// Ensure a buffer slot has capacity for at least `count` items of `stride` bytes.
/// Reallocates with 50% headroom if too small. Callers access via `slot.as_ref().unwrap().0`.
fn ensure_buf(
    device: &wgpu::Device,
    slot: &mut Option<(wgpu::Buffer, usize)>,
    label: &str,
    count: usize,
    stride: usize,
    usage: wgpu::BufferUsages,
) {
    let needed = count.max(1);
    let realloc = match slot {
        Some((_, cap)) => *cap < needed,
        None => true,
    };
    if realloc {
        let cap = needed + needed / 2;
        *slot = Some((
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (cap * stride) as u64,
                usage,
                mapped_at_creation: false,
            }),
            cap,
        ));
    }
}

struct PendingFogReadback {
    buffer: wgpu::Buffer,
    teams: Vec<u8>,
    width: u32,
    height: u32,
    cell_count: usize,
}

struct PendingTargetingReadback {
    buffer: wgpu::Buffer,
    count: usize,
}

/// Manages GPU compute pipelines with pre-allocated buffers.
pub struct GpuComputeManager {
    device: wgpu::Device,
    queue: wgpu::Queue,

    fog_pipeline: wgpu::ComputePipeline,
    fog_bgl: wgpu::BindGroupLayout,
    fog_params_buf: wgpu::Buffer,

    targeting_pipeline: wgpu::ComputePipeline,
    targeting_bgl: wgpu::BindGroupLayout,
    targeting_params_buf: wgpu::Buffer,

    // Pre-allocated buffer slots: (buffer, capacity_in_items)
    fog_unit_buf: Option<(wgpu::Buffer, usize)>,
    fog_grid_buf: Option<(wgpu::Buffer, usize)>,
    fog_readback_buf: Option<(wgpu::Buffer, usize)>,
    shooter_buf: Option<(wgpu::Buffer, usize)>,
    candidate_buf: Option<(wgpu::Buffer, usize)>,
    fog_for_targeting_buf: Option<(wgpu::Buffer, usize)>,
    results_buf: Option<(wgpu::Buffer, usize)>,
    results_readback_buf: Option<(wgpu::Buffer, usize)>,

    pending_fog: Option<PendingFogReadback>,
    pending_targeting: Option<PendingTargetingReadback>,
}

impl GpuComputeManager {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let fog_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fog.wgsl").into()),
        });
        let targeting_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("targeting compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/targeting.wgsl").into()),
        });

        let fog_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog bgl"),
            entries: &[bgl_uniform(0), bgl_storage_ro(1), bgl_storage_rw(2)],
        });
        let targeting_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("targeting bgl"),
            entries: &[bgl_uniform(0), bgl_storage_ro(1), bgl_storage_ro(2), bgl_storage_ro(3), bgl_storage_rw(4)],
        });

        let fog_pipeline = mk_pipeline(&device, &fog_shader, &fog_bgl, "fog_main");
        let targeting_pipeline = mk_pipeline(&device, &targeting_shader, &targeting_bgl, "targeting_main");

        let fog_params_buf = mk_uniform(&device, "fog params", std::mem::size_of::<GpuFogParams>());
        let targeting_params_buf = mk_uniform(&device, "targeting params", std::mem::size_of::<GpuTargetingParams>());

        Self {
            device, queue,
            fog_pipeline, fog_bgl, fog_params_buf,
            targeting_pipeline, targeting_bgl, targeting_params_buf,
            fog_unit_buf: None, fog_grid_buf: None, fog_readback_buf: None,
            shooter_buf: None, candidate_buf: None, fog_for_targeting_buf: None,
            results_buf: None, results_readback_buf: None,
            pending_fog: None, pending_targeting: None,
        }
    }

    /// Read back previous frame's results. Blocks if GPU hasn't finished yet.
    pub fn read_previous_results(&mut self) -> (Option<BTreeMap<u8, Vec<u8>>>, Option<Vec<i32>>) {
        let fog = self.pending_fog.take().map(|pf| {
            let slice = pf.buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::Maintain::Wait);
            let data = slice.get_mapped_range();
            let raw: &[u32] = bytemuck::cast_slice(&data);
            let mut grids = BTreeMap::new();
            for (ti, &team) in pf.teams.iter().enumerate() {
                let off = ti * pf.cell_count;
                let end = (off + pf.cell_count).min(raw.len());
                grids.insert(team, raw[off..end].iter().map(|&v| v as u8).collect());
            }
            drop(data);
            pf.buffer.unmap();
            grids
        });

        let targeting = self.pending_targeting.take().map(|pt| {
            let slice = pt.buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::Maintain::Wait);
            let data = slice.get_mapped_range();
            let result: Vec<i32> = bytemuck::cast_slice(&data).to_vec();
            drop(data);
            pt.buffer.unmap();
            result
        });

        (fog, targeting)
    }

    /// Submit fog + targeting compute for this frame. Returns immediately
    /// (GPU work runs async). Call read_previous_results() next frame.
    pub fn submit_frame(
        &mut self,
        fog_params: Option<(&FogGridParams, &[FogUnitInput], &BTreeMap<u8, Vec<u8>>)>,
        shooters: &[TargetingShooterInput],
        candidates: &[TargetingCandidateInput],
        fog_grids_for_targeting: Option<&BTreeMap<u8, Vec<u8>>>,
        fog_width: u32,
        fog_height: u32,
        fog_cell_size_raw: i64,
    ) {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("compute frame"),
        });

        // --- Fog pass ---
        if let Some((params, units, prev_grids)) = fog_params {
            let cell_count = (params.width as usize) * (params.height as usize);

            // Per-team fog (process largest team, TODO: batch all teams in one dispatch)
            for &team in &params.teams {
                let team_units: Vec<GpuFogUnit> = units
                    .iter()
                    .filter(|u| u.team == team)
                    .map(|u| GpuFogUnit::from_raw(u.pos_x_raw, u.pos_z_raw, u.range_raw, u.team))
                    .collect();
                if team_units.is_empty() { continue; }

                // Ensure + write unit buffer.
                ensure_buf(&self.device, &mut self.fog_unit_buf, "fog units",
                    team_units.len(), std::mem::size_of::<GpuFogUnit>(),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
                self.queue.write_buffer(&self.fog_unit_buf.as_ref().unwrap().0, 0, bytemuck::cast_slice(&team_units));

                // Grid data.
                let mut grid_data: Vec<u32> = if let Some(prev) = prev_grids.get(&team) {
                    prev.iter().map(|&v| if v == 2 { 1u32 } else { v as u32 }).collect()
                } else {
                    vec![0u32; cell_count]
                };
                grid_data.resize(cell_count, 0);

                ensure_buf(&self.device, &mut self.fog_grid_buf, "fog grid",
                    cell_count, 4,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
                self.queue.write_buffer(&self.fog_grid_buf.as_ref().unwrap().0, 0, bytemuck::cast_slice(&grid_data));

                // Params.
                let gpu_params = GpuFogParams {
                    width: params.width, height: params.height,
                    cell_size: i64_to_pair(params.cell_size_raw),
                    half_cell: i64_to_pair(params.cell_size_raw >> 1),
                    unit_count: team_units.len() as u32,
                    team_count: params.teams.len() as u32,
                };
                self.queue.write_buffer(&self.fog_params_buf, 0, bytemuck::bytes_of(&gpu_params));

                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None, layout: &self.fog_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.fog_params_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.fog_unit_buf.as_ref().unwrap().0.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: self.fog_grid_buf.as_ref().unwrap().0.as_entire_binding() },
                    ],
                });

                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&self.fog_pipeline);
                    pass.set_bind_group(0, &bg, &[]);
                    pass.dispatch_workgroups(1, team_units.len() as u32, 1);
                }

                // Readback.
                ensure_buf(&self.device, &mut self.fog_readback_buf, "fog readback",
                    cell_count, 4,
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
                encoder.copy_buffer_to_buffer(
                    &self.fog_grid_buf.as_ref().unwrap().0, 0,
                    &self.fog_readback_buf.as_ref().unwrap().0, 0,
                    (cell_count * 4) as u64,
                );

                self.pending_fog = Some(PendingFogReadback {
                    buffer: self.fog_readback_buf.as_ref().unwrap().0.clone(),
                    teams: vec![team],
                    width: params.width, height: params.height, cell_count,
                });
            }
        }

        // --- Targeting pass ---
        if !shooters.is_empty() {
            use bytemuck::Zeroable;

            let gpu_shooters: Vec<GpuShooter> = shooters.iter().map(|s| GpuShooter {
                pos_x: i64_to_pair(s.pos_x_raw), pos_y: i64_to_pair(s.pos_y_raw), pos_z: i64_to_pair(s.pos_z_raw),
                max_range: i64_to_pair(s.max_range_raw),
                min_range_0: i64_to_pair(s.weapon_min_ranges[0]),
                min_range_1: i64_to_pair(if s.weapon_count > 1 { s.weapon_min_ranges[1] } else { 0 }),
                min_range_2: i64_to_pair(if s.weapon_count > 2 { s.weapon_min_ranges[2] } else { 0 }),
                min_range_3: i64_to_pair(if s.weapon_count > 3 { s.weapon_min_ranges[3] } else { 0 }),
                team: s.team as u32, fire_mode: s.fire_mode as u32,
                has_indirect: if s.has_indirect { 1 } else { 0 },
                weapon_count: s.weapon_count as u32,
                manual_target_idx: s.manual_target_idx, last_attacker_idx: s.last_attacker_idx,
                _pad0: 0, _pad1: 0,
            }).collect();

            let gpu_candidates: Vec<GpuCandidate> = if candidates.is_empty() {
                vec![GpuCandidate::zeroed()]
            } else {
                candidates.iter().map(|c| {
                    let mut flags = 0u32;
                    if c.is_dead { flags |= 1; }
                    if c.has_weapons { flags |= 2; }
                    if c.is_building { flags |= 4; }
                    GpuCandidate {
                        pos_x: i64_to_pair(c.pos_x_raw), pos_y: i64_to_pair(c.pos_y_raw), pos_z: i64_to_pair(c.pos_z_raw),
                        health: i64_to_pair(c.health_raw), pending_damage: i64_to_pair(c.pending_damage_raw),
                        sim_id_lo: c.sim_id as u32, sim_id_hi: (c.sim_id >> 32) as u32,
                        team: c.team as u32, flags, _pad0: 0, _pad1: 0,
                    }
                }).collect()
            };

            let (fog_data, has_fog) = if let Some(grids) = fog_grids_for_targeting {
                let cc = (fog_width as usize) * (fog_height as usize);
                let mt = grids.keys().max().copied().unwrap_or(0) as usize;
                let total = ((mt + 1) * cc).max(1);
                let mut flat = vec![0u32; total];
                for (&team, grid) in grids {
                    let off = (team as usize) * cc;
                    for (i, &v) in grid.iter().enumerate() {
                        if off + i < flat.len() { flat[off + i] = v as u32; }
                    }
                }
                (flat, 1u32)
            } else {
                (vec![0u32; 4], 0u32)
            };

            let gpu_params = GpuTargetingParams {
                shooter_count: shooters.len() as u32, candidate_count: candidates.len() as u32,
                fog_width, fog_height, fog_cell_size: i64_to_pair(fog_cell_size_raw),
                has_fog, _pad: 0,
            };
            self.queue.write_buffer(&self.targeting_params_buf, 0, bytemuck::bytes_of(&gpu_params));

            ensure_buf(&self.device, &mut self.shooter_buf, "shooters",
                gpu_shooters.len(), std::mem::size_of::<GpuShooter>(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            self.queue.write_buffer(&self.shooter_buf.as_ref().unwrap().0, 0, bytemuck::cast_slice(&gpu_shooters));

            ensure_buf(&self.device, &mut self.candidate_buf, "candidates",
                gpu_candidates.len(), std::mem::size_of::<GpuCandidate>(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            self.queue.write_buffer(&self.candidate_buf.as_ref().unwrap().0, 0, bytemuck::cast_slice(&gpu_candidates));

            ensure_buf(&self.device, &mut self.fog_for_targeting_buf, "targeting fog",
                fog_data.len(), 4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            self.queue.write_buffer(&self.fog_for_targeting_buf.as_ref().unwrap().0, 0, bytemuck::cast_slice(&fog_data));

            ensure_buf(&self.device, &mut self.results_buf, "results",
                shooters.len(), 4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);

            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None, layout: &self.targeting_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.targeting_params_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.shooter_buf.as_ref().unwrap().0.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.candidate_buf.as_ref().unwrap().0.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: self.fog_for_targeting_buf.as_ref().unwrap().0.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: self.results_buf.as_ref().unwrap().0.as_entire_binding() },
                ],
            });

            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.targeting_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups((shooters.len() as u32 + 63) / 64, 1, 1);
            }

            ensure_buf(&self.device, &mut self.results_readback_buf, "results readback",
                shooters.len(), 4,
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            encoder.copy_buffer_to_buffer(
                &self.results_buf.as_ref().unwrap().0, 0,
                &self.results_readback_buf.as_ref().unwrap().0, 0,
                (shooters.len() * 4) as u64,
            );

            self.pending_targeting = Some(PendingTargetingReadback {
                buffer: self.results_readback_buf.as_ref().unwrap().0.clone(),
                count: shooters.len(),
            });
        }

        self.queue.submit(Some(encoder.finish()));
    }
}

// Backward-compatible trait impls (synchronous, for testing).
impl FogCompute for GpuComputeManager {
    fn compute_fog(&mut self, params: &FogGridParams, units: &[FogUnitInput], prev: &BTreeMap<u8, Vec<u8>>) -> BTreeMap<u8, Vec<u8>> {
        self.submit_frame(Some((params, units, prev)), &[], &[], None, 0, 0, 0);
        self.read_previous_results().0.unwrap_or_default()
    }
}

impl TargetCompute for GpuComputeManager {
    fn compute_targets(&mut self, shooters: &[TargetingShooterInput], candidates: &[TargetingCandidateInput],
        fog_grids: Option<&BTreeMap<u8, Vec<u8>>>, fw: u32, fh: u32, fcs: i64) -> Vec<i32> {
        self.submit_frame(None, shooters, candidates, fog_grids, fw, fh, fcs);
        self.read_previous_results().1.unwrap_or_else(|| vec![-1; shooters.len()])
    }
}

// --- Helpers ---

fn bgl_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn bgl_storage_ro(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn bgl_storage_rw(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn mk_pipeline(device: &wgpu::Device, shader: &wgpu::ShaderModule, bgl: &wgpu::BindGroupLayout, entry: &str) -> wgpu::ComputePipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None, bind_group_layouts: &[bgl], push_constant_ranges: &[],
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: Some(&layout), module: shader, entry_point: Some(entry),
        compilation_options: Default::default(), cache: None,
    })
}
fn mk_uniform(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label), size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
