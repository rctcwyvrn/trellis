# The Trellis Daemon — Agent-Facing Contract

*Status: **frozen — contract v1** (drafted as impl plan 03 step 1;
§10 open questions resolved with the user on 2026-08-25, adding
`read_spec` to the v1 surface; approved in full 2026-08-25). Changing
anything here is a contract change requiring a user decision;
additive amendments follow the soil0-cli precedent. This is the
contract of the daemon's agent- and user-facing surfaces: the MCP tool surface the lowering agent is allow-listed to,
the context-bundle layout it reads, the diagnostic-registry format and
`spec_ref` scheme, and the question/answer schema. These are the
surfaces plan 05's strangler swap and every future provider must keep
stable. Explicitly **not** contractual: the CLI↔daemon JSON-RPC (both
ends live in one binary and ship together), the `trellis` library API,
`providers.toml`, `f.log` beyond its documented event kinds (§9), and
diagnostic message *wording* (codes, repair classes, and `spec_ref`s
are contractual; prose is not — the soil0-cli §12 rule). References:
design §4.6 (lowering, tools, packer), tr-grammar (blocks, decisions,
write-back §8), lock-schema (§3 edges, §5 test names),
`docs/contracts/soil0-cli.md` (§8.4–§8.6 outputs, §10 bundles, whose
shapes are reused verbatim), impl plan 03 §8 (approved micro-pins).*

---

## 1. Conventions

- JSON documents follow tr-grammar §7 conventions wherever the data is
  Soil-shaped: internally tagged sums, records with every field
  present, `Option` encoding for optionality, booleans as JSON
  `true`/`false`. Soil *values* are produced only by `soil-rt`'s
  canonical encoder.
- Schemas below are written as Soil type declarations, encoding per
  tr-grammar §7 (the soil0-cli §4 device).
- Hashes are `sha256:` + 64 lowercase hex. Paths inside bundles are
  bundle-relative, `/`-separated, and never contain `..`.
- This contract has a version, surfaced to the agent in the MCP
  `initialize` response (`serverInfo.version` carries the `trellis`
  crate version; the skill states the contract version). Additive
  amendments follow the soil0-cli precedent: recorded here, dated,
  old inputs unaffected.

## 2. The MCP surface

The daemon's MCP face is the stdio shim: the agent CLI spawns
`trellis _mcp --job <id>` per its MCP config, and the shim forwards to
the daemon's socket (impl plan 03 §8.8). The shim implements the MCP
subset a tools-only server needs — `initialize`, `tools/list`,
`tools/call`, and notifications — as JSON-RPC 2.0 over stdio.

- `tools/list` returns exactly the seven tools of §3, with the input
  schemas below as JSON Schema. **The tool allow-list is the
  sandbox** (design §4.6): there is no shell tool, no filesystem tool,
  and no real-capability path; `run_tests` runs sandboxed tests only.
- Every `tools/call` result carries exactly **one** `text` content
  item containing exactly **one compact JSON document** (the soil0
  stdout discipline, applied to tool results). Structured failures
  set `isError` and the text item is a diagnostics document (§6
  shape) — the agent always parses one JSON document either way.
- Every tool call is logged by the daemon to `f.log` (§9) with
  duration and a result digest; the daemon, not the agent CLI,
  enforces the job's turn, wall-clock, and cost caps.

## 3. The seven tools

### 3.1 `read_context`

```
input:  { path : Utf8 }
output: File { content : Utf8 } | Dir { entries : List Utf8 }
```

Reads a file or lists a directory inside the job's context bundle
(§4). `entries` are sorted names, directories suffixed `/`. A path
outside the bundle or not found is an `isError` result
(`bundle-path`, `bundle-not-found`).

### 3.2 `check_types`

```
input:  {}   -- operates on the current draft (§3.5)
```

