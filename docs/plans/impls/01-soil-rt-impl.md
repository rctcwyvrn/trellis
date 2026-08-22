# Implementation Plan 01 — `soil-rt`

*Detailed implementation guide for [`../01-soil-rt.md`](../01-soil-rt.md).
That plan states the scope and exit criteria; this one states how the crate
is actually structured and built, records the implementation decisions
resolved with the user on 2026-08-22, and proposes the remaining
micro-details (§8) for review before code is written. References: design
§3.7, §3.10, §3.12, tr-grammar §7, soil-syntax-spec §1 (string escapes),
plan 02 (the first consumer).*

---

## 1. Resolved implementation decisions (2026-08-22)

| Decision | Choice | Rationale |
|---|---|---|
| `Value` representation | Rust enum, heap variants behind refcounted boxes | Idiomatic and mostly safe; layout stays internal to the crate, so the C ABI and later compiled code see only opaque `SoilValue*` handles and the representation can change without ABI breakage. A uniform tagged word was rejected as unsafe-heavy before any profiling justifies it — "slow is accepted" (bootstrap plan). |
| Reference counts | Non-atomic, single-threaded instances | Rc-style counts; a runtime instance and all its values belong to one thread (documented C ABI rule). The daemon parallelizes with one instance per thread. Atomic counts were rejected as a permanent tax for a concurrency story the design defers (concurrency is FFI to host libraries). |
| `Ref<T>` | Thin newtype over `std::rc::Rc<T>` plus a debug-build live-allocation counter | Minimal unsafe, satisfies the leak-check exit criterion. A custom refcount header was rejected as premature: Perceus reuse is compiler-side and far away, and nothing inspects headers yet. |
| Canonical JSON | Hand-rolled encoder; decode parses via `serde_json` into a generic tree, then a type-directed walk | The encoder *is* the canonicality contract (byte-equal output), so it must be owned code, using `ryu`/`itoa` for shortest-round-trip numerals. Parsing is borrowed from a battle-tested crate; outsourcing the *encoder* to `serde_json` was rejected because the core spec guarantee would depend on a third-party crate's formatting stability. |
| Closure invocation | `Value::Closure` wraps a boxed `Fn(&[Value]) -> Result<Value, SoilError>` | One invocation path for everyone: plan-01 tests pass plain Rust closures; soil0's interpreter (which links this crate) captures AST+env in a Rust closure; compiled code later wraps an `extern "C"` fn + env value in the same shape. A registered-invoker callback was rejected: it needs a second native mechanism for tests anyway. |
| Derived `hash` | FNV-1a 64-bit, fixed and documented | `hash` is language-observable (design §3.7), so determinism across OS/arch/runs/toolchains is the requirement. Soil maps are comparator-ordered, not hash tables, so there is no DoS surface and SipHash's machinery buys nothing; `std::DefaultHasher` is explicitly unstable across Rust releases. |
| `hash` input bytes | The value's canonical JSON encoding; the `opaque` strategy hashes the address | There is exactly one byte-form of a value in the whole system. Equal ⇒ byte-equal JSON is already the canonical-encoding guarantee, so equal ⇒ same hash follows for free, and ignored fields contribute a constant by construction (they are omitted from the encoding). Cost is an encode per hash call — acceptable; only the algorithm, not the definition, would change if it ever binds. A parallel structural byte-feed was rejected as a second definition of a value's byte form to keep in agreement. |
| Type identity | Interned per-instance `TypeId(u32)`, dense index into a per-runtime registry | Cheap comparisons and lookups; the C ABI passes the u32. Ids are not stable across runs — anything persistent uses the type *name*, which is fine because filename/name is identity in Trellis (design §4.3). String names everywhere was rejected: every lookup becomes map-by-string and ids get retrofitted later anyway. |

The one language-observable item (the `hash` definition) is recorded in
design §3.7; the rest are runtime-internal and live here.

---

## 2. Workspace and crate skeleton

