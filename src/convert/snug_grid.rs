//! Snug voxel grids: the smallest box of a reference grid that holds a shape.
//!
//! Voxels are only a hub between contours, meshes, and the display, so a ROI that is not stored
//! as voxels holds them only in a box around its shape, never over the whole reference grid. The
//! box is a sub-grid of the reference grid (same lattice, see `VoxelGeometry::offset_in`), so
//! world positions and voxel layers agree with the full grid. Boxes only grow while a ROI is
//! edited: a stable box keeps the retained voxel cache and the mesh chunks reusable.

use crate::convert::{contour_geometry_voxel_aabb, world_mm_to_voxel_index};
use crate::model::{ContourData, MeshData, VoxelGeometry};

/// A voxel box as `(min, max_exclusive)` in the reference grid's voxel indices.
pub type VoxelBox = ([u32; 3], [u32; 3]);

/// Voxels of margin around a shape, so its surface never sits on the box's faces.
const MARGIN: u32 = 1;

/// The box of `reference` that holds the contour loops (and the depth range of its slices), with
/// a margin, grown to also cover `keep` when that is given.
pub fn snug_geometry_for_contour(
    contour: &ContourData,
    reference: VoxelGeometry,
    keep: Option<VoxelGeometry>,
) -> VoxelGeometry {
    snug_geometry(
        contour_geometry_voxel_aabb(contour, reference),
        reference,
        keep,
    )
}

/// The box of `reference` that holds the mesh vertices, with a margin, grown to also cover
/// `keep` when that is given.
pub fn snug_geometry_for_mesh(
    mesh: &MeshData,
    reference: VoxelGeometry,
    keep: Option<VoxelGeometry>,
) -> VoxelGeometry {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        let index = world_mm_to_voxel_index(vertex.world_mm, reference);
        if index.iter().any(|value| !value.is_finite()) {
            continue;
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(index[axis]);
            max[axis] = max[axis].max(index[axis]);
        }
    }
    let bounds = (min[0].is_finite()).then(|| {
        let dimensions = reference.dimensions();
        let low = std::array::from_fn(|axis| {
            min[axis].floor().clamp(0.0, dimensions[axis] as f32) as u32
        });
        let high = std::array::from_fn(|axis| {
            (max[axis].ceil() + 1.0).clamp(0.0, dimensions[axis] as f32) as u32
        });
        (low, high)
    });
    snug_geometry(bounds, reference, keep)
}

fn snug_geometry(
    bounds: Option<VoxelBox>,
    reference: VoxelGeometry,
    keep: Option<VoxelGeometry>,
) -> VoxelGeometry {
    let dimensions = reference.dimensions();
    let mut wanted: Option<VoxelBox> = bounds.map(|(min, max)| {
        (
            min.map(|value| value.saturating_sub(MARGIN)),
            std::array::from_fn(|axis| (max[axis] + MARGIN).min(dimensions[axis])),
        )
    });
    if let Some((offset, kept)) = keep.and_then(|kept| Some((kept.offset_in(reference)?, kept))) {
        let kept_max: [u32; 3] = std::array::from_fn(|axis| offset[axis] + kept.dimensions()[axis]);
        wanted = Some(match wanted {
            Some((min, max)) => (
                std::array::from_fn(|axis| min[axis].min(offset[axis])),
                std::array::from_fn(|axis| max[axis].max(kept_max[axis])),
            ),
            None => (offset, kept_max),
        });
    }
    // A shape with nothing in it still needs a one-voxel box to be a valid grid.
    let (min, max) = wanted
        .filter(|(min, max)| (0..3).all(|axis| min[axis] < max[axis]))
        .unwrap_or(([0; 3], [1; 3]));
    let box_dimensions = std::array::from_fn(|axis| max[axis] - min[axis]);
    reference
        .cropped(min, box_dimensions)
        .expect("a box inside a valid grid is a valid grid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContourLoop, ContourPoint, ContourSlice, MeshVertex, OrthogonalFamily};

    fn reference() -> VoxelGeometry {
        VoxelGeometry::new([20, 20, 20], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()
    }

    fn mesh(points: &[[f32; 3]]) -> MeshData {
        MeshData {
            vertices: points
                .iter()
                .map(|world_mm| MeshVertex {
                    world_mm: *world_mm,
                })
                .collect(),
            faces: Vec::new(),
        }
    }

    #[test]
    fn test_a_mesh_box_covers_its_vertices_with_a_margin_inside_the_grid() {
        let geometry = snug_geometry_for_mesh(
            &mesh(&[[5.2, 6.0, 7.0], [8.7, 9.0, 10.0]]),
            reference(),
            None,
        );
        assert_eq!(geometry.offset_in(reference()), Some([4, 5, 6]));
        assert_eq!(geometry.dimensions(), [7, 6, 6]);
        // A shape at the grid's edge is clamped to the grid.
        let edge = snug_geometry_for_mesh(
            &mesh(&[[0.0, 0.0, 0.0], [19.0, 19.0, 19.0]]),
            reference(),
            None,
        );
        assert_eq!(edge.offset_in(reference()), Some([0, 0, 0]));
        assert_eq!(edge.dimensions(), [20, 20, 20]);
    }

    #[test]
    fn test_a_box_only_grows_when_asked_to_keep_the_previous_one() {
        let first = snug_geometry_for_mesh(
            &mesh(&[[5.0, 5.0, 5.0], [6.0, 6.0, 6.0]]),
            reference(),
            None,
        );
        let moved = snug_geometry_for_mesh(
            &mesh(&[[10.0, 10.0, 10.0], [11.0, 11.0, 11.0]]),
            reference(),
            Some(first),
        );
        let offset = moved.offset_in(reference()).unwrap();
        assert_eq!(offset, [4, 4, 4], "the old box is still inside the new one");
        assert!(moved.dimensions().iter().all(|d| *d >= 9));
        // A shape that fits in the previous box leaves it unchanged.
        let inside = snug_geometry_for_mesh(&mesh(&[[5.0, 5.0, 5.0]]), reference(), Some(moved));
        assert_eq!(inside.identity(), moved.identity());
    }

    #[test]
    fn test_an_empty_shape_gets_a_one_voxel_box_or_keeps_the_previous_one() {
        let empty = snug_geometry_for_mesh(&mesh(&[]), reference(), None);
        assert_eq!(empty.dimensions(), [1, 1, 1]);
        let kept = snug_geometry_for_mesh(&mesh(&[]), reference(), Some(empty));
        assert_eq!(kept.identity(), empty.identity());
    }

    #[test]
    fn test_a_contour_box_covers_its_loops_and_slice_depths() {
        let plane = crate::convert::orthogonal_plane_from_volume_uv(
            crate::convert::PlaneFamily::Axial,
            [0.5, 0.5, 8.5 / 20.0],
            reference(),
        )
        .unwrap();
        let contour = ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]
                        .map(|local_mm| ContourPoint { local_mm })
                        .to_vec(),
                    is_closed: true,
                }],
            }],
        };
        let geometry = snug_geometry_for_contour(&contour, reference(), None);
        let offset = geometry.offset_in(reference()).unwrap();
        let dimensions = geometry.dimensions();
        // The slice is at layer 8; x and y span about 4 voxels around the plane centre.
        assert!(offset[2] <= 8 && 8 < offset[2] + dimensions[2]);
        assert!(
            dimensions[2] <= 3,
            "a single slice needs a thin box: {dimensions:?}"
        );
        assert!(dimensions[0] >= 4 && dimensions[0] <= 8, "{dimensions:?}");
    }
}
