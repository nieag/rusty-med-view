//! A signed distance field of a contour stack, for meshing (ADR 0006).
//!
//! The field is sampled at the voxel centres of a snug box of the reference grid and holds the
//! distance in millimetres to the surface of the shape the contours describe, negative inside. A
//! mesh extracted at the zero level (marching cubes, linear along each edge) follows the drawn
//! loops to a small fraction of a voxel, because the values are exact distances to the loop
//! segments, not a quantised mask.
//!
//! The shape: each drawn slice stands for one layer (a slab of half a layer on each side of its
//! plane). Slices in consecutive layers form a **stack** and blend their in-plane distances
//! linearly between neighbouring planes; a gap breaks the stack, so nothing is invented across a
//! gap (interpolation is a separate, explicit tool, backlog 3.8). The distance of a stack to its
//! prism is `max(in_plane, along_depth)` inside and their root sum of squares outside, which makes
//! the values near a cap the true distance to it, and the field of several stacks is their
//! minimum.

use crate::convert::{
    snug_geometry_for_contour, voxel_index_to_world_mm, world_mm_to_plane_local_mm,
    world_mm_to_voxel_index,
};
use crate::model::{ContourData, MeshData, OrthogonalFamily, PlaneDefinition, VoxelGeometry};
use std::sync::Arc;

/// Slices closer than this many layers (centre to centre) are consecutive and blend.
const CONSECUTIVE_LAYERS: f32 = 1.5;

/// Signed distances in millimetres (negative inside) at the voxel centres of `geometry`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourField {
    pub geometry: VoxelGeometry,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourFieldError {
    InvalidPlane,
}

fn family_axes(family: OrthogonalFamily) -> (usize, usize, usize) {
    match family {
        OrthogonalFamily::Axial => (2, 0, 1),
        OrthogonalFamily::Coronal => (1, 0, 2),
        OrthogonalFamily::Sagittal => (0, 1, 2),
    }
}

/// The field of the contours inside the snug box of `reference` (grown to cover `keep`), or
/// `None` when the contours have no loops.
pub fn contour_distance_field(
    contour: &ContourData,
    reference: VoxelGeometry,
    keep: Option<VoxelGeometry>,
) -> Result<Option<ContourField>, ContourFieldError> {
    Ok(ContourFieldState::build(None, contour, reference, keep)?.map(|state| state.field))
}

/// One drawn slice with its in-plane distances, kept so an edit elsewhere does not redo it.
#[derive(Debug, Clone)]
struct CachedSlice {
    plane: PlaneDefinition,
    loops: Vec<Vec<[f32; 2]>>,
    /// Position along the depth axis, in voxel-index units of the box.
    depth: f32,
    distance: Arc<Vec<f32>>,
}

/// The depth samples whose values can differ between two lists of slices (sorted by depth, in the
/// same box): those between a changed slice and its neighbours, and beyond it up to the edge of
/// the box when it ends a stack (the distance outside a stack grows with the distance to its end).
fn changed_depth_range(
    old: &[CachedSlice],
    new: &[CachedSlice],
    depth_samples: usize,
) -> std::ops::Range<usize> {
    let shared = |slice: &CachedSlice, other: &[CachedSlice]| {
        other
            .iter()
            .any(|candidate| Arc::ptr_eq(&candidate.distance, &slice.distance))
    };
    let (mut low, mut high) = (usize::MAX, 0usize);
    let mut include = |slices: &[CachedSlice], k: usize| {
        let depth = slices[k].depth;
        let reach = |neighbour: Option<&CachedSlice>, towards_end: usize| match neighbour {
            Some(other) if (other.depth - depth).abs() <= CONSECUTIVE_LAYERS => {
                other.depth.round().clamp(0.0, depth_samples as f32) as usize
            }
            _ => towards_end,
        };
        let below = reach(k.checked_sub(1).map(|k| &slices[k]), 0);
        let above = reach(slices.get(k + 1), depth_samples);
        low = low.min(below.min(depth.floor().max(0.0) as usize));
        high = high.max(above.max(depth.ceil().max(0.0) as usize + 1));
    };
    for k in 0..new.len() {
        if !shared(&new[k], old) {
            include(new, k);
        }
    }
    for k in 0..old.len() {
        if !shared(&old[k], new) {
            include(old, k);
        }
    }
    if low == usize::MAX {
        return 0..0;
    }
    low.saturating_sub(1)..(high + 1).min(depth_samples)
}

