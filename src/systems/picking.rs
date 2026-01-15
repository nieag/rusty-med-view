// src/systems/picking.rs
use crate::components::*;
use glam::Vec3;
use hecs::World;

/// Get the HU intensity value at the current mouse position.
pub fn get_hu_at_mouse(world: &World, entities: &AppEntities) -> Option<f32> {
    let (viewport_entity, mouse_uv) = {
        if let Ok(input) = world.get::<&InputState>(entities.input) {
            (input.active_viewport, input.mouse_uv)
        } else {
            (None, [0.5, 0.5])
        }
    };

    let voxel_pos = get_voxel_at_mouse(world, entities, viewport_entity?, mouse_uv)?;

    let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    if let Some((_, vol)) = query.iter().next() {
        let [w, h, d] = vol.dimensions;
        if w == 0 || h == 0 || d == 0 {
            return None;
        }
        let x = ((voxel_pos[0] * w as f32) as u32).min(w - 1);
        let y = ((voxel_pos[1] * h as f32) as u32).min(h - 1);
        let z = ((voxel_pos[2] * d as f32) as u32).min(d - 1);

        let idx = (z * h * w + y * w + x) as usize;
        if idx < vol.intensities.len() {
            return Some(vol.intensities[idx]);
        }
    }
    None
}

