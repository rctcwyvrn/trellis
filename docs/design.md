# Trellis: Design Document

*Trellis is the project, the specification layer, and the IDE. Soil is the target language it lowers to. The name reflects what the tool does: vibes grow, the trellis shapes them.*

*Status: design phase. This document records every decision made so far, the reasoning behind each, the items explicitly deferred, and the planned build order. It is intended to be the single reference for the project until the `.tr` grammar and lock schema documents supersede the relevant sections.*

---

## 1. Vision

### 1.1 The core thesis

If an AI agent writes the implementations, the human-authored layer of a program should consist of specifications, types, tests, and structural constraints rather than code. The target language those implementations are written in should be designed to be easy for a machine to write and easy for a checker to verify, rather than pleasant for a human to type. Soil is that target language; Trellis is the specification layer and the tooling that lowers specifications into Soil.

Soil is, in effect, the assembly language of vibe-coding: a small, strict, verifiable language that humans read but rarely write.

### 1.2 The shape of a project

Both intended users write a *slice*. An individual writes a small program that calls foreign libraries. A team writes a small module inside a large foreign codebase. In both cases Trellis owns a bounded region of the program and treats everything outside that region as untrusted foreign code with a tested boundary. The scope pitch is: *Trellis owns the region you care about being right; the rest of your stack is FFI.*

Trellis remains a vibe-coding language, with two tiers:

- **Unchecked vibing:** an agent writes the `.tr` files and an agent lowers them. The human reads nothing. This is still strictly better than vibe-coding as commonly practiced, because every function has a type that checks and tests that pass, even if no human looked at them.
- **Disciplined vibing:** the human writes the `.tr` files and an agent lowers them. The human's attention goes entirely to specifying and judging; they never write a loop, a match, or a signature. They are forced to think about correctness, not about code.

Both tiers produce identical artifacts, so a project can move from unchecked to disciplined one function at a time: a human takes over a `.tr` file, reviews or rewrites the tests, and marks the definition `accepted`. This is the migration path from "prototype I vibed" to "thing I trust" without a rewrite. The "humans write tests" rule concerns what gates lowering in the disciplined tier, not what workflows are permitted.

### 1.3 Users

- **First user:** the author, building small projects that use Python libraries.
- **Target users:** both individuals writing small projects against libraries, and large teams migrating small slices of code within a larger codebase.
- **First users beyond the author:** other individuals on their own small projects. This keeps team tooling deferred but pulls installation and the first-hour experience forward, since each user cannot be hand-held.

The rule adopted for resolving the tension between these two: decisions that affect *format or semantics* must be made now to accommodate both users; decisions that are *tooling only* are made in the simplest form for the first user and grown later.

### 1.4 Design tenet

**Nothing that affects correctness exists only in an agent's context.** Signatures, human answers to agent questions, export pins, style examples, and every other input to a lowering lives in a hashed file. An agent's "memory" of a function is its lock entry and its last lowering, nothing more.

---

## 2. The three components

### 2.1 Soil: the target language

A small, strict, refinement-typed ML with effect tracking, designed as an agent target and a glue language for tying other languages together.

### 2.2 Trellis: the specification layer

A file format and toolchain in which humans write prose, types (optionally), and tests, and an agent lowers each definition to Soil under the supervision of a type checker, a refinement checker, and the tests.

### 2.3 Trellis IDE and build system

A graph-oriented IDE over the definition graph, with an interactive lowering interface, and a build system that compiles a TOML declaration of dependencies down to Nix.

---

## 3. Soil language design

### 3.1 Surface language

| Aspect | Decision | Reasoning |
|---|---|---|
| Style | Direct style | CPS as a user-visible language is a liability for both agents and human readers. CPS/ANF is used as an intermediate representation only. |
| Typing | Strict, ML-style type inference, with refinement types | ML inference is kept because it reduces the surface area for agent error. Refinements add verifiable claims without requiring full dependent types. |
| Evaluation | Strict | Simpler for effects, backends, and agent reasoning about performance. Follows Koka and Idris 2. |
| Data | Sum types, product types, type aliases/typedefs | Standard ML data modelling. |
| Pattern matching | Yes | Standard. |
| Minimalism | "There is only one way to do something" | Less surface for the agent to hallucinate, more for the checker to catch. |
| Intermediate representation | ANF preferred over CPS for the mid-end | Easier to optimize for register machines and the JVM; easier for an agent to read when debugging lowering failures. |

### 3.2 Refinement types, not full dependent types

Full dependent types (Agda/Idris style) were considered and rejected for v1. They are hard to infer, would discard ML inference, and agents write proofs poorly. Refinement types (Liquid Haskell / F* style) keep inference, let the agent write specifications rather than proofs, and discharge obligations through an SMT solver. Full dependent types remain a possible future escape hatch for the small fraction of functions where SMT cannot help.

Where inference stops: types may mention terms only if those terms are `total` and fall in a decidable fragment (linear arithmetic, uninterpreted functions, lengths of lists and sets). Beyond that, the refinement is unproven and the function is demoted (see §6.4).

Refinements are erased at runtime in release builds *only if they were proven* (see §3.9).

**Refinements may eliminate error cases.** A function returning `Result` may carry a postcondition such as `{r | is_ok r}` under a precondition, allowing callers that can discharge the precondition to skip the match. This pushes obligations up the call chain, which is the intended ratchet behaviour: a caller that cannot prove the precondition simply matches on the `Result` as normal. The `Err` arm must still exist at runtime in unrefined/debug builds, because the refinement is erased.

### 3.3 Effects

Effects are tracked as **effect rows on types**, not as monads. Rows compose without the transformer-stacking problem, and row-polymorphic higher-order functions inherit the effects of their arguments automatically (so `map` over a `total` function is `total`).

The effect system is **purely a tracking mechanism**. There are no first-class algebraic effect handlers and no effect runtime. This keeps backends simple. The consequence accepted: no mock handlers for testing, which is addressed instead by the capability model (§3.5).

**Effect lattice:**

- `total`: the empty row; the function provably terminates and has no effects.
- `div`: may diverge.
- `panic`: may crash at runtime (indexing, division, unproven refinements, FFI).
- `io`: touches the world.
- `ffi`: calls foreign code; implies `panic`.
- User-declared algebraic effects are a possible future extension but not part of v1.

There is **no `exn` effect and no exceptions.** `Result a e` is the error channel — success type first, as in OCaml and Rust. Haskell's error-first `Either e a` order exists so the partially applied constructor can be the Functor/Monad instance, a motivation that cannot arise in Soil (no type classes, no higher-kinded abstraction), so the more widely known order wins. Cases that cannot be expressed as `Result` and cannot be proven safe by refinement carry `panic`.

**Function application annotations** (the Trellis feature of declaring what a function may call) fall out of the effect system: `f` may call `g` iff `g`'s row is a subset of `f`'s row. The graph view of the IDE is therefore also an effect-flow diagram.

### 3.4 Totality

Totality is tracked as the absence of `div` in the effect row. Termination is established by an **Idris/Agda-style termination checker**: structural decrease on an argument, or a user-supplied measure (`decreases n`). This is preferred over Koka's syntactic approach because an agent can usually supply the decreasing argument, and the annotation is cheap for both the agent to write and the checker to verify. If the checker fails, the function acquires `div` and the manifest may refuse it.

Only `total` functions may appear in type indices (refinements), because the type checker must normalize them. Totality is thus an effect at the term level and a gate at the type level.

Policy: total by default, `div` opt-in (the Idris policy), which is the right default for an agent-written language.

