# Plan 05 — `soilc`: the compiler as the first Trellis project

*References: docs/bootstrap-plan.md §3–4, design §3.1 (ANF), §3.4
(termination), §6.4 (demotion), §3.11 (Cranelift backend).*

## Goal

The Soil compiler written as a Trellis project — specs, agent lowerings,
locks — running interpreted on `soil0`, differentially tested against it,
and finally compiling itself to native code through Cranelift with a
byte-identical fixed point. Refinement checking and termination checking
enter the system here, as passes.

## Scope

Passes in order; each is a Trellis module of pure functions with the AST
as Trellis type definitions (the JSON schema from `docs/soil0-cli.md` is
the conformance target — `soilc`'s AST types must round-trip it).

1. **Lexer.** Warm-up; oracle `soil0 lex`.
2. **Parser.** One definition with `let rec … and …` locals — the
   mutual-recursion-ban stress test, taken early on purpose. Oracle
   `soil0 parse`. If the ban genuinely fails here, that is a design
   finding to raise, not to code around.
3. **Renamer.** No-shadowing, `::`, `_private` visibility. Canonical
   fresh-name allocation (deterministic counters, no iteration-order
   dependence) — this is where fixed-point determinism is won or lost.
4. **Type + effect inference.** The hardest lowering target in the whole
   plan; split the module aggressively (unify, generalize, rows, operator
   elaboration as separate definitions). Oracle `soil0 infer`.
5. **Exhaustiveness + pattern compilation** (to decision trees). Oracle
   `soil0 check` for the boolean verdicts; pattern compilation is new but
   testable by semantics (compiled and source matches agree — property
   tests through the interpreter).
6. **ANF transformation.** New; tested by properties (well-formedness of
   the output IR; evaluation equivalence via `soil0 run` on both forms).
7. **Termination checker.** New functionality: structural decrease +
   `decreases` measures. Tested by spec (accept/reject corpus). On
   completion, run over the prelude to discharge the interim `div` flags
   from plan 04.
8. **Refinement checker.** Desugars `requires`/`ensures`/inline
   refinements to SMT-LIB text (a pure function, golden-testable); Z3
   runs behind a new `Solver` capability added to the prelude (the `Py`
   pattern: opaque type, fake for tests). Implements demotion (design
   §6.4) and arithmetic obligations (syntax spec §5). The daemon's
   `check_refinements` stub goes live here.
9. **CLIF backend.** Pure pass ANF → CLIF text, plus a small Rust
   **Cranelift driver** crate in the workspace (CLIF in, object files
   out, links `soil-rt`, x86-64 + arm64). Golden CLIF tests plus
   execution equivalence: compiled output vs `soil0 run` on the test
   corpus.

**Strangler integration:** as each pass reaches `accepted`, the daemon
swaps its `soil0` counterpart for the Trellis pass (invoked via
`soil0 run` while interpreted). `soil0` passes are demoted to oracles,
never deleted.

**Self-hosting closure:** interpreted `soilc` compiles the prelude and
itself → `soilc₁`; `soilc₁` compiles the same sources → `soilc₂`; the
build fails unless `soilc₁ ≡ soilc₂` byte-identical. Then the daemon uses
`soilc₁` for execution, keeping `soil0` for differential runs.

## Non-goals

Optimization (beyond what Cranelift gives), Perceus reuse analysis, FFI
codegen (plan 06 extends the backend), JVM/C/direct-x86 backends,
concurrent lowering.

## Testing

- Per pass: differential against the `soil0` CLI oracle over (a) the
  golden corpus from plan 02, (b) the prelude, (c) `soilc`'s own sources
  — the compiler is its own largest test input.
- Property tests per pass (round-trips, well-formedness, evaluation
  equivalence through the interpreter).
- Determinism harness: compile the corpus twice from clean state,
  byte-compare all outputs — run continuously from pass 3 onward, not
  discovered at stage 3.

## Exit criteria

- All passes `accepted`; daemon runs with `soilc` passes strangled in.
- Prelude totality flags discharged by the termination checker;
  refinement demotion live end-to-end (a deliberately unprovable example
  demotes, is visible in the lock, and still runs its check).
- The fixed point holds: `soilc₁ ≡ soilc₂`.
- `examples/csvstats/median.soil`'s refinements actually prove.

## Decision points — resolved 2026-08-22

- **Solver surface:** one-shot —
  `solve : (s : Solver) -> (script : SmtScript) -> io SolveResult` with
  `SolveResult = Sat CounterModel | Unsat | Unknown { reason }`. Each
  obligation is an independent script: trivially fakeable, cacheable by
  script hash. Incremental sessions only if solve time ever hurts.
- **Decision trees are internal.** The public schema covers surface AST
  and ANF; pattern-compilation output is free to change and is tested by
  semantic equivalence against `soil0`, not by goldens.
- **Fixed-point scope: all emitted artifacts** — per-definition CLIF
  text, object files, and the linked binary must be byte-identical, so
  nondeterminism is caught at the layer that caused it. Artifacts may
  contain no timestamps or logs by construction.
