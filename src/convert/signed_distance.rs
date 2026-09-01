use crate::app::roi::VoxelData;

const DISTANCE_INFINITY: f32 = 1.0e20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedDistanceError {
    InvalidRawDataLength { expected: usize, actual: usize },
}

/// Computes a signed Euclidean distance field in world millimetres.
///
/// Negative values are inside the occupied label. The field is sampled at the
/// ROI grid's voxel centres, so a sign change lies halfway between adjacent
/// inside and outside centres for a binary label.
pub fn signed_distance_from_voxel_data(
    voxel_data: &VoxelData,
) -> Result<Vec<f32>, SignedDistanceError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected = voxel_count(dimensions);
    if voxel_data.raw_data.len() != expected {
        return Err(SignedDistanceError::InvalidRawDataLength {
            expected,
            actual: voxel_data.raw_data.len(),
        });
    }

    let inside = voxel_data
        .raw_data
        .iter()
        .map(|value| *value != 0)
        .collect::<Vec<_>>();
    let distance_to_inside =
        squared_distance_transform(&inside, dimensions, voxel_data.geometry.spacing);
    let outside = inside.iter().map(|value| !value).collect::<Vec<_>>();
    let distance_to_outside =
        squared_distance_transform(&outside, dimensions, voxel_data.geometry.spacing);

    Ok(inside
        .into_iter()
        .zip(distance_to_inside)
        .zip(distance_to_outside)
        .map(|((is_inside, to_inside), to_outside)| {
            let squared = if is_inside { to_outside } else { to_inside };
            let distance = squared.sqrt();
            if is_inside {
                -distance
            } else {
                distance
            }
        })
        .collect())
}

fn squared_distance_transform(seeds: &[bool], dimensions: [u32; 3], spacing: [f32; 3]) -> Vec<f32> {
    let mut values = seeds
        .iter()
        .map(|seed| if *seed { 0.0 } else { DISTANCE_INFINITY })
        .collect::<Vec<_>>();
    if !seeds.iter().any(|seed| *seed) {
        return values;
    }

    transform_axis(&mut values, dimensions, spacing[0], 0);
    transform_axis(&mut values, dimensions, spacing[1], 1);
    transform_axis(&mut values, dimensions, spacing[2], 2);
    values
}

fn transform_axis(values: &mut [f32], dimensions: [u32; 3], spacing_mm: f32, axis: usize) {
    let line_len = dimensions[axis] as usize;
    if line_len == 0 {
        return;
    }
    let mut line = vec![0.0; line_len];
    let mut transformed = vec![0.0; line_len];
    let other_axes = match axis {
        0 => [1, 2],
        1 => [0, 2],
        2 => [0, 1],
        _ => unreachable!("axis is always one of the three voxel axes"),
    };

    for second in 0..dimensions[other_axes[1]] {
        for first in 0..dimensions[other_axes[0]] {
            for coordinate in 0..dimensions[axis] {
                let mut index = [0; 3];
                index[axis] = coordinate;
                index[other_axes[0]] = first;
                index[other_axes[1]] = second;
                line[coordinate as usize] = values[linear_index(index, dimensions)];
            }
            distance_transform_1d(&line, spacing_mm, &mut transformed);
            for coordinate in 0..dimensions[axis] {
                let mut index = [0; 3];
                index[axis] = coordinate;
                index[other_axes[0]] = first;
                index[other_axes[1]] = second;
                values[linear_index(index, dimensions)] = transformed[coordinate as usize];
            }
        }
    }
}

fn distance_transform_1d(input: &[f32], spacing_mm: f32, output: &mut [f32]) {
    let Some(first_seed) = input.iter().position(|value| *value < DISTANCE_INFINITY) else {
        output.fill(DISTANCE_INFINITY);
        return;
    };
    let len = input.len();
    let mut sites = vec![0usize; len];
    let mut boundaries = vec![0.0f32; len + 1];
    let mut count = 0usize;
    sites[0] = first_seed;
    boundaries[0] = f32::NEG_INFINITY;
    boundaries[1] = f32::INFINITY;

    for query in first_seed + 1..len {
        if input[query] >= DISTANCE_INFINITY {
            continue;
        }
        let mut boundary = parabola_intersection(input, spacing_mm, query, sites[count]);
        while boundary <= boundaries[count] {
            count -= 1;
            boundary = parabola_intersection(input, spacing_mm, query, sites[count]);
        }
        count += 1;
        sites[count] = query;
        boundaries[count] = boundary;
        boundaries[count + 1] = f32::INFINITY;
    }

    let mut site = 0usize;
    for (coordinate, value) in output.iter_mut().enumerate() {
        while boundaries[site + 1] < coordinate as f32 * spacing_mm {
            site += 1;
        }
        let delta = (coordinate as f32 - sites[site] as f32) * spacing_mm;
        *value = input[sites[site]] + delta * delta;
    }
}

fn parabola_intersection(input: &[f32], spacing_mm: f32, query: usize, site: usize) -> f32 {
    let query_position = query as f32 * spacing_mm;
    let site_position = site as f32 * spacing_mm;
    ((input[query] + query_position * query_position)
        - (input[site] + site_position * site_position))
        / (2.0 * (query_position - site_position))
}

fn voxel_count(dimensions: [u32; 3]) -> usize {
    dimensions[0] as usize * dimensions[1] as usize * dimensions[2] as usize
}

fn linear_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::roi::VoxelGeometry;

    fn voxel_data(dimensions: [u32; 3], spacing: [f32; 3], raw_data: Vec<u8>) -> VoxelData {
        VoxelData {
            geometry: VoxelGeometry {
                dimensions,
                spacing,
                origin: [0.0; 3],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            raw_data,
        }
    }

    #[test]
    fn test_signed_distance_uses_world_mm_spacing() {
        let field =
            signed_distance_from_voxel_data(&voxel_data([3, 1, 1], [2.0, 1.0, 1.0], vec![1, 0, 0]))
                .unwrap();

        assert_eq!(field, vec![-2.0, 2.0, 4.0]);
    }

    #[test]
    fn test_signed_distance_is_negative_inside_and_positive_outside() {
        let field = signed_distance_from_voxel_data(&voxel_data(
            [3, 3, 1],
            [1.0, 3.0, 1.0],
            vec![0, 0, 0, 0, 1, 0, 0, 0, 0],
        ))
        .unwrap();

        assert_eq!(field[4], -1.0);
        assert_eq!(field[3], 1.0);
        assert_eq!(field[1], 3.0);
    }

    #[test]
    fn test_signed_distance_rejects_invalid_raw_data_length() {
        let error =
            signed_distance_from_voxel_data(&voxel_data([2, 2, 2], [1.0; 3], vec![1])).unwrap_err();

        assert_eq!(
            error,
            SignedDistanceError::InvalidRawDataLength {
                expected: 8,
                actual: 1,
            }
        );
    }
}
