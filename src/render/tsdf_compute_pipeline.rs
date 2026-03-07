//! GPU TSDF compute pipeline using Jump Flood Algorithm (JFA).
//!
//! Builds truncated signed distance fields on the GPU instead of the CPU,
//! reducing TSDF build time from ~200–500 ms (CPU EDT) to ~5–30 ms.
//!
//! ## Algorithm
//!
//! Three compute passes dispatched for every labelmap segment:
//!
//! 1. **`init_seeds`** — mark fg/bg boundary voxels as JFA seeds.
//! 2. **`jfa_pass`** — propagate the nearest known seed (⌈log₂ max_dim⌉
//!    passes with step sizes `max_pow2/2 … 1`).
//! 3. **`finalize_tsdf`** — convert nearest-seed distance to signed,
//!    truncated, quantised i16 output.
//!
//! Computation runs in **ROI space**: a cropped sub-volume containing the
//! label boundary plus `truncation_mm / spacing` voxels of padding.  This
//! keeps GPU buffer sizes small (typically ≪ 256 MB even for large volumes).
//!
//! ## WASM / WebGL2
//!
//! Synchronous GPU readback (`device.poll(Maintain::Wait)`) is not available
//! on WASM.  The pipeline struct and `new()` constructor are compiled
//! everywhere so it can be stored in `RenderingContext`, but `build_tsdf_sync`
//! is gated to `#[cfg(not(target_arch = "wasm32"))]`.  Callers should fall
//! back to [`crate::convert::build_tsdf_chunks_from_labelmap`] on WASM.

use crate::app::segment::ChunkKey;
use crate::convert::{chunk_bounds_for_key, chunk_keys_for_bounds, TsdfChunk};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// Return type of [`TsdfComputePipeline::build_tsdf_sync`]:
/// `(chunks, tsdf_dims, tsdf_spacing, tsdf_origin)`.
pub type TsdfBuildResult = (HashMap<ChunkKey, TsdfChunk>, [u32; 3], [f32; 3], [f32; 3]);

// ── Uniform buffer layout (matches tsdf_compute.wgsl) ────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct TsdfComputeUniforms {
    roi_dims: [u32; 3], // ROI sub-volume dimensions
    step: u32,          // JFA step size (0 for init / finalize passes)
    label: u32,         // Target label value (0 = any non-zero)
    truncation_mm: f32, // Truncation distance in mm
    trunc_scale: f32,   // 32 767 / truncation_mm  (quantisation scale)
    _pad0: u32,
    spacing: [f32; 3], // Voxel spacing in mm
    _pad1: u32,
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// GPU compute pipeline for TSDF generation via 3-D Jump Flood Algorithm.
pub struct TsdfComputePipeline {
    init_pipeline: wgpu::ComputePipeline,
    jfa_pipeline: wgpu::ComputePipeline,
    final_pipeline: wgpu::ComputePipeline,
    uniform_layout: wgpu::BindGroupLayout,
    data_layout: wgpu::BindGroupLayout,
}

