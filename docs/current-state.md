# Current Repository State

This repository currently contains a medical volume viewer with label overlay support. The active codebase supports:

- NIfTI volume loading
- Labelmap loading as overlay layers
- Orthogonal and 3D volume viewing
- Crosshair picking, pan, zoom, and rotation
- Windowing controls
- Annotation and note-taking UI

## Segmentation Status

The older contour/SDF/mesh segmentation implementation has been intentionally removed from the active codebase.

That redesign work has not been reintroduced yet. Any historical references to contour extraction, TSDF/SDF pipelines, Surface Nets, or contour editing should be treated as stale unless they appear in a new implementation plan added after this cleanup.

## Implementation Status

State: clean-slate baseline

Completed:
- Removed stale documentation that described the deleted segmentation pipeline
- Updated repository guidance to match the current viewer/overlay application
- Kept the current app buildable on native and WASM targets

Pending:
- Define the new segmentation direction in a fresh design/plan document
- Reintroduce segmentation code only behind a coherent staged plan
