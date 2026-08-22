# Plan 03 — the Trellis daemon

*References: design §4 (the specification layer, lowering, the daemon),
§6 (hashing and locks), docs/tr-grammar.md, docs/lock-schema.md.*

## Goal

The long-running service that makes Trellis a language: parses `.tr`
files, computes the hashes, maintains locks and the derived manifest,
assembles context bundles, runs lowering jobs against an agent CLI with
the MCP tool surface, and serves the CLI (and later the IDE and LSP).
Execution is delegated to `soil0` throughout.

## Scope

1. **`.tr` parsing.** Frontmatter, reserved fenced blocks, block
   mini-languages (signature, requires/ensures/invariant predicates, test
   call-arrow lines with `with` bindings, property, cram, reference,
   exports, allow), validity rules — all per docs/tr-grammar.md. Unknown
   blocks pass through as prose.
2. **Hashing.** The three-part hash (formal/test/prose per tr-grammar §1),
   `soil_hash` with content addressing (free variables replaced by callee
   hashes, private helpers folded in), the recursive-type cycle hash.
   Canonicalization rules documented; hashes must be reproducible across
   machines.
3. **Locks and manifest.** Read/write `.lock` sidecars per
   docs/lock-schema.md in canonical JSON; derive `soil.lock` by merge
   (definitions, `soil_private` nodes, escape-hatch audit, trusted
   packages); enforce the schema invariants (accepted gating, oracle
   acceptance rule, block-author pinning).
4. **Incremental state.** Watch the Soil root; on change, recompute
   hashes, apply the invalidation table (lock-schema §8), update statuses.
5. **Context bundles.** The fixed directory layout per design §4.6:
   `spec.md`, `callees/` (signatures only; demoted refinements shown as
   base type + note), `tests.json`, `examples/` (prelude corpus),
   `reference.py` or CLI-oracle stanza, `previous.soil`.
6. **MCP server + lowering jobs.** The six tools (`read_context`,
   `check_types`, `check_refinements` — a stub returning `none` until
   soilc, `run_tests` (sandboxed, fakes only), `write_soil`, `ask_human`);
   one headless agent invocation per lowering (Claude Code provider
   first) with turn/time/cost caps, isolated agent home, every tool call
   logged to `f.log`; `ask_human` ends the invocation, the answer is
   written into the `.tr` prose, and the job re-runs fresh; serial queue
   with invalidation when a dependency is edited mid-flight.
7. **Test running.** Expect/property via `soil0 test` with fakes;
   contradiction pre-flight (mechanical same-input/different-output check
   before any tokens are spent); cram runner (temp dir, `with file`
   fixtures, literal output, `[n]` exit codes) and `trellis call` (real
   capabilities) — real mode never exposed to the lowering sandbox.
8. **Thin CLI.** `trellis check|status|lower|test|call|repl` speaking to
   the daemon. REPL: call any definition with JSON args (Trellis level),
   feeding later REPL-to-test promotion.
9. **Telemetry.** Tokens, cost, retries, provider, model per lowering, to
   `f.log`; provider/model into the lock's lowering record.

## Non-goals

The IDE (plan 06), LSP beyond bare diagnostics, refinement checking,
concurrent lowerings, raw-API provider, hosted anything.

## Testing

- Hash stability: golden hashes over `examples/`; mutation tests (edit
  prose → only `prose_hash` moves, etc. — one test per invalidation row).
- Lock round-trip: `examples/*.lock` re-emitted byte-identical.
- A scripted fake agent provider (plays back canned tool-call sequences)
  to test the job loop, question channel, caps, and log without spending
  tokens; one live smoke test against real Claude Code headless.
- End-to-end: a fixture project lowers `mean.tr`-sized definitions to
  green through the fake provider.

## Exit criteria

- `examples/` fully round-trips: parse → hash → lock regeneration matches
  the checked-in sidecars (update examples if the daemon exposes spec
  drift — spec first, then code).
- One real headless lowering of a trivial definition completes: bundle →
  agent → `write_soil` → checks → tests → lock, with `f.log` populated.
- A question round-trip works: `ask_human` → IDEless CLI answer → prose
  diff → re-invoke → green.

## Decision points — resolved 2026-08-22

- **IPC:** JSON-RPC over a Unix socket
  (`$XDG_RUNTIME_DIR/trellis/<root-hash>.sock`); LSP speaks its own stdio
  transport; TCP is the future remote-serving path.
- **Language:** Rust, in the `rust/` workspace as `trellis-daemon`,
  linking `soil0` and `soil-rt` (the `soil0` CLI remains the oracle
  contract regardless).
- **Sandboxing: all three layers.** Tool allow-list (the six MCP tools,
  no shell) + the agent CLI's own sandbox and isolated home + a
  chroot-style OS jail around the agent process (unprivileged via user
  namespaces / bubblewrap in practice). Defense in depth from v1.
- **Incremental state:** explicit refresh — hashes re-checked at request
  boundaries plus `trellis refresh`; the watcher arrives with the IDE
  milestone on the same invalidation code path.
