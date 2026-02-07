# Phase 5: SDF → Mesh (Marching Cubes)

## Goal

Generate a triangle mesh from the SDF using the Marching Cubes algorithm.

## Files to Create

### [NEW] `src/convert/marching_cubes.rs`

## Algorithm Overview

Marching Cubes examines each 2×2×2 cell of voxels:
1. Classify vertices as inside/outside based on SDF sign
2. Look up edge configuration from table
3. Interpolate vertex positions along edges
4. Generate triangles

## Core Implementation

### `src/convert/marching_cubes.rs`

```rust
use crate::app::segment::{SdfVolume, MeshData};

/// Edge table: which edges are intersected for each of 256 cases
/// Each bit represents one of 12 edges
const EDGE_TABLE: [u16; 256] = [
    0x000, 0x109, 0x203, 0x30a, 0x406, 0x50f, 0x605, 0x70c,
    0x80c, 0x905, 0xa0f, 0xb06, 0xc0a, 0xd03, 0xe09, 0xf00,
    // ... (full 256-entry table)
    // See: http://paulbourke.net/geometry/polygonise/
];

/// Triangle table: which triangles to generate for each case
/// Each row is up to 5 triangles (15 indices), terminated by -1
const TRI_TABLE: [[i8; 16]; 256] = [
    [-1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 8, 3, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    // ... (full 256-entry table)
];

/// Generate mesh from SDF using Marching Cubes
pub fn marching_cubes(sdf: &SdfVolume, iso_value: f32) -> MeshData {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    
    let [dx, dy, dz] = sdf.dimensions;
    
    for z in 0..(dz - 1) {
        for y in 0..(dy - 1) {
            for x in 0..(dx - 1) {
                process_cell(
                    sdf, x, y, z, iso_value,
                    &mut vertices, &mut indices,
                );
            }
        }
    }
    
    let mut mesh = MeshData {
        vertices,
        normals: Vec::new(),
        indices,
    };
    
    // Compute normals
    compute_normals_from_sdf(&mut mesh, sdf);
    
    mesh
}

/// Process a single 2×2×2 cell
fn process_cell(
    sdf: &SdfVolume,
    x: u32, y: u32, z: u32,
    iso_value: f32,
    vertices: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
) {
    // Get 8 corner values
    let v = [
        sdf.get(x,     y,     z),
        sdf.get(x + 1, y,     z),
        sdf.get(x + 1, y + 1, z),
        sdf.get(x,     y + 1, z),
        sdf.get(x,     y,     z + 1),
        sdf.get(x + 1, y,     z + 1),
        sdf.get(x + 1, y + 1, z + 1),
        sdf.get(x,     y + 1, z + 1),
    ];
    
    // Determine cube index (which vertices are inside)
    let mut cube_index = 0u8;
    for i in 0..8 {
        if v[i] < iso_value {
            cube_index |= 1 << i;
        }
    }
    
    // Skip if entirely inside or outside
    let edges = EDGE_TABLE[cube_index as usize];
    if edges == 0 {
        return;
    }
    
    // Get corner world positions
    let p = cell_corners(sdf, x, y, z);
    
    // Interpolate edge vertices
    let mut edge_verts = [[0.0f32; 3]; 12];
    
    if edges & 0x001 != 0 { edge_verts[0]  = interpolate(p[0], p[1], v[0], v[1], iso_value); }
    if edges & 0x002 != 0 { edge_verts[1]  = interpolate(p[1], p[2], v[1], v[2], iso_value); }
    if edges & 0x004 != 0 { edge_verts[2]  = interpolate(p[2], p[3], v[2], v[3], iso_value); }
    if edges & 0x008 != 0 { edge_verts[3]  = interpolate(p[3], p[0], v[3], v[0], iso_value); }
    if edges & 0x010 != 0 { edge_verts[4]  = interpolate(p[4], p[5], v[4], v[5], iso_value); }
    if edges & 0x020 != 0 { edge_verts[5]  = interpolate(p[5], p[6], v[5], v[6], iso_value); }
    if edges & 0x040 != 0 { edge_verts[6]  = interpolate(p[6], p[7], v[6], v[7], iso_value); }
    if edges & 0x080 != 0 { edge_verts[7]  = interpolate(p[7], p[4], v[7], v[4], iso_value); }
    if edges & 0x100 != 0 { edge_verts[8]  = interpolate(p[0], p[4], v[0], v[4], iso_value); }
    if edges & 0x200 != 0 { edge_verts[9]  = interpolate(p[1], p[5], v[1], v[5], iso_value); }
    if edges & 0x400 != 0 { edge_verts[10] = interpolate(p[2], p[6], v[2], v[6], iso_value); }
    if edges & 0x800 != 0 { edge_verts[11] = interpolate(p[3], p[7], v[3], v[7], iso_value); }
    
    // Generate triangles
    let tri_row = &TRI_TABLE[cube_index as usize];
    let base_index = vertices.len() as u32;
    
    let mut i = 0;
    while tri_row[i] >= 0 {
        let e0 = tri_row[i] as usize;
        let e1 = tri_row[i + 1] as usize;
        let e2 = tri_row[i + 2] as usize;
        
        vertices.push(edge_verts[e0]);
        vertices.push(edge_verts[e1]);
        vertices.push(edge_verts[e2]);
        
        indices.push(base_index + i as u32);
        indices.push(base_index + i as u32 + 1);
        indices.push(base_index + i as u32 + 2);
        
        i += 3;
    }
}

/// Get world-space positions of cell corners
fn cell_corners(sdf: &SdfVolume, x: u32, y: u32, z: u32) -> [[f32; 3]; 8] {
    [
        sdf.index_to_world([x,     y,     z]),
        sdf.index_to_world([x + 1, y,     z]),
        sdf.index_to_world([x + 1, y + 1, z]),
        sdf.index_to_world([x,     y + 1, z]),
        sdf.index_to_world([x,     y,     z + 1]),
        sdf.index_to_world([x + 1, y,     z + 1]),
        sdf.index_to_world([x + 1, y + 1, z + 1]),
        sdf.index_to_world([x,     y + 1, z + 1]),
    ]
}

/// Interpolate vertex position along edge based on SDF values
fn interpolate(
    p1: [f32; 3], p2: [f32; 3],
    v1: f32, v2: f32,
    iso_value: f32,
) -> [f32; 3] {
    if (v2 - v1).abs() < 1e-10 {
        return p1;
    }
    
    let t = (iso_value - v1) / (v2 - v1);
    let t = t.clamp(0.0, 1.0);
    
    [
        p1[0] + t * (p2[0] - p1[0]),
        p1[1] + t * (p2[1] - p1[1]),
        p1[2] + t * (p2[2] - p1[2]),
    ]
}
```

