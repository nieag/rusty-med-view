use crate::app::segment::ChunkKey;
use std::collections::HashSet;

/// Inclusive chunk-aligned bounds in index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBounds {
    pub min: [u32; 3],
    pub max: [u32; 3],
}

/// Return unique chunk keys intersecting inclusive index bounds [x0,y0,z0,x1,y1,z1].
pub fn chunk_keys_for_bounds(bounds: [u32; 6], chunk_size: u32) -> Vec<ChunkKey> {
    let cs = chunk_size.max(4);
    let x0 = bounds[0] / cs;
    let x1 = bounds[3] / cs;
    let y0 = bounds[1] / cs;
    let y1 = bounds[4] / cs;
    let z0 = bounds[2] / cs;
    let z1 = bounds[5] / cs;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let key = ChunkKey {
                    x: x as i32,
                    y: y as i32,
                    z: z as i32,
                };
                if seen.insert(key) {
                    out.push(key);
                }
            }
        }
    }
    out
}

/// Convert a chunk key to inclusive cube-index bounds for meshing.
pub fn chunk_bounds_for_key(key: ChunkKey, chunk_size: u32, dims: [u32; 3]) -> Option<[u32; 6]> {
    if key.x < 0 || key.y < 0 || key.z < 0 {
        return None;
    }
    let cs = chunk_size.max(4);
    let sx = key.x as u32 * cs;
    let sy = key.y as u32 * cs;
    let sz = key.z as u32 * cs;
    if sx >= dims[0].saturating_sub(1) || sy >= dims[1].saturating_sub(1) || sz >= dims[2].saturating_sub(1) {
        return None;
    }
    let ex = sx.saturating_add(cs - 1).min(dims[0].saturating_sub(2));
    let ey = sy.saturating_add(cs - 1).min(dims[1].saturating_sub(2));
    let ez = sz.saturating_add(cs - 1).min(dims[2].saturating_sub(2));
    Some([sx, sy, sz, ex, ey, ez])
}

/// Convert a chunk key to typed bounds for callers that prefer structured bounds.
pub fn chunk_bounds_struct_for_key(
    key: ChunkKey,
    chunk_size: u32,
    dims: [u32; 3],
) -> Option<ChunkBounds> {
    let b = chunk_bounds_for_key(key, chunk_size, dims)?;
    Some(ChunkBounds {
        min: [b[0], b[1], b[2]],
        max: [b[3], b[4], b[5]],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_keys_for_bounds() {
        let keys = chunk_keys_for_bounds([0, 0, 0, 33, 33, 33], 32);
        assert_eq!(keys.len(), 8);
    }

    #[test]
    fn test_chunk_bounds_for_key_clamps() {
        let b = chunk_bounds_for_key(ChunkKey { x: 1, y: 0, z: 0 }, 32, [50, 40, 30]).unwrap();
        assert_eq!(b, [32, 0, 0, 48, 31, 28]);
    }

    #[test]
    fn test_chunk_bounds_struct_for_key() {
        let b =
            chunk_bounds_struct_for_key(ChunkKey { x: 0, y: 0, z: 0 }, 16, [32, 32, 32]).unwrap();
        assert_eq!(b.min, [0, 0, 0]);
        assert_eq!(b.max, [15, 15, 15]);
    }
}
