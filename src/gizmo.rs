//! 3D Orientation Gizmo using glam for proper quaternion math

use egui::{Color32, Pos2, Rect, Ui};
use glam::{Quat, Vec3};

/// Draws an orientation gizmo in the given UI rect.
///
/// `view_rotation`: The rotation quaternion representing the volume's orientation.
/// This should be the INVERSE of the camera's view rotation.
pub fn draw_gizmo(ui: &mut Ui, rect: Rect, view_rotation: Quat) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let axis_length = radius * 0.7;

    // Define axes with medical standard labels
    // X = Right/Left, Y = Anterior/Posterior, Z = Superior/Inferior
    let axes = [
        (Vec3::X, "R", Color32::from_rgb(255, 100, 100)), // Right (Red)
        (Vec3::NEG_X, "L", Color32::from_rgb(180, 60, 60)), // Left
        (Vec3::Y, "A", Color32::from_rgb(100, 255, 100)), // Anterior (Green)
        (Vec3::NEG_Y, "P", Color32::from_rgb(60, 180, 60)), // Posterior
        (Vec3::Z, "S", Color32::from_rgb(100, 150, 255)), // Superior (Blue)
        (Vec3::NEG_Z, "I", Color32::from_rgb(60, 100, 180)), // Inferior
    ];

    // Transform and project axes
    struct TransformedAxis<'a> {
        pos2: Pos2,
        label: &'a str,
        color: Color32,
        depth: f32,
    }

    let mut transformed: Vec<TransformedAxis> = axes
        .iter()
        .map(|(vec, label, color)| {
            // Apply rotation - direct multiplication for Object->World
            let q = view_rotation.to_array();
            let m = crate::orientation::quat_to_mat3(q);
            let rotated = crate::orientation::rotate_vec3(m, vec.to_array());

            // Project to 2D (orthographic)
            // Flip Y because screen Y is down, 3D Y is up
            // RADIOLOGICAL: Rotated.X (Right) should map to Left on screen.
            // AND we now have a vertical flip in the main 3D view too.
            let screen_x = center.x - rotated[0] * axis_length;
            let screen_y = center.y - rotated[1] * axis_length; // Match main 3D view's fix

            TransformedAxis {
                pos2: Pos2::new(screen_x, screen_y),
                label,
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
            axis.label,
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
