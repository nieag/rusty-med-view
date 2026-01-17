use crate::app::components::LabelmapData;
use std::collections::HashMap;

pub const CHUNK_SIZE: usize = 32;

/// A single 32x32x32 chunk of i8 TSDF values.
/// Values are normalized to the range [-127, 127] representing [-truncation, truncation].
pub struct Chunk {
    pub data: Box<[i8; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            data: Box::new([127; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]),
        }
    }
}

pub struct ChunkedTSDF {
    pub chunks: HashMap<(i16, i16, i16), Chunk>,
    pub truncation: f32, // Distance at which value is +/- 127
    pub dirty_chunks: std::collections::HashSet<(i16, i16, i16)>,
}

impl ChunkedTSDF {
    pub fn new(truncation: f32) -> Self {
        Self {
            chunks: HashMap::new(),
            truncation,
            dirty_chunks: std::collections::HashSet::new(),
        }
    }

    pub fn get_distance(&self, x: i32, y: i32, z: i32) -> f32 {
        let (cx, cy, cz, lx, ly, lz) = self.world_to_local(x, y, z);
        if let Some(chunk) = self.chunks.get(&(cx, cy, cz)) {
            let idx = (lz * (CHUNK_SIZE as i32) * (CHUNK_SIZE as i32)
                + ly * (CHUNK_SIZE as i32)
                + lx) as usize;
            (chunk.data[idx] as f32 / 127.0) * self.truncation
        } else {
            // If chunk doesn't exist, we assume it's far away.
            // But we don't know if it's inside or outside without more context.
            // For now, return truncation (outside).
            self.truncation
        }
    }

    pub fn set_distance(&mut self, x: i32, y: i32, z: i32, dist: f32) {
        let (cx, cy, cz, lx, ly, lz) = self.world_to_local(x, y, z);
        let chunk = self.chunks.entry((cx, cy, cz)).or_insert_with(Chunk::new);
        let idx = (lz * (CHUNK_SIZE as i32) * (CHUNK_SIZE as i32) + ly * (CHUNK_SIZE as i32) + lx)
            as usize;

        let normalized = (dist / self.truncation * 127.0).clamp(-127.0, 127.0) as i8;
        if chunk.data[idx] != normalized {
            chunk.data[idx] = normalized;
            self.dirty_chunks.insert((cx, cy, cz));
            // Also mark neighbors dirty if we are at boundary, for smooth meshing
            if lx == 0 {
                self.dirty_chunks.insert((cx - 1, cy, cz));
            }
            if lx == (CHUNK_SIZE - 1) as i32 {
                self.dirty_chunks.insert((cx + 1, cy, cz));
            }
            if ly == 0 {
                self.dirty_chunks.insert((cx, cy - 1, cz));
            }
            if ly == (CHUNK_SIZE - 1) as i32 {
                self.dirty_chunks.insert((cx, cy + 1, cz));
            }
            if lz == 0 {
                self.dirty_chunks.insert((cx, cy, cz - 1));
            }
            if lz == (CHUNK_SIZE - 1) as i32 {
                self.dirty_chunks.insert((cx, cy, cz + 1));
            }
        }
    }

    pub fn apply_brush(&mut self, center: [i32; 3], radius: f32) {
        let r_int = (radius + 0.5) as i32;
        let r2 = radius * radius;

        for dz in -r_int..=r_int {
            for dy in -r_int..=r_int {
                for dx in -r_int..=r_int {
                    let d2 = (dx * dx + dy * dy + dz * dz) as f32;
                    if d2 <= r2 {
                        let x = center[0] + dx;
                        let y = center[1] + dy;
                        let z = center[2] + dz;

                        let current_dist = self.get_distance(x, y, z);
                        let brush_dist = d2.sqrt() - radius;

                        // "Inside" is negative, so we take the minimum of current and brush distance
                        if brush_dist < current_dist {
                            self.set_distance(x, y, z, brush_dist);
                        }
                    }
                }
            }
        }
    }