### 3.5 Capabilities

`io` is not ambient. A function that performs I/O receives an opaque capability value representing the permission, and the `io` effect in its row indicates that it uses it.

```
type Fs = opaque
type Net = opaque
type Clock = opaque

read_file : Fs -> Path -> io (Result String FsError)
now       : Clock -> io Time
```

`World` is the root capability, handed to `main` by the runtime (or by the host in embedded mode). Sub-capabilities are derived from it and cannot be constructed any other way because the types are opaque.

```
main : World -> io Unit
main w =
  let fs = world_fs w in
  ...
```

**Testing:** the prelude provides fake capabilities of the same type (`fake_fs [("config.toml", "...")]`), so a function under test cannot distinguish a real capability from a fake. The capability *is* the handler, passed by hand; no effect handlers are needed.

**Capability set (confirmed):** `Fs`, `Net`, `Clock`, `Env`, `Proc`, `Rand`, `Py`.

**Effect row stays a bare `io`.** Naming capabilities in the row (`<io:fs,net>`) was considered and rejected for v1 as the capability arguments already carry the same information.

**FFI needs a capability too.** A Python call takes `Py`. This makes FFI-calling functions visible, allows stubbed fakes for tests, and makes a prelude fork without a `Py` constructor into a Python-free sandbox.

**Boilerplate accepted:** the individual user writes `read_file fs path` rather than `read_file path`. A `with fs` sugar for implicit passing was considered and rejected, since implicit parameters are a form of type classes.

**Rationale for deciding this now:** the capability style must be present in every `io` signature in the prelude from the start. Retrofitting it would invalidate the corpus. A default "real world" capability keeps the individual user's CLI experience unchanged.

### 3.6 Polymorphism

**Parametric polymorphism only.** No type classes, no functors, no overloading of any kind. This is accepted as painful for humans and acceptable for an agent target, and it fits "one way to do things." Global coherence constraints of a class system would also conflict with the per-file incremental model.

Consequences accepted:

- No overloaded numeric literals. There is no general-purpose `Int` or `String` type; see §3.12. Literals have a default type (`1` is `I64`, `1.0` is `F64`, `"..."` is `Utf8`) unless annotated, as in Rust.
- **Maps take an explicit comparator** (OCaml `Map.Make` as a plain higher-order function). This is the confirmed idiom and the prelude will show it.
- **Sort takes a key function** (`sort_by : (a -> k) -> List a -> List a`), with structural `compare` on `k`. The Python idiom; agents know it.

### 3.7 Derived functions: `eq`, `show`, `compare`, `hash`

With no ad-hoc polymorphism there is exactly one possible `eq` per type, so all four are **auto-derived for every type** rather than opted into as in Rust.

Implementation: derived per type as new definitions (`Foo::eq`, etc. — `::` is the namespace separator, `.` being reserved for field access), rather than as polymorphic primitives. The user's reasoning was the ability to statically exclude closures and to give float types a specific treatment. (Both approaches are semantically equivalent given a kind restriction; per-type derivation was the chosen spelling.)

