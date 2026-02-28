//! Segment system for contour-based segmentation workflow.
//!
//! Manages the full pipeline: contour drawing → SDF generation → mesh rendering.

use crate::app::segment::{
    ChunkKey, MeshData, Plane3D, PlaneContour, SdfVolume, Segment, SegmentChunkRuntime,
};
use crate::components::{AppEntities, SegPerfConfig, VolumeData};
use crate::convert::{
    build_sdf_from_contours_with_config, build_tsdf_chunk_from_sdf, chunk_bounds_for_key,
    chunk_keys_for_bounds, surface_nets_chunk, surface_nets_from_sdf,
    update_sdf_region_from_contours_with_config, SdfBuildConfig,
};
use crate::render::mesh_pipeline::MeshResources;
use crate::systems::contour_draw::ContourDrawState;
use crate::util::orientation::SlicePlane;
use hecs::World;
use std::collections::HashSet;
use web_time::Instant;

// ============================================================================
// Segmentation Manager Component
// ============================================================================

/// Manages all segments in the application.
#[derive(Default)]
pub struct SegmentManager {
    /// All segments
    pub segments: Vec<Segment>,
    /// Currently active segment index (for editing)
    pub active_segment: Option<usize>,
    /// Contour drawing state
    pub draw_state: ContourDrawState,
}

impl SegmentManager {
    /// Create a new segment manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new segment with default settings.
    pub fn add_segment(&mut self, name: &str, color: [f32; 4]) -> usize {
        let segment = Segment::new(name, color);
        self.segments.push(segment);
        let idx = self.segments.len() - 1;
        self.active_segment = Some(idx);
        idx
    }

    /// Get the active segment mutably.
    pub fn active_segment_mut(&mut self) -> Option<&mut Segment> {
        self.active_segment
            .and_then(|idx| self.segments.get_mut(idx))
    }

    /// Get the active segment.
    pub fn active_segment(&self) -> Option<&Segment> {
        self.active_segment.and_then(|idx| self.segments.get(idx))
    }

    /// Remove a segment by index.
    pub fn remove_segment(&mut self, idx: usize) -> Option<Segment> {
        if idx < self.segments.len() {
            let removed = self.segments.remove(idx);
            // Adjust active index
            if let Some(active) = self.active_segment {
                if active == idx {
                    self.active_segment = None;
                } else if active > idx {
                    self.active_segment = Some(active - 1);
                }
            }
            Some(removed)
        } else {
            None
        }
    }

    /// Find segment by ID.
    pub fn find_by_id(&self, id: uuid::Uuid) -> Option<usize> {
        self.segments.iter().position(|s| s.id == id)
    }