/// Calculate volume coordinate (0.0..1.0) under the mouse cursor.
pub fn get_voxel_at_mouse(
    world: &World,
    entities: &AppEntities,
    viewport_entity: hecs::Entity,
    mouse_uv: [f32; 2],
) -> Option<[f32; 3]> {
    let (zoom, pan, user_rotation, mode) = if let (Ok(vp), Ok(vs)) = (
        world.get::<&Viewport>(viewport_entity),
        world.get::<&ViewportState>(viewport_entity),
    ) {
        (vs.zoom, vs.pan, vs.user_rotation, vp.mode)
    } else {
        (1.0, [0.0, 0.0], [0.0, 0.0, 0.0, 1.0], ViewMode::ThreeD)
    };

    let viewport_rect = world
        .get::<&WindowSettings>(entities.window_settings)
        .map(|w| w.viewport_rect)
        .unwrap_or([0.0, 0.0, 100.0, 100.0]);

    let (vol_aspects, vol_dims) = {
        let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
        if let Some((_, vol)) = query.iter().next() {
            (vol.aspect_ratios(), Some(vol.dimensions))
        } else {
            ([1.0, 1.0, 1.0], None)
        }
    };

    let cursor_pos = world
        .get::<&Transform>(entities.cursor)
        .map(|t| t.position)
        .unwrap_or([0.0, 0.0, 0.0]);

    if mode != ViewMode::ThreeD {
        // --- 2D Slices ---
        let screen_w = viewport_rect[2];
        let screen_h = viewport_rect[3];
        let screen_aspect = if screen_h > 0.0 {
            screen_w / screen_h
        } else {
            1.0
        };
        let plane = crate::orientation::SlicePlane::from_mode(mode)?;
        let slice_aspect = plane.slice_aspect(vol_aspects);
        let k = screen_aspect / slice_aspect;

        let volume_uv = [
            ((mouse_uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
            (mouse_uv[1] - 0.5) / zoom + 0.5 + pan[1],
        ];

        let pos = plane.screen_uv_to_volume(volume_uv, cursor_pos[plane.depth_axis()]);
        if (0.0..=1.0).contains(&pos[0])
            && (0.0..=1.0).contains(&pos[1])
            && (0.0..=1.0).contains(&pos[2])
        {
            return Some(pos);
        }
        None
    } else {
        // --- 3D View ---
        let mut aspect = 1.0;
        if viewport_rect[3] > 0.0 {
            aspect = viewport_rect[2] / viewport_rect[3];
        }

        if let Some([width, height, depth]) = vol_dims {
            // --- Phase 3: Centralized Picking ---
            let mut data_orientation = [0.0f32, 0.0, 0.0, 1.0];
            let mut vol_aspects = [1.0f32; 3];
            for (_, vol) in world.query::<&VolumeData>().iter() {
                data_orientation = vol.orientation;
                vol_aspects = vol.aspect_ratios();
            }

            let final_rotation =
                crate::orientation::compose_view_rotation(data_orientation, user_rotation);
            let (cam_pos_obj_raw, ray_dir_obj_raw) =
                crate::orientation::screen_to_ray_3d(mouse_uv, final_rotation, zoom, pan, aspect);

            let cam_pos_obj = Vec3::from(cam_pos_obj_raw);
            let ray_dir_obj = Vec3::from(ray_dir_obj_raw);

            let aspect_ratios = vol_aspects;
            let half_ar = [
                aspect_ratios[0] * 0.5,
                aspect_ratios[1] * 0.5,
                aspect_ratios[2] * 0.5,
            ];

            let min_bound = Vec3::from([-half_ar[0], -half_ar[1], -half_ar[2]]);
            let max_bound = Vec3::from([half_ar[0], half_ar[1], half_ar[2]]);

            if let Some(t_entry) = intersect_aabb(
                cam_pos_obj.into(),
                ray_dir_obj.into(),
                min_bound.into(),
                max_bound.into(),
            ) {
                let mut t_exit = f32::INFINITY;
                for i in 0..3 {
                    if ray_dir_obj[i].abs() > f32::EPSILON {
                        let t1 = (min_bound[i] - cam_pos_obj[i]) / ray_dir_obj[i];
                        let t2 = (max_bound[i] - cam_pos_obj[i]) / ray_dir_obj[i];
                        t_exit = t_exit.min(t1.max(t2));
                    }
                }

                let mut best_t = t_entry;
                let mut max_density = 0u8;

                for (_, vol) in world.query::<&VolumeData>().iter() {
                    let steps = 64;
                    let step_size = (t_exit - t_entry) / steps as f32;
                    for i in 0..steps {
                        let t = t_entry + step_size * i as f32;
                        let p = [
                            cam_pos_obj[0] + ray_dir_obj[0] * t,
                            cam_pos_obj[1] + ray_dir_obj[1] * t,
                            cam_pos_obj[2] + ray_dir_obj[2] * t,
                        ];
                        let uvw = [
                            (p[0] / aspect_ratios[0]) + 0.5,
                            (p[1] / aspect_ratios[1]) + 0.5,
                            (p[2] / aspect_ratios[2]) + 0.5,
                        ];
                        let ix = (uvw[0] * width as f32) as i32;
                        let iy = (uvw[1] * height as f32) as i32;
                        let iz = (uvw[2] * depth as f32) as i32;
                        if ix >= 0
                            && ix < width as i32
                            && iy >= 0
                            && iy < height as i32
                            && iz >= 0
                            && iz < depth as i32
                        {
                            let idx = (iz as u32 * height * width + iy as u32 * width + ix as u32)
                                as usize;
                            if let Some(&intensity) = vol.intensities.get(idx) {
                                let d = (intensity * 255.0) as u8;
                                if d > max_density {
                                    max_density = d;
                                    best_t = t;
                                    if d >= 250 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                let final_t = if max_density > 20 { best_t } else { t_entry };
                let hit_point = [
                    cam_pos_obj[0] + ray_dir_obj[0] * final_t,
                    cam_pos_obj[1] + ray_dir_obj[1] * final_t,
                    cam_pos_obj[2] + ray_dir_obj[2] * final_t,
                ];

                return Some([
                    ((hit_point[0] / aspect_ratios[0]) + 0.5).clamp(0.0, 1.0),
                    ((hit_point[1] / aspect_ratios[1]) + 0.5).clamp(0.0, 1.0),
                    ((hit_point[2] / aspect_ratios[2]) + 0.5).clamp(0.0, 1.0),
                ]);
            }
        }
        None
    }
}

pub fn intersect_aabb(
    origin: [f32; 3],
    dir: [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
) -> Option<f32> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < f32::EPSILON {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
        } else {
            let t1 = (min[i] - origin[i]) / dir[i];
            let t2 = (max[i] - origin[i]) / dir[i];
            let (tm1, tm2) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_min = t_min.max(tm1);
            t_max = t_max.min(tm2);
        }
    }
    if t_min <= t_max && t_max >= 0.0 {
        Some(t_min.max(0.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intersect_aabb_direct_hit() {
        let result = intersect_aabb(
            [0.0, 0.0, -2.0],
            [0.0, 0.0, 1.0],
            [-1.0, -1.0, -1.0],
            [1.0, 1.0, 1.0],
        );
        assert!((result.unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_intersect_aabb_miss() {
        let result = intersect_aabb(
            [5.0, 0.0, -2.0],
            [0.0, 0.0, 1.0],
            [-1.0, -1.0, -1.0],
            [1.0, 1.0, 1.0],
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_intersect_aabb_inside() {
        let result = intersect_aabb(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, -1.0, -1.0],
            [1.0, 1.0, 1.0],
        );
        assert_eq!(result, Some(0.0));
    }

    #[test]
    fn test_intersect_aabb_glancing_edge() {
        let result = intersect_aabb(
            [1.0, 1.0, -2.0],
            [0.0, 0.0, 1.0],
            [-1.0, -1.0, -1.0],
            [1.0, 1.0, 1.0],
        );
        assert!(result.is_some());
        assert!((result.unwrap() - 1.0).abs() < 1e-6);
    }
}
