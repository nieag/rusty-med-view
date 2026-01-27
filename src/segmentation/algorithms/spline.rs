//! Catmull-Rom spline evaluation and tessellation.
//!
//! Used for smooth polygon tool - control points lie on the curve.

use glam::Vec2;

/// A Catmull-Rom spline defined by control points.
#[derive(Debug, Clone, Default)]
pub struct CatmullRomSpline {
    pub control_points: Vec<Vec2>,
    /// Tension parameter: 0.0 = uniform, 0.5 = centripetal (default), 1.0 = chordal
    pub tension: f32,
}

impl CatmullRomSpline {
    /// Create a new spline with default centripetal parameterization.
    pub fn new() -> Self {
        Self {
            control_points: Vec::new(),
            tension: 0.5,
        }
    }

    /// Add a control point.
    pub fn add_point(&mut self, p: Vec2) {
        self.control_points.push(p);
    }

    /// Remove the last control point.
    pub fn remove_last(&mut self) -> Option<Vec2> {
        self.control_points.pop()
    }

    /// Check if spline is closed (first and last points coincide).
    pub fn is_closed(&self) -> bool {
        if self.control_points.len() < 3 {
            return false;
        }
        let first = self.control_points.first().unwrap();
        let last = self.control_points.last().unwrap();
        first.distance(*last) < 1e-6
    }

    /// Evaluate a single segment of the Catmull-Rom spline.
    /// p0, p1, p2, p3 are four consecutive control points.
    /// t is the interpolation parameter [0, 1] between p1 and p2.
    pub fn evaluate_segment(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
        // Catmull-Rom basis matrix coefficients
        let t2 = t * t;
        let t3 = t2 * t;

        // Standard Catmull-Rom with tension = 0.5
        let c0 = -0.5 * t3 + t2 - 0.5 * t;
        let c1 = 1.5 * t3 - 2.5 * t2 + 1.0;
        let c2 = -1.5 * t3 + 2.0 * t2 + 0.5 * t;
        let c3 = 0.5 * t3 - 0.5 * t2;

        p0 * c0 + p1 * c1 + p2 * c2 + p3 * c3
    }

    /// Tessellate the spline into a polyline.
    /// Returns points along the spline at regular intervals.
    ///
    /// `segments_per_control`: Number of line segments between each pair of control points.
    pub fn tessellate(&self, segments_per_control: usize) -> Vec<Vec2> {
        let n = self.control_points.len();
        if n < 2 {
            return self.control_points.clone();
        }
        if n == 2 {
            // Just a line segment
            return self.control_points.clone();
        }

        let mut result = Vec::new();
        let segments = segments_per_control.max(1);

        // For a closed spline, we wrap around
        let closed = self.is_closed();
        let num_segments = if closed { n } else { n - 1 };

        for i in 0..num_segments {
            // Get four control points for this segment
            let p0 = self.get_wrapped_point(i as i32 - 1, closed);
            let p1 = self.control_points[i];
            let p2 = self.get_wrapped_point(i as i32 + 1, closed);
            let p3 = self.get_wrapped_point(i as i32 + 2, closed);

            // Generate points along this segment
            for j in 0..segments {
                let t = j as f32 / segments as f32;
                result.push(Self::evaluate_segment(p0, p1, p2, p3, t));
            }
        }

        // Add the final point (unless closed)
        if !closed && n > 2 {
            result.push(*self.control_points.last().unwrap());
        }

        result
    }

    /// Get a control point with wrapping for closed splines.
    fn get_wrapped_point(&self, idx: i32, closed: bool) -> Vec2 {
        let n = self.control_points.len() as i32;
        if closed {
            // Wrap around, but skip the duplicate last point
            let effective_n = n - 1; // Last point == first point for closed
            let wrapped = ((idx % effective_n) + effective_n) % effective_n;
            self.control_points[wrapped as usize]
        } else {
            // Clamp to valid range
            let clamped = idx.clamp(0, n - 1);
            self.control_points[clamped as usize]
        }
    }

    /// Clear all control points.
    pub fn clear(&mut self) {
        self.control_points.clear();
    }

    /// Number of control points.
    pub fn len(&self) -> usize {
        self.control_points.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.control_points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catmull_rom_linear() {
        // Control points on a line should produce points on the same line
        let mut spline = CatmullRomSpline::new();
        spline.add_point(Vec2::new(0.0, 0.0));
        spline.add_point(Vec2::new(1.0, 0.0));
        spline.add_point(Vec2::new(2.0, 0.0));
        spline.add_point(Vec2::new(3.0, 0.0));

        let tessellated = spline.tessellate(10);

        // All points should have y ≈ 0
        for p in &tessellated {
            assert!(p.y.abs() < 1e-5, "Point {:?} not on line", p);
        }
    }

    #[test]
    fn test_catmull_rom_closed() {
        // A square that closes on itself
        let mut spline = CatmullRomSpline::new();
        spline.add_point(Vec2::new(0.0, 0.0));
        spline.add_point(Vec2::new(1.0, 0.0));
        spline.add_point(Vec2::new(1.0, 1.0));
        spline.add_point(Vec2::new(0.0, 1.0));
        spline.add_point(Vec2::new(0.0, 0.0)); // Close the loop

        assert!(spline.is_closed());

        let tessellated = spline.tessellate(5);
        assert!(!tessellated.is_empty());
    }

    #[test]
    fn test_tessellation_count() {
        let mut spline = CatmullRomSpline::new();
        spline.add_point(Vec2::new(0.0, 0.0));
        spline.add_point(Vec2::new(1.0, 1.0));
        spline.add_point(Vec2::new(2.0, 0.0));
        spline.add_point(Vec2::new(3.0, 1.0));

        let segments_per = 10;
        let tessellated = spline.tessellate(segments_per);

        // For 4 points (3 segments), we expect: 3 * 10 + 1 = 31 points
        // (10 per segment, plus final endpoint)
        assert_eq!(tessellated.len(), 3 * segments_per + 1);
    }

    #[test]
    fn test_empty_spline() {
        let spline = CatmullRomSpline::new();
        assert!(spline.is_empty());
        assert!(!spline.is_closed());
        assert!(spline.tessellate(10).is_empty());
    }

    #[test]
    fn test_two_point_spline() {
        let mut spline = CatmullRomSpline::new();
        spline.add_point(Vec2::new(0.0, 0.0));
        spline.add_point(Vec2::new(1.0, 1.0));

        let tessellated = spline.tessellate(10);
        assert_eq!(tessellated.len(), 2);
    }
}
