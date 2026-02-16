//! Shared coordinate mapping helpers for slice/view alignment.
//!
//! Conventions:
//! - Slice index from cursor UV uses center-based cursor semantics.
//! - Slice center UV is `(slice + 0.5) / dim`.
//! - Grid-node UV is `idx / (dim - 1)` for `dim > 1`.

/// Compute clamped slice index from cursor UV in `[0,1]`.
pub fn slice_index_from_cursor_uv(cursor_uv: f32, dim: u32) -> i32 {
    if dim == 0 {
        return 0;
    }
    // Match texel-center semantics: uv = (i + 0.5)/dim maps to slice i.
    let idx = (cursor_uv * dim as f32 - 0.5).round() as i32;
    idx.clamp(0, dim as i32 - 1)
}

/// Center UV for a slice index.
pub fn slice_center_uv(slice_index: i32, dim: u32) -> f32 {
    if dim == 0 {
        return 0.5;
    }
    let clamped = slice_index.clamp(0, dim as i32 - 1) as f32;
    (clamped + 0.5) / dim as f32
}

/// Convert a grid node index to normalized UV in `[0,1]`.
pub fn index_to_node_uv(idx: u32, dim: u32) -> f32 {
    if dim <= 1 {
        0.0
    } else {
        idx as f32 / (dim - 1) as f32
    }
}

/// Convert volume UV to world coordinate along one axis.
pub fn uv_to_world_axis(uv: f32, dim: u32, spacing: f32) -> f32 {
    uv * dim.saturating_sub(1) as f32 * spacing
}

/// Convert center-based cursor UV to center world depth (mm).
pub fn cursor_uv_to_world_depth(cursor_uv: f32, dim: u32, spacing: f32) -> f32 {
    cursor_uv * dim as f32 * spacing
}

/// Convert plane distance (mm) back to slice index using center-based slices.
pub fn plane_distance_to_slice_index(distance_mm: f32, spacing: f32, dim: u32) -> i32 {
    if dim == 0 {
        return 0;
    }
    (((distance_mm / spacing.max(1e-6)) - 0.5).round() as i32).clamp(0, dim as i32 - 1)
}

/// Convert world depth to SDF slice index using center-based slice semantics.
pub fn world_depth_to_sdf_slice(
    world_depth: f32,
    sdf_origin: f32,
    sdf_spacing: f32,
    sdf_dim: u32,
) -> i32 {
    if sdf_dim == 0 {
        return 0;
    }
    ((((world_depth - sdf_origin) / sdf_spacing.max(1e-6)) - 0.5).round() as i32)
        .clamp(0, sdf_dim as i32 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_index_from_cursor_uv_clamps() {
        assert_eq!(slice_index_from_cursor_uv(-1.0, 128), 0);
        assert_eq!(slice_index_from_cursor_uv(2.0, 128), 127);
        assert_eq!(slice_index_from_cursor_uv(0.0, 128), 0);
    }

    #[test]
    fn test_slice_center_roundtrip() {
        let dim = 180;
        let slice = 118;
        let uv = slice_center_uv(slice, dim);
        assert_eq!(slice_index_from_cursor_uv(uv, dim), slice);
    }

    #[test]
    fn test_index_to_node_uv_endpoints() {
        assert!((index_to_node_uv(0, 64) - 0.0).abs() < 1e-6);
        assert!((index_to_node_uv(63, 64) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_plane_distance_to_slice_index_matches_center_convention() {
        let spacing = 1.5;
        let dim = 100;
        let slice = 17;
        let dist = (slice as f32 + 0.5) * spacing;
        assert_eq!(plane_distance_to_slice_index(dist, spacing, dim), slice);
    }

    #[test]
    fn test_world_depth_to_sdf_slice_center_convention() {
        let spacing = 0.5;
        let dim = 200;
        let slice = 23;
        let world = (slice as f32 + 0.5) * spacing;
        assert_eq!(world_depth_to_sdf_slice(world, 0.0, spacing, dim), slice);
    }
}
