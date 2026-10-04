# Agent-first Architecture: What to Borrow and How

Status: Proposed (2026-10-04). Not yet accepted; the decisions it needs are at the end.

## Where this comes from

`storytold/photocraft` is a clean-room Photoshop rewrite in Rust (egui and wgpu, native and wasm). Its domain differs from ours, but its structure answers a question we have: how do the same actions run from the UI, from a test, and from an agent or script. We read its `AGENTS.md`, workspace, `xtask`, CI, engine command registry, `automation` crate, `control.rs`, `state.rs`, `actions.rs`, `enable_rules.rs` and `docs/control-protocol.md`.

Their core idea is one rule: **everything is a command**. A command has an id, params as JSON, an `enabled` check that returns a reason, and a `run` function on a session that needs no window. The egui UI, the CLI, a JSON control channel and an MCP server all dispatch by id. The consequences, each of which we want:

- an agent can do anything a user can, with no extra work per feature;
- undo, macros and replay come from one journal of `(id, params)` pairs;
- tools are tested with a list of events and no mouse, no GPU and no browser;
- the menu, shortcuts and tool buttons are tables, and greying and its reason come from one place.

This document picks the parts that fit us, says where they go in this repository, and orders the work. It does not copy their size: they have 24 crates and 500 commands, we have one crate and will have about a dozen editing tools.

## What we have and what is missing

| Area | Today | Gap |
| --- | --- | --- |
| Mutation path | Input systems and egui panels call domain functions with `(world, session)` directly (`systems/contour_editing.rs`, `gui/toolbar.rs`). `AppEvent` has 9 variants. | No single place where an edit enters. No action vocabulary an agent could use. |
| Headless | `advance_roi_work` takes `gpu: Option<_>` and the ROI tests build a plain `World`. | No `Scene`-level API to open a volume, edit and read back without winit or wgpu. |
| Undo | Per-ROI snapshots (`app/roi/history.rs`), driven from the toolbar. | Not tied to any command or journal. |
| Tools | `EditorTool` enum and fields on `EditorState`; pointer handling in `systems/input.rs` tied to winit and egui events. | Tools cannot be driven by recorded events. |
| Agent surface | `__viewerQa` in the browser: read-only state, metrics, logs, `lastError`. | Cannot drive the app. Native has nothing. |
| Dev tooling | Commands copied between `AGENTS.md`, `ci.yml`, `clippy.toml`; they disagree (backlog 6.3). | No single entry point for the checks. |

## Principles for the borrowed parts

1. **Commands are the only way to change the scene.** UI handlers and pointer systems compute intent (in world mm, per `docs/spatial-model.md`) and call a command. They do not mutate ROI state themselves.
2. **Typed in Rust, JSON on the wire.** Their params are `serde_json::Value` because they have hundreds of commands. We have few, so each command is a variant of one `serde`-derived enum. We keep compile-time checking, and the JSON form comes for free.
3. **World coordinates only.** A command never mentions a viewport pixel. This fits the spatial model and keeps commands valid from any view, from a script, and from an agent.
4. **Disabled has a reason.** The same string is shown in the tooltip and returned to an agent.
5. **Never panic on input.** A command returns `Err` for any params and any scene state, and a test enforces it.
6. **The browser is a first-class target.** Anything native-only (TCP, threads) sits behind a transport, and the protocol is designed first, transports second.
7. **Remove as we go** (existing project rule). Each step deletes the code it replaces.

## The parts to borrow, in order

### B1. One command layer (the foundation; everything else depends on it)

**What.** A `src/command/` module holding the `Command` enum, its enablement and its apply function.

**Where it sits.** Above `model`, `convert`, `util`, `io`, and next to `app::roi`. It may use `app` types (the scene, `Roi`, history). `systems`, `gui` and the QA/control surface call it; it never calls them. Add it to `tests/layering.rs` as a layer whose allowed imports exclude `gui`, `render` and `systems`.

**Shape.**

