use crate::convert::{
    build_smooth_mesh_field, build_smooth_mesh_field_block, extract_mesh_chunk_from_field_on_grid,
    extract_smooth_mesh_chunk_from_field, extract_smooth_mesh_from_voxel_data,
    smooth_mesh_cell_ranges, SmoothMeshExtractionError, SmoothMeshField,
};
use crate::model::{GeometryIdentity, MeshData, MeshFace, VoxelData, VoxelGeometry};
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
    /// The voxel grid the chunks were extracted from. Chunk keys are indices of this grid, so
    /// chunks can only be reused for voxel data on the same grid.
    pub grid: GeometryIdentity,
    pub chunks: Vec<MeshChunk>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalChunkedMeshRebuild {
    result: ChunkedMeshData,
    smooth_field: SmoothMeshField,
    /// The grid of the field when it did not come from voxel data (a contour distance field),
    /// which then also replaces the voxel data `step` takes.
    field_grid: Option<VoxelGeometry>,
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
                grid: voxel_data.geometry.identity(),
                chunks: Vec::new(),
            },
            smooth_field,
            field_grid: None,
            pending_keys: all_mesh_chunk_keys(dimensions, chunk_size),
            next_key: 0,
        })
    }

    /// Rebuilds only the chunks a change inside `min_inclusive..max_exclusive` can alter and keeps
    /// the others from `chunked`. The result is identical to a clean full rebuild.
    ///
    /// A cell that crosses the surface reads field values of at most the largest voxel spacing
    /// (they are distances between adjacent voxels of opposite label), and such a value changes
    /// only if a voxel within that distance changed. So the affected cells are the dirty box plus
    /// that margin, and the field is computed only over those chunks plus the same margin again,
    /// which is enough for every value that matters to equal the whole-volume value.
    pub fn begin_for_voxel_aabb(
        chunked: ChunkedMeshData,
        voxel_data: &VoxelData,
        min_inclusive: [u32; 3],
        max_exclusive: [u32; 3],
    ) -> Result<Self, VoxelMeshExtractionError> {
        validate_voxel_data(voxel_data)?;
        let dimensions = voxel_data.geometry.dimensions;
        let chunk_size = chunked.chunk_size;
        let empty_box = (0..3).any(|axis| max_exclusive[axis] <= min_inclusive[axis]);
        if chunk_size == 0 || chunked.grid != voxel_data.geometry.identity() || empty_box {
            return Self::begin_full(voxel_data, chunk_size);
        }

        let spacing = voxel_data.geometry.spacing();
        let largest = spacing.iter().copied().fold(0.0_f32, f32::max);
        let smallest = spacing.iter().copied().fold(f32::INFINITY, f32::min);
        if !(largest.is_finite() && smallest.is_finite() && smallest > 0.0) {
            return Self::begin_full(voxel_data, chunk_size);
        }
        // Voxels within the largest spacing of the box, plus one for the cell corner.
        let margin = (largest / smallest).ceil() as u32 + 1;

        // Cells (padded indices) whose field values can change.
        let dirty_cells: [std::ops::Range<u32>; 3] = std::array::from_fn(|axis| {
            min_inclusive[axis].saturating_sub(margin)
                ..(max_exclusive[axis] + margin + 1).min(dimensions[axis] + 1)
        });
        let counts = dimensions.map(|dimension| dimension.div_ceil(chunk_size));
        let mut pending_keys = Vec::new();
        let mut cells_min = [u32::MAX; 3];
        let mut cells_max = [0u32; 3];
        for z in 0..counts[2] {
            for y in 0..counts[1] {
                for x in 0..counts[0] {
                    let key = MeshChunkKey { index: [x, y, z] };
                    let min = key.index.map(|index| index.saturating_mul(chunk_size));
                    let max = std::array::from_fn(|axis| {
                        min[axis].saturating_add(chunk_size).min(dimensions[axis])
                    });
                    let cells = smooth_mesh_cell_ranges(dimensions, min, max);
                    let touches = (0..3).all(|axis| {
                        cells[axis].start < dirty_cells[axis].end
                            && dirty_cells[axis].start < cells[axis].end
                    });
                    if touches {
                        pending_keys.push(key);
                        for axis in 0..3 {
                            cells_min[axis] = cells_min[axis].min(cells[axis].start);
                            cells_max[axis] = cells_max[axis].max(cells[axis].end);
                        }
                    }
                }
            }
        }
        if pending_keys.is_empty() {
            return Ok(Self {
                result: chunked,
                smooth_field: SmoothMeshField::empty(),
                field_grid: None,
                pending_keys,
                next_key: 0,
            });
        }

        // Corners of the wanted cells are padded indices `start..=end`; add the margin around.
        let block_min = std::array::from_fn(|axis| cells_min[axis].saturating_sub(margin));
        let block_max =
            std::array::from_fn(|axis| (cells_max[axis] + margin).min(dimensions[axis] + 1));
        let smooth_field = build_smooth_mesh_field_block(voxel_data, block_min, block_max)
            .map_err(VoxelMeshExtractionError::SmoothMesh)?;
        let kept = chunked
            .chunks
            .into_iter()
            .filter(|chunk| !pending_keys.contains(&chunk.key))
            .collect();
        Ok(Self {
            result: ChunkedMeshData {
                chunk_size,
                grid: voxel_data.geometry.identity(),
                chunks: kept,
            },
            smooth_field,
            field_grid: None,
            pending_keys,
            next_key: 0,
        })
    }

    /// Every chunk of a signed-distance field (see `smooth_mesh_field_from_signed_distance`) on
    /// the grid `geometry`.
    pub fn begin_full_from_field(
        geometry: VoxelGeometry,
        smooth_field: SmoothMeshField,
        chunk_size: u32,
    ) -> Result<Self, VoxelMeshExtractionError> {
        if chunk_size == 0 {
            return Err(VoxelMeshExtractionError::InvalidChunkSize);
        }
        Ok(Self {
            result: ChunkedMeshData {
                chunk_size,
                grid: geometry.identity(),
                chunks: Vec::new(),
            },
            smooth_field,
            field_grid: Some(geometry),
            pending_keys: all_mesh_chunk_keys(geometry.dimensions, chunk_size),
            next_key: 0,
        })
    }

    /// Only the `changed` chunks of a new field, keeping the rest of `previous` (which must be on
    /// the same grid).
    pub fn begin_changed_from_field(
        previous: ChunkedMeshData,
        geometry: VoxelGeometry,
        smooth_field: SmoothMeshField,
        changed: Vec<MeshChunkKey>,
    ) -> Result<Self, VoxelMeshExtractionError> {
        if previous.chunk_size == 0 || previous.grid != geometry.identity() {
            return Self::begin_full_from_field(geometry, smooth_field, previous.chunk_size);
        }
        let kept = previous
            .chunks
            .into_iter()
            .filter(|chunk| !changed.contains(&chunk.key))
            .collect();
        Ok(Self {
            result: ChunkedMeshData {
                chunk_size: previous.chunk_size,
                grid: geometry.identity(),
                chunks: kept,
            },
            smooth_field,
            field_grid: Some(geometry),
            pending_keys: changed,
            next_key: 0,
        })
    }

    /// Like [`Self::step`] for a rebuild that began from a field, which carries its own grid.
    pub fn step_field(&mut self) -> Result<bool, VoxelMeshExtractionError> {
        let geometry = self
            .field_grid
            .ok_or(VoxelMeshExtractionError::InvalidChunkSize)?;
        let Some(key) = self.pending_keys.get(self.next_key).copied() else {
            return Ok(true);
        };
        let chunk_size = self.result.chunk_size;
        let min = key.index.map(|index| index.saturating_mul(chunk_size));
        let max = std::array::from_fn(|axis| {
            min[axis]
                .saturating_add(chunk_size)
                .min(geometry.dimensions[axis])
        });
        let data = extract_mesh_chunk_from_field_on_grid(geometry, &self.smooth_field, min, max);
        self.finish_chunk(key, data);
        Ok(self.is_complete())
    }

    fn finish_chunk(&mut self, key: MeshChunkKey, data: MeshData) {
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
        self.finish_chunk(key, data);
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

/// The chunks of a mesh on `geometry` whose surface can differ between two signed-distance fields
/// of that grid: those with a changed sample at a corner of one of their cells.
pub fn changed_mesh_chunks(
    geometry: VoxelGeometry,
    old_values: &[f32],
    new_values: &[f32],
    chunk_size: u32,
) -> Vec<MeshChunkKey> {
    let dims = geometry.dimensions;
    let counts = dims.map(|dimension| dimension.div_ceil(chunk_size.max(1)));
    // Cell `c` (in the padded grid) belongs to chunk `(c - 1) / chunk_size`, the first cell to 0.
    let chunk_of_cell = |cell: u32, axis: usize| {
        if cell == 0 {
            0
        } else {
            ((cell - 1) / chunk_size).min(counts[axis] - 1)
        }
    };
    let mut changed = std::collections::BTreeSet::new();
    let mut flat = 0usize;
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                if old_values[flat] != new_values[flat] {
                    // The sample is padded index `v + 1`, a corner of cells `v` and `v + 1`.
                    for dz in 0..2 {
                        for dy in 0..2 {
                            for dx in 0..2 {
                                changed.insert([
                                    chunk_of_cell(x + dx, 0),
                                    chunk_of_cell(y + dy, 1),
                                    chunk_of_cell(z + dz, 2),
                                ]);
                            }
                        }
                    }
                }
                flat += 1;
            }
        }
    }
    changed
        .into_iter()
        .map(|index| MeshChunkKey { index })
        .collect()
}
