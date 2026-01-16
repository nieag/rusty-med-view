// src/systems/paint.rs
use crate::components::*;
use crate::systems::picking::get_voxel_at_mouse;
use hecs::World;

/// Paint System: Updates labelmap based on mouse stroke
pub fn sys_paint(world: &mut World, entities: &AppEntities, queue: &wgpu::Queue) {
    let mut interactions = Vec::new();

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        if input.is_dragging && !input.egui_wants_input && !input.is_panning && !input.is_rotating {
            if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
                if editor.active_tool != EditorTool::Navigation {
                    interactions.push((
                        input.active_viewport,
                        input.mouse_uv,
                        editor.active_tool,
                        editor.brush_size,
                        editor.active_label_index,
                        editor.active_layer,
                        editor.last_paint_voxel,
                    ));
                }
            }
        }
    }

    if interactions.is_empty() {
        return;
    }

    // Cache volume dimensions for voxel conversion
    let mut vol_dims = [100, 100, 100];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_dims = vol.dimensions;
    }

    for (viewport_entity, mouse_uv, tool, brush_size, label_idx, layer_entity, last_voxel_opt) in
        interactions
    {
        let viewport_entity = if let Some(e) = viewport_entity {
            e
        } else {
            continue;
        };

        let mode = if let Ok(vp) = world.get::<&Viewport>(viewport_entity) {
            vp.mode
        } else {
            continue;
        };

        let layer_ent = if let Some(e) = layer_entity {
            e
        } else {
            continue;
        };
        let cur_target_opt = get_voxel_at_mouse(world, entities, viewport_entity, mouse_uv);

        if let Some(pos) = cur_target_opt {
            if let Ok(mut query) =
                world.query_one::<(&mut LabelmapData, &Representation)>(layer_ent)
            {
                if let Some((label_data, repr)) = query.get() {
                    let res = if let Representation::Voxel(res) = repr {
                        res
                    } else {
                        continue;
                    };
                    let dim = label_data.dimensions;
                    let width = dim[0];
                    let height = dim[1];
                    let depth = dim[2];

                    let cx = (pos[0] * width as f32) as i32;
                    let cy = (pos[1] * height as f32) as i32;
                    let cz = (pos[2] * depth as f32) as i32;
                    let current_voxel = [cx, cy, cz];

                    if mode == ViewMode::ThreeD {
                        continue;
                    }

                    let points_to_paint = get_stroke_points(current_voxel, last_voxel_opt, mode);

                    let val_to_write = if tool == EditorTool::Eraser {
                        0
                    } else {
                        label_idx
                    };

                    let mut min_bound = [width as i32, height as i32, depth as i32];
                    let mut max_bound = [-1, -1, -1];
                    let mut modified = false;

                    for pt in points_to_paint {
                        if apply_brush_step(
                            pt,
                            mode,
                            brush_size,
                            val_to_write,
                            label_data,
                            &mut modified,
                            &mut min_bound,
                            &mut max_bound,
                        ) {
                            // modified updated in call
                        }
                    }

                    if modified {
                        update_gpu_texture(
                            queue,
                            &res.texture,
                            label_data,
                            min_bound,
                            max_bound,
                            width,
                            height,
                        );
                    }
                }
            }
        }

        if let Some(target) = cur_target_opt {
            if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                editor.last_paint_voxel = Some([
                    (target[0] * vol_dims[0] as f32) as u32,
                    (target[1] * vol_dims[1] as f32) as u32,
                    (target[2] * vol_dims[2] as f32) as u32,
                ]);
            }
        }
    }
}

/// Helper to get interpolates points between current and last voxel.
fn get_stroke_points(
    current: [i32; 3],
    last_opt: Option<[u32; 3]>,
    mode: ViewMode,
) -> Vec<[i32; 3]> {
    let mut points = Vec::new();
    let cx = current[0];
    let cy = current[1];
    let cz = current[2];

    if let Some(last_voxel) = last_opt {
        let lx = last_voxel[0] as i32;
        let ly = last_voxel[1] as i32;
        let lz = last_voxel[2] as i32;
        let dist = match mode {
            ViewMode::Axial => (((cx - lx).pow(2) + (cy - ly).pow(2)) as f32).sqrt(),
            ViewMode::Coronal => (((cx - lx).pow(2) + (cz - lz).pow(2)) as f32).sqrt(),
            ViewMode::Sagittal => (((cy - ly).pow(2) + (cz - lz).pow(2)) as f32).sqrt(),
            _ => 0.0,
        };
        let steps = dist.ceil() as i32 + 1;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (ix, iy, iz) = match mode {
                ViewMode::Axial => (
                    (lx as f32 + (cx - lx) as f32 * t) as i32,
                    (ly as f32 + (cy - ly) as f32 * t) as i32,
                    cz,
                ),
                ViewMode::Coronal => (
                    (lx as f32 + (cx - lx) as f32 * t) as i32,
                    cy,
                    (lz as f32 + (cz - lz) as f32 * t) as i32,
                ),
                ViewMode::Sagittal => (
                    cx,
                    (ly as f32 + (cy - ly) as f32 * t) as i32,
                    (lz as f32 + (cz - lz) as f32 * t) as i32,
                ),
                _ => (cx, cy, cz),
            };
            points.push([ix, iy, iz]);
        }
    } else {
        points.push(current);
    }
    points
}

