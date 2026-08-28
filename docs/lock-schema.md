# The Lock Entry Schema — Prototype

*Status: prototype. Tentatively resolves §9.3 of `docs/design.md`. Example
sidecars live next to the `.tr` examples in `examples/`. Hashes in examples
are abbreviated.*

---

## 1. Shape

One lock file per definition, sidecar: `f.tr` → `f.lock` (design §6.3). A
lock file is a single JSON object in the canonical form of the value
encoding (grammar prototype §7): sorted-stable key order, so diffs are
minimal and semantic. The global `soil.lock` manifest is derived by merging
sidecars (§7 below) and is gitignored.

Top-level keys, in order:

| Key | Kind | Purpose |
|---|---|---|
| `lock_format` | all | schema version of the lock itself |
| `name` | all | definition name, matching the `.tr` frontmatter |
| `kind` | all | `function` \| `type` \| `module` (inferred from the `.tr`, recorded for the manifest) |
| `versions` | all | `{trellis, soil}` versions the current artifacts target (design §6.3: upgrades must not invalidate silently) |
| `spec` | all | hashes and provenance of the `.tr` (§2) |
| `lowering` | function | the generated Soil (§3); `null` before first lowering |
| `checks` | function, type | checker facts (§4) |
| `tests` | function, type | per-case results (§5) |
| `oracles` | function | test-level edges (§5) |
| `ffi` | function | binding-only trust record (§6) |
| `cycle_hash` | type | combined hash for recursive type groups (design §3.8); `null` if acyclic |
| `accepted` | all | the human's trust flag (design §4.7) — the only field a human sets directly |

## 2. `spec`

```json
"spec": {
  "provenance": "human",
  "hashes": {
    "formal": "sha256:9f2c41aa",
    "test": "sha256:41aa73c0",
    "prose": "sha256:c8172d99"
  },
  "prose_state": "fresh",
  "blocks": [
    { "block": "soil-sig", "author": "agent" },
    { "block": "test odd-length", "author": "human" }
  ],
  "pinned": false,
  "escape_hatches": []
}
```

- `provenance`: `human` | `agent` — who authored the `.tr` overall,
  distinguishing the two vibing tiers (design §1.2, §9.3).
- `hashes`: the three-part hash (design §6.2). The block-to-hash mapping is
  grammar prototype §1. `test` is `null` for type and module files.
- `prose_state`: `fresh` | `review-suggested`. Set to `review-suggested`
  when `prose` changes under an unchanged lowering; auto-cleared when an
  agent re-reads the prose and confirms the Soil still matches (design
  §6.2).
- `blocks`: per-block provenance for the write-back scheme (grammar
  prototype §8). `block` is the info string minus modifiers (`xfail` is not
  identity). `author: human` means unmarked, therefore pinned — the agent
  may not edit it. Frontmatter is implicitly human.
- `pinned`: the export-pin flag (design §4.4) — human approval of the
  public signature. Present from the start, unenforced in v1.
- `escape_hatches`: the contents of the `allow` block, aggregated by the
  manifest into the audit view (design §4.1).
- `decisions` (module and project entries only; adopted 2026-08-24,
  tr-grammar §5.2): a map from entry label to the hash of that entry's
  text (rejected lines included). Decisions are their own hash class —
  `decisions` blocks are excluded from `prose` — and labels are
  identity (a rename is remove + add). Function entries record their
  reliance on these under `lowering.decisions` (§3).

## 3. `lowering`

```json
"lowering": {
  "soil_hash": "sha256:77b04e12",
  "provenance": "agent",
  "provider": "claude-code",
  "model": "claude-opus-4-7",
  "private_helpers": [ { "name": "_parse_cell", "hash": "sha256:3fe210bb" } ],
  "calls": [ { "name": "sort_by", "hash": "sha256:aa90b1f3" } ],
  "decisions": {
    "scope_hash": "sha256:5b21aa04",
    "applied": [ { "scope": "project", "label": "timestamps", "hash": "sha256:c91d20fe" } ]
  }
}
```

