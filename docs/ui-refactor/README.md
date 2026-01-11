# UI Refactor: Professional Medical Imaging Interface

This document describes the planned refactor of the application's UI to match professional medical imaging viewers like **VolView**.

## Reference Design

The target interface features:
- **Top toolbar** with data loading and tool controls
- **2×2 viewport grid** with clear visual separation
- **Rich viewport overlays** showing slice info, windowing, and anatomical orientation
- **Collapsible metadata sidebar** for volume info, layers, and annotations

---

## Current State

The existing UI (`gui.rs`, ~729 lines) uses a left sidebar containing:
- Data loading buttons (Volume, Labelmap)
- Windowing controls (sliders + presets)
- Toolbox (Nav/Brush/Eraser radio buttons, brush size)
- Layers list with visibility/opacity
- Annotations list with locate/delete
- Controls help text

**Current viewport overlays** only show:
- Viewport label ("3D View", "Axial", etc.)
- Current slice position as a float

---

## Proposed Architecture

### 1. Top Toolbar Panel

A new `egui::TopBottomPanel::top()` containing:

```
┌────────────────────────────────────────────────────────────────────────┐
│ 🩺 Medical Viewer  │  📂 Volume  📂 Label  │  ◉ Nav  ○ Brush  ○ Erase  │
└────────────────────────────────────────────────────────────────────────┘
```

**Components:**
| Element | Description |
|---------|-------------|
| App Title | Left-aligned branding text |
| Load Buttons | "📂 Volume" and "📂 Label" buttons trigger file dialogs |
| Tool Selection | Radio button group for Navigation/Brush/Eraser |
| Windowing Presets | Quick buttons: Soft Tissue, Lung, Bone, Brain |

**Code location:** New section at start of `gui.rs::prepare()` before the sidebar.

---

### 2. Viewport Separation

Add visible borders/separators between the 4 viewports using egui's painting API.

**Implementation options:**
1. **Painter lines**: Draw lines at viewport midpoints using `ui.painter().line_segment()`
2. **Frame borders**: Wrap each viewport area in `egui::Frame` with stroke

**Visual result:**
```
┌─────────────────┬─────────────────┐
│                 │                 │
│     3D View     │   Axial View    │
│                 │                 │
├─────────────────┼─────────────────┤
│                 │                 │
│  Coronal View   │  Sagittal View  │
│                 │                 │
└─────────────────┴─────────────────┘
```

**Border style:** 1-2px line, dark gray (`Color32::from_gray(60)`)

---

### 3. Enhanced Viewport Overlays

Each viewport will display rich information overlays:

#### 3.1 Top-Left Corner Info Block

For 2D viewports (Axial, Coronal, Sagittal):
```
Axial (Top)
Slice: 148/295
W/L: 410.00 / 70.00
```

For 3D viewport:
```
3D View
W/L: 410.00 / 70.00
```

**Data sources:**
- `Slice: current/total` — Computed from `cursor_pos` and `VolumeData.dimensions`
- `W/L` — From `VolumeWindowing.center` / `VolumeWindowing.width`

#### 3.2 Anatomical Orientation Markers

Single-letter markers at the edges of each 2D viewport showing anatomical directions:

| Viewport | Top | Bottom | Left | Right |
|----------|-----|--------|------|-------|
| Axial    | A   | P      | R    | L     |
| Coronal  | S   | I      | R    | L     |
| Sagittal | S   | I      | A    | P     |

**Rendering:** Use `egui::Area` positioned at viewport edge centers with bold text.

#### 3.3 Optional Bottom-Right Info Icon

Small "ⓘ" icon in bottom-right corner for additional metadata on hover.

---

### 4. Restructured Sidebar

Convert the sidebar from tool-centric to metadata-centric:

