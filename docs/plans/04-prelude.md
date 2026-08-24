# Plan 04 — the pure-core prelude

*References: design §5 (the prelude as a trusted corpus), §3.5
(capabilities), examples/read_file.tr.*

## Goal

The first Trellis code: the pure core prelude, written as `.tr` specs and
agent-lowered through the real daemon pipeline, human-reviewed to
`accepted`. It is simultaneously the stdlib, the few-shot corpus that
defines the agent's Soil style, and the first honest test of the whole
loop. Interpreted on `soil0`; small (a few thousand lines of Soil).

## Scope

1. **`read_file` first** (design §5): capabilities, effects, `Result`, and
   the runtime boundary in one definition. Promote
   `examples/read_file.tr` into the real prelude root and lower it for
   real; reconcile any drift back into `examples/`.
2. **Core types and functions**, roughly in dependency order: `Bool`
   (with its JSON special case), `Option`, `Result`, `List` (map, filter,
   fold, len, nth, append, reverse, sort_by…), `Utf8` (split, trim,
   parse-number…), `Bytes`, `BigInt`, `Map` with explicit comparator
   (the `Map.Make`-as-function idiom, design §3.6), JSON encode/decode
   surface (thin wrappers over the runtime). The list *primitives*
   (`list_len`, `list_nth`, `list_empty`, `list_append`, …) are
   native-backed prelude definitions like the fakes — Soil has no list
   literals or patterns, so nothing list-shaped is writable without them
   — with names frozen in `docs/contracts/soil0-cli.md` (impl plan 02 §8.7,
   resolved 2026-08-22); the rest of the `List` functions are written in
   Soil on top of them.
3. **Capabilities and fakes.** The capability types (`Fs`, `Net`, `Clock`,
   `Env`, `Proc`, `Rand`) as opaque types with their `World` derivations,
   and the fake constructors with pinned seeds/timestamps — signatures in
   Trellis, backed by `soil0`/`soil-rt` native primitives.
4. **Corpus duty.** Every lowering is reviewed as a *style exemplar*, not
   just for correctness: idiomatic match shapes, naming, use of local
   `let`s vs private helpers. Style disagreements are settled by PR-style
   review with the user and become the corpus.
5. **Trust and packaging.** The prelude is a Soil root with `soil.toml`;
   on completion, pin its package hash as trusted (design §5); all
   exported definitions `accepted` and `pinned`.
6. **Kernel reconciliation** (deferred here from the soil0 CLI review,
   2026-08-22). `docs/contracts/soil0-cli.md` §6.1 and §11 pin *provisional*
   prelude surface: the kernel type shapes (`FsError`, `Utf8Error`,
   `Path`), the builtin names and signatures (`clock_now : Clock -> io
   I64` vs design §3.5's sketched `now : Clock -> io Time`; the list
   primitives; `unit`; `T::compare : T -> T -> I64` returning −1/0/1
   vs an `Ordering` sum; the fake constructors and their determinism
   guarantees). The prelude's first `.tr` specs must adopt these
   exactly, or revise them with the user and update the contract —
   before the corpus teaches them.

## Adoptions (2026-08-23, agentlanguages survey)

- **Hostile capability fakes** (design §4.5): failing fakes — an `Fs`
  that errors mid-stream, a backwards-jumping `Clock` — are ordinary
  prelude values in the capability modules from the start.
- **TrellisBench before the corpus grows** (design §10): even ~10
  spec+tests problems wired through the real daemon per release, so
  prelude/corpus changes are measured, not vibed. Stand it up at this
  plan's kickoff.

## The totality problem (known, planned for)

`soil0` has no termination checker, so every recursive prelude function
conservatively carries `div` — but the prelude's signatures *claim*
`total`, and those claims matter for the corpus and for callers. Interim
policy (confirm with user at kickoff): the `.tr` signatures state the
intended row; the daemon records a per-definition `div`-unverified flag in
the lock (like a demoted refinement) rather than widening signatures; the
stage-2 termination checker (plan 05) later discharges them in bulk. This
mirrors the demotion philosophy: unproven, visible, tests still gate.

## Non-goals

Batteries layers (`soil-rs-std`, `soil-py-std` — plan 06), `Py`
capability, retrieval (whole prelude fits in context), performance.

## Testing

- Every definition: expect tests + properties per the effect-row budget
  (pure functions fuzzed hard); capability functions get fake-capability
  tests; `read_file` keeps its real-mode cram test.
- Cross-cutting properties: `sort_by` stability and order laws,
  `parse ∘ show` identity on prelude types, Map comparator-order
  invariants.
- Differential where cheap: CLI oracles against Python equivalents
  (`statistics`, `str` methods) for the numeric/string corners.

## Exit criteria

- Every exported definition `accepted`, `pinned`, tests green, package
  hash pinned in `soil.toml`.
- The corpus test: a fresh lowering of a new small function, given the
  prelude as examples, produces Soil the user judges idiomatic without
  style corrections.
- `examples/` and the real prelude agree wherever they overlap.

## Decision points — resolved 2026-08-22

- **Totality gap:** signatures claim the intended row; the lock records
  `checks.termination: "unverified"` (mirroring refinement demotion —
  unproven, visible, tests still gate); soilc's termination checker
  (plan 05) discharges the flags in bulk. Recorded in
  docs/lock-schema.md §4.
- **Location:** in this repo, as a `prelude/` Soil root; extraction into
  its own forkable repo waits for a second user.
- **Inventory:** a concrete reviewed list before lowering begins
  (`read_file` + the §5 core types with ~6–12 functions each), then
  additions strictly by consumer need — the corpus stays curated.
