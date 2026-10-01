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
use web_time::Instant;

/// Slices closer than this many layers (centre to centre) are consecutive and blend.
const CONSECUTIVE_LAYERS: f32 = 1.5;

/// Signed distances in millimetres (negative inside) at the voxel centres of `geometry`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourField {
    pub geometry: VoxelGeometry,
    pub values: Arc<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourFieldError {
    InvalidPlane,
}

pub(crate) fn family_axes(family: OrthogonalFamily) -> (usize, usize, usize) {
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
    /// The shape revision of the ROI the field was built for (set by the owner of the state).
    pub generation: u64,
    family: OrthogonalFamily,
    slices: Vec<CachedSlice>,
}

impl ContourFieldState {
    /// The whole build in one go; see [`ContourFieldBuild`] for the time-sliced form.
    pub fn build(
        previous: Option<&Self>,
        contour: &ContourData,
        reference: VoxelGeometry,
        keep: Option<VoxelGeometry>,
    ) -> Result<Option<Self>, ContourFieldError> {
        let Some(mut build) = ContourFieldBuild::begin(previous, contour, reference, keep)? else {
            return Ok(None);
        };
        while !build.step(None) {}
        Ok(Some(build.finish()))
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

enum SliceWork {
    Done(CachedSlice),
    Computing(Box<SliceSweep>),
}

struct SliceSweep {
    plane: PlaneDefinition,
    loops: Vec<Vec<[f32; 2]>>,
    depth: f32,
    rows: RowSweep,
}

/// A contour field being built in steps that each stop at a deadline, so a large update is spread
/// over frames: first the in-plane distances of the slices that changed (row by row), then the
/// depths of the field that those slices can influence.
pub struct ContourFieldBuild {
    previous: Option<ContourFieldState>,
    geometry: VoxelGeometry,
    family: OrthogonalFamily,
    work: Vec<SliceWork>,
    next_work: usize,
    assembly: Option<Assembly>,
}

struct Assembly {
    slices: Vec<CachedSlice>,
    stacks: Vec<std::ops::Range<usize>>,
    values: Vec<f32>,
    z_range: std::ops::Range<usize>,
    next_z: usize,
}

impl ContourFieldBuild {
    /// Plans the build; `None` when the contours have no loops. Slices equal to one of
    /// `previous` (same plane and loops, same box) are reused.
    pub fn begin(
        previous: Option<&ContourFieldState>,
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
        let (na, nb) = (dims[a_axis] as usize, dims[b_axis] as usize);
        // Cached slices are valid only in the same box and family.
        let reusable = previous.filter(|state| {
            state.family == family && state.field.geometry.identity() == geometry.identity()
        });

        let mut work = Vec::new();
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
                work.push(SliceWork::Done(cached.clone()));
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
            work.push(SliceWork::Computing(Box::new(SliceSweep {
                plane: slice.plane,
                rows: RowSweep::new(&loops, us, vs),
                loops,
                depth,
            })));
        }
        Ok(Some(Self {
            previous: reusable.cloned(),
            geometry,
            family,
            work,
            next_work: 0,
            assembly: None,
        }))
    }

    /// Works until done (`true`) or until `deadline` has passed (`false`); `None` never stops.
    pub fn step(&mut self, deadline: Option<Instant>) -> bool {
        let expired = || deadline.is_some_and(|deadline| Instant::now() >= deadline);
        while self.next_work < self.work.len() {
            if let SliceWork::Computing(sweep) = &mut self.work[self.next_work] {
                if !sweep.rows.step(deadline) {
                    return false;
                }
                let finished = std::mem::take(&mut sweep.rows.result);
                let cached = CachedSlice {
                    plane: sweep.plane,
                    loops: std::mem::take(&mut sweep.loops),
                    depth: sweep.depth,
                    distance: Arc::new(finished),
                };
                self.work[self.next_work] = SliceWork::Done(cached);
            }
            self.next_work += 1;
            if expired() && self.next_work < self.work.len() {
                return false;
            }
        }
        if self.assembly.is_none() {
            self.assembly = Some(self.begin_assembly());
        }
        let geometry = self.geometry;
        let family = self.family;
        let assembly = self.assembly.as_mut().expect("assembly just created");
        while assembly.next_z < assembly.z_range.end {
            assembly.assemble_depth(geometry, family, assembly.next_z);
            assembly.next_z += 1;
            if expired() && assembly.next_z < assembly.z_range.end {
                return false;
            }
        }
        true
    }

    fn begin_assembly(&mut self) -> Assembly {
        let mut slices: Vec<CachedSlice> = std::mem::take(&mut self.work)
            .into_iter()
            .filter_map(|work| match work {
                SliceWork::Done(slice) => Some(slice),
                SliceWork::Computing(_) => None,
            })
            .collect();
        slices.sort_by(|a, b| a.depth.total_cmp(&b.depth));

        // Stacks of consecutive layers.
        let mut stacks = Vec::new();
        let mut start = 0;
        for k in 1..=slices.len() {
            if k == slices.len() || slices[k].depth - slices[k - 1].depth > CONSECUTIVE_LAYERS {
                stacks.push(start..k);
                start = k;
            }
        }
        let (depth_axis, _, _) = family_axes(self.family);
        let depth_samples = self.geometry.dimensions[depth_axis] as usize;
        // Only depths a changed slice can influence are reassembled; the rest is kept.
        let (values, z_range) = match &self.previous {
            Some(state) => (
                (*state.field.values).clone(),
                changed_depth_range(&state.slices, &slices, depth_samples),
            ),
            None => (
                vec![
                    f32::INFINITY;
                    self.geometry
                        .dimensions
                        .iter()
                        .map(|d| *d as usize)
                        .product::<usize>()
                ],
                0..depth_samples,
            ),
        };
        Assembly {
            next_z: z_range.start,
            slices,
            stacks,
            values,
            z_range,
        }
    }

    /// The finished state. Only valid after `step` returned `true`.
    pub fn finish(mut self) -> ContourFieldState {
        let assembly = self.assembly.take().expect("field build is not finished");
        {
            ContourFieldState {
                field: ContourField {
                    geometry: self.geometry,
                    values: Arc::new(assembly.values),
                },
                generation: 0,
                family: self.family,
                slices: assembly.slices,
            }
        }
    }
}

impl Assembly {
    /// The field at depth sample `z`: the minimum over stacks of the prism distance.
    fn assemble_depth(&mut self, geometry: VoxelGeometry, family: OrthogonalFamily, z: usize) {
        let (depth_axis, a_axis, b_axis) = family_axes(family);
        let dims = geometry.dimensions;
        let (na, nb) = (dims[a_axis] as usize, dims[b_axis] as usize);
        let depth_spacing = geometry.spacing()[depth_axis];
        let index_of = |i: usize, j: usize| {
            let mut index = [0usize; 3];
            index[a_axis] = i;
            index[b_axis] = j;
            index[depth_axis] = z;
            (index[2] * dims[1] as usize + index[1]) * dims[0] as usize + index[0]
        };
        for j in 0..nb {
            for i in 0..na {
                self.values[index_of(i, j)] = f32::INFINITY;
            }
        }
        let slices = &self.slices;
        let zf = z as f32;
        for stack in &self.stacks {
            let first = slices[stack.start].depth;
            let last = slices[stack.end - 1].depth;
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
                    let slot = &mut self.values[index_of(i, j)];
                    *slot = slot.min(distance);
                }
            }
        }
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
/// rectangular lattice with columns `us` and rows `vs` to the loops, computed a row at a time.
struct RowSweep {
    segments: Vec<([f32; 2], [f32; 2])>,
    nearest: SegmentTree,
    us: Vec<f32>,
    vs: Vec<f32>,
    crossings: Vec<f32>,
    result: Vec<f32>,
    next_row: usize,
    hint: usize,
}

impl RowSweep {
    fn new(loops: &[Vec<[f32; 2]>], us: Vec<f32>, vs: Vec<f32>) -> Self {
        let mut segments: Vec<([f32; 2], [f32; 2])> = Vec::new();
        for points in loops {
            for k in 0..points.len() {
                segments.push((points[k], points[(k + 1) % points.len()]));
            }
        }
        let nearest = SegmentTree::new(&segments);
        Self {
            result: vec![0.0; us.len() * vs.len()],
            segments,
            nearest,
            us,
            vs,
            crossings: Vec::new(),
            next_row: 0,
            hint: 0,
        }
    }