/// A contour field together with the per-slice work behind it. Building from the state of the
/// previous revision recomputes only the slices that changed (about 6 ms each on the liver) and
/// reassembles the field from the cached slices.
#[derive(Debug, Clone)]
pub struct ContourFieldState {
    pub field: ContourField,
    family: OrthogonalFamily,
    slices: Vec<CachedSlice>,
}

impl ContourFieldState {
    pub fn build(
        previous: Option<&Self>,
        contour: &ContourData,
        reference: VoxelGeometry,
        keep: Option<VoxelGeometry>,
    ) -> Result<Option<Self>, ContourFieldError> {
        if !contour.has_loops() {
            return Ok(None);
        }
        let geometry = snug_geometry_for_contour(contour, reference, keep);
        let family = contour.active_plane_family;
        let (depth_axis, a_axis, b_axis) = family_axes(family);
        let dims = geometry.dimensions;
        let (na, nb, nd) = (
            dims[a_axis] as usize,
            dims[b_axis] as usize,
            dims[depth_axis] as usize,
        );
        let depth_spacing = geometry.spacing()[depth_axis];
        // Cached slices are valid only in the same box and family.
        let reusable = previous.filter(|state| {
            state.family == family && state.field.geometry.identity() == geometry.identity()
        });

        let mut slices: Vec<CachedSlice> = Vec::new();
        for slice in &contour.slices {
            let loops: Vec<Vec<[f32; 2]>> = slice
                .loops
                .iter()
                .filter(|contour_loop| contour_loop.is_valid_closed_loop())
                .map(|contour_loop| contour_loop.points.iter().map(|p| p.local_mm).collect())
                .collect();
            if loops.is_empty() {
                continue;
            }
            let cached = reusable.and_then(|state| {
                state
                    .slices
                    .iter()
                    .find(|cached| cached.plane == slice.plane && cached.loops == loops)
            });
            if let Some(cached) = cached {
                slices.push(cached.clone());
                continue;
            }
            let depth = world_mm_to_voxel_index(slice.plane.origin_mm, geometry)[depth_axis];
            if !depth.is_finite() {
                return Err(ContourFieldError::InvalidPlane);
            }
            // In-plane position of every sample column and row, in the slice's own millimetres.
            // The box's axes are the plane's, so columns share one `u` and rows share one `v`.
            let local = |i: usize, j: usize| {
                let mut index = [0.0_f32; 3];
                index[a_axis] = i as f32;
                index[b_axis] = j as f32;
                index[depth_axis] = depth;
                world_mm_to_plane_local_mm(voxel_index_to_world_mm(index, geometry), slice.plane)
            };
            let us: Vec<f32> = (0..na).map(|i| local(i, 0)[0]).collect();
            let vs: Vec<f32> = (0..nb).map(|j| local(0, j)[1]).collect();
            let distance = Arc::new(slice_signed_distances(&loops, &us, &vs));
            slices.push(CachedSlice {
                plane: slice.plane,
                loops,
                depth,
                distance,
            });
        }
        slices.sort_by(|a, b| a.depth.total_cmp(&b.depth));

        // Stacks of consecutive layers.
        let mut stacks: Vec<std::ops::Range<usize>> = Vec::new();
        let mut start = 0;
        for k in 1..=slices.len() {
            if k == slices.len() || slices[k].depth - slices[k - 1].depth > CONSECUTIVE_LAYERS {
                stacks.push(start..k);
                start = k;
            }
        }

        // Only depths a changed slice can influence are reassembled; the rest is kept.
        let (mut values, z_range) = match reusable {
            Some(state) => (
                state.field.values.clone(),
                changed_depth_range(&state.slices, &slices, nd),
            ),
            None => (vec![f32::INFINITY; na * nb * nd], 0..nd),
        };
        let index_of = |i: usize, j: usize, z: usize| {
            let mut index = [0usize; 3];
            index[a_axis] = i;
            index[b_axis] = j;
            index[depth_axis] = z;
            (index[2] * dims[1] as usize + index[1]) * dims[0] as usize + index[0]
        };
        for z in z_range.clone() {
            for j in 0..nb {
                for i in 0..na {
                    values[index_of(i, j, z)] = f32::INFINITY;
                }
            }
        }
        for stack in &stacks {
            let first = slices[stack.start].depth;
            let last = slices[stack.end - 1].depth;
            for z in z_range.clone() {
                let zf = z as f32;
                // Distance along the depth axis to the stack's slab (half a layer beyond its end
                // slices): negative inside, so it is minus the distance to the nearest cap.
                let along = ((first - 0.5 - zf).max(zf - (last + 0.5))) * depth_spacing;
                // The slices that blend at this depth.
                let (low, high, alpha) = if zf <= first {
                    (stack.start, stack.start, 0.0)
                } else if zf >= last {
                    (stack.end - 1, stack.end - 1, 0.0)
                } else {
                    let upper = (stack.start..stack.end)
                        .find(|k| slices[*k].depth > zf)
                        .unwrap_or(stack.end - 1);
                    let lower = upper - 1;
                    let span = slices[upper].depth - slices[lower].depth;
                    (lower, upper, (zf - slices[lower].depth) / span)
                };
                for j in 0..nb {
                    for i in 0..na {
                        let cell = i + na * j;
                        let in_plane = slices[low].distance[cell] * (1.0 - alpha)
                            + slices[high].distance[cell] * alpha;
                        let distance = if in_plane > 0.0 && along > 0.0 {
                            in_plane.hypot(along)
                        } else {
                            in_plane.max(along)
                        };
                        let slot = &mut values[index_of(i, j, z)];
                        *slot = slot.min(distance);
                    }
                }
            }
        }
        Ok(Some(Self {
            field: ContourField { geometry, values },
            family,
            slices,
        }))
    }

