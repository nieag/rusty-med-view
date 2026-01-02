// src/nifti_loader.rs
//! NIfTI volume loading module with platform-agnostic byte-based API.
//!
//! Supports both `.nii` and `.nii.gz` files via gzip detection.

use flate2::read::GzDecoder;
use nifti::{InMemNiftiVolume, NiftiHeader, RandomAccessNiftiVolume};
use std::io::{Cursor, Read};

/// Result of loading a NIfTI volume
#[derive(Debug)]
pub struct LoadedVolume {
    /// Volume dimensions [width, height, depth]
    pub dimensions: [u32; 3],
    /// Voxel spacing in mm [x, y, z]
    pub spacing: [f32; 3],
    /// Raw intensity data as f32 (HU or similar units)
    pub float_data: Vec<f32>,
    /// Data range [min, max]
    pub intensity_range: [f32; 2],
}

/// Error type for NIfTI loading operations
#[derive(Debug)]
pub enum LoadError {
    InvalidMagic,
    DecompressionFailed(String),
    HeaderParseFailed(String),
    VolumeParseFailed(String),
    UnsupportedDataType(String),
    DimensionError(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::InvalidMagic => write!(f, "Invalid NIfTI file magic number"),
            LoadError::DecompressionFailed(e) => write!(f, "Gzip decompression failed: {}", e),
            LoadError::HeaderParseFailed(e) => write!(f, "Failed to parse NIfTI header: {}", e),
            LoadError::VolumeParseFailed(e) => write!(f, "Failed to parse NIfTI volume: {}", e),
            LoadError::UnsupportedDataType(t) => write!(f, "Unsupported NIfTI data type: {}", t),
            LoadError::DimensionError(e) => write!(f, "Invalid volume dimensions: {}", e),
        }
    }
}

impl std::error::Error for LoadError {}

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
        .map_err(|e| LoadError::DecompressionFailed(e.to_string()))?;
    Ok(decompressed)
}

/// Load a NIfTI volume from raw bytes (works on both native and WASM)
///
/// Automatically handles gzip decompression if the file is compressed.
pub fn load_nifti_from_bytes(data: &[u8]) -> Result<LoadedVolume, LoadError> {
    // Handle gzip compression
    let decompressed;
    let raw_data = if is_gzipped(data) {
        decompressed = decompress_gzip(data)?;
        &decompressed[..]
    } else {
        data
    };

    // Parse header
    let mut cursor = Cursor::new(raw_data);
    let header = NiftiHeader::from_reader(&mut cursor)
        .map_err(|e| LoadError::HeaderParseFailed(e.to_string()))?;

    // Get dimensions
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

    // Get voxel spacing from pixdim
    let spacing = [header.pixdim[1], header.pixdim[2], header.pixdim[3]];

    // Skip to volume data (header is 352 bytes for NIfTI-1)
    let vox_offset = header.vox_offset as usize;
    if vox_offset > raw_data.len() {
        return Err(LoadError::VolumeParseFailed(format!(
            "vox_offset {} exceeds file size {}",
            vox_offset,
            raw_data.len()
        )));
    }

    let volume_data = &raw_data[vox_offset..];
    let volume_cursor = Cursor::new(volume_data);

    // Load the volume
    let volume = InMemNiftiVolume::from_reader(volume_cursor, &header)
        .map_err(|e| LoadError::VolumeParseFailed(e.to_string()))?;

    // Convert volume data to f32 intensities
    let total_voxels = (width * height * depth) as usize;
    let mut intensity_data = Vec::with_capacity(total_voxels);

    // Get scaling factors
    let scl_slope = if header.scl_slope == 0.0 {
        1.0
    } else {
        header.scl_slope
    };
    let scl_inter = header.scl_inter;

    // Read voxel values and apply scaling
    for z in 0..depth as u16 {
        for y in 0..height as u16 {
            for x in 0..width as u16 {
                let value: f64 = volume.get_f64(&[x, y, z]).unwrap_or(0.0);
                let scaled = (value * scl_slope as f64 + scl_inter as f64) as f32;
                intensity_data.push(scaled);
            }
        }
    }

    // Find min/max for normalization
    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;
    for &v in &intensity_data {
        if v.is_finite() {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }

    // Avoid division by zero
    if max_val <= min_val {
        max_val = min_val + 1.0;
    }

    // No longer convert to RGBA8 - return raw float data for HU-based windowing
    Ok(LoadedVolume {
        dimensions: [width, height, depth],
        spacing,
        float_data: intensity_data,
        intensity_range: [min_val, max_val],
    })
}

/// Specialized loader for labelmaps (raw u8 IDs, no normalization)
pub fn load_label_from_bytes(
    data: &[u8],
    filename: String,
) -> Result<crate::components::LoadedLabel, LoadError> {
    // Reuse decompression and header parsing logic (simplified for this brief)
    // For brevity, I will re-implement the core loop or refactor later.
    // Actually, let's just do a clean implementation for Labels.
    let decompressed;
    let raw_data = if is_gzipped(data) {
        decompressed = decompress_gzip(data)?;
        &decompressed[..]
    } else {
        data
    };

    let mut cursor = Cursor::new(raw_data);
    let header = NiftiHeader::from_reader(&mut cursor)
        .map_err(|e| LoadError::HeaderParseFailed(e.to_string()))?;

    let dims = header
        .dim()
        .map_err(|e| LoadError::DimensionError(e.to_string()))?;
    let width = dims[0] as u32;
    let height = dims[1] as u32;
    let depth = dims[2] as u32;
    let spacing = [header.pixdim[1], header.pixdim[2], header.pixdim[3]];

    let vox_offset = header.vox_offset as usize;
    let volume_data = &raw_data[vox_offset..];
    let volume_cursor = Cursor::new(volume_data);
    let volume = InMemNiftiVolume::from_reader(volume_cursor, &header)
        .map_err(|e| LoadError::VolumeParseFailed(e.to_string()))?;

    let total_voxels = (width * height * depth) as usize;
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
        data: label_data,
        filename,
    })
}

/// Load a NIfTI volume from a file path (native only)
#[cfg(not(target_arch = "wasm32"))]
pub fn load_nifti_from_file(path: &std::path::Path) -> Result<LoadedVolume, LoadError> {
    let data = std::fs::read(path)
        .map_err(|e| LoadError::HeaderParseFailed(format!("Failed to read file: {}", e)))?;
    load_nifti_from_bytes(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gzip_detection() {
        assert!(is_gzipped(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzipped(&[0x00, 0x00, 0x00]));
        assert!(!is_gzipped(&[0x1f])); // Too short
    }
}