- `soil_hash`: content-address of the Soil body **plus transitively
  referenced private helpers** (design §6.3), free variables replaced by
  callee hashes (design §6.1). *Adopted 2026-08-23 (design §6.1, §4.8):*
  the hash is computed over the **canonical text of the alpha-normalized
  AST** — local binders as indices, display names excluded, exactly one
  printed form per AST (the printer is a compiler pass; soil0 impl step
  11) — so local renames and formatting can never invalidate caching or
  verification.
- `provenance`: `agent` | `human-verified` | `hand-edited` | `prelude-fork`.
  `hand-edited` skips re-lowering until the spec changes (design §4.8).
- `provider` / `model`: which agent produced the current Soil, for audit and
  future model routing. Costs, retries, and timings live in the gitignored
  `f.log`, not here (design §4.6). Present only when an agent ran:
  hand-written entries (`human-verified`, `hand-edited` without a prior
  agent run) carry plain `null` — the lock's own null convention, as
  with an absent `lowering` or `test` hash (resolved 2026-08-28 with
  the examples-regeneration policy — a lock must not assert a lowering
  event that never happened).
- `private_helpers`: the `_private.soil` definitions this lowering owns
  (design §4.2); the manifest derives its `soil-private` nodes from these,
  and a helper with no remaining owner is garbage-collected.
- `calls`: callee edges with the hashes they were checked against — the
  graph view and the invalidation record (design §4.3, §6.1).
- `decisions` (adopted 2026-08-24, design §4.3; tr-grammar §5.2):
  **reliance edges**, the decisions analog of `calls`. `applied` lists
  the decision entries the lowerer cited as applied — the citation is a
  required part of the lowering's completion output, possibly empty —
  each pinned at the entry hash it was applied under (`scope` is
  `project` | `module`). `scope_hash` covers the sorted entry *labels*
  in scope (membership, not text), so additions and removals are
  visible without text edits flagging everyone. Staleness is derived,
  never stored (§8): an `applied` hash that no longer matches the
  current entry — or whose entry is gone — flags this lowering; a
  `scope_hash` mismatch flags everything in scope. An *editorial*
  reclassification by the human, or a triage-sweep confirmation that
  the Soil still conforms, re-stamps these hashes without re-lowering.

## 4. `checks`

```json
"checks": {
  "types": "ok",
  "termination": "verified",
  "refinements": { "upper bound": "proven", "non-empty": "runtime" },
  "holes": 0
}
```

- `types`: `ok` | `error`.
- `termination`: `verified` | `unverified` | `n/a` — whether the claimed
  row's absence of `div` is established. Until the termination checker
  exists (plans 04–05), recursive definitions claiming totality carry
  `unverified`: the demotion philosophy applied to `div` — unproven,
  visible, tests still gate.
- `refinements`: **assurance per clause** (adopted 2026-08-23, design
  §6.4), a map from clause label (grammar prototype §3.2) to `proven`
  (SMT unsat; erased in release) | `runtime` (solver unknown/timeout —
  demoted, guard active in every build) | `trusted` (human escape
  hatch). `{}` when the definition has no refinements. A definition can
  honestly be two-proven-one-runtime; the IDE renders the unproven chain
  from the `runtime` entries. **A counterexample is never a state**: sat
  with a model fails the lowering, and the counterexample becomes a
  structured repair input and an offered expect test (design §6.4) —
  `runtime` means "undecided", never "known wrong".
- `holes`: the count of unfilled typed holes (design §3.16). Nonzero is
  the `partial` state: the lowering paused with goals open.
- Type entries use `invariants` in place of `refinements` (same
  per-clause map).

## 5. `tests` and `oracles`

