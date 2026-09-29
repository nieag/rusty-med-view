use crate::app::roi::{ContourData, ContourLoop, ContourPoint, ContourSlice, MeshData};
use crate::convert::{world_mm_to_plane_local_mm, PlaneDefinition};
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};

const PLANE_EPSILON_MM: f32 = 1e-4;
const POINT_QUANTIZATION_PER_MM: f32 = 10_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshPlaneIntersectionError {
    InvalidPlane,
    InvalidFace { face_index: usize },
    InvalidVertex { vertex_index: usize },
}

pub fn intersect_mesh_with_plane(
    mesh: &MeshData,
    plane: PlaneDefinition,
) -> Result<ContourData, MeshPlaneIntersectionError> {
    let normal = Vec3::from_array(plane.normal_mm);
    if !normal.is_finite() || normal.length_squared() <= 1e-12 {
        return Err(MeshPlaneIntersectionError::InvalidPlane);
    }
    let normal = normal.normalize();
    let origin = Vec3::from_array(plane.origin_mm);
    let mut segments = BTreeSet::new();
    let mut points_by_key = BTreeMap::new();

    for (face_index, face) in mesh.faces.iter().enumerate() {
        let mut triangle = [Vec3::ZERO; 3];
        for (corner, vertex_index) in face.vertex_indices.into_iter().enumerate() {
            let Some(vertex) = mesh.vertices.get(vertex_index as usize) else {
                return Err(MeshPlaneIntersectionError::InvalidFace { face_index });
            };
            triangle[corner] = Vec3::from_array(vertex.world_mm);
            if !triangle[corner].is_finite() {
                return Err(MeshPlaneIntersectionError::InvalidVertex {
                    vertex_index: vertex_index as usize,
                });
            }
        }

        let distances = triangle.map(|point| (point - origin).dot(normal));
        let mut intersections = Vec::with_capacity(3);
        for [a, b] in [[0, 1], [1, 2], [2, 0]] {
            let da = distances[a];
            let db = distances[b];
            let a_on = da.abs() <= PLANE_EPSILON_MM;
            let b_on = db.abs() <= PLANE_EPSILON_MM;
            if a_on && b_on {
                continue;
            }
            if a_on {
                intersections.push(triangle[a]);
            } else if b_on {
                intersections.push(triangle[b]);
            } else if (da < 0.0) != (db < 0.0) {
                let t = da / (da - db);
                intersections.push(triangle[a].lerp(triangle[b], t));
            }
        }
        dedup_points(&mut intersections);
        if intersections.len() < 2 {
            continue;
        }
        let (a, b) = farthest_pair(&intersections);
        let a_local = world_mm_to_plane_local_mm(a.to_array(), plane);
        let b_local = world_mm_to_plane_local_mm(b.to_array(), plane);
        let a_key = point_key(a_local);
        let b_key = point_key(b_local);
        if a_key == b_key {
            continue;
        }
        points_by_key.entry(a_key).or_insert(a_local);
        points_by_key.entry(b_key).or_insert(b_local);
        segments.insert(ordered_edge(a_key, b_key));
    }

    let loops = chain_segments(segments, &points_by_key);
    Ok(ContourData {
        active_plane_family: plane.family,
        slices: if loops.is_empty() {
            Vec::new()
        } else {
            vec![ContourSlice { plane, loops }]
        },
    })
}

fn dedup_points(points: &mut Vec<Vec3>) {
    let mut index = 0;
    while index < points.len() {
        let point = points[index];
        let duplicate = points[..index].iter().any(|existing| {
            existing.distance_squared(point) <= PLANE_EPSILON_MM * PLANE_EPSILON_MM
        });
        if duplicate {
            points.remove(index);
        } else {
            index += 1;
        }
    }
}

fn farthest_pair(points: &[Vec3]) -> (Vec3, Vec3) {
    let mut best = (points[0], points[1]);
    let mut best_distance = best.0.distance_squared(best.1);
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            let distance = points[i].distance_squared(points[j]);
            if distance > best_distance {
                best = (points[i], points[j]);
                best_distance = distance;
            }
        }
    }
    best
}

fn point_key(point: [f32; 2]) -> [i32; 2] {
    point.map(|value| (value * POINT_QUANTIZATION_PER_MM).round() as i32)
}

fn ordered_edge(a: [i32; 2], b: [i32; 2]) -> ([i32; 2], [i32; 2]) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn chain_segments(
    mut unused: BTreeSet<([i32; 2], [i32; 2])>,
    points: &BTreeMap<[i32; 2], [f32; 2]>,
) -> Vec<ContourLoop> {
    let mut adjacency: BTreeMap<[i32; 2], BTreeSet<[i32; 2]>> = BTreeMap::new();
    for (a, b) in &unused {
        adjacency.entry(*a).or_default().insert(*b);
        adjacency.entry(*b).or_default().insert(*a);
    }
    let mut loops = Vec::new();
    while let Some(&(start, next)) = unused.iter().next() {
        unused.remove(&ordered_edge(start, next));
        let mut keys = vec![start, next];
        let mut previous = start;
        let mut current = next;
        let mut closed = false;
        for _ in 0..adjacency.len().saturating_mul(2).max(8) {
            if current == start {
                keys.pop();
                closed = true;
                break;
            }
            let Some(candidate) = adjacency.get(&current).and_then(|neighbors| {
                neighbors.iter().copied().find(|candidate| {
                    *candidate != previous && unused.contains(&ordered_edge(current, *candidate))
                })
            }) else {
                break;
            };
            unused.remove(&ordered_edge(current, candidate));
            previous = current;
            current = candidate;
            keys.push(current);
        }
        let contour_points = keys
            .iter()
            .filter_map(|key| points.get(key))
            .map(|local_mm| ContourPoint {
                local_mm: *local_mm,
            })
            .collect::<Vec<_>>();
        if contour_points.len() >= if closed { 3 } else { 2 } {
            loops.push(ContourLoop {
                points: contour_points,
                is_closed: closed,
            });
        }
    }
    loops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{VoxelData, VoxelGeometry};
    use crate::convert::{
        extract_mesh_from_voxel_data, orthogonal_plane_from_volume_uv, PlaneFamily,
    };

    #[test]
    fn test_closed_voxel_mesh_intersection_produces_closed_loop() {
        let geometry =
            VoxelGeometry::new([4, 4, 4], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let mut raw_data = vec![0_u8; 64];
        for z in 1..=2 {
            for y in 1..=2 {
                for x in 1..=2 {
                    raw_data[(z * 16 + y * 4 + x) as usize] = 1;
                }
            }
        }
        let mesh = extract_mesh_from_voxel_data(&VoxelData { geometry, raw_data }).unwrap();
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();

        let contour = intersect_mesh_with_plane(&mesh, plane).unwrap();

        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        assert!(contour.slices[0].loops[0].is_valid_closed_loop());
    }

    #[test]
    fn test_plane_missing_mesh_returns_empty_contour_data() {
        let geometry =
            VoxelGeometry::new([2, 2, 2], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let mesh = extract_mesh_from_voxel_data(&VoxelData {
            geometry,
            raw_data: vec![1; 8],
        })
        .unwrap();
        let mut plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
        plane.origin_mm[2] = 100.0;

        let contour = intersect_mesh_with_plane(&mesh, plane).unwrap();

        assert!(contour.slices.is_empty());
    }
}
