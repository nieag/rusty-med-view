//! Splitting a multi-label labelmap into one ROI per label.
//!
//! A labelmap imported from a segmentation model usually holds several structures (liver 1,
//! tumor 2, and so on). Contour, mesh, and SDF derivation treat any non-zero voxel as inside, so
//! importing the whole map as one ROI would merge the structures on the first conversion. Each
//! label becomes its own ROI, keeping its id as the voxel value so the overlay colormap still
//! colours it the same way.

use crate::io::volume::label_lut_rgba;

/// Upper bound on the total number of mask voxels created by one import (masks times voxels).
///
/// Every split ROI holds a full-size mask, an authoritative copy, a cached copy, and a GPU
/// texture, so a many-label import of a large volume would exhaust memory. Exceeding the budget
/// fails with a message instead of crashing; cropping masks to their bounds is tracked in the
/// backlog (item 4.5).
pub const MAX_IMPORT_LABEL_VOXELS: usize = 256 * 1024 * 1024;

/// One label of a split labelmap. `data` has the label id where the label is present and 0
/// elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMask {
    pub label: u8,
    pub data: Vec<u8>,
}

/// Distinct non-zero label ids present in the map, ascending.
pub fn present_label_ids(data: &[u8]) -> Vec<u8> {
    let mut present = [false; 256];
    for value in data {
        present[usize::from(*value)] = true;
    }
    (1..=255_u8)
        .filter(|label| present[usize::from(*label)])
        .collect()
}

/// Rejects imports whose split masks would exceed [`MAX_IMPORT_LABEL_VOXELS`].
pub fn check_label_import_budget(voxel_count: usize, mask_count: usize) -> Result<(), String> {
    let total = voxel_count.saturating_mul(mask_count.max(1));
    if total > MAX_IMPORT_LABEL_VOXELS {
        return Err(format!(
            "Labelmap has {mask_count} labels of {voxel_count} voxels; importing one ROI per label \
             needs {total} mask voxels, over the limit of {MAX_IMPORT_LABEL_VOXELS}."
        ));
    }
    Ok(())
}

/// Splits the map into one mask per id in `labels` (from [`present_label_ids`]) in a single pass.
pub fn split_labelmap(data: &[u8], labels: &[u8]) -> Vec<LabelMask> {
    let mut slot_of = [usize::MAX; 256];
    for (slot, label) in labels.iter().enumerate() {
        slot_of[usize::from(*label)] = slot;
    }
    let mut masks: Vec<LabelMask> = labels
        .iter()
        .map(|label| LabelMask {
            label: *label,
            data: vec![0; data.len()],
        })
        .collect();
    for (index, value) in data.iter().enumerate() {
        let slot = slot_of[usize::from(*value)];
        if *value != 0 && slot != usize::MAX {
            masks[slot].data[index] = *value;
        }
    }
    masks
}

/// ROI name for a label: the file name alone for a single label, otherwise with the label id.
pub fn label_roi_name(filename: &str, label: u8, label_count: usize) -> String {
    if label_count <= 1 {
        filename.to_string()
    } else {
        format!("{filename} [label {label}]")
    }
}

/// ROI colour for a label, matching the overlay colormap.
pub fn label_color(label: u8) -> [f32; 4] {
    label_lut_rgba(label).map(|channel| f32::from(channel) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_present_label_ids_lists_distinct_non_zero_ids_ascending() {
        assert_eq!(present_label_ids(&[0, 2, 0, 1, 2, 7, 0]), vec![1, 2, 7]);
        assert!(present_label_ids(&[0, 0, 0]).is_empty());
        assert!(present_label_ids(&[]).is_empty());
        assert_eq!(present_label_ids(&[255, 0, 255]), vec![255]);
    }

    #[test]
    fn test_split_labelmap_keeps_ids_and_separates_structures() {
        let data = [0, 1, 1, 2, 0, 2, 3, 1];
        let masks = split_labelmap(&data, &present_label_ids(&data));

        assert_eq!(masks.len(), 3);
        assert_eq!(masks[0].label, 1);
        assert_eq!(masks[0].data, vec![0, 1, 1, 0, 0, 0, 0, 1]);
        assert_eq!(masks[1].label, 2);
        assert_eq!(masks[1].data, vec![0, 0, 0, 2, 0, 2, 0, 0]);
        assert_eq!(masks[2].label, 3);
        assert_eq!(masks[2].data, vec![0, 0, 0, 0, 0, 0, 3, 0]);
    }

    #[test]
    fn test_split_masks_partition_the_original_labelmap() {
        let data: Vec<u8> = (0..1000_u32).map(|i| (i % 5) as u8).collect();
        let masks = split_labelmap(&data, &present_label_ids(&data));

        let mut merged = vec![0_u8; data.len()];
        for mask in &masks {
            for (target, value) in merged.iter_mut().zip(&mask.data) {
                assert!(*target == 0 || *value == 0, "masks must not overlap");
                *target |= *value;
            }
        }
        assert_eq!(merged, data);
    }

    #[test]
    fn test_split_of_an_empty_map_yields_no_masks() {
        assert!(split_labelmap(&[0, 0, 0], &present_label_ids(&[0, 0, 0])).is_empty());
    }

    #[test]
    fn test_label_names_use_the_file_name_only_for_a_single_label() {
        assert_eq!(label_roi_name("liver.nii", 1, 1), "liver.nii");
        assert_eq!(label_roi_name("liver.nii", 2, 3), "liver.nii [label 2]");
    }

    #[test]
    fn test_label_color_matches_the_overlay_colormap() {
        assert_eq!(label_lut_rgba(0), [0, 0, 0, 0]);
        assert_eq!(label_lut_rgba(1), [123, 231, 73, 255]);
        assert_eq!(
            label_color(1),
            [123.0 / 255.0, 231.0 / 255.0, 73.0 / 255.0, 1.0]
        );
        assert_ne!(label_color(1), label_color(2));
    }

    #[test]
    fn test_import_budget_rejects_only_oversized_splits() {
        assert!(check_label_import_budget(180 * 180 * 125, 30).is_ok());
        assert!(check_label_import_budget(512 * 512 * 300, 1).is_ok());
        let error = check_label_import_budget(512 * 512 * 300, 20).unwrap_err();
        assert!(error.contains("20 labels"), "{error}");
        assert!(check_label_import_budget(usize::MAX, 2).is_err());
    }
}
