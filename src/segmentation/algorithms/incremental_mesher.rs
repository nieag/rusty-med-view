// src/segmentation/algorithms/incremental_mesher.rs
//
// Incremental Surface Nets implementation that processes only dirty chunks.

use crate::segmentation::tsdf::{ChunkedTSDF, CHUNK_SIZE};
use glam::Vec3;
use rustc_hash::FxHashMap;

/// Mesh data for a single 32³ chunk
#[derive(Default, Clone)]
pub struct ChunkMesh {
    pub vertices: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>, // Local indices (0-based per chunk)
}

/// Incremental mesher that processes only dirty chunks
pub struct IncrementalMesher {
    pub chunk_meshes: FxHashMap<(i16, i16, i16), ChunkMesh>,
    pub isovalue: f32,
}

impl Default for IncrementalMesher {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl IncrementalMesher {
    pub fn new(isovalue: f32) -> Self {
        Self {
            chunk_meshes: FxHashMap::default(),
            isovalue,
        }
    }

    /// Mesh a single chunk with 1-voxel overlap for boundary stitching
    pub fn mesh_chunk(&mut self, tsdf: &ChunkedTSDF, chunk_id: (i16, i16, i16), dims: [u32; 3]) {
        let chunk_size = CHUNK_SIZE as i32;
        let (cx, cy, cz) = chunk_id;

        // Chunk world-space bounds
        let base_x = cx as i32 * chunk_size;
        let base_y = cy as i32 * chunk_size;
        let base_z = cz as i32 * chunk_size;

        // Iterate over chunk cells - extend by 1 in each negative direction to include
        // cells that boundary quads may reference (append_quad looks at offsets like (0,-1,0))
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut grid_to_vertex: FxHashMap<(i32, i32, i32), u32> = FxHashMap::default();

        // Generate vertices: include cells at -1 offset for boundary stitching
        for lz in -1..chunk_size {
            for ly in -1..chunk_size {
                for lx in -1..chunk_size {
                    let x = base_x + lx;
                    let y = base_y + ly;
                    let z = base_z + lz;

                    // Compute 8-corner mask
                    let mut mask = 0u8;
                    for i in 0..8 {
                        let cx = x + (i & 1);
                        let cy = y + ((i >> 1) & 1);
                        let cz = z + ((i >> 2) & 1);
                        if tsdf.get_distance(cx, cy, cz) < self.isovalue {
                            mask |= 1 << i;
                        }
                    }

                    // Skip empty or full cells
                    if mask == 0 || mask == 0xFF {
                        continue;
                    }

                    // Create vertex at cell center (normalized to 0..1)
                    let v = Vec3::new(
                        (x as f32 + 0.5) / dims[0] as f32,
                        (y as f32 + 0.5) / dims[1] as f32,
                        (z as f32 + 0.5) / dims[2] as f32,
                    );
                    grid_to_vertex.insert((x, y, z), vertices.len() as u32);
                    vertices.push(v);
                }
            }
        }

        // Generate faces for edges - only for cells inside this chunk, but quads can reference boundary vertices
        for lz in 0..chunk_size {
            for ly in 0..chunk_size {
                for lx in 0..chunk_size {
                    let x = base_x + lx;
                    let y = base_y + ly;
                    let z = base_z + lz;

                    let v0 = tsdf.get_distance(x, y, z) < self.isovalue;

                    // X edge - check crossing to x+1
                    {
                        let v1 = tsdf.get_distance(x + 1, y, z) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 0, v0);
                        }
                    }
                    // Y edge
                    {
                        let v1 = tsdf.get_distance(x, y + 1, z) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 1, v0);
                        }
                    }
                    // Z edge
                    {
                        let v1 = tsdf.get_distance(x, y, z + 1) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 2, v0);
                        }
                    }
                }
            }
        }

        // Calculate normals
        let mut normals = vec![Vec3::ZERO; vertices.len()];
        for i in (0..indices.len()).step_by(3) {
            if i + 2 >= indices.len() {
                break;
            }
            let i0 = indices[i] as usize;
            let i1 = indices[i + 1] as usize;
            let i2 = indices[i + 2] as usize;

            if i0 >= vertices.len() || i1 >= vertices.len() || i2 >= vertices.len() {
                continue;
            }

            let v0 = vertices[i0];
            let v1 = vertices[i1];
            let v2 = vertices[i2];

            let face_normal = (v1 - v0).cross(v2 - v0);
            normals[i0] += face_normal;
            normals[i1] += face_normal;
            normals[i2] += face_normal;
        }

        for n in &mut normals {
            *n = n.normalize_or_zero();
        }

        self.chunk_meshes.insert(
            chunk_id,
            ChunkMesh {
                vertices,
                normals,
                indices,
            },
        );
    }

    /// Rebuild meshes for all dirty chunks
    pub fn update_dirty(&mut self, tsdf: &ChunkedTSDF, dims: [u32; 3]) {
        let dirty: Vec<_> = tsdf.dirty_chunks.iter().copied().collect();

        for chunk_id in dirty {
            // Check if chunk still exists (might have been deleted)
            if tsdf.chunks.contains_key(&chunk_id) {
                self.mesh_chunk(tsdf, chunk_id, dims);
            } else {
                // Chunk was deleted, remove its mesh
                self.chunk_meshes.remove(&chunk_id);
            }
        }
    }

    /// Flatten all chunk meshes into a single mesh for GPU upload
    pub fn flatten(&self) -> (Vec<Vec3>, Vec<Vec3>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut normals = Vec::new();
        let mut indices = Vec::new();

        for mesh in self.chunk_meshes.values() {
            let offset = vertices.len() as u32;
            vertices.extend_from_slice(&mesh.vertices);
            normals.extend_from_slice(&mesh.normals);

            // Adjust indices by offset
            for idx in &mesh.indices {
                indices.push(idx + offset);
            }
        }

        (vertices, normals, indices)
    }

    fn append_quad(
        &self,
        indices: &mut Vec<u32>,
        grid: &FxHashMap<(i32, i32, i32), u32>,
        x: i32,
        y: i32,
        z: i32,
        axis: u8,
        v0_inside: bool,
    ) {
        // For an edge crossing, the quad connects the 4 surrounding dual vertices
        let offsets = match axis {
            0 => [(0, 0, 0), (0, -1, 0), (0, -1, -1), (0, 0, -1)], // X edge
            1 => [(0, 0, 0), (0, 0, -1), (-1, 0, -1), (-1, 0, 0)], // Y edge
            2 => [(0, 0, 0), (-1, 0, 0), (-1, -1, 0), (0, -1, 0)], // Z edge
            _ => return,
        };

        let mut quads = [0u32; 4];
        for (i, off) in offsets.iter().enumerate() {
            let px = x + off.0;
            let py = y + off.1;
            let pz = z + off.2;
            if let Some(&idx) = grid.get(&(px, py, pz)) {
                quads[i] = idx;
            } else {
                return; // Missing vertex (boundary)
            }
        }

        // Emit two triangles
        if v0_inside {
            indices.push(quads[0]);
            indices.push(quads[1]);
            indices.push(quads[2]);
            indices.push(quads[0]);
            indices.push(quads[2]);
            indices.push(quads[3]);
        } else {
            indices.push(quads[0]);
            indices.push(quads[2]);
            indices.push(quads[1]);
            indices.push(quads[0]);
            indices.push(quads[3]);
            indices.push(quads[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_mesh_produces_geometry() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        tsdf.apply_brush([16, 16, 16], 8.0);

        let mut mesher = IncrementalMesher::new(0.0);
        mesher.mesh_chunk(&tsdf, (0, 0, 0), [32, 32, 32]);

        let mesh = mesher.chunk_meshes.get(&(0, 0, 0)).unwrap();
        assert!(!mesh.vertices.is_empty(), "Chunk should produce vertices");
        assert_eq!(mesh.vertices.len(), mesh.normals.len());
    }

    #[test]
    fn test_flatten_index_validity() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        tsdf.apply_brush([16, 16, 16], 8.0);
        tsdf.apply_brush([48, 48, 48], 8.0);

        let mut mesher = IncrementalMesher::new(0.0);
        mesher.mesh_chunk(&tsdf, (0, 0, 0), [64, 64, 64]);
        mesher.mesh_chunk(&tsdf, (1, 1, 1), [64, 64, 64]);

        let (verts, _, indices) = mesher.flatten();

        for (i, idx) in indices.iter().enumerate() {
            assert!(
                (*idx as usize) < verts.len(),
                "Index {} at position {} is out of bounds",
                idx,
                i
            );
        }
    }

    #[test]
    fn test_update_dirty_only_processes_dirty() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        tsdf.apply_brush([16, 16, 16], 5.0);

        let mut mesher = IncrementalMesher::new(0.0);
        mesher.update_dirty(&tsdf, [64, 64, 64]);

        // Should have meshed chunk (0,0,0)
        assert!(mesher.chunk_meshes.contains_key(&(0, 0, 0)));

        // Clear dirty and add new brush in different chunk
        tsdf.dirty_chunks.clear();
        tsdf.apply_brush([48, 48, 48], 5.0);

        let old_mesh_ptr = mesher.chunk_meshes.get(&(0, 0, 0)).unwrap() as *const _;
        mesher.update_dirty(&tsdf, [64, 64, 64]);

        // Old mesh should be unchanged (same pointer)
        let new_mesh_ptr = mesher.chunk_meshes.get(&(0, 0, 0)).unwrap() as *const _;
        assert_eq!(
            old_mesh_ptr, new_mesh_ptr,
            "Unchanged chunk should not be remeshed"
        );

        // New chunk should exist
        assert!(mesher.chunk_meshes.contains_key(&(1, 1, 1)));
    }
}
