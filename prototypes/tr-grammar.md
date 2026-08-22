# The `.tr` File Format — Prototype Grammar

*Status: prototype. Tentatively resolves §9.1 (grammar), §9.2 (JSON value encoding), and §9.4 (refinement prose syntax) of `docs/design.md`, and proposes an answer to §9.5 (write-back markers). Worked examples live in `examples/`.*

---

## 1. Container format

A `.tr` file is CommonMark. Formal content lives in fenced code blocks whose
info string begins with a **reserved word**. Everything else — including
fenced blocks in unreserved languages such as `python` or `text` — is prose:
it is hashed under `prose_hash` and never parsed.

There are three file kinds, distinguished by content, with filename as
identity (design §4.3):

| Kind | Filename | Defines |
|---|---|---|
| Function | `<name>.tr` | one function; name = filename stem verbatim |
| Type | `<name>.tr` | one type; PascalCase name declared in frontmatter, filename is its snake_case form |
| Module header | `_module.tr` | module prose and the export list |

Filenames are lowercase snake_case for every kind, so identity survives
case-insensitive filesystems.

Every `.tr` file begins with YAML frontmatter. `name` is required and is the
definition's canonical name: a function's equals the filename stem; a type's
is PascalCase and the filename is its snake_case form; a module's equals its
directory name. There is no `kind` field — kind is inferred (`_module.tr` by
filename; a `soil-type` block makes a type file; otherwise the file is a
function).

`tags` is optional: a list drawn from a per-project vocabulary declared in a
`[tags]` table in `soil.toml`; an undeclared tag is an error. Tags are
non-semantic metadata — IDE graph filtering and colouring, CI policy in
`soil.toml` (e.g. "every definition tagged `api` must be `accepted`") — and
are never part of the lowering context bundle. Unknown frontmatter keys are
errors.

```
---
name: ParseError
tags: [parser]
---
```

### Reserved block languages

| Info string | File kind | Count | Purpose |
|---|---|---|---|
| `soil-sig` | function | ≤ 1 | Soil type signature |
| `requires` | function | ≤ 1 | preconditions |
| `ensures` | function | ≤ 1 | postconditions |
| `test <name> [xfail]` | function | any | expect test |
| `property <name> [xfail]` | function | any | property test |
| `cram <name> [xfail]` | function | any | shell-transcript test (`io` fallback, `main`) |
| `reference` | function | ≤ 1 | reference-implementation attachment |
| `allow` | function | ≤ 1 | escape hatches (human-only) |
| `soil-type` | type | = 1 | the type's shape |
| `invariant` | type | ≤ 1 | type invariants |
| `exports` | module | = 1 | export list |

### Mapping to the three-part hash (design §6.2)

- `formal_hash`: the frontmatter `name`, `soil-sig`, `requires`, `ensures`,
  `soil-type`, `invariant`, `exports`, `allow`.
- `test_hash`: `test`, `property`, `cram`, `reference` (the reference is an
  oracle; changing it re-runs differential tests, not the lowering).
- `prose_hash`: everything else in the file, including `tags`.

---

## 2. Shared mini-languages

### 2.1 JSON values

`json` below means an RFC 8259 JSON value, interpreted type-directedly under
the encoding of §7.

### 2.2 Identifiers

`ident` is lowercase snake_case (functions, parameters, fields).
`Ctor` and `TypeName` are PascalCase.

### 2.3 The predicate language

Shared by `requires`, `ensures`, `invariant`, and `property`. It is a
restricted Soil boolean expression: calls may target only `total`
definitions, and predicates outside the decidable fragment (linear
arithmetic, uninterpreted functions, lengths — design §3.2) still parse but
demote the function to unverified.

```
predicate  ::= disj [ "implies" predicate ]            (right-assoc)
disj       ::= conj { "or" conj }
conj       ::= neg { "and" neg }
neg        ::= [ "not" ] atom
atom       ::= comparison | is-test | call | "(" predicate ")"
comparison ::= expr relop expr
relop      ::= "==" | "!=" | "<" | "<=" | ">" | ">="
is-test    ::= expr "is" Ctor [ "(" ident ")" ]        (binds the payload)
expr       ::= literal | path | call
             | expr ("+" | "-" | "*") expr | "(" expr ")"
path       ::= ident { "." ident }                     (record field access)
call       ::= ident "(" [ expr { "," expr } ] ")"
literal    ::= JSON literal
```

Names in scope: the signature's named parameters; `result` (in `ensures`
only); `self` (in `invariant` only); `forall` binders (in `property` only);
and `total` definitions visible to the file.

