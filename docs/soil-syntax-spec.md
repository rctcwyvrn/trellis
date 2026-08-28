# Soil Surface Syntax — Elaborated Specification

*Status: prototype. Elaborates `soil-syntax.md` into a lexical spec, EBNF,
and static rules. Decisions folded in from review: OCaml-style match (no
terminator, parenthesize non-tail nested matches), word connectives
(`and`/`or`/`not`) shared with the predicate language, shadowing forbidden,
`::` for namespaces with `.` reserved for field access, `..` required in
partial record patterns, parameterless `let` bindings (functions are
explicit lambdas), and (2026-08-22, surfaced by impl plan 02) constructor
qualification `Type::Ctor` with the exactly-one-spelling rule (§5.9).
Semantics (typing, effect, and refinement rules) arrive
with the Soil core milestone (design §11); this document is the parser's
contract.*

---

## 1. Lexical structure

- Source is UTF-8. Whitespace separates tokens and is otherwise
  insignificant — there is no layout rule.
- Comments run from `--` to end of line. No block comments.

### Identifiers

| Class | Form | Used for |
|---|---|---|
| `ident` | `[a-z][a-z0-9_]*` | values, parameters, fields, type variables, row variables, module names |
| `private-ident` | `_` `ident` | module-private definitions (only in `_private.soil`) |
| `TypeName` | `[A-Z][A-Za-z0-9]*` | types and constructors (one class; constructors live in the type's namespace) |

### Keywords

```
let  rec  and  or  not  in  fun  match  with  if  then  else  decreases
```

Effect names are reserved in type position only: `div`, `panic`, `io`,
`ffi`. The boolean connectives are the keywords `and`/`or`/`not` — the same
spelling as the predicate language, so specs and code read identically
(`implies` remains predicate-exclusive). There are no boolean literals:
`Bool` is the prelude sum `True | False` (see §6 for its JSON special
case).

`and` serves both as the mutual-binding connector and the boolean
connective; see §3.3 for the disambiguation rule.

### Operators and punctuation

```
->  =  |  :  ::  .  ,  ..  ( )  { }
==  !=  <  <=  >  >=  +  -  *  /  %  _
```

`?` appears only as the head of a typed hole: `?` immediately followed by
an `ident` lexes as one hole token (`?rest`); a bare `?` is an error
(§5.11, added 2026-08-23).

### Literals

- Integer: `[0-9][0-9_]*`, default type `I64`. Other widths by annotation
  (`(42 : U32)`) or by context — an integer literal adopts the type
  inference demands ("as in Rust", design §3.6), so `b == 0` checks with
  `b : U64`; the default applies only when unconstrained. No suffixes. A
  literal is range-checked against its resolved width at check time
  (impl plan 02 §8.14, amended 2026-08-23 — the fully-fixed-at-lex
  reading would have broken the normative `gcd` example); since negation
  is an operator, `I64::MIN` is not writable as a literal (the C/Rust
  wart, accepted — a prelude constant covers it).
- Float: digits `.` digits, with an optional exponent — lowercase `e`,
  an optional `-` (no `+`, no `E`), digits — as in `1.5e3`, `1.5e-3`.
  Default type `F64`. Underscores are permitted in every digit run of
  numeric literals and are not part of the value. Lexing extends a
  numeric literal only when what follows can continue it: `1..2` lexes
  as `1` `..` `2`, and `1.5e` as `1.5` followed by the identifier `e`.
  (Edges resolved 2026-08-22 with the soil0 lexer.)
- String: `"…"`, default `Utf8`. The escape set is exactly `\"`, `\\`,
  `\n`, `\r`, `\t`, and `\u{hex}` with one to six hex digits denoting a
  Unicode scalar value (surrogates U+D800–U+DFFF and values above U+10FFFF
  are errors — a `Utf8` value can never hold them). Any other character
  after `\` is an error; there are no octal/hex byte escapes (`Bytes` are
  built by prelude functions, not literals) and no line-continuation
  escapes. Raw characters are unrestricted: any character other than `"`
  and `\` — including newlines and non-ASCII — stands for itself, so
  strings may span lines; a string is unterminated only at end of file.
  (Resolved 2026-08-22 with the soil0 lexer.)
- No character literals; no `Bytes` literals (construct via prelude
  functions).
- Negation is the unary operator, not part of the literal.

## 2. Files

- `<name>.soil` holds **exactly one** definition; its name equals the
  filename stem and the `.tr` frontmatter `name`.
- `_private.soil` holds any number of `private-ident` definitions
  (design §4.2).
- `.soil` files contain **no type declarations** — type shapes live in the
  `.tr` `soil-type` blocks and the compiler materializes them — and **no
  imports**: names resolve through the manifest (design §6.1), and the
  lock's import set is computed.

## 3. Grammar

Notation: `{ x }` is zero-or-more, `[ x ]` optional, `|` alternation,
terminals quoted.

### 3.1 Definitions

```
soil-file      ::= definition
private-file   ::= { definition }

definition     ::= signature [ decreases-line ] equation
signature      ::= defname ":" type
decreases-line ::= "decreases" expr
equation       ::= defname { ident } "=" expr
defname        ::= ident | private-ident
```

Equation parameters are bare identifiers — destructuring happens in the
body. The `defname` of the signature and equation must agree.

### 3.2 Types

```
type       ::= [ dom "->" ] cod                      (right-assoc arrows)
dom        ::= "(" ident ":" type ")" | app-type | refinement
cod        ::= [ row ] type
row        ::= effect { effect } [ ident ]           (trailing ident = row variable)
             | ident                                  (row variable alone — only before a type)
effect     ::= "div" | "panic" | "io" | "ffi"
app-type   ::= TypeName { atype } | atype
atype      ::= TypeName | ident | refinement | "(" type ")"
refinement ::= "{" ident ":" type "|" predicate "}"
```

- Quantification is implicit and prenex: free lowercase type variables are
  universally quantified. There is no `forall`.
- **Row parsing is unambiguous without HKT:** type variables have kind `*`
  and are never applied, so in `a -> e b` and `a -> e (List b)` the leading
  lowercase ident followed by another type can only be a row variable. A
  lone ident after `->` is the return type. An absent row is the empty row
  (`total`).
- `predicate` is the **shared predicate language of the `.tr` grammar**
  (tr-grammar §2.3): refinements are the spec language embedded in Soil,
  desugaring one-to-one from `requires`/`ensures` clauses. Its connectives
  (`and`/`or`/`not`) are the same words the term layer uses, so there is
  exactly one spelling everywhere; `implies` exists only in predicates.

### 3.3 Expressions

```
expr      ::= "let" [ "rec" ] binding { "and" binding } "in" expr
            | "fun" ident { ident } "->" expr
            | "if" expr "then" expr "else" expr
            | "match" expr "with" arms
            | or-expr

binding   ::= ( defname | "_" ) "=" expr
arms      ::= "|" arm { "|" arm }
arm       ::= pattern "->" expr

or-expr   ::= and-expr { "or" and-expr }
and-expr  ::= not-expr { "and" not-expr }
not-expr  ::= "not" not-expr | cmp-expr
cmp-expr  ::= add-expr [ cmpop add-expr ]            (non-associative)
cmpop     ::= "==" | "!=" | "<" | "<=" | ">" | ">="
add-expr  ::= mul-expr { ("+" | "-") mul-expr }
mul-expr  ::= unary { ("*" | "/" | "%") unary }
unary     ::= "-" unary | app
app       ::= atom { atom }                          (left-assoc application)

atom      ::= literal
            | path
            | qualified
            | TypeName                               (constructor)
            | record
            | hole
            | "(" expr [ ":" type ] ")"

hole      ::= "?" ident                              (typed hole, §5.11; design §3.16)

path      ::= (ident | private-ident) { "." ident }  (variable + field projections)
qualified ::= ident "::" ident                       (module::def)
            | TypeName "::" ident                    (Type::derived)
            | TypeName "::" TypeName                 (Type::Ctor — §5.9)
record    ::= "{" [ path "with" ] field { "," field } "}"
field     ::= ident "=" expr
```

- **Bindings carry no parameters** — a let-bound function is an explicit
  lambda: `let go = fun x -> … in …`. One way to write a function.
- `let _ = e in …` discards the result — the sequencing idiom for effects.
  `_` binds nothing and is exempt from the no-shadowing rule; it is not
  permitted in `let rec`.
- **The `and` disambiguation:** after `and`, the two-token sequence
  `defname "="` begins a new binding of the enclosing `let rec`; anything
  else makes `and` the boolean connective. This is unambiguous because `=`
  never occurs in expressions (equality is `==`) and bindings are
  parameterless.
- An arm's body extends as far as possible; a subsequent `|` belongs to the
  innermost open `match`. A nested match in non-tail position must be
  parenthesized (the OCaml rule, chosen deliberately).
- Constructor application is ordinary application with a `TypeName` head:
  `Ok bytes`, `BadCell { index = 1, text = "x" }`, nullary `None` — or a
  qualified head `Result::Ok bytes` exactly when the variant name
  collides in scope (§5.9).
- Record update bases are paths, not arbitrary expressions:
  `{ r with cells = ys }`.
- `(e : type)` is a local annotation, the only way to give a literal a
  non-default type.

### 3.4 Patterns

```
pattern    ::= "_" | ident | literal
             | ctor-name [ pat-atom ]
             | record-pat
             | "(" pattern ")"
pat-atom   ::= "_" | ident | literal | ctor-name | record-pat | "(" pattern ")"
ctor-name  ::= [ TypeName "::" ] TypeName            (qualification per §5.9)
record-pat ::= "{" fieldpat { "," fieldpat } [ "," ".." ] "}"
fieldpat   ::= ident [ "=" pattern ]                 (bare ident = punning)
```

A record pattern must name every field of the record type unless it ends
with `..` — omitting fields silently is an error, so adding a field to a
type breaks exactly the patterns that need reviewing.

Patterns nest arbitrarily. There are no or-patterns, no guards, and no `as`
bindings (nested `match` and fresh `let`s instead).

## 4. Precedence

Tightest to loosest:

1. field access `.`
2. application (juxtaposition), `::`
3. unary `-`
4. `*` `/` `%`
5. `+` `-`
6. `==` `!=` `<` `<=` `>` `>=` (non-associative — `a < b < c` is a parse error)
7. `not`
8. `and`
9. `or`
10. `if` / `fun` / `let` / `match` bodies

This ladder is the predicate language's ladder (tr-grammar §2.3) with the
arithmetic tiers inserted below the comparisons and `implies` absent.

## 5. Static rules

1. **No shadowing.** Binding a name already in scope — by `let`, `fun`, an
   equation parameter, or a pattern — is an error. Fresh names only.
2. **Exhaustive matches**, and redundant arms are errors.
3. **Effect subsumption:** `f` may call `g` iff `g`'s row ⊆ `f`'s row;
   `ffi` implies `panic` (design §3.3).
4. **Operators are notation, not overloading** (design §3.6, §3.7). The
   elaborator rewrites at the inferred monomorphic type; in polymorphic
   position the operators are unavailable and the function is taken as a
   parameter.

   | Surface | Elaborates to |
   |---|---|
   | `==` `!=` | `T::eq` (negated for `!=`) |
   | `<` `<=` `>` `>=` | `T::compare` |
   | `+` `-` `*` `/` `%`, unary `-` | per-type numeric primitives (`I64::add`, `F64::div`, …) |
   | `and` `or` | short-circuit builtins on `Bool` |
   | `not` | builtin on `Bool` |

5. **Arithmetic obligations:** overflow, and a zero divisor for `/` and
   `%` on integers, are refinement obligations (design §3.2, §3.12). If SMT
   discharges the obligation the operation is `total`; otherwise the
   enclosing function's row acquires `panic`. There is no panic *syntax*;
   explicit panics are a prelude function.
6. **Integer division is floor division.** On integer types `/` rounds
   toward negative infinity and `%` is the matching floor modulus (the
   result carries the divisor's sign), preserving
   `(a / b) * b + a % b == a`. These are Python's semantics — the
   reference-implementation language — not C/Rust truncation, so
   differential tests agree without adjustment. `I64::MIN / -1` is an
   overflow obligation like any other. On `F64` the operators are IEEE 754.
7. **Termination:** a recursive definition needs structural decrease or a
   `decreases` measure; failing both, its row acquires `div` (design §3.4).
8. **Derived functions** are reached by qualification: `Row::eq`,
   `Row::show`, `Row::compare`, `Row::hash` (design §3.7).
9. **Constructor resolution has exactly one legal spelling.** A bare
   constructor resolves iff its variant name is unique among the sum
   types in scope, and the bare form is then the *only* legal form;
   when two types in scope share the variant name, the qualified
   `Type::Ctor` form is required. Qualifying a unique constructor is an
   error. One spelling per context (design §3.15); the error on a
   collision names the candidate types. The rule applies identically in
   expressions, patterns, and the predicate language's `is` tests
   (tr-grammar §2.3).
10. **Definition names follow the same exactly-one-spelling rule**
    (resolved 2026-08-22, design §3.15). A bare name resolves iff it is
    unique across the visible definition set (the manifest, design
    §6.1) plus the builtins — so the prelude is called bare — and bare
    is then the only legal form; when two modules define the name, the
    qualified `module::def` form is required, and qualifying a unique
    name is an error. Private definitions are visible only within
    their own module and cannot be qualified (`::` takes a plain
    `ident` on the right; private-idents never cross modules).
11. **Typed holes** (adopted 2026-08-23, design §3.16). `?name` is an
    expression of any type; hole names are unique within a definition
    (a repeated hole name is an error). A definition containing holes
    *checks*, with each hole's goal type reported; it is `partial` —
    never testable, never `accepted`, never built, and `run`/`test`
    refuse it (`unfilled-hole`). Holes are a lowering-time state.

## 6. Interaction with the value encoding

`Bool` is a prelude sum type but encodes as JSON `true`/`false`, not
`{"tag": "True"}` — the one special case in the sum encoding, matching what
every host expects (recorded in tr-grammar §7).

## 7. Worked examples

The checked-in sample (`examples/read_file.soil`):

```
read_file : (fs : Fs) -> (path : Path) -> io (Result Utf8 FsError)
read_file fs path =
  match fs_read_bytes fs path with
  | Err e -> Err e
  | Ok bytes ->
    match utf8_decode bytes with
    | Ok text -> Ok text
    | Err _ -> Err (NotUtf8 { path = path })
```

A measure, symbolic operators, and notation elaboration:

```
gcd : U64 -> U64 -> U64
decreases b
gcd a b = if b == 0 then a else gcd b (a % b)
```

Row polymorphism, a lambda, and qualification:

```
sum_lengths : (rows : List Row) -> I64
sum_lengths rows =
  fold (fun acc r -> acc + len r.cells) 0 rows
```

## 8. Open questions raised by this spec

All questions raised by the first draft have been resolved and folded in:
partial record patterns require `..` (§3.4); the term layer uses the word
connectives, unifying with the predicate language (§1, §3.3); the string
escape set is fixed and scalar-value-only (§1); `let` bindings are
parameterless and functions are explicit lambdas (§3.3).

Constructor-name ambiguity, surfaced by impl plan 02 (its §9.1), was
resolved 2026-08-22: `Type::Ctor` qualification with the
exactly-one-spelling rule (§3.3, §3.4, §5.9); design §3.15 records the
rationale and the rejected alternatives.

**Canonical text form**: adopted 2026-08-23 and specified in §9 below
(soil0 impl step 11); `soil_hash` is computed over the canonical text of
the alpha-normalized AST (design §6.1).

## 9. Canonical form

*Added 2026-08-23 (impl plan 02 step 11; design §4.8). Soil has exactly
one printed form per AST: the printer is a compiler pass, `soil0 print`
emits it, `parse → print` is a fixpoint, and the lowerer's `write_soil`
canonicalizes — the agent never controls formatting. Layout is
**structural and width-independent**: no rule consults line length. The
checked-in examples are the style oracle — printing them is
byte-identity, which is a conformance test.*

- **Definitions**: the signature on one line (`name : type`); the
  `decreases` line; the equation head `name p1 … =` with the body inline
  on the same line when it is not a block, else on the next line at
  indent 1. Definitions in `_private.soil` are separated by one blank
  line. Indentation is two spaces per level; no trailing whitespace; the
  file ends with one newline.
- **Blocks** are `let`, `if`, and `match`; they lay out multi-line in
  statement positions (definition bodies, let bodies, arm bodies,
  `then`/`else` operands) and single-line inline everywhere else
  (parenthesized argument positions).
- **`let`** at indent *k*: `let [rec] name = value in` on one line when
  the value is inline; the body follows at indent *k*. A block-valued or
  block-bodied-lambda binding prints its header (`let f = fun a b ->`),
  the value block at *k*+1, then `in` alone at *k*, then the body at
  *k*.
- **`if`** at *k*: `if cond`, `then X`, `else Y` on three lines at *k*;
  a block operand moves to *k*+1 on the following line.
- **`match`** at *k*: `match scrutinee with` then one `| pattern ->
  body` line per arm at *k*; a block arm body moves to *k*+1.
- **Parentheses are minimal by precedence** (§4): emitted only where
  reparsing would change the tree — plus one style rule from the
  examples: an applied type constructor after an effect row is
  parenthesized (`io (Result Utf8 FsError)`).
- **Spacing**: single spaces around binary operators, `:` in signatures
  and refinements, `=` in bindings and record fields, and `|` in
  refinements; `{ a = 1, b = 2 }` record spacing; `x.f` and `m::f`
  unspaced; unary `-` attached — except that a negated negation
  prints `-(-x)`, since attached `--` would lex as a comment (a
  consequence of the minimal-parens rule; found by the printer's
  fixpoint property, 2026-08-26).
- **Literals**: integers and floats print their underscore-free source
  text; strings escape exactly `\"`, `\\`, `\n`, `\r`, `\t`, and
  lowercase `\u{…}` for remaining control characters, all other
  characters raw.

### 9.1 Hash form

*Added 2026-08-24 (impl plan 03 §9.2; design §6.1, lock-schema §3):
the text `soil_hash` is computed over. The canonical form of §9 is the
display form; the hash form is the same text with local names
alpha-normalized, so renaming a local binder can never move a hash.*

- The hash form is the §9 canonical text with every **local binder**
  — equation parameters, `let`/`let rec` binders, `fun` parameters,
  and pattern binders — replaced by `%N`, where `N` numbers binding
  sites `0, 1, 2, …` in pre-order of appearance within the
  definition; every occurrence of a bound name prints its binder's
  `%N`. `_` binds nothing and stays `_`.
- Everything at definition level or above is untouched: the
  definition's own name, callee and private-helper names, type names,
  constructors, field names, builtin names, and hole names (`?name`
  is a named goal; renaming it is a visible change to a lowering-time
  state that never reaches `tested`).
- The **signature line keeps its declared parameter names** — they
  are spec surface: the `.tr` signature and its `requires`/`ensures`
  predicates name them. Consequently renaming a *parameter* is a
  visible spec change that moves `formal_hash` and `soil_hash`
  together; only `let`, `fun`, and pattern binder renames are
  invisible (clarified 2026-08-28, with the step-4 hash tests).
- The hash form is **not reparsable** (`%N` is not in the grammar)
  and is not a CLI surface: it is produced by a `soil0` library
  function the daemon calls. The CLI contract's `print` remains the
  display form; soilc's printer (plan 05) is differentially tested
  against `print`, with the hash transform one shared implementation
  above it. Exposing a `print --hash-form` flag would be a future
  additive contract amendment; none is planned.
