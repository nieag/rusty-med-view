//! Volumes of a ROI's shape, from the form that is authoritative (ADR 0006).
//!
//! A contour ROI's volume is the sum over its slices of the area inside the loops times the layer
//! thickness, because a drawn slice stands for one layer; it needs no voxels and no mesh. A mesh's
//! volume is exact by the divergence theorem. Neither depends on a voxel grid, so neither carries
//! its quantisation.

use crate::convert::contour_field::family_axes;
use crate::model::{ContourData, MeshData, VoxelGeometry};

/// The volume enclosed by a closed, consistently wound mesh (sum of signed tetrahedra against the
/// origin, so the winding only decides the sign).
pub fn mesh_volume_mm3(mesh: &MeshData) -> f32 {
    let mut volume = 0.0_f64;
    for face in &mesh.faces {
        let [a, b, c] = face.vertex_indices.map(|index| {
            mesh.vertices
                .get(index as usize)
                .map_or([0.0; 3], |vertex| vertex.world_mm.map(f64::from))
        });
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    volume.abs() as f32
}

/// The volume of drawn contours: per slice the area inside the loops (a loop inside an odd number
/// of others is a hole) times the layer thickness of `reference` along the slice normal.
pub fn contour_volume_mm3(contour: &ContourData, reference: VoxelGeometry) -> f32 {
    let (depth_axis, _, _) = family_axes(contour.active_plane_family);
    let thickness = f64::from(reference.spacing()[depth_axis]);
    let area: f64 = contour
        .slices
        .iter()
        .map(|slice| {
            let loops: Vec<Vec<[f32; 2]>> = slice
                .loops
                .iter()
                .filter(|contour_loop| contour_loop.is_valid_closed_loop())
                .map(|contour_loop| contour_loop.points.iter().map(|p| p.local_mm).collect())
                .collect();
            slice_area_mm2(&loops)
        })
        .sum();
    (area * thickness) as f32
}

fn slice_area_mm2(loops: &[Vec<[f32; 2]>]) -> f64 {
    let mut area = 0.0_f64;
    for (index, points) in loops.iter().enumerate() {
        let nesting = loops
            .iter()
            .enumerate()
            .filter(|(other, outer)| *other != index && contains(outer, points[0]))
            .count();
        let magnitude = signed_area(points).abs();
        area += if nesting % 2 == 0 {
            magnitude
        } else {
            -magnitude
        };
    }
    area.max(0.0)
}

fn signed_area(points: &[[f32; 2]]) -> f64 {
    let mut twice = 0.0_f64;
    for k in 0..points.len() {
        let (a, b) = (points[k], points[(k + 1) % points.len()]);
        twice += f64::from(a[0]) * f64::from(b[1]) - f64::from(b[0]) * f64::from(a[1]);
    }
    twice / 2.0
}

/// Even-odd point in polygon.
fn contains(points: &[[f32; 2]], p: [f32; 2]) -> bool {
    let mut inside = false;
    for k in 0..points.len() {
        let (a, b) = (points[k], points[(k + 1) % points.len()]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ContourLoop, ContourPoint, ContourSlice, MeshFace, MeshVertex, OrthogonalFamily,
        PlaneDefinition, PlaneFamily,
    };

    fn square(half: f32) -> ContourLoop {
        ContourLoop {
            points: [[-half, -half], [half, -half], [half, half], [-half, half]]
                .map(|local_mm| ContourPoint { local_mm })
                .to_vec(),
            is_closed: true,
        }
    }

    fn slice(loops: Vec<ContourLoop>) -> ContourSlice {
        ContourSlice {
            plane: PlaneDefinition {
                family: PlaneFamily::Axial,
                origin_mm: [0.0; 3],
                u_axis_mm: [1.0, 0.0, 0.0],
                v_axis_mm: [0.0, 1.0, 0.0],
                normal_mm: [0.0, 0.0, 1.0],
            },
            loops,
        }
    }

    #[test]
    fn test_contour_volume_is_area_times_layer_thickness_and_holes_subtract() {
        let reference =
            VoxelGeometry::new([4, 4, 4], [1.0, 1.0, 2.5], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let contour = ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            // A 4 x 4 square with a 2 x 2 hole, and a plain 2 x 2 square on another slice.
            slices: vec![
                slice(vec![square(2.0), square(1.0)]),
                slice(vec![square(1.0)]),
            ],
        };
        let expected = ((16.0 - 4.0) + 4.0) * 2.5;
        assert!((contour_volume_mm3(&contour, reference) - expected).abs() < 1e-4);
    }

    #[test]
    fn test_mesh_volume_of_a_tetrahedron_is_a_sixth_of_its_box() {
        let vertex = |world_mm| MeshVertex { world_mm };
        let mesh = MeshData {
            vertices: vec![
                vertex([1.0, 1.0, 1.0]),
                vertex([3.0, 1.0, 1.0]),
                vertex([1.0, 3.0, 1.0]),
                vertex([1.0, 1.0, 3.0]),
            ],
            faces: [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]
                .map(|vertex_indices| MeshFace { vertex_indices })
                .to_vec(),
        };
        assert!((mesh_volume_mm3(&mesh) - 8.0 / 6.0).abs() < 1e-5);
    }
}
