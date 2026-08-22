# Plan 01 — `soil-rt`: the runtime crate

*References: design §3.7 (derived functions), §3.10 (runtime and
embedding), §3.12 (primitives), tr-grammar §7 (JSON value encoding).*

## Goal

A Rust crate owning the Soil value model, memory management, derived
operations, the canonical JSON bridge, and a C ABI for embedding. This
crate is permanent — it is never bootstrapped away — and everything else
(interpreter, compiled code, hosts) manipulates values only through it.

## Scope

1. **Workspace skeleton.** Cargo workspace at the repo root (or a `rust/`
   subdirectory — decision point) containing `soil-rt`; later crates
   (`soil0`, daemon, Cranelift driver) join the same workspace.
2. **Value model.** A `Value` representation covering: fixed-width integers
   (`I64`, `U64`, `I32`, `U32`, `I16`, `U16`, `I8`, `U8`), `BigInt`, `F64`
   with total order (`total_cmp`; NaN equal to itself, sorts last), `Utf8`
   (immutable, validated), `Bytes`, `Unit`, records (fields in declaration
   order), sums (tag + at most one payload), `List`, `Map` (stored in
   comparator order — the comparator is a Soil closure carried by the map),
   closures, and opaque handles. `Bool` is the prelude sum `True | False`;
   the runtime may represent it natively as an optimization but must
   present it as a sum.
3. **Type descriptors.** Runtime-registered metadata per type: field names
   and order, variant names, derivation strategy (`structural` / `opaque` /
   per-field `ignored` with default thunks). Descriptors drive derived ops
   and type-directed JSON decode. Registration API used by the interpreter
   now and compiled code later.
4. **Memory.** Reference counting. No Perceus reuse yet (that is
   compiler-side, stage 3+); no cycle collector (strategy explicitly
   deferred, design §10 — document that cycles leak for now).
5. **Derived operations.** One implementation each of structural
   `eq`/`compare`/`hash`/`show` over `Value` + descriptor, honoring
   strategies: `opaque` → identity/`"<handle>"`/address; `ignored` →
   skipped. Deriving `eq` over a closure is rejected at descriptor
   registration.
6. **Canonical JSON.** Encode and type-directed decode per tr-grammar §7,
   exactly: internally tagged sums, `Bool` as JSON booleans, `BigInt`
   hybrid by range (±(2^53−1)), `F64` `"NaN"`/`"Inf"`/`"-Inf"` strings,
   `Bytes` base64, `Unit` null, maps as comparator-ordered
   `{"key","value"}` arrays, ignored fields omitted on encode and refilled
   from default thunks on decode, opaque decode error, closures a hard
   error both ways. Encoding is canonical: declaration-order fields,
   shortest round-trip floats — byte-equal output for equal values.
7. **Errors.** A structured `SoilError` (panic kind, message, trace hook,
   offending JSON inputs when available) — the debug-mode payload of
   design §3.11's host-stub rule.
8. **C ABI.** No global state; explicit `soil_init`/`soil_teardown`;
   opaque `SoilValue*` handles with constructors/accessors/refcount ops;
   error out-parameters; a trivial `main` wrapper entry. Header via
   cbindgen. A minimal C demo program proves embeddability.

## Non-goals

Execution (plan 02), Perceus reuse, cycle collection, pyo3 (plan 06),
refinements (checker-side only, and later).

## Testing

- Rust unit tests per module; property tests (proptest) for: JSON
  round-trip identity on random well-typed values, `eq`/`compare`/`hash`
  agreement (equal ⇒ same hash; compare total order laws incl. NaN),
  canonical encoding determinism (encode twice, byte-equal).
- A `fixtures/` corpus of (type descriptor, value, canonical JSON) triples,
  checked in — these become shared goldens for soilc's passes later.
- The C demo compiled and run in CI fashion (a script for now).

## Exit criteria

- The C demo constructs values through the ABI, round-trips them through
  canonical JSON, and tears down cleanly (no leaks under a debug counter).
- All tr-grammar §7 rows demonstrably implemented, including the `Bool`,
  `BigInt`-range, and ignored-field-default cases, backed by fixtures.
- Descriptor API documented well enough that plan 02 needs no runtime
  changes to interpret the prelude.

## Decision points — resolved 2026-08-22

- **Workspace location:** `rust/` subdirectory holding the Cargo workspace
  (`soil-rt`, later `soil0`, the daemon, the Cranelift driver); the repo
  root stays docs/examples/Trellis-roots.
- **Integer widths:** all eight (`I64`…`U8`) from the start — adding
  widths later ripples through JSON, the C ABI, and descriptors, and
  together they are mostly a macro.
- **BigInt:** `num-bigint`.
- **Map representation:** sorted vec of pairs with the carried comparator
  closure — trivially correct and canonically ordered for `show`/JSON;
  swap for a tree behind the same API only if profiling demands it.

## Implementation plan

A detailed implementation guide exists at
[`impls/01-soil-rt-impl.md`](impls/01-soil-rt-impl.md). It records a
second round of decisions resolved 2026-08-22 — `Value` as a Rust enum
with nonatomic refcounted boxes, closures as boxed Rust callables,
hand-rolled canonical encoder with `serde_json` decode, interned
per-instance `TypeId`s, and `hash` as FNV-1a 64 over canonical JSON bytes
(also recorded in design §3.7, being language-observable) — plus the
module map, build order, fixture format, C ABI surface, and a set of
micro-pins (its §8, approved 2026-08-22, including: all dev dependencies
managed through a repo-root `shell.nix`, which is also the single pin for
the Rust toolchain).