```json
"tests": [
  { "name": "odd-length#1", "tier": "expect", "mode": "sandboxed", "origin": "spec", "result": "pass" },
  { "name": "ensures lower bound", "tier": "property", "mode": "sandboxed", "origin": "derived", "result": "pass" },
  { "name": "real-read#1", "tier": "cram", "mode": "real", "origin": "spec", "result": "pass" }
],
"oracles": [
  { "kind": "reference", "path": "ref/stats.py::median", "hash": "sha256:5e8f0b2a" },
  { "kind": "definition", "name": "sort_by", "formal_hash": "sha256:aa90b1f3" }
]
```

- `name`: `block-name#k` for multi-case blocks; the bare block name for
  single-case blocks. Derived tests are namespaced (resolved
  2026-08-24 with impl plan 03 §9.5): `derived:ensures:<label>` /
  `derived:requires:<label>` / `derived:invariant:<label>` for
  clause-derived properties, `derived:differential:<reference-line>`
  for the reference tier, `derived:contract:<n>` reserved for FFI
  contract tests (plan 06). The `derived:` prefix can never collide
  with a spec block name; chosen over bare clause labels for exactly
  that reason. (The pre-daemon example locks used ad hoc names; the
  regeneration pass rewrites them.)
- `tier`: the test lattice (design §4.5): `expect` | `property` | `cram` |
  `contract` | `differential` | `proof`.
- `mode`: `sandboxed` (fakes, runnable by the lowerer's `run_tests`) |
  `real` (cram; never available inside the lowering sandbox). New modes can
  be added without a format change (design §4.5).
- `origin`: `spec` (a block in the `.tr`) | `derived` (auto-generated:
  properties from refinement clauses per design §4.1, contract tests for
  bindings per design §4.5, invariant properties for types).
- `result`: `pass` | `fail` | `xfail` (expected failure, failed) | `xpass`
  (expected failure, passed — needs spec attention). Failure details
  (counterexample, diff, and the `blame` field — `caller` for a
  `requires` violation, `callee` for `ensures`/`invariant`, design §6.4)
  are not stored here; they live in `f.log` and the IDE. Results are
  cache entries keyed by the hashes (design §6.1), so they churn only
  when inputs do.
- `mutation` (reserved 2026-08-23, design §4.5; written by the post-v1
  `trellis mutants` job): `{ "score": 0.87, "survivors": n }` — the
  test-strength record, reservable now so its arrival is not a format
  change. Survivor details live in `f.log` and the IDE's
  mutant-to-test flow.
- `oracles`: what the tests depend on beyond the definition itself — the
  reference implementation (by file hash), `accepted` definitions used as
  oracles (by `formal_hash`), and CLI oracles (by executable hash):
  `{ "kind": "cli", "command": "soil0 parse", "hash": "sha256:…" }`. A
  changed oracle re-runs dependent tests (design §4.5).

## 6. FFI bindings

A binding is an ordinary function entry plus an `ffi` section (design §3.11,
§6.3, §8):

```json
"ffi": {
  "trust": "contract",
  "symbol": "requests.get",
  "symbol_hash": "sha256:be77a0c1",
  "package": "/nix/store/a1b2…-python3.12-requests-2.32.3"
}
```

- `trust`: `declared-only` | `contract` | `harvested`.
- `symbol_hash`: hash of the symbol's machine-readable declaration (`.pyi`
  stub, `cargo doc` JSON, C header decl) — the *interface* record, designed
  so symbol-level invalidation can slot in later (design §8).
- `package`: Nix store path — the *provenance* record.

## 7. The derived manifest (`soil.lock`)

A pure merge of the sidecars — never separately maintained (design §6.3) —
plus nodes and aggregates that only exist globally:

```json
{
  "lock_format": 1,
  "definitions": { "csvstats/median": { …sidecar contents… } },
  "soil_private": [
    { "name": "csvstats/_parse_cell", "hash": "sha256:3fe210bb", "owners": ["csvstats/parse_row"] }
  ],
  "escape_hatches": {},
  "trusted_packages": { "prelude": "sha256:e0a1b2c3", "soil-rs-std": "sha256:f1b2c3d4" }
}
```