impl TsdfComputePipeline {
    /// Create the compute pipeline.  Should only be called when the wgpu
    /// backend supports compute shaders (i.e. `!SegPerfConfig::fallback_active`).
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("TSDF Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tsdf_compute.wgsl").into()),
        });

        // Bind group 0: uniform buffer.
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("TSDF Uniform Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                        TsdfComputeUniforms,
                    >() as u64),
                },
                count: None,
            }],
        });

        // Bind group 1: labelmap (read-only) + seeds (read-write) + tsdf output.
        let data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("TSDF Data Layout"),
            entries: &[
                // binding 0 — labelmap (read-only storage)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 1 — seeds (read-write)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 2 — tsdf output (read-write)
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
            label: Some("TSDF Pipeline Layout"),
            bind_group_layouts: &[&uniform_layout, &data_layout],
            push_constant_ranges: &[],
        });

        let make_pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        Self {
            init_pipeline: make_pipeline("init_seeds"),
            jfa_pipeline: make_pipeline("jfa_pass"),
            final_pipeline: make_pipeline("finalize_tsdf"),
            uniform_layout,
            data_layout,
        }
    }

    /// Build TSDF chunks for a single label from a binary labelmap using the GPU.
    ///
    /// Returns `(chunks, tsdf_dims, tsdf_spacing, tsdf_origin)` mirroring the
    /// CPU [`build_tsdf_chunks_from_labelmap`] API, or `None` if the ROI
    /// dimensions exceed the 10-bit coordinate limit (> 1023 per axis) or if
    /// no foreground voxels are found.
    ///
    /// **Native only** — synchronous GPU readback via `device.poll(Maintain::Wait)`
    /// is not available on WASM.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn build_tsdf_sync(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mask: &[u8],
        dims: [u32; 3],
        spacing: [f32; 3],
        label: u8,
        chunk_size: u32,
        truncation_mm: f32,
    ) -> Option<TsdfBuildResult> {
        use wgpu::util::DeviceExt;

        // ── Step 1: Foreground bounding box ──────────────────────────────────
        let (lo, hi) = fg_bounding_box(mask, dims, label)?;

        // ── Step 2: Expand by truncation padding → ROI bounds ────────────────
        let trunc_vox = [
            (truncation_mm / spacing[0]).ceil() as u32 + 1,
            (truncation_mm / spacing[1]).ceil() as u32 + 1,
            (truncation_mm / spacing[2]).ceil() as u32 + 1,
        ];
        let roi_origin = [
            lo[0].saturating_sub(trunc_vox[0]),
            lo[1].saturating_sub(trunc_vox[1]),
            lo[2].saturating_sub(trunc_vox[2]),
        ];
        let roi_end = [
            (hi[0] + trunc_vox[0]).min(dims[0].saturating_sub(1)),
            (hi[1] + trunc_vox[1]).min(dims[1].saturating_sub(1)),
            (hi[2] + trunc_vox[2]).min(dims[2].saturating_sub(1)),
        ];
        let roi_dims = [
            roi_end[0] - roi_origin[0] + 1,
            roi_end[1] - roi_origin[1] + 1,
            roi_end[2] - roi_origin[2] + 1,
        ];

        // 10-bit packing limit: each coordinate must fit in 10 bits (≤ 1023).
        if roi_dims.iter().any(|&d| d > 1023) {
            return None;
        }

        // ── Step 3: Extract ROI sub-volume (1 u32 per voxel) ─────────────────
        let roi_n = (roi_dims[0] * roi_dims[1] * roi_dims[2]) as usize;
        let mut roi_data: Vec<u32> = Vec::with_capacity(roi_n);
        for z in roi_origin[2]..=roi_end[2] {
            for y in roi_origin[1]..=roi_end[1] {
                for x in roi_origin[0]..=roi_end[0] {
                    let vi = (x + y * dims[0] + z * dims[0] * dims[1]) as usize;
                    roi_data.push(mask[vi] as u32);
                }
            }
        }

        // ── Step 4: GPU buffers ───────────────────────────────────────────────
        let buf_size = (roi_n * 4) as u64;

        let labelmap_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("TSDF Labelmap"),
            contents: bytemuck::cast_slice(&roi_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let seeds_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("TSDF Seeds"),
            size: buf_size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let tsdf_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("TSDF Output"),
            size: buf_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("TSDF Staging"),
            size: buf_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Step 5: Uniform buffer ────────────────────────────────────────────
        let trunc_scale = i16::MAX as f32 / truncation_mm;
        let mut uniforms = TsdfComputeUniforms {
            roi_dims,
            step: 0,
            label: label as u32,
            truncation_mm,
            trunc_scale,
            _pad0: 0,
            spacing,
            _pad1: 0,
        };
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("TSDF Uniforms"),
            size: std::mem::size_of::<TsdfComputeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Step 6: Bind groups ───────────────────────────────────────────────
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TSDF Uniform BG"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let data_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TSDF Data BG"),
            layout: &self.data_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: labelmap_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: seeds_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tsdf_buf.as_entire_binding(),
                },
            ],
        });

        // ── Step 7: Dispatch workgroups ───────────────────────────────────────
        // Shader workgroup size is (8, 8, 4) = 256 threads per group.
        let wgx = roi_dims[0].div_ceil(8);
        let wgy = roi_dims[1].div_ceil(8);
        let wgz = roi_dims[2].div_ceil(4);

        // Helper: dispatch one pass with the current uniforms.
        let dispatch = |pipeline: &wgpu::ComputePipeline| {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("TSDF dispatch"),
            });
            {
                let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cp.set_pipeline(pipeline);
                cp.set_bind_group(0, &uniform_bg, &[]);
                cp.set_bind_group(1, &data_bg, &[]);
                cp.dispatch_workgroups(wgx, wgy, wgz);
            }
            enc.finish()
        };

        // init_seeds pass.
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));
        queue.submit([dispatch(&self.init_pipeline)]);

        // jfa_pass: ⌈log₂(max_dim)⌉ steps.
        let max_dim = *roi_dims.iter().max().unwrap_or(&1);
        let mut step = max_dim.next_power_of_two() / 2;
        while step >= 1 {
            uniforms.step = step;
            queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));
            queue.submit([dispatch(&self.jfa_pipeline)]);
            step /= 2;
        }

        // finalize_tsdf pass + copy result to staging buffer.
        uniforms.step = 0;
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("TSDF finalize"),
        });
        {
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cp.set_pipeline(&self.final_pipeline);
            cp.set_bind_group(0, &uniform_bg, &[]);
            cp.set_bind_group(1, &data_bg, &[]);
            cp.dispatch_workgroups(wgx, wgy, wgz);
        }
        enc.copy_buffer_to_buffer(&tsdf_buf, 0, &staging_buf, 0, buf_size);
        queue.submit([enc.finish()]);

        // ── Step 8: Wait + readback ───────────────────────────────────────────
        let (tx, rx) = std::sync::mpsc::channel();
        staging_buf
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().ok()?;

        let tsdf_i32: Vec<i32> = {
            let mapped = staging_buf.slice(..).get_mapped_range();
            bytemuck::cast_slice::<u8, i32>(&mapped).to_vec()
        };
        staging_buf.unmap();

        // ── Step 9: Split flat TSDF values into TsdfChunks ───────────────────
        let tsdf_dims = roi_dims;
        let tsdf_spacing = spacing;
        let tsdf_origin = [
            roi_origin[0] as f32 * spacing[0],
            roi_origin[1] as f32 * spacing[1],
            roi_origin[2] as f32 * spacing[2],
        ];

        let full_bounds = [
            0,
            0,
            0,
            tsdf_dims[0].saturating_sub(1),
            tsdf_dims[1].saturating_sub(1),
            tsdf_dims[2].saturating_sub(1),
        ];
        let chunk_keys = chunk_keys_for_bounds(full_bounds, chunk_size);
        let mut chunks: HashMap<ChunkKey, TsdfChunk> = HashMap::new();

        for key in &chunk_keys {
            let Some(cb) = chunk_bounds_for_key(*key, chunk_size, tsdf_dims) else {
                continue;
            };
            // Pad ±1 voxel so Surface Nets can stitch across chunk boundaries.
            let padded = [
                cb[0].saturating_sub(1),
                cb[1].saturating_sub(1),
                cb[2].saturating_sub(1),
                (cb[3] + 1).min(tsdf_dims[0].saturating_sub(1)),
                (cb[4] + 1).min(tsdf_dims[1].saturating_sub(1)),
                (cb[5] + 1).min(tsdf_dims[2].saturating_sub(1)),
            ];
            let cdims = [
                padded[3] - padded[0] + 1,
                padded[4] - padded[1] + 1,
                padded[5] - padded[2] + 1,
            ];
            let chunk_origin = [
                tsdf_origin[0] + padded[0] as f32 * spacing[0],
                tsdf_origin[1] + padded[1] as f32 * spacing[1],
                tsdf_origin[2] + padded[2] as f32 * spacing[2],
            ];

            let cap = (cdims[0] * cdims[1] * cdims[2]) as usize;
            let mut values: Vec<i16> = Vec::with_capacity(cap);
            let mut weight: Vec<u8> = Vec::with_capacity(cap);
            let mut has_valid = false;

            for z in padded[2]..=padded[5] {
                for y in padded[1]..=padded[4] {
                    for x in padded[0]..=padded[3] {
                        let vi = (x + y * tsdf_dims[0] + z * tsdf_dims[0] * tsdf_dims[1]) as usize;
                        let raw = tsdf_i32[vi];
                        // Clamp to i16 range and treat saturated-max as invalid.
                        let v = raw.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                        let w: u8 = u8::from(v.abs() < i16::MAX);
                        values.push(v);
                        weight.push(w);
                        if w > 0 {
                            has_valid = true;
                        }
                    }
                }
            }

            if has_valid {
                chunks.insert(
                    *key,
                    TsdfChunk {
                        dims: cdims,
                        voxel_size: spacing,
                        origin: chunk_origin,
                        truncation_mm,
                        values,
                        weight,
                        revision: 0,
                    },
                );
            }
        }

        if chunks.is_empty() {
            return None;
        }

        Some((chunks, tsdf_dims, tsdf_spacing, tsdf_origin))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `(min, max)` voxel coordinates of the foreground label in the mask,
/// or `None` if no matching voxel exists.
fn fg_bounding_box(mask: &[u8], dims: [u32; 3], label: u8) -> Option<([u32; 3], [u32; 3])> {
    let mut lo = [u32::MAX; 3];
    let mut hi = [0u32; 3];
    let mut found = false;

    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let i = (x + y * dims[0] + z * dims[0] * dims[1]) as usize;
                let is_fg = if label == 0 {
                    mask[i] != 0
                } else {
                    mask[i] == label
                };
                if is_fg {
                    lo[0] = lo[0].min(x);
                    lo[1] = lo[1].min(y);
                    lo[2] = lo[2].min(z);
                    hi[0] = hi[0].max(x);
                    hi[1] = hi[1].max(y);
                    hi[2] = hi[2].max(z);
                    found = true;
                }
            }
        }
    }

    if found {
        Some((lo, hi))
    } else {
        None
    }
}
