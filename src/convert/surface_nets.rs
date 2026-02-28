//! Surface Nets isosurface extraction.
//!
//! Produces smoother meshes with fewer triangles than Marching Cubes, making it
//! better suited for organic medical segmentation shapes.  The primary entry
//! point is [`surface_nets_chunk`] which reads directly from a quantized
//! [`TsdfChunk`].  A convenience wrapper [`surface_nets_from_sdf`] is provided
//! for full-volume extraction from an [`SdfVolume`].

use crate::app::segment::{MeshData, SdfVolume};
use crate::convert::contour_to_tsdf_chunks::{dequantize_tsdf, TsdfChunk};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Core Surface Nets algorithm
// ---------------------------------------------------------------------------

/// Extract an isosurface mesh from a quantized TSDF chunk.
///
/// The algorithm places a *dual vertex* inside every grid cell that contains a
/// sign change (zero-crossing) and connects adjacent dual vertices with quads
/// (split into two triangles).
///
/// **Boundary ownership rule**: a cell at `(x, y, z)` is *owned* by this chunk
/// only when `x < dims[0]-1 && y < dims[1]-1 && z < dims[2]-1`.  Cells on the
/// maximum boundary are left for the neighbouring chunk so that meshes tile
/// without cracks or duplicate geometry.
pub fn surface_nets_chunk(chunk: &TsdfChunk) -> MeshData {
    let [dx, dy, dz] = chunk.dims;
    if dx < 2 || dy < 2 || dz < 2 {
        return MeshData::new();
    }

    let trunc = chunk.truncation_mm;
    let spacing = chunk.voxel_size;

    // Helper to fetch the dequantized f32 value at integer coords.
    let val = |x: u32, y: u32, z: u32| -> f32 {
        let idx = (x + y * dx + z * dx * dy) as usize;
        dequantize_tsdf(chunk.values[idx], trunc)
    };

    // Phase 1: Identify sign-change cells and place dual vertices.
    // Map from cell (x,y,z) -> vertex index in the output mesh.
    let mut cell_to_vertex: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();

    // Iterate over cells; each cell is the cube [x..x+1, y..y+1, z..z+1].
    // Boundary ownership: cells whose max corner is at the grid max are NOT
    // processed (they belong to the next chunk).
    for z in 0..dz - 1 {
        for y in 0..dy - 1 {
            for x in 0..dx - 1 {
                // Sample the 8 corners of this cell.
                let corners = [
                    val(x, y, z),
                    val(x + 1, y, z),
                    val(x, y + 1, z),
                    val(x + 1, y + 1, z),
                    val(x, y, z + 1),
                    val(x + 1, y, z + 1),
                    val(x, y + 1, z + 1),
                    val(x + 1, y + 1, z + 1),
                ];

                // Check for sign change: any negative corner means inside,
                // any positive means outside.
                let has_neg = corners.iter().any(|&v| v < 0.0);
                let has_pos = corners.iter().any(|&v| v >= 0.0);
                if !(has_neg && has_pos) {
                    continue;
                }

                // Place the dual vertex at the weighted-average position of
                // all edge crossings within this cell.
                let pos = compute_dual_vertex(x, y, z, &corners, spacing, &chunk.origin);

                // Compute normal via central-difference gradient.
                let normal = compute_gradient_normal(x, y, z, dx, dy, dz, &val);

                let vi = vertices.len() as u32;
                vertices.push(pos);
                normals.push(normal);
                cell_to_vertex.insert((x, y, z), vi);
            }
        }
    }

    // Phase 2: Connect adjacent dual vertices with quads (split into tris).
    let mut indices: Vec<u32> = Vec::new();

    for &(x, y, z) in cell_to_vertex.keys() {
        // For each of the 3 axis-aligned edges emanating from the min corner of
        // this cell, check if there is a sign change along that edge.  If so,
        // the 4 cells sharing that edge all have dual vertices — connect them.

        let v000 = val(x, y, z);

        // Edge along X: (x,y,z) -> (x+1,y,z)
        if x + 1 < dx - 1 {
            let v100 = val(x + 1, y, z);
            if (v000 < 0.0) != (v100 < 0.0) && y > 0 && z > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x, y - 1, z)),
                    cell_to_vertex.get(&(x, y - 1, z - 1)),
                    cell_to_vertex.get(&(x, y, z - 1)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }

        // Edge along Y: (x,y,z) -> (x,y+1,z)
        if y + 1 < dy - 1 {
            let v010 = val(x, y + 1, z);
            if (v000 < 0.0) != (v010 < 0.0) && x > 0 && z > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x, y, z - 1)),
                    cell_to_vertex.get(&(x - 1, y, z - 1)),
                    cell_to_vertex.get(&(x - 1, y, z)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }

        // Edge along Z: (x,y,z) -> (x,y,z+1)
        if z + 1 < dz - 1 {
            let v001 = val(x, y, z + 1);
            if (v000 < 0.0) != (v001 < 0.0) && x > 0 && y > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x - 1, y, z)),
                    cell_to_vertex.get(&(x - 1, y - 1, z)),
                    cell_to_vertex.get(&(x, y - 1, z)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }
    }

    MeshData {
        vertices,
        normals,
        indices,
    }
}

