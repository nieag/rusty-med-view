//! Cutting a closed mesh with planes: exact contours of a surface, with no voxels in between.
//!
//! A plane splits the vertices into "above" and "not above". A vertex exactly on the plane counts
//! as above (a symbolic nudge), so no triangle is ever cut at a vertex or along an edge, every
//! cut triangle yields exactly one segment, and the segments of a closed surface link into closed
//! loops without any tolerance. Crossing points are shared through the edge they lie on, so the
//! linking is exact too.
//!
//! [`contours_from_mesh`] cuts a whole stack of reference-grid layers in one pass over the
//! triangles (about 15 ms for the liver), [`intersect_mesh_with_plane`] cuts one plane.

use crate::convert::{reference_layer_plane, world_mm_to_plane_local_mm, PlaneDefinition};
use crate::model::{
    ContourData, ContourLoop, ContourPoint, ContourSlice, MeshData, OrthogonalFamily, VoxelGeometry,
};
use glam::{DVec3, Vec3};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshPlaneIntersectionError {
    InvalidPlane,
    InvalidFace { face_index: usize },
    InvalidVertex { vertex_index: usize },
}

/// The mesh with vertices welded by exact position, so triangles that meet share vertex ids and
/// edges can name crossing points.
struct WeldedMesh {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
}

impl WeldedMesh {
    fn new(mesh: &MeshData) -> Result<Self, MeshPlaneIntersectionError> {
        let mut vertices = Vec::new();
        let mut ids: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.vertices.len());
        let mut source_to_welded = Vec::with_capacity(mesh.vertices.len());
        for (vertex_index, vertex) in mesh.vertices.iter().enumerate() {
            if vertex.world_mm.iter().any(|value| !value.is_finite()) {
                return Err(MeshPlaneIntersectionError::InvalidVertex { vertex_index });
            }
            let key = vertex
                .world_mm
                .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let id = *ids.entry(key).or_insert_with(|| {
                vertices.push(vertex.world_mm);
                vertices.len() as u32 - 1
            });
            source_to_welded.push(id);
        }
        let mut triangles = Vec::with_capacity(mesh.faces.len());
        for (face_index, face) in mesh.faces.iter().enumerate() {
            if face
                .vertex_indices
                .iter()
                .any(|index| *index as usize >= mesh.vertices.len())
            {
                return Err(MeshPlaneIntersectionError::InvalidFace { face_index });
            }
            triangles.push(
                face.vertex_indices
                    .map(|index| source_to_welded[index as usize]),
            );
        }
        Ok(Self {
            vertices,
            triangles,
        })
    }
}

/// One cut of a triangle: the edges it crosses (named by their welded vertex ids) and where.
struct Segment {
    ends: [(u64, [f32; 3]); 2],
}

fn edge_key(a: u32, b: u32) -> u64 {
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    (u64::from(low) << 32) | u64::from(high)
}

/// Cuts the mesh at every integer layer of `coordinate` (a plane per layer, the plane of layer `l`
/// being where `coordinate` equals `l`) inside `layers`, and returns the closed loops of each
/// layer as world-space points, in one pass over the triangles.
fn cut_layers(
    mesh: &WeldedMesh,
    coordinate: impl Fn(Vec3) -> f64,
    layers: std::ops::RangeInclusive<i64>,
) -> Vec<(i64, Vec<Vec<[f32; 3]>>)> {
    let t: Vec<f64> = mesh
        .vertices
        .iter()
        .map(|vertex| coordinate(Vec3::from_array(*vertex)))
        .collect();
    let mut by_layer: HashMap<i64, Vec<Segment>> = HashMap::new();
    for triangle in &mesh.triangles {
        let ts = triangle.map(|vertex| t[vertex as usize]);
        let low = ts[0].min(ts[1]).min(ts[2]);
        let high = ts[0].max(ts[1]).max(ts[2]);
        // Layer `l` cuts the triangle when some vertex is below it and some is at or above it:
        // `low < l <= high`.
        let first = (low.floor() as i64 + 1).max(*layers.start());
        let last = (high.floor() as i64).min(*layers.end());
        for layer in first..=last {
            let above = ts.map(|value| value >= layer as f64);
            let mut ends = Vec::with_capacity(2);
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                if above[a] != above[b] {
                    let fraction = (layer as f64 - ts[a]) / (ts[b] - ts[a]);
                    let pa = DVec3::from_array(mesh.vertices[triangle[a] as usize].map(f64::from));
                    let pb = DVec3::from_array(mesh.vertices[triangle[b] as usize].map(f64::from));
                    ends.push((
                        edge_key(triangle[a], triangle[b]),
                        pa.lerp(pb, fraction).as_vec3().to_array(),
                    ));
                }
            }
            if let [first_end, second_end] = ends[..] {
                by_layer.entry(layer).or_default().push(Segment {
                    ends: [first_end, second_end],
                });
            }
        }
    }
    let mut layers: Vec<(i64, Vec<Vec<[f32; 3]>>)> = by_layer
        .into_iter()
        .map(|(layer, segments)| (layer, link_loops(segments)))
        .filter(|(_, loops)| !loops.is_empty())
        .collect();
    layers.sort_unstable_by_key(|(layer, _)| *layer);
    layers
}

