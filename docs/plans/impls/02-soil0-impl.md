# Implementation Plan 02 — `soil0`

*Detailed implementation guide for [`../02-soil0.md`](../02-soil0.md).
That plan states the scope and exit criteria; this one states how the
crate is structured and built, records the implementation decisions
resolved with the user on 2026-08-22, and proposes the remaining
micro-details (§8) and newly surfaced spec gaps (§9) for review before
code is written. References: `docs/soil-syntax-spec.md` (the parser's
contract), tr-grammar §2.3 (predicate language) and §7 (value encoding),
lock-schema §5 (test-result vocabulary), design §3, impl plan 01 (the
runtime this builds on).*

---

## 1. Resolved implementation decisions (2026-08-22)

| Decision | Choice | Rationale |
|---|---|---|
| Inference engine | Mutable union-find for unification; generalization by scanning the environment's free variables (no levels) | No substitution composition to get wrong (the textbook-W error source), and no level bookkeeping (the classic subtle-bug source in the OCaml approach). Env scanning is quadratic — "slow is accepted" (plan 02). Signatures are mandatory on every definition (syntax spec §3.1), so inference is mostly *checking* against declared types; the machinery can stay small. |
| AST spans | Every node is a `Spanned` wrapper record: `{"span": …, "item": <tagged sum>}` | Expressible as one generic Soil record type `Spanned a`, so soilc's Trellis AST types (plan 05) reproduce one wrapper rather than a span field repeated per variant or a node-id side table soilc would have to renumber identically. Follows "as if the AST were already Soil data" (plan 02 scope 1). |
| `soil0 infer` frozen contract | Signature only: inferred type + effect row per definition, type variables canonically renamed. A `--dump-ast` flag additionally emits the elaborated, per-node-typed AST, explicitly **non-contractual** | Behavioral divergence in local types or operator elaboration surfaces in `run`/`test` results, which are already differential oracles; freezing per-node annotations would constrain soilc's inference internals and drag normalization rules into the contract. The dump exists because localizing a soilc divergence without it means bisecting by hand. Signature write-back into `.tr` files is the daemon/agent's job (design §4.6) — `soil0` never sees `.tr`. |
| Program input | A **program manifest** `program.json`: `{"types": "env.json", "defs": [paths…]}` — types by reference, definitions as an ordered file list | One growable place for future options instead of a widening flag surface. The manifest is machine-written: the daemon (plan 03) generates it per invocation; before the daemon exists, `soil0`'s own test harness generates it; handwritten manifests exist only as checked-in test fixtures. Types stay a *reference* so the descriptor format remains exactly the one plan 02 froze (`--types env.json`), usable alone by single-file commands. Inline embedding was rejected as duplicating the contract; a stdin stream was rejected as inventing a multi-definition container the language deliberately lacks. |

None of these are language-observable (the CLI contract itself is
recorded in `docs/contracts/soil0-cli.md`, step 1), so per the plan-01 precedent
they live here and not in `docs/design.md`.

---

## 2. Crate skeleton

A new workspace member beside `soil-rt`; per plan 02's resolved decision,
the crate builds a **library** (linked by the daemon for in-process
checking; API unstable) and a **binary** (the frozen compatibility
contract; oracle and conformance tests always go through it).

```
rust/
  Cargo.toml           -- members = ["soil-rt", "soil0"]
  check.sh             -- extended: soil0 fmt/clippy/test + CLI conformance run
  soil0/
    Cargo.toml         -- lib + [[bin]] soil0
    src/               -- module map in §3
    tests/             -- golden harness + corpus (§6 of ../02-soil0.md)
      golden/          -- accept corpus: <case>/{*.soil, program.json, env.json, expected/*}
      reject/          -- rejection corpus: one case per static rule, keyed by error code
      bundles/         -- test-bundle fixtures for `soil0 test`
```

Dependencies, kept deliberately short (§8.11):

