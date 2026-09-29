// src/nifti_loader.rs
//! NIfTI volume loading module with platform-agnostic byte-based API.
//!
//! Supports both `.nii` and `.nii.gz` files via gzip detection.

use flate2::read::GzDecoder;
use nifti::{InMemNiftiVolume, NiftiHeader, RandomAccessNiftiVolume};
use std::io::{Cursor, Read};
use thiserror::Error;

/// Result of loading a NIfTI volume
#[derive(Debug)]
pub struct LoadedVolume {
    /// Volume dimensions [width, height, depth]
    pub dimensions: [u32; 3],
    /// Voxel spacing in mm [x, y, z]
    pub spacing: [f32; 3],
    /// Origin/translation in patient/world mm from NIfTI sform
    pub origin: [f32; 3],
    /// Raw intensity data as f32 (HU or similar units)
    pub float_data: Vec<f32>,
    /// Data range [min, max]
    pub intensity_range: [f32; 2],
    /// Orientation quaternion from NIfTI affine matrix [x, y, z, w]
    pub orientation: [f32; 4],
}

/// Error type for NIfTI loading operations
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("Gzip decompression failed: {0}")]
    DecompressionFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to parse NIfTI header: {0}")]
    HeaderParseFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to parse NIfTI volume: {0}")]
    VolumeParseFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Invalid volume dimensions: {0}")]
    DimensionError(String),
}

/// Check if data starts with gzip magic bytes
fn is_gzipped(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

/// Decompress gzipped data
fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, LoadError> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| LoadError::DecompressionFailed(e.into()))?;
    Ok(decompressed)
}

/// Extract rotation matrix from the NIfTI spatial affine and convert to quaternion.
///
/// Prefer valid sform rows, then fall back to qform. This matters for common
/// NIfTI pairs where the image stores patient-space translation in qform while
/// the label stores it in sform.
fn extract_orientation_from_header(header: &NiftiHeader) -> [f32; 4] {
    let Some([row_x, row_y, row_z]) = spatial_affine_rows(header) else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    calculate_orientation_from_rows(row_x, row_y, row_z)
}

/// Extract translation/origin from the NIfTI spatial affine.
fn extract_origin_from_header(header: &NiftiHeader) -> [f32; 3] {
    let Some([row_x, row_y, row_z]) = spatial_affine_rows(header) else {
        return [0.0, 0.0, 0.0];
    };
    extract_origin_from_rows(row_x, row_y, row_z)
}

fn spatial_affine_rows(header: &NiftiHeader) -> Option<[[f32; 4]; 3]> {
    if has_valid_sform_axes(header.srow_x, header.srow_y, header.srow_z) {
        return Some([header.srow_x, header.srow_y, header.srow_z]);
    }

    if header.qform_code != 0 && qform_fields_are_usable(header) {
        let rows = qform_affine_rows(header)?;
        if has_valid_sform_axes(rows[0], rows[1], rows[2]) {
            return Some(rows);
        }
    }

    None
}

fn qform_fields_are_usable(header: &NiftiHeader) -> bool {
    let qfac = header.pixdim[0];
    let qfac_valid = (qfac - 1.0).abs() <= 1e-6 || (qfac + 1.0).abs() <= 1e-6;
    let quat_vector_len2 = header.quatern_b * header.quatern_b
        + header.quatern_c * header.quatern_c
        + header.quatern_d * header.quatern_d;
    qfac_valid
        && header.pixdim[1] > 0.0
        && header.pixdim[2] > 0.0
        && header.pixdim[3] > 0.0
        && header.pixdim[1].is_finite()
        && header.pixdim[2].is_finite()
        && header.pixdim[3].is_finite()
        && header.quatern_x.is_finite()
        && header.quatern_y.is_finite()
        && header.quatern_z.is_finite()
        && header.quatern_b.is_finite()
        && header.quatern_c.is_finite()
        && header.quatern_d.is_finite()
        && quat_vector_len2 <= 1.0 + 1e-5
}

fn qform_affine_rows(header: &NiftiHeader) -> Option<[[f32; 4]; 3]> {
    let b = header.quatern_b;
    let c = header.quatern_c;
    let d = header.quatern_d;
    let a2 = 1.0 - (b * b + c * c + d * d);
    let a = if a2 > 0.0 { a2.sqrt() } else { 0.0 };
    let rotation = glam::Mat3::from_quat(glam::Quat::from_xyzw(b, c, d, a).normalize());
    let qfac = if header.pixdim[0] < 0.0 { -1.0 } else { 1.0 };
    let col0 = rotation.x_axis * header.pixdim[1];
    let col1 = rotation.y_axis * header.pixdim[2];
    let col2 = rotation.z_axis * header.pixdim[3] * qfac;
    let rows = [
        [col0.x, col1.x, col2.x, header.quatern_x],
        [col0.y, col1.y, col2.y, header.quatern_y],
        [col0.z, col1.z, col2.z, header.quatern_z],
    ];

    if rows.iter().flatten().all(|component| component.is_finite()) {
        Some(rows)
    } else {
        None
    }
}

