# Phase 7: Integration & Polish

## Goal

Wire all components together into a working end-to-end system.

## Files to Modify

### [MODIFY] `src/app/components.rs`
### [MODIFY] `src/gui.rs`
### [MODIFY] `src/lib.rs`
### [MODIFY] `src/render/mod.rs`
### [NEW] `src/systems/segment_system.rs`

## Component Registration

Add `Segment` to ECS in `src/app/components.rs`:

```rust
use crate::app::segment::Segment;

// Add to entity creation or as standalone component
pub struct Segmentation {
    pub segments: Vec<Segment>,
    pub active_segment_idx: Option<usize>,
}

impl Default for Segmentation {
    fn default() -> Self {
        Self {
            segments: Vec::new(),
            active_segment_idx: None,
        }
    }
}
```

## Segment System

### `src/systems/segment_system.rs`

```rust
use crate::app::segment::{Segment, ContourSet};
use crate::convert::contour_to_sdf::build_sdf_from_contours;
use crate::convert::marching_cubes::marching_cubes;

/// Update derived representations when contours change
pub fn sys_update_segment_caches(
    segment: &mut Segment,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) {
    // Rebuild SDF if contours changed
    if segment.sdf_dirty && !segment.contours.is_empty() {
        segment.sdf = Some(build_sdf_from_contours(
            &segment.contours,
            volume_dims,
            volume_spacing,
            segment.sdf_resolution_multiplier,
        ));
        segment.sdf_dirty = false;
        segment.mesh_dirty = true;  // SDF changed, mesh needs rebuild
    }
    
    // Rebuild mesh if SDF changed
    if segment.mesh_dirty {
        if let Some(ref sdf) = segment.sdf {
            segment.mesh = Some(marching_cubes(sdf, 0.0));
            segment.mesh_dirty = false;
        }
    }
}

/// Get contours visible on current slice
pub fn get_visible_contours(
    segment: &Segment,
    slice_plane: SlicePlane,
    slice_index: i32,
) -> Vec<&PlaneContour> {
    segment.contours
        .contours_at_slice(slice_plane, slice_index)
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}
```

## GUI Integration

Add to `src/gui.rs`:

```rust
/// Segment management panel
pub fn segment_panel(ui: &mut egui::Ui, segmentation: &mut Segmentation) {
    ui.heading("Segments");
    
    // Create new segment button
    if ui.button("➕ New Segment").clicked() {
        let colors = [
            [0.8, 0.2, 0.2, 0.7],  // Red
            [0.2, 0.8, 0.2, 0.7],  // Green
            [0.2, 0.2, 0.8, 0.7],  // Blue
            [0.8, 0.8, 0.2, 0.7],  // Yellow
        ];
        let idx = segmentation.segments.len();
        let color = colors[idx % colors.len()];
        let segment = Segment::new(&format!("Segment {}", idx + 1), color);
        segmentation.segments.push(segment);
        segmentation.active_segment_idx = Some(idx);
    }
    
    ui.separator();
    
    // List segments
    for (i, segment) in segmentation.segments.iter_mut().enumerate() {
        let is_active = segmentation.active_segment_idx == Some(i);
        
        ui.horizontal(|ui| {
            // Color swatch
            let color = egui::Color32::from_rgba_unmultiplied(
                (segment.color[0] * 255.0) as u8,
                (segment.color[1] * 255.0) as u8,
                (segment.color[2] * 255.0) as u8,
                (segment.color[3] * 255.0) as u8,
            );
            let rect = ui.allocate_space(egui::vec2(16.0, 16.0));
            ui.painter().rect_filled(rect.1, 2.0, color);
            
            // Visibility toggle
            ui.checkbox(&mut segment.visible, "");
            
            // Name (selectable)
            let response = ui.selectable_label(is_active, &segment.name);
            if response.clicked() {
                segmentation.active_segment_idx = Some(i);
            }
            
            // Delete button
            if ui.small_button("🗑").clicked() {
                // Mark for deletion (handle outside loop)
            }
        });
    }
}

/// Tool selection
pub fn tool_panel(ui: &mut egui::Ui, current_tool: &mut Tool) {
    ui.heading("Tools");
    
    ui.horizontal(|ui| {
        if ui.selectable_label(*current_tool == Tool::Navigate, "🖐 Navigate").clicked() {
            *current_tool = Tool::Navigate;
        }
        if ui.selectable_label(*current_tool == Tool::ContourDraw, "✏️ Contour").clicked() {
            *current_tool = Tool::ContourDraw;
        }
    });
}
```