- **Functions:** deriving `eq` on a type containing an arrow is a type error.
- **Floats:** **total order**. NaN is equal to itself and sorts last (Rust's `total_cmp`). `Float` therefore derives all four functions and can be a map key. Decided.
- **FFI handles and abstract types:** pointer identity via the `opaque` strategy.
- **`show` is the JSON encoder** and `read`/`parse` is the decoder. Expect tests compare on JSON. One value format for everything.
- **Refinements:** erased at runtime; `eq` on `{v:Int | v > 0}` is `eq` on `Int`.
- **Large structures:** structural `eq` is O(n); accepted.

**No user override for now.** A first-class override mechanism was discussed and recognized as type classes returning through the side door (coherence, hash/eq agreement, equivalence-relation guarantees). Instead, types may declare one of a small fixed menu of **derivation strategies**:

| Strategy | `eq` | `show` | `compare`/`hash` |
|---|---|---|---|
| structural (default) | structural | JSON | structural |
| `opaque` | identity | `"<handle>"` | on address |
| `ignored` | always `True` | omitted | constant |

`ignored` is a per-field marker, not a per-type strategy, and must carry a default expression — total calls only, with the record's other fields in scope — which refills the field wherever a value is materialized without it (JSON decode, `py_to_soil`, host stubs): `cached_word_count : U64 ignored = word_count(text)`. Because there is no mutation (§3.13), the default always reproduces the value the constructor stored, so omission from `show` is lossless. `ignored` thus means "excluded from derivation, reconstructible on demand."

The cases a user override would have served are handled without it: case-insensitive strings via a newtype with normalization in the constructor; ignored cache fields via the `ignored` strategy; FFI handles via `opaque`. Real overrides, if ever needed, are understood to be the addition of a class system and are deferred indefinitely.

### 3.8 Recursion

**Mutual recursion across Trellis definitions is forbidden.** Content addressing becomes a tree, incremental checking is a topological walk, and every lowering has a well-defined "everything below me is already checked."

What is lost: mutual algorithms (even/odd, recursive-descent parsers with `expr`/`term`, traversals of mutually recursive data). The workaround is the standard one: one function with sum-typed dispatch, or mutually recursive locals. Nothing is inexpressible; only decomposition into separately specified units is constrained. Parsers are expected to be the most painful case, and to hurt the agent more than the human because the mutual shape is the idiom it has seen most.

**Within a single `.soil` file, `let rec ... and ...` is allowed freely.** Mutual recursion among locals and module-private helpers is a Soil-level detail invisible to Trellis. This covers most parser cases: `parse` is one Trellis definition whose lowering contains mutually recursive locals.

**Not a stdlib-only feature.** The stdlib must be written in the same dialect it teaches, or the corpus shows idioms users cannot use.

**Recursive types across definitions remain allowed.** Types are definitions but are not lowered, so cycles among them do not break the lowering walk. They need a combined cycle hash in the lock; this is the one cycle the lock must represent.

### 3.9 Debug and release modes

**First-class in Soil**, not two separate lowerings. Two lowerings would mean two artifacts that can diverge and two hashes. Instead, one lowering carries refinements, invariant checks, `panic` guards, and test hooks as erasable annotations.

**Release mode only erases what was proven.** A demoted (unproven) refinement still runs its check in release, because tests are the trust root and an unproven claim must not become an unchecked one. Unverified code carries runtime checks forever, which is the correct incentive.

### 3.10 Runtime and embedding

**The Soil runtime is a Rust crate exposing a C ABI, with a trivial `main` wrapper.** Writing the runtime in Rust makes three things one codebase: embeddability as a C library, shared ownership with Rust batteries (Rust values are runtime-owned, so no FFI and no conversion), and the memory model (reference counting, Perceus-style reuse, cycle handling). The compiler may still emit C, native code, or JVM bytecode; this decision is about the runtime only.

**Soil is embeddable as a C library.** This is committed to from the first commit: no global state, explicit init/teardown, callable from C. It serves both users from the same compiled artifact:

- Individual user: Soil is `main`, Python is a library it calls.
- Migrating team: Python (or another host) is `main` and imports the Soil module.

Writing the runtime as a program and libraryizing it later would be a rewrite.

### 3.11 Backends and FFI

- **v1 backend: Cranelift.** The backend proper is a pure pass emitting CLIF text (golden-testable like every compiler pass); a small Rust driver feeds Cranelift for instruction selection, register allocation, and object emission — x86-64 and arm64 from one backend, no C toolchain dependency. C emission, JVM, and direct x86 were considered and deferred: direct emission means writing the two largest, least-testable compiler phases first, and C is a semantically messy target that drags in an external toolchain. See `prototypes/bootstrap-plan.md` §4.
- **v1 FFI: both SysV/C ABI and Python.** They are different kinds of work and share no duplicated code. The C ABI is a layout and calling-convention problem in the compiler backend and is close to free once the runtime is Rust (Rust `extern "C"` functions are the native case). Python is a marshalling and lifecycle problem: CPython embedded from the Rust runtime (via `pyo3`), GIL held around calls, `PyObject*` wrapped as an opaque refcounted handle, `py_to_soil`/`soil_to_py` defined over the same JSON-shaped value model as `show`, tests, and host stubs (one value model, never two). Sequencing: C ABI → Rust batteries → Python embedding → Python batteries, each usable before the next. Rust is for the stdlib; Python is for libraries. Node comes later.
- **Hand-written bindings only in v1** for both FFIs. "Hand-written" means a specific binding exists because someone asked for it, one at a time, with a spec; it does not mean a human types it. A binding is three artifacts: a `.tr` spec (prose, Soil signature with effects and capabilities, auto-generated contract tests per §4.5); a shim (a Rust `extern "C"` function, or a `pyo3` function across the GIL); and a lock entry with trust level `declared-only`, `contract`, or `harvested`. The corpus includes one worked example of each shim kind so the agent writes them inside a normal lowering.
- **The whole-package binding generator is deferred, likely permanently.** Reading a crate or package and emitting its full surface is a separate project with an unbounded difficulty profile: every foreign type system is a new translator (Rust lifetimes, traits, generics; Python's effectively untyped `.pyi` stubs; C pointers and ownership); effects and capabilities are invisible in foreign signatures, so the generator either assigns everything `io + ffi + panic` with `World` (defeating the capability system) or guesses; refinements are entirely absent, so the valuable annotation is still manual; the surface is unbounded and most of it is never called, flooding the lock with untrusted entries (the same trust hole closed by rejecting `helpers.tr`); and subtly wrong generated bindings are memory-safety bugs surfacing far from their cause. Hand-written bindings grow the batteries layers by demand, are mostly agent-written anyway, and surface which foreign types are actually hard to map one at a time. The thing that *is* built, **in v1**, is a **per-symbol binding assistant**: `trellis bind requests.get` or `trellis bind regex::Regex::new`. This is vital to the first project, which is the kind of program that would otherwise have been vibed in pure Python and will call a dozen functions from a few packages; each needs a binding before any lowering that uses it can proceed, and a dozen hand-written bindings is a week of friction at the exact moment the tool's pleasantness is being tested. The assistant is not the bulk generator and has none of its problems, because it is a lowering job with a different context bundle: the daemon fetches that one symbol's metadata (`.pyi` stub, `inspect.signature`, docstring for Python; `cargo doc` JSON for Rust) into `symbol.json` and `docstring.md`, adds one corpus shim example, and runs the normal lowering loop with the normal MCP tools. The output is a `.tr` with prose summarized from the docstring, an agent-proposed signature with effects and capabilities, the shim, and auto-generated contract tests; the human reviews and accepts it like any other lowering. At single-symbol scale the type-mapping, effect-guessing, and refinement problems each have a human in the loop who corrects in one click, which is fine for twelve symbols and not for twelve thousand. No new agent loop, trust path, or file format. The bulk importer (`trellis bind regex`) is never built.
- **Embedding direction in v1 is Soil-hosts-Python.** The `py_module` build target and generated `.pyi` (Python-hosts-Soil) are small but post-v1.
- **Generated bindings** (reading documentation and types to produce FFI interfaces) will be constrained to sources with machine-readable types: C headers, `.pyi`, `.d.ts`. This feature is expected to be the largest bug source.
- **Primitive types:** strings, integers, floats in all the variants the FFI targets need, with the stdlib providing interop. Defining these with the FFI in mind from day one is a known requirement; the specific technical decisions are deferred (§10).
- **Host stub erasure rule (confirmed):** an exported Soil function with effects `io, panic` becomes a host-language function that may raise `SoilError`; capabilities become host-side objects passed in. The generated `.pyi`/`.d.ts` stub documents the effect row. Decided once, applied to every host language. In debug mode `SoilError` is structured and carries the trace, the demoted refinement if any, and the JSON inputs, so host test suites can locate Soil bugs.
- **Foreign values are opaque handles and carry no refinements.** A `Py` handle stays a handle; refinements could be invalidated by foreign mutation, so they attach only to Soil values. Converting a handle to a Soil value is an explicit, visible call (`py_to_soil`) whose cost the caller chooses to pay. Big structures stay in Python and are manipulated by handle-in, handle-out batteries functions; small results cross the boundary and may be refined after conversion. Rust-backed batteries differ: Rust structures are owned by Soil's runtime and are therefore ordinary Soil values, refinable and potentially `total`, with no conversion and no capability (capabilities are about the world, not the implementing language; a Rust function that does no `io` takes none).

### 3.12 Primitive types

There is **no blessed general-purpose integer or string type**. The types that exist are those with unambiguous semantics, so that backends, FFI, and refinements all agree:

- Fixed-width integers: `I64`, `U64`, `I32`, `U32`, etc. Overflow is `panic`, or is refinement-checked away. Integer `/` and `%` are floor division and floor modulus (Python's semantics, matching the reference-implementation language so differential tests agree without adjustment), not C/Rust truncation; a zero divisor is `panic` or refinement-checked away like overflow.
- `BigInt`: arbitrary precision, implemented in the pure core prelude (not foreign-backed, so it remains `total`). SMT reasons about unbounded integers natively.
- `F64`: total-ordered (§3.7).
- `Utf8`: validated UTF-8 bytes, no O(1) indexing. `Bytes` for raw data.

A default literal type exists for ergonomics (`1` is `I64`, `"..."` is `Utf8`), which is the only concession. The pressure to make `I64` and `Utf8` feel general-purpose falls on the prelude, not the language. The Python batteries layer (§5) uses `BigInt` at its boundary because that is what Python numbers are.

### 3.13 Mutation and records

**No mutation.** Every function body is a term; there is no `st` effect and refinements are sound without an aliasing story. In-place update is recovered by the backend where possible (Perceus-style reuse analysis, as in Koka) without the language being aware.

**Records only.** Product types are records with named fields; there are no positional tuples and no named arguments. This is what JSON wants and what agents read best.

### 3.14 Module exports and types

Export lists name functions **and types explicitly**. If an exported function's signature references a type that is not exported, it is an error the agent must repair, with two permitted repairs: export the type, or mark it as intentionally **abstract** (callers may hold values of it but not inspect them). Abstract types are therefore a deliberate feature rather than an accident.

`main` is an ordinary definition with effect `io` and a `World` argument; it has a `.tr`, tests (cram only), and a lock. `soil.toml` names it as the entrypoint and it receives no other special treatment.

---

## 4. Trellis: the specification layer

### 4.1 Trust model

| Role | Owns |
|---|---|
| Human | Prose, tests, escape hatches, export pins, the "accepted" status |
| Agent | Type signatures, refinement annotations, proofs, Soil bodies, module-private helpers |
| Tests | The root of trust for correctness |
| Refinement checker | A ratchet: proves the lowering satisfies the agent's own claims; catches internal inconsistency (off-by-one, missed cases) but does not establish intent |
| Reference implementation | An executable spec and differential-testing oracle |

Key consequences:

- A passing refinement check proves the body satisfies the *agent's* claim, not the human's intent. The IDE shows "typed" and "tested" as separate badges; only "tested" means correct.
- **Tests are written by humans.** If someone wants to truly vibe-code, they use their agent to generate Trellis files; the Trellis layer itself does not generate the gating tests. Agent-generated tests may exist as a clearly labelled differential tier run against the reference implementation, but they do not gate lowering.
- A cheap tightening: auto-generate property tests from refinements (`{v | v > 0}` becomes a QuickCheck property), so refinements are cross-checked against the trust root.
- **Escape hatches** (`unsafe`, `partial`, raw FFI) are written only by the human, in the spec, and every escape hatch in the project is listed in the manifest so the audit view shows where trust is concentrated.

### 4.2 The unit: one file per definition

A definition is a function, a type, or a module header. Each lives in its own file.

Reasoning: the unit of lowering, checking, testing, and locking is the function; file = definition aligns every per-function artifact, makes git diffs map to semantic changes, eliminates merge conflicts between people editing different definitions, makes content addressing trivial (hash the file), and naturally bounds the agent's context.

Costs accepted: external tooling (grep, git log, GitHub review) degrades with many small files, mitigated by a "module view" in the IDE that renders a directory as one virtual file.

**Helpers are not Trellis definitions.** A `helpers.tr` magic file with reduced requirements was considered and rejected: it would be a hole in the trust model that grows until all real logic lives there. The friction of requiring a spec and tests for anything named at the Trellis level is the correct friction; if a helper is not worth specifying, it is not Trellis's business.

Helpers live at the Soil layer in two forms:

1. **Local `let` bindings** inside a lowering, hashed as part of the parent, invisible to Trellis. Covers most cases.
2. **Module-private Soil definitions** (`_private.soil`) when the same helper is needed across several lowerings in a module. Rules:
   - Cannot be called from outside the module; cannot appear in any Trellis signature or refinement.
   - Owned by the lowerings that use them; garbage-collected when no caller references them. The agent does not accumulate a private standard library.
   - The lowering skill prefers local lets and promotes to a private helper only to avoid duplication.
   - Their hashes feed into their callers' `soil_hash`.
   - The global manifest lists them as nodes flagged `soil-private` with no spec hash; the IDE greys them out. A module with many private helpers and few definitions is a smell the IDE flags.

**Promotion path:** the IDE offers "promote to Trellis definition," which stubs a spec file with the inferred signature and existing Soil as the initial lowering, and requires prose and tests before the lock entry is valid. A helper crosses the boundary only by acquiring a spec, never by exemption.

### 4.3 File format

- **Markdown with fenced code blocks.** Prose is freeform Markdown; formal parts (signature, tests, calls) are fenced blocks with designated languages.
- **Signature:** a combination of English prose and Soil types, as the human prefers. Since the agent owns types, the human may write no formal signature at all.
- **Minimal valid definition:** frontmatter naming the definition, one sentence of prose, and one expect test. This is the onboarding story.
- **Filename is identity.** Filenames are static; renaming a file without updating every reference is an error, and the IDE provides refactor-rename. The lock stores the filename; hashes are for invalidation, not identity. No separate name table is needed. Filenames are lowercase snake_case for every file kind, and every file carries YAML frontmatter with a required `name`: a function's equals the filename stem, a type's is PascalCase with the filename its snake_case form (the one formal record of casing, so identity survives case-insensitive filesystems), a module's equals its directory. File kind itself stays inferred, never declared. Frontmatter also admits optional `tags`, drawn from a vocabulary declared in `soil.toml`: non-semantic, user-extensible metadata for IDE graph filtering and CI policy, hashed under `prose_hash` and never an input to lowering.
- **Tests are named blocks**, and a file may contain any number. Names are used by the lock to report failures, by the IDE for click-to-run, and by REPL-to-test promotion to know where to append. **Expect tests are call-arrow lines** (`("1,2") => {"tag": "Ok", "value": [1, 2]}`): the function under test is implicit (file = definition), `with` lines bind fake capabilities with pinned seeds, `panic` is a legal outcome only under a `panic` row, and `xfail` is an info-string modifier.
- **Refinements are written in a prose-friendly, human-readable form** that is still machine-readable. Decided: separate `requires`/`ensures` blocks of `label: predicate` clauses; the signature block stays plain Soil types. Labels are what the lock and checker errors pin; the shared predicate language (also used by type invariants and property tests) is restricted to `total` calls in the decidable fragment. `ensures` on a `Result` uses `result is Ok(v) implies …`.
- **There is no `calls` annotation.** The original "function application annotation" idea is fully subsumed by effect rows and capabilities: a function without a `Net` argument cannot reach the network regardless of what it calls. Call edges are tracked by the lock for the graph view but are not human-written.
- **Values:** JSON, with a block drag-and-drop UI in the IDE for constructing them. Every Trellis type is round-trippable through JSON; this is also the derivation mechanism for `show`/`eq` (§3.7). **Sum types are internally tagged** (`{"tag": "Ok", "value": …}`; nullary variants `{"tag": "None"}`): one uniform shape for every variant, self-describing for hosts and generic tooling. The verbosity is accepted because the IDE's widgets, not humans, write and read these values. Decoding is always type-directed; `show` output is canonical (declaration-order fields, comparator-order maps, shortest round-trip floats) so expect tests compare on the string. Full encoding table in the grammar prototype.
- **Type definitions** are definitions: prose plus a shape (or agent-inferred shape) plus optional invariants expressed as refinements on aliases, which are checked as properties every constructor must preserve. Confirmed; whether invariants are checked on every constructor call or only proven at definition sites is an open detail (§9).
- **The `.tr` grammar** is prototyped in `prototypes/tr-grammar.md` with worked examples in `prototypes/examples/`. The reserved block languages are `soil-sig`, `requires`, `ensures`, `test`, `property`, `cram`, `reference`, `allow`, `soil-type`, `invariant`, `exports`; fenced blocks in any other language are prose. Finalization into `docs/` is pending (§11).

### 4.4 Module structure

- **Folder = module.** `_module.tr` holds module-level prose and the export list.
- **Export list is an explicit list of Trellis files (functions), exactly.** Easy to hash. The export list is the FFI surface; host stubs are generated from it.
- **Export pinning (proposed, unenforced in v1):** because exported signatures are agent-authored, the public API of a module is agent-authored. The proposal is that exported signatures require human approval via a `pinned` flag in the lock, after which the agent cannot change them without a Trellis error. Same principle as "humans write the tests." Enforcement is tooling and is deferred; the flag exists in the lock format from the start.

### 4.5 Tests

Tiers, expressed as a lattice the manifest can describe:

`expect` (cheap, always run) → `property`/`quickcheck`/`fuzz` → `differential against reference` → `proof`.

- The user declares which tier each function must reach; the lowering agent escalates automatically when a cheaper tier is green.
- **Test budget is inferred from the effect row:** pure functions are fuzzed hard; `io` functions get expect/cram tests only. Per-function override available.
- Not every tier is equally English-describable: expect tests and properties translate well from prose; fuzz harness configuration is just code.
- **Reference implementation / validator** in Python, or a **CLI oracle**: a JSON-in/JSON-out executable invoked as a black box and hashed like any oracle (amended for the compiler bootstrap, where `soil0`'s passes are the oracles — see `prototypes/bootstrap-plan.md`). Always attached explicitly by the human, never auto-detected. Differential tests call it through the Python FFI with a `Py` capability in the test harness. serves as a differential-testing oracle, an executable spec the agent reads when prose is ambiguous, and a migration path (an existing Python codebase *is* the reference spec, and Trellis becomes a verified port tool).
- **Test dependencies:** tests may reference the prelude, the reference implementation, the function under test, and **other user definitions only if those definitions are `accepted`**. Acceptance is the human's trust signal, so accepted definitions are legitimate oracles, and this creates test-level edges only to frozen work. The lock tracks the edge; if the oracle's spec changes, dependent tests re-run. Unaccepted definitions cannot be oracles, which prevents oracle cycles among unfinished work.
- **`xfail` marker:** a test may be marked expected-to-fail to document a known limitation. It stays in the spec, is shown in the lock, and blocks `accepted` until resolved, so the disciplined alternative to deleting a test exists.
- **Non-deterministic functions:** `Rand` and `Clock` fakes take seeds and timestamps; the IDE's test widgets expose these as fields so every such test is pinned by construction.
- **Contradiction pre-flight:** before spending any tokens, the daemon checks tests against each other and against the prose for mechanical contradictions (same input, different expected output). This is a distinguished check; subtler contradictions become a distinguished class of `ask_human` question.
- **The kind of test a definition needs depends on what it is.** Every definition has tests, but not the same tests: pure functions get expect and property tests; `io` functions get fake-capability tests; FFI bindings get contract tests (below). The invariant "nothing in the lock is untested" holds throughout.
- **FFI bindings get auto-generated contract tests, not human-written behavioural tests.** A binding's spec is "faithfully cross the boundary," and that is checkable without understanding the library. The daemon derives contract tests from the signature and effect row: a call with an obvious valid input yields `Ok`; an invalid input yields `Err`, not `panic`; returned handles are accepted by the sibling bindings for that type; a loop of calls under the debug runtime's leak checker shows no growth; Soil values survive `soil_to_py`/`py_to_soil` unchanged. The human supplies prose and at most one example input. The lock records tests as `contract` rather than `expect`, so the trust level stays visible. Behavioural correctness of foreign code is not the binding's job; it is caught one level up by the user's own tested functions, which is the same place a hand-written wrapper around a C library would fail.
- **Harvested tests remain optional and strictly better** when the foreign library has examples or a test suite worth translating. Trust level `harvested` vs `contract` is recorded in the lock.
- **Refined bindings need one real test per refinement.** A refinement on a binding (e.g. `{p | valid_regex p} -> total Regex`) does work for downstream proofs that contract tests do not exercise, so each refinement requires one human-written counterexample. Most bindings carry no refinements.
- **`io` testing:** fake capabilities (§3.5) are the primary mode. Cram tests against real side effects are the fallback for the individual user and for the FFI boundary, where fakes stop being possible. A cram transcript runs in a fresh temp dir with `with file` fixtures and may invoke built binary targets or `trellis call <def> <json-args>` (any `io` definition, real `World`-derived capabilities, canonical JSON out) — the real-mode escape for non-`main` functions. Cram never runs inside the lowering sandbox (§4.6). The lock tags a function's `io` tests with their mode so additional modes can be added later without a format change.

### 4.6 Lowering

**Input context per lowering** is a **fixed directory layout** (the context bundle): `spec.md`, `callees/` (signatures only, never bodies), `tests.json`, `examples/` (prelude), `reference.py`, and `previous.soil` if re-lowering. The agent reads it through a tool; humans can inspect it. **Unverified callees are shown as their base type plus a note** that the refinement is demoted, so the agent cannot rely on an unproven claim.

**Lowerings run strictly serially and in disciplined order: a definition cannot lower until all its callees have.** There is no separate signature-inference step; a definition with no lowering has no checkable signature, and callers wait. The dependency tree provides the queue order, the IDE shows what is blocking what, and the IDE supports queuing lowerings while other definitions are still being written. If a human edits a definition that a queued or running lowering depends on, the daemon invalidates that job.

**Fresh context per lowering.** Every lowering is its own session receiving a fresh context built from the current (human-edited) files. More expensive in tokens; guaranteed to be correct and free of stale memory. A separate "memory cache" of agent observations was considered and rejected under the design tenet: anything the lowerer learns that would help next time either belongs in the prose, the tests, the module header, or the prelude fork, or it is not an input. The one exception allowed: a per-lowering log (`f.log`, gitignored) of what was tried and why it failed, for the human to read only; never an input to the next session.

**Lowerer implementation.** Each lowering is one headless agent invocation, which implements the fresh-context rule via the process boundary. The daemon (§4.9) assembles the context bundle and invokes the agent with a prompt to lower it. The daemon is an **MCP server**, and the agent is allow-listed to exactly its tools: `read_context`, `check_types`, `check_refinements`, `run_tests`, `write_soil`, `ask_human`. No raw shell, no filesystem outside the scratch directory. Consequences:

- *The sandbox is the tool allow-list.* `run_tests` runs in the daemon's sandbox with fake capabilities; real-world tests are not a tool the lowerer has.
- *The check loop lives inside one invocation.* The agent writes Soil, checks, reads structured errors, revises, tests. The daemon caps turns, time, and cost rather than implementing retries.
- *Every tool call is logged by the daemon*, which is `f.log` and the cost telemetry with no instrumentation of the agent.

**The question channel ends the invocation.** `ask_human` writes a structured question and exits. The daemon surfaces it in the IDE, the human answers, the daemon writes the answer into the `.tr` prose, and re-invokes the lowerer fresh. The agent never sees an answer that is not already in the spec, and every question/answer pair is a visible diff. A blocking in-context variant may be added later as a cost optimization.

**Provider abstraction.** One interface, `lower(bundle, tools, budget) -> outcome`, with two provider kinds: *agent-CLI providers* (Claude Code headless / Agent SDK, and other headless agent CLIs; nearly free to build; runs on subscription quotas) and a *raw-API provider* (own loop against a model API; full control; pay-as-you-go). The IDE supports multiple agents with user-supplied credentials. Agent-CLI is v1; raw-API is v2. Running the lowerer through Claude Code is a **high-priority feature** because it is how subscription users avoid paying twice, though subscription quota policy for headless use has shifted recently and should be re-verified. Daemon responsibilities from day one: per-invocation timeouts, turn caps, cost caps, and isolated agent home directories.

**Per-function granularity**, manifest/type/lint checked at each step. Failures are local and retryable.

**No widening.** If lowering `f` reveals that `g`'s signature is wrong, the agent does not widen `g`; it is a Trellis error for the human.

**Agent write-back:** the agent writes its inferred signature back into the `.tr` file. How agent-authored parts are marked, and what happens when a human edits them (presumably: they become pinned), is an open format question (§9).

**Agent questions before lowering:** the agent may raise an ambiguity as a blocking state ("should `parse` accept trailing whitespace?"). The human's answer is written back into the prose. Ambiguity becomes spec improvement rather than silent guessing. Implied by the interactive UI decision; not separately confirmed.

**Errors for the agent as a first-class audience:** the LSP has two output modes, human and agent; the agent mode is structured (JSON) with concrete counterexample, violated spec clause, failing test, and a suggested repair class.

**Interactive lowering UI:** the user interacts with lowering errors and reports through a prompt or UI. Rule: anything the human says to the lowerer that changes the outcome must be persisted to the `.tr` file, or the lowerer refuses to act on it.

**Model selection:** user-customizable, with "auto" functionality that chooses when no model is selected. Auto keys on spec size, effect row, presence of refinements, and number of past lowering attempts; escalates on retry. A per-project cost ceiling is planned because auto-with-escalation is exactly the setting where one pathological function burns a budget. All of this is tooling and ships minimal for v1 (one model, fixed retry count).

**Style and the prelude corpus:** see §5.

### 4.7 "Done"

**Done is the user's decision.** The lowering's current status (tests, proofs, demotions) is presented transparently and the user decides when they are happy. The lock status vocabulary is `typed`, `tested`, `verified`, `accepted`; only `accepted` is set by a human. CI policy ("every exported definition must be `accepted`") is a line in `soil.toml`, not a semantic rule.

### 4.8 Generated Soil

- **Checked in.** Agent output is not reproducible, so the Soil is the artifact and the agent is a code generator like `protoc`; regeneration is a deliberate step.
- **Editable by humans.** People will do it anyway. The lock records `hand-edited` and skips re-lowering until the spec changes.
- **Round-trip check:** handled by the spec rather than by diffing prose summaries. The checked type of the generated function must entail the spec type from the manifest (a mechanical subtyping/entailment check). Prose drift is handled separately via hashing (§6.2).

### 4.9 The daemon

Because the IDE is the primary product (§7.1), the lowerer, checker, REPL, and runtime are **services** rather than batch commands. A long-running `trellis` daemon holds incremental compiler state, exposes the LSP, runs lowerings as jobs with the question channel, serves the REPL, and exposes the MCP tool surface used by the lowering agent. The CLI and the IDE are both thin clients; CI support later is "run the daemon headless."

**REPL:** nearly free given incremental compilation, since the daemon already holds every definition compiled. It works at the Trellis level (call a definition with JSON) as well as the Soil level; the former feeds REPL-to-expect-test promotion.

**Debug mode:** debug builds instrument every Trellis definition boundary (not every Soil function) with entry/exit, JSON arguments and return, timing, and which refinement checks fired. One run yields a trace convertible to expect tests by clicking a call, a flame graph at definition granularity, and a repro for any `panic` with exact inputs. Slowness is treated as a bug; the spec-granularity flame graph lets the human see it without reading Soil. Run logs are labelled sandboxed (from lowering) or real (from the human), and the IDE shows which one a proposed test came from.

---

## 5. The prelude as a trusted corpus

The standard library is a **Trellis project whose lock file is trusted**. Each entry is a spec plus a blessed, human-verified lowering. This serves simultaneously as:

- The stdlib.
- The few-shot example corpus that defines the style of Soil the agent writes. Examples are real checked code and cannot drift from the language.
- The mechanism for **forks with different capabilities**: a `no-io` fork is a sandboxed profile; a fork without `Py` is Python-free. Forking the stdlib is forking a repo; no new mechanism.
- The governance mechanism: users contribute by promoting their own definitions; style is a PR-review question rather than a prompt-engineering one.
- The home for shared oracles (§4.5).

**Structure: a pure core plus a foreign-backed batteries layer.** The core prelude is written in Soil and is pure and `total` where possible: `Option`, `Result`, `List`, `Map` (with comparator), `BigInt`, `Utf8`, `Bytes`, JSON encode/decode, the derived-function primitives, the capabilities and their fakes, and `Py`. This is small (a few thousand lines) and is what teaches the agent what Soil looks like. A separate **batteries** layer provides refinement-typed Soil signatures over foreign stdlib functions. **`soil-rs-std` is built first:** Rust-backed functions are runtime-owned, can be `total`, and are refinable, so the batteries layer teaches the agent sound idioms. `soil-py-std` follows in v1 as the library layer, with handle-in/handle-out style, `ffi`/`panic`, and the `Py` capability. Python-backed functions must not be the core or the first batteries, or nothing would be `total` and the corpus would teach "call out" as the idiom. `BigInt` may be Rust-backed (`num-bigint`) and still count as core, since runtime-owned values are ordinary.

**Trust is by full hash** per package, pinned in `soil.toml`: the core prelude and each batteries package are separate hashes, and the lock holds the list of trusted packages. A fork is a different hash; partial trust of a package is not possible. Batteries functions come in two visible flavours: handle-in/handle-out (cheap, unrefined) and handle-in/Soil-out (converts, refinable on the result).

**Corpus retrieval:** for v1 the whole prelude fits in context. At scale, retrieval by signature similarity and effect row is the obvious mechanism, and the lock file is already most of the index. Deferred.

The prelude must be written in the same dialect users are permitted to use (no stdlib-only features), and must use capability-style `io` signatures from the start.

**First prelude definition to write:** `read_file`, since it exercises capabilities, effects, `Result`, and the FFI boundary at once.

---

## 6. Hashing, locking, and incrementality

### 6.1 Content addressing (Unison-style)

Every definition is identified by the hash of its syntax tree with free variables replaced by the hashes of what they refer to. Names are a lookup table on the side. Consequences:

- A function's identity includes the identities of everything it calls. Change `g` and `f` gets a new hash automatically; unchanged functions do not.
- No name-based conflicts; renames are free.
- Check results, test results, and lowering results are cached by hash.
- Composes with Nix, which is also content-addressed.

Because cross-definition mutual recursion is forbidden (§3.8), the definition graph is a tree for functions. Recursive types need a combined cycle hash.

### 6.2 Three-part hashing per definition

| Hash | Covers | On change |
|---|---|---|
| `formal_hash` | Signature, effect row, refinements, import set | Must re-lower or re-verify |
| `test_hash` | Human-written tests; the `reference` attachment | Must re-lower or re-verify (a reference change re-runs differential tests only — the reference is an oracle, not an input to lowering) |
| `prose_hash` | Everything else | Flag `review-suggested`; existing lowering stays valid |

A `prose-stale` state may be auto-cleared when an agent re-reads the prose and confirms the existing Soil still matches. This gives a cheap round-trip check without forced regeneration. Prose is thus "somewhere between hashed exactly and allowed to drift."

### 6.3 Lock file

- **One lock file per definition:** `f.tr` → `f.lock`. Sidecar.
- **Global manifest is derived**, gitignored, regenerated from the sidecars. It is what Nix consumes. It must be a merge of the sidecars, never separately maintained.
- Merge conflicts can only arise when two people edit the same definition, which is a real conflict anyway.
- **Verbosity is fine.** Fields expected per entry: `formal_hash`, `test_hash`, `prose_hash`, `soil_hash` (covering the Soil body plus transitively referenced private helpers), check status, test status and test mode tags, provenance (`agent`, `human-verified`, `hand-edited`, `prelude-fork`), trust level for FFI bindings (`harvested`, `generated`, `declared-only`), language/format version, `pinned` flag, `accepted` flag, escape hatch list, cycle hash for recursive types, and for FFI bindings the symbol hash plus the Nix store path of the package.
- **Language versioning:** the lock records which Soil and Trellis versions a lowering targeted, so upgrades do not invalidate silently.
- **The lock schema is prototyped** in `prototypes/lock-schema.md` with example sidecars in `prototypes/examples/`. Decisions: JSON in the canonical value form (one format, one parser, diff-stable key order); component statuses (`checks` facts plus per-test results) with the `typed`/`tested`/`verified`/`accepted` ladder derived by the IDE, never stored; the lowering record carries `provider` and `model` for audit while costs, retries, and timings stay in the gitignored `f.log`; per-block provenance under `spec.blocks` implements the `@agent` write-back scheme; `oracles` records test-level edges by hash; `accepted` requires no `xfail`/`xpass` results.

### 6.4 Incremental refinement checking and demotion

Refinement types are modular: each function is checked against its own signature plus the *signatures* of callees. Checking is per-function; changing a body without changing its signature invalidates nothing downstream. Changing a signature invalidates callers via content addressing.

**Fallback on proof failure:** the function is demoted to its base ML type (refinements erased from the checker's view), marked `unverified`, and tests remain required. Callers relying on the refinement are checked against the weaker type. Demotion is explicit and local; the agent never silently widens a signature. The IDE shows the unproven chain. This is effectively gradual refinement typing (Lehmann & Tanter).

Blast radius is kept small by design: the checker is for extra safety; tests are what correctness is based on.

---

## 7. Product and IDE

### 7.1 IDE-first

The IDE experience is the primary goal; the first milestone is a tool the author wants to use. Industry adoption within larger teams is the secondary goal (such teams will want a `trellis derive` command that stubs `.tr` files from an existing codebase). CI and headless operation are later concerns, served by running the daemon headless.

- **Platform:** a web app served by Electron, for portability and simplicity, and so the same UI can later be served remotely. The Electron tax is accepted.
- **Editing model:** a text editor with widgets. Widgets are the JSON test blocks (drag-and-drop), REPL-to-test promotion, and the question/answer panel. Everything else is Markdown.
- **Lowering UX:** clicking "lower" starts an interactive session in which the agent can prompt back with questions and error reports. The human answers in the IDE; answers are persisted to the `.tr`.
- **Fix mode:** a debug-mode run produces a trace, flame graph, and proposed test cases; the IDE supports turning any of these into tests.
- **Git:** the IDE never commits on its own. At accept, pin, and rename it suggests a commit with a message; one click to commit, one to decline.
- **Lints:** definition-size and split-suggestion lints (e.g. a 400-line lowering for a one-paragraph spec) come from the Soil checker and surface in the IDE as suggestions, never blocking.
- **Status:** the lock is ugly and never read directly; the IDE renders it. Code review is supported by the IDE rendering "specs changed, tests changed, N re-lowered, M newly accepted."
- **Telemetry:** the lowerer tracks tokens, cost, retries, and provider per lowering from day one; this is what later makes automatic model selection possible.
- **Build targets:** `trellis build` builds whatever `soil.toml` specifies: `binary`, `py_module`, `shared_lib`, `jvm_jar`, etc. One tool, one flag surface.
- **Upgrades:** lowerings record the language version they targeted and are left alone on upgrade until their spec changes; the Soil compiler therefore needs a compatibility policy.
- **Licensing:** the language, daemon, and prelude are open; the IDE is not.
- **Hosted lowering:** eventually possible by design (the daemon is a service), but not a v1 or v2 concern.
- **Naming:** the project, the specification layer, and the IDE are **Trellis**; the target language is **Soil**. The CLI is `trellis` (e.g. `trellis lower`, `trellis bind`). "Trellis" was the placeholder (declarative vibe-coding).

### 7.2 On-disk layout

**Soil lives in its own directory**, any directory with a `soil.toml` at its root. Zip files were rejected as opaque to git; inline embedding in foreign source was rejected as worse. A repo may contain multiple independent Soil roots with no cross-root imports; slices that need to share are one root.

**Spec, generated Soil, and lock sit side by side** per definition. A split into `spec/` and `gen/` was rejected as losing the locality that justified one-file-per-definition.

```
soil/
  soil.toml            -- deps, prelude fork, build targets, CI policy, tag vocabulary
  soil.lock            -- derived global manifest, gitignored
  parser/
    _module.tr         -- module prose, export list
    parse.tr
    parse.soil
    parse.lock
    parse.log          -- gitignored, human-readable lowering log
    tokenize.tr
    tokenize.soil
    tokenize.lock
    _private.soil      -- module-private helpers
  soil/__init__.pyi    -- generated host stub
```

Every definition is three files; the module header and private helpers are the two exceptions; the only non-derived global file is `soil.toml`.

---

## 8. Build system

- **Declare dependencies in TOML; Trellis compiles it to a Nix build script.** Nix is the right model (hermetic, content-addressed) but adopting it wholesale couples users to its ecosystem. The approach mirrors `dream2nix`, `crate2nix`, `poetry2nix`; Trellis is the polyglot roof over them.
- Generate `flake.lock`-style pinned inputs.
- **No "escape to raw Nix" field** in the TOML, or every project will use it and the tool becomes Nix with extra steps.
- A `dev` mode that shells out to native toolchains without Nix is planned for contributor onboarding.
- **Foreign file locking:** Nix hashes lock *provenance* (which bytes); Trellis's lock records *interface* (which shape) via symbol hashes of `.pyi`, `.d.ts`, or C header declarations. v1 accepts Nix's coarse invalidation (any upstream commit invalidates all bindings); the lock format is designed so finer symbol-level invalidation slots in later.
- Buck2 was noted as an alternative if Nix proves too heavy. Deferred.
- **FFI boundary tests:** harvesting tests from the dependency's own suite is the default; generated boundary property tests from declared types are the fallback; each binding records its trust level. A foreign call has effect `ffi` (implying `panic`) and "tests are the only guarantee here"; no attempt to shrink effect rows for foreign code.

---

## 9. Open questions

1. **`.tr` grammar** — *prototyped* in `prototypes/tr-grammar.md` (§4.3) with its follow-up questions resolved: type files carry YAML frontmatter declaring the type's cased name (filenames are snake_case everywhere); property `where` filters use constrained generation, not rejection sampling; the `cram` block is a minimal cram subset (`with file` fixtures, fresh temp dir, literal output, `[n]` exit codes, `trellis call` for real-capability invocation); property-only and cram-only files are valid (`main` and other toplevel functions are typically of that shape); every file carries frontmatter with a required `name` and optional non-semantic `tags` whose vocabulary is declared in `soil.toml`, while file kind stays inferred. Finalization into `docs/` pending.
2. **JSON encoding of Soil values** — *resolved*: internally tagged sums, type-directed decode, canonical `show` output, opaque one-way `"<handle>"`, functions a hard error (grammar prototype §7). `BigInt` is hybrid by range: a JSON number within ±(2^53−1), a string beyond, and decode accepts either — small values stay readable while big ones survive float-only host JSON parsers.
3. **Lock entry schema** — *prototyped* in `prototypes/lock-schema.md` (§6.3), including `.tr` provenance for the vibing tiers and per-block provenance for the write-back scheme (5). Newly open from the prototype: whether module entries participate in `accepted`; a fixed naming scheme for derived tests; whether entries pin the prelude-fork hash or leave it global in `soil.toml`.
4. **Prose-friendly refinement syntax** — *resolved*: labelled `requires`/`ensures` clauses over a shared predicate language (§4.3; grammar prototype §2.3).
5. **Agent write-back markers** — *tentative proposal* (grammar prototype §8): agent-authored blocks carry `@agent` in the info string; a human edit removes the marker, and an unmarked formal block is pinned — the agent may not change it, only `ask_human`.
6. **Type invariants:** checked on every constructor call, or only proven at definition sites.
7. **Naming conventions** for agent-created private helpers.

---

## 10. Deferred decisions and their triggers

| Item | Current stance | Revisit when |
|---|---|---|
| Memory model details (cycle collection strategy, regions) | Runtime is Rust; reference counting with Perceus-style reuse; cycle collection strategy open | When the Rust runtime is built |
| Primitive type details for FFI (UTF-8/16, bigints, floats) | Language supports all variants; stdlib provides interop; agent handles polyglot strings | Strict technical decision when writing the FFI layer |
| Additional `io` testing modes beyond fake capabilities | Cram tests available as fallback; lock tags mode | Fakes prove insufficient |
| Export pin enforcement | Flag exists in lock; nothing enforces | Second user |
| Retry policy, model routing, cost ceiling | Minimal fixed versions | Second user / cost pain |
| Corpus retrieval | Whole prelude in context | Prelude outgrows context |
| Merge tooling for locks | None needed with per-definition sidecars | Second user |
| IDE (graph view, click-to-generate, Q/A buttons, JSON block editor, module view) | Not built | After the CLI loop proves pleasant |
| Nix backend | Not built | After the CLI loop |
| Whole-package binding generator | Not built; per-symbol `trellis bind` is v1 | Likely never; see §3.11 |
| Backends beyond the first | Not built | After the loop works |
| User-declared algebraic effects, effect handlers, concurrency | Not in language; concurrency via FFI to host libraries | Only if glue-language positioning changes |
| User overrides of derived functions | Not allowed; would be a class system | Indefinitely |
| Full dependent types beyond refinements | Not in language | SMT proves insufficient for a meaningful fraction of functions |
| Buck2 as build alternative | Not pursued | Nix proves too heavy |
| Capability names in effect rows | Bare `io` | Never, unless capability arguments prove insufficient |
| Symbol-level invalidation of foreign bindings | Coarse Nix invalidation | After v1 |
| `py_module` build target and Python-hosts-Soil embedding | Not built | After v1 |
| Blocking in-context `ask_human` | Exit-and-reinvoke only | If question round-trips prove too costly |
| Raw-API lowering provider | Agent-CLI providers only | v2 |
| Hosted lowering | Not built | Post-v2 |
| `trellis derive` from an existing codebase | Not built | Second user / team adoption |
| Concurrent lowerings | Strictly serial with a queue | If serial throughput hurts |
| Separate signature-inference step before lowering | Not allowed; disciplined order enforced | If waiting on callees proves too annoying |

---

## 11. Build order

The build order follows the bootstrap plan (`prototypes/bootstrap-plan.md`): the compiler itself is the first Trellis project, self-hosted on a minimal Rust implementation that is kept forever as a differential oracle.

1. **`soil0` + `soil-rt`.** A Rust workspace: the runtime crate (values, reference counting, JSON bridge, C ABI with a trivial `main` wrapper from the first commit) and a minimal Soil implementation — parser, ML + effect-row inference, exhaustiveness, tree-walking interpreter, test runner. No refinements, no termination checker, no codegen, no FFI yet. Every pass is exposed as a JSON-in/JSON-out CLI command (`soil0 parse`, `soil0 infer`, `soil0 run`) — the future differential oracles. Deliberately small; interpreted execution is the engine for the whole bootstrap, and slow is accepted.
2. **The daemon.** Incremental compiler state, LSP, context-bundle assembler, MCP tool surface, lowering jobs with the exit-and-reinvoke question channel, serial queue, agent-CLI provider (Claude Code headless first), REPL endpoint, debug-mode instrumentation, cost telemetry, per-definition locks and the derived manifest. This is where the effort goes; it is smaller than it sounds because the agent CLI supplies the loop.
3. **The pure-core prelude** as the first Trellis code, interpreted on `soil0`.
4. **`soilc`: the compiler as the first Trellis project.** Passes in oracle-ready order — lexer, parser (the mutual-recursion stress test, deliberately early), renamer, type + effect inference, exhaustiveness/patterns, ANF, termination checker, refinement checker (SMT-LIB out, Z3 behind a `Solver` capability), CLIF backend — each differentially tested against the matching `soil0` command and swapped into the daemon at `accepted` (strangler pattern). Refinements and demotion therefore arrive here, as compiler passes, not as a later milestone. Closure: `soilc` interpreted compiles the prelude and itself via the Cranelift driver → `soilc₁`; `soilc₁` compiles the same sources → `soilc₂`; the build requires `soilc₁` ≡ `soilc₂` byte-identical. `soil0` is retained permanently as oracle and debug-mode engine.
5. **FFI, `trellis bind`, minimal IDE.** C-ABI FFI to Rust and Python FFI via embedded CPython, hand-written bindings, sequenced C ABI → Rust batteries → Python embedding → Python batteries; one corpus shim example per FFI; the per-symbol bind assistant; the Electron-served IDE over the daemon (Markdown editor with test-block widgets, graph view, lower button with streaming output and the question panel, REPL, lock rendering), as thin as possible.
6. **The Python-glue project as the second Trellis project.** A program the author would otherwise have had an agent write in pure Python: Rust crates through `soil-rs-std`, a dozen Python functions through `trellis bind`, real work in Soil. The compiler validates the pure core; this validates the FFI/capability/bind half of the pitch and the pleasantness test — writing `.tr` files and reading generated Soil.
7. **Then** trace-to-test fix mode, profiling, build targets, TOML→Nix, raw-API provider, `trellis derive`, headless CI mode.

**Immediate next artifacts:**

- The `.tr` grammar specification — prototyped (`prototypes/tr-grammar.md` plus `prototypes/examples/`, including the JSON value encoding and the refinement prose syntax); to be finalized into `docs/` once the prototype has been exercised.
- The lock entry schema — prototyped (`prototypes/lock-schema.md` plus example `.lock` sidecars); to be finalized into `docs/` with the grammar.
- The prelude's `read_file` as the first real definition (drafted as `prototypes/examples/read_file.tr` with `read_file.lock` and `read_file.soil`).
- A high-level Soil surface syntax: prototyped in `prototypes/soil-syntax.md` (Haskell-style signatures and inline refinements, OCaml-style terms, `decreases` lines, comparison operators as derived-function notation, no imports; guards/`;`/`let?` deliberately absent or deferred), elaborated into a lexical spec, EBNF, and static rules in `prototypes/soil-syntax-spec.md` (OCaml-style match with parenthesized nesting; one connective spelling `and`/`or`/`not` shared by terms and predicates, `implies` predicate-only; shadowing forbidden; `::` namespacing; `..` required in partial record patterns; parameterless `let` bindings with explicit lambdas; a fixed scalar-value-only string escape set; arithmetic operators as notation with overflow/zero-divisor as refinement obligations; `Bool` encoding as JSON booleans). `Result a e` is success-first (§3.3). Full semantics arrive with the Soil core milestone.

---

## 12. Prior art to consult

- **Idris 2 / Agda / Lean:** type-as-spec, hole-driven workflow (Trellis without the LLM); termination checking; total-by-default policy.
- **Hazel:** live holes, typed structure editing, for the IDE.
- **Unison:** content-addressed definitions, incremental typechecking, codebase-as-database; cycle hashing; closest existing thing to the manifest idea.
- **Dafny / Verus:** spec-then-implementation with a checker in the loop.
- **Liquid Haskell / F\*:** refinement types with SMT discharge.
- **Lehmann & Tanter:** gradual refinement types, for the demotion model.
- **Koka:** direct-style surface with effect rows, `div` as an effect, Perceus reference counting; its effect types subsume function-application annotations.
- **OCaml:** polymorphic `compare`, `Map.Make` comparator idiom.
- **Rust:** `derive`, `total_cmp` for floats, debug/release split.
- **dream2nix / crate2nix / poetry2nix:** per-ecosystem TOML-to-Nix precedent.
- **Buck2:** polyglot build alternative.
- **Inform 7, AppleScript, COBOL, Wolfram:** history of natural-language programming. The consistent lesson: prose as *syntax* fails; prose as *spec alongside formal structure* works. Trellis is on the right side of that line.