    /// Computes rows until done (`true`) or `deadline` has passed (`false`).
    fn step(&mut self, deadline: Option<Instant>) -> bool {
        while self.next_row < self.vs.len() {
            let j = self.next_row;
            let v = self.vs[j];
            // Where the row crosses the loops (half-open, so a vertex on the row counts once).
            self.crossings.clear();
            for (a, b) in &self.segments {
                if (a[1] > v) != (b[1] > v) {
                    self.crossings
                        .push(a[0] + (b[0] - a[0]) * (v - a[1]) / (b[1] - a[1]));
                }
            }
            self.crossings.sort_by(f32::total_cmp);
            for (i, u) in self.us.iter().enumerate() {
                let to_the_right =
                    self.crossings.len() - self.crossings.partition_point(|x| x <= u);
                let distance = self
                    .nearest
                    .distance(&self.segments, [*u, v], &mut self.hint);
                self.result[i + self.us.len() * j] = if to_the_right % 2 == 1 {
                    -distance
                } else {
                    distance
                };
            }
            self.next_row += 1;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return self.next_row >= self.vs.len();
            }
        }
        true
    }
}

/// Segments in a bounding-box tree, for exact nearest-segment queries.
///
/// A query starts from the nearest segment of the previous sample (a valid upper bound, since
/// neighbouring samples have the same nearest segment almost always) and visits only the boxes
/// closer than that bound, nearest first, so near and far samples alike cost a handful of box and
/// segment tests. The result is the exact minimum (compared as squared distances).
struct SegmentTree {
    nodes: Vec<TreeNode>,
    /// Segment indices, grouped so that every leaf is a contiguous run.
    order: Vec<u32>,
}