    pub fn approx_bytes(&self) -> usize {
        self.field.values.len() * 4
            + self
                .slices
                .iter()
                .map(|slice| slice.distance.len() * 4)
                .sum::<usize>()
    }
}

/// The zero-level surface of the field (marching cubes, linear along each edge).
pub fn mesh_from_contour_field(field: &ContourField) -> MeshData {
    let smooth =
        crate::convert::smooth_mesh_field_from_signed_distance(field.geometry, &field.values);
    crate::convert::extract_mesh_chunk_from_field_on_grid(
        field.geometry,
        &smooth,
        [0; 3],
        field.geometry.dimensions,
    )
}

/// Signed in-plane distance (negative inside, even-odd over all loops) from every sample of the
/// rectangular lattice with columns `us` and rows `vs` to the loops.
fn slice_signed_distances(loops: &[Vec<[f32; 2]>], us: &[f32], vs: &[f32]) -> Vec<f32> {
    let mut segments: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for points in loops {
        for k in 0..points.len() {
            segments.push((points[k], points[(k + 1) % points.len()]));
        }
    }
    let nearest = SegmentGrid::new(&segments, us, vs);
    let mut result = vec![0.0_f32; us.len() * vs.len()];
    let mut crossings: Vec<f32> = Vec::new();
    for (j, v) in vs.iter().enumerate() {
        // Where the row crosses the loops (half-open, so a vertex on the row counts once).
        crossings.clear();
        for (a, b) in &segments {
            if (a[1] > *v) != (b[1] > *v) {
                crossings.push(a[0] + (b[0] - a[0]) * (v - a[1]) / (b[1] - a[1]));
            }
        }
        crossings.sort_by(f32::total_cmp);
        for (i, u) in us.iter().enumerate() {
            let to_the_right = crossings.len() - crossings.partition_point(|x| x <= u);
            let distance = nearest.distance([*u, *v]);
            result[i + us.len() * j] = if to_the_right % 2 == 1 {
                -distance
            } else {
                distance
            };
        }
    }
    result
}