/// Links segments that share a crossing edge into loops. At an edge shared by more than two
/// cuts (touching shells) any pairing is valid: the even-odd fill of the loops is the same.
fn link_loops(segments: Vec<Segment>) -> Vec<Vec<[f32; 3]>> {
    let mut touching: HashMap<u64, Vec<usize>> = HashMap::with_capacity(segments.len() * 2);
    for (index, segment) in segments.iter().enumerate() {
        for (key, _) in &segment.ends {
            touching.entry(*key).or_default().push(index);
        }
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let first_key = segments[start].ends[0].0;
        let mut points = vec![segments[start].ends[0].1];
        let mut key = segments[start].ends[1].0;
        let mut position = segments[start].ends[1].1;
        loop {
            points.push(position);
            if key == first_key {
                points.pop(); // back at the start
                break;
            }
            let Some(next) = touching
                .get(&key)
                .and_then(|candidates| candidates.iter().copied().find(|index| !used[*index]))
            else {
                break; // an open chain: the surface is not closed here
            };
            used[next] = true;
            let [a, b] = &segments[next].ends;
            (key, position) = if a.0 == key { (b.0, b.1) } else { (a.0, a.1) };
        }
        if points.len() >= 3 {
            loops.push(points);
        }
    }
    loops
}

/// Points closer than this are one point: a vertex that lies on the cutting plane shows up as
/// two crossings (one per edge) at nearly the same place.
const MERGE_DISTANCE_MM: f32 = 1e-4;

fn to_local_loops(loops: &[Vec<[f32; 3]>], plane: PlaneDefinition) -> Vec<ContourLoop> {
    loops
        .iter()
        .filter_map(|points| {
            let mut local: Vec<[f32; 2]> = Vec::with_capacity(points.len());
            for world in points {
                let point = world_mm_to_plane_local_mm(*world, plane);
                let repeated = local.last().is_some_and(|last| {
                    (last[0] - point[0]).hypot(last[1] - point[1]) <= MERGE_DISTANCE_MM
                });
                if !repeated {
                    local.push(point);
                }
            }
            while local.len() > 1 {
                let (first, last) = (local[0], local[local.len() - 1]);
                if (first[0] - last[0]).hypot(first[1] - last[1]) <= MERGE_DISTANCE_MM {
                    local.pop();
                } else {
                    break;
                }
            }
            (local.len() >= 3).then(|| ContourLoop {
                points: local
                    .into_iter()
                    .map(|local_mm| ContourPoint { local_mm })
                    .collect(),
                is_closed: true,
            })
        })
        .collect()
}

/// The contours of one plane through the mesh: one slice, or none when the plane misses it.
pub fn intersect_mesh_with_plane(
    mesh: &MeshData,
    plane: PlaneDefinition,
) -> Result<Vec<ContourSlice>, MeshPlaneIntersectionError> {
    let normal = Vec3::from_array(plane.normal_mm);
    if !normal.is_finite() || normal.length_squared() <= 1e-12 {
        return Err(MeshPlaneIntersectionError::InvalidPlane);
    }
    let normal = normal.normalize();
    let origin = Vec3::from_array(plane.origin_mm);
    let welded = WeldedMesh::new(mesh)?;
    let layers = cut_layers(
        &welded,
        |point| f64::from((point - origin).dot(normal)),
        0..=0,
    );
    Ok(layers
        .into_iter()
        .next()
        .map(|(_, loops)| {
            vec![ContourSlice {
                plane,
                loops: to_local_loops(&loops, plane),
            }]
        })
        .unwrap_or_default())
}

