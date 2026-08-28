# Adoption Record — agentlanguages.dev Survey (2026-08-23)

*Status: decided 2026-08-23. The user reviewed the survey document
("Ideas for Trellis from the agentlanguages.dev Verification Camp",
38-language catalogue, August 2026) and adopted its recommendations as
below. This file traces every item to where the decision is recorded
and when it is built; the source document's numbering (§) is preserved.
Assessed against the project state of 2026-08-23: CLI contract frozen
(v1 → amended v1.1 here), soil0 mid-implementation (steps 2–6 done),
hashing/locks not yet built (plan 03).*

---

## Adopted — recorded now, built at the named milestone

| # | Item | Recorded in | Built |
|---|---|---|---|
| 2.1 | Per-clause assurance (`proven`/`runtime`/`trusted`); badges aggregate by minimum over exports | design §6.4, §7.1; lock-schema §4, §8 | plan 05 (emission), IDE (plan 06) |
| 2.2 | Counterexample is a failure, never a downgrade; becomes repair input + offered test | design §6.4; lock-schema §4; plan 05 | plan 05 |
| 2.3 | `blame` (caller/callee) in every violation record and `SoilError` | design §6.4; lock-schema §5; plan 05 | plan 05 (with the guards) |
| 3.1 | Mutation testing (`trellis mutants`); lock fields reserved now | design §4.5, §10; lock-schema §5 | post-v1 daemon job |
| 3.2 | Vacuity probes in the contradiction pre-flight | design §4.5; plan 03 | plan 03 |
| 3.3 | Hostile tier: boundary-biased properties + failing capability fakes | design §4.5; plan 04 | plan 04 (prelude values) |
| 4.1 | Stable codes → typed repair classes + `spec_ref`, CI drift gate | design §4.6; plan 03 | plan 03 (soil0's contract §2 registry is the substrate; its diagnostics stay unchanged) |
| 4.3 | Typed holes (`?name`), goal reporting, `partial(holes)` | design §3.16; syntax-spec §3.3/§5.11; contract v1.1 §4.3/§8.4; lock-schema §4/§8; impl plan 02 step 6b | **soil0 now** (step 6b); loop use in plan 03 |
| 4.4 | Budgeted context packer; sectioned spec; spec-token budget as CI tenet | design §3.1 (tenet row), §4.6; plan 03 | plan 03 (packer); tenet immediate |
| 4.5 | Skill generated from toolchain registries, drift-gated | design §4.6; plan 03 | plan 03 |
| 5.1 | Byte-exact canonical Soil; printer as a pass; agent never formats | design §4.8; syntax-spec §8 note; contract v1.1 (`print` reserved); impl plan 02 step 11 | soil0 step 11, **before plan 03's first hash** |
| 5.2 | `soil_hash` over the alpha-normalized AST | design §6.1; lock-schema §3 | plan 03 (hashing) |
| 5.3 | Toolchain pin, refuse-on-mismatch; explicit `trellis toolchain update` | design §8; plan 03 | plan 03 (soil.toml format) |
| 6.2 | `Literal` provenance fact; provenance-aware prelude/batteries signatures; human-only `trust_literal` | design §3.17; plan 06 | plan 06 (first boundary: `Proc`/`Py`) |
| 7 | JSON value model must suffice for remote marshalling (constraint) | design §3.11 | constraint immediate; `coproc` mode post-v1 |
| 8.2 | Decision blocks (`decisions` in `_module.tr`, new `_project.tr`), bundle-included, write-back target | design §4.3, §4.6; tr-grammar §1/§5.2/§6/§9 | plan 03 (packer); grammar immediate |

## Adopted as deferred-table entries (design §10, with triggers)

| # | Item | Trigger |
|---|---|---|
| 6.1 | seccomp/Landlock emission from capabilities; `trellis run --deny` | `trellis build` (plan 06+) |
| 6.1b | Per-resource confinement (scoped `Fs` via Landlock paths) | after kind-level sandbox |
| 8.1 | TrellisBench (published, multi-seed, pass@k) | plan 04 kickoff — before the corpus grows |
| 4.2 | Speculative proof-delta tools (`speculative_check`, spec-edit blast radius) | warm incremental checking (plan 03+) |
| 7b | Co-process Python isolation (`coproc` per binding, recorded in the lock) | post-v1; lowering sandbox first user |
| 5.2b | Misleading-name lint on Soil binders | after soilc |
| 8.3b | Bounded-model-checking rung between `proven` and `runtime` | if SMT coverage proves insufficient |

## Declined, deliberately (recorded in design §12)

De Bruijn surface syntax; AST-as-source/JSON programs; mandatory
contracts with no opt-out; bounded model checking as the primary
engine; first-person compiler personas. Reasons in design §12.

## Open questions created by the adoptions

- Decisions hash under `prose_hash` (change flags, doesn't invalidate) —
  or is a decision closer to formal? *Resolved 2026-08-24: neither* —
  a per-entry hash class with reliance edges (lowerings cite the
  decisions they applied), editorial reclassification for typo-class
  edits, and a batched triage sweep as the clearing path. Recorded in
  design §4.3, tr-grammar §1/§5.2/§9, lock-schema §2/§3/§8.
- The spec-token budget's concrete number (target ~8k tokens for the
  core reference) and the CI counter that enforces it — fix when the
  Soil spec is sectioned (plan 03/05).