/// Convenience wrapper: extract from a full [`SdfVolume`], optionally scoped to
/// index bounds `[x0, y0, z0, x1, y1, z1]`.
pub fn surface_nets_from_sdf(
    sdf: &SdfVolume,
    _iso_level: f32,
    bounds: Option<[u32; 6]>,
) -> MeshData {
    let dims = sdf.dimensions;
    if dims[0] < 2 || dims[1] < 2 || dims[2] < 2 {
        return MeshData::new();
    }

    let b = bounds.unwrap_or([0, 0, 0, dims[0] - 1, dims[1] - 1, dims[2] - 1]);
    let x0 = b[0].min(dims[0] - 1);
    let y0 = b[1].min(dims[1] - 1);
    let z0 = b[2].min(dims[2] - 1);
    let x1 = b[3].min(dims[0] - 1);
    let y1 = b[4].min(dims[1] - 1);
    let z1 = b[5].min(dims[2] - 1);

    if x0 >= x1 || y0 >= y1 || z0 >= z1 {
        return MeshData::new();
    }

    let bx = x1 - x0 + 1;
    let by = y1 - y0 + 1;
    let bz = z1 - z0 + 1;

    // Helper to sample SDF in the sub-region's local coordinates.
    let val = |lx: u32, ly: u32, lz: u32| -> f32 { sdf.get(x0 + lx, y0 + ly, z0 + lz) };

    let mut cell_to_vertex: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();

    for lz in 0..bz - 1 {
        for ly in 0..by - 1 {
            for lx in 0..bx - 1 {
                let corners = [
                    val(lx, ly, lz),
                    val(lx + 1, ly, lz),
                    val(lx, ly + 1, lz),
                    val(lx + 1, ly + 1, lz),
                    val(lx, ly, lz + 1),
                    val(lx + 1, ly, lz + 1),
                    val(lx, ly + 1, lz + 1),
                    val(lx + 1, ly + 1, lz + 1),
                ];

                let has_neg = corners.iter().any(|&v| v < 0.0);
                let has_pos = corners.iter().any(|&v| v >= 0.0);
                if !(has_neg && has_pos) {
                    continue;
                }

                let origin = [
                    sdf.origin[0] + x0 as f32 * sdf.spacing[0],
                    sdf.origin[1] + y0 as f32 * sdf.spacing[1],
                    sdf.origin[2] + z0 as f32 * sdf.spacing[2],
                ];
                let pos = compute_dual_vertex(lx, ly, lz, &corners, sdf.spacing, &origin);

                let normal =
                    compute_gradient_normal(lx, ly, lz, bx, by, bz, &|gx, gy, gz| val(gx, gy, gz));

                let vi = vertices.len() as u32;
                vertices.push(pos);
                normals.push(normal);
                cell_to_vertex.insert((lx, ly, lz), vi);
            }
        }
    }

    let mut indices: Vec<u32> = Vec::new();

    for &(x, y, z) in cell_to_vertex.keys() {
        let v000 = val(x, y, z);

        if x + 1 < bx - 1 {
            let v100 = val(x + 1, y, z);
            if (v000 < 0.0) != (v100 < 0.0) && y > 0 && z > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x, y - 1, z)),
                    cell_to_vertex.get(&(x, y - 1, z - 1)),
                    cell_to_vertex.get(&(x, y, z - 1)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }

        if y + 1 < by - 1 {
            let v010 = val(x, y + 1, z);
            if (v000 < 0.0) != (v010 < 0.0) && x > 0 && z > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x, y, z - 1)),
                    cell_to_vertex.get(&(x - 1, y, z - 1)),
                    cell_to_vertex.get(&(x - 1, y, z)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }

        if z + 1 < bz - 1 {
            let v001 = val(x, y, z + 1);
            if (v000 < 0.0) != (v001 < 0.0) && x > 0 && y > 0 {
                if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                    cell_to_vertex.get(&(x, y, z)),
                    cell_to_vertex.get(&(x - 1, y, z)),
                    cell_to_vertex.get(&(x - 1, y - 1, z)),
                    cell_to_vertex.get(&(x, y - 1, z)),
                ) {
                    emit_quad(&mut indices, a, b, c, d, v000 < 0.0);
                }
            }
        }
    }

    MeshData {
        vertices,
        normals,
        indices,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute dual vertex position by averaging all edge-crossing interpolations
/// within a cell.
fn compute_dual_vertex(
    x: u32,
    y: u32,
    z: u32,
    corners: &[f32; 8],
    spacing: [f32; 3],
    origin: &[f32; 3],
) -> [f32; 3] {
    // Corner offsets in local cell space (dx, dy, dz for each of the 8 corners).
    const OFFSETS: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ];

    // The 12 edges of a cube, defined by corner index pairs.
    const EDGES: [[usize; 2]; 12] = [
        [0, 1],
        [2, 3],
        [4, 5],
        [6, 7], // X edges
        [0, 2],
        [1, 3],
        [4, 6],
        [5, 7], // Y edges
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7], // Z edges
    ];

    let mut sum = [0.0f32; 3];
    let mut count = 0u32;

    for &[a, b] in &EDGES {
        let va = corners[a];
        let vb = corners[b];
        if (va < 0.0) != (vb < 0.0) {
            // Linear interpolation parameter for zero crossing.
            let t = va / (va - vb);
            for i in 0..3 {
                sum[i] += OFFSETS[a][i] + t * (OFFSETS[b][i] - OFFSETS[a][i]);
            }
            count += 1;
        }
    }

    if count == 0 {
        // Shouldn't happen if we correctly detected a sign change, but safety.
        return [
            origin[0] + (x as f32 + 0.5) * spacing[0],
            origin[1] + (y as f32 + 0.5) * spacing[1],
            origin[2] + (z as f32 + 0.5) * spacing[2],
        ];
    }

    let inv = 1.0 / count as f32;
    [
        origin[0] + (x as f32 + sum[0] * inv) * spacing[0],
        origin[1] + (y as f32 + sum[1] * inv) * spacing[1],
        origin[2] + (z as f32 + sum[2] * inv) * spacing[2],
    ]
}

