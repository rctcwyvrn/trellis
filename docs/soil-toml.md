# `soil.toml` — Prototype (v1 subset)

*Status: **prototype — approved 2026-08-25** (drafted as impl plan 03
step 1 with `docs/contracts/trellis-daemon.md`; §4 questions resolved
and the whole reviewed with the user; prototype status like tr-grammar
and lock-schema — exercised by plan 03, finalized with them).
`soil.toml` is the only
non-derived global file in a Soil root (design §7.2), but no document
specified it; this prototype pins the subset the daemon needs —
the toolchain pin, the tag vocabulary, the entrypoint — and reserves
the rest of design §8's surface as named future sections. Prototype
status like tr-grammar and lock-schema: exercised by plan 03, finalized
with them. References: design §7.2 (layout), §8 (build system,
toolchain pin), §4.3 (tags), §3.14 (`main`), §5 (trusted packages),
tr-grammar §1 (tag validation), impl plan 03 §4.*

---

## 1. Role and location

A **Soil root** is any directory with a `soil.toml` at its top; every
`.tr`/`.soil`/`.lock` under it belongs to that root, and roots never
import across each other (design §7.2). The CLI discovers its root by
walking up from the working directory, unless `TRELLIS_ROOT` is set —
then that directory is the root (and must hold a `soil.toml`, else
`config-no-root`). The env override exists for processes running
*outside* the tree that still belong to it: the cram runner (impl
plan 03 step 8, resolved 2026-08-28) exports it so a transcript in
its fresh temp dir can invoke `trellis call` back into the root. The file is TOML, written by
the human (and by `trellis toolchain update`, which rewrites exactly
one table). The derived `soil.lock` manifest sits beside it,
gitignored.

Parsing is strict, matching the `.tr` frontmatter rule (tr-grammar
§6): an unknown table or key is an error (`config-unknown-key`), so
typos fail loudly instead of silently configuring nothing.

## 2. v1 surface

```toml
entrypoint = "main"            # optional

[toolchain]
trellis = "0.1.0"
soil0_cli = 1
# skill = "sha256:…"           # written by `trellis toolchain update`
                               # once `trellis skill` exists (§2.1)

[tags]
api = "public surface; CI policy keys on this"
parser = "the parsing subsystem"
```

### 2.1 `[toolchain]` — the pin

Refuse-on-mismatch (design §8, adopted 2026-08-23): any command that
lowers or verifies compares this table against the running binary and
**refuses** with the structured `toolchain-mismatch` diagnostic when
they disagree — never silently writing lock entries that claim more
than they should. `trellis toolchain update` is the explicit upgrade
event: it rewrites the table to the running toolchain's identity and
is the natural trigger for a re-verification sweep (design §7.1).

- `trellis` — the required `trellis` crate version (the daemon and
  CLI are one binary, so one version covers both).
- `soil0_cli` — the soil0 contract major (`soil0 --version`'s
  `soil0_cli`); builtins and error codes key on it.
- `skill` — the hash of the generated skill bundle (`trellis skill`,
  design §4.6). **Optional only while no generator exists** (impl
  plan 03 steps 2–11, which run on a hand-carried interim skill):
  during that window a missing pin warns. Once `trellis skill` ships
  (step 12), a missing `skill` under a generator-capable toolchain
  **refuses** like any other mismatch (resolved 2026-08-25, §4.1):
  the skill is a lowering input under the design §1.4 tenet, so an
  unpinned one is exactly the "exists only in an agent's context"
  hole. `trellis toolchain update` writes and maintains it — the fix
  is one command.
- The prelude and batteries hashes land in `[trusted_packages]` with
  plan 04 (§3); the pin table stays about the *toolchain*.

Read-only commands (`status`, `check` of the static pipeline,
`context`) run under a mismatch — inspecting a foreign-pinned root
must be possible — but anything that writes a lock refuses.

### 2.2 `[tags]` — the vocabulary

The per-project tag vocabulary of tr-grammar §1: keys are the legal
`tags:` values in `.tr` frontmatter (a `.tr` tag not declared here is
the `tr-undeclared-tag` error); values are one-line descriptions the
IDE shows. Tags are non-semantic — graph filtering and CI policy —
and never enter a context bundle.

### 2.3 `entrypoint`

Optional; names the definition `trellis build` will treat as `main`
(an ordinary `io` definition with a `World` argument, design §3.14).
Parsed and validated (the definition must exist when set); **unused
until build targets** (plan 06+). Recorded now so the key's shape is
settled before anything depends on it.

## 3. Reserved sections (named, unparsed until their milestone)

Declaring one of these before its milestone is an error (strictness
rule, §1) — the names are reserved so their arrival is additive:

| Table | Arrives | Content (design ref) |
|---|---|---|
| `[trusted_packages]` | plan 04 | prelude and batteries pinned by full package hash (design §5); merged into `soil.lock` |
| `[deps]` | plan 06+ | dependency declarations compiled to Nix (design §8); no raw-Nix escape field, ever |
| `[build]` | plan 06+ | build targets: `binary`, `py_module`, `shared_lib`, … (design §7.1) |
| `[ci]` | plan 06+ | policy lines ("every definition tagged `api` must be `accepted`", mutation-score floors — design §4.7, §4.5) |
| `[providers]` | never (see below) | — |

Provider configuration (models, budgets, caps, credentials) is
deliberately **not** in `soil.toml`: it is user-level, not
project-level, and credentials must never sit in a committed file. It
lives in `~/.config/trellis/providers.toml` (impl plan 03 §8.9,
non-contractual).

## 4. Open questions raised by this prototype — resolved 2026-08-25

1. **The `skill` bootstrap leniency (§2.1).** *Resolved: required
   once the generator exists* — from step 12 a missing `skill` pin is
   `toolchain-mismatch` (an unpinned skill is a lowering input living
   outside a hashed file, the design §1.4 hole); warning-only remains
   solely for the pre-generator window. Recorded in §2.1; the step-12
   drift gate enforces it.
2. **`[ci]` shape.** *Resolved: deferred to its milestone* (plan
   06+): the table stays reserved-by-name — which already prevents ad
   hoc invention — and its schema is drafted when headless CI and
   build targets arrive, informed by real badge rendering and the
   mutants job's actual fields.