| Crate | Why | Where |
|---|---|---|
| `soil-rt` (path) | values, registry, canonical JSON, `SoilError` | runtime |
| `serde`, `serde_json` | CLI input/output JSON (manifests, bundles, AST, errors) — *Soil values* embedded in outputs still go through `soil-rt`'s canonical encoder, never `serde_json` | runtime |
| `proptest` | parser panic-freedom + determinism properties (§8.12) | dev-only |

Argument parsing is hand-rolled (§8.11): seven subcommands and a handful
of flags do not justify a dependency. All tools come from the repo-root
`shell.nix` (impl plan 01 §8.12); the toolchain stays the single pinned
stable Rust.

---

## 3. Module map

```
src/
  lib.rs         -- pipeline entry points (parse_file, check_program, …); unstable API
  span.rs        -- Span {start, end, line, col}; byte offsets canonical (plan 02)
  diag.rs        -- Diagnostic {code, message, span, file, notes}; JSON rendering (§8.1)
  token.rs       -- token set
  lexer.rs       -- hand-written; owns the escape-set and literal rules
  ast.rs         -- surface AST: Spanned<T>, defs, types, rows, exprs, patterns, predicates
  ast_json.rs    -- AST <-> JSON exactly per docs/contracts/soil0-cli.md
  parser.rs      -- recursive descent per soil-syntax-spec §3
  kernel.rs      -- kernel typedefs (contract §6.1), builtin/derived/numeric name tables (§11)
  manifest.rs    -- program.json + env.json loading (type descriptors with params, §8.6)
  rename.rs      -- scopes, no-shadowing, `::` resolution, `_private` visibility, reference sets
  types.rs       -- semantic types: union-find vars, rows (effect set + optional tail var)
  infer.rs       -- checking/inference, row constraint solving, operator elaboration
  elab.rs        -- the elaborated AST the interpreter consumes; --dump-ast rendering
  exhaust.rs     -- usefulness-based exhaustiveness + redundancy (Maranget)
  interp/
    mod.rs       -- tree-walk evaluator over soil-rt values; trace frames
    env.rs       -- cons-list environments; let-rec knot-tying
    builtins.rs  -- the provisional builtin table (§8.7): numerics, conversions, lists
    caps.rs      -- real capability primitives + fakes (fake_fs, fake_rand, fake_clock)
  testrun.rs     -- test-bundle execution, result vocabulary per lock-schema §5
  cli.rs         -- subcommand dispatch, exit codes, stdout/stderr discipline
  main.rs        -- thin wrapper over cli.rs
```

Dependency direction is a pipeline: `cli → testrun → interp → {elab,
exhaust} → infer → rename → parser → lexer`, everything using `span`,
`diag`, `ast`, `manifest`. Nothing reaches back; `soil-rt` is below all
of it.

---

## 4. The CLI contract: `docs/contracts/soil0-cli.md`

Written and **reviewed by the user before any code** (plan 02 scope 1).
It is the freeze point for everything plan 05's soilc must reproduce and
plan 03's daemon consumes. Required sections:

1. **Conventions.** Invocation shapes, which commands take a source file
   vs a manifest, stdout is exclusively the command's JSON result,
   structured errors on stderr, exit codes (§8.1), and the compact
   single-line output rule (§8.13).
2. **Span and diagnostic schemas** (§8.1, §8.2).
3. **The AST JSON schema**, the largest section: every node kind for
   definitions, types, rows, expressions, patterns, and predicates
   (parsed and retained per plan 02), in tr-grammar §7 conventions —
   internally tagged sums, records, the `Spanned` wrapper. Predicates use
   the tr-grammar §2.3 grammar verbatim.
4. **`env.json`**: the type-environment schema — soil-rt's descriptor
   JSON generalized with type parameters and aliases (§8.6). This is
   where plan 02's "the daemon generates it from `.tr` files later"
   contract is frozen.
5. **`program.json`** (§8.5).
6. **Per-command output schemas**: token list (`lex`), AST (`parse`),
   reference sets (`rename`, §8.4), signatures (`infer`/`check`, §8.3),
   canonical-JSON result (`run`), per-case results (`test`, §8.10).