```
┌─────────────────────────┐
│  ▼ Volume Info          │
│    Dimensions: 512×512×295
│    Spacing: 0.5×0.5×2.0 mm
│    Range: -1024 to 3071 HU
├─────────────────────────┤
│  ▼ Windowing            │
│    Center: [====●====]  │
│    Width:  [====●====]  │
│    Presets: [ST][Lg][Bn]│
├─────────────────────────┤
│  ▼ Layers               │
│    ☑ Layer 1  [===●===] │
│    ☐ Layer 2  [===●===] │
│    [+ Create New]       │
├─────────────────────────┤
│  ▼ Annotations          │
│    🎯 "Tumor" [🗑]      │
│    🎯 "Marker" [🗑]     │
│    [+ Add Annotation]   │
├─────────────────────────┤
│  ▼ Controls             │
│    LMB: Pick/Paint      │
│    MMB: Pan             │
│    Scroll: Zoom/Slice   │
└─────────────────────────┘
```

**Sections:**
1. **Volume Info** — Read-only display of loaded volume metadata
2. **Windowing** — Detailed sliders + preset buttons (moved from current location)
3. **Layers** — Existing layer logic (visibility, opacity, selection)
4. **Annotations** — Existing annotation list logic
5. **Controls** — Keyboard/mouse reference

---

## File Changes

### [gui.rs](file:///Users/nieage/dev/git/rust_starter_app/src/gui.rs)

| Section | Change |
|---------|--------|
| Lines 85-370 | Restructure sidebar from tools → metadata focus |
| Lines 60-83 | Add `TopBottomPanel::top()` for toolbar |
| Lines 412-452 | Enhance overlay Areas with slice counts and W/L |
| New | Add anatomical orientation marker Areas at viewport edges |
| New | Draw viewport separator lines in central area |

### [components.rs](file:///Users/nieage/dev/git/rust_starter_app/src/components.rs)

| Change |
|--------|
| No structural changes needed — slice counts computable from existing `VolumeData.dimensions` |

---

## Slice Number Calculation

For each viewport, compute current and total slices:

```rust
// Given cursor position [x, y, z] in normalized [0,1] space
// and volume dimensions [dim_x, dim_y, dim_z]

// Axial (varies in Z)
let current_z = (cursor_pos[2] * dim_z as f32).round() as u32;
let total_z = dim_z;  // Display as "Slice: {current_z}/{total_z}"

// Coronal (varies in Y)  
let current_y = (cursor_pos[1] * dim_y as f32).round() as u32;
let total_y = dim_y;

// Sagittal (varies in X)
let current_x = (cursor_pos[0] * dim_x as f32).round() as u32;
let total_x = dim_x;
```

---

## Verification Plan

### Automated Tests
```bash
cargo test
```
All 18 existing tests should pass — changes are UI-only.

### Manual Verification Checklist

1. **Top Toolbar**
   - [ ] App title visible at left
   - [ ] Load Volume button opens file dialog
   - [ ] Load Label button opens file dialog
   - [ ] Tool radio buttons switch active tool
   
2. **Viewport Separation**
   - [ ] Visible lines between all 4 viewports
   - [ ] No visual artifacts at intersections

3. **Viewport Overlays**
   - [ ] Each 2D viewport shows "Slice: X/Y" format
   - [ ] All viewports show "W/L: center / width"
   - [ ] Orientation markers (S/I/A/P/L/R) visible at edges
   - [ ] Overlays update in real-time during interaction

4. **Sidebar**
   - [ ] Volume Info section shows correct dimensions
   - [ ] Windowing sliders work correctly
   - [ ] Layer controls function as before
   - [ ] Annotation controls function as before
   - [ ] Sidebar collapses/expands properly

---

## Implementation Order

1. **Phase 1**: Top toolbar (minimal disruption)
2. **Phase 2**: Viewport separators (visual only)
3. **Phase 3**: Enhanced overlays (slice counts, W/L, orientations)
4. **Phase 4**: Sidebar restructure (reorganize existing code)
5. **Phase 5**: Polish and verification
