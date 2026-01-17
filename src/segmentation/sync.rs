use crate::app::components::{LabelmapData, Segmentation, ViewMode};
use crate::segmentation::algorithms::surface_nets::DistanceSampler;
use crate::segmentation::algorithms::MarchingSquares;
use crate::segmentation::mesh::GpuMeshResources;
use hecs::World;

/// System to synchronize Labelmap voxel data to 2D contours
pub fn sys_sync_labelmap_to_contours(world: &mut World) {
    let mut to_update = Vec::new();
    for (entity, seg) in world.query_mut::<&mut Segmentation>() {
        if let Some(ref tsdf) = seg.tsdf {
            if !tsdf.dirty_chunks.is_empty() || seg.contour_set.slices.is_empty() {
                to_update.push(entity);
            }
        } else if seg.contour_set.slices.is_empty() {
            // Fallback for old labelmap-only segmentations
            to_update.push(entity);
        }
    }
    for entity in to_update {
        if let Ok(mut query) = world.query_one::<(&mut Segmentation, &LabelmapData)>(entity) {
            if let Some((seg, labelmap)) = query.get() {
                if seg.tsdf.is_some() {
                    sync_tsdf_to_contours(seg, labelmap.dimensions);
                } else {
                    sync_full_labelmap(seg, labelmap);
                }
            }
        }
    }
}

pub fn sync_tsdf_to_contours(seg: &mut Segmentation, dims: [u32; 3]) {
    let tsdf = if let Some(t) = seg.tsdf.take() {
        t
    } else {
        return;
    };

    let ms = MarchingSquares::new(0.0);

    // Determine which slices are dirty
    let mut axial_dirty = std::collections::HashSet::new();
    let mut coronal_dirty = std::collections::HashSet::new();
    let mut sagittal_dirty = std::collections::HashSet::new();

    // If contour set is empty, we must do a full sync of all occupied chunks
    if seg.contour_set.slices.is_empty() {
        for &(cx, cy, cz) in tsdf.chunks.keys() {
            for dz in 0..32 {
                axial_dirty.insert(cz as i32 * 32 + dz);
            }
            for dy in 0..32 {
                coronal_dirty.insert(cy as i32 * 32 + dy);
            }
            for dx in 0..32 {
                sagittal_dirty.insert(cx as i32 * 32 + dx);
            }
        }
    } else {
        for &(cx, cy, cz) in &tsdf.dirty_chunks {
            for dz in 0..32 {
                axial_dirty.insert(cz as i32 * 32 + dz);
            }
            for dy in 0..32 {
                coronal_dirty.insert(cy as i32 * 32 + dy);
            }
            for dx in 0..32 {
                sagittal_dirty.insert(cx as i32 * 32 + dx);
            }
        }
    }

    let (min, max) = tsdf.bounds();

    // Axial (XY)
    for z in axial_dirty {
        if z < min[2] || z >= max[2] {
            continue;
        }
        let w = (max[0] - min[0]) as u32;
        let h = (max[1] - min[1]) as u32;
        if w < 2 || h < 2 {
            continue;
        }

        let mut slice_data = vec![0.0f32; (w * h) as usize];
        for ly in 0..h {
            for lx in 0..w {
                slice_data[(ly * w + lx) as usize] =
                    tsdf.get_distance(min[0] + lx as i32, min[1] + ly as i32, z);
            }
        }

        let mut contours = ms.extract(&slice_data, (w, h), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (min[0] as f32 + p.x * (w as f32 - 1.0)) / (dims[0] as f32 - 1.0);
                p.y = (min[1] as f32 + p.y * (h as f32 - 1.0)) / (dims[1] as f32 - 1.0);
            }
        }
        seg.contour_set.update_slice(ViewMode::Axial, z, contours);
    }

    // Coronal (XZ)
    for y in coronal_dirty {
        if y < min[1] || y >= max[1] {
            continue;
        }
        let w = (max[0] - min[0]) as u32;
        let h = (max[2] - min[2]) as u32;
        if w < 2 || h < 2 {
            continue;
        }

        let mut slice_data = vec![0.0f32; (w * h) as usize];
        for lz in 0..h {
            for lx in 0..w {
                slice_data[(lz * w + lx) as usize] =
                    tsdf.get_distance(min[0] + lx as i32, y, min[2] + lz as i32);
            }
        }

        let mut contours = ms.extract(&slice_data, (w, h), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (min[0] as f32 + p.x * (w as f32 - 1.0)) / (dims[0] as f32 - 1.0);
                p.y = (min[2] as f32 + p.y * (h as f32 - 1.0)) / (dims[2] as f32 - 1.0);
            }
        }
        seg.contour_set.update_slice(ViewMode::Coronal, y, contours);
    }

    // Sagittal (YZ)
    for x in sagittal_dirty {
        if x < min[0] || x >= max[0] {
            continue;
        }
        let w = (max[1] - min[1]) as u32;
        let h = (max[2] - min[2]) as u32;
        if w < 2 || h < 2 {
            continue;
        }

        let mut slice_data = vec![0.0f32; (w * h) as usize];
        for lz in 0..h {
            for ly in 0..w {
                slice_data[(lz * w + ly) as usize] =
                    tsdf.get_distance(x, min[1] + ly as i32, min[2] + lz as i32);
            }
        }

        let mut contours = ms.extract(&slice_data, (w, h), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (min[1] as f32 + p.x * (w as f32 - 1.0)) / (dims[1] as f32 - 1.0);
                p.y = (min[2] as f32 + p.y * (h as f32 - 1.0)) / (dims[2] as f32 - 1.0);
            }
        }
        seg.contour_set
            .update_slice(ViewMode::Sagittal, x, contours);
    }

    seg.tsdf = Some(tsdf);
}