7. **The builtin table** (§8.7): every native definition's name, Soil
   signature (effects and capabilities included), and semantics —
   including the fakes' determinism guarantees (§8.8).
8. **Non-contractual surfaces**, listed explicitly: `--dump-ast` output,
   the library API, and anything else free to change.

---

## 5. Build order

Steps are sequential; a step is done when its tests pass and `check.sh`
is green. Golden and rejection tests **shell out to the built `soil0`
binary** rather than calling the library — the harness thereby proves
continuously that a downstream consumer can drive everything through the
CLI, which is an exit criterion.

### Step 1 — `docs/contracts/soil0-cli.md`

The contract document (§4), reviewed before code. Micro-pins §8.1–§8.10
land in it; user approval of this plan plus that document unblocks
everything below.

### Step 2 — skeleton, spans, diagnostics, CLI shell

Workspace member, `span.rs`, `diag.rs`, `cli.rs` with all seven
subcommands wired to stubs, exit-code discipline, `check.sh` extension.
Tests: diagnostic JSON golden, exit codes.

### Step 3 — lexer and `soil0 lex`

The token set of syntax-spec §1: keywords (effect names are *not*
keywords — reserved in type position only, handled by the parser),
identifier classes (`ident`, `private-ident`, `TypeName`), operators
including `..` and `::` under maximal munch, `--` comments, integer
literals with underscores, float literals, and the string escape set —
exactly `\"` `\\` `\n` `\r` `\t` `\u{1–6 hex}`, rejecting surrogates,
values above U+10FFFF, and any other escape. Every token carries a span.

Tests: golden token streams; one rejection per lexical rule (bad escape,
surrogate, unterminated string, stray character).

### Step 4 — parser, AST JSON, and `soil0 parse`

Hand-written recursive descent per syntax-spec §3, with the named
disambiguations:

- **`and`**: after `and`, the two-token lookahead `defname "="` continues
  the enclosing `let`'s bindings; anything else parses `and` as the
  boolean connective (spec §3.3).
- **Non-associative comparisons**: `a < b < c` is a parse error with its
  own code (spec §4).
- **Match arm extent**: an arm body extends maximally; `|` attaches to
  the innermost open match. The "parenthesize non-tail nested matches"
  rule is a *consequence* of this attachment, not a separate check.
- **Row vs return type**: after `->`, a lowercase ident followed by
  another type is a row variable; alone it is the return type (spec
  §3.2).
- **Named domain**: `(` then `ident :` begins a named parameter type;
  otherwise a parenthesized type.
- **Refinements**: `{` in type position opens a refinement; its predicate
  is parsed with the tr-grammar §2.3 grammar and retained in the AST.
- `decreases` lines between signature and equation; signature/equation
  `defname` agreement is a parse-level error.

`ast_json.rs` implements §4.3's schema in both directions (`parse` emits;
`rename`/`infer` accept AST JSON per plan 02's CLI list).

Tests: golden ASTs for the syntax-spec §7 worked examples and both
checked-in `.soil` examples; parse-level rejection cases (nonassoc
comparison, missing `..` is *not* here — it is a checker rule — but
malformed record patterns, bad `decreases` placement, name mismatch
are); proptest properties — arbitrary byte input never panics, parsing
is deterministic, spans are well-formed and properly nested (§8.12).

### Step 5 — renamer and `soil0 rename`

Operates on a manifest (all defs). Scope stack per definition:
equation parameters, `let`/`fun` binders, pattern binders, over an outer
scope of manifest definition names and builtins.

- **No shadowing** (spec §5.1): any binding of a name already in scope —
  including a parameter colliding with a manifest definition or builtin —
  is an error. `_` binds nothing, is exempt, and is rejected in
  `let rec`.
- **`::` resolution**: `module::def` resolves iff a manifest definition
  `def` lives in directory `module`; `Type::derived` resolves against
  the env's types and the fixed derived-function set (`eq`, `show`,
  `compare`, `hash`); derivability itself is checked later (infer),
  existence here.