Per plan 01's resolved decision points: the Cargo workspace lives in
`rust/`; the repo root stays docs/examples/Trellis-roots.

```
shell.nix              -- repo root; the dev environment, single source of truth (§8.12)
rust/
  Cargo.toml           -- workspace; members = ["soil-rt"] (later soil0, daemon, clif driver)
  check.sh             -- runs inside nix-shell: fmt --check, clippy -D warnings, test, C demo build+run
  soil-rt/
    Cargo.toml
    cbindgen.toml
    src/               -- module map in §3
    fixtures/          -- (descriptor, value, canonical JSON) triples, §6
    cdemo/
      main.c
      build.sh         -- cc against the staticlib + generated header
```

**All dev dependencies are managed through a repo-root `shell.nix`**: the
Rust toolchain (rustc/cargo — no `rust-toolchain.toml`; the nixpkgs pin in
`shell.nix` is the single source of the compiler version), `cbindgen`, the
C compiler for the demo, and whatever later milestones add (Z3, Python).
Building outside `nix-shell` is off the supported path. Rust *crate*
dependencies remain in `Cargo.toml`/`Cargo.lock` as usual — `shell.nix`
manages tools, Cargo manages crates.

Crate dependencies, kept deliberately short:

| Crate | Why | Where |
|---|---|---|
| `num-bigint` | `BigInt` (plan 01 resolved decision) | runtime |
| `ryu`, `itoa` | shortest-round-trip float / integer formatting in the canonical encoder | runtime |
| `serde_json` | decode-side parsing to a generic tree only; never encodes | runtime |
| `base64` | `Bytes` encoding | runtime |
| `proptest` | property tests | dev-only |
| `cbindgen` | header generation | tool from `shell.nix`, invoked from `check.sh`, not a `build.rs` dependency |

`soil-rt` builds as both `rlib` (for `soil0` and the daemon) and
`staticlib` (for the C demo and embedding).

---

## 3. Module map

```
src/
  lib.rs         -- Runtime (owns Registry + debug counters), re-exports, crate docs
  error.rs       -- SoilError, PanicKind, TraceFrame
  value.rs       -- Value, Ref<T>, heap payloads (Record, SumVal, MapVal, Closure, OpaqueVal)
  descriptor.rs  -- TypeShape, TypeDesc, FieldDesc, Registry, TypeId; descriptor JSON (§5)
  ops.rs         -- derived eq / compare / hash over Value + descriptor
  json/
    mod.rs
    encode.rs    -- the canonical encoder; owns every canonicality guarantee
    decode.rs    -- type-directed decode over the serde_json tree
  capi.rs        -- the C ABI surface (§7), the only module containing `extern "C"`
```

Dependency direction is strictly downward: `capi → {json, ops} →
{descriptor, value} → error`. No module reaches back up; no global state
anywhere (`Runtime` is a value the embedder holds).

---

## 4. Build order

Each step names its deliverable, the API it stabilizes, and its tests.
Steps are sequential; a step is done when its tests pass and `check.sh` is
green.

### Step 1 — skeleton and `SoilError`

Workspace files, empty modules, `check.sh`, toolchain pin.

```rust
pub struct SoilError {
    pub kind: PanicKind,       // Overflow, DivideByZero, DecodeError,
                               // TypeError, DerivationError, CapiMisuse, …
    pub message: String,
    pub trace: Vec<TraceFrame>, // hook only; filled by soil0/daemon later
    pub inputs: Option<String>, // offending canonical-JSON inputs when available
}
```

This is the debug-mode payload of design §3.11's host-stub rule; plan 02
raises interpreter panics as `SoilError`, so the shape is API from day one.
Tests: construction and `Display` formatting only.

### Step 2 — `Value` and `Ref<T>`

