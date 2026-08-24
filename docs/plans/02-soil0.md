# Plan 02 — `soil0`: the minimal Soil implementation

*References: docs/soil-syntax-spec.md (the parser's contract), design §3
(language), bootstrap plan §1.*

## Goal

A deliberately small Rust implementation of Soil — parser, static checks,
tree-walking interpreter over `soil-rt` values — whose every pass is a
JSON-in/JSON-out CLI command. It is the execution engine for the entire
bootstrap and the permanent differential oracle for `soilc`. Resist every
temptation to make it good; make it correct and small.

## Scope

1. **The AST JSON schema — a spec artifact, written first.** The exact
   JSON encoding of the surface AST and of check results. This is the CLI
   oracle contract that `soilc`'s Trellis type definitions must later
   reproduce, so it follows the tr-grammar §7 value-encoding conventions
   (internally tagged sums, records) as if the AST were already Soil data.
   Deliverable: `docs/contracts/soil0-cli.md`, reviewed before code.
2. **Lexer + parser.** Hand-written recursive descent implementing
   `docs/soil-syntax-spec.md` exactly: the `and` binding/connective
   disambiguation, non-associative comparisons, parenthesized non-tail
   match, `..` record patterns, the fixed escape set, `decreases` lines.
   Errors carry source spans.
3. **Renamer.** Scope resolution, the no-shadowing rule, `::`
   qualification, `_private` visibility.
4. **Type + effect inference.** ML inference (HM with records and sums from
   registered type shapes — no row-polymorphic records needed), effect rows
   as sets with row variables, subsumption at calls, operator-notation
   elaboration (`==` → `T::eq`, arithmetic → per-type primitives at
   monomorphic types only). **No refinements** (parsed, retained in the
   AST, otherwise ignored). **No termination checking**: every
   self-recursive or `let rec` definition conservatively acquires `div`
   (consequence handled in plan 04) — recorded by `check` as an
   unverified-termination *fact* rather than a hard error, so
   `total`-signed examples still pass (the demotion philosophy;
   soil0-cli §8.5, approved 2026-08-22).
5. **Exhaustiveness + redundancy** checking for matches.
6. **Interpreter.** Strict tree-walk over `soil-rt` values; closures;
   capability primitives implemented natively (`fs_read_bytes`, clock,
   rand, env, proc) plus the prelude's *fake* capability constructors
   (`fake_fs`, seeded `fake_rand`, `fake_clock`); arithmetic follows floor
   division/modulus and panics (as `SoilError`) on overflow and zero
   divisors — obligations don't exist yet, so these are always-on runtime
   checks, matching debug-mode semantics.
7. **CLI.** `soil0 lex|parse|rename|infer|check|run|test`, each reading
   source (or AST JSON) and emitting schema-conformant JSON on stdout,
   errors as structured JSON on stderr, nonzero exit. `run` takes
   `--entry name --args <json-array>` and prints the canonical JSON
   result; `test` executes a JSON test bundle (assembled by the daemon)
   and reports per-case results in the lock's result vocabulary.

## Non-goals

Refinement checking, termination checking, codegen, optimization of any
kind, `.tr` parsing (daemon's job, plan 03), Python FFI.

## Testing

- Golden tests: a corpus of `.soil` fragments → expected AST/type/effect
  JSON, including every syntax-spec static rule as a rejection test (one
  test per rule: shadowing, redundant arm, `a < b < c`, missing `..`,
  parameterless `let` violation, bad escape…).
- `examples/read_file.soil` and `examples/csvstats/median.soil` must
  parse, check, and (with stub callees) run.
- Interpreter: expect-style tests mirroring the `.tr` examples' test
  blocks, run through `soil0 test`.
- Fuzz the parser (cargo-fuzz or a simple generator) for panic-freedom.

## Exit criteria

- `docs/contracts/soil0-cli.md` exists and every command conforms to it.
- Both checked-in `.soil` examples check and run with correct results.
- The rejection-test corpus covers every static rule in
  `docs/soil-syntax-spec.md` §5.
- A downstream consumer (plan 03) can drive lex→check→test entirely
  through the CLI without linking `soil0` as a library.

## Decision points — resolved 2026-08-22

- **Library + CLI:** the daemon links `soil0` as a crate for in-process
  checking, but the CLI is the frozen compatibility contract — oracle and
  conformance tests always go through the CLI, never the library API.
- **Spans:** UTF-8 byte offsets are canonical (`start`/`end`), with
  derived `line`/`col` included alongside for display.
- **Type environment:** commands take `--types env.json` — type
  descriptors in the schema's own encoding. Early tests hand-write it;
  the daemon generates it from `.tr` files later. `soil0` never parses
  `.tr`.

## Implementation plan

A detailed implementation guide exists at
[`impls/02-soil0-impl.md`](impls/02-soil0-impl.md). It records a second
round of decisions resolved 2026-08-22 — union-find inference with naive
env-scan generalization, `Spanned` wrapper records in the AST JSON, a
signature-only frozen contract for `soil0 infer` (with a non-contractual
`--dump-ast`), and a machine-written `program.json` manifest referencing
`env.json` — plus the module map, build order, the required contents of
`docs/contracts/soil0-cli.md`, a set of micro-pins (its §8, approved 2026-08-22),
and three spec gaps it surfaced, all resolved 2026-08-22 (its
§9): constructor qualification `Type::Ctor` with the
exactly-one-spelling rule (now soil-syntax-spec §5.9, design §3.15),
the `fake_fs` example corrected to the canonical map encoding, and the
list primitives adopted as native-backed prelude surface (plan 04).
Step 1's deliverable, `docs/contracts/soil0-cli.md`, was reviewed and
**frozen as contract v1 on 2026-08-22** (amended v1.1 on 2026-08-23:
typed holes, `print`). **All implementation steps (2–11) completed
2026-08-23**: every contract command is live, both normative examples
check *and run with correct results* (median through a real insertion
sort; read_file through fake and real capabilities), the rejection
corpus is audited by an executable checklist, and the canonical printer
is byte-identical on the examples with `parse → print` a fixpoint. The
exit criteria are met; plan 03 is unblocked.
