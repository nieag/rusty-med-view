use glam::Vec3;
use std::collections::HashMap;

pub trait DistanceSampler {
    fn get_distance(&self, x: i32, y: i32, z: i32) -> f32;
    fn bounds(&self) -> ([i32; 3], [i32; 3]); // min, max
}

pub struct SurfaceNets {
    pub isovalue: f32,
}

impl SurfaceNets {
    pub fn new(isovalue: f32) -> Self {
        Self { isovalue }
    }

    pub fn extract<S: DistanceSampler>(
        &self,
        sampler: &S,
        dims: [u32; 3],
    ) -> (Vec<Vec3>, Vec<Vec3>, Vec<u32>) {
        let (min, max) = sampler.bounds();
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut grid_to_vertex = HashMap::new();

        // 1. Generate vertices
        for z in min[2]..max[2] {
            for y in min[1]..max[1] {
                for x in min[0]..max[0] {
                    let mut mask = 0u8;
                    for i in 0..8 {
                        let cx = x + (i & 1) as i32;
                        let cy = y + ((i >> 1) & 1) as i32;
                        let cz = z + ((i >> 2) & 1) as i32;
                        if sampler.get_distance(cx, cy, cz) < self.isovalue {
                            mask |= 1 << i;
                        }
                    }

                    if mask != 0 && mask != 0xFF {
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
        }

        // 2. Generate faces
        for z in min[2]..max[2] {
            for y in min[1]..max[1] {
                for x in min[0]..max[0] {
                    let v0 = sampler.get_distance(x, y, z) < self.isovalue;

                    if x + 1 < max[0] {
                        let v1 = sampler.get_distance(x + 1, y, z) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 0, v0);
                        }
                    }
                    if y + 1 < max[1] {
                        let v1 = sampler.get_distance(x, y + 1, z) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 1, v0);
                        }
                    }
                    if z + 1 < max[2] {
                        let v1 = sampler.get_distance(x, y, z + 1) < self.isovalue;
                        if v0 != v1 {
                            self.append_quad(&mut indices, &grid_to_vertex, x, y, z, 2, v0);
                        }
                    }
                }
            }
        }

        // 3. Calculate Normals
        let mut normals = vec![Vec3::ZERO; vertices.len()];
        for i in (0..indices.len()).step_by(3) {
            let i0 = indices[i] as usize;
            let i1 = indices[i + 1] as usize;
            let i2 = indices[i + 2] as usize;

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

        (vertices, normals, indices)
    }

    fn append_quad(
        &self,
        indices: &mut Vec<u32>,
        grid: &HashMap<(i32, i32, i32), u32>,
        x: i32,
        y: i32,
        z: i32,
        axis: u8,
        v0_inside: bool,
    ) {
        // For an edge crossing, the quad connects the 4 surrounding dual vertices.
        // E.g. for X edge, surrounding cubes are (x, y, z), (x, y-1, z), (x, y-1, z-1), (x, y, z-1)
        let mut quads = [0u32; 4];
        let offsets = match axis {
            0 => [(0, 0, 0), (0, -1, 0), (0, -1, -1), (0, 0, -1)], // X edge
            1 => [(0, 0, 0), (0, 0, -1), (-1, 0, -1), (-1, 0, 0)], // Y edge
            2 => [(0, 0, 0), (-1, 0, 0), (-1, -1, 0), (0, -1, 0)], // Z edge
            _ => return,
        };

        for (i, off) in offsets.iter().enumerate() {
            let px = x + off.0;
            let py = y + off.1;
            let pz = z + off.2;
            if let Some(&idx) = grid.get(&(px, py, pz)) {
                quads[i] = idx;
            } else {
                return; // Missing cube vertex (boundary)
            }
        }

        // Triangle 1
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