```rust
pub enum Value {
    I64(i64), U64(u64), I32(i32), U32(u32),
    I16(i16), U16(u16), I8(i8), U8(u8),
    F64(f64),
    BigInt(Ref<BigInt>),
    Utf8(Ref<str>),
    Bytes(Ref<[u8]>),
    Unit,
    Record(Ref<Record>),     // TypeId + field values in declaration order
    Sum(Ref<SumVal>),        // TypeId + variant index + optional payload
    List(Ref<Vec<Value>>),
    Map(Ref<MapVal>),        // sorted Vec<(Value, Value)> + comparator Closure
    Closure(Ref<Closure>),   // Box<dyn Fn(&[Value]) -> Result<Value, SoilError>>
    Opaque(Ref<OpaqueVal>),  // TypeId + Box<dyn Any>; identity = address
}
```

- `Value` is small and passed by value; `clone` is a refcount bump on heap
  variants.
- `Ref<T>` wraps `Rc<T>`; in debug builds every allocation increments and
  every final drop decrements a **thread-local** live counter (consistent
  with instances being single-threaded), exposed as
  `soil_debug_live_values()` for the leak-check exit criterion. Release
  builds compile the counter out.
- `Bool` is not a `Value` variant: it is the prelude sum `True | False`
  (plan 01 scope item 2). The registry pre-registers it (step 3) and the
  JSON layer special-cases its `TypeId`.
- Cycles leak; documented on `Ref` (design §10 defers the strategy).
- `MapVal` maintains the sorted-vec invariant internally: insertion is
  binary search via the carried comparator (a `Closure` — fallible, so
  every map operation is fallible). Plan 01's resolved decision: swap for
  a tree behind the same API only if profiling demands it.

Tests: refcount behavior (clone/drop, counter returns to zero),
map insert/lookup/remove with a native comparator closure, closure
invocation, `Value` size assertion (fits two words + discriminant).

### Step 3 — descriptors and the registry

```rust
pub enum TypeShape {
    I64, U64, I32, U32, I16, U16, I8, U8,
    BigInt, F64, Utf8, Bytes, Unit,
    List(Box<TypeShape>),
    Map(Box<TypeShape>, Box<TypeShape>),
    Closure,                  // may appear in shapes; poisons derivation
    Named(TypeId),            // records, sums, opaques — including recursion
}

pub struct TypeDesc {
    pub name: String,               // cased name, e.g. "SummaryRow"
    pub strategy: Strategy,         // Structural | Opaque
    pub body: TypeBody,             // Record(Vec<FieldDesc>) | Sum(Vec<VariantDesc>) | Opaque
}
pub struct FieldDesc {
    pub name: String,
    pub shape: TypeShape,
    pub ignored: Option<Closure>,   // default thunk; presence marks the field ignored
}

impl Registry {
    pub fn declare(&mut self, name: &str) -> Result<TypeId, SoilError>;
    pub fn define(&mut self, id: TypeId, desc: TypeDesc) -> Result<(), SoilError>;
    pub fn register(&mut self, desc: TypeDesc) -> Result<TypeId, SoilError>; // declare+define
    pub fn get(&self, id: TypeId) -> &TypeDesc;
    pub fn lookup(&self, name: &str) -> Option<TypeId>;
}
```

- **Two-phase registration** (`declare` then `define`) exists because
  recursive types across definitions are legal (design §3.8); a
  self-referential shape names its own declared id. Using an undefined id
  in any runtime operation is `CapiMisuse`.
- **Derivability is computed at `define` time**: a type whose shape
  transitively contains `Closure` (through fields, payloads, list/map
  elements — map *comparators* excluded, they are structure, not content)
  is marked non-derivable, per "deriving `eq` on a type containing an
  arrow is a type error" (design §3.7). Derived ops on such a type return
  `DerivationError`; the static rejection happens in soil0's checker.
- The registry pre-registers `Bool` (sum `True | False`, in that
  declaration order) at construction and exposes `Registry::BOOL`.
- Strategies per design §3.7: `Structural` (default), `Opaque`
  (identity/`"<handle>"`/address); `ignored` is per-field with a mandatory
  default thunk.

Tests: registration round-trips, recursive type via declare/define,
closure-poisoning marks non-derivable, duplicate names rejected,
`Bool` present.

