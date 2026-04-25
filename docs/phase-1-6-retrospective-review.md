# Phase 1-6 Retrospective Review

Date: 2026-04-25

Scope:
- `Subplan 1: ROI Core Model`
- `Subplan 2: Conversion and Job Runtime`
- `Subplan 3: Voxel Baseline Integration`
- `Pre-Subplan 4 Course Corrections`
- `Subplan 4: Transform, Plane, and Geometry Context`
- `Subplan 5: Contour Representation Architecture`
- `Subplan 6: Contour Editing V1`

This review was done after Subplan 6 was completed, because the Subplan 6 review exposed plan and implementation drift that may also exist in earlier phases.

## Review Summary

Overall state:
- The ROI core model, runtime scaffold, voxel baseline, geometry transform layer, and contour-authoritative data model are present and tested.
- The Phase 1 ROI core model is aligned with the plan and no Phase 1-specific fixup is required.
- The implementation is ready to continue after the focused fixup pass tracked below.
- Most issues are boundary-contract issues rather than broken compilation or missing core types.

Validation during review:
- `cargo test -q`: pass
- `cargo check --target wasm32-unknown-unknown -q`: pass
- `cargo clippy --all-targets --all-features -- -D warnings`: pass
- `git diff --check 45eb4cf..HEAD`: pass

Commit mapping:
- Phase 1: `415d329` (`Phase 1: introduce ROI core model`)
- Phase 2: `cc87fa8` (`Phase 2: add ROI conversion runtime contract`)
- Phase 3: `e1853ff` (`Phase 3: wire voxel ROI baseline through runtime`)
- Pre-Subplan 4 docs: `d2b97f9`, `f5795a8`
- Pre-Subplan 4 implementation: `ad37e27`
- Subplan 4: `f2c64e7`, `f6f446c`, `8bb8f8e`, `45eb4cf`
- Subplan 5: implemented in `8d353db` together with Subplan 6A-6C work
- Subplan 6: `8d353db`, `1764ee2`, `de35bab`, `a4031eb`, `3b632e4`, `87be697`

## Findings

### F1. ROI overlay slot accounting mixes voxel overlays and contour ROIs

Severity: Medium

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplan 3
- Pre-Subplan 4
- Subplan 6, by consequence

Evidence:
- `visible_roi_count(...)` counts every visible `Roi`, regardless of whether it has a renderable voxel cache.
- `can_enable_roi_visibility(...)` uses that count for the current two-overlay renderer cap.
- `sys_prepare_render_data(...)` builds overlay flags and opacities from every visible `Roi`.
- `recreate_scene_bind_groups(...)` only binds ROIs with `renderable_voxel_cache()`.

Risk:
- Visible contour ROIs can consume voxel overlay slots even though contours use a separate native contour pass.
- Overlay flags/opacities can diverge from the actual texture views bound to the shader.
- Loading or enabling voxel labels after creating contour ROIs can behave incorrectly or confusingly.

Recommended fixup:
- Introduce a runtime helper that returns the ordered list of renderable voxel overlay ROIs.
- Use the same helper for visibility cap decisions, bind-group texture selection, and render uniform overlay flags/opacities.
- Keep contour visibility separate from voxel texture overlay slot accounting.

Resolution:
- Added a renderable voxel overlay view helper used by bind-group rebuild and render prep.
- Contour ROI visibility no longer consumes voxel overlay texture slots.
- Voxel overlay flags/opacities now follow the same ordered overlay list used for texture binding.

Suggested tests:
- visible contour ROIs do not count against the two voxel texture overlay slots
- overlay opacities align with the same ROIs used to build overlay texture bind groups
- loading a voxel label after creating visible contour ROIs still auto-shows when voxel overlay slots are available

### F2. Geometry-mismatched labels are preserved but may still be visually misleading

Severity: Medium

Status: Fixed in retrospective fixup pass.

Affected phases:
- Pre-Subplan 4
- Subplan 3
- future rendering/registration work

Evidence:
- `prepare_voxel_roi_import(...)` preserves label geometry and logs a warning when it differs from the main volume.
- The current renderer still samples label textures in the viewer's volume UV space.
- Registration and resampling are explicitly out of scope.

Risk:
- A label with different dimensions, spacing, origin, or orientation can be represented correctly in ROI data but still appear visually overlaid as if it were aligned to the main image.
- This is acceptable as a deferred limitation only if the UI/runtime makes it explicit enough that users do not treat the overlay as spatially registered.

Recommended fixup:
- Preserve label-owned geometry and allow visibility even when the label grid differs from the main image.
- Keep mismatch logging for diagnostics, but do not present valid non-identical label grids as a load warning.
- Add a plan note that registration/resampling belongs to explicit alignment/conversion workflows, not basic label visibility.

Resolution:
- Label/main geometry mismatch remains diagnostic-only.
- Mismatched label ROIs preserve their own geometry and can start visible when overlay capacity allows.
- The main plan now records that differing label grids are valid inputs.