---

## 3. Function-definition blocks

### 3.1 `soil-sig`

```
sig    ::= defname ":" arrow
arrow  ::= param "->" arrow | ret
param  ::= "(" ident ":" type ")" | type
ret    ::= [ row ] type
row    ::= effect { effect }
effect ::= "div" | "panic" | "io" | "ffi"
```

An empty row means `total`. `type` is Soil type syntax, specified separately;
this grammar treats it as opaque. The signature may be absent — the agent
infers it and writes it back (§8). If `requires`/`ensures` refer to a
parameter by name, the signature must exist and use the named-parameter form.

### 3.2 `requires` / `ensures`

```
block  ::= clause { clause }
clause ::= label ":" predicate                          (one per line)
label  ::= free text not containing ":"
```

Labels are the pinnable names: the lock, the IDE, and checker errors refer to
clauses by label. In `ensures`, `result` is bound to the return value; for
`Result`-typed functions the idiom is `result is Ok(v) implies …`
(design §3.2: refinements may eliminate error cases).

### 3.3 `test`

The function under test is implicit — it is the file's definition. A block
holds any number of cases; case *k* of block *name* is reported as `name#k`.

```
block     ::= { with-line } case-line { case-line }
with-line ::= "with" ident "=" fake-call
fake-call ::= ident { json }
case-line ::= "(" [ arg { "," arg } ] ")" "=>" outcome
arg       ::= json | ident                              (a with-binding)
outcome   ::= json | "panic"
```

- `with` lines construct fake capabilities from the prelude (design §3.5);
  their arguments are JSON, so seeds and timestamps are pinned by
  construction (`with clock = fake_clock 1700000000`).
- `panic` as an outcome is only legal if the signature's row carries `panic`.
- `xfail` in the info string marks the whole block expected-to-fail; it
  blocks `accepted` until resolved (design §4.5).

### 3.4 `property`

```
block       ::= forall-line { forall-line } predicate
forall-line ::= "forall" ident ":" type [ "where" predicate ]
```

Generators are derived from the binder's type; a `where` filter is a
generator constraint, satisfied by constrained generation rather than
rejection sampling. Properties may call the function under test, the
prelude, the reference implementation, and `accepted` definitions
(design §4.5).

### 3.5 `cram`

For `io` functions where fakes stop being possible, for FFI bindings, and
for `main`. The dialect is a minimal subset of classic cram: unindented, no
`(re)`/`(glob)` matchers (extensible later).

```
block       ::= { with-line } step { step }
with-line   ::= "with" "file" string "=" string      (JSON strings)
step        ::= command { output-line } [ exit ]
command     ::= "$ " rest-of-line                    (a shell command)
output-line ::= any line not beginning "$ " or "["
exit        ::= "[" integer "]"
```

- Each block runs in a fresh temp dir. `with file` lines materialize
  fixtures before the transcript runs — path and contents are JSON strings,
  so escapes are pinned.
- Commands run sequentially in one shell session in that dir. Expected
  output is combined stdout+stderr, matched literally. An omitted exit line
  means 0.
- The transcript may invoke built binary targets from `soil.toml` (placed on
  `PATH`), and `trellis call <def> <json-arg>…`, which runs an `io`
  definition with real `World`-derived capabilities and prints its result as
  canonical JSON — the real-mode escape for non-`main` `io` functions and
  FFI bindings. Capability parameters are injected from `World`; the JSON
  arguments fill the remaining parameters in order.
- The lock tags these tests mode `real` (design §4.5). Cram never runs
  inside the lowering sandbox: the lowerer's `run_tests` tool exposes only
  fake-capability tests (design §4.6).

### 3.6 `reference`

One line: `relpath "::" symbol`, e.g. `ref/stats.py::median`. Python only
(design §4.5). Attached explicitly by the human, never auto-detected.

### 3.7 `allow`

One escape hatch per line, from the fixed set `partial`, `unsafe`,
`ffi-raw`. Written only by the human; every `allow` block in the project is
surfaced in the manifest's audit view (design §4.1).

---

## 4. Type-definition blocks

### 4.1 `soil-type`

```
typedef ::= "type" TypeName { tyvar } "=" body
body    ::= "opaque" | record | sum | type              (last = alias)
record  ::= "{" field { "," field } "}"
field   ::= ident ":" type [ "ignored" "=" expr ]
sum     ::= [ "|" ] ctor { "|" ctor }
ctor    ::= Ctor [ record ]
```

