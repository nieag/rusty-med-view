// src/nifti_loader.rs
//! NIfTI volume loading module with platform-agnostic byte-based API.
//!
//! Supports both `.nii` and `.nii.gz` files via gzip detection.

use crate::app::roi::VoxelGeometry;
use flate2::read::GzDecoder;
use glam::{DMat4, DVec3, DVec4};
use nifti::{InMemNiftiVolume, NiftiHeader, RandomAccessNiftiVolume};
use std::io::{Cursor, Read};
use thiserror::Error;

/// Result of loading a NIfTI volume
#[derive(Debug)]
pub struct LoadedVolume {
    /// Volume dimensions [width, height, depth]
    pub dimensions: [u32; 3],
    /// Validated voxel grid with the full IJK-to-world affine from the NIfTI sform or qform
    pub geometry: VoxelGeometry,
    /// Raw intensity data as f32 (HU or similar units)
    pub float_data: Vec<f32>,
    /// Data range [min, max]
    pub intensity_range: [f32; 2],
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
    #[error("Invalid spatial geometry: {0}")]
    InvalidGeometry(String),
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

/// Spatial affine rows from the header, following the NIfTI-1 precedence: a valid sform
/// (`sform_code > 0`), then a valid qform (`qform_code > 0`). `None` means neither is usable.
fn spatial_affine_rows(header: &NiftiHeader) -> Option<[[f32; 4]; 3]> {
    if header.sform_code > 0 && has_valid_sform_axes(header.srow_x, header.srow_y, header.srow_z) {
        return Some([header.srow_x, header.srow_y, header.srow_z]);
    }

    if header.qform_code > 0 && qform_fields_are_usable(header) {
        let rows = qform_affine_rows(header)?;
        if has_valid_sform_axes(rows[0], rows[1], rows[2]) {
            return Some(rows);
        }
    }

    None
}

/// Full IJK-to-world affine for the header. Reflections and shear in the sform are preserved;
/// without a usable sform or qform the legacy `pixdim` scale with zero origin is used.
fn spatial_affine(header: &NiftiHeader) -> Result<DMat4, LoadError> {
    if let Some([row_x, row_y, row_z]) = spatial_affine_rows(header) {
        let column = |index: usize, w: f64| {
            DVec4::new(
                f64::from(row_x[index]),
                f64::from(row_y[index]),
                f64::from(row_z[index]),
                w,
            )
        };
        return Ok(DMat4::from_cols(
            column(0, 0.0),
            column(1, 0.0),
            column(2, 0.0),
            column(3, 1.0),
        ));
    }

    let spacing = [header.pixdim[1], header.pixdim[2], header.pixdim[3]];
    if spacing
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(LoadError::InvalidGeometry(format!(
            "no usable sform or qform and pixdim spacing {spacing:?} is not positive"
        )));
    }
    Ok(DMat4::from_scale(DVec3::new(
        f64::from(spacing[0]),
        f64::from(spacing[1]),
        f64::from(spacing[2]),
    )))
}

fn geometry_from_header(
    header: &NiftiHeader,
    dimensions: [u32; 3],
) -> Result<VoxelGeometry, LoadError> {
    VoxelGeometry::from_affine(dimensions, spatial_affine(header)?)
        .map_err(|error| LoadError::InvalidGeometry(error.to_string()))
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

fn has_valid_sform_axes(srow_x: [f32; 4], srow_y: [f32; 4], srow_z: [f32; 4]) -> bool {
    let col0 = [srow_x[0], srow_y[0], srow_z[0]];
    let col1 = [srow_x[1], srow_y[1], srow_z[1]];
    let col2 = [srow_x[2], srow_y[2], srow_z[2]];

    let len0 = (col0[0] * col0[0] + col0[1] * col0[1] + col0[2] * col0[2]).sqrt();
    let len1 = (col1[0] * col1[0] + col1[1] * col1[1] + col1[2] * col1[2]).sqrt();
    let len2 = (col2[0] * col2[0] + col2[1] * col2[1] + col2[2] * col2[2]).sqrt();
    let translation_finite = [srow_x[3], srow_y[3], srow_z[3]]
        .iter()
        .all(|value| value.is_finite());

    len0 >= 1e-6
        && len1 >= 1e-6
        && len2 >= 1e-6
        && len0.is_finite()
        && len1.is_finite()
        && len2.is_finite()
        && translation_finite
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

    let geometry = geometry_from_header(&header, [width, height, depth])?;
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

    Ok(LoadedVolume {
        dimensions: [width, height, depth],
        geometry,
        float_data: intensity_data,
        intensity_range: [min_val, max_val],
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

    let geometry = geometry_from_header(&header, [width, height, depth])?;

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
        geometry,
        data: label_data,
        filename,
    })
}

#[cfg(test)]
mod tests;