Suggested tests:
- label/main geometry mismatch is detected without blocking visibility when overlay capacity allows
- mismatched labels keep ROI-owned geometry
- mismatched labels do not silently become visible if the chosen policy is to hide them by default

### F3. Subplan 4 oblique support is partial outside picking and contour paths

Severity: Medium

Status: Fixed for annotation/render projection in retrospective fixup pass.

Affected phases:
- Subplan 4
- Subplan 6, by dependency

Evidence:
- Oblique picking was routed through `PlaneDefinition` and shared viewport mapping.
- `render::geometry::world_to_ndc(...)` still uses `SlicePlane::from_mode(...)` for 2D projection, which returns `None` for oblique.
- Annotation dragging/projection in `gui/overlays.rs` also depends on `SlicePlane::from_mode(...)` for editable 2D views.

Risk:
- The plan language can be read as if render projection parity covers oblique views broadly, but current support is not universal.
- Contour code has its own oblique-compatible path, but annotation/render projection remains orthogonal-first.

Recommended fixup:
- Either implement oblique `world_to_ndc(...)` and annotation dragging through `PlaneDefinition`, or explicitly document oblique annotation/render projection as deferred.
- Add tests that make this boundary unambiguous.

Resolution:
- Oblique `world_to_ndc(...)` now projects through `PlaneDefinition` and shared viewport mapping.
- Annotation dragging in oblique view now maps through the oblique plane path.
- Added an oblique projection regression test.

Suggested tests:
- oblique `world_to_ndc(...)` projects a point on the oblique plane through shared mapping
- oblique annotation dragging is rejected/documented or implemented through shared geometry
- existing axial/coronal/sagittal projection tests remain unchanged

### F4. Subplan 5 implementation is committed inside a Subplan 6 commit

Severity: Low

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplan 5
- plan tracking discipline

Evidence:
- `docs/subplan-5-contour-handoff.md` asks for step-level status tracking and commit hashes.
- `docs/segmentation-reimplementation-plan.md` marks Subplan 5 steps complete but does not tie them to a dedicated Subplan 5 commit.
- The code appears to have landed in `8d353db` (`Phase 6A-6C: add contour editing scaffolding and native renderer`).

Risk:
- Future implementers may think Subplan 5 was skipped or may have trouble auditing the model/runtime contract separately from editing/rendering work.

Recommended fixup:
- Update the implementation status to state explicitly that Subplan 5 landed in `8d353db`.
- Optionally add a short "reviewed complete" note in this retrospective rather than rewriting history.

Resolution:
- Main plan status now explicitly records that Subplan 5 implementation landed in `8d353db`.

Suggested tests:
- no code tests needed; this is documentation hygiene.

### F5. Subplan 2 runtime is a scaffold, not a real conversion scheduler

Severity: Low

Status: Documented; no code change required.

Affected phases:
- Subplan 2
- Subplan 10 future performance/cache work

Evidence:
- Runtime supports cache status, dirty/current checks, queued/running job state, begin, and completion.
- There is no real frame-budgeted execution, progress reporting, cancellation, worker scheduling, or generation-aware stale-job discard yet.
- The main plan mostly describes this honestly as a minimal scaffold.

Risk:
- The phrase "complete Subplan 2" can still be misread as "conversion runtime is done."

Recommended fixup:
- Keep the current code, but make future handoffs call it a "minimal runtime contract" or "scheduler scaffold."
- Ensure Subplan 10 owns real progress/cancel/frame-budget behavior.

Suggested tests:
- no immediate code tests needed unless scheduler behavior is expanded.

### F6. Earlier phases do not have the same manual verification discipline as Subplan 6

Severity: Low

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplans 1-5
- future handoffs

Evidence:
- Subplan 6 handoff explicitly required manual verification details.
- Earlier phases mostly track compile/test verification and implementation status.

Risk:
- Viewer behavior claims such as "preserve current viewer behavior" are harder to audit later.

Recommended fixup:
- Add a lightweight review log entry to the main plan after this retrospective.
- For future subplans, require manual verification when behavior is visible in the viewer, even if the subplan is mostly architecture.

Resolution:
- Main plan now links this retrospective review and records the fixup checkpoint.

Suggested tests:
- no code tests needed.

### F7. Contour point drag preview mutates authoritative data outside the runtime replacement path

Severity: Medium

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplan 6
- Subplan 5 mutation contract, by dependency

Evidence:
- `move_selected_point_preview(...)` routes into `move_selected_point_internal(..., queue_rebuild = false)`.
- The internal move function mutates `RoiAuthoritativeData::Contour(...)` directly.
- Rebuild queueing is deferred until mouse release through `finalize_selected_point_move(...)`.
- Subplan 5 and Subplan 6 both state that committed contour edits should route through `replace_contour_data(...)`.