Variant payloads are records (no positional products, design §3.13).
Derivation strategies (design §3.7): structural is the default; an `opaque`
body selects the opaque strategy; the `ignored` field marker selects the
ignored strategy for that field.

An `ignored` field must carry a default: an `expr` from the predicate
language (§2.3) — so calls target only `total` definitions — with the
record's non-ignored fields in scope. The default materializes the field
wherever a value is built without it: JSON decode, `py_to_soil`, host stubs.
`ignored` thus means "excluded from derivation, reconstructible on demand":

```
type Doc = { text : Utf8, cached_word_count : U64 ignored = word_count(text) }
```

### 4.2 `invariant`

Same clause grammar as `ensures`, with `self` bound to a value of the type.
Invariants are properties every constructor must preserve (whether checked at
every construction or proven at definition sites remains open, design §9.6).
Each invariant clause auto-generates a property test (design §4.1), which is
why type files carry no hand-written test blocks.

---

## 5. Module headers

### 5.1 `exports`

```
line ::= ident
       | "type" TypeName
       | "abstract" "type" TypeName
```

An exact list of definitions (design §4.4). If an exported signature
references an unexported type, the two permitted repairs are exporting it or
marking it `abstract` here (design §3.14).

---

## 6. Validity rules

1. Every file begins with frontmatter carrying a `name`. Unknown frontmatter
   keys, and tags not declared in `soil.toml`, are errors.
2. Function file: `name` equals the filename stem; at least one prose
   paragraph and at least one `test`, `property`, or `cram` block. The
   minimal valid definition is the frontmatter, one sentence of prose, and
   one expect test (design §4.3); property-only and cram-only files are
   valid — `main` and other toplevel functions are typically of that shape.
3. Type file: the snake_case form of `name` equals the filename stem; at
   least one prose paragraph and exactly one `soil-type` block. No test
   blocks.
4. Module header: `name` equals the containing directory's name; exactly one
   `exports` block.
5. Block multiplicities per the table in §1; `test`/`property`/`cram` names
   unique within a file.
6. The names declared in `soil-sig` and `soil-type`, when present, must
   equal the frontmatter `name`. Renaming is refactor-rename (design §4.3).
7. Predicates may reference parameters only via a named-parameter `soil-sig`.

---

## 7. JSON value encoding (resolves design §9.2)

One encoding serves `show`/`parse`, tests, the REPL, and host stubs.
Encoding and decoding are always type-directed. Sum types are **internally
tagged**: one uniform shape for every variant, self-describing for hosts and
generic tooling. The verbosity is accepted because JSON values are primarily
written and read through the IDE's block widgets (design §4.3), not typed by
hand.

| Soil type | JSON |
|---|---|
| `I64`, `U64`, `I32`, … | number (integer) |
| `BigInt` | number within ±(2^53−1), string beyond; decode accepts either |
| `F64` | number; `"NaN"`, `"Inf"`, `"-Inf"` as strings |
| `Utf8` | string |
| `Bytes` | string, base64 |
| `Unit` | `null` |
| record | object; every field present; `ignored` fields omitted by `show`, refilled from their default on decode |
| sum, nullary variant | `{"tag": "Name"}` |
| sum, payload variant | `{"tag": "Name", "value": <payload>}` |
| `List a` | array |
| `Map k v` | array of `{"key": k, "value": v}` in comparator order |
| opaque | `show` emits `"<handle>"`; decoding is an error |
| function | hard error in both directions |

Canonical output: `show` emits record fields in declaration order, map
entries in comparator order, and floats in shortest round-trip form, so equal
values produce byte-equal JSON and expect tests can compare on the string
(design §3.7).

The keys `"tag"` and `"value"` are produced only by the sum encoding; since
decoding is type-directed, a record field named `tag` is not ambiguous, but
the linter warns on it.

---

## 8. Provenance and write-back (tentative, design §9.5)

Agent-authored formal blocks carry an `@agent` marker at the end of the info
string:

````
```soil-sig @agent
mean : (xs : List F64) -> F64
```
````

The agent may freely rewrite blocks marked `@agent`. When a human edits such
a block they remove the marker; an unmarked formal block is human-authored
and therefore pinned — the agent may not change it, only raise `ask_human`.
The lock records provenance per block alongside the hashes.

---

## 9. Open questions raised by this prototype

All questions raised by the first draft — type-name casing (frontmatter,
§1), `ignored`-field decoding (defaults, §4.1), `BigInt` interop (hybrid by
range, §7), generator strategy (constrained, §3.4), the `cram` grammar
(§3.5), and property-only files (valid, §6) — have been resolved and folded
into the sections above.