    pub fn apply_eraser(&mut self, center: [i32; 3], radius: f32) {
        let r_int = (radius + 0.5) as i32;
        let r2 = radius * radius;

        for dz in -r_int..=r_int {
            for dy in -r_int..=r_int {
                for dx in -r_int..=r_int {
                    let d2 = (dx * dx + dy * dy + dz * dz) as f32;
                    if d2 <= r2 {
                        let x = center[0] + dx;
                        let y = center[1] + dy;
                        let z = center[2] + dz;

                        let current_dist = self.get_distance(x, y, z);
                        let brush_dist = -(d2.sqrt() - radius); // Flip sign for eraser

                        // We want to "push" the boundary away, so take maximum of current and inverted brush distance
                        if brush_dist > current_dist {
                            self.set_distance(x, y, z, brush_dist);
                        }
                    }
                }
            }
        }
    }

    pub fn from_labelmap(labelmap: &LabelmapData, truncation: f32) -> Self {
        let mut tsdf = Self::new(truncation);
        let dims = labelmap.dimensions;
        let data = &labelmap.raw_data;

        // 1. Initialize with large values
        // We only care about voxels within the labelmap bounds + truncation
        // But for now, let's just process the labelmap bounds.

        // Initialize a grid of distances
        let total_voxels = (dims[0] * dims[1] * dims[2]) as usize;
        let mut distances = vec![truncation; total_voxels];

        let get_idx = |x: u32, y: u32, z: u32| (z * dims[0] * dims[1] + y * dims[0] + x) as usize;

        // 2. Identify boundary voxels and initialize with 0.5 distance
        // Use 6-connectivity to find adjacent voxels with different labels
        let neighbors = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        let mut queue = std::collections::VecDeque::new();
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let idx = get_idx(x, y, z);
                    let label = data[idx];

                    for (dx, dy, dz) in neighbors {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        let nz = z as i32 + dz;
                        if nx >= 0
                            && nx < dims[0] as i32
                            && ny >= 0
                            && ny < dims[1] as i32
                            && nz >= 0
                            && nz < dims[2] as i32
                        {
                            let n_idx = get_idx(nx as u32, ny as u32, nz as u32);
                            if data[n_idx] != label {
                                if distances[idx] > 0.5 {
                                    distances[idx] = 0.5;
                                    queue.push_back((x as i32, y as i32, z as i32));
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. BFS to propagate distances
        while let Some((x, y, z)) = queue.pop_front() {
            let dist = distances[get_idx(x as u32, y as u32, z as u32)];
            if dist >= truncation {
                continue;
            }

            for (dx, dy, dz) in neighbors {
                let nx = x + dx;
                let ny = y + dy;
                let nz = z + dz;

                if nx >= 0
                    && nx < dims[0] as i32
                    && ny >= 0
                    && ny < dims[1] as i32
                    && nz >= 0
                    && nz < dims[2] as i32
                {
                    let n_idx = get_idx(nx as u32, ny as u32, nz as u32);
                    let new_dist = dist + 1.0;
                    if new_dist < distances[n_idx] {
                        distances[n_idx] = new_dist;
                        queue.push_back((nx, ny, nz));
                    }
                }
            }
        }

        // 4. Fill TSDF and apply sign (inside is negative)
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let idx = get_idx(x, y, z);
                    let dist = distances[idx];

                    // Optimization: We only need to store values within the truncation band.
                    // If dist >= truncation, it's outside. If -dist <= -truncation, it's deep inside.
                    // Chunk::new() defaults to 127 (truncation outside), so we only set if different.
                    if dist < truncation {
                        let label = data[idx];
                        let signed_dist = if label > 0 { -dist } else { dist };
                        tsdf.set_distance(x as i32, y as i32, z as i32, signed_dist);
                    } else {
                        let label = data[idx];
                        if label > 0 {
                            // Deep inside - we need to set it to -truncation
                            tsdf.set_distance(x as i32, y as i32, z as i32, -truncation);
                        }
                    }
                }
            }
        }

        tsdf
    }

    pub fn to_labelmap(&self, dims: [u32; 3]) -> LabelmapData {
        let total_voxels = (dims[0] * dims[1] * dims[2]) as usize;
        let mut raw_data = vec![0u8; total_voxels];

        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let dist = self.get_distance(x as i32, y as i32, z as i32);
                    if dist <= 0.0 {
                        let idx = (z * dims[0] * dims[1] + y * dims[0] + x) as usize;
                        raw_data[idx] = 1; // Assuming label 1
                    }
                }
            }
        }

        LabelmapData {
            dimensions: dims,
            raw_data,
        }
    }

    fn world_to_local(&self, x: i32, y: i32, z: i32) -> (i16, i16, i16, i32, i32, i32) {
        let cx = (x >> 5) as i16; // x / 32
        let cy = (y >> 5) as i16;
        let cz = (z >> 5) as i16;

        let lx = x & 31; // x % 32
        let ly = y & 31;
        let lz = z & 31;

        (cx, cy, cz, lx, ly, lz)
    }
}

