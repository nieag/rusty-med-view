use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rusty_med_view::app::segment::SdfVolume;
use rusty_med_view::convert::{marching_cubes, surface_nets_from_sdf};

/// Build a sphere SDF volume of `size^3` voxels with voxel spacing 1 mm.
/// Returns values: distance to a sphere of radius size/3 centered in the grid.
fn build_sphere_sdf(size: u32) -> SdfVolume {
    let dims = [size, size, size];
    let spacing = [1.0f32; 3];
    let origin = [0.0f32; 3];
    let mut sdf = SdfVolume::new(dims, spacing, origin);
    let center = size as f32 * 0.5;
    let radius = size as f32 / 3.0;
    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dz = z as f32 - center;
                let dist = (dx * dx + dy * dy + dz * dz).sqrt() - radius;
                sdf.set(x, y, z, dist);
            }
        }
    }
    sdf
}

fn bench_marching_cubes(c: &mut Criterion) {
    let mut group = c.benchmark_group("marching_cubes");
    for size in [32u32, 64, 128] {
        let sdf = build_sphere_sdf(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &sdf, |b, sdf| {
            b.iter(|| marching_cubes(black_box(sdf), black_box(0.0)))
        });
    }
    group.finish();
}

fn bench_surface_nets(c: &mut Criterion) {
    let mut group = c.benchmark_group("surface_nets");
    for size in [32u32, 64, 128] {
        let sdf = build_sphere_sdf(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &sdf, |b, sdf| {
            b.iter(|| surface_nets_from_sdf(black_box(sdf), black_box(0.0), black_box(None)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_marching_cubes, bench_surface_nets);
criterion_main!(benches);