- **`_private` visibility**: `private-ident` definitions are legal only
  in files named `_private.soil`, and callable only from definitions
  whose file shares that directory (design §4.2). Module identity is the
  definition file's parent directory (§8.5).
- **Success output**: per-definition **reference sets** — the definitions,
  types, builtins, and private helpers it references (§8.4). This is the
  computed import set design §6.1 and the lock's call-edge data need, so
  it is load-bearing for plan 03, not debug output.

Tests: golden reference sets; rejections for every scoping rule
(shadowing in each binder position, `_` in `let rec`, private call
across modules, private def outside `_private.soil`, unknown name,
unknown qualification).

### Step 6 — types, effect rows, inference, elaboration; `soil0 infer`

The largest step. Semantic types in `types.rs`: scalars and `List`/`Map`
as builtin constructors, named records/sums/aliases from the env
(instantiated at their parameters), arrows carrying a row, unification
variables as union-find indices. Rows are an effect set (`div`, `panic`,
`io`, `ffi`) plus at most one tail variable; `ffi ⇒ panic` is normalized
at construction.

Checking discipline: every definition carries a declared signature, so
the checker verifies the body against it, using callees' *declared*
signatures (already checked — manifest order is callee-first, §8.5).
Within a body: HM inference with union-find; generalization only at
`let` (scan env free vars, no levels); no polymorphic recursion.

- **Effect rows**: collect subset constraints (callee row ⊆ context row)
  during inference; solve by fixpoint propagation after unification.
  Row-polymorphic higher-order signatures (`map : (a -> e b) -> List a
  -> e (List b)` style) instantiate their tail variables per call site,
  so `map` over a `total` function stays `total` (design §3.3).
- **Recursion**: any self-recursive definition or `let rec` group
  conservatively acquires `div` (plan 02 scope 4); a declared row
  lacking `div` yields `termination: "unverified"` in the check facts
  rather than an error (soil0-cli §8.5, approved 2026-08-22; `io`/`ffi`
  deficits remain hard errors). `decreases` lines are parsed and
  ignored.
- **Operator elaboration** (spec §5.4): at the zonked monomorphic type,
  `==`/`!=` → `T::eq`, comparisons → `T::compare`, arithmetic and unary
  minus → per-type numeric primitives, `and`/`or` → short-circuit `Bool`
  builtins, `not` → `Bool` builtin. An operator whose operand type is
  still a variable or is polymorphic is an error ("operators are
  notation, not overloading"). Derivability (no closures in the type) is
  checked here for `==`/`!=`/comparisons and explicit `Type::derived`
  uses.
- **Literals**: type = the immediately enclosing annotation if present,
  else the default (`I64`, `F64`, `Utf8`); integer literals are
  range-checked against that width at check time (§8.14).
- **Field access**: resolved against the zonked record type; if still a
  variable, an "annotation needed" error (no row-polymorphic records,
  plan 02 scope 4).
- **Constructors**: exactly-one-spelling resolution (syntax-spec §5.9):
  a bare constructor resolves iff its variant name is unique among the
  env's sum types; on a collision the qualified `Type::Ctor` form is
  required, and qualifying a unique constructor is an error. Rejection
  tests cover all three failure modes (ambiguous bare, unknown
  qualified, needlessly qualified).
- **Refinements**: erased to their base types before checking (parsed,
  retained, otherwise ignored — plan 02 scope 4).

Output (§8.3): per-definition inferred signature — structured type JSON
plus effect row, type variables canonically renamed in order of first
appearance. `--dump-ast` additionally emits the elaborated AST with
per-node types, marked non-contractual.

Tests: golden signatures and check-facts for all examples
(undeclared-`div` recursion and panic obligations are golden *facts*,
not rejections — soil0-cli §8.5); rejections for `io`/`ffi` subsumption
violations, operator-at-polymorphic-type, literal out of range,
arity/type mismatches, non-derivable `==`; a dedicated HOF
row-propagation suite (`sum_lengths`, map-over-total,
map-over-panicking).

### Step 7 — exhaustiveness, redundancy; `soil0 check`

Maranget-style usefulness over the pattern matrix: constructor sets from
the env's sum types, records with the mandatory-`..` rule (a partial
record pattern without `..` is an error, spec §3.4), literal patterns
useful only under a wildcard/binder default. Non-exhaustive matches
report a witness pattern in the diagnostic; redundant arms are errors
per arm.

