# Spatial Model

How the viewer places everything in space: the image volume, the ROI forms (contours, meshes, voxels), the grids their voxels use, slice planes, the views, and the annotations. Read this before touching any code that converts between spaces. The decisions behind it are in ADR 0003 (ROI-owned spatial metadata) and ADR 0004 (ROI bodies); the rendering rules are in `rendering-architecture.md`.

## 1. Spaces and what lives in them

Every position is expressed in one of these spaces. Everything that crosses between them goes through `src/convert/` (and `src/util/orientation.rs` for the 3D view), never through ad hoc math in GUI or render code.

| Space | Unit | Used for | Defined by |
| --- | --- | --- | --- |
| **World** | millimetres | the common space: mesh vertices, annotation anchors, plane origins, every conversion between grids | the affine of the image (NIfTI sform or qform), world axes as in NIfTI (x right, y anterior, z superior) |
| **Voxel index** (IJK) | voxels, continuous | positions inside one grid | `VoxelGeometry`: dimensions plus an IJK-to-world affine |
| **Volume UV** | 0 to 1 per axis | the cursor, texture sampling, the overlay shader | `uv = (index + 0.5) / dimension` |
| **Plane-local** | millimetres, 2D | contour points | a `PlaneDefinition` (origin, u axis, v axis, normal, all in world mm) |
| **Viewport UV** | 0 to 1 on screen | picking, drawing, markers | the viewport mapping (zoom, pan, pivot, aspect) |
| **NDC / pixels** | | the last step to the GPU | the viewport rectangle |

Rules that hold everywhere:

- **Voxel centres are at integer indices.** The affine maps index `(0,0,0)` to the world position of the *centre* of the first voxel, which is the NIfTI, DICOM and ITK convention (not the corner). A grid spans `[-0.5, dimension - 0.5]` in index space and `dimension * spacing` millimetres. Normalized volume UV is cell-centred and edge-to-edge: UV 0 and 1 are the outer voxel faces and voxel `k` is centred at `(k + 0.5) / dimension`, which is also the GPU texture convention (`convert/coord_mapping.rs`, mirrored in the shader).
- **The affine is the truth.** `spacing()`, `origin()` and `orientation()` are derived views for display code; registration and measurement use the affine. Reflections (LAS-stored volumes) and shear are kept. A singular or non-finite transform is rejected at import and never becomes identity.
- **World millimetres are the only meeting point.** The image, every ROI grid, contours and meshes connect through world space and nothing else. Texture coordinates of a label are never assumed to equal those of the image.

## 2. The image volume

One main volume is loaded. Its grid defines the **display grid**: the layers of the slice views, the cursor, the crosshair, and the field of view. The cursor is a position in the main volume's UV.

Views are **index-space views**, not resampled to anatomical axes. An axial view shows the grid's `k` layers, with the grid's `i` axis across and `j` axis up, in radiological style (screen-left is the larger index; see `convert::screen_uv_to_volume_uv_for_family`). For a volume stored in another orientation the view still shows the grid's layers; the edge letters (R, L, A, P, S, I) come from the affine (`orientation::index_axis_letters`), so they are always true to the data, and `anatomical_plane_name` reports which anatomical plane a view really shows.

## 3. ROIs: one shape, three forms

A ROI is one structure (a label, a drawn contour, an edited mesh). It is a shape in world space, and it can be held in three forms. **Any of them can be the authoritative one**; the other two are derived from it and rebuilt when it changes (ADR 0001 and ADR 0004).

| Form | Coordinates | Natural for | Becomes authoritative when |
| --- | --- | --- | --- |
| **Contours** | loops of points in plane-local mm, on planes in world mm (section 4) | drawing and editing slice by slice | you draw or edit in a slice view |
| **Mesh** | vertices in world mm | 3D editing, smooth surfaces | you deform it in the 3D view |
| **Voxels** | a mask on a grid | import, display of fill, measurement, and as the common currency between the other two | a labelmap is imported (then it is a read-only source) |

Switching authority is automatic and one undo step: the first gesture that needs a different form converts the ROI (`ensure_editable`). Contours and meshes are first-class; imported voxels are the only form that cannot be edited directly, and the first edit converts them.

Only the voxel form needs a grid. Contours and meshes live in world millimetres and need none.

### The voxel form and the reference lattice