```rust
// src/command/mod.rs
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "id", content = "params", rename_all = "snake_case")]
pub enum Command {
    RoiSetActive { roi: EntityId },
    RoiSetVisible { roi: EntityId, visible: bool },
    RoiSetLocked { roi: EntityId, locked: bool },
    ContourApplyLoop { roi: EntityId, plane: PlaneRef, points_mm: Vec<[f64; 3]>, mode: LoopMode },
    ContourMovePoint { /* ... */ },
    MeshBrush { /* ... */ },
    EditUndo,
    EditRedo,
    // ...
}

pub struct CommandCtx<'a> {
    pub world: &'a mut hecs::World,
    pub session: &'a mut Session,
}

impl Command {
    /// `Err(reason)` is what the toolbar shows greyed and what an agent receives.
    pub fn enabled(&self, ctx: &CommandCtx) -> Result<(), Disabled>;
    pub fn apply(self, ctx: &mut CommandCtx) -> Result<Outcome, CommandError>;
    pub fn id(&self) -> &'static str;
    pub fn label(&self) -> &'static str;
}
```

- `EntityId` is the stable id from ADR 0005, not a `hecs::Entity`, so a command is valid across sessions and in a saved journal (ties into 3.7).
- `Outcome` is small and serialisable: what changed (ROI ids, dirty bounds), a status message, nothing pixel-sized.
- A catalog (`command::catalog()`) lists every command with id, label, default shortcut and one example params value. A test fails if a variant has no catalog entry.

**Journal and undo.** `Session` gets `journal: Vec<(String, Value)>` of applied commands that changed something. Undo stays the existing per-ROI snapshot history for now: a command that edits a ROI goes through `replace_contour_data_with_history` or `replace_mesh_data_with_history` as today. Journaling is additive; do not redesign undo in this step.

**Migration (smallest first, each its own commit, green).**

1. Scaffold the module, `Command`, the catalog and the `panic_hunt` test with three commands that already exist as functions: `RoiSetActive`, `RoiSetVisible`, `EditUndo`/`EditRedo`. Toolbar and sidebar call `command::run` instead of the functions.
2. `RoiSetLocked`, the `is_roi_locked` check moves into `enabled()` (today it is inline in `gui/toolbar.rs`). The toolbar greys with the reason.
3. Contour commands: `ContourApplyLoop`, `ContourMovePoint`, delete. `systems/contour_editing.rs` keeps only "what did the pointer mean" and ends by calling a command. The preview and commit split (`contour_move_pending_commit`) stays in the system; only the commit is a command.
4. Mesh commands, same way.
5. Delete what became unused (direct calls from gui, duplicated lock checks).

**Done when.** No file under `src/gui/` and no pointer system mutates an ROI without going through `Command`, checked by a grep-style test like the layering test. All lifecycle tests and the QA spec pass unchanged.

**Cost.** M for steps 1 and 2, then S to M per tool family. It also makes the editing tools plan (T1 to T11) cheaper: each new tool is a command plus a pointer handler, with a headless test written first.

### B2. A headless scene

**What.** Build the scene without a window: `Scene::headless()` creating the `World` and `Session`, loading a volume from bytes, running `advance_roi_work` with `gpu: None` until idle, and reading the result.

**Where.** A constructor next to `Session::new` in `src/app/components.rs`, plus `app/headless.rs` for `load_volume`, `run_until_idle`, `snapshot`. No wgpu import in either file; `tests/layering.rs` enforces it.

**Steps.**

1. `run_until_idle(&mut self, budget)` loops `advance_roi_work` until `pending` is false. The existing tests already do the pieces by hand.
2. Move the sample and synthetic bootstraps (`app/qa/sample.rs`, `app/qa/synthetic.rs`) so they can be called from `cargo test` and not only from the browser.
3. `snapshot()` returns the same JSON the QA state returns, so one serialiser serves both.

**Done when.** An integration test in `tests/` opens the liver sample, runs `ContourApplyLoop`, runs to idle and asserts the volume, bounds and that the mesh is current, with no GPU. This replaces a number of Playwright cases that only exist because nothing else could drive the app.

**Cost.** S to M. It is the lowest-risk step with the highest return for CI, because GitHub runners have no WebGPU (backlog 6.3).

### B3. Pointer events as an input type

**What.** Separate "what the device did" from "what the tool should do". Define `ToolEvent { kind: Down|Move|Up, world_mm: [f64; 3], plane: PlaneRef, pressure, modifiers }` and have `systems/input.rs` translate winit and egui events into it. Tools consume `ToolEvent` only.

**Why.** Their `ui.pointer {events:[…]}` feeds the real tool code, so tool tests need no mouse. Our world-mm model already means a tool does not need pixels, so the translation is the only viewport-dependent step, and it uses the shared helpers in `src/convert/`, as `AGENTS.md` requires.

