# Trellis-prose: Design Document

*Trellis-prose is a sibling of Trellis that lowers a human-written sentence
plan into finished prose. The human writes directives — the content and the
rhetorical moves — and an agent writes the sentences.*

*Status: draft, design phase. This document records the decisions made so
far with their reasoning, in the style of `docs/design.md`. It is
self-contained: nothing here changes Trellis. Worked examples live in
`examples/prose/`. Decisions below marked **(provisional)** were drafted to
complete the format and await user confirmation; they are listed in §8.*

---

## 1. Vision

### 1.1 The core thesis

Trellis applies one idea to code: the human authors the specification, an
agent authors the artifact, and a checker stands between them. Trellis-prose
applies the same idea to writing. The human writes a *sentence plan* — a
sequence of directives, each naming a rhetorical move and its payload — and
an agent lowers the plan into polished full-sentence prose. The human's
attention goes entirely to content and structure: what each sentence says,
where the paragraph breaks fall, which sentence is an example and which is a
pivot. The agent's job is wording.

The plan is to prose what a `.tr` file is to Soil: the layer where intent
lives, hashed and diffable, so that "what the document says" can be reviewed
and re-lowered independently of "how it is phrased."

### 1.2 What is checkable about prose

Code lowerings are gated by a type checker and tests. Prose has no type
checker, but the directive language is designed so that one structural
property *is* mechanically checkable: the **strict 1:1 mapping** (§4). Each
prose directive produces exactly one output sentence, in plan order, inside
the paragraph the plan declares. The checker can verify paragraph count,
per-paragraph sentence count, order, and verbatim headings without any
judgment. Style conformance, which cannot be checked mechanically, is judged
against a shared prose style guide (§5) and recorded in the lock as a
judgment, not a proof — the analog of an unproven refinement.

### 1.3 Design tenets inherited from Trellis

- **Nothing that affects the lowering exists only in an agent's context.**
  The plan, the surrounding notes, and the style guide are all hashed into
  the lock.
- **There is only one way to do something.** The directive set is closed and
  small; a new move must pay for itself (§8.1). Verbosity for the human is
  preferred over ambiguity for the checker.

---

## 2. The `.trp` file **(provisional container)**

A `.trp` file is CommonMark with YAML frontmatter, mirroring the `.tr`
container: formal content lives in one fenced block whose info string is the
reserved word `plan`; everything else is prose *notes* — context for the
agent (audience, purpose, background facts), hashed but never parsed.

Frontmatter keys (unknown keys are errors, as in `.tr`):

- `name` (required): the document's canonical name; equals the filename
  stem, lowercase snake_case.
- `style` (required): relative path to the style guide (§5). Required rather
  than defaulted so the lowering's full input set is visible in the file.

Exactly one `plan` block is required. Its contents are directive lines only:
blank lines are permitted for human readability but carry no meaning
(paragraphing is explicit, §3.2), and any non-blank line that does not parse
as a known directive is an error.

The three-part hash, mirroring `.tr`:

- `plan_hash`: the `plan` block. Changing it invalidates the lowering.
- `notes_hash`: frontmatter and all surrounding prose. Changing it marks the
  lowering `review-suggested` rather than invalidating it.
- `style_hash`: the resolved style guide file. Changing it marks *every*
  document that references it `review-suggested` (§5.2).

## 3. The directive language

A directive is one line: `<move>: <payload>`. The move names *what kind of
sentence this is*; the payload carries the content in the human's shorthand
— sentence fragments, abbreviations, and bullet-point telegraphese are all
fine, because wording is the agent's job.

### 3.1 Prose moves

Each prose move renders **exactly one sentence** (§4).

| Move | Payload | Renders |
|---|---|---|
| `sentence:` | the sentence's content | one sentence asserting the payload |
| `bridge to:` | an idea | one transition sentence connecting the preceding content to the idea |
| `example:` | a concrete instance | one sentence presenting the payload as an example of the preceding claim |
| `contrast:` | a counterpoint | one sentence pivoting against the preceding content; the agent supplies the "but / whereas / by contrast" framing |
| `restate:` | an idea already made in this document | one sentence re-expressing that idea in fresh words |
| `define:` | a term, optionally `— gloss` | one sentence defining the term |

Reasoning for this particular set: `sentence:` and `bridge to:` are the two
primitives — content the human dictates, and connective tissue the human
cannot fully dictate because it depends on the wording the agent chose for
the neighbours. The other four are the moves that a `sentence:` payload
expresses badly: an example needs "for instance" framing relative to the
previous sentence; a contrast needs the pivot word; a restatement must
*repeat* earlier content, which is otherwise forbidden (the agent may not
introduce claims absent from the plan, and `restate:` is the one move
licensed to echo); a definition has a fixed rhetorical shape worth naming.
Candidate moves that did not make v1 are listed in §8.1.

### 3.2 Structural moves

| Move | Payload | Effect |
|---|---|---|
| `paragraph:` | optional topic | opens a new paragraph; renders zero sentences. A topic payload is a cohesion hint to the agent and a target the judge can hold the paragraph to; an empty payload is allowed. |
| `heading <level>:` | verbatim text | a heading at the given level. The payload passes through verbatim — headings are structure, not wording, so the agent may not rewrite them. The level is always explicit (`heading 1:`, `heading 2:`); a defaulting rule would be a second way to say the same thing. |

Structure is entirely explicit: blank lines in the plan are ignored, and a
prose directive outside an open paragraph is an error (a `heading` closes
any open paragraph). "Blank line = paragraph break" was considered and
rejected: it reintroduces significant whitespace alongside the directive
language, i.e. a second syntax. Agent-chosen paragraphing was rejected
because it is uncheckable and would churn between lowerings.

