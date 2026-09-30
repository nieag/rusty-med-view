//! Brush deformation of a closed surface mesh that cannot fold it through itself.
//!
//! The brush moves vertices by a smooth falloff of their distance along the surface. Where the
//! moved part would pass through another part of the surface (or itself), the displacement is
//! scaled back to the largest fraction that stays free of new intersections, so a drag meets the
//! surface like a wall instead of producing a mesh that fails validation at commit.

use crate::convert::mesh_voxelize::faces_overlap_beyond_shared_boundary;
use crate::model::{MeshData, MeshVertex};
use glam::Vec3;
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::Vector;
use parry3d::shape::TriMesh;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// Bisection steps when searching for the collision-free fraction (resolution 1/256).
const FRACTION_SEARCH_STEPS: usize = 8;
const BOUNDS_MARGIN_MM: f32 = 1e-3;

/// Topology of a mesh prepared once per drag: coincident vertices are welded so the surface
/// moves as one piece, and adjacency and per-vertex faces are stored flat.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshDeformBase {
    /// Welded vertex id of every source vertex.
    welded_of: Vec<u32>,
    welded_positions: Vec<[f32; 3]>,
    faces: Vec<[u32; 3]>,
    neighbor_offsets: Vec<u32>,
    neighbors: Vec<u32>,
    face_offsets: Vec<u32>,
    vertex_faces: Vec<u32>,
}

impl MeshDeformBase {
    /// `None` when a face references a missing vertex.
    pub fn new(mesh: &MeshData) -> Option<Self> {
        let mut ids: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.vertices.len());
        let mut welded_positions = Vec::new();
        let mut welded_of = Vec::with_capacity(mesh.vertices.len());
        for vertex in &mesh.vertices {
            let key = vertex
                .world_mm
                .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let id = *ids.entry(key).or_insert_with(|| {
                welded_positions.push(vertex.world_mm);
                (welded_positions.len() - 1) as u32
            });
            welded_of.push(id);
        }
        let mut faces = Vec::with_capacity(mesh.faces.len());
        for face in &mesh.faces {
            if face
                .vertex_indices
                .iter()
                .any(|index| *index as usize >= mesh.vertices.len())
            {
                return None;
            }
            faces.push(face.vertex_indices.map(|index| welded_of[index as usize]));
        }
        let count = welded_positions.len();
        let mut neighbor_lists = vec![Vec::new(); count];
        let mut face_lists = vec![Vec::new(); count];
        for (face_index, face) in faces.iter().enumerate() {
            let [a, b, c] = face.map(|id| id as usize);
            for (from, to) in [(a, b), (b, c), (c, a)] {
                neighbor_lists[from].push(to as u32);
                neighbor_lists[to].push(from as u32);
            }
            for vertex in [a, b, c] {
                face_lists[vertex].push(face_index as u32);
            }
        }
        let (neighbor_offsets, neighbors) = flatten(neighbor_lists);
        let (face_offsets, vertex_faces) = flatten(face_lists);
        Some(Self {
            welded_of,
            welded_positions,
            faces,
            neighbor_offsets,
            neighbors,
            face_offsets,
            vertex_faces,
        })
    }

    fn neighbors_of(&self, vertex: usize) -> &[u32] {
        let range =
            self.neighbor_offsets[vertex] as usize..self.neighbor_offsets[vertex + 1] as usize;
        &self.neighbors[range]
    }

    fn faces_of(&self, vertex: usize) -> &[u32] {
        let range = self.face_offsets[vertex] as usize..self.face_offsets[vertex + 1] as usize;
        &self.vertex_faces[range]
    }
}

fn flatten(lists: Vec<Vec<u32>>) -> (Vec<u32>, Vec<u32>) {
    let mut offsets = Vec::with_capacity(lists.len() + 1);
    let mut flat = Vec::new();
    offsets.push(0);
    for list in lists {
        flat.extend(list);
        offsets.push(flat.len() as u32);
    }
    (offsets, flat)
}

/// Full (unscaled) displacement of every welded vertex the brush reaches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrushDisplacement {
    pub vertices: Vec<u32>,
    pub offsets: Vec<[f32; 3]>,
}