struct TreeNode {
    min: [f32; 2],
    max: [f32; 2],
    /// For a leaf the first entry of `order`; for an inner node the index of the left child (the
    /// right child follows the left one's subtree, so it is stored in `right`).
    first: u32,
    /// The right child of an inner node.
    right: u32,
    /// Segments in a leaf; 0 for an inner node.
    count: u32,
}

const LEAF_SEGMENTS: usize = 4;

impl SegmentTree {
    fn new(segments: &[([f32; 2], [f32; 2])]) -> Self {
        let mut order: Vec<u32> = (0..segments.len() as u32).collect();
        let mut nodes = Vec::with_capacity(2 * segments.len() / LEAF_SEGMENTS + 2);
        let len = order.len();
        Self::build(segments, &mut order, 0, len, &mut nodes);
        Self { nodes, order }
    }

    fn build(
        segments: &[([f32; 2], [f32; 2])],
        order: &mut [u32],
        start: usize,
        end: usize,
        nodes: &mut Vec<TreeNode>,
    ) -> u32 {
        let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for index in &order[start..end] {
            let (a, b) = segments[*index as usize];
            for point in [a, b] {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
        }
        let id = nodes.len() as u32;
        nodes.push(TreeNode {
            min,
            max,
            first: start as u32,
            right: 0,
            count: (end - start) as u32,
        });
        if end - start <= LEAF_SEGMENTS {
            return id;
        }
        // Split at the median segment midpoint along the longer side.
        let axis = usize::from(max[1] - min[1] > max[0] - min[0]);
        let middle = start + (end - start) / 2;
        order[start..end].select_nth_unstable_by(middle - start, |x, y| {
            let centre = |index: &u32| {
                let (a, b) = segments[*index as usize];
                a[axis] + b[axis]
            };
            centre(x).total_cmp(&centre(y))
        });
        let left = Self::build(segments, order, start, middle, nodes);
        let right = Self::build(segments, order, middle, end, nodes);
        let node = &mut nodes[id as usize];
        node.count = 0;
        node.first = left;
        node.right = right;
        id
    }

    /// The exact distance from `point` to the nearest segment; `hint` holds the nearest segment of
    /// the previous query and is updated.
    fn distance(
        &self,
        segments: &[([f32; 2], [f32; 2])],
        point: [f32; 2],
        hint: &mut usize,
    ) -> f32 {
        let (a, b) = segments[*hint];
        let mut best = point_segment_distance_squared(point, a, b);
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            // The margin keeps a segment whose rounded distance is a hair below the box bound.
            if box_distance_squared(point, node.min, node.max) > best * (1.0 + 1e-5) {
                continue;
            }
            if node.count > 0 {
                let run = &self.order[node.first as usize..(node.first + node.count) as usize];
                for index in run {
                    let (a, b) = segments[*index as usize];
                    let squared = point_segment_distance_squared(point, a, b);
                    if squared < best {
                        best = squared;
                        *hint = *index as usize;
                    }
                }
            } else {
                let (left, right) = (node.first, node.right);
                let left_node = &self.nodes[left as usize];
                let right_node = &self.nodes[right as usize];
                let near_left = box_distance_squared(point, left_node.min, left_node.max)
                    <= box_distance_squared(point, right_node.min, right_node.max);
                // The nearer child is visited first, so it is pushed last.
                let (far, near) = if near_left {
                    (right, left)
                } else {
                    (left, right)
                };
                stack[top] = far;
                stack[top + 1] = near;
                top += 2;
            }
        }
        best.sqrt()
    }
}