`soil0 check` is the full static pipeline — parse → rename → infer →
exhaustiveness — over a manifest; its success output is `infer`'s
(§8.13), its failure output the combined diagnostics.

Tests: exhaustiveness/redundancy golden + rejection cases (missing
variant, missing `..`, redundant arm, literal match without default);
this completes the "every static rule in syntax-spec §5" corpus
obligation together with steps 5–6 (checklist in step 10).

### Step 8 — interpreter

Strict tree-walk over the elaborated AST, producing `soil-rt` values.

- Environments are persistent cons-lists (`env.rs`); `let rec` closures
  tie the knot via `Rc<RefCell<…>>` backpatching.
- Closures are `soil_rt::Closure::native` wrapping `Rc`'d elaborated AST
  plus captured env — the single invocation path impl plan 01 §1
  committed to.
- **Arithmetic**: hand-rolled floor division and floor modulus on
  integers (Python semantics; note Rust's `div_euclid` is *not* floor
  for negative divisors — §7 risks); checked overflow everywhere;
  overflow and zero divisors raise `SoilError` (`Overflow`,
  `DivideByZero`) — always-on runtime checks matching debug-mode
  semantics (plan 02 scope 6). `F64` operators are IEEE 754. `BigInt`
  primitives are total.
- **Ground-instance registration**: env types are parameterized, but
  `soil-rt`'s registry holds ground descriptors; the interpreter
  instantiates and registers ground instances on demand behind a cache
  keyed by the instantiated shape (§8.6). Every runtime value thereby
  carries a real `TypeId`, so `show`/`eq`/decode work unchanged.
- **Trace frames**: entry to every definition pushes a `TraceFrame`;
  raised `SoilError`s carry the stack (the design §3.11 debug payload).
- **Capabilities** (`caps.rs`): opaque values wrapping native handles.
  Real primitives (`fs_read_bytes`, clock, rand, env, proc) and the fake
  constructors (`fake_fs`, seeded `fake_rand`, `fake_clock`) per the
  builtin table (§8.7, §8.8).

Tests: expect-style unit tests over the interpreter for each expression
form; the arithmetic golden table (all sign combinations of `/` and `%`,
overflow edges, `I64::MIN / -1`); fake-capability behavior (seeded rand
reproducibility, fake_fs hit/miss).

### Step 9 — `soil0 run` and `soil0 test`

- `run --entry name --args <json-array>`: decodes each non-capability
  parameter from the array in order (type-directed, via `soil-rt`
  decode; entry parameters must be ground types), injects real
  `World`-derived capabilities for capability-typed parameters
  (mirroring `trellis call`, tr-grammar §3.5 — §8.9), prints the
  result's canonical JSON on stdout; a runtime `SoilError` is reported
  as a structured error on stderr with nonzero exit.
- `test <bundle.json>`: executes a daemon-assembled bundle — per case:
  construct `with`-bound fakes, decode args, call the definition under
  test, compare outcomes (§8.10) — and reports per-case results in the
  lock's vocabulary: `pass` | `fail` | `xfail` | `xpass` (lock-schema
  §5). `panic` expectations are legal outcomes matching any raised
  `SoilError`.

Tests: bundle fixtures mirroring the `.tr` examples' test blocks
(`read_file.tr`'s fake-fs cases, `median.tr`'s expect cases), xfail/xpass
reporting, capability injection.

### Step 6b — typed holes (contract v1.1, adopted 2026-08-23)

