# Implementation Plan 03 — the Trellis daemon

*Detailed implementation guide for [`../03-daemon.md`](../03-daemon.md).
That plan states the scope and exit criteria; this one states how the
crate is structured and built, restates the decisions resolved with the
user on 2026-08-22, and records the micro-details (§8, approved
2026-08-24) and the newly surfaced spec gaps (§9, all resolved —
2026-08-24, plus the step-7 batch on 2026-08-28 — and propagated to
the spec docs).
References: design §4 (specification layer, lowering, the daemon), §6
(hashing and locks), §8 (toolchain pin), `docs/tr-grammar.md`,
`docs/lock-schema.md`, `docs/contracts/soil0-cli.md` (frozen v1.1 — the
surface this milestone drives), soil-syntax-spec §9 (canonical form),
impl plan 02 (the library this links), and
`docs/plans/extra/agentlanguages-adoptions.md` (the survey items that
land here).*

---

## 1. Resolved decisions (2026-08-22, from plan 03)

Restated from the plan's decision points; none are reopened here.

| Decision | Choice | Rationale (recorded in plan 03) |
|---|---|---|
| IPC | JSON-RPC over a Unix socket at `$XDG_RUNTIME_DIR/trellis/<root-hash>.sock` | One transport for CLI and (later) IDE; the LSP speaks its own stdio transport when it arrives; TCP is the future remote-serving path. |
| Language | Rust, workspace member beside `soil-rt`/`soil0`, linking both | The daemon does in-process checking through the `soil0` library; the `soil0` *CLI* remains the oracle contract regardless (contract §12). |
| Sandboxing | All three layers: MCP tool allow-list + the agent CLI's own sandbox and isolated home + an OS jail (unprivileged user namespaces / bubblewrap) around the agent process | Defense in depth from v1. |
| Incremental state | Explicit refresh — hashes re-checked at request boundaries plus `trellis refresh`; no watcher until the IDE milestone | The watcher later reuses the same invalidation code path. |

Also load-bearing and already settled elsewhere: `soil_hash` consumes
the canonical printer (soil0 step 11 is done; `parse → print` is a
fixpoint on the examples); the lowering loop treats typed holes as the
retry unit with `partial(holes)` in the lock (design §3.16, contract
v1.1); the MCP tools (seven since 2026-08-25 — `read_spec` added with
the step-1 contract review) and the exit-and-reinvoke question channel
(design §4.6); Claude Code headless as the first provider.

**Boundary notes.** No LSP server in this milestone — the plan's goal
line reads "serves the CLI (and later the IDE and LSP)", and the
repair-class registry (§4.4) is built now so the LSP inherits it later.
No refinement checking (`check_refinements` is a stub returning none),
no concurrent lowerings, no raw-API provider, no watcher, no
`trellis mutants` (lock fields stay reserved), no speculative
proof-delta tools (shaped by the registry and RPC design only).

---

## 2. Crate skeleton

One new workspace member. Per §8.1 the CLI and the daemon are **one
binary** (`trellis`), so there is exactly one artifact to version, pin,
and jail-mount.

```
rust/
  Cargo.toml           -- members += ["trellis"]
  check.sh             -- extended: trellis fmt/clippy/test + drift gates (§5 step 12)
  trellis/
    Cargo.toml         -- [[bin]] trellis; lib for tests
    registry/
      diagnostics.json -- error code -> repair class + spec_ref (§4.4); drift-gated
    src/               -- module map in §3
    tests/
      golden/          -- .tr parse trees, hashes, bundles, packed contexts
      reject/          -- one case per .tr validity rule, keyed by error code
      fixture_root/    -- a small Soil root (csvstats-shaped) for e2e through the fake provider
      scripts/         -- scripted-provider playbacks (§8.9)
```

Dependencies, kept deliberately short:

| Crate | Why | Where |
|---|---|---|
| `soil-rt` (path) | canonical value JSON for everything Soil-shaped in bundles and results | runtime |
| `soil0` (path) | parser (types, predicates), checker, interpreter, test runner, canonical printer — in-process (§8.11) | runtime |
| `serde`, `serde_json` | RPC, locks, bundles, registry, telemetry | runtime |
| `sha2` | SHA-256 for every hash the lock stores | runtime |
| `pulldown-cmark` | CommonMark fenced-block extraction for `.tr` (§8.3) | runtime |
| `toml` | `soil.toml` parsing (added at step 2, 2026-08-25: the table above missed that a TOML file needs a TOML parser; hand-rolling TOML repeats exactly the subtle-divergence risk the pulldown-cmark pin rejected) | runtime |
| `proptest` | hash canonicalization + packer properties | dev-only |

No async runtime, no clap, no YAML crate, no MCP SDK (§8.2, §8.3,
§8.7). Tools added to the repo-root `shell.nix`: `bubblewrap` (the OS
jail) and `python3` (the reference-oracle runner, §9.9). The `claude`
CLI is deliberately **not** managed by `shell.nix` — it is a
user-supplied ambient tool, exercised only by the opt-in live smoke
test (§5 step 11, §7).

---

## 3. Module map