**Steps.**

1. Introduce `ToolEvent` and a `fn handle_tool_event(&mut CommandCtx, ToolEvent)` for the contour tool; keep the old path calling it.
2. Move `contour_editing.rs` and `mesh_editing.rs` logic behind it, deleting the winit types from their signatures.
3. Add tests that feed an event list (drag, hole, undo) and assert the contour data.

**Done when.** The contour and mesh tool tests are lists of events with no winit imports.

**Cost.** M. Do it together with the first tool of the editing plan (T1) so the new tool is the first one written against it.

### B4. Serialisable UI state and a tool table

**What.** One `UiState` struct (serde) for what a user or agent may want to read or set: active tool, active plane family, visible panels, layout protocol, zoom and pan per viewport. A `Tool` enum with an `ALL` array and a table of `(Tool, label, shortcut, command)`. The toolbar is drawn from the table.

**Where.** `EditorState`, `GuiState` and parts of `Session.input` are the sources; the new struct lives in `src/app/ui_state.rs` and is a field of `Session`. Pure view state (hover, in-flight drag) stays out of it.

**Why.** The QA snapshot then reads one struct instead of collecting from several, and setting the tool or layout from a script is one call.

**Cost.** S to M. Do it after B1 step 2, since "set tool" is a command.

### B5. A control channel and MCP, on one protocol

**What.** A transport-agnostic handler `control::handle(&mut CommandCtx, Request) -> Response` over JSON, with the methods:

- `engine.execute {id, params}` runs a command;
- `engine.commands` lists the catalog with current enablement and reasons;
- `scene.inspect` returns the snapshot of B2, `ui.inspect` and `ui.set` read and write `UiState`;
- `ui.pointer {events}` feeds B3;
- `app.open {path}` loads a volume or labelmap;
- `ui.screenshot` (needs B6).

Then two transports:

- **Native.** A loopback TCP listener in a thread that forwards requests to the UI thread over a channel and answers between frames (their `control_server.rs` and `control.rs`). Behind a `--control <port>` flag. An MCP server crate wraps the same protocol later, if wanted.
- **Browser.** Extend `__viewerQa` with `execute(command)`, `commands()`, `inspect()` and `pointer(events)`. It stays one `wasm_bindgen` closure layer calling the same handler. This is the real route for agent control of the deployed web app.

**Why a protocol first.** Native-only transport code cannot run on wasm. If the handler is plain Rust over JSON, both transports are thin.

**Cost.** M. A journal replay command (`engine.replay {journal}`) is a few lines once B1 exists and gives regression tests that replay a recorded session.

### B6. Offscreen snapshot

**What.** A wgpu readback that renders one frame to a texture and returns a PNG, native and wasm, behind `ui.screenshot` and `__viewerQa.screenshot()`. This is backlog 6.5.

**Why.** Their rule is "verify UI changes by rendering a PNG and looking at it". It is the missing piece for visual regression and for an agent to see the result. CI still lacks a GPU, so this runs locally and in the browser spec on a developer machine.

**Cost.** M. Independent of B1 to B5.

### B7. Tooling: an `xtask` and a stricter workspace

Independent of the above; can run in parallel.

1. **`cargo xtask ci`** runs fmt, clippy (`--all-targets --all-features -D warnings`), `cargo test`, the release guard tests and the wasm check, in that order, stopping at the first failure. `AGENTS.md`, `clippy.toml` and `ci.yml` all call it, which fixes the flag disagreement in backlog 6.3. Add the `[alias]` in `.cargo/config.toml`.
2. **Dev profile.** Check whether debug builds are slow on conversion and meshing. If so, `[profile.dev.package."*"] opt-level = 2` and `[profile.dev] opt-level = 1`, as they do.
3. **Fuzz the NIfTI parser.** A `cargo fuzz` target on `io::nifti` (load from bytes), run manually or on a schedule. Parsers of untrusted files are what fuzzing is for.
4. **`panic_hunt` test.** Part of B1: serialise every catalog example, then mutate params (empty, huge, negative, NaN, unknown ids) and apply on a small scene; every result must be `Ok` or `Err`, never a panic.
5. **Test corpora out of git.** Backlog 6.7: `qa_samples/` stays in history, but new data comes from `cargo xtask corpus --download`.
6. **Ratchets as tests.** A constant that must not go down, enforced by a test: for example the number of catalog commands with a headless test, or the QA spec's case count. They do this for menu parity.