Risk:
- The direct mutation path was added for drag responsiveness and is understandable, but it weakens the authoritative mutation contract.
- If mouse release/finalization is interrupted, authoritative contour data may be changed without a queued voxel rebuild.
- Future undo/redo, audit, or generation-aware cache logic will be harder if preview and commit semantics remain mixed.

Recommended fixup:
- Make drag preview explicit editor/session state, then apply one committed `replace_contour_data(...)` on release.
- If keeping direct preview mutation for now, document it as a temporary exception and add a test that release/finalization queues the rebuild after preview mutation.

Resolution:
- Contour drag preview now lives in editor preview state.
- Authoritative contour data is committed through `replace_contour_data(...)` on release.
- Added a regression test for preview-before-commit and release queueing.

Suggested tests:
- preview move changes visible contour position without queuing rebuild
- release after preview queues `RebuildVoxelCache`
- interrupted/cancelled drag either reverts preview state or still leaves runtime dirty/queued consistently

### F8. Contour edit tool naming and state are stale after select/move merge

Severity: Low

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplan 6

Evidence:
- Toolbar exposes `Contour Edit` as `EditorTool::ContourSelect`.
- Point movement is now part of the contour edit/select tool.

Risk:
- The current user-facing behavior is fine, but the enum and plan wording can mislead future implementers into reintroducing a separate move tool.

Recommended fixup:
- Remove `ContourPointMove`.
- Update the plan wording to say point move is part of `Contour Edit` / `ContourSelect`.

Resolution:
- Removed `EditorTool::ContourPointMove`.
- Updated Subplan 6 wording to treat point dragging as part of contour edit/select mode.

Suggested tests:
- editor tool default remains `Navigation`
- switching to `Contour Edit` preserves point selection and allows point drag
- switching to `Contour Draw` clears selection/draft according to existing rules

### F9. Subplan 6 closure documentation is close but not exact enough

Severity: Low

Status: Fixed in retrospective fixup pass.

Affected phases:
- Subplan 6
- plan tracking discipline

Evidence:
- Step 6H lists implementation commits through `3b632e4` but omits the closure commit `87be697`.
- The manual verification notes use broad "observed pass" statements rather than exact manual actions, expected results, observed results, pass/fail, and gaps.

Risk:
- The plan is usable, but not as strong as the handoff requested.
- A lower-tier implementer or reviewer may not know exactly what was manually verified.

Recommended fixup:
- Add `87be697` to the Subplan 6 closure entry.
- Expand the manual verification note into a concise checklist with explicit actions and outcomes.

Resolution:
- Main plan now includes `87be697` in the Subplan 6 closure entry.
- Manual verification notes were expanded into explicit action/expected/observed entries.

Suggested tests:
- no code tests needed.

## Phase Notes

Subplan 1:
- Core ROI types and active ROI migration are present.
- Aligned with the plan; no Phase 1-specific fixup required.
- Render prep and bind-group construction now share the voxel overlay view model from F1.

Subplan 2:
- Cache/job scaffolding exists and is tested.
- No immediate code fix needed.
- Future scheduler expectations should remain in later phases.

Subplan 3:
- Label loading and voxel ROI stats are routed through ROI runtime state.
- Overlay rendering now works through a shared renderable voxel overlay view model.

Pre-Subplan 4:
- ROI-owned voxel geometry was added and label geometry is preserved.
- Non-identical label grids remain valid and can start visible when overlay capacity allows.
- The overlay cap now applies to renderable voxel texture slots, not every visible ROI type.

Subplan 4:
- Shared coordinate, plane, viewport, and world conversion helpers exist and have good unit coverage.
- Oblique picking moved into the shared geometry path.
- Oblique annotation/render projection now uses the shared oblique plane path.

Subplan 5:
- Contour data model and mutation/runtime contracts are present.
- No conversion algorithms were added, which matches scope.
- Subplan 5 commit traceability is documented in the main plan.

Subplan 6:
- Contour editing V1 is present: create contour ROI, native contour rendering, draw/close/select/move/insert/delete.
- The native contour renderer is separate from egui drawing, which matches the agreed direction.
- Drag preview now commits through the runtime mutation contract on release.
- Tool naming and closure docs were cleaned up.

## Proposed Fixup Pass

Recommended order:
1. Fixed voxel overlay slot accounting and render-prep/bind-group ordering (F1).
2. Added geometry mismatch user-facing policy for loaded labels (F2).
3. Implemented oblique annotation/render projection through shared geometry (F3).
4. Fixed the contour drag preview mutation path (F7).
5. Updated plan status for Subplan 5 and Subplan 6 traceability (F4 and F9).
6. Cleaned up contour edit tool naming/state after the select/move merge (F8).
7. Added this retrospective-review checkpoint to `docs/segmentation-reimplementation-plan.md`.

Mesh architecture work can proceed after the fixup commit is validated.