### Step 4 — derived operations (`ops.rs`)

One implementation each of `eq`, `compare`, `hash` over
`(&Runtime, &Value)`; `show` is the canonical encoder (step 5), per
"`show` is the JSON encoder" (design §3.7).

- **`compare` is a total order per type.** Numeric types compare
  numerically within their own type; comparing values of different types
  is `TypeError` (the checker prevents it; the runtime is defensive).
- **`F64`:** all NaN bit patterns are one logical value — equal to each
  other, sorting after `+Inf` (design §3.7: "NaN is equal to itself and
  sorts last"). This is `total_cmp` semantics with the NaN payload/sign
  distinctions collapsed, because canonical JSON has a single `"NaN"` and
  round-tripping must preserve `eq`. `-0.0 < 0.0` stays distinct
  (canonical JSON distinguishes `-0.0` from `0.0`). Pinned in §8.
- **Structural equality**: records field-wise in declaration order; sums
  by variant index then payload; lists element-wise then by length; maps
  by sorted entry sequence (**the comparator closure is excluded from
  `eq`** — it is structure, not content); `Utf8`/`Bytes` byte-wise;
  `BigInt` numerically.
- **`opaque` strategy**: `eq`/`compare`/`hash` on the payload address.
- **`ignored` fields**: skipped by `eq`/`compare`; constant `hash`
  contribution by construction (omitted from the canonical encoding).
- **`hash`**: `fnv1a64(canonical_json_bytes(v))` with the standard FNV-1a
  offset basis and prime, documented in the module; `opaque` hashes the
  address bytes instead.
- `eq`/`compare`/`hash` reaching a `Closure` value is `DerivationError`
  (defense in depth behind the descriptor-level rejection).

Tests: unit tests per type; property tests deferred to step 6 where
generators exist.

### Step 5 — the canonical encoder (`json/encode.rs`)

Implements tr-grammar §7 exactly, writing into a `Vec<u8>`. This module
owns every canonicality guarantee; nothing else in the system may produce
value JSON.

| Case | Encoding |
|---|---|
| fixed-width ints | `itoa` |
| `BigInt` | JSON number within ±(2^53−1); decimal string beyond |
| `F64` | `ryu` shortest round-trip; `"NaN"`, `"Inf"`, `"-Inf"` as strings |
| `Utf8` | JSON string, escape set in §8 |
| `Bytes` | base64 string (standard alphabet, padded, unwrapped — §8) |
| `Unit` | `null` |
| `Bool` | `true` / `false` (TypeId special case) |
| record | object, fields in declaration order, ignored fields omitted |
| sum | `{"tag": "Name"}` / `{"tag": "Name", "value": …}` |
| `List` | array |
| `Map` | array of `{"key": …, "value": …}` in comparator order (storage order) |
| opaque | `"<handle>"` |
| closure | hard error |

No whitespace anywhere (element separators are `,` and `:` alone — §8).
Encoding is value-directed: scalars self-describe via their variant,
records/sums carry their `TypeId`, and the registry supplies field and
variant names.

Tests: golden strings per row of the table, including BigInt at
±(2^53−1)±1, negative zero, integral floats (`1.0` not `1`), and the
determinism test (encode twice, byte-equal).

### Step 6 — type-directed decode (`json/decode.rs`) and fixtures

`decode(&Runtime, &TypeShape, &str) -> Result<Value, SoilError>`: parse
with `serde_json` into `serde_json::Value`, then walk the shape. Strict:
every deviation is a `DecodeError` naming the JSON path.

- Numbers must be integral and in range for the target width; `BigInt`
  accepts number-or-string (hybrid); `F64` accepts numbers and the three
  strings.
- Records: missing non-ignored field, unknown field, or a *present*
  ignored field are errors (§8); after the other fields decode, each
  ignored field is refilled by invoking its default thunk with the
  non-ignored fields as arguments in declaration order (§8).
- Sums: exactly the keys `tag` (+ `value` iff the variant has a payload);
  unknown tag is an error. `Bool` accepts only `true`/`false`.
- Opaque shapes and closure shapes are errors (tr-grammar §7).
- Recursion depth is bounded by `serde_json`'s parser limit; the walk
  itself is iterative or depth-checked to keep `panic` out of the crate.

**Fixtures** (`fixtures/*.json`, the shared goldens plan 01 requires,
reused later by soilc's passes):

```json
{
  "types": [ …descriptor JSON, §5-encoded… ],
  "type": "SummaryRow",
  "canonical": "{\"count\":3,\"mean\":1.5}",
  "accepts": ["{\"mean\":1.5,\"count\":3}"],
  "rejects": ["{\"count\":3}", "{\"count\":3,\"mean\":1.5,\"x\":0}"]
}
```

The harness decodes `canonical`, re-encodes, requires byte equality;
decodes each `accepts` entry and requires `eq` with the canonical value;
requires each `rejects` entry to fail. Fixture files cover every row of
the tr-grammar §7 table, including the `Bool`, BigInt-range, and
ignored-field-default cases named in the exit criteria. Default thunks in
fixtures are limited to a tiny built-in vocabulary the harness provides
(e.g. a constant, a field copy), since fixtures are data, not code.

**Property tests** (proptest), closing plan 01's testing section:

- generator of random well-formed descriptors + well-typed values;
- decode(encode(v)) `eq` v; encode determinism (byte-equal);
- `eq` ⇒ same `hash`; `compare` total-order laws (reflexive, antisymmetric,
  transitive, total) including NaN and `-0.0`;
- `compare == Equal` ⇔ `eq`.

### Step 7 — descriptor JSON (`descriptor.rs`, serialization half)

The fixtures and the C ABI both need descriptors as data. The encoding
follows tr-grammar §7's own conventions, as if descriptors were Soil
values (internally tagged sums, records):

```json
{ "name": "SummaryRow",
  "strategy": {"tag": "Structural"},
  "body": {"tag": "Record", "value": [
    {"name": "count", "shape": {"tag": "U64"}, "ignored": null},
    {"name": "mean",  "shape": {"tag": "F64"}, "ignored": null} ] } }
```

`TypeShape::Named` serializes by *name*, not id (ids are per-instance);
loading a descriptor list resolves names in two passes (declare all,
then define all), which handles recursion for free.

This format is the seed of plan 02's `--types env.json` contract; plan 02
freezes it in `docs/soil0-cli.md`, so it should be reviewed with that in
mind, but it is *not* frozen by this plan.

### Step 8 — the C ABI (`capi.rs`) and the demo

Surface (prefix `soil_`, verbatim rules: no global state, explicit
init/teardown, no unwinding across the boundary):

```c
SoilRuntime *soil_init(void);
void         soil_teardown(SoilRuntime *);

/* types: registered as descriptor JSON — one format everywhere */
int  soil_register_types(SoilRuntime *, const char *desc_json, SoilError **err);

/* values: opaque handles; constructors, accessors, refcounting */
SoilValue *soil_i64_new(int64_t);                 /* …one per scalar kind */
SoilValue *soil_record_new(SoilRuntime *, uint32_t type_id,
                           SoilValue *const *fields, size_t n, SoilError **err);
/* …sum_new, list_new, accessors (checked, error out-param)… */
SoilValue *soil_value_clone(const SoilValue *);
void       soil_value_free(SoilValue *);

/* derived ops and the JSON bridge */
bool   soil_eq(SoilRuntime *, const SoilValue *, const SoilValue *, SoilError **);
char  *soil_show(SoilRuntime *, const SoilValue *, SoilError **);   /* canonical JSON */
SoilValue *soil_decode(SoilRuntime *, const char *shape_json,
                       const char *value_json, SoilError **);

/* errors */
const char *soil_error_message(const SoilError *);
int         soil_error_kind(const SoilError *);
void        soil_error_free(SoilError *);

/* debug builds only */
size_t soil_debug_live_values(void);

int soil_main(int argc, char **argv, SoilMainFn);   /* trivial main wrapper */
```

- `SoilValue*` is a leaked `Box<Value>`; clone/free manage it. Handles and
  the runtime are single-threaded (decision §1); documented in the header.
- Every entry point wraps its body in `catch_unwind`; a caught panic
  becomes a `SoilError` of kind `Internal` — Rust panics never cross the
  boundary.
- Registration goes through descriptor JSON rather than a C struct
  surface: one format for fixtures, plan 02's `env.json`, and embedding,
  and the C API stays five functions instead of thirty (§8).
- Header generated by cbindgen into `soil_rt.h`; `cdemo/main.c` registers
  a record type, constructs a value through the ABI, `show`s it, decodes
  it back, checks `eq`, frees everything, and asserts
  `soil_debug_live_values() == 0`. `check.sh` builds and runs it — the
  embeddability exit criterion.

Closures are **not** constructible over the C ABI in this plan
(`soil_closure_new` arrives with compiled code, plan 05); the demo
therefore uses map-free, ignored-free types, and Rust tests cover the
rest.

### Step 9 — docs pass

Rustdoc on `Runtime`, `Value`, `Ref`, `Registry`, `TypeShape`, the JSON
modules (stating the canonicality guarantees and the FNV-1a definition),
and `capi` (threading rule, error contract). Exit criterion: plan 02 can
interpret the prelude against this API without runtime changes, judged by
walking plan 02's scope list against the rustdoc.

---

## 5. Exit-criteria traceability

| Plan 01 exit criterion | Where it lands here |
|---|---|
| C demo constructs, round-trips, tears down leak-free | Step 8 (`cdemo` + debug counter) |
| Every tr-grammar §7 row implemented, incl. `Bool`, BigInt range, ignored defaults, backed by fixtures | Steps 5–6 (encoder table, fixtures corpus) |
| Descriptor API documented for plan 02 | Steps 3, 7, 9 |

---

## 6. Non-goals (restated from plan 01)

Execution of any kind, Perceus reuse, cycle collection, pyo3,
refinements. Additionally out of scope here: C-ABI closure construction
(plan 05), stable `TypeId`s across runs (names are the stable identity),
and any performance work beyond the size assertion on `Value`.

---

## 7. Risks and checks

- **Canonicality regressions** are spec violations, not bugs of degree;
  the determinism property test and fixture byte-comparisons run in every
  `check.sh`.
- **`ryu` output drift** (crate update changing formatting) would break
  canonical bytes: the fixtures pin the expected strings, so an update
  that changes output fails loudly; the lockfile pins the version.
- **`serde_json` float parsing** is imprecise without the
  `float_roundtrip` feature — found by the property tests during
  implementation (§8.15). The feature is on; the round-trip property
  guards against regression.
- **Map comparator misbehavior** (a comparator that is not a total order)
  silently corrupts the sorted-vec invariant. The runtime does not detect
  it (that is the refinement checker's future job); documented on
  `MapVal`.
- **Descriptor/value mismatch through the C ABI** (wrong field count,
  wrong scalar kind) must be a checked `SoilError`, never UB: `record_new`
  and friends validate against the descriptor.

---

## 8. Micro-pins — approved 2026-08-22

Per the overview's rule that new choices go to the user, these were
presented as proposals and **approved by the user on 2026-08-22** (item 12
added at approval time). They are pinned; reopening one is a new decision
point.

1. **String escape set (canonical JSON):** escape exactly `"` , `\`, and
   control characters U+0000–U+001F; use the short forms `\n` `\r` `\t`
   `\b` `\f` where they exist and lowercase `\u00xx` otherwise; all other
   characters (including non-ASCII) are raw UTF-8; no `\/`. (RFC 8785's
   choices; deliberately *not* Soil's source escape set, which is a
   different layer — soil-syntax-spec §1.)
2. **Whitespace:** none. Separators are `,` and `:` only.
3. **NaN and zero:** all NaN bit patterns are one logical value (equal,
   sorts after `+Inf`); `-0.0` and `0.0` are distinct with `-0.0 < 0.0`,
   and `ryu` renders them `-0.0` / `0.0`, which round-trip. NaN collapses
   because canonical JSON has a single `"NaN"`; zeroes stay distinct
   because the encoding distinguishes them.
4. **Base64 for `Bytes`:** standard alphabet, with padding, no line
   wrapping; decode rejects non-canonical padding/alphabet.
5. **Strict decode:** unknown record fields, missing non-ignored fields,
   and *present* ignored fields are all `DecodeError`s. Rationale for the
   last: encode omits them, so accepting them would admit a second,
   unverifiable source for a field whose value is defined to be
   reconstructed (design §3.7); one canonical form in both directions.
6. **Default-thunk arity:** an ignored field's default closure receives
   the record's **non-ignored fields, in declaration order**, as its
   arguments ("the record's other fields in scope", design §3.7,
   restricted to non-ignored to avoid ordering dependencies among ignored
   fields).
7. **Duplicate keys in decoded JSON:** rejected (`DecodeError`), not
   last-wins. Requires walking with a duplicate check since `serde_json`'s
   default map is last-wins — use its `preserve_order`/raw-value facilities
   or a custom visitor; whichever is chosen, the observable rule is
   "duplicates reject".
8. **Debug leak counter:** thread-local (instances are single-threaded),
   debug builds only, exposed as `soil_debug_live_values()`.
9. **Panic policy:** the crate itself never panics on valid API use;
   `catch_unwind` at the C ABI converts bugs to `SoilError::Internal`.
   Rust-side consumers (soil0) get `Result` everywhere.
10. **C-ABI type registration via descriptor JSON** (not a C struct
    builder API): one descriptor format across fixtures, `env.json`, and
    embedding.
11. **Descriptor JSON is reviewable but not frozen here**; plan 02
    freezes it inside `docs/soil0-cli.md` as the `--types` contract.
12. **All dev dependencies are managed through a repo-root `shell.nix`**
    (§2): it is the single pin for the Rust toolchain (no
    `rust-toolchain.toml`) and provides every tool (`cbindgen`, the C
    compiler, later Z3/Python); building outside `nix-shell` is
    unsupported. Cargo continues to manage Rust crate dependencies.
    Location and single-pin choice resolved with the user 2026-08-22.

Items 13–15 were added during implementation (2026-08-22, autonomous
session) — recorded here and **flagged for user review**:

13. **Decoded maps carry the derived structural order.** A map arriving
    through JSON has no program-supplied comparator closure to carry, so
    `MapVal` orders are `Structural` (the derived `compare`) or
    `Custom(closure)`; decode always builds `Structural`, map operations
    take the runtime (structural order consults the registry), and the
    order is structure, not content — `eq`/`compare` see only the
    entries. Map decode accepts entries in any order (re-sorted) but
    rejects duplicate keys.
14. **Ignored-default vocabulary.** `FieldDesc.ignored` holds an
    `IgnoredDefault`: `Const` (a constant of the field's own shape,
    scalar-only), `CopyField` (a non-ignored, same-shaped field), or
    `Native` (an arbitrary embedder thunk — what a Soil default
    expression eventually compiles to). `Const`/`CopyField` are the
    serializable subset used by descriptor JSON and fixtures; `Native`
    does not serialize. The C ABI's `soil_record_new` takes non-ignored
    fields and refills ignored ones, mirroring decode.
15. **`serde_json` needs its `float_roundtrip` feature.** The default
    float parse is not correctly rounded (`1.8821735589659427e48`
    re-parses to different bits), which breaks canonical round-trips;
    the round-trip property test caught it. The feature is enabled and
    pinned in `Cargo.toml` with a comment.

---

## 9. Open questions

- None currently. §8 was approved 2026-08-22; reopening any of its items
  is a new decision point for the user.
