# Milestone Plans — Overview

*These plans are implementation guides for future agents. Each corresponds
to a milestone of the build order (design §11, bootstrap plan) and states
its goal, scope, work breakdown, testing strategy, exit criteria, and open
decision points.*

## How to use these plans

- Read `CLAUDE.md`, then `docs/design.md` for the decisions and reasoning,
  then the plan for your milestone. The plan tells you *what to build and
  in what order*; the specs (`docs/tr-grammar.md`, `docs/lock-schema.md`,
  `docs/soil-syntax-spec.md`) tell you *what the artifacts must look like*.
- **Decision points listed in a plan are not yours to make.** The initial
  sets were all resolved with the user on 2026-08-22 and are recorded in
  each plan; if implementation surfaces a *new* open choice, present
  options to the user and record the outcome in the plan and in
  `docs/design.md` the same way.
- Exit criteria are the definition of done. Do not start the next
  milestone's work early; the ordering is load-bearing (each stage is the
  oracle or substrate for the next).
- When implementation reveals a spec contradiction or gap, stop and raise
  it — spec fixes propagate to `docs/` and `examples/` before code works
  around them.

## Sequence

| Plan | Milestone | Depends on |
|---|---|---|
| [`01-soil-rt.md`](01-soil-rt.md) | The runtime crate (values, JSON, C ABI) | — |
| [`02-soil0.md`](02-soil0.md) | Minimal Rust Soil: parser, checker, interpreter, CLI oracles | 01 |
| [`03-daemon.md`](03-daemon.md) | The Trellis daemon: hashing, locks, lowering jobs, MCP tools | 01, 02 |
| [`04-prelude.md`](04-prelude.md) | The pure-core prelude, first Trellis code | 03 |
| [`05-soilc.md`](05-soilc.md) | The compiler as the first Trellis project; self-hosting | 04 |
| [`06-ffi-bind-ide.md`](06-ffi-bind-ide.md) | FFI (Rust + Python), `trellis bind`, minimal IDE | 05 |
| [`07-python-glue.md`](07-python-glue.md) | The second project: validate the FFI half of the pitch | 06 |

Rust code accumulates in one workspace (`soil-rt`, `soil0`, later the
daemon and the Cranelift driver). Trellis code (prelude, soilc) lives in
Soil roots with `soil.toml`. Nothing in these plans exists yet; the repo is
design-only until plan 01 begins.
