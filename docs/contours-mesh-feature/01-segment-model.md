# 01 - Segment Data Model

Implements the core `Segment` struct from [00-polymorph-architecture](00-polymorph-architecture.md).

## Problem

Current `Segmentation` + `LabelmapData` components only support voxel representation.

## Solution

```rust
// src/segment.rs

pub struct Segment {
    pub name: String,
    pub color: [f32; 4],
    
    /// Which representation is the source of truth
    pub source: SourceKind,
    
    /// Lazy-loaded representations
    pub labelmap: Option<LabelmapData>,
    pub contours: Option<ContourData>,
    pub mesh: Option<MeshData>,
    
    /// Dirty flags
    pub labelmap_dirty: bool,
    pub contours_dirty: bool,
    pub mesh_dirty: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SourceKind {
    Labelmap,
    Contours,
    Mesh,
}

pub struct ContourData {
    /// Per-slice contours: (axis, slice_idx) → polylines
    pub slices: HashMap<(u8, u32), Vec<ContourPolyline>>,
}

pub struct ContourPolyline {
    pub label_id: u8,
    pub points: Vec<[f32; 2]>,  // UV coords (0-1)
    pub closed: bool,
}

pub struct MeshData {
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}
```

## Subtasks

- [ ] Create `src/segment.rs` with structs above
- [ ] Add `get_contours()` / `get_mesh()` lazy accessors
- [ ] Add `invalidate_derived()` helper
- [ ] Migrate existing `Segmentation` to new model
- [ ] Update GUI to show active source indicator

## Files

| File | Change |
|------|--------|
| `src/segment.rs` | NEW |
| `src/components.rs` | Remove old `Segmentation`, add `Segment` |
| `src/lib.rs` | Add `mod segment;` |