/// The exact contours of a closed mesh in `family`: one slice per layer of the `reference` grid
/// that the mesh crosses, on the same planes as `extract_contours_in_grid`, with the loops of the
/// true surface cross-section (not voxel staircases). Layers outside the reference grid are not
/// cut.
pub fn contours_from_mesh(
    mesh: &MeshData,
    reference: VoxelGeometry,
    family: OrthogonalFamily,
) -> Result<ContourData, MeshPlaneIntersectionError> {
    let layer_count = reference_depth_len(reference, family);
    let first_plane = reference_layer_plane(family, 0, reference)
        .ok_or(MeshPlaneIntersectionError::InvalidPlane)?;
    let normal = DVec3::from_array(first_plane.normal_mm.map(f64::from));
    let origin = DVec3::from_array(first_plane.origin_mm.map(f64::from));
    // The distance between neighbouring layer planes along the normal; a single layer has no
    // neighbour, so its thickness is the spacing along the axis.
    let step = if layer_count > 1 {
        let second = reference_layer_plane(family, 1, reference)
            .ok_or(MeshPlaneIntersectionError::InvalidPlane)?;
        (DVec3::from_array(second.origin_mm.map(f64::from)) - origin).dot(normal)
    } else {
        f64::from(reference.spacing()[depth_axis(family)])
    };
    if step == 0.0 || !step.is_finite() {
        return Err(MeshPlaneIntersectionError::InvalidPlane);
    }
    let welded = WeldedMesh::new(mesh)?;
    let layers = cut_layers(
        &welded,
        |point| {
            (DVec3::new(f64::from(point.x), f64::from(point.y), f64::from(point.z)) - origin)
                .dot(normal)
                / step
        },
        0..=layer_count as i64 - 1,
    );
    let mut slices = Vec::with_capacity(layers.len());
    for (layer, loops) in layers {
        let plane = reference_layer_plane(family, layer as u32, reference)
            .ok_or(MeshPlaneIntersectionError::InvalidPlane)?;
        slices.push(ContourSlice {
            plane,
            loops: to_local_loops(&loops, plane),
        });
    }
    Ok(ContourData {
        active_plane_family: family,
        slices,
    })
}

fn depth_axis(family: OrthogonalFamily) -> usize {
    match family {
        OrthogonalFamily::Axial => 2,
        OrthogonalFamily::Coronal => 1,
        OrthogonalFamily::Sagittal => 0,
    }
}