fn extract_origin_from_rows(srow_x: [f32; 4], srow_y: [f32; 4], srow_z: [f32; 4]) -> [f32; 3] {
    if !has_valid_sform_axes(srow_x, srow_y, srow_z) {
        return [0.0, 0.0, 0.0];
    }

    let origin = [srow_x[3], srow_y[3], srow_z[3]];
    if origin.iter().all(|component| component.is_finite()) {
        origin
    } else {
        [0.0, 0.0, 0.0]
    }
}

fn has_valid_sform_axes(srow_x: [f32; 4], srow_y: [f32; 4], srow_z: [f32; 4]) -> bool {
    let col0 = [srow_x[0], srow_y[0], srow_z[0]];
    let col1 = [srow_x[1], srow_y[1], srow_z[1]];
    let col2 = [srow_x[2], srow_y[2], srow_z[2]];

    let len0 = (col0[0] * col0[0] + col0[1] * col0[1] + col0[2] * col0[2]).sqrt();
    let len1 = (col1[0] * col1[0] + col1[1] * col1[1] + col1[2] * col1[2]).sqrt();
    let len2 = (col2[0] * col2[0] + col2[1] * col2[1] + col2[2] * col2[2]).sqrt();

    len0 >= 1e-6
        && len1 >= 1e-6
        && len2 >= 1e-6
        && len0.is_finite()
        && len1.is_finite()
        && len2.is_finite()
}

/// Calculate orientation quaternion from sform rows.
/// Split out for unit testing without NiftiHeader.
fn calculate_orientation_from_rows(
    srow_x: [f32; 4],
    srow_y: [f32; 4],
    srow_z: [f32; 4],
) -> [f32; 4] {
    // Build 3x3 matrix (column vectors)
    // srow_x = [m00, m01, m02, tx] means first row of rotation is [m00, m01, m02]
    // We want column vectors for normalization
    let col0 = [srow_x[0], srow_y[0], srow_z[0]];
    let col1 = [srow_x[1], srow_y[1], srow_z[1]];
    let col2 = [srow_x[2], srow_y[2], srow_z[2]];

    // Check if sform is valid (non-zero columns)
    let len0 = (col0[0] * col0[0] + col0[1] * col0[1] + col0[2] * col0[2]).sqrt();
    let len1 = (col1[0] * col1[0] + col1[1] * col1[1] + col1[2] * col1[2]).sqrt();
    let len2 = (col2[0] * col2[0] + col2[1] * col2[1] + col2[2] * col2[2]).sqrt();

    if len0 < 1e-6 || len1 < 1e-6 || len2 < 1e-6 {
        // Invalid sform, return identity quaternion
        return [0.0, 0.0, 0.0, 1.0];
    }

    // Normalize columns to get pure rotation (removes scale)
    let col0 = normalize_vec3(col0);
    let col1 = normalize_vec3(col1);
    let col2 = normalize_vec3(col2);

    // Rotation matrix (row-major for quaternion conversion)
    // m[row][col]
    let m = [
        [col0[0], col1[0], col2[0]],
        [col0[1], col1[1], col2[1]],
        [col0[2], col1[2], col2[2]],
    ];

    // Convert rotation matrix to quaternion using Shepperd's method
    rotation_matrix_to_quaternion(m)
}

/// Normalize a 3D vector to unit length
fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-8 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [1.0, 0.0, 0.0] // Default to X axis if degenerate
    }
}

/// Convert a 3x3 rotation matrix to quaternion [x, y, z, w]
/// Uses Shepperd's method for numerical stability
fn rotation_matrix_to_quaternion(m: [[f32; 3]; 3]) -> [f32; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];

    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0; // s = 4 * w
        let w = 0.25 * s;
        let x = (m[2][1] - m[1][2]) / s;
        let y = (m[0][2] - m[2][0]) / s;
        let z = (m[1][0] - m[0][1]) / s;
        [x, y, z, w]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0; // s = 4 * x
        let w = (m[2][1] - m[1][2]) / s;
        let x = 0.25 * s;
        let y = (m[0][1] + m[1][0]) / s;
        let z = (m[0][2] + m[2][0]) / s;
        [x, y, z, w]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0; // s = 4 * y
        let w = (m[0][2] - m[2][0]) / s;
        let x = (m[0][1] + m[1][0]) / s;
        let y = 0.25 * s;
        let z = (m[1][2] + m[2][1]) / s;
        [x, y, z, w]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0; // s = 4 * z
        let w = (m[1][0] - m[0][1]) / s;
        let x = (m[0][2] + m[2][0]) / s;
        let y = (m[1][2] + m[2][1]) / s;
        let z = 0.25 * s;
        [x, y, z, w]
    }
}

/// Shared parsed result from decompression + header parsing + volume loading.
struct ParsedNifti {
    header: NiftiHeader,
    volume: InMemNiftiVolume,
    /// [width, height, depth]
    dimensions: [u32; 3],
    total_voxels: usize,
}