`soil_private` nodes are derived from every `lowering.private_helpers` list
(greyed out in the IDE; many helpers + few definitions is a flagged smell,
design §4.2). `escape_hatches` is the audit view. `trusted_packages` pins
the prelude and batteries by full package hash (design §5).

## 8. Derived status and invariants

The `typed` / `tested` / `verified` / `accepted` ladder (design §4.7) is
**derived, never stored**:

- `typed` — `checks.types == "ok"` and `checks.holes == 0`.
- `partial` — `checks.holes > 0` (design §3.16): the lowering paused with
  open goals; never `tested`, never `accepted`, excluded from release
  targets.
- `tested` — typed, and every test result is `pass` or `xfail`.
- `verified` — tested, and every `checks.refinements` clause is `proven`
  (unreachable when the map is empty; only `tested` means correct, design
  §4.1).
- `accepted` — the stored flag.

Module and project badges are the **minimum** over exported definitions'
statuses and clause assurances (design §7.1) — derived by the IDE, never
stored.

**Decision staleness** (2026-08-24) is likewise derived, never stored:
compare `lowering.decisions` against the module/project entries'
current hashes (§3). It surfaces alongside `review-suggested` and does
not block the ladder — tests remain the trust root; the triage sweep
and the editorial reclassification (tr-grammar §5.2) are the clearing
paths.

Invariants the daemon enforces:

1. `accepted` may be `true` only if `tested` holds and no result is `xfail`
   or `xpass` (design §4.5) — resolving an xfail is a spec change, which
   clears `accepted` via the hash rules below. This gate applies to
   function entries; module and project entries carry `accepted`
   ungated (they have no tests — §9, resolved 2026-08-24).
2. A `human`-authored block may not be rewritten by the agent; the lowerer
   can only `ask_human`.
3. Only `accepted` definitions may appear in another entry's `oracles`.
4. A definition with `checks.holes > 0` cannot be `tested` or `accepted`
   and never reaches a build target (design §3.16).

Invalidation on change:

| Changed | Effect |
|---|---|
| `spec.hashes.formal` | lowering invalid; re-lower or re-verify; callers follow via `calls[].hash` |
| `spec.hashes.test` | re-run tests; re-lower if failing |
| `spec.hashes.prose` | `prose_state: "review-suggested"`; lowering stays valid |
| a decision entry's text | citing lowerings become decision-stale (derived from `lowering.decisions.applied`); cleared by editorial reclassification, triage confirmation, or re-lowering |
| a decision entry added or removed | every in-scope lowering's `scope_hash` mismatches — flagged once; a removal may be reclassified editorial |
| a decision edit reclassified *editorial* by the human | recorded hashes re-stamped; nothing flagged |
| an oracle's hash | re-run the dependent tests only |
| Soil hand-edit | `soil_hash` updated, `provenance: "hand-edited"`, re-lowering skipped until spec changes |
| `versions` | nothing, until the spec changes (design §7.1 upgrade policy) |

## 9. Open questions raised by this prototype

All three questions raised by the first draft were resolved 2026-08-24
with impl plan 03 (§9.5, §9.6):

- Module and project entries **carry `accepted`** — a human can
  meaningfully accept an export list and a decisions block — but CI
  policy and badges continue to key on exported *function*
  definitions, so nothing changes downstream. Module/project entries
  have no tests, so their `accepted` has no derived gate (invariant 1
  in §8 applies to function entries).
- The `derived` naming scheme is the `derived:` namespace (§5).
- The prelude-fork hash stays **global** in `soil.toml`'s
  `trusted_packages`: per-entry pins would repeat one hash everywhere
  until per-entry forks exist, and the toolchain pin (design §8)
  already refuses to lower or verify under a changed prelude.