```
src/
  main.rs        -- thin dispatch: client subcommands vs `daemon run` vs `_mcp`
  cli.rs         -- the thin client: parse args, ensure daemon, RPC, render
  rpc.rs         -- JSON-RPC 2.0 over ndjson frames; request/response types (§8.2)
  daemon.rs      -- socket server, connection threads, shared state, lifecycle
  config.rs      -- soil.toml (toolchain pin, tags, entrypoint) + providers.toml
  trfile/
    mod.rs       -- TrFile: frontmatter, block list, kind inference, validity rules
    frontmatter.rs -- the two-key YAML subset (§8.4)
    blocks.rs    -- fence extraction via pulldown-cmark; info-string parsing
    sig.rs       -- soil-sig via soil0's type parser
    predicate.rs -- requires/ensures/invariant/property via soil0's predicate parser
    tests_blk.rs -- test call-arrow lines, with-bindings; property; cram; reference
    exports.rs   -- exports + decisions block grammars
  diag.rs        -- daemon diagnostics: code, message, span, repair_class, spec_ref
  registry.rs    -- loads registry/diagnostics.json; the drift-gate helpers
  hash/
    mod.rs       -- the three-part hash (§8.5), soil_hash (§8.6), cycle hash
  lock.rs        -- lock model, canonical reader/writer (§8.7), schema invariants
  manifest.rs    -- soil.lock derivation by merge (lock-schema §7)
  state.rs       -- the definition graph, statuses, invalidation table, refresh
  envgen.rs      -- .tr types -> env.json (§9.1); program.json assembly
  testrun.rs     -- bundle assembly, expect via soil0, derived tests, results -> lock
  propgen.rs     -- property generators, where-filter constraints, seeds (§8.12)
  preflight.rs   -- contradiction check + vacuity probes
  oracle.rs      -- reference (python3 subprocess) and CLI oracles; differential tier
  cram.rs        -- cram runner: temp dir, fixtures, transcript match; `trellis call`
  bundle.rs      -- context-bundle assembly; the budgeted packer (§8.10)
  mcp.rs         -- the `_mcp` stdio shim + the seven tool implementations (§8.8)
  jail.rs        -- bubblewrap invocation, isolated home, env scrubbing (§8.9)
  provider.rs    -- Provider trait; claude-code provider; scripted fake provider
  job.rs         -- the lowering job loop: queue, caps, invalidation, outcomes
  question.rs    -- ask_human persistence, `trellis answer`, prose write-back
  telemetry.rs   -- f.log JSONL events (§8.14)
  repl.rs        -- `trellis repl` and `trellis call` evaluation path
  skill.rs       -- `trellis skill` assembly from registries + corpus
```

Dependency direction: `cli/daemon → job → {provider, mcp, jail, bundle,
question} → {testrun, preflight, oracle, cram} → {state, envgen} →
{lock, manifest, hash} → trfile → soil0/soil-rt`, everything using
`diag`, `registry`, `config`, `telemetry`. Nothing reaches back up.

---

## 4. The contract surface: `docs/contracts/trellis-daemon.md` and friends

Written and **reviewed by the user before code** (step 1), because this
milestone freezes three agent- or user-facing surfaces. The internal
CLI↔daemon RPC is explicitly *not* contractual (both ends live in one
binary and ship together).

1. **The MCP tool surface** — the seven tools' names, input schemas,
   output schemas, and error shapes: `read_context`, `check_types`,
   `check_refinements` (returns the `Option` encoding's `None` until
   soilc), `run_tests`, `write_soil`, `read_spec` (pinned spec
   sections by anchor, served from the toolchain's embedded doc
   copies — added 2026-08-25 at review), `ask_human`. This is what
   the generated skill documents and what plan 05's strangler swap
   must keep stable.
2. **The context-bundle layout** (design §4.6): the fixed directory
   tree — `spec.md`, `decisions.md`, `callees/<name>.md` (signature +
   refinement clauses with per-clause assurance; a `runtime` clause
   shown as base type plus a demotion note), `tests.json` (the soil0
   bundle-case shapes, reused verbatim), `examples/`, `reference.py`
   or the CLI-oracle stanza, `previous.soil` — plus the packer's
   priority order and drop rules (§8.10).
3. **The diagnostic registry format** (§4.4 adoption): every daemon
   diagnostic carries `code`, `repair_class`, and `spec_ref`;
   `registry/diagnostics.json` is the single source; the doc renders
   the table from it. The soil0 contract §2 registry is the substrate:
   soil0 diagnostics pass through with their codes unchanged and gain
   `repair_class`/`spec_ref` by registry lookup.
4. **`spec_ref` anchors**: the scheme `<doc>#<section>` pinned against
   the numbered sections of `docs/soil-syntax-spec.md` and
   `docs/tr-grammar.md` (e.g. `soil-syntax-spec#5.9`). Renumbering a
   referenced section becomes a drift-gate failure, which is the point.
5. **The question and answer schema**: what `ask_human` writes, what
   `trellis answer` consumes, and the prose write-back format (§9.7).
6. **`f.log` event schema** (§8.14) — documented because humans read
   it, marked non-contractual.

Separately, **`docs/soil-toml.md`** (status: prototype, like the
grammar docs) specifies the v1 subset the daemon needs: `[toolchain]`
(the pin — `trellis`, `soil0_cli`, `skill` hash; prelude hash arrives
in plan 04), `[tags]` (the vocabulary `.tr` validation reads),
`entrypoint` (parsed, unused until build targets). Everything else in
design §8 stays future sections. `trellis toolchain update` rewrites
`[toolchain]` to the running binary's identity; any command that
lowers or verifies under a mismatched pin refuses with the
`toolchain-mismatch` diagnostic (design §8).

---

## 5. Build order

Steps are sequential; a step is done when its tests pass and `check.sh`
is green. Everything through step 9 is testable hermetically; step 10
adds the scripted fake provider so the whole loop is CI-testable
without tokens; only step 11 touches a real agent.

### Step 1 — contracts and spec resolutions

