# ROI Workstation TODOs

Ordered backlog for the next direction. Complete top-to-bottom unless a measured regression changes the order.

## Now: foundation

- [ ] Fix the sidebar overlay-cap message: derive it from the shared renderer cap of eight.
- [ ] Add regression coverage: eight renderable voxel overlays are accepted; the ninth is explicitly rejected/truncated.
- [ ] Write a short manual baseline script: load image, import label, create contour ROI, commit contour, inspect 2D/3D, verify current caches.
- [ ] Decide annotation scope: migrate viewport annotation markers/labels and hit testing to native WGPU/input, or mark annotations provisional and remove them from stable-product claims.

## Next: make the current ROI loop understandable

- [ ] Add one persistent active-ROI status area: name, authority, active tool, lock state, and derived-work state.
- [ ] Show concise processing, ready, and actionable failure messages after ROI edits and authority changes.
- [ ] Ensure tool activation, loop completion, cancel, and undo/redo have visible, consistent feedback.
- [ ] Verify the full axial and oblique contour loop manually after the UI changes.

## Then: ROI catalog

- [ ] Turn the layer list into an ROI catalog with reliable select, visibility, opacity, and basic facts.
- [ ] Add rename.
- [ ] Add lock/unlock with an explicit blocked-edit message.
- [ ] Make the eight-overlay rendering limit visible in the catalog without hiding non-rendered ROIs.

## Then: authoring polish

- [ ] Improve contour draw/edit affordances: hover target, active-tool indication, close-loop/cancel guidance, and shortcut hints.
- [ ] Keep the existing contour, voxel, and mesh algorithms unchanged.
- [ ] Exercise axial, coronal, sagittal, and oblique contour edits with undo/redo.

## Then: useful output

- [ ] Show selected ROI facts from its current voxel cache: voxel count and volume in mm³.
- [ ] Export one selected ROI as a NIfTI labelmap using its owned reference geometry.
- [ ] Refuse stale-cache export with a clear rebuild/wait message.
- [ ] Reload the exported labelmap and verify geometry and occupied-voxel bounds.

## Later, only with evidence

- [ ] Profile representative large/multi-ROI workloads before optimizing further.
- [ ] Consider GPU compute only if a measured conversion path misses its interaction budget.
- [ ] Consider session persistence, registration/resampling, DICOM, or collaboration only after the author-to-export loop is useful.

## Not tasks now

- ECS rewrite.
- Generic job framework.
- Coordinate-system rewrite.
- New contour algorithms. Meshing work is tracked separately in
  `docs/mesh-authority-and-meshing-plan.md`.
- GPU compute migration without a measured bottleneck.