`?name` per syntax-spec §3.3/§5.11 and design §3.16: lexer hole token,
`Expr::Hole` (contract §4.3), checker support (a hole is a fresh
variable; its zonked goal type reported in `checks.holes` with canonical
renaming; duplicate hole names error), and the `unfilled-hole` refusal
rule for `run`/`test` (lands with step 9). Tests: goal-type reporting,
duplicate names, hole-free outputs byte-identical to v1.

### Step 10 — corpus completion, examples, conformance pass

- The rejection corpus is audited against a checklist enumerating every
  static rule: syntax-spec §5.1–§5.8 plus every lexical/parse rule from
  steps 3–4 — one case per rule, keyed by error code.
- `examples/read_file.soil` checks and runs against an env defining
  `Path`, `FsError`, and a fake fs; `examples/csvstats/median.soil`
  checks and runs with stub callees — `len`/`nth` as thin Soil wrappers
  over the list builtins and `sort_by` as a real Soil insertion sort
  (~10 lines), which doubles as the integration test for recursion,
  closures, and comparison elaboration.
- Full proptest run; `docs/contracts/soil0-cli.md` conformance re-review command by
  command; rustdoc pass on the library surface (marked unstable) and the
  builtin table.

### Step 11 — canonical printer (`soil0 print`, adopted 2026-08-23)