fn reference_depth_len(reference: VoxelGeometry, family: OrthogonalFamily) -> u32 {
    reference.dimensions()[depth_axis(family)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{
        extract_mesh_from_voxel_data, orthogonal_plane_from_volume_uv, PlaneFamily,
    };
    use crate::model::{MeshVertex, VoxelData};

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

        assert_eq!(contour.len(), 1);
        assert_eq!(contour[0].loops.len(), 1);
        assert!(contour[0].loops[0].is_valid_closed_loop());
    }

    #[test]
    fn test_plane_missing_mesh_returns_no_slices() {
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

        assert!(contour.is_empty());
    }

    use crate::convert::{
        extract_contours_in_grid, rasterize_contours_to_voxel_data, slice_center_uv,
    };
    use glam::Quat;

    fn random_mask(dimensions: [u32; 3], seed: &mut u32, density: u32) -> Vec<u8> {
        (0..dimensions.iter().product::<u32>())
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                u8::from((*seed).is_multiple_of(density))
            })
            .collect()
    }

    /// A surface cut at the layer centres and filled back must reproduce the voxels it came from:
    /// the mesh passes midway between occupied and empty centres, and a centre on the cutting
    /// plane is inside the cut exactly when it is inside the mesh. The surface of a random mask
    /// touches the cutting planes at vertices and along edges everywhere, which is the hard case.
    #[test]
    fn test_cutting_a_voxel_surface_layer_by_layer_and_filling_it_back_is_exact() {
        let dimensions = [11, 9, 8];
        let mut seed = 0x2468_ace1_u32;
        for case in 0..60 {
            let geometry = VoxelGeometry::new(
                dimensions,
                [[1.0, 1.0, 1.0], [0.7, 1.3, 2.1], [2.0, 2.0, 3.0]][case % 3],
                [10.0, 20.0, 30.0],
                Quat::from_rotation_y(0.4).to_array(),
            )
            .unwrap();
            let raw_data = random_mask(dimensions, &mut seed, [2, 3, 4][case % 3]);
            if raw_data.iter().all(|value| *value == 0) {
                continue;
            }
            let source = VoxelData { geometry, raw_data };
            let mesh = extract_mesh_from_voxel_data(&source).unwrap();
            for family in [
                OrthogonalFamily::Axial,
                OrthogonalFamily::Coronal,
                OrthogonalFamily::Sagittal,
            ] {
                let contours = contours_from_mesh(&mesh, geometry, family).unwrap();
                for slice in &contours.slices {
                    assert!(
                        slice.loops.iter().all(|loop_| loop_.is_valid_closed_loop()),
                        "case {case} {family:?}: every cut of a closed surface is a closed loop"
                    );
                }
                let filled = rasterize_contours_to_voxel_data(&contours, geometry).unwrap();
                let differing: Vec<usize> = (0..filled.raw_data.len())
                    .filter(|i| filled.raw_data[*i] != source.raw_data[*i])
                    .collect();
                assert!(
                    differing.is_empty(),
                    "case {case} {family:?}: cut and fill differ at {} voxels, first {:?}",
                    differing.len(),
                    &differing[..differing.len().min(6)]
                );
                // And the slices sit on the planes extraction from voxels uses.
                let extracted = extract_contours_in_grid(&source, family, geometry).unwrap();
                for slice in &extracted.slices {
                    let cut = contours
                        .slices
                        .iter()
                        .find(|cut| cut.plane == slice.plane)
                        .unwrap_or_else(|| {
                            panic!("case {case} {family:?}: a cut on the same plane")
                        });
                    assert!(!cut.loops.is_empty());
                }
            }
        }
        let _ = slice_center_uv;
    }

    #[test]
    fn test_a_smooth_surface_is_cut_into_its_true_cross_section() {
        // A sphere of radius 4 mm around a point between voxel centres, as a hand-made
        // icosphere-like mesh: cut at a plane through its centre, the loop is a circle of 4 mm.
        let centre = Vec3::new(5.3, 5.1, 5.2);
        let radius = 4.0_f32;
        let rings = 24;
        let sectors = 48;
        let mut vertices = vec![MeshVertex {
            world_mm: (centre + Vec3::Z * radius).to_array(),
        }];
        for ring in 1..rings {
            let polar = std::f32::consts::PI * ring as f32 / rings as f32;
            for sector in 0..sectors {
                let azimuth = std::f32::consts::TAU * sector as f32 / sectors as f32;
                vertices.push(MeshVertex {
                    world_mm: (centre
                        + radius
                            * Vec3::new(
                                polar.sin() * azimuth.cos(),
                                polar.sin() * azimuth.sin(),
                                polar.cos(),
                            ))
                    .to_array(),
                });
            }
        }
        vertices.push(MeshVertex {
            world_mm: (centre - Vec3::Z * radius).to_array(),
        });
        let south = vertices.len() as u32 - 1;
        let ring_start = |ring: u32| 1 + (ring - 1) * sectors as u32;
        let mut faces = Vec::new();
        for sector in 0..sectors as u32 {
            let next = (sector + 1) % sectors as u32;
            faces.push(crate::model::MeshFace {
                vertex_indices: [0, ring_start(1) + next, ring_start(1) + sector],
            });
            faces.push(crate::model::MeshFace {
                vertex_indices: [
                    south,
                    ring_start(rings as u32 - 1) + sector,
                    ring_start(rings as u32 - 1) + next,
                ],
            });
        }
        for ring in 1..rings as u32 - 1 {
            for sector in 0..sectors as u32 {
                let next = (sector + 1) % sectors as u32;
                let (a, b) = (ring_start(ring) + sector, ring_start(ring) + next);
                let (c, d) = (ring_start(ring + 1) + sector, ring_start(ring + 1) + next);
                faces.push(crate::model::MeshFace {
                    vertex_indices: [a, b, c],
                });
                faces.push(crate::model::MeshFace {
                    vertex_indices: [b, d, c],
                });
            }
        }
        let mesh = MeshData { vertices, faces };
        let plane = PlaneDefinition::new(
            crate::convert::PlaneFamily::Axial,
            [0.0, 0.0, 5.2],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        )
        .unwrap();
        let slices = intersect_mesh_with_plane(&mesh, plane).unwrap();
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].loops.len(), 1);
        for point in &slices[0].loops[0].points {
            let world = crate::convert::plane_local_mm_to_world_mm(point.local_mm, plane);
            let distance = (Vec3::from_array(world) - centre).length();
            // The facets lie inside the sphere by at most the sagitta of one facet.
            assert!((distance - radius).abs() < 0.05, "{distance}");
        }
    }
}