`docs/contracts/trellis-daemon.md` and `docs/soil-toml.md` (§4),
reviewed by the user. The §9 spec gaps were all resolved with the
user on 2026-08-24 and are recorded in the spec docs; this step also
lands their document artifacts that belong to plan 03 — syntax-spec
§9.1 (the hash form) is already written, the contract v1.2 amendment
is recorded, and the two new docs above are drafted here. Approval of
those two documents unblocks everything below.

### Step 2 — skeleton, lifecycle, RPC, tooling

Workspace member, `main.rs` dispatch, the ndjson JSON-RPC layer, the
socket server with one thread per connection, `trellis daemon
run|stop|status`, client auto-spawn (§8.1), `shell.nix` additions
(`bubblewrap`, `python3`), `.gitignore` entries (`.trellis/`, `*.log`,
`soil.lock`), `check.sh` extension. `config.rs` reads `soil.toml` and
enforces the toolchain pin from the first command.

Tests: socket round-trip, concurrent clients, stale-socket recovery,
auto-spawn, pin mismatch refusal.

### Step 3 — `.tr` parsing

The full tr-grammar: frontmatter (name required, tags against the
`soil.toml` vocabulary, unknown keys are errors), fenced-block
extraction with reserved-language recognition (everything else is
prose, including unreserved fences), the block mini-languages —
`soil-sig` and the predicate blocks parsed through the linked `soil0`
parsers so there is exactly one grammar implementation for types and
predicates; `test` (with-lines, call-arrow cases, JSON args via
canonical decode), `property` (forall/where), `cram` (the §3.5
subset), `reference` (both forms), `allow`, `exports`, `decisions`,
`soil-type` (via soil0's type declarations). Kind inference and every
§6 validity rule. `@agent` info-string markers recorded per block
(tr-grammar §8).

Tests: golden parse trees for every `examples/*.tr`; a rejection
corpus with one case per validity rule, keyed by daemon error code
(`unknown-frontmatter-key`, `undeclared-tag`, `name-mismatch`,
`duplicate-test-name`, `missing-test-block`, `panic-without-row`, …);
proptest: arbitrary bytes never panic the parser, block spans
partition the file.

### Step 4 — hashing

The three-part hash over the §8.5 canonicalization; per-entry decision
hashes and the scope-membership hash (§9.4); `soil_hash` per §8.6 —
canonical printer output in hash form (§9.2), free references replaced
by referent hashes, private helpers folded in; the recursive-type
cycle hash. All hashes are `sha256:` + 64 hex (§9.10).

Tests: golden hashes over `examples/`; **one mutation test per
invalidation row** (lock-schema §8): edit prose → only `prose_hash`
moves; edit a test block → only `test_hash`; edit the signature →
`formal_hash`; rename a local binder in the `.soil` → **no** hash
moves (the alpha-normalization test); reformat the `.soil` →
no hash moves (the canonical-form test); edit a callee body → the
caller's `soil_hash` moves; edit one decision entry → only that
entry's hash moves, the three-part hashes and the scope hash
untouched; add an entry → only the scope hash moves. Reproducibility: hashes byte-identical
across two runs and under a copied tree at a different absolute path.

### Step 5 — locks and the manifest

The lock model exactly per lock-schema, the canonical writer (§8.7),
the reader with schema validation, the §8 invariants enforced on
write (`accepted` gating, oracle acceptance, block-author pinning,
holes never `tested`); `soil.lock` derived by pure merge —
definitions, `soil_private` nodes from `private_helpers` with
ownership GC, the escape-hatch audit view, `trusted_packages`.

This step includes the **examples regeneration pass** (§9.10; policy
resolved 2026-08-28): **staged** — step 5 regenerates the spec
hashes, decisions, lowering records, and checks facts, carrying the
existing test-result rows forward verbatim; step 7 re-runs every test
and writes the final rows, and the byte-identity exit criterion is
judged there. Scope: **every `.tr` gets a real lock** (`mean.tr` and
`parse_error.tr` backfilled; `mean.tr` stays the not-yet-lowered
exemplar with `lowering: null`), and `parse_row.soil` plus
`csvstats/_private.soil` (`_parse_cell`) are **authored** so
`parse_row.lock`'s private-helper and call edges describe real files
— reviewed like any normative example. Provenance goes honest:
`human-verified` with provider/model absent (the Soil is
hand-written; no agent ran). Drift is reviewed with the user at each
stage.

Tests: every regenerated `examples/*.lock` re-emitted byte-identical
through read → write; invariant rejection cases; manifest merge
golden including helper GC.

### Step 6 — incremental state and the check pipeline

The definition graph (references from `soil0 rename` reference sets;
tests' oracle edges; type cycles), the status ladder derived per
lock-schema §8, the full invalidation table applied on refresh, and
the static pipeline: `.tr` validity → env/program generation →
`soil0` check in-process → check facts into the lock. Every
diagnostic leaves through `diag.rs` and therefore carries
`repair_class` and `spec_ref` (step 1's registry).

Decision staleness is part of the derived pass (§9.4): compare
`lowering.decisions` edges and scope hashes against current entry
hashes; when a change is detected the CLI reports which lowerings it
flags and names the reclassification escape.

CLI: `trellis check [def]`, `trellis status` (human table; `--json`),
`trellis refresh`, `trellis decisions editorial <label>` (re-stamp a
detected change the human declares meaning-preserving; `--module <m>`
when the label exists in both scopes).

Tests: invalidation-table cases end-to-end (edit file on disk →
refresh → expected statuses); a `review-suggested` prose flow; the
decision flows — edit flags only citers, addition flags the scope
once, editorial re-stamp flags nothing;
status goldens over the fixture root; registry coverage test — every
code the daemon can emit is in the registry (the drift gate's first
half).

### Step 7 — test running

- **env/program generation**: `.tr` type files → `env.json` (§9.1
  resolution), dependency-ordered `program.json` (callee-first — the
  daemon owns the tree; contract §7).
- **Expect tests** → soil0 bundles, verbatim shapes (contract §10);
  results into the lock in the §5 vocabulary.
- **Derived tests** (design §4.1, §4.5): properties from
  requires/ensures/invariant clauses; the differential tier from the
  `reference` attachment; named per §9.5.
- **Property runner** (§8.12): random generators from types (`where`
  filters refused with `unsupported-where-filter` — §9.8; constrained
  generation arrives with Z3, plan 05), pinned seeds, predicates
  evaluated by synthesizing a Bool-returning wrapper definition and
  running it through the interpreter.
- **Differential oracles**: Python references via a `python3`
  subprocess driver, CLI oracles by pipe; both hashed into `oracles`
  and compared through canonical re-encode (§9.9).
- **Contradiction pre-flight + vacuity probes** (design §4.5): the
  mechanical same-input/different-output check over decoded canonical
  args; a `requires` no test input satisfies; a syntactically trivial
  `ensures`; an expect set that never exercises a declared result
  variant. All run before any tokens are spent.

CLI: `trellis test [def]` (sandboxed tiers only).

Tests: bundle-assembly goldens against `examples/` test blocks;
property determinism under a fixed seed; each vacuity probe has a
firing and a non-firing fixture; a deliberately contradictory pair of
expect cases.

### Step 8 — cram and `trellis call`

The cram runner: fresh temp dir per block, `with file` fixtures,
sequential shell session, literal combined-output match, `[n]` exit
codes; built targets and `trellis call` on `PATH`. `trellis call
<def> <json-args>` runs the definition with real `World`-derived
capabilities via the soil0 `run` injection semantics (contract §8.6),
canonical JSON out. Cram results enter the lock with mode `real`;
real mode is never reachable from the lowering sandbox (the MCP
`run_tests` tool simply has no path to it).

Tests: `read_file.tr`'s cram block green against a real temp file;
exit-code and output-mismatch failures; a fixture proving `trellis
call` rejects non-ground or capability-mismatched args cleanly.

### Step 9 — context bundles and the packer

Bundle assembly per the step-1 contract; the budgeted packer with the
fixed priority spec > tests > callee signatures > corpus examples >
module prose, decisions always included per §9.4's resolution;
demoted-refinement rendering in `callees/`; `previous.soil` on
re-lower; the corpus is `examples/` until plan 04 replaces it with
the prelude. CLI: `trellis context <def> --budget <n>` writes the
bundle to a directory and prints the manifest of what was packed and
what was dropped.

Tests: packed-bundle goldens for fixture definitions at generous and
starvation budgets (drop order is observable and pinned); the
never-drop rule for spec; token-estimate monotonicity property.

### Step 10 — the lowering loop against the fake provider

The MCP stdio shim (`trellis _mcp`, §8.8) and the seven tools; the
bubblewrap jail and isolated agent home (§8.9); the `Provider` trait
with the **scripted fake provider** (§8.9) that plays back canned
tool-call sequences; the job loop — serial queue in disciplined
callee-first order, pre-flight, bundle, invoke, per-tool logging to
`f.log`, turn/time caps enforced by the daemon, outcome handling:
green → canonicalized `.soil` moved into place + lock updated;
`ask_human` → question persisted, job ends, `trellis answer` writes
the answer into the `.tr` (§9.7) and re-queues fresh; budget
exhausted with holes → `partial(holes)` committed honestly
(design §3.16); dependency edited mid-flight → job invalidated and
requeued. `write_soil` canonicalizes through the printer and rejects
non-parsing text with structured diagnostics; the job's completion
payload requires the `decisions_applied` citation list (§9.4),
recorded as `lowering.decisions` edges. The **triage sweep** is a
second job kind on the same queue and provider machinery: bundle = a
decision entry's diff plus the flagged definition's spec and Soil;
outcome = conforms (re-stamp the edge) or not (leave flagged, report
in `f.log` and status).

Tests: the whole plan-03 testing bullet — scripted playbacks for the
happy path, the question path, cap exhaustion, invalidation
mid-flight, a `write_soil` of hole-bearing Soil, a citation-recording
playback plus a scripted triage run (one conforming, one not), and
the e2e fixture:
the fixture root's `mean`-sized definitions lower to green entirely
through the fake provider, locks and hashes regenerating
deterministically.

### Step 11 — the live provider

The claude-code provider: headless invocation with the MCP config
pointing at the shim, tool allow-list, `--max-turns`, stream-json
parsed for token/cost telemetry into `f.log` and provider/model into
the lock. Re-verify the headless-subscription quota policy noted in
design §4.6 before relying on it, and record the finding in the
provider doc. One **opt-in** live smoke test (`TRELLIS_LIVE=1`):
a trivial fixture definition lowers bundle → agent → `write_soil` →
checks → tests → lock with `f.log` populated — the plan's second exit
criterion. CLI: `trellis lower <def>`.

### Step 12 — REPL, skill, drift gates, conformance

`trellis repl` (call any definition with JSON args at the Trellis
level, `trellis call` semantics, the future REPL-to-test promotion
hook); `trellis skill` assembling the lowering skill from the
diagnostic registry, the effect lattice, the derivation strategies,
the tool contract, and the corpus, with the committed copy under a
CI drift gate; the second half of the registry drift gate (registry ↔
contract doc ↔ emitted codes); the `skill` pin flips from
warning to required — a missing pin under a generator-capable
toolchain is `toolchain-mismatch` (soil-toml §2.1, resolved
2026-08-25); a final conformance pass over
`docs/contracts/trellis-daemon.md`; the exit-criteria audit (§6).

---

## 6. Exit-criteria traceability

| Plan 03 exit criterion | Where it lands here |
|---|---|
| `examples/` fully round-trips: parse → hash → lock regeneration matches checked-in sidecars | Steps 3–5 (parse, hash, lock goldens; the regeneration pass makes the sidecars real, §9.10); step 6 keeps them stable under refresh |
| One real headless lowering completes with `f.log` populated | Step 11 (live smoke test; telemetry from step 10) |
| Question round-trip: `ask_human` → CLI answer → prose diff → re-invoke → green | Step 10 (fake provider) proves the machinery; step 11 exercises it live if the smoke lowering asks |
| Plan's testing bullets (golden hashes, mutation tests, lock round-trip, scripted fake agent, e2e fixture) | Steps 4, 5, 10 respectively |

---

## 7. Risks and checks

- **Hash reproducibility.** Everything feeding a hash must be bytes
  the repo controls: `.tr`/`.soil` are hashed as raw bytes with
  `.gitattributes` pinning `eol=lf`, paths never enter any hash
  (§8.5), and the reproducibility test runs the tree from two
  locations. The alpha-normalized hash form (§9.2) is specified
  before step 4 writes a single hash.
- **bubblewrap availability.** Unprivileged user namespaces are off
  on some kernels/CI images. `jail.rs` probes at startup; a failed
  probe **refuses to lower** (the sandbox is not best-effort) with a
  diagnostic naming the missing capability. CI runs the jail tests
  only where the probe passes; the fake-provider loop is also
  exercised jail-less so the queue logic stays covered everywhere.
- **The `claude` CLI is unpinned.** It is an ambient tool with a
  moving flag surface, and headless quota policy has shifted
  recently (design §4.6). Containment: the provider is one module
  behind the trait, the live test is opt-in, and step 11 starts by
  re-verifying invocation flags and quota policy.
- **Python reference oracles and float text.** Python's `repr` is not
  the canonical float form. The driver never compares strings from
  Python: its JSON is parsed and re-encoded through `soil-rt`'s
  canonical encoder before comparison, same normalization as test
  expectations (impl plan 02 §8.10).
- **Serial queue vs a stuck agent.** The wall-clock cap is enforced
  by the daemon (kill the process group), not trusted to the agent
  CLI, or one hung lowering blocks the queue forever.
- **Lock canonical-writer drift vs examples.** Prevented structurally
  the plan-02 way: the byte-identity test over regenerated examples
  runs in `check.sh` from step 5 onward.
- **Packer token estimates are approximate** (§8.10). Accepted: the
  budget is a packing target, not a hard API limit; the provider's
  own context limit is the backstop and the estimate constant is one
  number in `providers.toml`.
- **prose write-back merge conflicts.** `trellis answer` re-parses
  and re-hashes the `.tr` before writing; if the file changed since
  the question was asked, the answer is refused with a diff rather
  than blindly appended (the invalidation rule already requeued the
  job).

---

## 8. Micro-pins — approved 2026-08-24

Per the overview's rule, these were presented as proposals and
**approved by the user on 2026-08-24** (all sixteen, as written); each
lands in the step-1 contract docs where user-visible, and reopening
one is a new decision point. None are language-observable.

1. **One binary, auto-spawned daemon.** `trellis` is both client and
   server: `trellis daemon run` serves; every other subcommand
   connects to `$XDG_RUNTIME_DIR/trellis/<root-hash>.sock` (root-hash
   = first 16 hex of SHA-256 of the canonicalized absolute root path)
   and spawns `trellis daemon run` for the root if absent. Explicit
   `trellis daemon stop|status`; no idle shutdown in v1. A version
   mismatch between client and daemon restarts the daemon (one
   binary, so mismatch means an upgrade happened).
2. **RPC framing**: JSON-RPC 2.0, one compact JSON document per line
   (ndjson) both directions. Not contractual. Threads + `Mutex`
   state, a dedicated job-runner thread for the serial queue; no
   async runtime (dependency-floor precedent, impl plans 01–02).
3. **CommonMark via `pulldown-cmark`.** Fenced-block extraction has
   real corner cases (tildes, indented fences, fences inside quoted
   or listed prose) and the crate is small and pure; hand-rolling was
   rejected as a corpus of subtle divergences from the CommonMark
   `.tr` files are defined to be. Only block extraction is used;
   prose is never rendered.
4. **Frontmatter is a hand-rolled two-key YAML subset**: `name:
   <scalar>` and `tags: [a, b]` / block-list form, nothing else —
   tr-grammar makes unknown keys errors, so the subset is the format.
   A YAML crate would be a large dependency for two keys.
5. **Three-part hash canonicalization.** The parsed file partitions
   into spans; each hash is SHA-256 over a length-prefixed
   concatenation, in document order, of its class's parts:
   `formal` = the frontmatter `name` value + every formal-class
   block's (info string, content bytes); `test` = every test-class
   block's (info string, content bytes) — the info string includes
   `xfail`, so toggling it re-runs what depends on `test_hash`;
   `prose` = the remaining file bytes verbatim (frontmatter minus
   `name`, prose, unreserved fences) with formal/test **and
   `decisions`** block spans (fences included) elided — decisions are
   their own class (tr-grammar §1, resolved 2026-08-24): each entry
   hashes as SHA-256 over its bytes from the label line through its
   `rejected:` lines, and the scope hash covers the sorted labels,
   NUL-separated. Length-prefixing prevents concatenation ambiguity;
   no whitespace normalization inside blocks (the bytes are the
   spec); class membership per tr-grammar §1. The info string
   includes the `@agent` marker (resolved 2026-08-28): removing it —
   a human adopting a block unchanged — is a formal event that moves
   the class hash and re-verifies, chosen over content-only hashing;
   recorded in tr-grammar §8.
6. **`soil_hash` recipe.** SHA-256 over: a format tag (`soil-hash/1`);
   the definition's canonical **hash-form** text (§9.2); then, sorted
   by name, one `name NUL referent-hash` line per free reference —
   callee definitions by their `soil_hash`, private helpers by the
   same recipe applied recursively (this is the "folded in": a
   helper's change moves every owner), types by their `formal_hash`
   (cycle members by the cycle hash), builtins by
   `soil0-builtin/<soil0_cli major>`. The cycle hash is SHA-256 over
   the members' canonical `soil-type` texts sorted by name. Rationale:
   substitution-by-edge-list gives the Unison property (change
   propagates hash-by-hash) without rewriting names inside printed
   text.
7. **Lock file surface form**: UTF-8, 2-space-indented pretty JSON,
   key order exactly the schema tables' order, elements of `blocks`,
   `tests`, `oracles`, `calls`, `private_helpers` printed one object
   per line (the checked-in examples' shape — diff-per-row), one
   trailing newline. `soil.lock` same style. Recorded in lock-schema
   §1 as the normative surface form.
8. **MCP is hand-rolled over a stdio shim.** The agent CLI spawns MCP
   servers as processes, so the daemon's MCP face is `trellis _mcp
   --job <id>`: a shim speaking MCP (JSON-RPC 2.0: `initialize`,
   `tools/list`, `tools/call`) on stdio and forwarding to the
   daemon's socket. The protocol subset for seven tools does not
   justify an SDK and its async runtime. The shim binary inside the
   jail is the same pinned `trellis`.
9. **Jail profile and providers.** bubblewrap with: fresh tmpfs
   `HOME`, the bundle's scratch dir bound read-write, the `trellis`
   binary and the agent CLI's install (plus CA certs, resolv.conf)
   read-only, network **allowed** (the agent must reach its API), the
   project tree **not** mounted, environment scrubbed to an
   allow-list. Provider config lives in
   `~/.config/trellis/providers.toml` (user-level, never in the
   repo): provider kind, model, turn/time/cost caps, context budget
   per model. The **scripted provider** replays a JSONL playbook of
   `{tool, args}` calls through the real shim path (so the loop,
   logging, and caps are exercised), asserting each result against
   an expectation; playbooks live in `tests/scripts/`.
10. **Packer accounting**: tokens ≈ bytes/4, the constant recorded in
    `providers.toml` per model. Priority is fixed (design §4.6):
    spec > tests > callee signatures > corpus examples > module
    prose; decisions travel with the spec tier (§9.4); `spec.md` and
    `tests.json` are never dropped — a budget too small for them is
    an error, not a silent truncation; every drop is listed in the
    bundle manifest and `f.log`.
11. **soil0 in-process.** All checking, running, and test execution
    call the `soil0` library (the resolved plan-03 decision); one
    integration test per command class asserts library and shelled
    binary agree byte-for-byte on the examples, keeping the CLI
    contract honest as the daemon's oracle.
12. **Property execution.** For each property the daemon synthesizes
    a private wrapper definition whose params are the `forall`
    binders and whose body is the predicate lowered to Soil (the
    predicate grammar is a Soil-expression subset; `implies`
    becomes `not a or b`, `is` becomes a match), checked and run
    in-process; a case passes iff the wrapper returns `True`. Case
    counts by effect row (design §4.5): 128 for pure signatures, 32
    when fakes are involved. Seeds are pinned by derivation:
    SplitMix64 seeded from the first 8 bytes of `test_hash`, so runs
    are reproducible and re-seed exactly when the tests change.
    Generators: uniform-with-boundary-bias for integer widths, finite
    `F64` plus the specials, length-geometric lists and `Utf8`,
    field/variant-recursive for records and sums (depth-capped),
    comparator-consistent `Map`s. `where` filters per §9.8.
13. **`trellis call` and the REPL** reuse contract §8.6 injection
    semantics exactly: capability params from a real `World` in
    order, remaining params decoded positionally, canonical JSON out,
    structured `runtime-panic` rendering on error. The REPL is a
    readline loop over the same RPC; no state between lines in v1.
14. **`f.log`** is JSONL, one event per line: `job_started` (def,
    hashes, provider, model, budget), `preflight`, `bundle_packed`
    (contents, drops, estimate), `tool_call` (name, duration, result
    digest), `question`, `agent_result` (turns, tokens in/out, cost),
    `outcome` (green / partial holes / failed / invalidated), plus
    per-test failure details (the lock stores results only,
    lock-schema §5). Human-readable, gitignored, never an input
    (design §4.6).
15. **Question persistence**: an open question is
    `<def>.question.json` beside the `.tr` (gitignored, like
    `f.log`): the structured question, the bundle hash, and the
    spec hashes it was asked against. `trellis status` surfaces it;
    `trellis answer <def>` (interactive or `--text`) validates
    staleness (§7), applies the write-back (§9.7), deletes the file,
    and re-queues.
16. **Daemon-owned diagnostics** get their own code namespace in the
    same registry (`tr-` prefix for `.tr` validity, `lock-`, `job-`,
    `toolchain-mismatch`, …), kebab-case like soil0's; the registry
    file is the union and the drift gate covers both halves.

---

## 9. Spec gaps and new decision points — all resolved

Raised per the overview's rule; all were resolved with the user
(§9.4 on 2026-08-24 in its own session; the rest on 2026-08-24 in the
review pass) and propagated to the spec docs as noted.

1. **`env.json` inline-record variant payloads** (contract §13.5 had
   deferred this to the daemon's env generator; bites at step 7).
   *Resolved: contract v1.2* — `VariantD` gains
   `fields : Option (List FieldD)` (payload and fields mutually
   exclusive), soil0 registers the record payload directly; additive
   like v1.1 (v1.1 env files decode with `fields = None`). The soil0
   side (env decoding + registration) is implemented as part of step
   7. Rejected: generator-synthesized hidden record types (reserved
   names leaking into diagnostics and `show`). Recorded in
   `docs/contracts/soil0-cli.md` §6/§13.5.
2. **The alpha-normalized hash form was never actually specified.**
   *Resolved: syntax-spec §9.1 "Hash form"* — the canonical text with
   local binding sites numbered `%N` in pre-order and occurrences
   printing their binder's number; definition names, callees, types,
   constructors, fields, and hole names untouched; emitted by a
   `soil0` **library** function, not a CLI command — the CLI contract
   stays the display-form oracle, soilc's printer is differentially
   tested against `print`, and the hash transform is one shared
   implementation above it. A `print --hash-form` flag remains a
   possible future additive amendment; none planned.
3. **`formal_hash` and the "import set".** *Resolved: tr-grammar is
   the authority* — `formal_hash` is spec-side only; callee identity
   flows through `lowering.calls[].hash` edges (already how
   lock-schema §8 routes caller invalidation; folding the computed
   import set into the spec hash would make it depend on the artifact
   it gates). Design §6.2's wording amended.
4. **Decisions: flag or invalidate** — *resolved 2026-08-24 as
   neither: reliance edges + editorial reclassification + triage
   sweep*, recorded in design §4.3, tr-grammar §1/§5.2/§9, lock-schema
   §2/§3/§8. Both binary options scaled with project size (scope-wide
   `review-suggested` fatigue, or a typo re-lowering the world);
   reliance scales with actual use. Consequences for this plan:
   decision entries are hashed **individually** and stored per label
   in the module/project locks (step 4/5); the lowering's completion
   payload gains a required `decisions_applied` label list (possibly
   empty), recorded as `lowering.decisions` edges plus the
   scope-membership hash (steps 5, 10; the citation requirement goes
   into the step-1 tool contract and the generated skill); decision
   staleness is *derived* by hash comparison in the invalidation pass
   (step 6); `trellis decisions editorial <label>` re-stamps a
   detected change the human declares meaning-preserving; and the
   **triage sweep** is a job kind on the same queue and provider
   machinery as lowerings — bundle: the entry's diff plus the flagged
   definition's spec and Soil; outcome: conforms (re-stamp) or not
   (leave flagged, report) (step 10).
5. **Derived-test naming.** *Resolved: the namespaced scheme* —
   `derived:ensures:<label>` / `derived:requires:<label>` /
   `derived:invariant:<label>`, `derived:differential:<reference-line>`,
   `derived:contract:<n>` reserved for plan 06. Chosen over the bare
   clause-label shapes in the pre-daemon example locks because the
   `derived:` prefix can never collide with a spec block name; the
   regeneration pass (step 5) rewrites the examples. Recorded in
   lock-schema §5/§9.
6. **Lock-schema §9 leftovers.** *Resolved:* module/project entries
   **carry `accepted`** (a human can accept exports and decisions;
   ungated — they have no tests) while CI policy and badges keep
   keying on exported function definitions; the prelude-fork hash
   stays **global** in `soil.toml` `trusted_packages` (per-entry pins
   would repeat one hash until per-entry forks exist, and the
   toolchain pin already refuses under a changed prelude). Recorded
   in lock-schema §8/§9.
7. **The `ask_human` write-back format.** *Resolved:* function-scoped
   answers land in a `## Clarifications` prose section (created once,
   at the end of the file) as `- **Q:** …` / `**A:** …` pairs —
   plain prose under `prose_hash`, visible in the review diff;
   rule-shaped answers (the CLI asks which kind) go to the module's
   or project's `decisions` block, the tr-grammar §5.2 write-back
   target. The IDE later adds a fold-into-prose action: an agent
   absorbs a Q/A pair into the prose proper and deletes it, as an
   ordinary prose edit. Recorded in tr-grammar §8 and design §4.6;
   rejected alternatives noted there.
8. **`where`-filter support** (tr-grammar §3.4 mandates constrained
   generation, not rejection sampling — full generality is a
   constraint solver). *Resolved: the trivial v1* — generators are
   random from the binder's type; a `where` filter is **refused**
   with a structured `unsupported-where-filter` error (never silently
   ignored — that would test outside the intended domain — and never
   rejection sampling, which tr-grammar forbids). Constrained
   generation is deferred to plan 05, when Z3 lands for the
   refinement checker: solver-backed generation for in-fragment
   predicates with model-blocking randomization — with the caveat
   recorded that raw solver models cluster (uniform SMT-space
   sampling is unsolved), so distribution quality gates adoption.
   Coverage-guided fuzzing (libFuzzer-style) was considered and
   rejected: coverage feedback would reflect the soil0 interpreter's
   branches rather than the Soil program's, it needs the
   nightly/sanitizer plumbing impl plan 02 §8.12 already rejected,
   and corpus evolution conflicts with pinned-seed reproducible
   results. Recorded in tr-grammar §3.4 (toolchain note) and design
   §10 (deferred row).
9. **Differential tests before the Python FFI exists.** *Resolved:
   the interim subprocess runner* — `python3` (from `shell.nix`) with
   a small driver importing `relpath::symbol`, feeding the test's
   JSON args, comparing through canonical re-encode; CLI oracles
   likewise by pipe. The lock is transport-agnostic (tier + oracle
   hashes), so plan 06 swaps in the real FFI without a schema change.
   Rejected: stripping differential rows from the examples until plan
   06 (weakens the round-trip exit criterion).
10. **Examples stop being fakes.** *Resolved: regenerate.* The step-5
    pass replaces abbreviated placeholder hashes with real 64-hex
    digests and updates the stale `checks` shapes (the current
    `median.lock` predates per-clause `refinements`, `termination`,
    and `holes`) and the old derived-test names (§9.5). The repo
    convention is amended in `CLAUDE.md` (2026-08-24): abbreviated
    fakes remain the style for **docs prose**; `examples/*.lock`
    carry real digests and are regenerated, never hand-edited.
    Missing sidecars are backfilled or documented during the pass
    (`mean.tr` stays the not-yet-lowered example), reviewed with the
    user. *Policy resolved 2026-08-28 (see step 5): staged
    regeneration (step 5 partial, step 7 final byte-identity), locks
    for every `.tr`, `parse_row.soil` + `_private.soil` authored,
    provenance `human-verified` with provider/model absent.*

11. **Step-7 resolutions (2026-08-28, with the user).** Five
    decision points surfaced while building the test runner:
    - *Invariant-derived properties are refused in v1*
      (`unsupported-invariant-property`): generating `self` from the
      bare structure tests exactly the values the invariant excludes —
      the honest generator is constrained generation, deferred with
      `where` filters to plan 05. Consequence: *type entries are
      accepted-ungated in v1* (chosen over flipping `Row` to
      unaccepted). Recorded in tr-grammar §4.2, lock-schema §8,
      design §10. Rejected: checking the invariant over expect-test
      values (a different, weaker check wearing a property's name) and
      honest-failure rows (breaks the flagship example for a toolchain
      gap, not a spec bug).
    - *Definition-oracle rows are deferred* to plan 04: the daemon
      records `reference`/`cli` oracle rows only. Recording (and
      enforcing acceptance for) every helper a predicate mentions
      would today flag `len`/`min`/`max`/`sort` — vocabulary the
      prelude will absorb. Recorded in lock-schema §5, design §4.5.
    - *Differential inputs are the expect cases' args* (§9.9's
      "feeding the test's JSON args" read literally); generated-input
      replay was rejected as seed-coupled and generation-quality-bound.
    - *Expect/cram row naming is always `block#k`* (property rows stay
      bare); lock-schema §5's stray "bare name for single-case blocks"
      sentence was corrected to match its own example and the corpus.
    - *The F64 generator keeps §8.12 as pinned* (finite plus the
      specials). The flagship specs were **improved instead of the
      generator weakened**: testing exposed that Soil's `F64`
      comparisons are the derived total order (design §3.7 — one
      logical NaN sorting last, `-0.0 < 0.0`), so `sort` is fully
      specified with *no* domain guard (`sorted` + `is_permutation`
      ensures), `median` keeps an `all_finite` requires because the
      mean of `±Inf` middles is NaN (which sorts past `+Inf`, escaping
      the bounds), `median.soil`'s even-length mean became
      overflow-safe, and `examples/csvstats/` gained a predicate
      vocabulary suite (`finite`, `all_finite`, `non_empty`, `sorted`,
      `contains`, `count_of`, `is_permutation`) — authored like
      `parse_row.soil`, provenance `human-verified`.

12. **Step-8 resolutions (2026-08-28, with the user).**
    - *`trellis call` executes in the client*, not over the daemon
      socket: it links soil0 anyway, and client-local execution makes
      the real `Fs`'s relative paths resolve against the caller's
      working directory — the cram temp dir — for free. The daemon
      route was rejected as needing either a carried-cwd/rooted-Fs
      mechanism or a per-call `chdir` in a threaded process. Contract
      §8.6 injection semantics are shared with `soil0 run` through the
      library (`interp::run_json`), so the two cannot drift; micro-pin
      §8.13's REPL decision is untouched (step 12 revisits transport).
    - *Root discovery honors `TRELLIS_ROOT`* before the `soil.toml`
      walk (recorded in soil-toml §1): the cram runner exports it so
      transcripts in temp dirs can call back into their root. Rejected:
      temp dirs under `<root>/.trellis/` (transcripts writing inside
      the project tree).
    - *`trellis test` runs cram (mode `real`) by default* — the
      real-mode exclusion is about the lowering sandbox's `run_tests`
      tool, not the human CLI; transcripts are hermetic temp-dir
      sessions the human wrote. The step-7 carried-rows mechanism is
      removed.
    - `read_file.tr`'s transcript expectation corrected to canonical
      compact JSON (`{"tag":"Ok",…}`) — the hand-written spaced form
      predated the real runner; cram matches literally and canonical
      JSON is the §7 byte-equal form.
