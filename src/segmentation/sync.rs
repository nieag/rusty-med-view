use crate::app::components::{LabelmapData, Segmentation, ViewMode, VolumeData};
use crate::segmentation::algorithms::surface_nets::DistanceSampler;
use crate::segmentation::algorithms::{baking, projection, MarchingSquares};
use crate::segmentation::contour::Contour;
use crate::segmentation::mesh::GpuMeshResources;
use crate::segmentation::ChunkedTSDF;
use glam::Vec3;
use hecs::World;

/// System to synchronize Authoritative Vectors to TSDF (Baking)
pub fn sys_sync_vector_to_tsdf(world: &mut World) {
    for (_, seg) in world.query_mut::<&mut Segmentation>() {
        if seg.vector_contours.dirty {
            // println!("SYNC: Baking vectors to TSDF...");
            // If TSDF is missing, create it
            if seg.tsdf.is_none() {
                 // We need a default truncation. 
                 // Ideally this comes from config or is inferred. 
                 // Using 4.0 (voxels) as a standard safe value.
                 seg.tsdf = Some(ChunkedTSDF::new(4.0));
            }

            if let Some(ref mut tsdf) = seg.tsdf {
                baking::bake_contours_to_tsdf(tsdf, &seg.vector_contours.contours);
                seg.vector_contours.dirty = false;
            }
        }
    }
}

