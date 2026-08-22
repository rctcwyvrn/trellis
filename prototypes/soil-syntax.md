# Soil Surface Syntax — Prototype Highlights

*Status: prototype, highlights only — the full grammar comes with the Soil
core (design §11, milestone 1). Haskell for the type-level look, OCaml for
the term-level look, minimal everywhere the design tenets demand it. First
sample: `examples/read_file.soil`.*

---

## 1. Definitions

Haskell-style signature line, then one equation. Curried, strict. The
signature is mandatory in `.soil` — the file must check standalone, and the
daemon verifies it entails the `.tr` spec type (design §4.8).

```
read_file : (fs : Fs) -> (path : Path) -> io (Result Utf8 FsError)
read_file fs path = ...
```

Named parameters `(x : T)` are optional except where refinements refer to
them.

## 2. Effect rows

Space-separated, before the return type; the empty row is `total`.

```
io (Result Utf8 FsError)
ffi panic io PyObject
```

## 3. Terms

OCaml: `let … in`, `let rec … and …` (free within a file, design §3.8),
`match … with`, `fun x -> e`, `if/then/else`. No `;` — sequencing an effect
is `let _ = log clock msg in …`. Let bindings are parameterless: a
let-bound function is an explicit lambda (`let go = fun x -> … in`).
Boolean connectives are the words `and`/`or`/`not`, the same spelling as
the predicate language. One way to do things.

## 4. Pattern matching

Variant patterns bind the payload; record payloads destructure by name with
punning. Exhaustiveness is enforced. `_` is the wildcard. **No guards** —
nested `if`/`match` instead.

```
match parse_cell text with
| Ok row                  -> ...
| Err { index, text = t } -> ...
```

## 5. Records

Construct with `=`, access with `.`, functional (non-mutating) update with
`with`:

```
let r = { cells = xs } in
let r2 = { r with cells = ys } in
r2.cells
```

## 6. Sums

Nullary variants are bare; a variant carries at most one payload of any
type, and multi-field payloads are inline records (no tuples): `None`,
`Ok bytes`, `BadCell { index = 1, text = "x" }`.

`Result a e` puts the success type first (OCaml/Rust order). Haskell's
error-first `Either e a` exists so the partially applied constructor can be
a Functor instance — impossible in Soil (no type classes, no higher-kinded
abstraction), so the widely known order wins.

## 7. Refinements

Inline in `.soil` signatures, Liquid-style — the agent-facing spelling that
the `.tr`'s `requires`/`ensures` clauses desugar into:

```
median : (xs : {v : List F64 | len v > 0}) -> {r : F64 | min xs <= r and r <= max xs}
```

## 8. Termination

A `decreases` line between signature and equation when structural decrease
is not inferable (Idris-style measure, design §3.4):

```
gcd : U64 -> U64 -> U64
decreases b
gcd a b = if b == 0 then a else gcd b (a % b)
```

## 9. Comparison operators are notation, not overloading

Every type has exactly one derived `eq`/`compare` (design §3.7), so the
elaborator rewrites `x == y` to `T::eq x y` at the inferred monomorphic
type.
In polymorphic code the operators are unavailable — take the function as a
parameter (`sort_by`, map comparators), the confirmed idiom (design §3.6).

## 10. Names and modules

No import statements; the daemon resolves names through the manifest, and
the lock's import set is computed, never written. Same-module definitions
and the prelude are bare; cross-module exports are qualified
(`csvstats::median`); derived functions are `Row::eq`; private helpers are
underscore-prefixed and live only in `_private.soil`. `::` is the namespace
separator, keeping `.` exclusively for record field access.

## 11. Literals and comments

`1` is `I64`, `1.0` is `F64`, `"…"` is `Utf8`; other widths by annotation
`(42 : U32)`, no suffixes. Comments are `--`.

## 12. Deliberately absent

Tuples, guards, `;`, do-notation, exceptions, mutation, type classes,
operator sections, user-defined operators, parameterized `let` bindings.

**Deferred, not rejected:** a `let? x = e in …` sugar for `Result`
propagation. v1 writes the match explicitly; if the corpus shows it is the
dominant noise, the sugar is one desugaring rule later.