impl BrushDisplacement {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// The brush: a smooth falloff of the distance along the surface from the seed triangle.
/// The radius grows with the drag so a long drag moves a wider area instead of a sharp spike.
pub fn brush_displacement(
    base: &MeshDeformBase,
    seed_indices: [u32; 3],
    anchor_world_mm: [f32; 3],
    delta_world_mm: [f32; 3],
    radius_mm: f32,
    strength: f32,
) -> BrushDisplacement {
    if !radius_mm.is_finite()
        || radius_mm <= 0.0
        || !strength.is_finite()
        || delta_world_mm.iter().any(|value| !value.is_finite())
    {
        return BrushDisplacement::default();
    }
    let delta = Vec3::from_array(delta_world_mm) * strength.max(0.0);
    let radius_mm = radius_mm.max(1.5 * delta.length());
    if !delta.is_finite() || !radius_mm.is_finite() || delta == Vec3::ZERO {
        return BrushDisplacement::default();
    }
    let anchor = Vec3::from_array(anchor_world_mm);
    let position = |vertex: usize| Vec3::from_array(base.welded_positions[vertex]);
    let mut distances = vec![f32::INFINITY; base.welded_positions.len()];
    let mut queue = BinaryHeap::new();
    for index in seed_indices {
        let Some(&welded) = base.welded_of.get(index as usize) else {
            return BrushDisplacement::default();
        };
        let welded = welded as usize;
        let distance = position(welded).distance(anchor);
        if distance <= radius_mm && distance < distances[welded] {
            distances[welded] = distance;
            queue.push(QueueEntry {
                distance,
                vertex: welded,
            });
        }
    }
    while let Some(QueueEntry { distance, vertex }) = queue.pop() {
        if distance != distances[vertex] {
            continue;
        }
        for &next in base.neighbors_of(vertex) {
            let next = next as usize;
            let next_distance = distance + position(vertex).distance(position(next));
            if next_distance <= radius_mm && next_distance < distances[next] {
                distances[next] = next_distance;
                queue.push(QueueEntry {
                    distance: next_distance,
                    vertex: next,
                });
            }
        }
    }
    let mut displacement = BrushDisplacement::default();
    for (vertex, distance) in distances.into_iter().enumerate() {
        if !distance.is_finite() {
            continue;
        }
        let normalized = 1.0 - distance / radius_mm;
        let weight = normalized * normalized * (3.0 - 2.0 * normalized);
        displacement.vertices.push(vertex as u32);
        displacement.offsets.push((delta * weight).to_array());
    }
    displacement
}

/// `mesh` with `fraction` of the displacement applied (`mesh` must be the mesh `base` was built
/// from).
pub fn apply_displacement(
    base: &MeshDeformBase,
    mesh: &MeshData,
    displacement: &BrushDisplacement,
    fraction: f32,
) -> MeshData {
    let mut offset_of = HashMap::with_capacity(displacement.vertices.len());
    for (vertex, offset) in displacement.vertices.iter().zip(&displacement.offsets) {
        offset_of.insert(*vertex, Vec3::from_array(*offset) * fraction);
    }
    let vertices = mesh
        .vertices
        .iter()
        .zip(&base.welded_of)
        .map(|(vertex, welded)| match offset_of.get(welded) {
            Some(offset) => MeshVertex {
                world_mm: (Vec3::from_array(vertex.world_mm) + *offset).to_array(),
            },
            None => *vertex,
        })
        .collect();
    MeshData {
        vertices,
        faces: mesh.faces.clone(),
    }
}

/// The largest fraction of the displacement (searched to 1/256) after which no moved triangle
/// intersects another triangle. Returns 1.0 when the undeformed mesh already intersects where the
/// brush reaches, since no fraction can then be told apart from the existing problem.
pub fn collision_free_fraction(base: &MeshDeformBase, displacement: &BrushDisplacement) -> f32 {
    if displacement.is_empty() {
        return 1.0;
    }
    let mut moving = vec![false; base.faces.len()];
    for vertex in &displacement.vertices {
        for face in base.faces_of(*vertex as usize) {
            moving[*face as usize] = true;
        }
    }
    let moving_faces: Vec<usize> = (0..moving.len()).filter(|face| moving[*face]).collect();
    if moving_faces.is_empty() {
        return 1.0;
    }
    let mut positions = base.welded_positions.clone();
    let place = |positions: &mut Vec<[f32; 3]>, fraction: f32| {
        for (vertex, offset) in displacement.vertices.iter().zip(&displacement.offsets) {
            let start = base.welded_positions[*vertex as usize];
            positions[*vertex as usize] =
                std::array::from_fn(|axis| start[axis] + offset[axis] * fraction);
        }
    };

    // Static triangles that could ever be touched: those near the swept region.
    let mut swept = face_bounds(&base.faces, &positions, &moving_faces);
    place(&mut positions, 1.0);
    swept = union(swept, face_bounds(&base.faces, &positions, &moving_faces));
    let swept = Aabb::new(
        swept.mins - Vector::splat(BOUNDS_MARGIN_MM),
        swept.maxs + Vector::splat(BOUNDS_MARGIN_MM),
    );
    place(&mut positions, 0.0);
    let static_faces: Vec<usize> = (0..base.faces.len())
        .filter(|face| !moving[*face])
        .filter(|face| bounds_of(&base.faces[*face], &positions).intersects(&swept))
        .collect();
    let static_mesh = build_tri_mesh(&base.faces, &positions, &static_faces);

    let mut is_free = |fraction: f32| -> bool {
        place(&mut positions, fraction);
        let Some(moving_mesh) = build_tri_mesh(&base.faces, &positions, &moving_faces) else {
            return true;
        };
        for &face in &moving_faces {
            let bounds = bounds_of(&base.faces[face], &positions);
            for other in moving_mesh.bvh().intersect_aabb(&bounds) {
                let other_face = moving_faces[other as usize];
                if other_face > face
                    && faces_overlap_beyond_shared_boundary(
                        &positions,
                        base.faces[face],
                        base.faces[other_face],
                    )
                {
                    return false;
                }
            }
            if let Some(static_mesh) = &static_mesh {
                for other in static_mesh.bvh().intersect_aabb(&bounds) {
                    let other_face = static_faces[other as usize];
                    if faces_overlap_beyond_shared_boundary(
                        &positions,
                        base.faces[face],
                        base.faces[other_face],
                    ) {
                        return false;
                    }
                }
            }
        }
        true
    };

    if is_free(1.0) || !is_free(0.0) {
        return 1.0;
    }
    let (mut free, mut blocked) = (0.0_f32, 1.0_f32);
    for _ in 0..FRACTION_SEARCH_STEPS {
        let middle = 0.5 * (free + blocked);
        if is_free(middle) {
            free = middle;
        } else {
            blocked = middle;
        }
    }
    free
}

/// Collision-free brush deformation of `mesh` (which `base` was built from), and the fraction of
/// the requested drag that was applied.
pub fn deform_mesh_surface_brush_limited(
    base: &MeshDeformBase,
    mesh: &MeshData,
    seed_indices: [u32; 3],
    anchor_world_mm: [f32; 3],
    delta_world_mm: [f32; 3],
    radius_mm: f32,
    strength: f32,
) -> (MeshData, f32) {
    let displacement = brush_displacement(
        base,
        seed_indices,
        anchor_world_mm,
        delta_world_mm,
        radius_mm,
        strength,
    );
    if displacement.is_empty() {
        return (mesh.clone(), 1.0);
    }
    let fraction = collision_free_fraction(base, &displacement);
    (
        apply_displacement(base, mesh, &displacement, fraction),
        fraction,
    )
}

/// The brush without collision limiting, for tests of the falloff itself.
#[cfg(test)]
pub fn deform_mesh_surface_brush(
    mesh: &MeshData,
    seed_indices: [u32; 3],
    anchor_world_mm: [f32; 3],
    delta_world_mm: [f32; 3],
    radius_mm: f32,
    strength: f32,
) -> MeshData {
    let Some(base) = MeshDeformBase::new(mesh) else {
        return mesh.clone();
    };
    let displacement = brush_displacement(
        &base,
        seed_indices,
        anchor_world_mm,
        delta_world_mm,
        radius_mm,
        strength,
    );
    apply_displacement(&base, mesh, &displacement, 1.0)
}

fn bounds_of(face: &[u32; 3], positions: &[[f32; 3]]) -> Aabb {
    let points = face.map(|id| Vector::from_array(positions[id as usize]));
    Aabb::new(
        points[0].min(points[1]).min(points[2]),
        points[0].max(points[1]).max(points[2]),
    )
}

fn face_bounds(faces: &[[u32; 3]], positions: &[[f32; 3]], subset: &[usize]) -> Aabb {
    subset
        .iter()
        .map(|face| bounds_of(&faces[*face], positions))
        .reduce(union)
        .expect("subset is not empty")
}

fn union(a: Aabb, b: Aabb) -> Aabb {
    Aabb::new(a.mins.min(b.mins), a.maxs.max(b.maxs))
}

fn build_tri_mesh(faces: &[[u32; 3]], positions: &[[f32; 3]], subset: &[usize]) -> Option<TriMesh> {
    if subset.is_empty() {
        return None;
    }
    TriMesh::new(
        positions.iter().map(|p| Vector::from_array(*p)).collect(),
        subset.iter().map(|face| faces[*face]).collect(),
    )
    .ok()
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    distance: f32,
    vertex: usize,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.distance.to_bits() == other.distance.to_bits() && self.vertex == other.vertex
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.vertex.cmp(&self.vertex))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests;
