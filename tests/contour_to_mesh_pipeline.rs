/// Integration tests: contour → SDF → mesh pipeline end-to-end.
use rusty_med_view::app::segment::{Plane3D, PlaneContour};
use rusty_med_view::convert::{build_sdf_from_contours, surface_nets_from_sdf};
use rusty_med_view::orientation::SlicePlane;

/// Build a square contour centered at `(cx, cy, z_mm)` with half-side `hs`.
/// The SDF origin is at [0,0,0], so cx/cy should be within the volume bounds.
fn square_axial_contour(cx: f32, cy: f32, z_mm: f32, hs: f32) -> PlaneContour {
    PlaneContour::with_points(
        Plane3D::from_axial(z_mm),
        vec![
            [cx - hs, cy - hs, z_mm],
            [cx + hs, cy - hs, z_mm],
            [cx + hs, cy + hs, z_mm],
            [cx - hs, cy + hs, z_mm],
        ],
        true,
    )
}

#[test]
fn contour_set_to_mesh_pipeline() {
    // 20×20×20 voxels at 1 mm spacing; grid occupies world coords [0, 19] mm on each axis.
    let dims = [20u32, 20, 20];
    let spacing = [1.0f32, 1.0, 1.0];

    // Draw square contours centered at (10, 10) on 3 central axial slices.
    let mut contours = rusty_med_view::app::segment::ContourSet::new();
    for z in [8i32, 9, 10] {
        let contour = square_axial_contour(10.0, 10.0, z as f32, 4.0);
        contours.add_contour(SlicePlane::Axial, z, contour);
    }

    // Build SDF from contours
    let sdf = build_sdf_from_contours(&contours, dims, spacing, 1.0);

    // Voxel (10, 10, 9) is at world position (10, 10, 9) mm — center of the square → inside.
    let center_val = sdf.get(10, 10, 9);
    assert!(
        center_val < 0.0,
        "Expected center SDF < 0 (inside), got {center_val}"
    );

    // Extract mesh with Surface Nets
    let mesh = surface_nets_from_sdf(&sdf, 0.0, None);
    assert!(
        !mesh.vertices.is_empty(),
        "Expected non-empty vertex list from surface nets"
    );
    assert!(
        !mesh.indices.is_empty(),
        "Expected non-empty index list from surface nets"
    );
    // Indices must reference valid vertices
    for &idx in &mesh.indices {
        assert!(
            (idx as usize) < mesh.vertices.len(),
            "Index {idx} out of bounds (vertices: {})",
            mesh.vertices.len()
        );
    }
}
