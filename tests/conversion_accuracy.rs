//! How much of a smooth shape survives the conversion chains (backlog 2c.1, ADR 0006).
//!
//! Smooth analytic shapes (a sphere and a thin plate) are drawn as exact contours on the layer
//! planes of a grid, then converted the ways the viewer converts them. Each result is compared
//! with the true shape: the volume error and the distance of the result's surface points to the
//! true surface. The numbers printed are the record; the assertions only catch an order of
//! magnitude regression. As chains improve, add them here and tighten the bounds.
//!
//! Run with `cargo test --release --test conversion_accuracy -- --nocapture`.
use rusty_med_view::convert::{
    contours_from_mesh, extract_contours_from_voxel_data, extract_mesh_from_voxel_data,
    orthogonal_plane_from_volume_uv, plane_local_mm_to_world_mm, rasterize_contours_to_voxel_data,
    slice_center_uv, voxel_index_to_world_mm, IncrementalMeshVoxelization, PlaneFamily,
};
use rusty_med_view::model::{
    ContourData, ContourLoop, ContourPoint, ContourSlice, MeshData, OrthogonalFamily, VoxelData,
    VoxelGeometry,
};

/// An analytic shape: the signed distance to its surface (negative inside) and its volume.
#[derive(Clone, Copy)]
enum Shape {
    Sphere { centre: [f32; 3], radius: f32 },
    Plate { centre: [f32; 3], half: [f32; 3] },
}

impl Shape {
    fn signed_distance(&self, p: [f32; 3]) -> f32 {
        match *self {
            Shape::Sphere { centre, radius } => {
                let d = [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - radius
            }
            Shape::Plate { centre, half } => {
                let q = [
                    (p[0] - centre[0]).abs() - half[0],
                    (p[1] - centre[1]).abs() - half[1],
                    (p[2] - centre[2]).abs() - half[2],
                ];
                let outside =
                    (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2) + q[2].max(0.0).powi(2)).sqrt();
                outside + q[0].max(q[1]).max(q[2]).min(0.0)
            }
        }
    }

    fn volume(&self) -> f32 {
        match *self {
            Shape::Sphere { radius, .. } => 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3),
            Shape::Plate { half, .. } => 8.0 * half[0] * half[1] * half[2],
        }
    }

    /// The exact cross-section at world height `z` as a loop of world points (an axial cut).
    fn axial_section(&self, z: f32) -> Option<Vec<[f32; 3]>> {
        match *self {
            Shape::Sphere { centre, radius } => {
                let dz = z - centre[2];
                let r2 = radius * radius - dz * dz;
                (r2 > 0.0).then(|| {
                    let r = r2.sqrt();
                    (0..512)
                        .map(|i| {
                            let a = std::f32::consts::TAU * i as f32 / 512.0;
                            [centre[0] + r * a.cos(), centre[1] + r * a.sin(), z]
                        })
                        .collect()
                })
            }
            Shape::Plate { centre, half } => ((z - centre[2]).abs() <= half[2]).then(|| {
                vec![
                    [centre[0] - half[0], centre[1] - half[1], z],
                    [centre[0] + half[0], centre[1] - half[1], z],
                    [centre[0] + half[0], centre[1] + half[1], z],
                    [centre[0] - half[0], centre[1] + half[1], z],
                ]
            }),
        }
    }
}

struct Case {
    name: &'static str,
    geometry: VoxelGeometry,
    shape: Shape,
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (label, spacing) in [
        ("1 mm cube voxels", [1.0, 1.0, 1.0]),
        ("1 x 1 x 2.5 mm", [1.0, 1.0, 2.5]),
    ] {
        let geometry =
            VoxelGeometry::new([40, 40, 32], spacing, [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        // Centres that fall between voxel centres, so nothing lines up by accident.
        let centre = [20.3 * spacing[0], 19.8 * spacing[1], 15.6 * spacing[2]];
        cases.push(Case {
            name: Box::leak(format!("sphere r=5.3 mm, {label}").into_boxed_str()),
            geometry,
            shape: Shape::Sphere {
                centre,
                radius: 5.3,
            },
        });
        cases.push(Case {
            name: Box::leak(format!("plate 14 x 14 x 1.4 voxels thick, {label}").into_boxed_str()),
            geometry,
            shape: Shape::Plate {
                centre,
                half: [7.0 * spacing[0], 7.0 * spacing[1], 0.7 * spacing[2]],
            },
        });
    }
    cases
}

/// The shape drawn as exact contours on every layer plane of the grid.
fn drawn_contours(case: &Case) -> ContourData {
    let [_, _, layers] = case.geometry.dimensions();
    let mut slices = Vec::new();
    for layer in 0..layers {
        let plane = orthogonal_plane_from_volume_uv(
            PlaneFamily::Axial,
            [0.5, 0.5, slice_center_uv(layer as i32, layers)],
            case.geometry,
        )
        .unwrap();
        if let Some(world_loop) = case.shape.axial_section(plane.origin_mm[2]) {
            let points = world_loop
                .iter()
                .map(|world| {
                    let delta = [world[0] - plane.origin_mm[0], world[1] - plane.origin_mm[1]];
                    ContourPoint {
                        local_mm: [
                            delta[0] * plane.u_axis_mm[0] + delta[1] * plane.u_axis_mm[1],
                            delta[0] * plane.v_axis_mm[0] + delta[1] * plane.v_axis_mm[1],
                        ],
                    }
                })
                .collect();
            slices.push(ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points,
                    is_closed: true,
                }],
            });
        }
    }
    ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices,
    }
}