Every ROI still carries a **reference grid**, a lattice (spacing, orientation, origin, stored as a `VoxelGeometry`, never changed), because voxels are how the forms are converted into each other and how fill is displayed. It defines how a ROI's voxels line up in the world.

- An imported label's reference grid is the label file's own grid. It is usually the image's grid but need not be.
- A ROI drawn from scratch uses the image's grid.
- The reference grid is a *definition*, not storage. **No ROI holds voxels over all of it.**

A ROI's voxels are a **snug box** of its lattice: the same spacing and orientation, shifted by a whole number of voxels, smaller in extent (`VoxelGeometry::cropped`, `offset_in`, `VoxelData::embedded_in`). The box is chosen by `convert/snug_grid.rs`:

- an imported label: the bounding box of that label at import (`app/roi/label_import.rs`);
- a contour ROI: the box around its loops and slice depths when its contours are rasterized;
- a mesh ROI: the box around its vertices when it is voxelized.

A box has a one-voxel margin and only grows while the ROI is edited, so the retained voxel cache and the mesh chunks stay usable. Mesh chunk sets record the grid they came from and are reused only on the same grid (`ChunkedMeshData::grid`). The synthetic 150-label case (512 x 512 x 300) holds 132 MB this way against 35 GB with full-size copies.

Voxels are a **conversion hub**, not the shape: contour to voxels to mesh, mesh to voxels to contours. They are derived on demand and kept only as long as something needs them (for example the overlay fill, or the next conversion). Every derived cache records the geometry identity it was built on and is rejected if it does not match (`GeometryIdentity`, `CacheInstallError::GeometryMismatch`).

## 4. Contours and slice planes

A **contour slice** is a `PlaneDefinition` plus closed loops of points in **plane-local millimetres**. The plane is an origin, a `u` axis, a `v` axis and a normal (their cross product), all in world millimetres. A contour is therefore just points in an arbitrary plane in space; that its planes usually line up with image layers is because that is where they are drawn.

**Orthogonal planes** (`PlaneFamily::{Axial, Coronal, Sagittal}`, typed as `OrthogonalFamily` for authoritative contours) are built from a grid and a cursor by `orthogonal_plane_from_volume_uv`: the axes are the grid's axis directions, the origin is the cursor's world position. **Oblique planes** come from the view rotation (`oblique_plane_from_view_rotation`) in the volume's physical, millimetre frame (`VoxelGeometry::axis_frame`). Oblique planes only ever exist as derived, per-view slices; they are never authoritative.

**When two planes are the same slice** (`convert::planes_are_same_slice`): same family, parallel normals (|cos| of at least 0.999), and less than half an image layer apart along the normal, measured in millimetres. The layer thickness is the display grid's spacing along that axis, so the rule follows the voxel size and has no seam at layer boundaries. Oblique planes match within half the smallest spacing. When a contour cut on a finer grid than the image puts several slices into one image layer, `nearest_matching_slice` picks the closest: the view shows it and edits go to it. The other slices of that layer are neither shown nor edited from that view (a known limit).