/// System to synchronize Labelmap voxel data to 3D mesh using incremental meshing
/// Throttled to max ~20fps mesh updates to reduce WASM lag
pub fn sys_sync_labelmap_to_mesh(device: &wgpu::Device, queue: &wgpu::Queue, world: &mut World) {
    use crate::segmentation::algorithms::IncrementalMesher;
    use web_time::{Duration, Instant};

    const MESH_THROTTLE_MS: u64 = 50; // ~20fps max for mesh updates

    let mut to_update = Vec::new();
    for (entity, seg) in world.query_mut::<&mut Segmentation>() {
        // Switch to TSDF-based sync if available
        if let Some(ref tsdf) = seg.tsdf {
            if !tsdf.dirty_chunks.is_empty() || seg.mesh.vertices.is_empty() {
                // Check throttle - always update if mesh is empty (initial) or enough time passed
                let should_update = seg.mesh.vertices.is_empty()
                    || seg
                        .last_mesh_update
                        .map(|t| t.elapsed() > Duration::from_millis(MESH_THROTTLE_MS))
                        .unwrap_or(true);

                if should_update {
                    to_update.push(entity);
                }
            }
        }
    }

    for entity in to_update {
        if let Ok(mut query) = world.query_one::<(&mut Segmentation, &LabelmapData)>(entity) {
            if let Some((seg, labelmap)) = query.get() {
                if let Some(mut tsdf) = seg.tsdf.take() {
                    // Initialize mesher if needed
                    let mesher = seg
                        .mesher
                        .get_or_insert_with(|| IncrementalMesher::new(0.0));

                    // Update only dirty chunks
                    mesher.update_dirty(&tsdf, labelmap.dimensions);

                    // Flatten to single mesh for GPU
                    let (v, n, i) = mesher.flatten();
                    seg.mesh.vertices = v;
                    seg.mesh.normals = n;
                    seg.mesh.indices = i;

                    // Clear dirty tracking and update timestamp
                    tsdf.dirty_chunks.clear();
                    seg.tsdf = Some(tsdf);
                    seg.last_mesh_update = Some(Instant::now());

                    // Update GPU buffers
                    if seg.gpu_mesh.is_none() {
                        seg.gpu_mesh = Some(GpuMeshResources::new(device, &seg.mesh));
                    } else if let Some(ref mut gpu) = seg.gpu_mesh {
                        gpu.update(device, queue, &seg.mesh);
                    }
                }
            }
        }
    }
}

pub fn sync_full_labelmap(seg: &mut Segmentation, labelmap: &LabelmapData) {
    let dims = labelmap.dimensions;
    let ms = MarchingSquares::new(0.5);

    seg.contour_set.clear();

    // Axial slices
    for z in 0..dims[2] {
        let mut slice_data = vec![0.0f32; (dims[0] * dims[1]) as usize];
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let idx = (z * dims[0] * dims[1] + y * dims[0] + x) as usize;
                // Currently only supporting label 1 for simplicity
                if labelmap.raw_data[idx] > 0 {
                    slice_data[(y * dims[0] + x) as usize] = 1.0;
                }
            }
        }
        let contours = ms.extract(&slice_data, (dims[0], dims[1]), 1);
        if !contours.is_empty() {
            seg.contour_set
                .update_slice(ViewMode::Axial, z as i32, contours);
        }
    }

    // Coronal slices
    for y in 0..dims[1] {
        let mut slice_data = vec![0.0f32; (dims[0] * dims[2]) as usize];
        for z in 0..dims[2] {
            for x in 0..dims[0] {
                let idx = (z * dims[0] * dims[1] + y * dims[0] + x) as usize;
                if labelmap.raw_data[idx] > 0 {
                    slice_data[(z * dims[0] + x) as usize] = 1.0;
                }
            }
        }
        let contours = ms.extract(&slice_data, (dims[0], dims[2]), 1);
        if !contours.is_empty() {
            seg.contour_set
                .update_slice(ViewMode::Coronal, y as i32, contours);
        }
    }

    // Sagittal slices
    for x in 0..dims[0] {
        let mut slice_data = vec![0.0f32; (dims[1] * dims[2]) as usize];
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                let idx = (z * dims[0] * dims[1] + y * dims[0] + x) as usize;
                if labelmap.raw_data[idx] > 0 {
                    slice_data[(z * dims[1] + y) as usize] = 1.0;
                }
            }
        }
        let contours = ms.extract(&slice_data, (dims[1], dims[2]), 1);
        if !contours.is_empty() {
            seg.contour_set
                .update_slice(ViewMode::Sagittal, x as i32, contours);
        }
    }
}