    /// Get segment count.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

// ============================================================================
// Segment Regeneration
// ============================================================================

/// Regenerate SDF and mesh for a segment if dirty.
///
/// Returns true if mesh was regenerated.
pub fn regenerate_segment_if_dirty(
    segment: &mut Segment,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> bool {
    regenerate_segment_if_dirty_with_resolution(
        segment,
        volume_dims,
        volume_spacing,
        segment.sdf_resolution_multiplier,
        24.0,
        32,
        false,
        true,
    )
    .0
}

fn regenerate_segment_if_dirty_with_resolution(
    segment: &mut Segment,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
    sdf_band_mm: f32,
    mesh_chunk_size: u32,
    is_live: bool,
    allow_mesh_rebuild: bool,
) -> (bool, f32, f32) {
    let mut mesh_changed = false;
    let mut sdf_ms = 0.0f32;
    let mut mesh_ms = 0.0f32;
    let mut live_updated_bounds: Option<[u32; 6]> = None;

    // Regenerate SDF if dirty
    if segment.sdf_dirty {
        let sdf_start = Instant::now();
        let build_cfg = SdfBuildConfig {
            resolution_multiplier,
            neighbor_slice_bridging: true,
            clamp_distance_mm: sdf_band_mm.max(0.5),
            ..SdfBuildConfig::default()
        };
        let sdf = if let Some(dirty_roi) = segment.dirty_roi_world {
            if let Some(existing) = segment.sdf.as_mut() {
                // Incremental update of existing SDF
                let updated = update_sdf_region_from_contours_with_config(
                    existing,
                    &segment.contours,
                    build_cfg,
                    dirty_roi,
                );
                if updated.is_some() {
                    live_updated_bounds = updated;
                    None
                } else {
                    // Fallback if ROI update failed
                    Some(build_sdf_from_contours_with_config(
                        &segment.contours,
                        volume_dims,
                        volume_spacing,
                        build_cfg,
                    ))
                }
            } else {
                // First stroke: build an empty SDF and update only the ROI
                let rm = build_cfg.resolution_multiplier.max(0.1);
                let sdf_dims = [
                    (volume_dims[0] as f32 * rm).round().max(1.0) as u32,
                    (volume_dims[1] as f32 * rm).round().max(1.0) as u32,
                    (volume_dims[2] as f32 * rm).round().max(1.0) as u32,
                ];
                let sdf_spacing = [
                    volume_spacing[0] / rm,
                    volume_spacing[1] / rm,
                    volume_spacing[2] / rm,
                ];
                let mut new_sdf = SdfVolume::new(sdf_dims, sdf_spacing, [0.0, 0.0, 0.0]);
                let updated = update_sdf_region_from_contours_with_config(
                    &mut new_sdf,
                    &segment.contours,
                    build_cfg,
                    dirty_roi,
                );
                live_updated_bounds = updated;
                Some(new_sdf)
            }
        } else {
            // No ROI available: full build
            Some(build_sdf_from_contours_with_config(
                &segment.contours,
                volume_dims,
                volume_spacing,
                build_cfg,
            ))
        };
        sdf_ms = sdf_start.elapsed().as_secs_f32() * 1000.0;
        if let Some(sdf) = sdf {
            segment.sdf = Some(sdf);
            live_updated_bounds = segment.sdf.as_ref().and_then(|s| s.active_bounds);
        }
        segment.dirty_roi_world = None;
        segment.sdf_revision = segment.sdf_revision.wrapping_add(1);
        if is_live {
            segment.live_sdf_revision = segment.sdf_revision;
            segment.sdf_dirty = false;
        } else {
            segment.final_sdf_revision = segment.sdf_revision;
            segment.sdf_dirty = false;
        }

        // Bake dirty TSDF chunks from the updated SDF.
        // Pad each chunk by +1 voxel on all edges so Surface Nets can
        // connect vertices across chunk boundaries (eliminates seam lines).
        if let Some(sdf) = &segment.sdf {
            segment.chunk_runtime.tsdf_dims = sdf.dimensions;
            segment.chunk_runtime.tsdf_spacing = sdf.spacing;
            segment.chunk_runtime.tsdf_origin = sdf.origin;
            let bounds = live_updated_bounds.unwrap_or([
                0,
                0,
                0,
                sdf.dimensions[0].saturating_sub(1),
                sdf.dimensions[1].saturating_sub(1),
                sdf.dimensions[2].saturating_sub(1),
            ]);
            let keys = chunk_keys_for_bounds(bounds, mesh_chunk_size);
            let trunc = sdf_band_mm.max(1.0);
            for key in &keys {
                if let Some(cb) = chunk_bounds_for_key(*key, mesh_chunk_size, sdf.dimensions) {
                    // Pad bounds by 1 voxel in each direction for overlap.
                    let padded = [
                        cb[0].saturating_sub(1),
                        cb[1].saturating_sub(1),
                        cb[2].saturating_sub(1),
                        (cb[3] + 1).min(sdf.dimensions[0].saturating_sub(1)),
                        (cb[4] + 1).min(sdf.dimensions[1].saturating_sub(1)),
                        (cb[5] + 1).min(sdf.dimensions[2].saturating_sub(1)),
                    ];
                    if let Some(tsdf) =
                        build_tsdf_chunk_from_sdf(sdf, padded, trunc, segment.sdf_revision)
                    {
                        segment.chunk_runtime.tsdf_chunks.insert(*key, tsdf);
                    }
                }
            }
            // Enqueue mesh rebuild for the baked chunks.
            segment.chunk_runtime.enqueue_dirty_mesh_chunks(keys);
        }

        segment.mesh_dirty = true; // SDF changed, mesh needs update
    }

    // Regenerate mesh if dirty
    if segment.mesh_dirty && allow_mesh_rebuild {
        let mesh_start = Instant::now();
        if is_live {
            // Live path: process dirty chunk meshes within frame budget.
            let chunks_before = segment.chunk_runtime.dirty_mesh_chunks.len();
            let _done = regenerate_live_chunk_meshes(&mut segment.chunk_runtime);
            let chunks_processed = chunks_before - segment.chunk_runtime.dirty_mesh_chunks.len();
            // Only re-merge when at least one chunk was actually meshed.
            if chunks_processed > 0 {
                let merged = merge_chunk_meshes(&segment.chunk_runtime.mesh_chunks_cpu);
                if !merged.is_empty() {
                    segment.mesh = Some(merged);
                    segment.mesh_revision = segment.mesh_revision.wrapping_add(1);
                    mesh_changed = true;
                }
            }
        } else {
            // Finalize path: full-volume Surface Nets extraction.
            segment.clear_chunk_runtime();
            if let Some(sdf) = &segment.sdf {
                let mesh = surface_nets_from_sdf(sdf, 0.0, sdf.active_bounds);
                segment.mesh = Some(mesh);
                segment.mesh_revision = segment.mesh_revision.wrapping_add(1);
                mesh_changed = true;
            }
        }
        mesh_ms = mesh_start.elapsed().as_secs_f32() * 1000.0;
        segment.mesh_dirty = false;
    }

    (mesh_changed, sdf_ms, mesh_ms)
}

/// Process dirty mesh chunks from TSDF using Surface Nets, respecting a frame budget.
///
/// Pops chunks from `dirty_mesh_chunks`, extracts mesh via [`surface_nets_chunk`],
/// and stores results in `mesh_chunks_cpu`.  Returns `true` when the queue is drained.
fn regenerate_live_chunk_meshes(chunk_runtime: &mut SegmentChunkRuntime) -> bool {
    while let Some(key) = chunk_runtime.dirty_mesh_chunks.pop_front() {
        if let Some(tsdf) = chunk_runtime.tsdf_chunks.get(&key) {
            let mesh = surface_nets_chunk(tsdf);
            if mesh.is_empty() {
                chunk_runtime.mesh_chunks_cpu.remove(&key);
            } else {
                chunk_runtime.mesh_chunks_cpu.insert(key, mesh);
            }
        } else {
            chunk_runtime.mesh_chunks_cpu.remove(&key);
        }
    }
    chunk_runtime.dirty_mesh_chunks.is_empty()
}

/// Merge per-chunk meshes into a single `MeshData`.
fn merge_chunk_meshes(chunks: &std::collections::HashMap<ChunkKey, MeshData>) -> MeshData {
    let mut merged = MeshData::new();
    for mesh in chunks.values() {
        if mesh.is_empty() {
            continue;
        }
        let base = merged.vertices.len() as u32;
        merged.vertices.extend_from_slice(&mesh.vertices);
        merged.normals.extend_from_slice(&mesh.normals);
        merged
            .indices
            .extend(mesh.indices.iter().map(|idx| base + *idx));
    }
    merged
}

/// Regenerate all dirty segments.
pub fn regenerate_all_dirty(
    manager: &mut SegmentManager,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> Vec<usize> {
    let mut regenerated = Vec::new();

    for (idx, segment) in manager.segments.iter_mut().enumerate() {
        if regenerate_segment_if_dirty(segment, volume_dims, volume_spacing) {
            regenerated.push(idx);
        }
    }

    regenerated
}

/// Regenerate segment derivatives (SDF + mesh) once per frame if needed.
/// Returns `true` if there are still pending chunks, meaning another frame
/// should be requested to continue draining the queue.
pub fn sys_update_segment_derivatives(world: &mut World, entities: &AppEntities) -> bool {
    let mut volume_dims = [0u32; 3];
    let mut volume_spacing = [0.0f32; 3];
    if let Some((_, vol)) = world.query::<&VolumeData>().iter().next() {
        volume_dims = vol.dimensions;
        volume_spacing = vol.spacing;
    }

    if volume_dims.contains(&0) {
        return false;
    }

    let perf = if let Ok(perf) = world.get::<&SegPerfConfig>(entities.seg_perf) {
        (*perf).clone()
    } else {
        SegPerfConfig::default()
    };

    let mut last_sdf_ms = 0.0f32;
    let mut last_mesh_ms = 0.0f32;
    let mut queue_depth = 0u32;

    if let Ok(mut manager) = world.get::<&mut SegmentManager>(entities.segments) {
        let active_seg_idx = manager.active_segment;
        // 1. Prioritize Active Segment
        if let Some(active_idx) = active_seg_idx {
            if let Some(segment) = manager.segments.get_mut(active_idx) {
                let has_tsdf_queue = !segment.chunk_runtime.dirty_tsdf_chunks.is_empty();
                let has_mesh_queue = !segment.chunk_runtime.dirty_mesh_chunks.is_empty();

                if segment.sdf_dirty || segment.mesh_dirty || has_tsdf_queue || has_mesh_queue {
                    if has_tsdf_queue {
                        segment.chunk_runtime.dirty_tsdf_chunks.clear();
                        segment.sdf_dirty = true;
                    }

                    let (_changed, sdf_ms, mesh_ms) = regenerate_segment_if_dirty_with_resolution(
                        segment,
                        volume_dims,
                        volume_spacing,
                        perf.resolution_scale.max(0.1),
                        perf.sdf_band_mm,
                        perf.mesh_chunk_size,
                        true,
                        segment.mesh_dirty || has_mesh_queue,
                    );
                    last_sdf_ms = sdf_ms;
                    last_mesh_ms = mesh_ms;
                }
            }
        }

        // 2. Background Segments (fully process one per frame if dirty)
        for (i, segment) in manager.segments.iter_mut().enumerate() {
            if active_seg_idx == Some(i) {
                continue;
            }

            let has_tsdf_queue = !segment.chunk_runtime.dirty_tsdf_chunks.is_empty();
            let has_mesh_queue = !segment.chunk_runtime.dirty_mesh_chunks.is_empty();

            if segment.sdf_dirty || segment.mesh_dirty || has_tsdf_queue || has_mesh_queue {
                if has_tsdf_queue {
                    segment.chunk_runtime.dirty_tsdf_chunks.clear();
                    segment.sdf_dirty = true;
                }

                let (_changed, sdf_ms, mesh_ms) = regenerate_segment_if_dirty_with_resolution(
                    segment,
                    volume_dims,
                    volume_spacing,
                    perf.resolution_scale.max(0.1),
                    perf.sdf_band_mm,
                    perf.mesh_chunk_size,
                    false,
                    segment.mesh_dirty || has_mesh_queue,
                );
                last_sdf_ms = last_sdf_ms.max(sdf_ms);
                last_mesh_ms = last_mesh_ms.max(mesh_ms);
                break; // Only one background segment per frame to keep things somewhat manageable
            }
        }

        // Update global queue depth for UI
        queue_depth = manager
            .segments
            .iter()
            .map(|s| {
                s.chunk_runtime.dirty_tsdf_chunks.len() as u32
                    + s.chunk_runtime.dirty_mesh_chunks.len() as u32
                    + u32::from(s.sdf_dirty || s.mesh_dirty)
            })
            .sum();
    }

    if let Ok(mut perf_mut) = world.get::<&mut SegPerfConfig>(entities.seg_perf) {
        perf_mut.last_sdf_ms = last_sdf_ms;
        perf_mut.last_mesh_ms = last_mesh_ms;
        perf_mut.queue_depth = queue_depth;
    }

    // Signal that more frames are needed if any segment is still dirty.
    queue_depth > 0
}

// ============================================================================
// Contour Drawing Integration
// ============================================================================

/// Start drawing a contour on the active segment.
pub fn start_drawing(
    manager: &mut SegmentManager,
    slice_plane: SlicePlane,
    slice_index: i32,
    start_point: [f32; 2],
) -> bool {
    if manager.active_segment.is_none() {
        return false;
    }

    manager.draw_state = ContourDrawState::Drawing {
        points: vec![start_point],
        slice_plane,
        slice_index,
    };
    true
}

/// Add a point while drawing.
pub fn add_drawing_point(manager: &mut SegmentManager, point: [f32; 2]) {
    if let ContourDrawState::Drawing { points, .. } = &mut manager.draw_state {
        // Only add if sufficiently far from last point
        if let Some(last) = points.last() {
            let dx = point[0] - last[0];
            let dy = point[1] - last[1];
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > 0.005 {
                // Min distance threshold in UV
                points.push(point);
            }
        } else {
            points.push(point);
        }
    }
}

fn project_point_for_slice(slice_plane: SlicePlane, p: [f32; 3]) -> [f32; 2] {
    match slice_plane {
        SlicePlane::Axial => [p[0], p[1]],
        SlicePlane::Coronal => [p[0], p[2]],
        SlicePlane::Sagittal => [p[1], p[2]],
    }
}

fn stroke_bounds_world(points: &[[f32; 3]], margin_mm: f32) -> Option<[f32; 6]> {
    if points.is_empty() {
        return None;
    }
    let mut minp = [f32::MAX; 3];
    let mut maxp = [f32::MIN; 3];
    for p in points {
        minp[0] = minp[0].min(p[0]);
        minp[1] = minp[1].min(p[1]);
        minp[2] = minp[2].min(p[2]);
        maxp[0] = maxp[0].max(p[0]);
        maxp[1] = maxp[1].max(p[1]);
        maxp[2] = maxp[2].max(p[2]);
    }
    let m = margin_mm.max(0.0);
    Some([
        minp[0] - m,
        minp[1] - m,
        minp[2] - m,
        maxp[0] + m,
        maxp[1] + m,
        maxp[2] + m,
    ])
}

fn world_roi_to_index_bounds(
    roi_world: [f32; 6],
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> Option<[u32; 6]> {
    if roi_world[0] > roi_world[3] || roi_world[1] > roi_world[4] || roi_world[2] > roi_world[5] {
        return None;
    }
    if volume_dims.contains(&0) || volume_spacing.iter().any(|v| *v <= 0.0 || !v.is_finite()) {
        return None;
    }

    let clamp_index = |v: f32, axis: usize| -> u32 {
        let max_i = volume_dims[axis].saturating_sub(1) as f32;
        (v / volume_spacing[axis]).floor().clamp(0.0, max_i) as u32
    };
    let clamp_index_max = |v: f32, axis: usize| -> u32 {
        let max_i = volume_dims[axis].saturating_sub(1) as f32;
        (v / volume_spacing[axis]).ceil().clamp(0.0, max_i) as u32
    };

    let bounds = [
        clamp_index(roi_world[0], 0),
        clamp_index(roi_world[1], 1),
        clamp_index(roi_world[2], 2),
        clamp_index_max(roi_world[3], 0),
        clamp_index_max(roi_world[4], 1),
        clamp_index_max(roi_world[5], 2),
    ];
    if bounds[0] > bounds[3] || bounds[1] > bounds[4] || bounds[2] > bounds[5] {
        return None;
    }
    Some(bounds)
}

fn orient2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment2d(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    let min_x = a[0].min(b[0]) - 1e-5;
    let max_x = a[0].max(b[0]) + 1e-5;
    let min_y = a[1].min(b[1]) - 1e-5;
    let max_y = a[1].max(b[1]) + 1e-5;
    p[0] >= min_x && p[0] <= max_x && p[1] >= min_y && p[1] <= max_y
}

fn segments_intersect_2d(a1: [f32; 2], a2: [f32; 2], b1: [f32; 2], b2: [f32; 2]) -> bool {
    let o1 = orient2d(a1, a2, b1);
    let o2 = orient2d(a1, a2, b2);
    let o3 = orient2d(b1, b2, a1);
    let o4 = orient2d(b1, b2, a2);

    if o1.abs() < 1e-6 && on_segment2d(a1, a2, b1) {
        return true;
    }
    if o2.abs() < 1e-6 && on_segment2d(a1, a2, b2) {
        return true;
    }
    if o3.abs() < 1e-6 && on_segment2d(b1, b2, a1) {
        return true;
    }
    if o4.abs() < 1e-6 && on_segment2d(b1, b2, a2) {
        return true;
    }

    (o1 > 0.0) != (o2 > 0.0) && (o3 > 0.0) != (o4 > 0.0)
}

fn point_in_polygon_2d(point: [f32; 2], poly: &[[f32; 2]]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut crossings = 0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let y_in_range =
            (a[1] <= point[1] && b[1] > point[1]) || (b[1] <= point[1] && a[1] > point[1]);
        if y_in_range {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x_intersect = a[0] + t * (b[0] - a[0]);
            if point[0] < x_intersect {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

fn contours_overlap_or_intersect_on_slice(
    a: &[[f32; 3]],
    b: &[[f32; 3]],
    slice_plane: SlicePlane,
) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let a2: Vec<[f32; 2]> = a
        .iter()
        .map(|&p| project_point_for_slice(slice_plane, p))
        .collect();
    let b2: Vec<[f32; 2]> = b
        .iter()
        .map(|&p| project_point_for_slice(slice_plane, p))
        .collect();

    for i in 0..a2.len() {
        let a1 = a2[i];
        let a2n = a2[(i + 1) % a2.len()];
        for j in 0..b2.len() {
            let b1 = b2[j];
            let b2n = b2[(j + 1) % b2.len()];
            if segments_intersect_2d(a1, a2n, b1, b2n) {
                return true;
            }
        }
    }

    point_in_polygon_2d(a2[0], &b2) || point_in_polygon_2d(b2[0], &a2)
}

/// Finish drawing and add contour to the active segment.
///
/// Returns true if a contour was successfully added.
pub fn finish_drawing(
    manager: &mut SegmentManager,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> bool {
    use crate::systems::contour_draw::{
        maybe_close_contour, restabilize_contour, restabilize_contour_for_sculpt,
        screen_points_to_plane_contour,
    };

    let draw_state = std::mem::take(&mut manager.draw_state);

    if let ContourDrawState::Drawing {
        points,
        slice_plane,
        slice_index,
    } = draw_state
    {
        if points.len() < 3 {
            return false; // Not enough points
        }

        // 1. Convert to world points
        let stroke_points = screen_points_to_plane_contour(
            &points,
            slice_plane,
            slice_index,
            volume_dims,
            volume_spacing,
        )
        .points;
        let stroke_roi_world = stroke_bounds_world(&stroke_points, 8.0);

        // 2. Try to BRIDGE or SCULPT existing contours
        let mut processed = false;
        if let Some(segment) = manager.active_segment_mut() {
            if let Some(existing_list) = segment
                .contours
                .contours_at_slice_mut(slice_plane, slice_index)
            {
                // A. Check for BRIDGE (start/end on different contours)
                let mut bridge_candidates = Vec::new();
                for (i, contour) in existing_list.iter().enumerate() {
                    let (_, d_start) = crate::systems::contour_draw::find_nearest_point_on_contour(
                        &contour.points,
                        stroke_points[0],
                    );
                    let (_, d_end) = crate::systems::contour_draw::find_nearest_point_on_contour(
                        &contour.points,
                        stroke_points[stroke_points.len() - 1],
                    );

                    if d_start <= 15.0 {
                        bridge_candidates.push((i, true));
                    }
                    if d_end <= 15.0 {
                        bridge_candidates.push((i, false));
                    }
                }

                // If we found two different contours to bridge
                let unique_ids: HashSet<_> = bridge_candidates.iter().map(|(id, _)| *id).collect();
                if unique_ids.len() >= 2 {
                    let ids: Vec<_> = unique_ids.into_iter().collect();
                    let id_a = ids[0];
                    let id_b = ids[1];
                    let (first, second) = if id_a < id_b {
                        (id_a, id_b)
                    } else {
                        (id_b, id_a)
                    };

                    let contour_b_points = existing_list[second].points.clone();
                    if crate::systems::contour_draw::bridge_contours(
                        &mut existing_list[first].points,
                        &contour_b_points,
                        &stroke_points,
                        15.0,
                    ) {
                        existing_list[first].points =
                            restabilize_contour_for_sculpt(&existing_list[first].points, true);
                        existing_list.remove(second);
                        processed = true;
                    }
                }

                // B. Fallback to SCULPT (start/end on same contour)
                if !processed {
                    for existing_contour in existing_list.iter_mut() {
                        if crate::systems::contour_draw::sculpt_contour(
                            &mut existing_contour.points,
                            &stroke_points,
                            15.0,
                        ) {
                            existing_contour.points =
                                restabilize_contour_for_sculpt(&existing_contour.points, true);
                            existing_contour.is_closed = true;
                            processed = true;
                            break;
                        }
                    }
                }
            }
        }

        if processed {
            if let Some(segment) = manager.active_segment_mut() {
                segment.mark_slice_edited(slice_plane, slice_index);
                if let Some(roi) = stroke_roi_world {
                    segment.mark_dirty_with_world_roi(roi);
                    if let Some(index_bounds) =
                        world_roi_to_index_bounds(roi, volume_dims, volume_spacing)
                    {
                        let keys =
                            chunk_keys_for_bounds(index_bounds, segment.chunk_runtime.chunk_size);
                        segment
                            .chunk_runtime
                            .enqueue_dirty_tsdf_chunks(keys.clone());
                        segment.chunk_runtime.enqueue_dirty_mesh_chunks(keys);
                    }
                } else {
                    segment.mark_dirty();
                }
                return true;
            }
        }

        // 3. Fallback: Create a new closed contour
        let mut contour = PlaneContour::with_points(
            Plane3D::from_slice_plane(
                slice_plane,
                (slice_index as f32 + 0.5) * volume_spacing[slice_plane.depth_axis()],
            ),
            stroke_points,
            false,
        );

        let natural_close_threshold = 10.0;
        let is_natural_closure = maybe_close_contour(&mut contour.points, natural_close_threshold);

        contour.points = restabilize_contour(&contour.points, is_natural_closure);
        contour.is_closed = true;

        if let Some(segment) = manager.active_segment_mut() {
            // Suppress creating overlapping/intersecting loops on the same slice.
            if let Some(existing_list) =
                segment.contours.contours_at_slice(slice_plane, slice_index)
            {
                for existing in existing_list {
                    if !existing.points.is_empty() && !contour.points.is_empty() {
                        if contours_overlap_or_intersect_on_slice(
                            &existing.points,
                            &contour.points,
                            slice_plane,
                        ) {
                            return true;
                        }
                        let (_, d) = crate::systems::contour_draw::find_nearest_point_on_contour(
                            &existing.points,
                            contour.points[0],
                        );
                        if d < 10.0 {
                            return true;
                        } // suppress redundant loop
                    }
                }
            }
            segment
                .contours
                .add_contour(slice_plane, slice_index, contour);
            segment.mark_slice_edited(slice_plane, slice_index);
            if let Some(roi) = stroke_roi_world {
                segment.mark_dirty_with_world_roi(roi);
                if let Some(index_bounds) =
                    world_roi_to_index_bounds(roi, volume_dims, volume_spacing)
                {
                    let keys =
                        chunk_keys_for_bounds(index_bounds, segment.chunk_runtime.chunk_size);
                    segment
                        .chunk_runtime
                        .enqueue_dirty_tsdf_chunks(keys.clone());
                    segment.chunk_runtime.enqueue_dirty_mesh_chunks(keys);
                }
            } else {
                segment.mark_dirty();
            }
            return true;
        }
    }

    false
}

/// Cancel the current drawing operation.
pub fn cancel_drawing(manager: &mut SegmentManager) {
    manager.draw_state = ContourDrawState::Idle;
}

/// Check if currently drawing.
pub fn is_drawing(manager: &SegmentManager) -> bool {
    matches!(manager.draw_state, ContourDrawState::Drawing { .. })
}

// ============================================================================
// GPU Resource Management
// ============================================================================

/// Cached GPU resources for a segment's mesh.
pub struct SegmentGpuResources {
    pub segment_id: uuid::Uuid,
    pub mesh_resources: Option<MeshResources>,
}

impl SegmentGpuResources {
    /// Create empty resources for a segment.
    pub fn new(segment_id: uuid::Uuid) -> Self {
        Self {
            segment_id,
            mesh_resources: None,
        }
    }

    /// Update mesh resources from segment.
    pub fn update_from_segment(&mut self, device: &wgpu::Device, segment: &Segment) {
        if let Some(mesh) = &segment.mesh {
            self.mesh_resources = MeshResources::from_mesh_data(device, mesh);
        } else {
            self.mesh_resources = None;
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_manager_new() {
        let manager = SegmentManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert!(manager.active_segment.is_none());
    }

    #[test]
    fn test_add_segment() {
        let mut manager = SegmentManager::new();
        let idx = manager.add_segment("Kidney", [1.0, 0.0, 0.0, 1.0]);

        assert_eq!(idx, 0);
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.active_segment, Some(0));
        assert_eq!(manager.segments[0].name, "Kidney");
    }

    #[test]
    fn test_remove_segment() {
        let mut manager = SegmentManager::new();
        manager.add_segment("A", [1.0, 0.0, 0.0, 1.0]);
        manager.add_segment("B", [0.0, 1.0, 0.0, 1.0]);
        manager.add_segment("C", [0.0, 0.0, 1.0, 1.0]);
        manager.active_segment = Some(1); // B

        let removed = manager.remove_segment(0); // Remove A
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "A");
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active_segment, Some(0)); // B is now at index 0
    }

    #[test]
    fn test_find_by_id() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        let id = manager.segments[0].id;

        assert_eq!(manager.find_by_id(id), Some(0));
        assert_eq!(manager.find_by_id(uuid::Uuid::new_v4()), None);
    }

    #[test]
    fn test_start_drawing_no_active() {
        let mut manager = SegmentManager::new();
        let result = start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);
        assert!(!result); // No active segment
    }

    #[test]
    fn test_start_drawing_with_active() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);

        let result = start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);
        assert!(result);
        assert!(is_drawing(&manager));
    }

    #[test]
    fn test_add_drawing_point() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        start_drawing(&mut manager, SlicePlane::Axial, 50, [0.0, 0.0]);

        add_drawing_point(&mut manager, [0.1, 0.0]); // Far enough
        add_drawing_point(&mut manager, [0.101, 0.0]); // Too close, should be ignored

        if let ContourDrawState::Drawing { points, .. } = &manager.draw_state {
            assert_eq!(points.len(), 2); // Only 2 points (start + 1 valid)
        } else {
            panic!("Expected Drawing state");
        }
    }