### 3.3 What the agent may and may not do

The agent chooses wording, sentence-internal structure, connectives,
pronouns, and rhythm. It may not add claims absent from the plan, drop or
merge directives, reorder, or rewrite heading text. The output is plain
paragraphs and headings — no lists, block quotes, or emphasis in v1 (§8.4).

---

## 4. The 1:1 mapping — the type check of prose

**Decision: strict 1:1, order-preserving.** Each prose directive renders
exactly one output sentence; sentences appear in directive order inside the
declared paragraphs.

Reasoning: this is the property that makes a prose lowering *checkable
rather than plausible*. The checker verifies, mechanically:

1. every plan line parses as a known directive (closed set; unknown move is
   an error);
2. the output's headings match the `heading` payloads verbatim, at the
   declared levels, in order;
3. the output's paragraph count equals the plan's `paragraph:` count;
4. each output paragraph's sentence count equals its prose-directive count.

With counts and order verified, output sentence *N* of a paragraph is known
to correspond to prose directive *N* — the lock needs no explicit map, the
way a Trellis lock needs no line table.

Alternatives rejected:

- **Free counts (order only):** loses the count gate; the only mechanical
  check left is heading text.
- **1:1 with opt-in spans (`sentence*:`):** keeps the gate but adds a second
  way to write a sentence. If one directive genuinely needs two sentences,
  the human writes two directives; that verbosity is the tenet working as
  intended.

Cost accepted: the checker needs a sentence segmenter, and segmentation of
arbitrary prose is not exact. The rule is that the *agent* carries the
burden: it must write prose the conservative segmenter parses unambiguously
(e.g. avoiding sentence-medial abbreviations the segmenter would split on).
A lowering the segmenter cannot parse cleanly is a failed lowering, exactly
as Soil that does not parse is.

---

## 5. Style

### 5.1 A prose style guide, not a rule file

**Decision: the shared style input is a free-prose style guide** — a house
style document (voice, register, preferences, banned habits) that every
document's frontmatter references. There are no mechanical style rules in
v1.

Reasoning: mechanical rules (max words per sentence, forbidden-word lists,
declared tense/person) were considered, both alone and as a hybrid
rules-plus-exemplar file. They were rejected for v1 because the rules that
are cheap to check are not the ones that make prose good, and a rule file
grows into a second style guide that fights the first. One document, one
voice, one place to edit it. The consequence is accepted openly: **style
conformance is judged, not proven.** A judge agent reads the style guide and
the lowering and returns pass/fail with cited violations; the lock records
the verdict as a judgment. If judging proves unreliable, mechanical rules
can be added later as a *floor* under the guide (§8.2) without invalidating
the format.

### 5.2 Freshness

The style guide is hashed into every lock that references it. Editing it
marks those lowerings `review-suggested` rather than stale: the plans still
say what they said, the prose may merely be off-voice — the same treatment
`.tr` gives a changed `decisions` block.

---

## 6. Unit of lowering and freshness

**Decision: the whole document.** One `.trp` lowers to one output document;
the lock covers the pair.

Reasoning: prose has document-global cohesion that code does not —
anaphora, transitions, the "we said this above" texture. Per-paragraph locks
would pin wording that a neighbouring edit needs to shift. Per-paragraph and
per-section units were rejected for v1.

Cost accepted: any `plan` edit invalidates the entire lowering, and
re-lowering may re-word paragraphs whose directives did not change. This is
tolerable because re-lowering a document is cheap relative to code, but the
churn is real and a stability mechanism is an open question (§8.5).

## 7. The lock **(provisional schema)**

A `<name>.lock` sidecar in the spirit of `docs/lock-schema.md`, holding: the
three spec hashes (§2); the output hash; lowering provenance (provider,
model); the mechanical check results (parse, structure); the style judgment
(judge provenance and verdict); and the `accepted` flag, which only a human
sets. See `examples/prose/small_languages.lock` for the worked shape.

---

## 8. Open questions

1. **Additional moves.** The user asked for candidates beyond the six.
   Proposed, each pending a hand-written example where a `sentence:`
   rendering is genuinely tortured (the adoption rule; a move that fails it
   stays out):
   - `question: $idea` — a rhetorical question; the one move ending in `?`.
   - `concede: $point` — a concession ("admittedly …"), the mirror of
     `contrast:`.
   - `therefore: $conclusion` — draws the consequence of the preceding
     sentences; differs from `bridge to:` in closing a line of argument
     rather than opening one.
   - `quote: $text — $source` — verbatim pass-through, like headings;
     the only prose move the agent may not reword.
2. **Mechanical style floor.** Revisit rule-based checks if the style judge
   proves unreliable (§5.1).
3. **Sentence segmentation.** Specify the conservative segmenter and the
   prose forms the agent must avoid (abbreviations, quoted periods,
   ellipses).
4. **Richer output.** Lists, block quotes, and emphasis are absent from v1;
   each would need a directive and a checkable rendering rule.
5. **Re-lowering churn.** Whether accepted documents need paragraph-level
   wording stability (e.g. the agent is shown the previous lowering and told
   to preserve wording for unchanged directives), given whole-document
   invalidation (§6).
6. **Provisional decisions to confirm:** the CommonMark container with a
   single `plan` block (§2), required `style` frontmatter (§2), explicit
   heading levels (§3.2), and the lock field shapes (§7).
7. **Beyond one document:** multi-document projects, cross-references, and
   whether `restate:` may reference another document.
