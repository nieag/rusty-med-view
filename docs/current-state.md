# Current Repository State

This repository contains a medical volume viewer with multi-representation ROI support. Active code supports:

- NIfTI volume loading
- Labelmap loading as overlay layers
- Orthogonal and 3D volume viewing
- Crosshair picking, pan, zoom, and rotation
- Windowing controls
- Annotation and note-taking UI
- voxel-, contour-, and mesh-primary ROIs
- ROI-owned voxel geometry and geometry-aware overlay sampling
- voxel-to-contour extraction and contour-to-voxel rebuilding
- contour editing in orthogonal and active oblique planes, including exact displayed-view promotion
- voxel/contour-to-mesh creation and native WGPU contour/mesh rendering

## Segmentation Status

ROIs use one authoritative primary representation. Other representations are derived session caches with explicit generation, dirty, rebuild, and blocker state.

Voxel/contour workflows and the mesh-primary correctness roundtrip are implemented. Mesh Edit selects a projected mesh vertex in 3D and applies a radius/strength brush during pointer drag. The mesh and direct intersections in visible 2D planes preview without changing authority; pointer release commits once and queues exact voxel/contour convergence. Contour point drags produce changed-slice voxel previews, cross-plane contours, and preview meshes without changing authority. Preview meshing reuses unchanged voxel chunks, rebuilds only chunks intersecting the merged old/new contour bounds, and resumes under a four-millisecond frame budget. Mesh rendering retains GPU buffers by ROI/view/chunk identity and uploads only changed projected chunks. Oblique image and geometry-aware voxel sampling now consume a shared CPU-derived plane basis, and requested oblique voxel contours build as current view caches. A current displayed oblique contour view can be promoted exactly and edited; committed rasterization preserves all authoritative oblique planes. Oblique commits currently use a full-contour voxel rebuild, so large-volume 60 FPS and preview-latency gates remain unmeasured.

Voxel, contour, and mesh authority can be switched on the same ROI whenever the target cache is current. Promotion preserves the ROI entity, identity, metadata, layer settings, and compatible caches; it advances authority generation once and activates the corresponding edit tool. Voxel-to-contour conversion uses the active 2D plane family, and contour plane-family switches rebuild the full requested family from the current voxel representation. The older duplicate-ROI extraction helpers remain available as runtime APIs but are no longer the primary sidebar editing workflow.

## Implementation Status

State: bidirectional ROI editing core implemented; acceptance closeout in progress

Architecture cleanup has started: authoritative ROI shape types now live under `src/app/roi/model.rs`, and primary representation is derived from authoritative data instead of stored redundantly.

Completed:
- ROI core model and ROI-owned spatial metadata
- conversion/job runtime scaffold
- voxel/contour roundtrip and contour view-cache promotion
- geometry-aware voxel overlays
- native WGPU contour and mesh rendering
- representation request/QA state contracts
- mesh-to-voxel-to-contour committed roundtrip
- direct mesh-plane 2D previews with mesh preview commit/cancel
- priority/dependency-aware conversion queue and bounded view caches
- dirty-AABB chunked contour-preview meshing with full-rebuild equivalence tests
- frame-budgeted resumable preview mesh extraction with stale-preview cancellation
- retained per-chunk WGPU mesh buffers with upload/reuse QA counters
- projected 3D mesh vertex selection and radius/strength brush deformation
- non-authoritative per-drag mesh previews with one commit on pointer release
- native WGPU selected-vertex handle and direct visible-plane intersection updates
- tool, ROI-switch, commit, and cancel lifecycle cleanup for pending previews
- same-ROI voxel-to-contour authority promotion for axial, coronal, and sagittal editing
- same-ROI current-mesh-cache promotion with stale/missing-cache rejection
- explicit lossy-conversion UI that activates the matching edit tool
- reversible voxel/contour/mesh authority controls with stale-cache gating
- explicit Edit Points, Add Loop, and Deform Mesh tool naming
- additive contour correction with merged planar boundaries, hole preservation, and inside-outside-inside auto-commit
- world-space reprojection between display-plane and authoritative contour-plane frames for addition, movement, insertion, and dirty-slice updates
- bounded authoritative contour/mesh undo and redo with preview-frame exclusion and normal derived-cache rebuilds
- slice-local contour undo/redo preserving incremental voxel-slab and affected mesh-chunk rebuild scope
- WASM-safe ROI history shortcut polling without nested egui context locking
- single-pass compositing for eight geometry-aware voxel ROIs with deterministic active-first ordering
- simultaneous native WGPU rendering of visible contour ROIs with active-only editing affordances
- shared CPU/GPU oblique plane basis for aligned image and geometry-aware voxel sampling
- requested oblique voxel-to-contour view caches with exact displayed-view authority promotion
- oblique contour rendering independent of empty orthogonal cache ordering
- oblique contour-to-voxel rebuild preserving all authoritative planes
- five-view ROI MPR protocol exposing axial, coronal, sagittal, rotatable oblique, and 3D viewports together
- viewport-local aspect ratios shared by rendering and picking in mixed-size protocols

Pending:
- measured 60 FPS and preview-latency acceptance on representative volumes
- performance/cache closeout work
- incremental oblique contour rasterization or measured acceptance of the full-contour fallback
- manual oblique image/voxel/contour alignment acceptance

Plan Document:
- See `docs/segmentation-reimplementation-plan.md` for the canonical roadmap and implementation status.
- See `docs/roi-authoring-runtime-cleanup-plan.md` for the post-closeout runtime modularization and segmentation-tool transition plan.