/// Parse raw NIfTI bytes (optionally gzip-compressed) into a `ParsedNifti`.
///
/// Handles: gzip detection, decompression, header parsing, 3D dimension
/// validation, u16 bounds check, vox_offset bounds check, volume loading,
/// and checked total-voxel multiplication.
fn parse_nifti_raw(data: &[u8]) -> Result<ParsedNifti, LoadError> {
    let decompressed;
    let raw_data = if is_gzipped(data) {
        decompressed = decompress_gzip(data)?;
        &decompressed[..]
    } else {
        data
    };

    let mut cursor = Cursor::new(raw_data);
    let header = NiftiHeader::from_reader(&mut cursor)
        .map_err(|e| LoadError::HeaderParseFailed(e.into()))?;

    let dims = header
        .dim()
        .map_err(|e| LoadError::DimensionError(e.to_string()))?;
    if dims.len() < 3 {
        return Err(LoadError::DimensionError(format!(
            "Expected 3D volume, got {}D",
            dims.len()
        )));
    }

    let width = dims[0] as u32;
    let height = dims[1] as u32;
    let depth = dims[2] as u32;

    // Validate dimensions fit in u16 (required by the nifti crate's get_f64 index API)
    for (name, dim) in [("width", width), ("height", height), ("depth", depth)] {
        if dim > u16::MAX as u32 {
            return Err(LoadError::DimensionError(format!(
                "{name} dimension {dim} exceeds maximum supported size of {}",
                u16::MAX
            )));
        }
    }

    let vox_offset = header.vox_offset as usize;
    if vox_offset > raw_data.len() {
        return Err(LoadError::VolumeParseFailed(
            format!(
                "vox_offset {} exceeds file size {}",
                vox_offset,
                raw_data.len()
            )
            .into(),
        ));
    }

    let volume_data = &raw_data[vox_offset..];
    let volume = InMemNiftiVolume::from_reader(Cursor::new(volume_data), &header)
        .map_err(|e| LoadError::VolumeParseFailed(e.into()))?;

    let total_voxels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|wh| wh.checked_mul(depth as usize))
        .ok_or_else(|| {
            LoadError::DimensionError(format!(
                "Volume {width}x{height}x{depth} exceeds addressable memory"
            ))
        })?;

    Ok(ParsedNifti {
        header,
        volume,
        dimensions: [width, height, depth],
        total_voxels,
    })
}

/// Load a NIfTI volume from raw bytes (works on both native and WASM)
///
/// Automatically handles gzip decompression if the file is compressed.
pub fn load_nifti_from_bytes(data: &[u8]) -> Result<LoadedVolume, LoadError> {
    let ParsedNifti {
        header,
        volume,
        dimensions: [width, height, depth],
        total_voxels,
    } = parse_nifti_raw(data)?;

    let spacing = [header.pixdim[1], header.pixdim[2], header.pixdim[3]];
    let origin = extract_origin_from_header(&header);
    let scl_slope = if header.scl_slope == 0.0 {
        1.0
    } else {
        header.scl_slope
    };
    let scl_inter = header.scl_inter;

    let mut intensity_data = Vec::with_capacity(total_voxels);
    for z in 0..depth as u16 {
        for y in 0..height as u16 {
            for x in 0..width as u16 {
                let value: f64 = volume.get_f64(&[x, y, z]).unwrap_or(0.0);
                intensity_data.push((value * scl_slope as f64 + scl_inter as f64) as f32);
            }
        }
    }

    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;
    for &v in &intensity_data {
        if v.is_finite() {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }
    if max_val <= min_val {
        max_val = min_val + 1.0;
    }

    let orientation = extract_orientation_from_header(&header);

    Ok(LoadedVolume {
        dimensions: [width, height, depth],
        spacing,
        origin,
        float_data: intensity_data,
        intensity_range: [min_val, max_val],
        orientation,
    })
}

/// Specialized loader for labelmaps (raw u8 IDs, no normalization)
pub fn load_label_from_bytes(
    data: &[u8],
    filename: String,
) -> Result<crate::components::LoadedLabel, LoadError> {
    let ParsedNifti {
        header,
        volume,
        dimensions: [width, height, depth],
        total_voxels,
        ..
    } = parse_nifti_raw(data)?;

    let spacing = [header.pixdim[1], header.pixdim[2], header.pixdim[3]];
    let origin = extract_origin_from_header(&header);
    let orientation = extract_orientation_from_header(&header);

    let mut label_data = Vec::with_capacity(total_voxels);
    for z in 0..depth as u16 {
        for y in 0..height as u16 {
            for x in 0..width as u16 {
                let value: f64 = volume.get_f64(&[x, y, z]).unwrap_or(0.0);
                label_data.push(value.clamp(0.0, 255.0) as u8);
            }
        }
    }

    Ok(crate::components::LoadedLabel {
        dimensions: [width, height, depth],
        spacing,
        origin,
        orientation,
        data: label_data,
        filename,
    })
}

#[cfg(test)]
mod tests;