fn box_distance_squared(p: [f32; 2], min: [f32; 2], max: [f32; 2]) -> f32 {
    let dx = (min[0] - p[0]).max(p[0] - max[0]).max(0.0);
    let dy = (min[1] - p[1]).max(p[1] - max[1]).max(0.0);
    dx * dx + dy * dy
}

fn point_segment_distance_squared(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let length_squared = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length_squared > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [ap[0] - t * ab[0], ap[1] - t * ab[1]];
    d[0] * d[0] + d[1] * d[1]
}

#[cfg(test)]
fn point_segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    point_segment_distance_squared(p, a, b).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::orthogonal_plane_from_volume_uv;
    use crate::model::{ContourLoop, ContourPoint, ContourSlice, PlaneFamily};

    fn squares(geometry: VoxelGeometry, layers: &[(f32, f32)]) -> ContourData {
        let slices = layers
            .iter()
            .map(|(layer, h)| {
                let plane = orthogonal_plane_from_volume_uv(
                    PlaneFamily::Axial,
                    [0.5, 0.5, (layer + 0.5) / geometry.dimensions[2] as f32],
                    geometry,
                )
                .unwrap();
                ContourSlice {
                    plane,
                    loops: vec![ContourLoop {
                        points: [[-h, -h], [*h, -h], [*h, *h], [-h, *h]]
                            .map(|local_mm| ContourPoint { local_mm })
                            .to_vec(),
                        is_closed: true,
                    }],
                }
            })
            .collect();
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices,
        }
    }

    #[test]
    fn test_nearest_segment_search_matches_brute_force() {
        let mut seed = 12345u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        for case in 0..20 {
            let count = 3 + case * 7;
            let loops: Vec<Vec<[f32; 2]>> = (0..2)
                .map(|_| {
                    let centre = [random() * 30.0, random() * 30.0];
                    (0..count)
                        .map(|k| {
                            let angle = k as f32 / count as f32 * std::f32::consts::TAU;
                            let radius = 2.0 + random() * 6.0;
                            [
                                centre[0] + radius * angle.cos(),
                                centre[1] + radius * angle.sin(),
                            ]
                        })
                        .collect()
                })
                .collect();
            let us: Vec<f32> = (0..90).map(|i| -20.0 + i as f32 * 0.7).collect();
            let vs: Vec<f32> = (0..80).map(|j| -15.0 + j as f32 * 0.8).collect();
            let mut sweep = RowSweep::new(&loops, us.clone(), vs.clone());
            assert!(sweep.step(None));
            for (j, v) in vs.iter().enumerate() {
                for (i, u) in us.iter().enumerate() {
                    let brute = sweep
                        .segments
                        .iter()
                        .map(|(a, b)| point_segment_distance([*u, *v], *a, *b))
                        .fold(f32::INFINITY, f32::min);
                    let got = sweep.result[i + us.len() * j].abs();
                    assert_eq!(got.to_bits(), brute.to_bits(), "case {case} at {i},{j}");
                }
            }
        }
    }

    #[test]
    fn test_a_build_stopped_at_every_deadline_equals_the_one_shot_build() {
        let geometry = VoxelGeometry::new(
            [30, 26, 20],
            [1.0, 1.0, 1.5],
            [0.0; 3],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let before = squares(geometry, &[(5.0, 4.0), (6.0, 6.0), (7.0, 5.0), (10.0, 2.0)]);
        let after = squares(
            geometry,
            &[(5.0, 4.0), (6.0, 5.5), (7.0, 5.0), (8.0, 3.0), (10.0, 2.0)],
        );
        let previous = ContourFieldState::build(None, &before, geometry, None)
            .unwrap()
            .unwrap();
        let keep = Some(previous.field.geometry);
        let expected = ContourFieldState::build(Some(&previous), &after, geometry, keep)
            .unwrap()
            .unwrap();

        let mut build = ContourFieldBuild::begin(Some(&previous), &after, geometry, keep)
            .unwrap()
            .unwrap();
        let mut steps = 0;
        while !build.step(Some(Instant::now())) {
            steps += 1;
            assert!(steps < 100_000, "a stopped build must still make progress");
        }
        assert!(steps > 3, "the work was spread over {steps} steps");
        let state = build.finish();
        assert_eq!(state.field, expected.field);
    }
}
