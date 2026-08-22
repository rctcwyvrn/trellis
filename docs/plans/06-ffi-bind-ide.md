# Plan 06 — FFI, `trellis bind`, and the minimal IDE

*References: design §3.11 (backends and FFI), §5 (batteries), §7.1 (IDE),
tr-grammar §3.5–3.6 (cram, contract tests via design §4.5).*

## Goal

Open the foreign world in the committed sequence — C ABI → Rust batteries
→ Python embedding → Python batteries — with hand-written (agent-written,
per-symbol) bindings, the `trellis bind` assistant, and the thinnest IDE
that makes the lowering loop pleasant.

## Scope

### FFI (sequenced; each step usable before the next)

1. **C ABI codegen.** The Cranelift backend learns `extern` calls against
   `soil-rt`'s C ABI; a worked shim example (a Rust `extern "C"` function
   wrapped as a Soil binding) joins the corpus.
2. **`soil-rs-std` batteries.** Refinement-typed Soil signatures over
   Rust-backed functions — runtime-owned values, so ordinary, refinable,
   capability-free when pure (design §3.11). Start from demand: what the
   compiler and prelude wished they had.
3. **Python embedding.** CPython via pyo3 inside `soil-rt`: GIL held
   around calls, `PyObject*` as opaque refcounted handles, `py_to_soil` /
   `soil_to_py` over the same JSON-shaped value model (one value model,
   never two), the `Py` capability + fake in the prelude. One worked
   pyo3 shim example joins the corpus.
4. **`soil-py-std` batteries.** Handle-in/handle-out style, `ffi panic
   io` rows, `Py` capability; the visible two flavours (cheap handles vs
   converting/refinable).
5. **Binding trust plumbing.** Contract tests auto-generated from
   signature + effect row (design §4.5: valid input → `Ok`, invalid →
   `Err` not `panic`, handle compatibility, leak-check loop, round-trip);
   lock `ffi` records (trust level, symbol hash, Nix store path — store
   path may be a plain path until the Nix milestone).

### `trellis bind <symbol>`

6. Symbol metadata fetch (Python: `.pyi`/`inspect`/docstring; Rust:
   `cargo doc` JSON) into `symbol.json` + `docstring.md`; the binding
   context bundle (one shim corpus example included); the normal lowering
   loop producing `.tr` + shim + contract tests + lock entry for human
   review. No bulk importer — ever (design §3.11).

### Minimal IDE

7. Electron-served web app over the daemon, as thin as possible: Markdown
   editor with test-block widgets (JSON drag-and-drop can start as
   guided JSON editing), lower button with streaming agent output, the
   question/answer panel (persisting answers to prose), graph view over
   the manifest (effect-flow coloring, `soil-private` greying), lock
   rendering as status badges (`typed`/`tested`/`verified`/`accepted`
   derived), REPL pane with REPL-to-expect-test promotion, accept/pin
   buttons with suggested git commits (never auto-commit).

## Non-goals

`py_module`/Python-hosts-Soil, `.pyi` stub generation, Node, TOML→Nix
(later milestone), whole-package binding generation (never), IDE polish.

## Testing

- FFI: contract-test generator exercised against deliberately broken
  shims (each failure mode caught); leak checks under the debug runtime;
  `py_to_soil`/`soil_to_py` round-trip properties.
- `trellis bind`: end-to-end against a fixed known symbol set (e.g.
  `json.dumps`, `re.compile`, one Rust crate fn) with recorded metadata
  so tests don't depend on the network.
- IDE: the golden path exercised in-browser against a fixture project —
  open, edit a test, lower, answer a question, accept — before calling
  any feature done.

## Exit criteria

- Both shim kinds exist as accepted corpus examples; a Soil program calls
  one Rust and one Python function through real bindings with contract
  tests green.
- `trellis bind requests.get` (offline-recorded metadata) produces a
  reviewable binding end-to-end.
- The IDE golden path works against the real daemon; question round-trip
  and accept flow usable without touching the CLI.

## Decision points — resolved 2026-08-22

- **IDE delivery: browser first.** The daemon grows an HTTP/WebSocket
  facade (`trellis daemon --serve`) and the web app is developed in a
  normal browser; Electron becomes a thin packaging wrapper later
  (design §7.1 unchanged — this sequences the wrapper last).
- **IDE stack:** Svelte + CodeMirror 6 (widget decorations for test
  blocks and the question panel); graph view via SVG or cytoscape.js.
- **`soil-rs-std` starts with the Rust standard library only** — a
  curated set drawn from `std` (math on floats, hashing, path/string
  utilities, whatever plans 04–05 wished for), no external crates
  initially. External crates (regex, chrono, …) arrive by demand through
  `trellis bind`, one symbol at a time.
- **Contract-test generation lives in the daemon** (Rust), next to the
  test runner it feeds; rewriting it in Trellis is possible dogfood
  later, not v1.