/// Segments bucketed in a uniform grid for nearest-segment queries by expanding rings.
struct SegmentGrid<'a> {
    segments: &'a [([f32; 2], [f32; 2])],
    origin: [f32; 2],
    cell: f32,
    cells: [usize; 2],
    buckets: Vec<Vec<u32>>,
}

impl<'a> SegmentGrid<'a> {
    fn new(segments: &'a [([f32; 2], [f32; 2])], us: &[f32], vs: &[f32]) -> Self {
        let step = |values: &[f32]| {
            if values.len() > 1 {
                (values[1] - values[0]).abs()
            } else {
                1.0
            }
        };
        let cell = (2.0 * step(us).max(step(vs))).max(1e-3);
        let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for (a, b) in segments {
            for p in [a, b] {
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
        }
        let origin = [min[0] - cell, min[1] - cell];
        let cells = [
            (((max[0] + cell - origin[0]) / cell).ceil() as usize).max(1),
            (((max[1] + cell - origin[1]) / cell).ceil() as usize).max(1),
        ];
        let mut grid = Self {
            segments,
            origin,
            cell,
            cells,
            buckets: vec![Vec::new(); cells[0] * cells[1]],
        };
        for (index, (a, b)) in segments.iter().enumerate() {
            let low = grid.cell_of([a[0].min(b[0]), a[1].min(b[1])]);
            let high = grid.cell_of([a[0].max(b[0]), a[1].max(b[1])]);
            for cy in low[1]..=high[1] {
                for cx in low[0]..=high[0] {
                    grid.buckets[cx + cells[0] * cy].push(index as u32);
                }
            }
        }
        grid
    }

    /// The cell holding `point`, clamped into the grid.
    fn cell_of(&self, point: [f32; 2]) -> [usize; 2] {
        std::array::from_fn(|axis| {
            (((point[axis] - self.origin[axis]) / self.cell)
                .floor()
                .max(0.0) as usize)
                .min(self.cells[axis] - 1)
        })
    }

    fn distance(&self, point: [f32; 2]) -> f32 {
        let [cx, cy] = self.cell_of(point);
        let mut best = f32::INFINITY;
        let widest = self.cells[0].max(self.cells[1]);
        for ring in 0..=widest {
            let (x0, x1) = (cx as isize - ring as isize, cx as isize + ring as isize);
            let (y0, y1) = (cy as isize - ring as isize, cy as isize + ring as isize);
            for y in y0..=y1 {
                if y < 0 || y >= self.cells[1] as isize {
                    continue;
                }
                let edge_row = y == y0 || y == y1;
                let step = if edge_row { 1 } else { (x1 - x0).max(1) };
                let mut x = x0;
                while x <= x1 {
                    if x >= 0 && x < self.cells[0] as isize {
                        for index in &self.buckets[x as usize + self.cells[0] * y as usize] {
                            let (a, b) = self.segments[*index as usize];
                            best = best.min(point_segment_distance(point, a, b));
                        }
                    }
                    x += step;
                }
            }
            // Anything in a farther ring is at least `ring` cells away.
            if best <= ring as f32 * self.cell {
                break;
            }
        }
        best
    }
}

fn point_segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let length_squared = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length_squared > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (ap[0] - t * ab[0]).hypot(ap[1] - t * ab[1])
}
