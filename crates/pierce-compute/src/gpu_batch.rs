//! GPU implementation of BatchMathOps.
//!
//! Currently implements batch_distance_sq on GPU. Other operations
//! fall back to the CPU implementation. Operations are added
//! incrementally as shaders are validated for determinism.

use pierce_math::BatchMathOps;

use crate::buffers::i64_to_pair;
use crate::cpu_batch::CpuBatchMath;

/// Minimum batch size for GPU dispatch. Below this, CPU is faster.
const GPU_THRESHOLD: usize = 512;

/// GPU batch math backend with CPU fallback for unimplemented ops.
pub struct GpuBatchMath {
    device: wgpu::Device,
    queue: wgpu::Queue,
    cpu: CpuBatchMath,

    // distance_sq pipeline
    dist_sq_pipeline: wgpu::ComputePipeline,
    dist_sq_bgl: wgpu::BindGroupLayout,
    params_buf: wgpu::Buffer,

    // Reusable buffer slots: (buffer, capacity_in_elements)
    buf_a: Option<(wgpu::Buffer, usize)>,
    buf_b: Option<(wgpu::Buffer, usize)>,
    buf_c: Option<(wgpu::Buffer, usize)>,
    buf_d: Option<(wgpu::Buffer, usize)>,
    buf_result: Option<(wgpu::Buffer, usize)>,
    buf_readback: Option<(wgpu::Buffer, usize)>,
}

/// Each i64 is stored as vec2<i32> = 8 bytes.
const ELEM_SIZE: usize = 8;

impl GpuBatchMath {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("batch math"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/batch_math.wgsl").into()),
        });

        let dist_sq_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dist_sq bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_storage_ro(1),
                bgl_storage_ro(2),
                bgl_storage_ro(3),
                bgl_storage_ro(4),
                bgl_storage_rw(5),
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&dist_sq_bgl],
            push_constant_ranges: &[],
        });

        let dist_sq_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("batch_distance_sq"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("batch_distance_sq_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("batch params"),
            size: 16, // BatchParams: count + 3 padding u32
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            cpu: CpuBatchMath,
            dist_sq_pipeline,
            dist_sq_bgl,
            params_buf,
            buf_a: None,
            buf_b: None,
            buf_c: None,
            buf_d: None,
            buf_result: None,
            buf_readback: None,
        }
    }

    fn ensure_buf(
        device: &wgpu::Device,
        slot: &mut Option<(wgpu::Buffer, usize)>,
        label: &str,
        count: usize,
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
                    size: (cap * ELEM_SIZE) as u64,
                    usage,
                    mapped_at_creation: false,
                }),
                cap,
            ));
        }
    }

    /// Convert i64 slice to bytes (vec2<i32> layout, same as raw i64 on little-endian).
    fn i64_to_bytes(data: &[i64]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 8)
        }
    }

    fn gpu_distance_sq(&mut self, ax: &[i64], az: &[i64], bx: &[i64], bz: &[i64]) -> Vec<i64> {
        let n = ax.len();

        // Upload params.
        let params = [n as u32, 0u32, 0u32, 0u32];
        self.queue.write_buffer(&self.params_buf, 0, bytemuck::cast_slice(&params));

        // Ensure buffers.
        let storage_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Self::ensure_buf(&self.device, &mut self.buf_a, "ax", n, storage_usage);
        Self::ensure_buf(&self.device, &mut self.buf_b, "az", n, storage_usage);
        Self::ensure_buf(&self.device, &mut self.buf_c, "bx", n, storage_usage);
        Self::ensure_buf(&self.device, &mut self.buf_d, "bz", n, storage_usage);
        Self::ensure_buf(
            &self.device,
            &mut self.buf_result,
            "result",
            n,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        Self::ensure_buf(
            &self.device,
            &mut self.buf_readback,
            "readback",
            n,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );

        // Upload data.
        self.queue.write_buffer(&self.buf_a.as_ref().unwrap().0, 0, Self::i64_to_bytes(ax));
        self.queue.write_buffer(&self.buf_b.as_ref().unwrap().0, 0, Self::i64_to_bytes(az));
        self.queue.write_buffer(&self.buf_c.as_ref().unwrap().0, 0, Self::i64_to_bytes(bx));
        self.queue.write_buffer(&self.buf_d.as_ref().unwrap().0, 0, Self::i64_to_bytes(bz));

        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.dist_sq_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.buf_a.as_ref().unwrap().0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.buf_b.as_ref().unwrap().0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.buf_c.as_ref().unwrap().0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.buf_d.as_ref().unwrap().0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.buf_result.as_ref().unwrap().0.as_entire_binding() },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.dist_sq_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups((n as u32 + 63) / 64, 1, 1);
        }

        let size = (n * ELEM_SIZE) as u64;
        encoder.copy_buffer_to_buffer(
            &self.buf_result.as_ref().unwrap().0, 0,
            &self.buf_readback.as_ref().unwrap().0, 0,
            size,
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = self.buf_readback.as_ref().unwrap().0.slice(..size);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);

        let data = slice.get_mapped_range();
        let result: Vec<i64> = data
            .chunks_exact(8)
            .map(|chunk| i64::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        drop(data);
        self.buf_readback.as_ref().unwrap().0.unmap();

        result
    }
}

impl BatchMathOps for GpuBatchMath {
    fn batch_distance_sq(&mut self, ax: &[i64], az: &[i64], bx: &[i64], bz: &[i64]) -> Vec<i64> {
        if ax.len() < GPU_THRESHOLD {
            return self.cpu.batch_distance_sq(ax, az, bx, bz);
        }
        self.gpu_distance_sq(ax, az, bx, bz)
    }

    // Remaining ops delegate to CPU for now. GPU shaders added incrementally.
    fn batch_heading(&mut self, dx: &[i64], dz: &[i64]) -> Vec<i64> {
        self.cpu.batch_heading(dx, dz)
    }

    fn batch_sincos(&mut self, angles: &[i64]) -> (Vec<i64>, Vec<i64>) {
        self.cpu.batch_sincos(angles)
    }

    fn batch_integrate(
        &mut self,
        pos_x: &mut [i64], pos_y: &mut [i64], pos_z: &mut [i64],
        vel_x: &[i64], vel_y: &[i64], vel_z: &[i64],
    ) {
        self.cpu.batch_integrate(pos_x, pos_y, pos_z, vel_x, vel_y, vel_z);
    }

    fn batch_normalize_2d(&mut self, vx: &[i64], vy: &[i64]) -> (Vec<i64>, Vec<i64>) {
        self.cpu.batch_normalize_2d(vx, vy)
    }

    fn batch_mul(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        self.cpu.batch_mul(a, b)
    }

    fn batch_div(&mut self, a: &[i64], b: &[i64]) -> Vec<i64> {
        self.cpu.batch_div(a, b)
    }
}

// --- Helpers ---

fn bgl_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_ro(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_rw(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