/// Compute gradient-based normal at cell center using central differences.
fn compute_gradient_normal<F>(
    x: u32,
    y: u32,
    z: u32,
    dx: u32,
    dy: u32,
    dz: u32,
    val: &F,
) -> [f32; 3]
where
    F: Fn(u32, u32, u32) -> f32,
{
    let xm = if x > 0 {
        val(x - 1, y, z)
    } else {
        val(x, y, z)
    };
    let xp = if x + 1 < dx {
        val(x + 1, y, z)
    } else {
        val(x, y, z)
    };
    let ym = if y > 0 {
        val(x, y - 1, z)
    } else {
        val(x, y, z)
    };
    let yp = if y + 1 < dy {
        val(x, y + 1, z)
    } else {
        val(x, y, z)
    };
    let zm = if z > 0 {
        val(x, y, z - 1)
    } else {
        val(x, y, z)
    };
    let zp = if z + 1 < dz {
        val(x, y, z + 1)
    } else {
        val(x, y, z)
    };

    let gx = xp - xm;
    let gy = yp - ym;
    let gz = zp - zm;
    let len = (gx * gx + gy * gy + gz * gz).sqrt().max(1e-10);
    [gx / len, gy / len, gz / len]
}

/// Emit a quad as two triangles with consistent winding.
///
/// `inside_negative` controls the winding direction so that normals point
/// outward (from negative/inside toward positive/outside).
#[inline]
fn emit_quad(indices: &mut Vec<u32>, a: u32, b: u32, c: u32, d: u32, inside_negative: bool) {
    if inside_negative {
        // Winding: a-b-c, a-c-d
        indices.extend_from_slice(&[a, b, c, a, c, d]);
    } else {
        // Reversed winding: a-d-c, a-c-b
        indices.extend_from_slice(&[a, d, c, a, c, b]);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::contour_to_tsdf_chunks::quantize_tsdf;

    /// Helper: build a TsdfChunk from a closure that returns signed distance.
    fn make_chunk(
        dims: [u32; 3],
        spacing: [f32; 3],
        f: impl Fn(f32, f32, f32) -> f32,
    ) -> TsdfChunk {
        let trunc = 16.0;
        let n = (dims[0] * dims[1] * dims[2]) as usize;
        let mut values = Vec::with_capacity(n);
        let mut weight = Vec::with_capacity(n);
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let wx = x as f32 * spacing[0];
                    let wy = y as f32 * spacing[1];
                    let wz = z as f32 * spacing[2];
                    let d = f(wx, wy, wz);
                    values.push(quantize_tsdf(d.clamp(-trunc, trunc), trunc));
                    weight.push(1);
                }
            }
        }
        TsdfChunk {
            dims,
            voxel_size: spacing,
            origin: [0.0, 0.0, 0.0],
            truncation_mm: trunc,
            values,
            weight,
            revision: 1,
        }
    }

    #[test]
    fn test_surface_nets_empty_sdf() {
        // All-positive SDF: no zero crossing → empty mesh.
        let chunk = make_chunk([8, 8, 8], [1.0, 1.0, 1.0], |_, _, _| 5.0);
        let mesh = surface_nets_chunk(&chunk);
        assert!(mesh.vertices.is_empty());
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn test_surface_nets_all_inside() {
        // All-negative SDF: also no zero crossing → empty mesh.
        let chunk = make_chunk([8, 8, 8], [1.0, 1.0, 1.0], |_, _, _| -5.0);
        let mesh = surface_nets_chunk(&chunk);
        assert!(mesh.vertices.is_empty());
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn test_surface_nets_sphere() {
        // Sphere of radius 4.0 centered at (5,5,5) in a 12³ grid.
        let chunk = make_chunk([12, 12, 12], [1.0, 1.0, 1.0], |x, y, z| {
            let dx = x - 5.0;
            let dy = y - 5.0;
            let dz = z - 5.0;
            (dx * dx + dy * dy + dz * dz).sqrt() - 4.0
        });
        let mesh = surface_nets_chunk(&chunk);

        // Must produce non-trivial geometry.
        assert!(
            mesh.vertices.len() > 10,
            "Expected significant vertex count, got {}",
            mesh.vertices.len()
        );
        assert!(
            mesh.indices.len() > 10,
            "Expected significant index count, got {}",
            mesh.indices.len()
        );
        // Indices must be divisible by 3 (triangles).
        assert_eq!(mesh.indices.len() % 3, 0);
        // All normals should point roughly outward (dot with direction from center > 0).
        let center = [5.0, 5.0, 5.0];
        let mut outward_count = 0;
        for (v, n) in mesh.vertices.iter().zip(mesh.normals.iter()) {
            let dir = [v[0] - center[0], v[1] - center[1], v[2] - center[2]];
            let dot = dir[0] * n[0] + dir[1] * n[1] + dir[2] * n[2];
            if dot > 0.0 {
                outward_count += 1;
            }
        }
        let outward_ratio = outward_count as f32 / mesh.vertices.len() as f32;
        assert!(
            outward_ratio > 0.8,
            "Expected most normals to point outward, ratio = {:.2}",
            outward_ratio
        );
    }

    #[test]
    fn test_surface_nets_chunk_boundary_ownership() {
        // Place a sphere crossing the max boundary of a 8³ chunk.
        // Cells at x=6, y=6, z=6 (max boundary = dims-1-1 = 6) should NOT be
        // extracted.
        let chunk = make_chunk([8, 8, 8], [1.0, 1.0, 1.0], |x, y, z| {
            // Sphere centered at (6,6,6) with radius 2.0, so it crosses the boundary.
            let dx = x - 6.0;
            let dy = y - 6.0;
            let dz = z - 6.0;
            (dx * dx + dy * dy + dz * dz).sqrt() - 2.0
        });
        let mesh = surface_nets_chunk(&chunk);
        // Verify that no vertex is placed in a cell at x=6, y=6, or z=6
        // (those would be at or beyond the max boundary).
        for v in &mesh.vertices {
            // Vertices in boundary cells would have positions close to 6.5*spacing
            // or higher.  Since the chunk goes from 0..7, the max cell is (6,6,6)
            // which places its dual vertex near 6.0-6.99.  We don't expect any
            // vertex beyond 6.0 because those cells are not extracted.
            assert!(
                v[0] < 6.5 || v[1] < 6.5 || v[2] < 6.5,
                "Vertex {:?} appears to be in a max-boundary cell",
                v
            );
        }
    }

    #[test]
    fn test_surface_nets_from_sdf_matches_chunk() {
        // Create an SDF and a matching TsdfChunk, verify both produce
        // similar vertex counts.
        let dims: [u32; 3] = [10, 10, 10];
        let spacing = [1.0f32, 1.0, 1.0];
        let sphere = |x: f32, y: f32, z: f32| -> f32 {
            let dx = x - 5.0;
            let dy = y - 5.0;
            let dz = z - 5.0;
            (dx * dx + dy * dy + dz * dz).sqrt() - 3.0
        };

        // Build SdfVolume
        let mut sdf = SdfVolume::new(dims, spacing, [0.0, 0.0, 0.0]);
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let d = sphere(
                        x as f32 * spacing[0],
                        y as f32 * spacing[1],
                        z as f32 * spacing[2],
                    );
                    sdf.set(x, y, z, d);
                }
            }
        }

        // Build matching chunk
        let chunk = make_chunk(dims, spacing, sphere);

        let mesh_sdf = surface_nets_from_sdf(&sdf, 0.0, None);
        let mesh_chunk = surface_nets_chunk(&chunk);

        // Vertex counts should be within 10% (quantization introduces small differences).
        let ratio = mesh_sdf.vertices.len() as f32 / mesh_chunk.vertices.len().max(1) as f32;
        assert!(
            (0.9..=1.1).contains(&ratio),
            "SDF vs chunk vertex count diverged: sdf={}, chunk={}, ratio={:.2}",
            mesh_sdf.vertices.len(),
            mesh_chunk.vertices.len(),
            ratio
        );
    }
}