/// System to synchronize Labelmap voxel data to 2D contours
pub fn sys_sync_labelmap_to_contours(world: &mut World) {
    let mut to_update = Vec::new();
    for (entity, seg) in world.query_mut::<&mut Segmentation>() {
        if !seg.vector_contours.contours.is_empty() {
            continue;
        }

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

/// System to project 3D authoritative vectors into 2D contours
pub fn sys_sync_vector_to_contours(world: &mut World) {
    let mut spacing = [1.0, 1.0, 1.0];
    if let Some((_, vol)) = world.query::<&VolumeData>().iter().next() {
        spacing = vol.spacing;
    }

    for (_, (seg, labelmap)) in world.query_mut::<(&mut Segmentation, &LabelmapData)>() {
        if seg.vector_contours.contours.is_empty() {
            continue;
        }

        let dims = labelmap.dimensions;
        let dims_f = Vec3::new(dims[0] as f32, dims[1] as f32, dims[2] as f32);

        // Clear OLD projections (from legacy or previous frame) to ensure authority
        seg.contour_set.clear();

        let mut new_projections = Vec::new();

        for contour in &seg.vector_contours.contours {
            // Find which orthogonal slices this contour intersects or is parallel to.
            // 1. Axial (XY)
            let axial_normal = Vec3::Z;
            let axial_right = Vec3::X;
            let axial_up = Vec3::Y;

            // If parallel to Axial, it's at a specific Z
            if contour.normal.dot(axial_normal).abs() > 0.99 {
                let spacing_z = spacing[2];
                let z_idx = (contour.origin.z / spacing_z - 0.5).round() as i32;
                if z_idx >= 0 && z_idx < dims[2] as i32 {
                    if let Some(projection::ProjectedResult::Full(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new(0.0, 0.0, (z_idx as f32 + 0.5) * spacing_z),
                            axial_normal,
                            axial_right,
                            axial_up,
                        )
                    {
                        // Normalize points to 0..1 for rendering
                        let norm_points = points
                            .into_iter()
                            .map(|p| {
                                glam::Vec2::new(
                                    (p.x / spacing[0]) / dims_f.x,
                                    (p.y / spacing[1]) / dims_f.y,
                                )
                            })
                            .collect();

                        new_projections.push((
                            ViewMode::Axial,
                            z_idx,
                            Contour {
                                points: norm_points,
                                is_closed: contour.is_closed,
                                segment_id: contour.id,
                                label_index: contour.label_index,
                            },
                        ));
                    }
                }
            } else {
                // Intersects Axial slices.
                let spacing_z = spacing[2];
                let z_start = ((contour.origin.z - contour.influence) / spacing_z).floor() as i32;
                let z_end = ((contour.origin.z + contour.influence) / spacing_z).ceil() as i32;

                for z_idx in z_start..=z_end {
                    if z_idx < 0 || z_idx >= dims[2] as i32 {
                        continue;
                    }

                    if let Some(projection::ProjectedResult::Intersections(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new(0.0, 0.0, (z_idx as f32 + 0.5) * spacing_z),
                            axial_normal,
                            axial_right,
                            axial_up,
                        )
                    {
                        for p in points {
                            let norm_p = glam::Vec2::new(
                                (p.x / spacing[0]) / dims_f.x,
                                (p.y / spacing[1]) / dims_f.y,
                            );
                            new_projections.push((
                                ViewMode::Axial,
                                z_idx,
                                Contour {
                                    points: vec![
                                        norm_p + glam::Vec2::new(-0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, 0.003),
                                        norm_p + glam::Vec2::new(-0.003, 0.003),
                                    ],
                                    is_closed: true,
                                    segment_id: uuid::Uuid::new_v4(), // Dot box needs unique ID to avoid deduplication
                                    label_index: contour.label_index,
                                },
                            ));
                        }
                    }
                }
            }

            // 2. Coronal (XZ)
            let coronal_normal = Vec3::Y;
            let coronal_right = Vec3::X;
            let coronal_up = Vec3::Z;
            if contour.normal.dot(coronal_normal).abs() > 0.99 {
                let spacing_y = spacing[1];
                let y_idx = (contour.origin.y / spacing_y - 0.5).round() as i32;
                if y_idx >= 0 && y_idx < dims[1] as i32 {
                    if let Some(projection::ProjectedResult::Full(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new(0.0, (y_idx as f32 + 0.5) * spacing_y, 0.0),
                            coronal_normal,
                            coronal_right,
                            coronal_up,
                        )
                    {
                        let norm_points = points
                            .into_iter()
                            .map(|p| {
                                glam::Vec2::new(
                                    (p.x / spacing[0]) / dims_f.x,
                                    (p.y / spacing[2]) / dims_f.z,
                                )
                            })
                            .collect();
                        new_projections.push((
                            ViewMode::Coronal,
                            y_idx,
                            Contour {
                                points: norm_points,
                                is_closed: contour.is_closed,
                                segment_id: contour.id,
                                label_index: contour.label_index,
                            },
                        ));
                    }
                }
            } else {
                // Intersects Coronal
                let spacing_y = spacing[1];
                let y_start = ((contour.origin.y - contour.influence) / spacing_y).floor() as i32;
                let y_end = ((contour.origin.y + contour.influence) / spacing_y).ceil() as i32;
                for y_idx in y_start..=y_end {
                    if y_idx < 0 || y_idx >= dims[1] as i32 {
                        continue;
                    }
                    if let Some(projection::ProjectedResult::Intersections(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new(0.0, (y_idx as f32 + 0.5) * spacing_y, 0.0),
                            coronal_normal,
                            coronal_right,
                            coronal_up,
                        )
                    {
                        for p in points {
                            let norm_p = glam::Vec2::new(
                                (p.x / spacing[0]) / dims_f.x,
                                (p.y / spacing[2]) / dims_f.z,
                            );
                            new_projections.push((
                                ViewMode::Coronal,
                                y_idx,
                                Contour {
                                    points: vec![
                                        norm_p + glam::Vec2::new(-0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, 0.003),
                                        norm_p + glam::Vec2::new(-0.003, 0.003),
                                    ],
                                    is_closed: true,
                                    segment_id: uuid::Uuid::new_v4(), // Dot box needs unique ID
                                    label_index: contour.label_index,
                                },
                            ));
                        }
                    }
                }
            }

            // 3. Sagittal (YZ)
            let sagittal_normal = Vec3::X;
            let sagittal_right = Vec3::Y;
            let sagittal_up = Vec3::Z;
            if contour.normal.dot(sagittal_normal).abs() > 0.99 {
                let spacing_x = spacing[0];
                let x_idx = (contour.origin.x / spacing_x - 0.5).round() as i32;
                if x_idx >= 0 && x_idx < dims[0] as i32 {
                    if let Some(projection::ProjectedResult::Full(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new((x_idx as f32 + 0.5) * spacing_x, 0.0, 0.0),
                            sagittal_normal,
                            sagittal_right,
                            sagittal_up,
                        )
                    {
                        let norm_points = points
                            .into_iter()
                            .map(|p| {
                                glam::Vec2::new(
                                    (p.x / spacing[1]) / dims_f.y,
                                    (p.y / spacing[2]) / dims_f.z,
                                )
                            })
                            .collect();
                        new_projections.push((
                            ViewMode::Sagittal,
                            x_idx,
                            Contour {
                                points: norm_points,
                                is_closed: contour.is_closed,
                                segment_id: contour.id,
                                label_index: contour.label_index,
                            },
                        ));
                    }
                }
            } else {
                // Intersects Sagittal
                let spacing_x = spacing[0];
                let x_start = ((contour.origin.x - contour.influence) / spacing_x).floor() as i32;
                let x_end = ((contour.origin.x + contour.influence) / spacing_x).ceil() as i32;
                for x_idx in x_start..=x_end {
                    if x_idx < 0 || x_idx >= dims[0] as i32 {
                        continue;
                    }
                    if let Some(projection::ProjectedResult::Intersections(points)) =
                        projection::project_to_plane(
                            contour,
                            Vec3::new((x_idx as f32 + 0.5) * spacing_x, 0.0, 0.0),
                            sagittal_normal,
                            sagittal_right,
                            sagittal_up,
                        )
                    {
                        for p in points {
                            let norm_p = glam::Vec2::new(
                                (p.x / spacing[1]) / dims_f.y,
                                (p.y / spacing[2]) / dims_f.z,
                            );
                            new_projections.push((
                                ViewMode::Sagittal,
                                x_idx,
                                Contour {
                                    points: vec![
                                        norm_p + glam::Vec2::new(-0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, -0.003),
                                        norm_p + glam::Vec2::new(0.003, 0.003),
                                        norm_p + glam::Vec2::new(-0.003, 0.003),
                                    ],
                                    is_closed: true,
                                    segment_id: uuid::Uuid::new_v4(), // Dot box needs unique ID
                                    label_index: contour.label_index,
                                },
                            ));
                        }
                    }
                }
            }
        }

        for (mode, idx, c) in new_projections {
            append_contour_to_slice(seg, mode, idx, c);
        }
    }
}

fn append_contour_to_slice(seg: &mut Segmentation, mode: ViewMode, idx: i32, contour: Contour) {
    if let Some(slice) = seg.contour_set.slices.get_mut(&(mode, idx)) {
        // Avoid duplicates if already projected
        if !slice
            .contours
            .iter()
            .any(|c| c.segment_id == contour.segment_id)
        {
            slice.contours.push(contour);
        }
    } else {
        seg.contour_set.update_slice(mode, idx, vec![contour]);
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
                // Correctly map voxel-center to centered UV
                let voxel_x = min[0] as f32 + p.x * (w as f32 - 1.0);
                let voxel_y = min[1] as f32 + p.y * (h as f32 - 1.0);
                p.x = (voxel_x + 0.5) / dims[0] as f32;
                p.y = (voxel_y + 0.5) / dims[1] as f32;
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
                let voxel_x = min[0] as f32 + p.x * (w as f32 - 1.0);
                let voxel_z = min[2] as f32 + p.y * (h as f32 - 1.0);
                p.x = (voxel_x + 0.5) / dims[0] as f32;
                p.y = (voxel_z + 0.5) / dims[2] as f32;
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
                let voxel_y = min[1] as f32 + p.x * (w as f32 - 1.0);
                let voxel_z = min[2] as f32 + p.y * (h as f32 - 1.0);
                p.x = (voxel_y + 0.5) / dims[1] as f32;
                p.y = (voxel_z + 0.5) / dims[2] as f32;
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
        let mut contours = ms.extract(&slice_data, (dims[0], dims[1]), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (p.x * (dims[0] as f32 - 1.0) + 0.5) / dims[0] as f32;
                p.y = (p.y * (dims[1] as f32 - 1.0) + 0.5) / dims[1] as f32;
            }
        }
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
        let mut contours = ms.extract(&slice_data, (dims[0], dims[2]), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (p.x * (dims[0] as f32 - 1.0) + 0.5) / dims[0] as f32;
                p.y = (p.y * (dims[2] as f32 - 1.0) + 0.5) / dims[2] as f32;
            }
        }
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
        let mut contours = ms.extract(&slice_data, (dims[1], dims[2]), 1);
        for c in &mut contours {
            for p in &mut c.points {
                p.x = (p.x * (dims[1] as f32 - 1.0) + 0.5) / dims[1] as f32;
                p.y = (p.y * (dims[2] as f32 - 1.0) + 0.5) / dims[2] as f32;
            }
        }
        if !contours.is_empty() {
            seg.contour_set
                .update_slice(ViewMode::Sagittal, x as i32, contours);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::MainVolumeTag;
    use crate::segmentation::contour::SpatialContour;
    use crate::segmentation::VectorContourSet;
    use glam::{Vec2, Vec3};
    use uuid::Uuid;

    #[test]
    fn test_vector_to_uv_projection() {
        // Setup a mock segmentation with one vector contour
        let mut seg = Segmentation {
            name: "test".into(),
            is_visible: true,
            contour_set: crate::segmentation::ContourSet::new(),
            vector_contours: VectorContourSet::new(),
            mesh: crate::segmentation::mesh::SegmentationMesh::default(),
            gpu_mesh: None,
            tsdf: None,
            mesher: None,
            last_mesh_update: None,
        };

        // Create a horizontal contour in Axial physical space at Z=62.0 (voxel index 15.5 * 4.0 spacing)
        let spacing = [1.0, 2.0, 4.0];
        let origin = Vec3::new(0.0, 0.0, 15.5 * 4.0);
        seg.vector_contours.contours.push(SpatialContour {
            id: Uuid::new_v4(),
            origin,
            normal: Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            points: vec![Vec2::new(15.5 * 1.0, 15.5 * 2.0)], // physical point at voxel center (15.5, 15.5)
            influence: 1.0,
            is_closed: false,
            label_index: 1,
        });

        // Setup world with components
        let mut world = World::new();
        let vol_data = VolumeData {
            dimensions: [32, 32, 32],
            spacing,
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let labelmap = LabelmapData {
            dimensions: [32, 32, 32],
            raw_data: vec![0; 32 * 32 * 32],
        };

        world.spawn((seg, labelmap));
        let vol_ent = world.spawn((vol_data,));
        world.insert(vol_ent, (MainVolumeTag,)).unwrap();

        // Run sync
        sys_sync_vector_to_contours(&mut world);

        // Verify result
        let (_, seg) = world
            .query_mut::<&Segmentation>()
            .into_iter()
            .next()
            .unwrap();
        let axial_slices = &seg.contour_set.slices;

        // Should find a projection at slice index 15
        let slice_15 = axial_slices.get(&(ViewMode::Axial, 15));
        assert!(
            slice_15.is_some(),
            "Projection should exist at Axial slice 15"
        );

        if let Some(slice) = slice_15 {
            // The point (15.5, 15.5) in a 32-voxel volume should be at UV (15.5 + 0.5 - 0.5??)
            // Wait, the formula in sync.rs is:
            // p.x = (voxel_x + 0.5) / dims[0]
            // My point in handoff was p.x = 15.0 * 1.0 (relative to origin 0.0)
            // So voxel_x = 15.5
            // uv_x = (15.5) / 32.0 ??? No.

            // Let's check sync.rs implementation:
            // let norm_p = glam::Vec2::new((p.x / spacing[0]) / dims_f.x, (p.y / spacing[1]) / dims_f.y);
            // p.x here is world_p - slice_origin.
            // world_p = origin + pt = (0,0,62) + (15.5, 31.0) = (15.5, 31.0, 62.0)
            // slice_origin = (0, 0, (15+0.5)*4.0) = (0, 0, 62.0)
            // local_v = (15.5, 31.0, 0.0)
            // p.x (local) = 15.5, p.y (local) = 31.0
            // uv_x = (15.5 / 1.0) / 32.0 = 15.5 / 32.0
            // uv_y = (31.0 / 2.0) / 32.0 = 15.5 / 32.0

            // Wait, (15.5 / 32.0) is exactly the center of the voxel (index 15) in UV space?
            // Voxel index 0 is center 0.5/32.
            // Voxel index 15 is center 15.5/32.
            // YES! This matches the shader's expectation.

            let uv = slice.contours[0].points[0]; // First point of first contour (dot box top-left/whatever)
                                                  // Actually, for Intersections, it's a dot box. The center of the dot box is norm_p.
                                                  // Let's re-read sync.rs Intersections logic:
                                                  // let norm_p = glam::Vec2::new((p.x / spacing[0]) / dims_f.x, (p.y / spacing[1]) / dims_f.y);
                                                  // new_projections.push(Contour { points: vec![norm_p + offset, ... ] })

            // So the center should be (15.5/32, 15.5/32)
            assert!((uv.x - 15.5 / 32.0).abs() < 0.01);
            assert!((uv.y - 15.5 / 32.0).abs() < 0.01);
        }
    }
}