/// The shape as a binary labelmap, as a segmentation file would hold it.
fn truth_mask(case: &Case) -> VoxelData {
    let [w, h, d] = case.geometry.dimensions().map(|v| v as usize);
    let mut raw = vec![0u8; w * h * d];
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                let p = voxel_index_to_world_mm([x as f32, y as f32, z as f32], case.geometry);
                raw[(z * h + y) * w + x] = u8::from(case.shape.signed_distance(p) < 0.0);
            }
        }
    }
    VoxelData {
        geometry: case.geometry,
        raw_data: raw,
    }
}

fn mesh_volume(mesh: &MeshData) -> f32 {
    let mut volume = 0.0_f64;
    for face in &mesh.faces {
        let [a, b, c] = face
            .vertex_indices
            .map(|i| mesh.vertices[i as usize].world_mm.map(f64::from));
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    volume.abs() as f32
}

struct Measure {
    volume_error_percent: f32,
    mean_mm: f32,
    max_mm: f32,
}

fn measure_points(shape: &Shape, points: impl Iterator<Item = [f32; 3]>, volume: f32) -> Measure {
    let (mut sum, mut max, mut count) = (0.0_f32, 0.0_f32, 0usize);
    for p in points {
        let distance = shape.signed_distance(p).abs();
        sum += distance;
        max = max.max(distance);
        count += 1;
    }
    Measure {
        volume_error_percent: 100.0 * (volume - shape.volume()) / shape.volume(),
        mean_mm: sum / count.max(1) as f32,
        max_mm: max,
    }
}

fn mesh_measure(shape: &Shape, mesh: &MeshData) -> Measure {
    measure_points(
        shape,
        mesh.vertices.iter().map(|v| v.world_mm),
        mesh_volume(mesh),
    )
}

/// Contour points are on the surface only in their own plane, so the distance of a loop point to
/// the true surface is the in-plane error of that contour. The volume is the mask volume.
fn contour_measure(shape: &Shape, contours: &ContourData, volume: f32) -> Measure {
    let points = contours.slices.iter().flat_map(|slice| {
        slice.loops.iter().flat_map(move |contour_loop| {
            contour_loop
                .points
                .iter()
                .map(move |p| plane_local_mm_to_world_mm(p.local_mm, slice.plane))
        })
    });
    measure_points(shape, points, volume)
}

fn mask_volume(mask: &VoxelData) -> f32 {
    let spacing = mask.geometry.spacing();
    mask.raw_data.iter().filter(|v| **v != 0).count() as f32 * spacing[0] * spacing[1] * spacing[2]
}

fn report(name: &str, chain: &str, measure: &Measure) {
    println!(
        "{name:52} {chain:34} volume {:+6.1} %   surface mean {:5.3} max {:5.3} mm",
        measure.volume_error_percent, measure.mean_mm, measure.max_mm
    );
}

#[test]
fn test_conversion_chains_against_analytic_shapes() {
    for case in cases() {
        let drawn = drawn_contours(&case);
        let mask = truth_mask(&case);

        // Reference: the shape as a mask (what a labelmap holds), and its contours.
        let mask_contours =
            extract_contours_from_voxel_data(&mask, OrthogonalFamily::Axial).unwrap();
        report(
            case.name,
            "mask: voxel count",
            &contour_measure(&case.shape, &mask_contours, mask_volume(&mask)),
        );

        // Chain 1: drawn contours -> mask (rasterize) -> mesh.
        let rasterized = rasterize_contours_to_voxel_data(&drawn, case.geometry).unwrap();
        let mesh = extract_mesh_from_voxel_data(&rasterized).unwrap();
        let chain1 = mesh_measure(&case.shape, &mesh);
        report(case.name, "contours -> voxels -> mesh", &chain1);

        // Chain 2: that mesh -> voxels -> contours (what a mesh to contour switch does today).
        let mut work = IncrementalMeshVoxelization::begin(&mesh, case.geometry).unwrap();
        work.step(usize::MAX);
        let voxelized = work.into_result().unwrap();
        let back = extract_contours_from_voxel_data(&voxelized, OrthogonalFamily::Axial).unwrap();
        let chain2 = contour_measure(&case.shape, &back, mask_volume(&voxelized));
        report(case.name, "... -> voxels -> contours", &chain2);

        // Chain 3: the same mesh cut with the layer planes (what the switch does now).
        let cut = contours_from_mesh(&mesh, case.geometry, OrthogonalFamily::Axial).unwrap();
        let chain3 = contour_measure(&case.shape, &cut, mesh_volume(&mesh));
        report(case.name, "mesh -> contours (cut)", &chain3);

        // Order-of-magnitude guards on the current chains (a voxel is at least 1 mm here).
        assert!(
            chain1.max_mm < 2.0,
            "{}: mesh surface error {}",
            case.name,
            chain1.max_mm
        );
        assert!(
            chain2.max_mm < 2.0,
            "{}: contour error {}",
            case.name,
            chain2.max_mm
        );
    }
}