Before plan 03 computes the first `soil_hash` (design §4.8/§6.1): write
the canonical-form section into soil-syntax-spec (user-reviewed — it is
spec), implement the printer as a pass with `parse → print → parse`
idempotence as a conformance test (proptest over the golden corpus), and
wire `soil0 print <file.soil>` (contract v1.1's reserved command). The
alpha-normalization for hashing (local binders as indices) is specified
with it; the daemon's `write_soil` canonicalizes through this printer.

---

## 6. Exit-criteria traceability

| Plan 02 exit criterion | Where it lands here |
|---|---|
| `docs/contracts/soil0-cli.md` exists; every command conforms | Step 1 (written, reviewed), step 10 (conformance pass) |
| Both checked-in `.soil` examples check and run correctly | Steps 8–10 (interpreter, run, stubs) |
| Rejection corpus covers every syntax-spec §5 static rule | Steps 3–7 accumulate; step 10 audits against the checklist |
| Plan 03 can drive lex→check→test through the CLI alone | The golden/rejection/bundle harness shells out to the binary exclusively (§5 preamble) |

---

## 7. Risks and checks

- **Floor vs Euclidean vs truncating division.** Rust's `/` truncates
  and `div_euclid` differs from floor for negative divisors
  (`7.div_euclid(-2) == -3` but Python `7 // -2 == -4`). The arithmetic
  golden table (step 8) covers all sign combinations and is generated
  once from the reference semantics (Python) rather than written from
  memory.
- **Row-constraint solving subtleties.** HOF effect propagation is where
  hand-rolled row systems typically go wrong (a dropped tail variable
  silently widens or narrows a row). The dedicated HOF suite in step 6
  exists for this; any fix adds a case.
- **The frozen CLI drifting from the evolving library.** Prevented
  structurally: no test calls the library except the interpreter/infer
  unit tests; everything observable goes through the binary.
- **No-shadowing strictness surprising the corpus.** Parameters
  colliding with builtin or definition names will be the most common
  agent error; the diagnostic must name the colliding binding site.
  Rejection tests pin the message shape.
- **Ground-instance cache correctness.** Two structurally equal
  instantiations must map to one `TypeId` (or `eq`/`show` split
  behavior); keyed by fully-zonked shape, with a property test.
- **`serde_json` recursion limits** on deep AST JSON: acceptable —
  depth-limited inputs fail loudly with a structured error, and the
  parser side (source text) is iterative-or-depth-checked like plan 01's
  decode walk.

---

## 8. Micro-pins — approved 2026-08-22

Per the overview's rule that new choices go to the user, these were
presented as proposals and **approved by the user on 2026-08-22**. They
are pinned; each lands in `docs/contracts/soil0-cli.md` (step 1), and reopening
one is a new decision point. The one language-observable item (§8.14,
literal range checking) is also recorded in soil-syntax-spec §1; the
rest are CLI- or crate-internal and live here.

1. **Diagnostics and exit codes.** stderr carries
   `{"errors": [{"code", "message", "file", "span", "notes": […]}]}`;
   `code` is a stable kebab-case string (`shadowing`, `redundant-arm`,
   `nonassoc-comparison`, `missing-record-rest`, …), one per rule — the
   rejection corpus keys on it. Exit codes: `0` success, `1` the input
   violates the spec (any diagnostic), `2` usage or internal error
   (malformed manifest/bundle, I/O failure, bug).
2. **Span JSON**: `{"start", "end", "line", "col"}` — UTF-8 byte offsets
   canonical (plan 02 resolved), `line`/`col` 1-based and derived from
   `start`, display-only.
3. **Signature output** (`infer`/`check`): structured type JSON (the AST
   type encoding), not a pretty string; effect rows as
   `{"effects": […], "var": name|null}` with effects in the fixed order
   `div, panic, io, ffi`; type and row variables renamed `a, b, c, …` in
   one sequence by order of first appearance. A pretty-printed string may
   accompany it but is non-contractual. *Approved extension
   (2026-08-22, with `docs/contracts/soil0-cli.md` §8.5):* the output also carries
   per-definition `checks` facts — `termination` in the lock-schema §4
   vocabulary plus `panic_obligations` — because `div`/`panic` deficits
   are recorded facts, not errors, while `io`/`ffi` subsumption remains
   enforced.
4. **`rename` output**: per-definition reference sets
   `{"defs", "types", "builtins", "privates"}`, each sorted
   lexicographically — the computed import set for the lock and the
   graph view (design §6.1).
5. **Manifest semantics**: `defs` must be ordered callee-before-caller
   (the daemon owns the dependency tree; disciplined order, design
   §4.6); a forward reference is a `2`-class error, not a checker
   diagnostic. Module identity for `_private` visibility and
   `module::def` is the definition file's parent directory name.
6. **`env.json` generalizes descriptor JSON.** soil-rt descriptors are
   ground, but Soil type definitions are parameterized (`Result a e`)
   and include aliases (`Path = Utf8`), which `TypeBody` cannot express.
   `env.json` therefore extends the impl-plan-01 §4.7 format with
   `"params": [tyvars…]`, a `Var` shape, and an `Alias` body. The
   checker consumes it directly; the interpreter registers **ground
   instances on demand** (cache keyed by instantiated shape) so runtime
   values carry real `TypeId`s and `soil-rt` stays unchanged. Aliases
   are checker-level and erased before registration. *Revised
   2026-08-22 (with `docs/contracts/soil0-cli.md` §6/§13.3):* field, payload, and
   alias types use the semantic type encoding (`SigType`) rather than a
   generalized descriptor `Shape` — one type encoding for signatures
   and environments; `SArrow` rejected until a prelude type needs a
   function field; soil0 lowers `SigType` to runtime descriptor shapes
   at ground-instance registration.
7. **A provisional builtin table**, enumerated in full in
   `docs/contracts/soil0-cli.md`: capability primitives and their fakes;
   `utf8_decode` and sibling conversions; per-type numeric primitives
   (the elaboration targets, including total `BigInt` arithmetic); and a
   minimal set of **list primitives** (`list_len`, `list_nth`,
   `list_empty`, `list_append`, …) — necessary because the grammar has
   no list literals or list patterns, so nothing list-shaped is writable
   in Soil without them. *Resolved 2026-08-22 (was §9.3): these are
   prelude surface* — native-backed prelude definitions like the fakes
   (plan 04 scope 2–3); the names frozen in `docs/contracts/soil0-cli.md` are the
   prelude names.
8. **Fake determinism**: `fake_rand seed` is SplitMix64 (fixed,
   documented — outcomes are pinned in expect tests, so the algorithm is
   observable and must never drift); `fake_clock t` returns the constant
   `t` on every call; `fake_fs files` holds an in-memory
   `Map Utf8 Utf8`, `fs_read_bytes` returning the UTF-8 bytes of the
   stored string, misses returning the env's `FsError` not-found
   variant.
9. **`run` capability injection**: parameters whose type is a capability
   are injected from a real `World` in order; remaining parameters
   decode from `--args` positionally; entry parameters must be ground
   types. Mirrors `trellis call` (tr-grammar §3.5) so plan 03 reuses the
   semantics.
10. **Test-outcome comparison**: the actual result is canonically
    encoded and byte-compared against the expected value's canonical
    form (expected JSON is decoded type-directedly and re-encoded, so
    accepted non-canonical spellings normalize — same normalization as
    plan 01's fixtures). `panic` expectations match any `SoilError`
    raised by the call; bundle-level problems (undecodable args, unknown
    definition) are `2`-class invocation errors, never `fail` results.
11. **Dependency floor**: `soil-rt`, `serde`, `serde_json`, dev-only
    `proptest`; hand-rolled argument parsing (no clap).
12. **Fuzzing via proptest generators on the pinned stable toolchain**,
    not cargo-fuzz: cargo-fuzz wants a nightly toolchain and sanitizer
    plumbing, and `shell.nix` pins exactly one stable Rust (impl plan 01
    §8.12). Properties: arbitrary bytes never panic the lexer/parser;
    generated token sequences never panic the parser; parse is
    deterministic. Revisit if coverage-guided fuzzing proves necessary.
13. **Output discipline**: every command's stdout is one compact
    (no-whitespace) single-line JSON document with field order fixed by
    the schema doc; golden tests byte-compare. Success output of `check`
    equals `infer`'s.
14. **Integer literals are range-checked** against their annotated-or-
    default width at check time. Consequence: `I64::MIN` is not writable
    as a literal (`9223372036854775808` overflows before negation is
    applied); accepted — a prelude constant covers it later, and the
    same wart exists in C and Rust's literal grammars. *Amended
    2026-08-23:* "or-default" is contextual, as design §3.6's "as in
    Rust" intends — a literal adopts the type inference demands and
    defaults to `I64` only when unconstrained; the fixed-at-lex reading
    broke the normative `gcd` example (`b == 0` with `b : U64`).
    Recorded in syntax-spec §1.
15. **`lex` output**: a JSON array of `Spanned` tokens, each an
    internally tagged sum (`{"tag": "Ident", "value": {"name": …}}`,
    literal tokens carrying their decoded value).

---

## 9. Spec gaps surfaced by this plan — all resolved 2026-08-22

Raised per the overview's rule ("when implementation reveals a spec
contradiction or gap, stop and raise it"); all three were resolved with
the user on 2026-08-22 and propagated to the spec docs as noted below.

1. **Constructor-name ambiguity.** *Resolved 2026-08-22: `Type::Ctor`
   qualification with the exactly-one-spelling rule* — bare iff the
   variant name is unique among the types in scope (bare then being the
   only legal form); qualified required on a collision; qualifying a
   unique constructor is an error. Recorded in soil-syntax-spec
   §3.3–§3.4 and §5.9, tr-grammar §2.3 (`is` tests), and design §3.15.
   Affects steps 4 (grammar: `ctor-name`, qualified production), 5–6
   (resolution), and the rejection corpus.
2. **`fake_fs` argument shape.** *Resolved 2026-08-22: the example was
   wrong.* `examples/read_file.tr` wrote a JSON object for a
   `Map Utf8 Utf8`; tr-grammar §7 encodes maps as arrays of
   `{"key", "value"}` pairs, and there is no object convenience form —
   one canonical shape. The example (and the design §3.5 illustration)
   now use the array form; soil0's test bundles do the same.
3. **The provisional list primitives** (§8.7). *Resolved 2026-08-22:
   they are prelude surface* — native-backed prelude definitions, names
   frozen in `docs/contracts/soil0-cli.md` and adopted by plan 04 (its scope 2
   now records this).
