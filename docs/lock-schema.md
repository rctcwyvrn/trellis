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

## 3. `lowering`

```json
"lowering": {
  "soil_hash": "sha256:77b04e12",
  "provenance": "agent",
  "provider": "claude-code",
  "model": "claude-opus-4-7",
  "private_helpers": [ { "name": "_parse_cell", "hash": "sha256:3fe210bb" } ],
  "calls": [ { "name": "sort_by", "hash": "sha256:aa90b1f3" } ]
}
```

- `soil_hash`: content-address of the Soil body **plus transitively
  referenced private helpers** (design §6.3), free variables replaced by
  callee hashes (design §6.1).
- `provenance`: `agent` | `human-verified` | `hand-edited` | `prelude-fork`.
  `hand-edited` skips re-lowering until the spec changes (design §4.8).
- `provider` / `model`: which agent produced the current Soil, for audit and
  future model routing. Costs, retries, and timings live in the gitignored
  `f.log`, not here (design §4.6).
- `private_helpers`: the `_private.soil` definitions this lowering owns
  (design §4.2); the manifest derives its `soil-private` nodes from these,
  and a helper with no remaining owner is garbage-collected.
- `calls`: callee edges with the hashes they were checked against — the
  graph view and the invalidation record (design §4.3, §6.1).

## 4. `checks`

```json
"checks": {
  "types": "ok",
  "refinements": "demoted",
  "demoted": ["upper bound"]
}
```

- `types`: `ok` | `error`.
- `refinements`: `proven` | `demoted` | `none`. Demotion is per design
  §6.4: the function drops to its base ML type for callers, tests remain
  required, and the runtime check stays in release builds (design §3.9).
- `demoted`: the clause labels (grammar prototype §3.2) that failed to
  prove — what the IDE renders as the unproven chain.
- Type entries use `invariants` in place of `refinements`.

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
  single-case blocks; a clause label for derived tests.
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
  (counterexample, diff) are not stored here; they live in `f.log` and the
  IDE. Results are cache entries keyed by the hashes (design §6.1), so they
  churn only when inputs do.
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

- `typed` — `checks.types == "ok"`.
- `tested` — typed, and every test result is `pass` or `xfail`.
- `verified` — tested, and `checks.refinements == "proven"` (unreachable
  when `refinements` is `none`; only `tested` means correct, design §4.1).
- `accepted` — the stored flag.

Invariants the daemon enforces:

1. `accepted` may be `true` only if `tested` holds and no result is `xfail`
   or `xpass` (design §4.5) — resolving an xfail is a spec change, which
   clears `accepted` via the hash rules below.
2. A `human`-authored block may not be rewritten by the agent; the lowerer
   can only `ask_human`.
3. Only `accepted` definitions may appear in another entry's `oracles`.

Invalidation on change:

| Changed | Effect |
|---|---|
| `spec.hashes.formal` | lowering invalid; re-lower or re-verify; callers follow via `calls[].hash` |
| `spec.hashes.test` | re-run tests; re-lower if failing |
| `spec.hashes.prose` | `prose_state: "review-suggested"`; lowering stays valid |
| an oracle's hash | re-run the dependent tests only |
| Soil hand-edit | `soil_hash` updated, `provenance: "hand-edited"`, re-lowering skipped until spec changes |
| `versions` | nothing, until the spec changes (design §7.1 upgrade policy) |

## 9. Open questions raised by this prototype

- Whether module entries participate in `accepted` (CI policy currently
  keys on exported *function* definitions) or carry only hashes.
- Naming scheme for `derived` tests is ad hoc (clause labels, contract
  names); needs fixing before the daemon exists.
- Whether `versions` should pin the prelude fork hash per entry or leave it
  global in `soil.toml`.
