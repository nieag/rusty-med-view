use crate::app::roi::{MeshData, MeshFace, VoxelData};
use crate::convert::{
    build_smooth_mesh_field, extract_smooth_mesh_chunk_from_field,
    extract_smooth_mesh_from_voxel_data, SmoothMeshExtractionError, SmoothMeshField,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelMeshExtractionError {
    InvalidRawDataLength { expected: usize, actual: usize },
    InvalidChunkSize,
    SmoothMesh(SmoothMeshExtractionError),
}

pub const DEFAULT_MESH_CHUNK_SIZE: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshChunkKey {
    pub index: [u32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshChunk {
    pub key: MeshChunkKey,
    pub data: Arc<MeshData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkedMeshData {
    pub chunk_size: u32,
    pub voxel_dimensions: [u32; 3],
    pub chunks: Vec<MeshChunk>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalChunkedMeshRebuild {
    result: ChunkedMeshData,
    smooth_field: SmoothMeshField,
    pending_keys: Vec<MeshChunkKey>,
    next_key: usize,
}

impl IncrementalChunkedMeshRebuild {
    pub fn begin_full(
        voxel_data: &VoxelData,
        chunk_size: u32,
    ) -> Result<Self, VoxelMeshExtractionError> {
        validate_voxel_data(voxel_data)?;
        if chunk_size == 0 {
            return Err(VoxelMeshExtractionError::InvalidChunkSize);
        }
        let dimensions = voxel_data.geometry.dimensions;
        let smooth_field =
            build_smooth_mesh_field(voxel_data).map_err(VoxelMeshExtractionError::SmoothMesh)?;
        Ok(Self {
            result: ChunkedMeshData {
                chunk_size,
                voxel_dimensions: dimensions,
                chunks: Vec::new(),
            },
            smooth_field,
            pending_keys: all_mesh_chunk_keys(dimensions, chunk_size),
            next_key: 0,
        })
    }

    pub fn begin_for_voxel_aabb(
        mut chunked: ChunkedMeshData,
        voxel_data: &VoxelData,
        min_inclusive: [u32; 3],
        max_exclusive: [u32; 3],
    ) -> Result<Self, VoxelMeshExtractionError> {
        validate_voxel_data(voxel_data)?;
        if chunked.chunk_size == 0 {
            return Err(VoxelMeshExtractionError::InvalidChunkSize);
        }
        if chunked.voxel_dimensions != voxel_data.geometry.dimensions {
            return Self::begin_full(voxel_data, chunked.chunk_size);
        }

        let smooth_field =
            build_smooth_mesh_field(voxel_data).map_err(VoxelMeshExtractionError::SmoothMesh)?;

        let pending_keys = mesh_chunk_keys_for_voxel_aabb(
            voxel_data.geometry.dimensions,
            min_inclusive,
            max_exclusive,
            chunked.chunk_size,
        );
        chunked
            .chunks
            .retain(|chunk| !pending_keys.contains(&chunk.key));
        Ok(Self {
            result: chunked,
            smooth_field,
            pending_keys,
            next_key: 0,
        })
    }

    pub fn step(&mut self, voxel_data: &VoxelData) -> Result<bool, VoxelMeshExtractionError> {
        let Some(key) = self.pending_keys.get(self.next_key).copied() else {
            return Ok(true);
        };
        let data = extract_mesh_chunk_from_voxel_data(
            voxel_data,
            &self.smooth_field,
            key,
            self.result.chunk_size,
        )?;
        if !data.faces.is_empty() {
            self.result.chunks.push(MeshChunk {
                key,
                data: Arc::new(data),
            });
        }
        self.next_key += 1;
        if self.is_complete() {
            self.result
                .chunks
                .sort_by_key(|chunk| chunk_sort_key(chunk.key));
        }
        Ok(self.is_complete())
    }

    pub fn is_complete(&self) -> bool {
        self.next_key >= self.pending_keys.len()
    }

    pub fn remaining_chunks(&self) -> usize {
        self.pending_keys.len().saturating_sub(self.next_key)
    }

    pub fn changed_keys(&self) -> &[MeshChunkKey] {
        &self.pending_keys
    }

    pub fn into_result(mut self) -> Option<ChunkedMeshData> {
        if !self.is_complete() {
            return None;
        }
        self.result
            .chunks
            .sort_by_key(|chunk| chunk_sort_key(chunk.key));
        Some(self.result)
    }
}

impl ChunkedMeshData {
    pub fn merged_mesh(&self) -> MeshData {
        let vertex_count = self
            .chunks
            .iter()
            .map(|chunk| chunk.data.vertices.len())
            .sum();
        let face_count = self.chunks.iter().map(|chunk| chunk.data.faces.len()).sum();
        let mut merged = MeshData {
            vertices: Vec::with_capacity(vertex_count),
            faces: Vec::with_capacity(face_count),
        };
        for chunk in &self.chunks {
            let vertex_offset = merged.vertices.len() as u32;
            merged.vertices.extend_from_slice(&chunk.data.vertices);
            merged
                .faces
                .extend(chunk.data.faces.iter().map(|face| MeshFace {
                    vertex_indices: face.vertex_indices.map(|index| index + vertex_offset),
                }));
        }
        merged
    }
}

pub fn extract_mesh_from_voxel_data(
    voxel_data: &VoxelData,
) -> Result<MeshData, VoxelMeshExtractionError> {
    validate_voxel_data(voxel_data)?;
    extract_smooth_mesh_from_voxel_data(voxel_data).map_err(VoxelMeshExtractionError::SmoothMesh)
}

pub fn extract_chunked_mesh_from_voxel_data(
    voxel_data: &VoxelData,
    chunk_size: u32,
) -> Result<ChunkedMeshData, VoxelMeshExtractionError> {
    let mut rebuild = IncrementalChunkedMeshRebuild::begin_full(voxel_data, chunk_size)?;
    while !rebuild.step(voxel_data)? {}
    rebuild
        .into_result()
        .ok_or(VoxelMeshExtractionError::InvalidChunkSize)
}

fn chunk_sort_key(key: MeshChunkKey) -> (u32, u32, u32) {
    (key.index[2], key.index[1], key.index[0])
}

pub fn rebuild_chunked_mesh_for_voxel_aabb(
    chunked: &mut ChunkedMeshData,
    voxel_data: &VoxelData,
    min_inclusive: [u32; 3],
    max_exclusive: [u32; 3],
) -> Result<Vec<MeshChunkKey>, VoxelMeshExtractionError> {
    let mut rebuild = IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(
        chunked.clone(),
        voxel_data,
        min_inclusive,
        max_exclusive,
    )?;
    let affected = rebuild.changed_keys().to_vec();
    while !rebuild.step(voxel_data)? {}
    *chunked = rebuild
        .into_result()
        .ok_or(VoxelMeshExtractionError::InvalidChunkSize)?;
    Ok(affected)
}

pub fn mesh_chunk_keys_for_voxel_aabb(
    dimensions: [u32; 3],
    min_inclusive: [u32; 3],
    max_exclusive: [u32; 3],
    chunk_size: u32,
) -> Vec<MeshChunkKey> {
    if chunk_size == 0 || dimensions.contains(&0) {
        return Vec::new();
    }

    // A changed cell can alter a face owned by an adjacent cell. Expand one
    // voxel before mapping to chunks so boundary faces are rebuilt once.
    let min: [u32; 3] = std::array::from_fn(|axis| min_inclusive[axis].saturating_sub(1));
    let max: [u32; 3] =
        std::array::from_fn(|axis| max_exclusive[axis].saturating_add(1).min(dimensions[axis]));
    if (0..3).any(|axis| min[axis] >= max[axis]) {
        return Vec::new();
    }

    let first = min.map(|value| value / chunk_size);
    let last: [u32; 3] = std::array::from_fn(|axis| (max[axis] - 1) / chunk_size);
    let mut keys = Vec::new();
    for z in first[2]..=last[2] {
        for y in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                keys.push(MeshChunkKey { index: [x, y, z] });
            }
        }
    }
    keys
}

fn validate_voxel_data(voxel_data: &VoxelData) -> Result<(), VoxelMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected_len = voxel_cell_count(dimensions);
    let actual_len = voxel_data.raw_data.len();
    if actual_len != expected_len {
        return Err(VoxelMeshExtractionError::InvalidRawDataLength {
            expected: expected_len,
            actual: actual_len,
        });
    }
    Ok(())
}

fn all_mesh_chunk_keys(dimensions: [u32; 3], chunk_size: u32) -> Vec<MeshChunkKey> {
    if chunk_size == 0 || dimensions.contains(&0) {
        return Vec::new();
    }
    let counts = dimensions.map(|dimension| dimension.div_ceil(chunk_size));
    let mut keys = Vec::new();
    for z in 0..counts[2] {
        for y in 0..counts[1] {
            for x in 0..counts[0] {
                keys.push(MeshChunkKey { index: [x, y, z] });
            }
        }
    }
    keys
}

fn extract_mesh_chunk_from_voxel_data(
    voxel_data: &VoxelData,
    smooth_field: &SmoothMeshField,
    key: MeshChunkKey,
    chunk_size: u32,
) -> Result<MeshData, VoxelMeshExtractionError> {
    if chunk_size == 0 {
        return Err(VoxelMeshExtractionError::InvalidChunkSize);
    }
    validate_voxel_data(voxel_data)?;
    let dimensions = voxel_data.geometry.dimensions;
    let min = key.index.map(|index| index.saturating_mul(chunk_size));
    let max =
        std::array::from_fn(|axis| min[axis].saturating_add(chunk_size).min(dimensions[axis]));
    extract_smooth_mesh_chunk_from_field(voxel_data, smooth_field, min, max)
        .map_err(VoxelMeshExtractionError::SmoothMesh)
}

fn voxel_cell_count(dimensions: [u32; 3]) -> usize {
    dimensions[0] as usize * dimensions[1] as usize * dimensions[2] as usize
}

#[cfg(test)]
fn voxel_linear_index(dimensions: [u32; 3], x: u32, y: u32, z: u32) -> usize {
    (z as usize * dimensions[1] as usize + y as usize) * dimensions[0] as usize + x as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{VoxelData, VoxelGeometry};
    use glam::{Quat, Vec3};

    fn geometry(
        dimensions: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> VoxelGeometry {
        VoxelGeometry {
            dimensions,
            spacing,
            origin,
            orientation,
        }
    }

    fn voxel_data_with_single_occupied(
        dimensions: [u32; 3],
        occupied: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> VoxelData {
        let len = voxel_cell_count(dimensions);
        let mut raw = vec![0u8; len];
        let idx = voxel_linear_index(dimensions, occupied[0], occupied[1], occupied[2]);
        raw[idx] = 1;
        VoxelData {
            geometry: geometry(dimensions, spacing, origin, orientation),
            raw_data: raw,
        }
    }

    #[test]
    fn test_empty_voxel_data_produces_empty_mesh() {
        let voxel = VoxelData {
            geometry: geometry(
                [0, 0, 0],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: Vec::new(),
        };

        let mesh = extract_mesh_from_voxel_data(&voxel).expect("empty data should succeed");
        assert!(mesh.vertices.is_empty());
        assert!(mesh.faces.is_empty());
    }

    #[test]
    fn test_simple_occupied_shape_produces_non_empty_mesh() {
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );

        let mesh = extract_mesh_from_voxel_data(&voxel)
            .expect("single occupied voxel should produce surface");
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.faces.is_empty());
    }

    #[test]
    fn test_roi_owned_geometry_controls_vertex_placement() {
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [2.0, 3.0, 4.0],
            [10.0, 20.0, 30.0],
            rotation,
        );
        let identity = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [2.0, 3.0, 4.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        let mesh = extract_mesh_from_voxel_data(&voxel).unwrap();
        let identity_mesh = extract_mesh_from_voxel_data(&identity).unwrap();

        for (rotated, local) in mesh.vertices.iter().zip(&identity_mesh.vertices) {
            let expected = Vec3::from_array(voxel.geometry.origin)
                + Quat::from_array(rotation) * Vec3::from_array(local.world_mm);
            assert!(Vec3::from_array(rotated.world_mm).distance(expected) < 1e-5);
        }
    }

    #[test]
    fn test_single_occupied_voxel_vertices_match_cell_boundary_aabb() {
        let voxel = voxel_data_with_single_occupied(
            [5, 6, 7],
            [2, 3, 4],
            [2.0, 3.0, 4.0],
            [10.0, 20.0, 30.0],
            Quat::IDENTITY.to_array(),
        );

        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert!(!mesh.vertices.is_empty());

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.world_mm[axis]);
                max[axis] = max[axis].max(vertex.world_mm[axis]);
            }
        }

        assert!(min.into_iter().zip([13.0, 27.5, 44.0]).all(|(a, b)| a >= b));
        assert!(max.into_iter().zip([15.0, 30.5, 48.0]).all(|(a, b)| a <= b));
    }

    #[test]
    fn test_extraction_is_deterministic_for_same_voxel_input() {
        let voxel = voxel_data_with_single_occupied(
            [3, 3, 3],
            [1, 1, 1],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );

        let first = extract_mesh_from_voxel_data(&voxel).expect("first extraction should succeed");
        let second =
            extract_mesh_from_voxel_data(&voxel).expect("second extraction should succeed");
        assert_eq!(first, second);
    }

    #[test]
    fn test_single_voxel_smooth_topology_has_no_cube_faces() {
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert_eq!(mesh.faces.len(), 8);
    }

    #[test]
    fn test_solid_2x2x2_smooth_mesh_is_smaller_than_cube_faces() {
        let dimensions = [2, 2, 2];
        let voxel = VoxelData {
            geometry: geometry(
                dimensions,
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1; 8],
        };
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert!(mesh.faces.len() < 48);
    }

    #[test]
    fn test_invalid_raw_data_length_returns_error() {
        let voxel = VoxelData {
            geometry: geometry(
                [2, 2, 2],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1; 7],
        };
        let err =
            extract_mesh_from_voxel_data(&voxel).expect_err("should reject invalid raw length");
        assert_eq!(
            err,
            VoxelMeshExtractionError::InvalidRawDataLength {
                expected: 8,
                actual: 7
            }
        );
    }

    #[test]
    fn test_all_non_zero_values_are_occupied() {
        let voxel = VoxelData {
            geometry: geometry(
                [2, 1, 1],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1, 2],
        };
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert!(mesh.faces.len() < 20);
    }

    fn canonical_triangles(mesh: &MeshData) -> Vec<[[i32; 3]; 3]> {
        let mut triangles = mesh
            .faces
            .iter()
            .map(|face| {
                let mut points = face.vertex_indices.map(|index| {
                    mesh.vertices[index as usize]
                        .world_mm
                        .map(|value| (value * 2.0).round() as i32)
                });
                points.sort();
                points
            })
            .collect::<Vec<_>>();
        triangles.sort();
        triangles
    }

    #[test]
    fn test_chunked_extraction_matches_full_mesh_across_chunk_boundaries() {
        let dimensions = [5, 3, 2];
        let mut raw_data = vec![0; voxel_cell_count(dimensions)];
        for occupied in [[1, 1, 0], [2, 1, 0], [4, 2, 1]] {
            raw_data[voxel_linear_index(dimensions, occupied[0], occupied[1], occupied[2])] = 1;
        }
        let voxel = VoxelData {
            geometry: geometry(
                dimensions,
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data,
        };

        let full = extract_mesh_from_voxel_data(&voxel).unwrap();
        let chunked = extract_chunked_mesh_from_voxel_data(&voxel, 2).unwrap();

        assert_eq!(
            canonical_triangles(&chunked.merged_mesh()),
            canonical_triangles(&full)
        );
    }

    #[test]
    fn test_incremental_chunk_rebuild_matches_clean_full_rebuild() {
        let dimensions = [6, 2, 1];
        let mut voxel = voxel_data_with_single_occupied(
            dimensions,
            [1, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        voxel.raw_data[voxel_linear_index(dimensions, 5, 0, 0)] = 1;
        let mut chunked = extract_chunked_mesh_from_voxel_data(&voxel, 2).unwrap();
        let untouched_before = chunked
            .chunks
            .iter()
            .find(|chunk| chunk.key.index == [2, 0, 0])
            .unwrap()
            .clone();

        voxel.raw_data[voxel_linear_index(dimensions, 2, 0, 0)] = 1;
        let rebuilt =
            rebuild_chunked_mesh_for_voxel_aabb(&mut chunked, &voxel, [2, 0, 0], [3, 1, 1])
                .unwrap();
        let full = extract_mesh_from_voxel_data(&voxel).unwrap();

        assert_eq!(
            rebuilt.iter().map(|key| key.index).collect::<Vec<_>>(),
            vec![[0, 0, 0], [1, 0, 0]]
        );
        assert_eq!(
            chunked
                .chunks
                .iter()
                .find(|chunk| chunk.key.index == [2, 0, 0])
                .unwrap(),
            &untouched_before
        );
        assert_eq!(
            canonical_triangles(&chunked.merged_mesh()),
            canonical_triangles(&full)
        );
    }

    #[test]
    fn test_incremental_rebuild_advances_one_chunk_per_step() {
        let dimensions = [40, 2, 1];
        let mut voxel = voxel_data_with_single_occupied(
            dimensions,
            [1, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        voxel.raw_data[voxel_linear_index(dimensions, 20, 0, 0)] = 1;
        voxel.raw_data[voxel_linear_index(dimensions, 39, 0, 0)] = 1;
        let mut rebuild = IncrementalChunkedMeshRebuild::begin_full(&voxel, 16).unwrap();

        assert_eq!(rebuild.remaining_chunks(), 3);
        assert!(!rebuild.step(&voxel).unwrap());
        assert_eq!(rebuild.remaining_chunks(), 2);
        assert!(!rebuild.step(&voxel).unwrap());
        assert_eq!(rebuild.remaining_chunks(), 1);
        assert!(rebuild.step(&voxel).unwrap());

        let incremental = rebuild.into_result().unwrap();
        let clean = extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
        assert_eq!(incremental, clean);
    }

    #[test]
    fn test_incremental_dirty_rebuild_preserves_untouched_chunks() {
        let dimensions = [48, 2, 1];
        let mut voxel = voxel_data_with_single_occupied(
            dimensions,
            [1, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        voxel.raw_data[voxel_linear_index(dimensions, 40, 0, 0)] = 1;
        let base = extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
        let untouched = base
            .chunks
            .iter()
            .find(|chunk| chunk.key.index == [2, 0, 0])
            .unwrap()
            .clone();

        voxel.raw_data[voxel_linear_index(dimensions, 16, 0, 0)] = 1;
        let mut rebuild = IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(
            base,
            &voxel,
            [16, 0, 0],
            [17, 1, 1],
        )
        .unwrap();
        assert_eq!(
            rebuild
                .changed_keys()
                .iter()
                .map(|key| key.index)
                .collect::<Vec<_>>(),
            vec![[0, 0, 0], [1, 0, 0]]
        );
        while !rebuild.step(&voxel).unwrap() {}
        let result = rebuild.into_result().unwrap();

        assert_eq!(
            result
                .chunks
                .iter()
                .find(|chunk| chunk.key.index == [2, 0, 0])
                .unwrap(),
            &untouched
        );
        assert_eq!(
            canonical_triangles(&result.merged_mesh()),
            canonical_triangles(&extract_mesh_from_voxel_data(&voxel).unwrap())
        );
    }

    #[test]
    fn test_dirty_voxel_aabb_includes_neighbor_chunk_for_face_ownership() {
        let keys = mesh_chunk_keys_for_voxel_aabb([8, 4, 4], [2, 1, 1], [3, 2, 2], 2);
        assert!(keys.iter().any(|key| key.index == [0, 0, 0]));
        assert!(keys.iter().any(|key| key.index == [1, 0, 0]));
        assert!(!keys.iter().any(|key| key.index[0] == 2));
    }
}
