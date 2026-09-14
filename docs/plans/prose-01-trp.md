# Plan P1 — `trp`: the trellis-prose checker

*References: `docs/trellis-prose.md` (the whole design), `docs/lock-schema.md`
(for the spirit of lock conventions), `examples/prose/` (normative
fixtures). This is the first milestone of the trellis-prose sibling track;
it shares the repo's Rust workspace but depends on nothing in the Soil
build order.*

## Decisions resolved with the user (2026-09-14)

These four were decided before this plan was written; they are recorded
here with reasoning, per the working rules.

1. **Deliverable: plan doc first, build later.** Building starts in a
   separate session from this plan, the way plans 01–07 work.
2. **Code home: a `trp` crate in the existing `rust/` workspace.** Shares
   the toolchain, `check.sh`, and testing conventions; parser/checker/lock
   code benefits from Rust's strictness. A separate repo was rejected while
   trellis-prose remains a sibling design inside this one; Python was
   rejected as a weaker fit with the workspace (and the segmentation rule
   below removes the need for NLP libraries).
3. **Lowering: checker-only tool, in-session agent.** `trp` is a pure
   oracle: it parses, verifies, and manages locks, and never calls a model.
   A Claude Code session performs the lowering and iterates against
   `trp check` until green — mirroring how Trellis lowering works
   pre-daemon. A `trp lower` API wrapper was rejected for v1 (it would own
   prompting, keys, and retry policy before the workflow is understood);
   it may return as a later milestone.
4. **Style judge: the human, in v1.** The mechanical checks gate; setting
   `accepted` doubles as the style verdict, recorded in the lock as
   `judge: human`. Agent judging (in-session or tool-invoked) is deferred
   until the workflow has been exercised. Recorded in
   `docs/trellis-prose.md` §5.1.

## Goal

A Rust crate `trp` providing the trellis-prose oracle: parse `.trp` files,
mechanically verify a candidate lowering against its plan (the strict 1:1
mapping, §4 of the design), compute the three-part hashes, and read, write,
and report on `.lock` sidecars. With `trp` green and a human `accept`, a
document has the full trellis-prose discipline: hashed spec, verified
structure, tracked freshness.

## Scope

1. **Spec finalization gate.** Before code: resolve design §8.3
   (segmentation — proposal below) and §8.6 (the provisional container,
   frontmatter, heading-level, and lock decisions) with the user, and
   freeze the lock's v1 field set. Outcomes are recorded in
   `docs/trellis-prose.md` in place, and `examples/prose/` is updated to
   conform before the parser is written against it.
2. **Crate and CLI skeleton.** `rust/trp` joins the workspace. Binary with
   three subcommands:
   - `trp check <name>.trp` — parse and validate the spec; if the sibling
     `<name>.md` exists, verify the lowering against the plan;
     `--write-lock` updates the lock on green (with the lowering
     provenance passed by flag).
   - `trp status [dir]` — freshness table across documents: `fresh`,
     `stale` (plan hash mismatch), `review-suggested` (notes or style hash
     mismatch), `unlowered`, `unlocked`.
   - `trp accept <name>` — the human-only action: sets `accepted`, records
     the style verdict as `judge: human`. Refuses if checks are not green
     or the lock is stale.
3. **Parser.** Frontmatter (`name`, `style`; unknown keys are errors), the
   single required `plan` fence, directive lines against the closed move
   set, with positioned diagnostics (file, line, what was expected).
   Structural rules from design §3.2: prose directive outside an open
   paragraph is an error; `heading` closes the open paragraph.
4. **Segmenter and structural verifier.** Split the output `.md` into
   headings and paragraphs; segment paragraphs into sentences; verify the
   four mechanical checks of design §4 (known directives, verbatim
   headings in order and level, paragraph count, per-paragraph sentence
   counts). Diagnostics name the first offending directive/sentence pair.
5. **Hashing, locks, freshness.** Compute `plan`/`notes`/`style` hashes and
   the output hash; serialize and parse the lock; implement the freshness
   states and the `review-suggested` semantics for notes and style edits.
6. **Fixtures.** `examples/prose/` is the normative positive fixture; add a
   negative corpus under `rust/trp/tests/fixtures/` (unknown move, sentence
   outside a paragraph, count mismatch, reordered/reworded heading,
   ambiguous segmentation, two `plan` blocks, unknown frontmatter key).
7. **Workflow validation.** Lower at least two real documents end-to-end in
   a Claude Code session using `trp check` as the oracle, through `accept`.
   Friction observed here (especially re-lowering churn, design §8.5) is
   recorded back into the design's open questions, not fixed ad hoc.

## Segmentation proposal (decision point 2)

Resolve design §8.3 as: sentence terminators are `.`, `!`, `?`; a sentence
ends at the first terminator; **terminators may not appear
sentence-internally** in v1 output. No abbreviations ("e.g.", "Dr."), no
decimal numerals, no ellipses. Segmentation becomes trivial and exact — the
checker needs no heuristics — and the burden falls on the agent's wording,
which is the tenet working as intended ("verbosity for the agent is
acceptable"). A lowering that needs an internal period must re-word.

## Non-goals

`trp lower` (API-driven lowering), the agent style judge, a mechanical
style floor (design §8.2), richer output than headings and paragraphs
(§8.4), wording-stability across re-lowerings (§8.5), multi-document
projects (§8.7). Each stays in the design's open questions until the
workflow validation produces evidence.

## Testing

- Rust unit tests per module; golden-file tests for diagnostics (match
  `soil0`'s conventions).
- `trp check` runs green over `examples/prose/` in `rust/check.sh`; the
  negative corpus asserts each documented diagnostic.
- Lock round-trip: parse → serialize is byte-identical for the example
  lock; freshness states are exercised by mutating fixture copies (edit
  plan → `stale`; edit notes → `review-suggested`; edit style →
  `review-suggested` for every referencing document).

## Exit criteria

1. `trp check examples/prose/small_languages.trp` is green, and every
   negative fixture produces its documented diagnostic.
2. `trp status` and `trp accept` demonstrate all freshness states and the
   accept gate on the fixtures.
3. Two new real documents lowered in-session to green, accepted, with
   locks committed.
4. Design §8.3 and §8.6 are resolved in place in `docs/trellis-prose.md`,
   `examples/prose/` conforms, and this plan is updated with the decision
   outcomes.
5. `rust/check.sh` covers `trp`.

## Decision points (for the user, at milestone start)

1. **§8.6 confirmations:** the CommonMark container with a single `plan`
   fence, required `style` frontmatter, explicit heading levels, lock field
   shapes.
2. **Segmentation:** the no-internal-terminators rule proposed above.
3. **Hash canonicalization:** hash raw bytes of each region, or
   newline-normalize first (affects cross-platform lock stability).
4. **Example hash policy:** repo convention is abbreviated fake hashes in
   examples, but `trp check` in CI cannot verify those. Options: a
   `--no-hash-verify` mode for the docs examples; or real (still
   abbreviated?) hashes in `examples/prose/` regenerated by CI.
5. **CLI contract:** whether to freeze a `docs/contracts/trp-cli.md` (as
   soil0 did) before agents start relying on the interface, or let the CLI
   settle through the workflow validation first.
