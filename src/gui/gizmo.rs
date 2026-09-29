//! 3D Orientation Gizmo using glam for proper quaternion math

use egui::{Color32, Pos2, Rect, Ui};
use glam::{Quat, Vec3};

/// Draws an orientation gizmo in the given UI rect.
///
/// `view_rotation`: The rotation quaternion representing the volume's orientation.
/// This should be the INVERSE of the camera's view rotation.
///
/// `axis_letters`: anatomical letters of the positive and negative direction of each voxel index
/// axis (from the grid's affine), so a reflected or permuted volume is labelled truthfully.
pub fn draw_gizmo(ui: &mut Ui, rect: Rect, view_rotation: Quat, axis_letters: [[char; 2]; 3]) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let axis_length = radius * 0.7;

    // Colours identify the voxel index axes (i red, j green, k blue); the letters say which
    // anatomical direction each one currently points toward.
    let axes = [
        (
            Vec3::X,
            axis_letters[0][0],
            Color32::from_rgb(255, 100, 100),
        ),
        (
            Vec3::NEG_X,
            axis_letters[0][1],
            Color32::from_rgb(180, 60, 60),
        ),
        (
            Vec3::Y,
            axis_letters[1][0],
            Color32::from_rgb(100, 255, 100),
        ),
        (
            Vec3::NEG_Y,
            axis_letters[1][1],
            Color32::from_rgb(60, 180, 60),
        ),
        (
            Vec3::Z,
            axis_letters[2][0],
            Color32::from_rgb(100, 150, 255),
        ),
        (
            Vec3::NEG_Z,
            axis_letters[2][1],
            Color32::from_rgb(60, 100, 180),
        ),
    ];

    // Transform and project axes
    struct TransformedAxis {
        pos2: Pos2,
        label: char,
        color: Color32,
        depth: f32,
    }

    let mut transformed: Vec<TransformedAxis> = axes
        .iter()
        .map(|(vec, label, color)| {
            // Use centralized projection
            let proj = crate::util::orientation::project_axis_3d(
                vec.to_array(),
                view_rotation.to_array(),
                [1.0, 1.0, 1.0], // Gizmo axes are unit length
                1.0,             // Internal gizmo aspect is 1:1
            );

            let screen_x = center.x + proj[0] * axis_length;
            let screen_y = center.y + proj[1] * axis_length;

            // Get depth for Z-sorting (forward direction is +Z in projection)
            let q = view_rotation.to_array();
            let m = crate::util::orientation::quat_to_mat3(q);
            let rotated = crate::util::orientation::rotate_vec3(m, vec.to_array());

            TransformedAxis {
                pos2: Pos2::new(screen_x, screen_y),
                label: *label,
                color: *color,
                depth: rotated[2],
            }
        })
        .collect();

    // Sort by depth (painters algorithm) - back axes draw first
    transformed.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let painter = ui.painter();

    // Draw background circle
    painter.circle_filled(
        center,
        radius,
        Color32::from_rgba_unmultiplied(20, 20, 30, 180),
    );

    for axis in &transformed {
        let is_front = axis.depth < 0.0;

        // Fade out back-facing axes
        let alpha = if is_front { 1.0 } else { 0.4 };
        let color = Color32::from_rgba_unmultiplied(
            (axis.color.r() as f32 * alpha) as u8,
            (axis.color.g() as f32 * alpha) as u8,
            (axis.color.b() as f32 * alpha) as u8,
            if is_front { 255 } else { 150 },
        );

        // Draw line from center to axis endpoint
        painter.line_segment(
            [center, axis.pos2],
            egui::Stroke::new(if is_front { 2.5 } else { 1.5 }, color),
        );

        // Draw label bubble
        let bubble_radius = if is_front { 10.0 } else { 7.0 };
        painter.circle_filled(axis.pos2, bubble_radius, color);

        // Draw label text
        painter.text(
            axis.pos2,
            egui::Align2::CENTER_CENTER,
            axis.label.to_string(),
            egui::FontId::proportional(if is_front { 11.0 } else { 9.0 }),
            Color32::WHITE,
        );
    }

    // Draw center dot
    painter.circle_filled(center, 4.0, Color32::WHITE);
}

/// Convert a [f32; 4] quaternion [x, y, z, w] to glam::Quat
pub fn quat_from_array(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(q[0], q[1], q[2], q[3])
}
