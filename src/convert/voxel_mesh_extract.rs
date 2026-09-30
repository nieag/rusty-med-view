use crate::convert::{
    build_smooth_mesh_field, extract_smooth_mesh_chunk_from_field,
    extract_smooth_mesh_from_voxel_data, SmoothMeshExtractionError, SmoothMeshField,
};
use crate::model::{MeshData, MeshFace, VoxelData};
use std::{collections::HashMap, sync::Arc};

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
        chunked: ChunkedMeshData,
        voxel_data: &VoxelData,
        _min_inclusive: [u32; 3],
        _max_exclusive: [u32; 3],
    ) -> Result<Self, VoxelMeshExtractionError> {
        // The Euclidean SDF is global: a single changed seed can alter edge
        // interpolation outside a fixed voxel halo. Rebuild all chunks until
        // old/new field differences can identify the exact affected chunks.
        Self::begin_full(voxel_data, chunked.chunk_size)
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

    #[cfg(test)]
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
        // Chunks index their shared boundary vertices independently. The editable
        // mesh needs shared indices too, otherwise a connected brush tears seams.
        let mut vertex_ids = HashMap::with_capacity(vertex_count);
        for chunk in &self.chunks {
            let remap: Vec<u32> = chunk
                .data
                .vertices
                .iter()
                .map(|vertex| {
                    let key = vertex
                        .world_mm
                        .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
                    *vertex_ids.entry(key).or_insert_with(|| {
                        let index = merged.vertices.len() as u32;
                        merged.vertices.push(*vertex);
                        index
                    })
                })
                .collect();
            merged
                .faces
                .extend(chunk.data.faces.iter().map(|face| MeshFace {
                    vertex_indices: face.vertex_indices.map(|index| remap[index as usize]),
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

#[cfg(test)]
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
mod tests;
