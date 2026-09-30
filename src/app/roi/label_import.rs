//! Splitting a multi-label labelmap into one ROI per label.
//!
//! A labelmap imported from a segmentation model usually holds several structures (liver 1,
//! tumor 2, and so on). Contour, mesh, and SDF derivation treat any non-zero voxel as inside, so
//! importing the whole map as one ROI would merge the structures on the first conversion. Each
//! label becomes its own ROI, keeping its id as the voxel value so the overlay colormap still
//! colours it the same way.

use crate::io::volume::label_lut_rgba;

/// Upper bound on the voxels held by all masks of one import.
///
/// Each mask is cropped to the bounding box of its label, so the total is about the volume of the
/// structures rather than labels times the volume. The bound is a guard against a labelmap whose
/// labels are scattered over the whole volume; it fails with a message instead of exhausting
/// memory.
pub const MAX_IMPORT_LABEL_VOXELS: usize = 256 * 1024 * 1024;

/// One label of a split labelmap, cropped to its bounding box. `data` has the label id where the
/// label is present and 0 elsewhere, over `dimensions` voxels starting at voxel `min` of the
/// labelmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMask {
    pub label: u8,
    pub min: [u32; 3],
    pub dimensions: [u32; 3],
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

/// Splits the map into one cropped mask per id in `labels` (from [`present_label_ids`]) in two
/// passes: bounding boxes, then the crops. `dimensions` are the labelmap's voxel dimensions.
pub fn split_labelmap(
    data: &[u8],
    dimensions: [u32; 3],
    labels: &[u8],
) -> Result<Vec<LabelMask>, String> {
    let [width, height, depth] = dimensions.map(|value| value as usize);
    if data.len() != width * height * depth {
        return Err(format!(
            "Labelmap has {} voxels but its dimensions are {dimensions:?}.",
            data.len()
        ));
    }
    let mut slot_of = [usize::MAX; 256];
    for (slot, label) in labels.iter().enumerate() {
        slot_of[usize::from(*label)] = slot;
    }
    let mut min = vec![[usize::MAX; 3]; labels.len()];
    let mut max = vec![[0usize; 3]; labels.len()];
    for z in 0..depth {
        for y in 0..height {
            let row = (z * height + y) * width;
            for x in 0..width {
                let value = data[row + x];
                let slot = slot_of[usize::from(value)];
                if value != 0 && slot != usize::MAX {
                    for (axis, coordinate) in [x, y, z].into_iter().enumerate() {
                        min[slot][axis] = min[slot][axis].min(coordinate);
                        max[slot][axis] = max[slot][axis].max(coordinate);
                    }
                }
            }
        }
    }
    let mut total = 0usize;
    let mut masks: Vec<LabelMask> = Vec::with_capacity(labels.len());
    for (slot, label) in labels.iter().enumerate() {
        let size: [usize; 3] = std::array::from_fn(|axis| max[slot][axis] - min[slot][axis] + 1);
        total = total.saturating_add(size[0] * size[1] * size[2]);
        if total > MAX_IMPORT_LABEL_VOXELS {
            return Err(format!(
                "The cropped labels need over {MAX_IMPORT_LABEL_VOXELS} voxels in total; the \
                 labelmap is too large to import as one ROI per label."
            ));
        }
        masks.push(LabelMask {
            label: *label,
            min: min[slot].map(|value| value as u32),
            dimensions: size.map(|value| value as u32),
            data: vec![0; size[0] * size[1] * size[2]],
        });
    }
    for z in 0..depth {
        for y in 0..height {
            let row = (z * height + y) * width;
            for x in 0..width {
                let value = data[row + x];
                let slot = slot_of[usize::from(value)];
                if value != 0 && slot != usize::MAX {
                    let mask = &mut masks[slot];
                    let size = mask.dimensions.map(|value| value as usize);
                    let local = [x - min[slot][0], y - min[slot][1], z - min[slot][2]];
                    mask.data[(local[2] * size[1] + local[1]) * size[0] + local[0]] = value;
                }
            }
        }
    }
    Ok(masks)
}