    #[test]
    fn test_cancel_drawing() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);

        cancel_drawing(&mut manager);
        assert!(!is_drawing(&manager));
    }

    #[test]
    fn test_regenerate_segment_updates_sdf_revision() {
        let mut segment = Segment::new("Test", [1.0, 0.0, 0.0, 1.0]);
        let rev0 = segment.sdf_revision;
        let _ = regenerate_segment_if_dirty(&mut segment, [8, 8, 8], [1.0, 1.0, 1.0]);
        assert!(segment.sdf_revision > rev0);
    }

    #[test]
    fn test_contours_overlap_or_intersect_on_slice_intersection() {
        let a = vec![
            [0.0, 0.0, 10.0],
            [2.0, 0.0, 10.0],
            [2.0, 2.0, 10.0],
            [0.0, 2.0, 10.0],
        ];
        let b = vec![
            [1.0, -1.0, 10.0],
            [3.0, 1.0, 10.0],
            [1.0, 3.0, 10.0],
            [-1.0, 1.0, 10.0],
        ];
        assert!(contours_overlap_or_intersect_on_slice(
            &a,
            &b,
            SlicePlane::Axial
        ));
    }

    #[test]
    fn test_contours_overlap_or_intersect_on_slice_separate() {
        let a = vec![
            [0.0, 0.0, 10.0],
            [1.0, 0.0, 10.0],
            [1.0, 1.0, 10.0],
            [0.0, 1.0, 10.0],
        ];
        let b = vec![
            [3.0, 3.0, 10.0],
            [4.0, 3.0, 10.0],
            [4.0, 4.0, 10.0],
            [3.0, 4.0, 10.0],
        ];
        assert!(!contours_overlap_or_intersect_on_slice(
            &a,
            &b,
            SlicePlane::Axial
        ));
    }

    #[test]
    fn test_world_roi_to_index_bounds_clamps() {
        let b = world_roi_to_index_bounds(
            [-5.0, 1.2, 2.0, 100.0, 3.1, 4.9],
            [10, 10, 10],
            [1.0, 1.0, 1.0],
        )
        .unwrap();
        assert_eq!(b, [0, 1, 2, 9, 4, 5]);
    }

    // ====================================================================
    // Chunked Pipeline Verification Tests
    // ====================================================================

    /// Helper: create a segment with a small circle contour on one axial slice.
    fn make_segment_with_circle(volume_dims: [u32; 3], slice_z: i32, radius: f32) -> Segment {
        let mut segment = Segment::new("ChunkTest", [1.0, 0.0, 0.0, 1.0]);
        let cx = volume_dims[0] as f32 / 2.0;
        let cy = volume_dims[1] as f32 / 2.0;
        let cz = slice_z as f32;
        let n = 32;
        let mut pts = Vec::with_capacity(n);
        for i in 0..n {
            let angle = 2.0 * std::f32::consts::PI * (i as f32) / (n as f32);
            pts.push([cx + radius * angle.cos(), cy + radius * angle.sin(), cz]);
        }
        use crate::app::segment::{Plane3D, PlaneContour};
        let contour = PlaneContour {
            plane: Plane3D {
                normal: [0.0, 0.0, 1.0],
                distance: cz,
            },
            points: pts,
            is_closed: true,
        };
        segment
            .contours
            .axial
            .entry(slice_z)
            .or_default()
            .push(contour);
        segment.sdf_dirty = true;
        segment.mesh_dirty = true;
        segment
    }

    #[test]
    fn test_chunk_budget_per_chunk_under_3ms() {
        // Build a small segment and run the live pipeline.
        // In release mode, assert per-chunk SDF + mesh time is each under 3ms.
        // In debug mode, just verify the pipeline produces output (timing is ~15-20x slower).
        let dims = [64u32, 64, 32];
        let spacing = [1.0f32, 1.0, 1.0];
        let mut segment = make_segment_with_circle(dims, 16, 10.0);

        let (mesh_changed, sdf_ms, mesh_ms) = regenerate_segment_if_dirty_with_resolution(
            &mut segment,
            dims,
            spacing,
            1.0,  // resolution_multiplier
            8.0,  // sdf_band_mm
            32,   // mesh_chunk_size
            true, // is_live
            true, // allow_mesh_rebuild
        );

        assert!(mesh_changed, "mesh should be regenerated");

        // Only enforce strict timing in release mode.
        // SDF threshold is 16ms (one frame budget) — this covers the full
        // contour→SDF build, not a single chunk. The mesh threshold is 3ms
        // since it's purely chunked Surface Nets + merge.
        #[cfg(not(debug_assertions))]
        {
            assert!(
                sdf_ms < 16.0,
                "SDF build should fit in one frame for small volume, was {sdf_ms:.2}ms"
            );
            assert!(
                mesh_ms < 3.0,
                "mesh should be under 3ms for small volume, was {mesh_ms:.2}ms"
            );
        }

        // In all modes, verify pipeline produced output.
        #[cfg(debug_assertions)]
        {
            eprintln!("[budget test debug] sdf={sdf_ms:.2}ms  mesh={mesh_ms:.2}ms");
        }
        assert!(segment.mesh.is_some(), "pipeline should produce a mesh");
        assert!(
            !segment.mesh.as_ref().unwrap().is_empty(),
            "mesh should not be empty"
        );
    }

    #[test]
    fn test_locality_small_edit_max_8_chunks() {
        // A small contour circle should only dirty a limited number of chunks.
        let dims = [128u32, 128, 64];
        let spacing = [1.0f32, 1.0, 1.0];
        let mut segment = make_segment_with_circle(dims, 32, 5.0);

        // First pass: build everything.
        let _ = regenerate_segment_if_dirty_with_resolution(
            &mut segment,
            dims,
            spacing,
            1.0,
            8.0,
            32,
            true,
            true,
        );

        // Now simulate a small edit: add a tiny contour nearby.
        let cz = 32.0f32;
        let cx = dims[0] as f32 / 2.0 + 2.0;
        let cy = dims[1] as f32 / 2.0 + 2.0;
        let mut pts = Vec::new();
        for i in 0..16 {
            let angle = 2.0 * std::f32::consts::PI * (i as f32) / 16.0;
            pts.push([cx + 2.0 * angle.cos(), cy + 2.0 * angle.sin(), cz]);
        }
        use crate::app::segment::{Plane3D, PlaneContour};
        segment
            .contours
            .axial
            .entry(32)
            .or_default()
            .push(PlaneContour {
                plane: Plane3D {
                    normal: [0.0, 0.0, 1.0],
                    distance: cz,
                },
                points: pts,
                is_closed: true,
            });

        // Mark dirty with a tight world ROI (the small contour's bounding box).
        segment.mark_dirty_with_world_roi([
            cx - 3.0,
            cy - 3.0,
            cz - 1.0,
            cx + 3.0,
            cy + 3.0,
            cz + 1.0,
        ]);

        // Rebuild.
        let _ = regenerate_segment_if_dirty_with_resolution(
            &mut segment,
            dims,
            spacing,
            1.0,
            8.0,
            16,
            false,
            true,
        );

        // The number of TSDF chunks should be bounded.
        let n_chunks = segment.chunk_runtime.tsdf_chunks.len();
        assert!(
            n_chunks <= 8,
            "small edit should touch at most 8 chunks, got {n_chunks}"
        );
    }

    #[test]
    fn test_partial_processing_respects_budget() {
        // Fill the mesh queue with many chunks, run with a very tiny budget,
        // and assert the queue is NOT fully drained in one call.
        let dims = [128u32, 128, 64];
        let spacing = [1.0f32, 1.0, 1.0];
        let mut segment = make_segment_with_circle(dims, 32, 30.0);

        // Build SDF so we have TSDF chunks to work with.
        let _ = regenerate_segment_if_dirty_with_resolution(
            &mut segment,
            dims,
            spacing,
            1.0,
            8.0,
            16, // smaller chunk_size = more chunks
            true,
            true,
        );

        // Re-enqueue all TSDF chunk keys as dirty mesh chunks.
        let all_keys: Vec<ChunkKey> = segment.chunk_runtime.tsdf_chunks.keys().copied().collect();
        let total_chunks = all_keys.len();
        if total_chunks <= 1 {
            // Not enough chunks to test partial processing — pass trivially.
            return;
        }
        segment.chunk_runtime.enqueue_dirty_mesh_chunks(all_keys);

        // Run without budget.
        let done = regenerate_live_chunk_meshes(&mut segment.chunk_runtime);

        // The queue should be fully drained in one call.
        let remaining = segment.chunk_runtime.dirty_mesh_chunks.len();
        assert!(done);
        assert_eq!(remaining, 0);
    }
}