Runs the full static pipeline — the daemon-assembled environment and
callee-first program, then the soil0 library equivalent of
`soil0 check` — over the current draft. Success output is the
soil0-cli §8.4 document for the definition under lowering (declared
signature, row, `checks` facts including `holes` with goal types —
a draft with holes *checks*; design §3.16). Failure output is the
combined diagnostics, each enriched with `repair_class` and
`spec_ref` (§6). Calling with no draft written yet is `isError`
(`no-draft`).

### 3.3 `check_refinements`

```
input:  {}
output: {"tag":"None"}
```

A stub until soilc (plan 05): the `Option` encoding's `None`,
meaning "no refinement checker is available — clauses are recorded
`runtime`". When soilc arrives this returns
`Some { clauses : Map Utf8 Assurance }` in the lock-schema §4
vocabulary; the stub shape is chosen so that arrival is additive.

### 3.4 `run_tests`

```
input:  {}
output: { cases : List CaseResult }    -- soil0-cli §10 output, verbatim
```

Runs every sandboxed case in `tests.json` (§4.5) — spec and derived —
plus the property tiers, against the current draft, with fakes
constructed per the bundle cases. Property inputs are generated
daemon-side under the pinned seed (impl plan 03 §8.12), so the agent
sees the same cases the final gate will run; their results report
under the property's name (spec properties) or `derived:` name
(clause-derived). Real-mode (cram) tests are not present in the
bundle and not runnable here, by construction. A draft with unfilled
holes is refused with the soil0 `unfilled-hole` diagnostic (contract
§8.6); no draft is `no-draft`.

### 3.5 `write_soil`

```
input:  { soil : Utf8
        , decisions_applied : List DecisionRef }
type DecisionRef = { scope : Scope, label : Utf8 }
type Scope      = Project | Module
output: { canonical : Utf8 }
```

Parses and **canonicalizes** the draft through the printer
(soil-syntax-spec §9) — the agent never controls formatting (design
§4.8) — and stores it as the job's current draft. The echoed
`canonical` text is what every later check, test, and hash sees.
Parse failures are `isError` with diagnostics; nothing is stored.

`decisions_applied` is the **required citation** (design §4.3,
resolved 2026-08-24): the decision entries, by scope and label, the
agent applied in this draft — `[]` when none. Each call restates the
full list; the last successful `write_soil` before a green outcome
is what the lock records as `lowering.decisions.applied`
(lock-schema §3). An unknown label is `isError`
(`unknown-decision`).

### 3.6 `read_spec`

```
input:  { ref : Utf8 }            -- a §7 anchor, e.g. "soil-syntax-spec#5.9"
output: { title : Utf8, text : Utf8 }
```

Serves one pinned spec section by anchor (added to v1 on 2026-08-25,
resolving §10.2: diagnostics carry `spec_ref`s, so the agent must be
able to dereference them — "the whole spec is never shipped blind",
design §4.6). Sections come from the **toolchain's embedded copies**
of the §7 documents, never the repo working tree: the spec an agent
reads must be the one the toolchain that checks it was built from
(the design §8 pin rationale). An unresolvable anchor is `isError`
(`unknown-spec-ref`). The skill still carries the budgeted core
reference; this tool is the lookup path for everything beyond it.

### 3.7 `ask_human`

```
input:  { kind : Ambiguity | Contradiction | Rule
        , question : Utf8
        , hole : Option Utf8          -- a hole name this blocks on
        , options : List Utf8 }       -- proposed answers, may be empty
```

Ends the invocation (design §4.6): the daemon acknowledges the call,
persists the question (§8), and terminates the job. `Contradiction`
is the distinguished class for spec-internal conflict the pre-flight
could not catch mechanically; `Rule` marks an answer expected to be a
project/module rule — its answer is written to a `decisions` block
rather than the definition's prose (tr-grammar §8). The agent never
sees an answer that is not already in the spec: the answered question
re-runs the job fresh.

## 4. The context bundle

