# Trellis

**Trellis** is a specification layer, toolchain, and IDE for programs whose
implementations are written by AI agents. **Soil** is the target language it
lowers to: a small, strict, refinement-typed ML with effect tracking —
the assembly language of vibe-coding, designed to be easy for a machine to
write and easy for a checker to verify, read by humans but rarely written
by them.

The thesis: if an agent writes the implementations, the human-authored
layer should consist of prose specifications, types, tests, and structural
constraints. Humans write `.tr` files (Markdown with formal fenced blocks);
an agent lowers each definition to Soil under the supervision of a type
checker, a refinement checker, and the tests; a per-definition lock file
records the hashes, provenance, and trust status of everything. Nothing
that affects correctness exists only in an agent's context.

**Status: design phase.** No implementation exists yet. The documents below
record every decision made so far and the reasoning behind each.

## Documents

Rendered at [trellis-lang.com](https://trellis-lang.com/).

| Document | What it covers |
|---|---|
| [`docs/design.md`](docs/design.md) | The single reference: vision, Soil language design, the Trellis trust model, hashing and locks, the daemon, the IDE, build order |
| [`docs/tr-grammar.md`](docs/tr-grammar.md) | The `.tr` file format: frontmatter, fenced-block languages, test syntax, the predicate language, the JSON value encoding |
| [`docs/lock-schema.md`](docs/lock-schema.md) | The `.lock` sidecar schema: three-part hashes, per-block provenance, check and test records, the derived manifest |
| [`docs/soil-syntax.md`](docs/soil-syntax.md) | Soil surface syntax, the highlights |
| [`docs/soil-syntax-spec.md`](docs/soil-syntax-spec.md) | Soil surface syntax, elaborated: lexical spec, EBNF, static rules |
| [`docs/bootstrap-plan.md`](docs/bootstrap-plan.md) | The path from a minimal Rust interpreter (`soil0`) to a self-hosted compiler (`soilc`) written in Trellis itself |
| [`docs/plans/`](docs/plans/) | Per-milestone implementation guides for future agents, `soil-rt` through the second project |

## Examples

[`examples/`](examples/) holds hand-written samples that exercise the
formats — a `csvstats` module (function, type, and module-header `.tr`
files with their `.lock` sidecars and a `.soil` lowering) and the prelude's
`read_file`, slated to be the first real definition:

- [`examples/read_file.tr`](examples/read_file.tr) — spec: prose, capability-style signature, fake-capability test, real-mode cram test
- [`examples/read_file.soil`](examples/read_file.soil) — its lowering
- [`examples/read_file.lock`](examples/read_file.lock) — its lock entry

## The shape of the thing

- **One file per definition.** A function, type, or module header lives in
  its own `.tr` file; its generated `.soil` and its `.lock` sit beside it.
- **Tests are the root of trust.** Humans write the tests that gate
  lowering; the refinement checker is a ratchet, not the trust root; only
  the human sets `accepted`.
- **Effects and capabilities.** Effect rows (`div`, `panic`, `io`, `ffi`)
  on types; `io` is not ambient — functions take opaque capability values
  (`Fs`, `Net`, `Clock`, …), and tests pass fakes.
- **Content-addressed everything.** Unison-style definition hashing makes
  checking incremental and caching exact.

## Licensing

Code is [GPL-3.0-or-later](LICENSE). Components that become part of
compiled user programs — the `soil-rt` runtime and, later, the prelude and
batteries — additionally carry the [GCC Runtime Library
Exception 3.1](LICENSE.exception), so programs built with the toolchain may
be licensed however their authors choose. Documentation, specifications,
and examples are [CC BY-SA 4.0](LICENSE.docs). Contributions are accepted
under the Developer Certificate of Origin (no CLA). The Trellis IDE is a
separate, closed-source product that shares no code with this repository.

## Roadmap

Per the [bootstrap plan](docs/bootstrap-plan.md): `soil0` + `soil-rt`
(minimal Rust interpreter and runtime) → the daemon → the prelude →
**`soilc`, the compiler, as the first Trellis project** (through
self-hosting closure via Cranelift) → FFI, `trellis bind`, and the IDE →
a Python-glue project as the second Trellis project, validating the half of
the pitch the compiler can't.