## Normal Computation

```rust
/// Compute normals from SDF gradient (smooth)
pub fn compute_normals_from_sdf(mesh: &mut MeshData, sdf: &SdfVolume) {
    mesh.normals = mesh.vertices
        .iter()
        .map(|&v| {
            let grad = sdf_gradient(sdf, v);
            normalize(grad)
        })
        .collect();
}

/// Compute SDF gradient at world position using central differences
fn sdf_gradient(sdf: &SdfVolume, pos: [f32; 3]) -> [f32; 3] {
    let eps = sdf.spacing[0];  // Use voxel spacing
    
    let dx = sample_sdf(sdf, [pos[0] + eps, pos[1], pos[2]])
           - sample_sdf(sdf, [pos[0] - eps, pos[1], pos[2]]);
    let dy = sample_sdf(sdf, [pos[0], pos[1] + eps, pos[2]])
           - sample_sdf(sdf, [pos[0], pos[1] - eps, pos[2]]);
    let dz = sample_sdf(sdf, [pos[0], pos[1], pos[2] + eps])
           - sample_sdf(sdf, [pos[0], pos[1], pos[2] - eps]);
    
    [dx, dy, dz]
}

/// Sample SDF at world position with trilinear interpolation
fn sample_sdf(sdf: &SdfVolume, pos: [f32; 3]) -> f32 {
    // Clamp to bounds and interpolate
    // ... (trilinear interpolation)
    0.0 // Placeholder
}

/// Compute normals from face normals (fast, faceted look)
pub fn compute_normals_from_faces(mesh: &mut MeshData) {
    mesh.normals = vec![[0.0; 3]; mesh.vertices.len()];
    
    for i in (0..mesh.indices.len()).step_by(3) {
        let i0 = mesh.indices[i] as usize;
        let i1 = mesh.indices[i + 1] as usize;
        let i2 = mesh.indices[i + 2] as usize;
        
        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];
        
        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let n = cross(e1, e2);
        
        // Accumulate face normal to each vertex
        for &idx in &[i0, i1, i2] {
            mesh.normals[idx][0] += n[0];
            mesh.normals[idx][1] += n[1];
            mesh.normals[idx][2] += n[2];
        }
    }
    
    // Normalize
    for n in &mut mesh.normals {
        *n = normalize(*n);
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 { return [0.0, 0.0, 1.0]; }
    [v[0] / len, v[1] / len, v[2] / len]
}
```

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_midpoint() {
        // Equal values = midpoint
        let p = interpolate([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], 0.5, 0.5, 0.5);
        assert!((p[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_interpolate_weighted() {
        // v1=0, v2=1, iso=0.25 → t=0.25
        let p = interpolate([0.0, 0.0, 0.0], [4.0, 0.0, 0.0], 0.0, 1.0, 0.25);
        assert!((p[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_marching_cubes_sphere() {
        // Create SDF for sphere of radius 5 centered at (10, 10, 10)
        let mut sdf = SdfVolume::new([20, 20, 20], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        
        for z in 0..20 {
            for y in 0..20 {
                for x in 0..20 {
                    let pos = sdf.index_to_world([x, y, z]);
                    let dist = ((pos[0] - 10.0).powi(2) 
                              + (pos[1] - 10.0).powi(2) 
                              + (pos[2] - 10.0).powi(2)).sqrt() - 5.0;
                    sdf.set(x, y, z, dist);
                }
            }
        }
        
        let mesh = marching_cubes(&sdf, 0.0);
        
        // Should have triangles
        assert!(!mesh.indices.is_empty());
        
        // Rough check: sphere surface area = 4πr² ≈ 314
        // With 1mm voxels, expect ~100-500 triangles
        assert!(mesh.triangle_count() > 50);
        assert!(mesh.triangle_count() < 1000);
    }

    #[test]
    fn test_mesh_is_closed() {
        // Generate sphere mesh
        // ... (use sphere SDF from above)
        
        // Check Euler characteristic: V - E + F = 2 for closed surface
        // F = triangle_count, V = vertex_count
        // E = 3F/2 for triangular mesh → V - 3F/2 + F = 2 → V = 2 + F/2
        // This is approximate due to shared vertices
    }

    #[test]
    fn test_normals_point_outward() {
        // Generate sphere mesh
        // ... 
        
        // For a sphere centered at (10,10,10), all normals should point outward
        // i.e., dot(normal, vertex - center) > 0
        let center = [10.0, 10.0, 10.0];
        for i in 0..mesh.vertices.len() {
            let v = mesh.vertices[i];
            let n = mesh.normals[i];
            let to_vertex = [v[0] - center[0], v[1] - center[1], v[2] - center[2]];
            let dot = n[0] * to_vertex[0] + n[1] * to_vertex[1] + n[2] * to_vertex[2];
            assert!(dot > 0.0, "Normal pointing inward at vertex {}", i);
        }
    }
}
```

## Verification

```bash
# Run phase 5 tests
cargo test marching_cubes::

# Expected: all tests pass
```

## Acceptance Criteria

- [ ] Marching Cubes generates valid mesh from SDF
- [ ] Interpolation places vertices correctly on zero-crossing
- [ ] Normals are computed and point outward
- [ ] Sphere SDF produces roughly spherical mesh
- [ ] Unit tests pass: `cargo test marching_cubes::`
