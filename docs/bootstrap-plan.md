# Bootstrapping Plan: Rust Interpreter → Self-Hosted Compiler

*Status: prototype plan. The Soil compiler (`soilc`) is written as a Trellis
project — the first Trellis project — bootstrapped on a minimal Rust
implementation (`soil0`) that is never thrown away. Decisions folded in from
review: slow interpreted bootstrapping is accepted; the reference-oracle
rule is amended to allow CLI oracles (design §4.5); the compiler is the
first project with the Python-glue project second; the backend emits
Cranelift CLIF, not C and not direct x86.*

---

## 0. Premise

A compiler is the ideal Trellis dogfood: every pass is a pure, total
function over trees, which is exactly what the JSON value model, expect
tables, property tests, and differential testing handle best. The AST is a
family of Trellis type definitions, so serialization (`show`/`parse`) and
golden tests come for free. And the stage-0 implementation is not scaffold
but permanent infrastructure: an independent implementation to
differentially test the real compiler against, forever.

The bias to keep in view: compiler code is the *easiest* kind of Trellis
code (pure, total, no FFI, no capabilities). It validates the language core
and the lowering workflow; it validates nothing about bindings,
capabilities, or `trellis bind`. Hence the Python-glue project stays as the
second project.

## 1. Stage 0 — `soil0` + `soil-rt` (hand-written Rust)

One workspace, two crates:

- **`soil-rt`** — the runtime, never bootstrapped away (design §3.10):
  values, reference counting, the JSON bridge, C ABI from the first commit,
  later pyo3.
- **`soil0`** — parser, ML type + effect-row inference, exhaustiveness,
  tree-walking interpreter, test runner. **No refinements, no termination
  checker, no codegen.** Deliberately the size of a class project.

The load-bearing requirement: **every pass is a JSON-in/JSON-out CLI
command** — `soil0 lex`, `soil0 parse`, `soil0 infer`, `soil0 run`. These
are the future differential oracles. Slowness is explicitly fine; the whole
bootstrap runs interpreted.

## 2. Stage 1 — Trellis toolchain on the interpreter

Design §11 milestones 2–3 unchanged in substance: daemon (incremental
state, hashing/locks, context bundles, MCP tool surface, lowering jobs,
agent-CLI provider), then the pure-core prelude as the first Trellis code.
Soil execution is `soil0` interpretation throughout. At the end of stage 1,
Trellis is a working language whose execution engine happens to be an
interpreter.

## 3. Stage 2 — `soilc`: the compiler as the first Trellis project

Each pass is a module of `.tr` specs, agent-lowered, running interpreted.
Pass order, chosen so each has its oracle before it is written:

1. **Lexer** — warm-up; calibrates the spec-and-lower workflow.
2. **Parser** — deliberately early: the predicted worst case for the
   mutual-recursion ban (design §3.8), written as one definition with
   `let rec … and …` locals. Stress-tests the language design while it is
   still cheap to change.
3. **Renamer / scope checker** — no-shadowing, `::` qualification.
4. **Type + effect inference.**
5. **Exhaustiveness + pattern compilation.**
6. **ANF lowering** (the mid-end, design §3.1).
7. **Termination checker** — new functionality, no `soil0` counterpart;
   tested by spec only.
8. **Refinement checker** — emits SMT-LIB text as a pure function; Z3 runs
   behind a new `Solver` capability (the `Py` pattern). Refinements and
   demotion therefore arrive as compiler passes, not a separate milestone.
9. **CLIF backend** — see stage 3.

**Strangler pattern:** as each pass reaches `accepted`, the daemon swaps
its `soil0` pass for the Trellis one (shelling to `soil0 run` while
interpreted). The `soil0` passes are demoted to oracles, never deleted.

**Testing:** every pass gets JSON→JSON expect tables, properties (e.g.
`parse` after `print` is identity), and differential tests against the
matching `soil0` command via CLI oracle. Passes 7–8 are spec-only.

## 4. Stage 3 — closing the loop

The backend is a pure Trellis pass **ANF → CLIF text** (golden-testable
like every other pass), plus a small Rust driver in the workspace that
feeds Cranelift for isel/regalloc/object emission — x86-64 and arm64, no C
toolchain.

Why not direct x86: instruction selection, register allocation, and ELF
emission are the largest and least testable chunk of a native backend,
platform-locked, with zero free optimization. ANF is already shaped like
portable three-address code; emitting a textual SSA IR outsources exactly
the bad part. Direct emission remains a deferred independence move, not a
foreclosed one.

Closure:

1. `soilc` (interpreted on `soil0`) compiles the prelude and itself →
   native `soilc₁`.
2. `soilc₁` compiles the same sources → `soilc₂`.
3. **Fixed point: `soilc₁` and `soilc₂` are byte-identical.** Soil is
   unusually well-positioned for this — no mutation, canonical JSON,
   comparator-ordered maps, content addressing — so determinism is the
   default, but the fixed-point test is what enforces it.

## 5. Permanent division of labor

| Stays Rust forever | Becomes Trellis |
|---|---|
| `soil-rt` (runtime, C ABI, pyo3) | all compiler passes |
| the Cranelift driver | SMT-LIB generation |
| the daemon's process/IO shell | pure daemon logic later (hashing, lock manipulation) |
| Z3 itself | |
| `soil0` — permanent differential oracle and debug-mode engine | |

## 6. Mapping to the build order

Design §11 is reordered (recorded there): `soil0`+`soil-rt`, daemon,
prelude, **`soilc` as first project** (through self-hosting closure), then
`trellis bind` + FFI batteries + minimal IDE, then the **Python-glue
project as second project** — it validates the FFI/capability/bind half of
the pitch that the compiler cannot touch.

## 7. Remaining risks

- **Biased dogfood:** the compiler proves the pure core, not the FFI story;
  mitigated only by actually doing the second project.
- **Fixed-point determinism:** any iteration-order or fresh-name
  nondeterminism in lowered passes breaks stage 3; the renamer must
  allocate names canonically.
- **Type-inference pass size:** the hardest single lowering target in the
  plan; if agent lowering strains anywhere, it is there — split the module
  aggressively.
