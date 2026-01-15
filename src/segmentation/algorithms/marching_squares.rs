use crate::segmentation::contour::Contour;
use glam::Vec2;
use uuid::Uuid;

/// Implementation of Marching Squares for generating contours from a 2D scalar field.
/// In this context, the scalar field is a slice of either a Labelmap or a TSDF.

pub struct MarchingSquares {
    threshold: f32,
}

impl MarchingSquares {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    /// Extracts contours from a 2D grid of values.
    /// values: Flattened 2D array of grid_dims.0 * grid_dims.1
    /// grid_dims: (width, height)
    /// label_index: The label we are extracting for (used for labeling the resulting contour)
    pub fn extract(&self, values: &[f32], grid_dims: (u32, u32), label_index: u8) -> Vec<Contour> {
        let (w, h) = grid_dims;
        if w < 2 || h < 2 {
            return vec![];
        }

        let mut segments = Vec::new();

        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let idx0 = (y * w + x) as usize;
                let idx1 = (y * w + (x + 1)) as usize;
                let idx2 = ((y + 1) * w + (x + 1)) as usize;
                let idx3 = ((y + 1) * w + x) as usize;

                let v0 = values[idx0];
                let v1 = values[idx1];
                let v2 = values[idx2];
                let v3 = values[idx3];

                let mut case = 0;
                if v0 >= self.threshold {
                    case |= 1;
                }
                if v1 >= self.threshold {
                    case |= 2;
                }
                if v2 >= self.threshold {
                    case |= 4;
                }
                if v3 >= self.threshold {
                    case |= 8;
                }

                if case == 0 || case == 15 {
                    continue;
                }

                self.add_segments(case, x, y, (v0, v1, v2, v3), w, h, &mut segments);
            }
        }

        self.reconstruct_contours(segments, label_index)
    }

    fn add_segments(
        &self,
        case: u8,
        x: u32,
        y: u32,
        v: (f32, f32, f32, f32),
        w: u32,
        h: u32,
        segments: &mut Vec<(Vec2, Vec2)>,
    ) {
        let (v0, v1, v2, v3) = v;
        let fx = x as f32;
        let fy = y as f32;
        let fw = w as f32;
        let fh = h as f32;

        // Normalized coordinates (UV)
        let get_p = |p: (f32, f32)| Vec2::new(p.0 / (fw - 1.0), p.1 / (fh - 1.0));

        let top = get_p((fx + self.lerp(v0, v1), fy));
        let bottom = get_p((fx + self.lerp(v3, v2), fy + 1.0));
        let left = get_p((fx, fy + self.lerp(v0, v3)));
        let right = get_p((fx + 1.0, fy + self.lerp(v1, v2)));

        match case {
            1 | 14 => segments.push((left, top)),
            2 | 13 => segments.push((top, right)),
            4 | 11 => segments.push((right, bottom)),
            8 | 7 => segments.push((bottom, left)),
            3 | 12 => segments.push((left, right)),
            6 | 9 => segments.push((top, bottom)),
            5 => {
                segments.push((left, bottom));
                segments.push((top, right));
            }
            10 => {
                segments.push((left, top));
                segments.push((right, bottom));
            }
            _ => {}
        }
    }

    fn lerp(&self, a: f32, b: f32) -> f32 {
        if (b - a).abs() < 1e-6 {
            return 0.5;
        }
        (self.threshold - a) / (b - a)
    }

    fn reconstruct_contours(
        &self,
        mut segments: Vec<(Vec2, Vec2)>,
        label_index: u8,
    ) -> Vec<Contour> {
        let mut contours = Vec::new();

        while !segments.is_empty() {
            let mut points = Vec::new();
            let (start, end) = segments.remove(0);
            points.push(start);
            points.push(end);

            let mut changed = true;
            while changed {
                changed = false;
                let mut i = 0;
                while i < segments.len() {
                    let (p0, p1) = segments[i];
                    if (p0 - *points.last().unwrap()).length() < 1e-6 {
                        points.push(p1);
                        segments.remove(i);
                        changed = true;
                    } else if (p1 - *points.last().unwrap()).length() < 1e-6 {
                        points.push(p0);
                        segments.remove(i);
                        changed = true;
                    } else if (p0 - *points.first().unwrap()).length() < 1e-6 {
                        points.insert(0, p1);
                        segments.remove(i);
                        changed = true;
                    } else if (p1 - *points.first().unwrap()).length() < 1e-6 {
                        points.insert(0, p0);
                        segments.remove(i);
                        changed = true;
                    } else {
                        i += 1;
                    }
                }
            }

            let is_closed = (*points.first().unwrap() - *points.last().unwrap()).length() < 1e-6;
            if is_closed && points.len() > 1 {
                points.pop(); // Remove duplicate point
            }

            if points.len() > 1 {
                contours.push(Contour {
                    points,
                    is_closed,
                    segment_id: Uuid::new_v4(),
                    label_index,
                });
            }
        }

        contours
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marching_squares_simple_square() {
        let ms = MarchingSquares::new(0.5);
        // 4x4 grid with a 2x2 square in the middle
        let values = vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        let contours = ms.extract(&values, (4, 4), 1);

        assert_eq!(contours.len(), 1);
        assert!(
            contours[0].is_closed,
            "Contour should be closed for centered object"
        );
        // For a 2x2 square in a 4x4 grid, we expect 4 segments -> 4 points
        assert_eq!(contours[0].points.len(), 8);
    }

    #[test]
    fn test_marching_squares_empty() {
        let ms = MarchingSquares::new(0.5);
        let values = vec![0.0; 9];
        let contours = ms.extract(&values, (3, 3), 1);
        assert_eq!(contours.len(), 0);
    }

    #[test]
    fn test_marching_squares_full() {
        let ms = MarchingSquares::new(0.5);
        let values = vec![1.0; 9];
        let contours = ms.extract(&values, (3, 3), 1);
        assert_eq!(contours.len(), 0); // No boundaries in a full grid
    }
}
