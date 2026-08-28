# CLAUDE.md

Trellis is a specification layer where humans write prose/types/tests in
`.tr` files and an agent lowers each definition to Soil (a small, strict,
refinement-typed ML). **The project is in design phase: this repo contains
only design documents and hand-written format examples. There is no
implementation.** Do not scaffold code unless asked.

## Where truth lives

- `docs/design.md` is the **single reference**. It records every decision
  *with its reasoning*, plus open questions (§9), deferred items (§10), and
  the build order (§11).
- The other docs elaborate specific areas: `docs/tr-grammar.md` (the `.tr`
  format and JSON value encoding), `docs/lock-schema.md` (`.lock`
  sidecars), `docs/soil-syntax.md` + `docs/soil-syntax-spec.md` (Soil
  surface syntax), `docs/bootstrap-plan.md` (soil0 → self-hosted soilc).
- `docs/plans/` holds per-milestone implementation guides. If you are
  implementing, start from your milestone's plan; its exit criteria define
  done, and its decision points must go to the user, not be decided
  unilaterally.
- `examples/` holds hand-written samples of the formats. They are
  normative illustrations: **every example must conform to the current
  specs**, and the layers must cohere (a `.lock`'s `calls` list matches its
  `.soil` body; `.tr` signatures match `.soil` signatures).

## Working rules

- **Design decisions belong to the user.** Present concrete options (with
  previews where possible) and let them choose. When they decide, record
  the decision **and the rationale** in the relevant spec doc *and* in
  `docs/design.md` — including moving items in/out of §9 open questions.
  Never leave the two out of sync.
- Decisions in this repo are argued, not just stated: when writing one up,
  capture *why* (and what was rejected), in the style of the existing docs.
- When touching syntax in examples, check `docs/soil-syntax-spec.md` first
  — details are strict (no shadowing, parameterless `let` with explicit
  lambdas, word connectives `and`/`or`/`not`, `::` for namespaces vs `.`
  for fields, `..` in partial record patterns, floor division).
- Sum-type JSON is internally tagged (`{"tag": …, "value": …}`);
  `Result a e` is success-first; filenames are snake_case with the cased
  name in frontmatter.
- Language minimalism is a tenet: "there is only one way to do something."
  When proposing syntax or format features, prefer removing options over
  adding them; verbosity for the agent is acceptable.

## Conventions in this repo

- Docs are Markdown, prose-first, with EBNF in fenced blocks; each spec doc
  carries a *Status* header noting what it resolves and where examples are.
- Spec docs end with an "open questions" section; resolve items in place
  and note the resolution rather than deleting silently.
- Hashes in **docs prose** are abbreviated fakes (`sha256:9f2c41aa`);
  keep that style. `examples/*.lock` sidecars carry **real** full
  digests once the daemon regenerates them (plan 03 step 5, decided
  2026-08-24) — never hand-edit those; regenerate.
