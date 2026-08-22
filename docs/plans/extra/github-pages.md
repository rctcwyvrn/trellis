# GitHub Pages site

*Status: implemented (2026-08-22) — awaiting the manual Pages-source step
and first push to `main`.* *References: `README.md`
(Documents table), `docs/` (book content), `examples/` (included as
chapters), `rust/soil-rt` (rustdoc).*

---

## Context

Trellis is a design-phase project whose substance is its docs: 6 spec docs in
`docs/`, 8 milestone plans + 1 impl guide in `docs/plans/`, hand-written format
examples in `examples/`, and a working Rust crate (`rust/soil-rt`) with
rustdoc-able API docs. This plan publishes all of it via GitHub Pages at
`https://rctcwyvrn.github.io/trellis/`.

**Decisions made:**

- Site = designed landing page + full rendered docs + rustdoc.
- Generator = **mdBook**, building from `docs/` in place (specs stay the single
  source of truth; the book is a pure view).
- Site layout: root landing `index.html`, book under `/book/`, `cargo doc`
  output under `/rustdoc/`.
- Examples appear **as book chapters** (thin wrapper pages that `{{#include}}`
  the canonical files in `examples/`).
- CI uses a **plain rustup toolchain action** for `cargo doc` (not nix) —
  rustdoc output does not need the pinned environment `rust/check.sh` uses.
- The untracked `rust/`, `.gitignore`, `shell.nix`, LICENSE files are committed
  separately by the user — the workflow must not break if `rust/` isn't on
  `main` yet.

## Files to create

### 1. `book.toml` (repo root)

```toml
[book]
title = "Trellis"
src = "docs"

[build]
build-dir = "book"

[output.html]
default-theme = "rust"
git-repository-url = "https://github.com/rctcwyvrn/trellis"
site-url = "/trellis/book/"
```

(Exact theme/options adjustable; keep minimal per project ethos.)

### 2. `docs/SUMMARY.md` (mdBook chapter index)

Ordered to mirror the README's Documents table:

```markdown
# Summary

- [Design](design.md)
- [The .tr format](tr-grammar.md)
- [Lock schema](lock-schema.md)
- [Soil syntax](soil-syntax.md)
- [Soil syntax spec](soil-syntax-spec.md)
- [Bootstrap plan](bootstrap-plan.md)

# Implementation plans

- [Overview](plans/00-overview.md)
  - [01 — soil-rt](plans/01-soil-rt.md)
    - [01 impl guide](plans/impls/01-soil-rt-impl.md)
  - [02 — soil0](plans/02-soil0.md)
  - [03 — daemon](plans/03-daemon.md)
  - [04 — prelude](plans/04-prelude.md)
  - [05 — soilc](plans/05-soilc.md)
  - [06 — FFI, bind, IDE](plans/06-ffi-bind-ide.md)
  - [07 — python glue](plans/07-python-glue.md)

# Examples

- [read_file](examples/read_file.md)
- [csvstats](examples/csvstats.md)
```

No separate intro chapter — the landing page plays that role; `/book/` opens on
the design doc. Existing relative `.md` links between plan docs are rewritten
to `.html` by mdBook automatically; the spec docs have no inter-doc markdown
links, so nothing else needs touching.

### 3. `docs/examples/read_file.md` and `docs/examples/csvstats.md` (wrapper chapters)

Each is a short page showing the canonical example files in fenced blocks via
mdBook's include preprocessor, e.g.:

`````markdown
## read_file.tr

````markdown
{{#include ../../examples/read_file.tr}}
````

## read_file.soil

```
{{#include ../../examples/read_file.soil}}
```

## read_file.lock

```json
{{#include ../../examples/read_file.lock}}
```
`````

Note: `.tr` files contain 3-backtick fences, so the wrapping fence must use ≥4
backticks. `csvstats.md` does the same for the 9 csvstats files (with a
one-line intro noting which layers each definition has). Sources remain
canonical in `examples/`; wrappers contain no copied content.

### 4. `site/index.html` (landing page)

Hand-written, self-contained (inline CSS, no external assets), matching the
README's tone: the Trellis/Soil thesis adapted from the README's opening,
status line, and three prominent links — **Docs** (`book/`), **Rust API**
(`rustdoc/soil_rt/`), **GitHub** (repo URL). Light/dark via
`prefers-color-scheme`. No frameworks.

### 5. `.github/workflows/pages.yml`

```yaml
name: pages
on:
  push: { branches: [main] }
  workflow_dispatch:
permissions:
  contents: read
  pages: write
  id-token: write
concurrency: { group: pages, cancel-in-progress: true }
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: taiki-e/install-action@v2
        with: { tool: mdbook }
      - run: mdbook build            # -> book/
      - name: rustdoc (skipped until rust/ is committed)
        run: |
          if [ -d rust ]; then
            rustup default stable
            cargo doc --no-deps --manifest-path rust/Cargo.toml
          fi
      - name: assemble site
        run: |
          mkdir -p _site
          cp site/index.html _site/
          cp -r book _site/book
          if [ -d rust/target/doc ]; then
            cp -r rust/target/doc _site/rustdoc
            printf '<meta http-equiv="refresh" content="0; url=soil_rt/">' > _site/rustdoc/index.html
          fi
      - uses: actions/upload-pages-artifact@v3
        with: { path: _site }
  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment: { name: github-pages, url: ${{ steps.deployment.outputs.page_url }} }
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

The `if [ -d rust ]` guard lets the site deploy correctly before `rust/` is
pushed; rustdoc appears automatically on the first push after it lands.
`cargo doc` has no top-level index, hence the `rustdoc/index.html` redirect to
`soil_rt/`.

### 6. Housekeeping

- Add `book/` to `.gitignore` (it currently has only `rust/target/`).
- Add a one-line link to the site at the top of `README.md`'s Documents
  section (small, matches README tone).

## Known rendering notes (verify, don't over-engineer)

- EBNF blocks are bare ``` fences; mdBook renders untagged blocks as plain
  code. Check highlight.js doesn't mis-autodetect them; if it does, the fix is
  tagging fences `text` (a doc-wide but mechanical edit — flag to the user
  first).
- Heavy GFM tables with long inline-code cells are the main styling risk;
  eyeball `design.md` and `tr-grammar.md` in the built book.
- `tr-grammar.md` has a 4-backtick nested fence — mdBook/CommonMark handles it;
  confirm visually.

## Not in scope

- Committing `rust/`, `shell.nix`, `.gitignore`, LICENSE files (done
  separately by the user).
- Rewriting prose cross-references ("design §4.5") into hyperlinks.
- Updating the stale "No implementation exists yet" line in README/CLAUDE.md.

## Manual step (repo owner)

Repo Settings → Pages → Source: **GitHub Actions** (or
`gh api -X POST repos/rctcwyvrn/trellis/pages -f build_type=workflow`).
Without this the deploy job fails on first run.

## Exit criteria

1. Local: `mdbook build` succeeds; `book/index.html` shows correct nav order,
   tables, EBNF blocks, the examples chapters (includes resolved, fences
   nested correctly), and working plan cross-links.
2. Local: `cargo doc --no-deps --manifest-path rust/Cargo.toml` succeeds;
   `_site/` assembled exactly as the workflow does opens with landing links
   resolving to `book/` and `rustdoc/soil_rt/`.
3. Pushed to `main`: the Actions run is green, and
   `https://rctcwyvrn.github.io/trellis/` (landing), `/trellis/book/` (docs),
   and — once `rust/` is pushed — `/trellis/rustdoc/soil_rt/` all render.