impl crate::segmentation::algorithms::surface_nets::DistanceSampler for ChunkedTSDF {
    fn get_distance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.get_distance(x, y, z)
    }

    fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        if self.chunks.is_empty() {
            return ([0, 0, 0], [0, 0, 0]);
        }
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for &(cx, cy, cz) in self.chunks.keys() {
            min[0] = min[0].min(cx as i32 * CHUNK_SIZE as i32);
            min[1] = min[1].min(cy as i32 * CHUNK_SIZE as i32);
            min[2] = min[2].min(cz as i32 * CHUNK_SIZE as i32);
            max[0] = max[0].max((cx as i32 + 1) * CHUNK_SIZE as i32);
            max[1] = max[1].max((cy as i32 + 1) * CHUNK_SIZE as i32);
            max[2] = max[2].max((cz as i32 + 1) * CHUNK_SIZE as i32);
        }
        (min, max)
    }
}

pub struct LabelmapSampler<'a> {
    pub labelmap: &'a LabelmapData,
}

impl<'a> crate::segmentation::algorithms::surface_nets::DistanceSampler for LabelmapSampler<'a> {
    fn get_distance(&self, x: i32, y: i32, z: i32) -> f32 {
        if x < 0
            || x >= self.labelmap.dimensions[0] as i32
            || y < 0
            || y >= self.labelmap.dimensions[1] as i32
            || z < 0
            || z >= self.labelmap.dimensions[2] as i32
        {
            return 1.0;
        }
        let idx = (z as u32 * self.labelmap.dimensions[0] * self.labelmap.dimensions[1]
            + y as u32 * self.labelmap.dimensions[0]
            + x as u32) as usize;
        if self.labelmap.raw_data[idx] > 0 {
            -1.0
        } else {
            1.0
        }
    }

    fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        (
            [0, 0, 0],
            [
                self.labelmap.dimensions[0] as i32,
                self.labelmap.dimensions[1] as i32,
                self.labelmap.dimensions[2] as i32,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsdf_coordinate_mapping() {
        let tsdf = ChunkedTSDF::new(4.0);
        let (cx, cy, cz, lx, ly, lz) = tsdf.world_to_local(10, 20, 30);
        assert_eq!(cx, 0);
        assert_eq!(cy, 0);
        assert_eq!(cz, 0);
        assert_eq!(lx, 10);
        assert_eq!(ly, 20);
        assert_eq!(lz, 30);

        let (cx, cy, cz, lx, ly, lz) = tsdf.world_to_local(33, -1, 65);
        assert_eq!(cx, 1);
        assert_eq!(cy, -1);
        assert_eq!(cz, 2);
        assert_eq!(lx, 1);
        assert_eq!(ly, 31);
        assert_eq!(lz, 1);
    }

    #[test]
    fn test_tsdf_roundtrip_simple() {
        // Create a 10x10x10 labelmap with a 4x4x4 cube in the middle
        let dims = [10, 10, 10];
        let mut raw_data = vec![0u8; 1000];
        for z in 3..7 {
            for y in 3..7 {
                for x in 3..7 {
                    raw_data[(z * 100 + y * 10 + x) as usize] = 1;
                }
            }
        }
        let labelmap = LabelmapData {
            dimensions: dims,
            raw_data: raw_data.clone(),
        };

        let tsdf = ChunkedTSDF::from_labelmap(&labelmap, 4.0);
        let roundtrip = tsdf.to_labelmap(dims);

        // Check if roundtrip matches original (mostly)
        for i in 0..1000 {
            assert_eq!(
                roundtrip.raw_data[i], raw_data[i],
                "Mismatch at index {}",
                i
            );
        }
    }
}