The fixed directory layout (design §4.6) the job's `read_context`
serves. Assembled by the budgeted packer (§5); also written to disk
by `trellis context <def> --budget <n>` for human inspection.

```
bundle/
  bundle.json          -- the manifest (§4.1)
  spec.md              -- §4.2
  decisions.md         -- §4.3
  callees/<name>.md    -- §4.4, one per direct callee
  tests.json           -- §4.5
  examples/            -- §4.6
  reference.py         -- §4.7, when a Python reference is attached
  previous.soil        -- §4.8, when re-lowering
```

### 4.1 `bundle.json`

```
type BundleManifest =
  { definition : Utf8                  -- e.g. "csvstats/median"
  , budget : U64                       -- tokens (estimate: bytes/4)
  , estimate : U64                     -- packed-size estimate
  , packed : List Utf8                 -- bundle-relative paths
  , dropped : List Dropped             -- §5; never silent
  , oracle : Option Oracle             -- §4.7
  , spec_hashes : SpecHashes }         -- what this bundle was built from
type Dropped    = { item : Utf8, reason : Utf8 }
type Oracle     = Reference { path : Utf8, symbol : Utf8 }
                | Cli { command : Utf8 }
type SpecHashes = { formal : Utf8, test : Utf8, prose : Utf8 }
```

`spec_hashes` is the staleness anchor: mid-flight invalidation and
question staleness (§8) compare against it.

### 4.2 `spec.md`

The definition's `.tr` file, **verbatim** (frontmatter included).
Never dropped by the packer.

### 4.3 `decisions.md`

The decisions in scope, project section first, then the module's,
each entry rendered with its scope and label:

```
# Decisions

## project

- **timestamps** — all timestamps are UTC seconds (I64)
  rejected: local time (DST bugs); F64 epoch (precision loss)

## module csvstats

- **cell-errors** — BadCell carries index and raw text
```

These are the labels `write_soil.decisions_applied` cites. Absent
when no decisions exist in scope; travels at spec priority (§5).

### 4.4 `callees/<name>.md`

One file per direct callee: **signatures only, never bodies** (design
§4.6). Content: the callee's `soil-sig` block; its
`requires`/`ensures` clauses, each annotated with its lock-schema §4
assurance; a `runtime` (demoted) clause is shown as the **base type
plus a demotion note** — the agent must not rely on an unproven
claim:

````
# sort_by

```soil-sig
sort_by : (key : a -> k) -> (xs : List a) -> List a
```

- ensures `stable` (proven): …
- ensures `sorted` (runtime — demoted, guard active; do not rely on
  this claim when reasoning about your own obligations): …
````

### 4.5 `tests.json`

```
{ "cases": [ …soil0-cli §10 case shape… ] }
```

Exactly the soil0 bundle-case shape — one format — minus the
`program` key (the daemon owns program assembly). Contains every
**sandboxed** case: the spec's expect tests, property cases the
runner will generate are *not* enumerated (properties appear as their
`forall` text inside `spec.md`; generated inputs are not bundle
content), and derived expect-shaped cases under their lock-schema §5
`derived:` names. Never dropped by the packer.

### 4.6 `examples/`

The corpus (design §5): `.tr`/`.soil` pairs. Until the prelude exists
(plan 04) this is the repo's `examples/`; afterwards, the pinned
prelude corpus. Dropped last among droppables, whole pairs at a time.

### 4.7 `reference.py` / the CLI stanza

A Python reference attachment (tr-grammar §3.6) copies the referenced
file verbatim as `reference.py`, with the symbol named in
`bundle.json.oracle`. A CLI oracle contributes no file — the stanza
in `bundle.json.oracle` is the whole record (the agent cannot run
either; they are context, and the differential tier runs
daemon-side).

### 4.8 `previous.soil`

On re-lowering: the current checked-in canonical Soil. Absent on a
first lowering.

## 5. The packer