### B8. Workspace split (later, only with evidence)

Their enforced layering is a table in `xtask` read from `cargo metadata`. The equivalent for us would be a workspace of 3 to 5 crates, for example `model` + `convert` (pure and wasm-safe), `io`, `app` (command layer, scene, headless) and the binary (winit, wgpu, egui). That turns `tests/layering.rs` into a build-time guarantee and lets a CLI or MCP binary depend on `app` without the window code.

Do not start it now. After B1 and B2 we will see whether `command` and the scene really have a clean boundary from `render` and `gui`; if they do, the split is mechanical. If not, the split would only hide the coupling. Treat it as a decision after B2, not before.

## What we are deliberately not borrowing

- **JSON params everywhere.** A typed enum is enough at our size (principle 2).
- **A fixed target menu.** They follow Photoshop's menu tree, and we have no equivalent. Our toolbar table is enough.
- **Many themes and a dock system.** One theme with tokens (colour constants in one place) is enough; the dock comes from their domain.
- **500 commands and a parity generator.** The ratchet idea is kept (B7.6), the scale is not.
- **A shared `craftrules` sibling repo.** They have several apps; we have one.
- **PSD-style byte-exact round trips.** We already have `nifti_roundtrip`; the lesson we keep is keeping one format test at the gate.

## Order and chunks

Follows the working agreement in `docs/roi-workstation-todos.md`: one chunk per branch, every commit green, an exit check and a short report.

| Chunk | Items | Exit check |
| --- | --- | --- |
| K Command layer | B1 steps 1 to 5, B2 | The headless integration test (B2) passes; no `gui` or pointer system mutates an ROI outside `Command`; `panic_hunt` green; lifecycle tests and QA spec unchanged |
| L Input and UI state | B3, B4 | Contour and mesh tool tests are event lists without winit; QA snapshot reads `UiState` |
| M Control surface | B5 (browser first), B6 | `__viewerQa.execute` drives an edit in the QA spec; a replay of a recorded journal reproduces the final state |
| N Tooling | B7 (any time, in parallel) | `cargo xtask ci` is the one documented check and CI calls it |

Chunk K should come **before** Phase 3 editing work (T1 onward), because every editing tool is then written once, as a command with a headless test, instead of being written against the window and refactored later. If the owner wants T1 first, write it as a command from the start and treat chunk K step 3 as already done for the contour tool.

## Risks

- **Undo and previews.** Drags produce many preview states and one commit. Commands must be the commit only; previews stay in the systems (as `contour_move_pending_commit` does today). If previews also became commands the journal would hold noise. Keep the line sharp.
- **`hecs::Entity` is not stable.** Commands must use the ADR 0005 ids; step 1 needs a lookup from id to entity, which probably exists for annotations and must exist for ROIs.
- **Browser re-entrancy.** `__viewerQa.execute` runs between frames on the one thread; it must queue and answer after the next frame, as their `AfterInput` outcome does, so a following `inspect` sees the effect.
- **Large payloads.** Commands carry intent (points, ids), never volumes or meshes; `Outcome` reports ids and bounds, never data.
- **Scope creep.** Chunk K touches the toolbar, sidebar and two systems. Keep to the migration order above and add no features in it.

## Decisions needed from the owner

1. Chunk K before Phase 3 (recommended), or T1 first as the first command?
2. Typed `Command` enum (recommended) or their JSON registry?
3. Browser control first (recommended, the deployed target is the web) or native TCP first?
4. Is an MCP server wanted, or is the JSON control channel enough for now?
5. Do journals need to be saved with sessions (3.7)? If yes, the ids in B1 must be the persistent ids from the start.

## Reference (their files)

Paths in `storytold/photocraft` (read 2026-10-04):

- `AGENTS.md` (golden rules, "everything is a command", never panic on input)
- `crates/engine/src/commands.rs` (`CommandSpec`, enablement, params)
- `crates/automation/src/{rpc,server,bridge,headless}.rs` (protocol, MCP, bridge, headless)
- `crates/ui-egui/src/{control,state,actions,enable_rules,menu_catalog}.rs` (control handler, UI state, journal macros, shared enablement, menu table)
- `docs/control-protocol.md` (the method list)
- `xtask/src/{main,layers}.rs`, `.github/workflows/ci.yml`, `.cargo/config.toml` (tooling and layering)