/// ROI name for a label: the file name alone for a single label, otherwise with the label id.
pub fn label_roi_name(filename: &str, label: u8, label_count: usize) -> String {
    if label_count <= 1 {
        filename.to_string()
    } else {
        format!("{filename} [label {label}]")
    }
}

/// ROI colour for a label: the complement of the label's overlay colour. Contours and the 3D
/// mesh are drawn in the ROI colour on top of the voxel overlay, so a colour equal to the overlay
/// would make them invisible; the complement stands out from both the overlay and the image, and
/// still differs per label.
pub fn label_color(label: u8) -> [f32; 4] {
    let [r, g, b, _] = label_lut_rgba(label);
    let complement = |channel: u8| 1.0 - f32::from(channel) / 255.0;
    [complement(r), complement(g), complement(b), 1.0]
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
    fn test_split_labelmap_crops_each_label_and_keeps_ids() {
        // A 4x2x1 map: labels 1 (left), 2 (right), and a 3 on its own.
        let data = [0, 1, 1, 2, 0, 2, 3, 1];
        let masks = split_labelmap(&data, [4, 2, 1], &present_label_ids(&data)).unwrap();

        assert_eq!(masks.len(), 3);
        assert_eq!(masks[0].label, 1);
        assert_eq!((masks[0].min, masks[0].dimensions), ([1, 0, 0], [3, 2, 1]));
        assert_eq!(masks[0].data, vec![1, 1, 0, 0, 0, 1]);
        assert_eq!(masks[1].label, 2);
        assert_eq!((masks[1].min, masks[1].dimensions), ([1, 0, 0], [3, 2, 1]));
        assert_eq!(masks[1].data, vec![0, 0, 2, 2, 0, 0]);
        assert_eq!(masks[2].label, 3);
        assert_eq!((masks[2].min, masks[2].dimensions), ([2, 1, 0], [1, 1, 1]));
        assert_eq!(masks[2].data, vec![3]);
    }

    #[test]
    fn test_cropped_masks_partition_the_original_labelmap() {
        let dimensions = [10, 10, 10];
        let data: Vec<u8> = (0..1000_u32).map(|i| (i % 5) as u8).collect();
        let masks = split_labelmap(&data, dimensions, &present_label_ids(&data)).unwrap();

        let mut merged = vec![0_u8; data.len()];
        for mask in &masks {
            let [w, h, d] = mask.dimensions.map(|value| value as usize);
            for z in 0..d {
                for y in 0..h {
                    for x in 0..w {
                        let value = mask.data[(z * h + y) * w + x];
                        let at = ((z + mask.min[2] as usize) * 10 + y + mask.min[1] as usize) * 10
                            + x
                            + mask.min[0] as usize;
                        assert!(merged[at] == 0 || value == 0, "masks must not overlap");
                        merged[at] |= value;
                    }
                }
            }
        }
        assert_eq!(merged, data);
    }

    #[test]
    fn test_split_of_an_empty_map_yields_no_masks() {
        assert!(
            split_labelmap(&[0, 0, 0], [3, 1, 1], &present_label_ids(&[0, 0, 0]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_label_names_use_the_file_name_only_for_a_single_label() {
        assert_eq!(label_roi_name("liver.nii", 1, 1), "liver.nii");
        assert_eq!(label_roi_name("liver.nii", 2, 3), "liver.nii [label 2]");
    }

    #[test]
    fn test_label_color_contrasts_with_the_overlay_colormap() {
        assert_eq!(label_lut_rgba(0), [0, 0, 0, 0]);
        assert_eq!(label_lut_rgba(1), [123, 231, 73, 255]);
        let color = label_color(1);
        assert_eq!(
            color,
            [
                1.0 - 123.0 / 255.0,
                1.0 - 231.0 / 255.0,
                1.0 - 73.0 / 255.0,
                1.0
            ]
        );
        assert_ne!(color[..3], [123.0 / 255.0, 231.0 / 255.0, 73.0 / 255.0]);
        assert_ne!(label_color(1), label_color(2));
    }
}