`trellis context <def> --budget <n>`; the same assembler feeds every
job. Fixed priority (design §4.6, adopted 2026-08-23):

```
spec (spec.md, decisions.md, bundle.json)  >  tests (tests.json)
  >  callee signatures  >  corpus examples  >  module prose
```

- Budget is in tokens, estimated at bytes/4 (impl plan 03 §8.10); the
  per-model budget lives in the provider config.
- `spec.md`, `decisions.md`, `bundle.json`, and `tests.json` are
  **never dropped**: a budget too small for them is a structured
  error (`budget-too-small`), not a silent truncation.
- Dropping is coarsest-first within a tier (a whole callee file, a
  whole example pair) and every drop is listed in
  `bundle.json.dropped` and logged — no silent caps.
- Module prose (`_module.tr`'s prose, lowest tier) is packed as
  `module.md` when it fits.

## 6. Diagnostics, the registry, and repair classes

Every diagnostic the daemon emits — its own and soil0's passed
through — carries:

```
type Diagnostic = { code : Utf8            -- kebab-case, from the registry
                  , message : Utf8         -- non-contractual wording
                  , file : Option Utf8
                  , span : Option Span     -- soil0-cli §3 shape
                  , notes : List Note
                  , repair_class : Utf8    -- from the registry
                  , spec_ref : Utf8 }      -- §7 anchor
```

An error document is `{ "errors": List Diagnostic }` (the soil0-cli
§2 shape, extended by the last two fields).

### 6.1 The registry file

`registry/diagnostics.json` in the `trellis` crate is the single
source; the skill and this contract's rendered tables are generated
from it, and **CI fails when registry, docs, and emitted codes
disagree** (design §4.6). The *format* is contractual; the *contents*
grow additively and are governed by the drift gate, not frozen here:

```
type Registry     = { registry_format : U64        -- 1
                    , repair_classes : List RepairClass
                    , codes : List CodeEntry }
type RepairClass  = { name : Utf8, summary : Utf8 }
type CodeEntry    = { code : Utf8
                    , source : Utf8                -- "soil0" | "trellis"
                    , repair_class : Utf8
                    , spec_ref : Utf8 }
```

- Every soil0-cli §2 code appears with `source: "soil0"`, unchanged;
  the registry adds only the mapping (soil0's own outputs stay
  byte-identical — the enrichment happens daemon-side).
- Daemon codes are namespaced by prefix: `tr-…` (`.tr` validity),
  `config-…` (`soil.toml` / provider config), `lock-…`, `bundle-…`,
  `job-…`, `preflight-…` (the design §4.5 contradiction check and
  vacuity probes — step 7), plus `toolchain-mismatch`,
  `budget-too-small`, `no-draft`, `unknown-decision`,
  `unknown-spec-ref`, `unsupported-where-filter`,
  `unsupported-invariant-property`, `test-ungenerable-type`, and
  `oracle-error`.

### 6.2 Initial repair classes

The starting vocabulary (typed actions an agent takes mechanically,
design §4.6); extension is additive and drift-gated:

| Class | Acts on |
|---|---|
| `rename-binding` | `shadowing` and collision errors |
| `qualify-name` / `unqualify-name` | the §5.9/§5.10 exactly-one-spelling errors |
| `widen-match` | `non-exhaustive-match` (the witness note names the arm to add) |
| `remove-redundant-arm` | `redundant-arm` |
| `add-record-rest` | `missing-record-rest` |
| `add-annotation` | `annotation-needed`, `operator-polymorphic` |
| `fix-type` | `type-mismatch`, `unknown-field`, arity errors |
| `thread-capability` | `effect-violation` (an `io`/`ffi` callee needs its capability and row threaded) |
| `fix-literal` | `literal-out-of-range` |
| `insert-guard` | panic-obligation repairs (match the zero/overflow case away) |
| `add-decreases` | termination deficits (parsed now, discharged plan 05) |
| `export-type-or-mark-abstract` | the design §3.14 export error |
| `fill-hole` | `unfilled-hole` at run/test time |
| `fix-spec` | contradictions and `.tr` validity errors only a spec edit can fix — the class whose repair is `ask_human` when the block is pinned |

## 7. `spec_ref` anchors

`<doc-slug>#<section>`: slugs `design`, `tr-grammar`, `lock-schema`,
`soil-syntax-spec`, `soil0-cli`, `trellis-daemon`; `<section>` is the
printed section number (`soil-syntax-spec#5.9`, `tr-grammar#3.3`).
The CI drift gate resolves every registry `spec_ref` against the
target document's headers, so renumbering a referenced section fails
the build — which is the point: the Soil spec is pinned, individually
addressable sections a daemon tool serves (design §4.6); `read_spec`
(§3.6) is that tool.

## 8. Questions and answers

`ask_human` persists the question as `<def>.question.json` beside the
`.tr` (gitignored, like `f.log`):

```
type Question = { def : Utf8
                , kind : Ambiguity | Contradiction | Rule
                , question : Utf8
                , hole : Option Utf8
                , options : List Utf8
                , asked_against : SpecHashes    -- §4.1
                , bundle : Utf8                 -- bundle content hash
                , created : Utf8 }              -- RFC 3339, display-only
```

`trellis status` surfaces open questions; `trellis answer <def>`
(interactive, or `--text <answer>`) applies the write-back and
re-queues the job fresh:

- kind `Ambiguity`/`Contradiction` → a Q/A pair in the `.tr`'s
  `## Clarifications` section (tr-grammar §8);
- kind `Rule` → an entry appended to the module's or project's
  `decisions` block (the CLI asks which scope when ambiguous);
- if the `.tr` changed since `asked_against`, the answer is refused
  with a diff (`job-stale-question`) — the invalidation rule already
  requeued or killed the job, and the question may no longer apply.

Every question/answer round-trip is therefore a visible spec diff,
and the re-invoked agent reads the answer from the spec like any
other input (design §4.6).

## 9. `f.log` (documented, non-contractual)

JSONL beside the definition, gitignored, human-readable, **never an
input** (design §4.6). One event per line, `event` plus fields:
`job_started` (definition, spec hashes, provider, model, caps),
`preflight` (contradiction/vacuity findings), `bundle_packed` (the
§4.1 manifest), `tool_call` (name, duration_ms, result digest),
`question`, `agent_result` (turns, tokens in/out, cost, provider
session id), `outcome` (`green` | `partial` holes | `failed` |
`invalidated`, plus per-test failure details — the lock stores
results only, lock-schema §5), `triage` (entry, verdict). Field sets
may grow; nothing may come to depend on them.

## 10. Open questions raised by this draft — resolved 2026-08-25

All four were resolved with the user on 2026-08-25 during the step-1
review:

1. **Registry completeness.** *Resolved: freeze the format, contents
   evolve.* The §6.2 repair classes and the daemon code list are a
   starting vocabulary; step 6 wires them, gaps land additively under
   the drift gate with no per-addition review gate.
2. **A `read_spec` tool.** *Resolved: ship it in v1* — §3.6. The
   diagnostics carry `spec_ref`s, so the agent can dereference them
   immediately; the skill still carries the budgeted core reference,
   and `read_spec` is the lookup path beyond it. Sections are served
   from the toolchain's embedded doc copies, matching the pin.
3. **MCP protocol revision.** *Resolved: pin at implementation time*
   — steps 10/11 pin the revision the Claude Code headless client
   then speaks, recorded here when known; the tool surface is small
   enough that revision drift is absorbed in the shim.
4. **`run_tests` case filtering.** *Resolved: all cases, always* —
   the agent always sees the same picture as the final gate. A
   `{ cases : List Utf8 }` filter input remains a possible additive
   amendment if long suites make the loop slow.
