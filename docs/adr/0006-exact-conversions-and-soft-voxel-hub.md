# Exact Conversions and the Soft Voxel Hub

Status: Proposed (2026-10-01). Not yet implemented. Follows ADR 0001 (one authoritative form per ROI), ADR 0004 (automatic switching), and `docs/spatial-model.md`.

## Context

Every conversion between a ROI's forms goes through a binary voxel mask on the ROI's own grid (the image resolution). That loses what is finer than a voxel at every step:

- contours to voxels samples voxel centres, so a loop edge moves by up to half a voxel;
- voxels to mesh smooths a quantised mask, so the surface only approximates the drawn shape;
- mesh to contours voxelizes and then extracts voxel-face loops, a staircase, even though the mesh could simply be cut with the layer planes.

Owner decisions (2026-10-01): a drawn contour is the truth and is never snapped to the voxel grid; conversions should be as exact and as fast as possible; the viewer is real time; ROI algebra (margins, union, intersection, subtraction) will be wanted later.

Reference: RayStation (`git show b1361d0:src/convert/segmentaiton_rep.md`) keeps a primary shape per ROI, cuts meshes with planes for contours, and turns contours into a **soft-valued voxel ROI on a fine reconstruction grid** by shape-based interpolation (signed distance per slice, linear interpolation between slices, "hats" half-way to the next slice); mesh and volume come from that voxel ROI. The older vector-authoritative plan in this repository's history baked a signed distance field straight from the loops. This ADR combines the two.

## Decision

### 1. Mesh to contours is a direct cut

Switching a mesh ROI to contours cuts the mesh with the reference-grid layer planes of the target family. No voxels are involved. A vertex exactly on a plane counts as above it (a symbolic nudge), so every cut triangle gives one segment and the segments of a closed surface link into closed loops without a tolerance; crossing points are shared through the mesh edge they lie on. Where shells touch along an edge, any pairing of the segments is valid because the even-odd fill is the same.

The cut contours are the true cross-sections: they are not a voxel staircase and not "lossless against a mask". The switch report says how the new form was made (cut from a surface, extracted from a mask, sampled from loops) instead of a lossless flag.

After the switch the voxel form is rebuilt from the contours in the background, like any derived form, and is not drawn until it is current.

### 2. The soft voxel hub

Voxels stay the conversion hub, but the hub becomes a **soft-valued voxel ROI on a refined grid**:

- **Values** are coverage, 0 to 255 (the fraction of the voxel inside the shape). The surface is the 50% level (a ROI is empty when every value is below 127.5, as in RayStation). A label imported as a binary mask is stored as 255 where it is present; its colour lives in the ROI's metadata, not in the voxel value.
- **Grid** is a refinement of the ROI's snug box: each voxel of the reference grid is split by an integer factor per axis, so fine voxels stay aligned with the world and with the image layers (`VoxelGeometry::refined`, alongside `cropped`). Default spacing is half the smallest image spacing, not below 0.5 mm; it is a setting, not a constant.
- **Built from the loops** by an exact signed distance to the contour polylines (not a rasterize, fill and distance-transform detour): in each slice the distance to the nearest segment with the sign from the even-odd inside test, linear interpolation of the signed distance between slices, hats half-way to the neighbouring slice (or half a layer when there is none), converted to coverage. Slices need not be on adjacent layers or on image layers.
- **Used for** everything that needs a mask: the fill overlay (coverage becomes the alpha, which anti-aliases the fill), volume (the sum of coverage times the voxel volume instead of a binary count), mesh generation, export, and later ROI algebra, which operates on the coverage directly.
- **On demand and snug.** It is built only for ROIs that need it, only inside the snug box, and dropped under memory pressure; it is a cache, never authoritative for contour or mesh ROIs.

### 3. The mesh comes from the hub

A contour ROI's 3D mesh is the 50% iso-surface of the hub (marching cubes on coverage with linear interpolation along edges), sub-voxel accurate and aligned with the loops. Other-family contour views of a contour ROI come from cutting that mesh (decision 1's cutter), not from rasterized voxels. A mesh ROI's own mesh is its authority and is untouched.

### 4. Real time

The targets below are what the implementation is judged by, on the liver sample, native release builds (the browser is expected to be 1.5 to 2 times slower). They go into `tests/switch_guard.rs` as budgets with generous margins, like the existing ones.

| Operation | Target |
| --- | --- |
| Mesh to contours switch | at most 20 ms (one pass over the triangles, bucketed by layer) |
| One contour slice edit (commit) to updated hub region and mesh chunks | at most one frame (16 ms) of main-thread work, the rest time-sliced over following frames |
| Contour drag preview | at most 8 ms per frame |
| Full hub of a large ROI, first build | progressive: a coarse pass at image resolution first, refinement in the background in chunk-sized steps, never a single stall |
| Mesh rebuild after an edit | local chunks only (already about 7 ms on the liver) |

How:

- **Local updates.** An edit changes the field only near the edited slice (its neighbours in the interpolation, and a narrow band of distance around the old and new loops). The hub region and the mesh chunks that it reaches are rebuilt, the rest is reused, the same locality argument as the incremental mesh rebuild. Away from the loops the value is just inside or outside, set by scanline parity, not by a distance search.
- **Spatial lookup** of loop segments (a grid per slice) so a sample looks at nearby segments only.
- **Progressive refinement.** During a drag and right after a commit the mesh is built from a coarse field (image resolution) so the 3D view follows immediately; a finer pass replaces it when idle. Time-sliced steps use the existing frame budget (4 ms) so the browser stays responsive, and native builds may use threads where it helps.
- **No hidden full-grid work:** every step runs on the snug box.

### 5. Verification

Before the hub is built, a test passes smooth shapes (a sphere of 5.3 voxels radius, a thin plate, a shape with a hole and a branch) through every chain and reports the overlap and the surface distance against the original. Acceptance for the new chains: a contours to hub to contours round trip stays within half a hub voxel, as RayStation claims for theirs; mesh to contours to fill is exact (the surface passes midway between voxel centres); the performance targets above hold.

## Staging

Each step is a commit series that leaves the lifecycle tests, the guards and the QA spec green.

1. The measurement test (decision 5), run on the current chains for the baseline.
2. Mesh to contours by direct cut (decision 1), with the exactness test and the guard budget.
3. `VoxelGeometry::refined`, coverage-valued voxel data, and the label import to coverage; the overlay shader reads coverage as alpha.
4. The hub from loops with local updates and progressive refinement (decision 2 and 4), then the mesh from the hub and the other-family views from the cut mesh (decision 3).
5. Volume from coverage; ROI algebra is a separate later item on top of the hub.

## Consequences

- Sub-voxel detail survives every conversion between contours and meshes; staircases only appear for imported binary labels.
- The voxel value stops meaning "label id", which touches the overlay shader, label import, and the voxel tests (round-trip checks become overlap and distance checks outside the exact mesh-to-contours chain).
- More derived forms depend on the mesh revision (other-family views of a contour ROI), so meshes are built for contour ROIs with visible slice views even when no 3D view is open, under the "nothing stale" rule.
- Fine hubs cost memory (a liver box at 0.5 mm is about 11 MB), which is why they are on demand and snug.

## Open questions

1. Default hub resolution: half the smallest image spacing with a 0.5 mm floor, or a fixed 0.5 mm.
2. Whether the coarse preview field is enough for drags on thin structures, or the preview needs the fine hub locally.
3. Whether volume statistics should read the hub or integrate the mesh directly (exact by the divergence theorem); both can be offered.