## Render Loop Integration

In `src/lib.rs` render loop:

```rust
// After volume rendering, render segment mesh (in 3D mode)
if view_state.view_mode == ViewMode::View3D {
    for segment in &segmentation.segments {
        if segment.visible {
            if let Some(ref mesh) = segment.mesh {
                if segment.mesh_resources.is_none() {
                    // Create GPU resources
                    segment.mesh_resources = Some(MeshResources::from_mesh(
                        &device, mesh, &mesh_pipeline
                    ));
                }
                
                if let Some(ref resources) = segment.mesh_resources {
                    // Update uniforms with current camera
                    let uniforms = compute_mesh_uniforms(
                        &view_state,
                        segment.color,
                        viewport_size,
                    );
                    resources.update_uniforms(&queue, &uniforms);
                    
                    render_mesh(&mut encoder, &view, &depth_view, &mesh_pipeline, resources);
                }
            }
        }
    }
}

// In 2D mode, render contour outlines
if view_state.view_mode != ViewMode::View3D {
    for segment in &segmentation.segments {
        if segment.visible {
            let contours = get_visible_contours(
                segment,
                view_state.slice_plane,
                view_state.cursor_voxel[view_state.slice_plane.depth_axis()],
            );
            
            if !contours.is_empty() {
                contour_resources.update(&device, &queue, &contours, segment.color);
                // Contour rendering happens in main shader pass
            }
        }
    }
}
```

## Event Flow

```
User draws contour
    ↓
InputState::contour_draw_state = Drawing { points }
    ↓
Mouse released
    ↓
screen_points_to_plane_contour()
    ↓
segment.contours.add_contour()
    ↓
segment.sdf_dirty = true
    ↓
sys_update_segment_caches() [next frame]
    ↓
SDF rebuilt, mesh rebuilt
    ↓
GPU resources updated
    ↓
Rendered
```

## Verification

### End-to-End Test

1. Run: `cargo run`
2. Load a volume
3. Create a new segment (click "New Segment")
4. Select Contour tool
5. Draw closed contours on 3-4 axial slices
6. Switch to 3D view
7. **Verify:** Colored mesh appears matching contours
8. Rotate view
9. **Verify:** Mesh has proper shading
10. Switch back to 2D view
11. **Verify:** Contour outlines visible on slices

### Multi-View Edit Test

1. Draw contour on Axial view
2. Switch to Coronal view
3. Draw another contour
4. Switch to 3D view
5. **Verify:** Mesh reflects both contours (blended)

### Acceptance Criteria

- [ ] Can create multiple segments
- [ ] Active segment is highlighted in GUI
- [ ] Contour drawing adds to active segment
- [ ] SDF and mesh regenerate automatically
- [ ] 2D view shows contour outlines
- [ ] 3D view shows lit mesh
- [ ] Multi-axis editing works

## Summary

After completing all 7 phases, you will have:

1. **Data Model** — `Segment` with contours, SDF, mesh
2. **Drawing** — Freehand contour input
3. **2D Display** — SDF line rendering of contours
4. **SDF Conversion** — Multi-axis, oblique-aware
5. **Mesh Generation** — Marching Cubes with normals
6. **3D Display** — Lit mesh rendering
7. **Integration** — Full workflow, GUI, events