/// Applies a single brush step to the labelmap data.
fn apply_brush_step(
    pt: [i32; 3],
    mode: ViewMode,
    brush_size: f32,
    val: u8,
    label_data: &mut LabelmapData,
    modified: &mut bool,
    min_bound: &mut [i32; 3],
    max_bound: &mut [i32; 3],
) -> bool {
    let px = pt[0];
    let py = pt[1];
    let pz = pt[2];
    let r = brush_size as i32;
    let r2 = (brush_size * brush_size) as i32;
    let width = label_data.dimensions[0];
    let height = label_data.dimensions[1];
    let depth = label_data.dimensions[2];

    match mode {
        ViewMode::Axial => {
            let start_x = (px - r).max(0);
            let end_x = (px + r).min(width as i32 - 1);
            let start_y = (py - r).max(0);
            let end_y = (py + r).min(height as i32 - 1);
            if pz >= 0 && pz < depth as i32 {
                for y in start_y..=end_y {
                    for x in start_x..=end_x {
                        if (x - px).pow(2) + (y - py).pow(2) <= r2 {
                            let idx =
                                (pz as u32 * width * height + y as u32 * width + x as u32) as usize;
                            if label_data.raw_data[idx] != val {
                                label_data.raw_data[idx] = val;
                                *modified = true;
                                update_bounds(min_bound, max_bound, x, y, pz);
                            }
                        }
                    }
                }
            }
        }
        ViewMode::Coronal => {
            let start_x = (px - r).max(0);
            let end_x = (px + r).min(width as i32 - 1);
            let start_z = (pz - r).max(0);
            let end_z = (pz + r).min(depth as i32 - 1);
            if py >= 0 && py < height as i32 {
                for z in start_z..=end_z {
                    for x in start_x..=end_x {
                        if (x - px).pow(2) + (z - pz).pow(2) <= r2 {
                            let idx =
                                (z as u32 * width * height + py as u32 * width + x as u32) as usize;
                            if label_data.raw_data[idx] != val {
                                label_data.raw_data[idx] = val;
                                *modified = true;
                                update_bounds(min_bound, max_bound, x, py, z);
                            }
                        }
                    }
                }
            }
        }
        ViewMode::Sagittal => {
            let start_y = (py - r).max(0);
            let end_y = (py + r).min(height as i32 - 1);
            let start_z = (pz - r).max(0);
            let end_z = (pz + r).min(depth as i32 - 1);
            if px >= 0 && px < width as i32 {
                for z in start_z..=end_z {
                    for y in start_y..=end_y {
                        if (y - py).pow(2) + (z - pz).pow(2) <= r2 {
                            let idx =
                                (z as u32 * width * height + y as u32 * width + px as u32) as usize;
                            if label_data.raw_data[idx] != val {
                                label_data.raw_data[idx] = val;
                                *modified = true;
                                update_bounds(min_bound, max_bound, px, y, z);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    *modified
}

fn update_bounds(min_bound: &mut [i32; 3], max_bound: &mut [i32; 3], x: i32, y: i32, z: i32) {
    min_bound[0] = min_bound[0].min(x);
    min_bound[1] = min_bound[1].min(y);
    min_bound[2] = min_bound[2].min(z);
    max_bound[0] = max_bound[0].max(x);
    max_bound[1] = max_bound[1].max(y);
    max_bound[2] = max_bound[2].max(z);
}

fn update_gpu_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    label_data: &LabelmapData,
    min_bound: [i32; 3],
    max_bound: [i32; 3],
    width: u32,
    height: u32,
) {
    let w = (max_bound[0] - min_bound[0] + 1) as u32;
    let h = (max_bound[1] - min_bound[1] + 1) as u32;
    let d = (max_bound[2] - min_bound[2] + 1) as u32;
    let mut sub_data = Vec::with_capacity((w * h * d) as usize);
    for z in min_bound[2]..=max_bound[2] {
        for y in min_bound[1]..=max_bound[1] {
            let start =
                (z as u32 * width * height + y as u32 * width + min_bound[0] as u32) as usize;
            sub_data.extend_from_slice(&label_data.raw_data[start..start + w as usize]);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: min_bound[0] as u32,
                y: min_bound[1] as u32,
                z: min_bound[2] as u32,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &sub_data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: d,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_stroke_points_single() {
        let current = [10, 10, 10];
        let points = get_stroke_points(current, None, ViewMode::Axial);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], current);
    }

    #[test]
    fn test_get_stroke_points_interpolated() {
        let current = [20, 10, 10];
        let last = Some([10, 10, 10]);
        let points = get_stroke_points(current, last, ViewMode::Axial);
        assert!(points.len() > 1);
        assert_eq!(points[0], [10, 10, 10]);
        assert_eq!(points.last().unwrap(), &current);
    }

    #[test]
    fn test_apply_brush_step() {
        let mut data = LabelmapData {
            dimensions: [100, 100, 100],
            raw_data: vec![0; 100 * 100 * 100],
        };
        let mut modified = false;
        let mut min = [100, 100, 100];
        let mut max = [-1, -1, -1];

        apply_brush_step(
            [50, 50, 50],
            ViewMode::Axial,
            2.0,
            1,
            &mut data,
            &mut modified,
            &mut min,
            &mut max,
        );

        assert!(modified);
        // Check center
        let idx = (50 * 100 * 100 + 50 * 100 + 50) as usize;
        assert_eq!(data.raw_data[idx], 1);
        // Check bounds
        assert!(min[0] <= 50);
        assert!(max[0] >= 50);
    }
}
