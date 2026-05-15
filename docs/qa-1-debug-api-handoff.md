# QA-1 Debug API Handoff

This is the implementation handoff for the first viewer QA/debug tooling slice.

QA-1 is implemented. Keep this doc as historical scope/reference and as the acceptance contract for QA-1 regressions.

## Scope

Implement only:

- a target-neutral Rust QA state/log module
- serde-backed JSON snapshots for state/logs/metrics/last error
- WASM-only `window.__viewerQa` installation when `?qa=1` is present
- a minimal state snapshot with honest placeholder values where real data is not wired yet
- focused unit tests for the pure Rust QA helpers

Do not implement:

- `image_label_mpr_basic` sample loading
- `setPreset(...)`
- true readiness for sample/preset workflows
- viewport crop computation beyond cheap existing facts if they are already available
- app-side screenshot capture
- egui QA panels
- native-rendered debug overlays

## Files

Expected files:

- `Cargo.toml`
- `src/app/mod.rs`
- `src/app/qa.rs`
- `src/lib.rs`
- `index.html` only if the JS wrapper cannot be installed cleanly from Rust
- `docs/viewer-qa-debug-tooling-plan.md`

Avoid touching renderer/shader files in QA-1 unless a compile error forces a tiny integration adjustment.

## Implementation Steps

1. Add dependencies:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

2. Add `src/app/qa.rs` with:

- `QA_API_VERSION: u32 = 1`
- `QaLevel`: stable snake_case values `debug`, `info`, `warn`, `error`
- `QaError`: `category`, `message`, string `fields`
- `QaLogEvent`: `seq`, `frame`, `level`, `category`, `message`, string `fields`
- `QaLogBuffer`: fixed capacity 500, monotonically increasing `seq`, drops oldest when full
- `QaSnapshot`: minimal versioned state shape
- `QaMetricsSnapshot`: minimal metrics shape
- `QaRuntime`: enabled flag, requested sample/preset strings, log buffer, last error

Use `BTreeMap<String, String>` for structured fields in QA-1.

3. Wire `QaRuntime` into `AppState`:

- add `pub mod qa;`
- add `qa: qa::QaRuntime` to `AppState`
- initialize it in `App::new()`
- enable only when URL query contains `qa=1` on WASM

4. Build minimal snapshots from existing app state:

- no context: return `qa.enabled`, `ready=false`, empty `viewports`, no volume loaded
- context exists: fill cheap status, main volume facts, active ROI id/name, viewport facts, and placeholder render counters where needed
- do not invent readiness

For QA-1, `qa.ready` may be true only for trivial `?qa=1` with no sample/preset requested.

5. Install `window.__viewerQa` on WASM only:

- absent without `?qa=1`
- present with `?qa=1`
- methods: `version()`, `state()`, `metrics()`, `logs()`, `lastError()`, `waitForReady(options)`
- methods return JS objects/null, not JSON strings

6. Mirror QA runtime errors/warnings to browser console where wired:

- `error` -> `console.error`
- `warn` -> `console.warn`
- no default mirroring for `info`/`debug`

## Tests

Rust unit tests:

- buffer keeps latest 500 events
- dropped events preserve increasing sequence numbers
- `last_error` is set and serialized
- snapshot serializes stable snake_case strings

Browser checks after `trunk serve`:

- `/`: `window.__viewerQa === undefined`
- `/?qa=1`: `window.__viewerQa` exists
- `version()` returns API version
- `state()` returns an object
- `logs()` returns object/array containing log events
- `lastError()` returns `null` or structured error
- `waitForReady({ timeoutMs: 100 })` follows QA-1 readiness contract

## Validation

Required:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
```

QA-1 is complete only when:

- normal app behavior is unchanged without `?qa=1`
- browser QA object is absent without `?qa=1`
- browser QA object exists with `?qa=1`
- Rust unit tests cover log buffer and JSON helpers
- no sample/preset loading has been added
- required Rust validation commands pass
