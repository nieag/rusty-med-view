// src/overlay/mod.rs
pub mod primitives;

use glam::Vec3;
pub use primitives::OverlayPrimitive;

/// Marker for an annotation in the overlay
#[derive(Clone, Debug)]
pub struct AnnotationMarker {
    pub world_pos: Vec3,
}

/// Preview of a brush stroke
#[derive(Clone, Debug)]
pub struct BrushPreview {
    pub world_pos: Vec3,
    pub radius: f32,
    pub viewport_mask: u32,
}

/// Manager for all overlay primitives, providing a cleaner abstraction than a flat vector.
pub struct OverlayManager {
    /// Specialized collections
    pub annotations: Vec<AnnotationMarker>,
    pub brush_preview: Option<BrushPreview>,

    /// Cached GPU primitives (built from specialized collections)
    pub primitives: Vec<OverlayPrimitive>,

    /// Current mouse position in screen UV coordinates (per viewport)
    pub mouse_screen_uv: [f32; 2],

    /// Index of primitive currently being dragged, if any
    pub dragging_idx: Option<usize>,

    /// The viewport where dragging is occurring
    pub dragging_viewport: u32,
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            annotations: Vec::new(),
            brush_preview: None,
            primitives: Vec::new(),
            mouse_screen_uv: [0.5, 0.5],
            dragging_idx: None,
            dragging_viewport: 0,
        }
    }

    /// Rebuild the flat primitive list for the GPU
    pub fn rebuild_primitives(&mut self) {
        self.primitives.clear();

        // Add annotations
        for ann in &self.annotations {
            self.primitives.push(OverlayPrimitive::circle(
                ann.world_pos,
                0.015,                // Fixed radius for now
                [1.0, 1.0, 0.0, 1.0], // Yellow
                15,                   // All viewports
            ));
        }

        // Add brush preview
        if let Some(brush) = &self.brush_preview {
            self.primitives.push(OverlayPrimitive::ring(
                brush.world_pos,
                brush.radius,
                0.005,                // thickness
                [1.0, 1.0, 1.0, 0.8], // White translucent
                brush.viewport_mask,
            ));
        }
    }

    pub fn add_annotation(&mut self, pos: Vec3) {
        self.annotations.push(AnnotationMarker { world_pos: pos });
    }
}

impl Default for OverlayManager {
    fn default() -> Self {
        Self::new()
    }
}