**Drawing** works on the plane the view shows. A click is mapped viewport UV to plane-local mm (`viewport_uv_to_plane_local_mm`); a loop joins an existing slice if the planes match (its points are re-projected onto that slice's plane through world space), otherwise it starts a new slice at the displayed plane.

**Extraction from voxels** (`extract_contours_in_grid`) cuts one slice per layer of the voxel box, at that layer's centre, with the plane taken from the ROI's reference grid (so a slice has one plane whether it came from a box or from the whole grid). **Rasterization** (`rasterize_contours_to_voxel_data`) maps each slice to its voxel layer, tests voxel centres against the loops by even-odd, and writes into the snug box.

**Derived contour views.** A voxel or mesh ROI shows contours in a view by cutting the displayed plane through its voxels (`extract_contour_slice_from_voxel_data`) or its mesh (`intersect_mesh_with_plane`), in world space. These are cached per view key (`ContourViewKey`: family, plane, quantised slice key) and drawn only when built from the ROI's current revision (section 7).

## 5. Meshes

Mesh vertices are stored in **world millimetres**. A mesh from voxels (`extract_mesh_from_voxel_data`) is a smoothed signed-distance surface built on the voxel box, emitted directly in world space, split into chunks by voxel index (`DEFAULT_MESH_CHUNK_SIZE`). A mesh is voxelized by scanline parity in voxel-index space (`IncrementalMeshVoxelization`), into the snug box of the mesh. The 3D view draws the mesh from GPU-resident world-space parts with a per-draw affine.

## 6. The views

**Slice views (2D).** One mapping serves rendering, picking, drawing and the markers (`convert::viewport_uv_to_volume_uv` and its inverse, with `ViewportMapping { zoom, pan, pivot, screen_aspect }`):

```
screen_uv.x = (viewport_uv.x - pivot.x) * k / zoom + pivot.x + pan.x      k = screen_aspect / slice_aspect
screen_uv.y = (viewport_uv.y - pivot.y)     / zoom + pivot.y + pan.y
```

`slice_aspect` is the plane's width over height in millimetres (`plane_display_aspect`), so pixels are isotropic on screen whatever the voxel shape. The shader implements the same arithmetic; a GPU test renders the main shader and compares the crosshair with this mapping for axial, coronal and sagittal at two zooms and pans (`render/pipeline/tests.rs`), and another pins the 3D scale constant to the shader.

**Oblique view.** The plane comes from the view rotation; the image is sampled along it from the volume (`oblique_*` uniforms), and its aspect is the plane's own extent in millimetres.

**3D view.** Orthographic, by ray marching a cached image (re-marched only when the camera, windowing, overlays or volume change). The composed rotation is `user_rotation * BASE_ROTATION * data_orientation` (`orientation::compose_view_rotation`), with a fixed half-height of `ORTHOGRAPHIC_VIEW_SCALE` normalized physical-volume units. Picking casts the same ray back (`screen_to_ray_3d`). The crosshair and meshes are drawn over the cached image.

**Voxel overlays.** The shader receives, per overlay slot, the ROI box's dimensions and an affine from main-volume index space to the ROI box's index space (`index_space_affine_from_src_to_dst`), samples nearest, and treats anything outside the box as empty. Up to eight overlays fill; contours and meshes show for every visible ROI.

## 7. Consistency rules

- **Nothing stale is drawn.** A derived form is drawn only if it was built from the ROI's current authoritative revision, or is the preview of the edit in progress (ADR 0004). Voxel overlays, derived meshes and derived contour views all pass through this gate.
- **One implementation of each mapping**, in `src/convert/` and `src/util/orientation.rs`, with tests; the shader is checked against it.
- **No coordinate math in GUI code.** Annotation markers and the orientation gizmo are drawn by egui from projections computed by the shared helpers; this is a documented exception to the egui boundary in `rendering-architecture.md` until annotations get a native renderer.

## 8. Annotations

An annotation anchor is a point in **world millimetres** (`Anchor`), so it keeps its place in every view and for every viewer. Each frame the anchor is converted to the main volume's UV and projected like any other point; a note shows in a slice view when its depth along the view axis is within a small tolerance (0.005 of the volume UV) of the cursor's slice, and in the 3D view always.

## 9. Where to look, and what pins it

| Topic | Code | Tests |
| --- | --- | --- |
| Geometry, boxes, embedding | `model/geometry.rs`, `convert/snug_grid.rs` | `model::geometry::tests`, `convert::snug_grid::tests` |
| Index and UV | `convert/coord_mapping.rs` | `convert::coord_mapping` tests |
| Planes, slice matching, viewport mapping | `convert/geometry.rs` | `convert::geometry::tests` |
| Shader against the mapping | `shaders/shader.wgsl`, `render/pipeline.rs` | `render::pipeline::tests` (GPU) |
| 3D view | `util/orientation.rs` | `util::orientation::tests` |
| Contours from and to voxels | `convert/voxel_contour_extract.rs`, `contour_raster.rs` | their `tests` modules, `tests/switch_guard.rs` |
| Meshes and voxelization | `convert/sdf_mesh_extract.rs`, `voxel_mesh_extract.rs`, `mesh_voxelize.rs` | their `tests` modules, `tests/switch_guard.rs` |
| Labels on another grid, cropping | `app/roi_runtime/create.rs`, `app/roi/label_import.rs` | `app::roi_runtime::tests`, `tests/scale_guard.rs` |

## 10. Known limits

- A ROI's voxel box never shrinks during editing, only grows.
- A label on a finer grid than the image can have several contour slices inside one image layer; only the nearest is shown and edited from a view.
- Views are index-space views; for an obliquely stored volume they do not resample to anatomical planes.
- The voxel fill is drawn for the first eight visible ROIs only.
