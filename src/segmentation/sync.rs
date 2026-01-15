use crate::app::components::*;
use crate::segmentation::algorithms::MarchingSquares;
use hecs::World;

/// System to synchronize Labelmap voxel data to 2D contours
pub fn sys_sync_labelmap_to_contours(world: &mut World) {
    let mut to_update = Vec::new();

    // Find all segmentations that have a labelmap and might need update
    for (entity, (seg, _labelmap)) in world.query_mut::<(&mut Segmentation, &LabelmapData)>() {
        // For now, let's sync if the contour set is empty or we can add a dirty flag to LabelmapData later.
        // To be safe and performant, we only sync the slices currently visible in the viewports?
        // Or just sync everything once for now.

        if seg.contour_set.slices.is_empty() {
            to_update.push(entity);
        }
    }

    for entity in to_update {
        if let Ok(mut seg) = world.get::<&mut Segmentation>(entity) {
            if let Ok(labelmap) = world.get::<&LabelmapData>(entity) {
                sync_full_labelmap(&mut seg, &*labelmap);
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
