use crate::app::segment::SdfVolume;

/// Quantized TSDF chunk for chunk-local storage and meshing.
#[derive(Debug, Clone)]
pub struct TsdfChunk {
    pub dims: [u32; 3],
    pub voxel_size: [f32; 3],
    pub origin: [f32; 3],
    pub truncation_mm: f32,
    pub values: Vec<i16>,
    pub weight: Vec<u8>,
    pub revision: u64,
}

impl TsdfChunk {
    #[inline]
    pub fn voxel_count(&self) -> usize {
        (self.dims[0] * self.dims[1] * self.dims[2]) as usize
    }
}

#[inline]
pub fn quantize_tsdf(value_mm: f32, truncation_mm: f32) -> i16 {
    let t = truncation_mm.max(1e-6);
    let normalized = (value_mm / t).clamp(-1.0, 1.0);
    (normalized * i16::MAX as f32).round() as i16
}

#[inline]
pub fn dequantize_tsdf(value_q: i16, truncation_mm: f32) -> f32 {
    let t = truncation_mm.max(1e-6);
    (value_q as f32 / i16::MAX as f32) * t
}

/// Build a quantized TSDF chunk by sampling an SDF over inclusive index bounds.
/// Bounds format: [x0, y0, z0, x1, y1, z1].
pub fn build_tsdf_chunk_from_sdf(
    sdf: &SdfVolume,
    bounds: [u32; 6],
    truncation_mm: f32,
    revision: u64,
) -> Option<TsdfChunk> {
    let dims = sdf.dimensions;
    if dims.contains(&0) {
        return None;
    }
    let x0 = bounds[0].min(dims[0].saturating_sub(1));
    let y0 = bounds[1].min(dims[1].saturating_sub(1));
    let z0 = bounds[2].min(dims[2].saturating_sub(1));
    let x1 = bounds[3].min(dims[0].saturating_sub(1));
    let y1 = bounds[4].min(dims[1].saturating_sub(1));
    let z1 = bounds[5].min(dims[2].saturating_sub(1));
    if x0 > x1 || y0 > y1 || z0 > z1 {
        return None;
    }

    let cx = x1 - x0 + 1;
    let cy = y1 - y0 + 1;
    let cz = z1 - z0 + 1;
    let mut values = Vec::with_capacity((cx * cy * cz) as usize);
    let mut weight = Vec::with_capacity((cx * cy * cz) as usize);
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let v = sdf.get(x, y, z);
                if v.is_finite() && v != f32::MAX {
                    values.push(quantize_tsdf(v, truncation_mm));
                    weight.push(1);
                } else {
                    values.push(quantize_tsdf(truncation_mm, truncation_mm));
                    weight.push(0);
                }
            }
        }
    }

    Some(TsdfChunk {
        dims: [cx, cy, cz],
        voxel_size: sdf.spacing,
        origin: sdf.index_to_world([x0, y0, z0]),
        truncation_mm,
        values,
        weight,
        revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantize_dequantize_roundtrip() {
        let t = 16.0;
        let v = -3.25;
        let q = quantize_tsdf(v, t);
        let r = dequantize_tsdf(q, t);
        assert!((r - v).abs() < 0.1);
    }

    #[test]
    fn test_build_tsdf_chunk_from_sdf_bounds() {
        let mut sdf = SdfVolume::new([8, 8, 8], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        sdf.set(2, 3, 4, -1.5);
        let chunk = build_tsdf_chunk_from_sdf(&sdf, [2, 3, 4, 5, 6, 7], 12.0, 7).unwrap();
        assert_eq!(chunk.dims, [4, 4, 4]);
        assert_eq!(chunk.voxel_count(), 64);
        assert_eq!(chunk.values.len(), 64);
        assert_eq!(chunk.weight.len(), 64);
        assert_eq!(chunk.revision, 7);
    }
}
