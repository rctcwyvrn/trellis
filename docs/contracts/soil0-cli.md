# The `soil0` CLI — Oracle Contract

*Status: **frozen — contract v1.1** (v1 approved 2026-08-22; amended to
v1.1 on 2026-08-23 with the agentlanguages-survey adoptions — typed
holes: the `Hole` expression, `checks.holes`, the `unfilled-hole` code,
`run`/`test` refusal; and the reserved `print` command for the canonical
printer, arriving with impl step 11. The amendments are additive: v1
outputs are byte-identical for hole-free programs, and `soil0_cli`
stays `1` for additive amendments.) This document is the compatibility
contract of the `soil0` binary, the format every differential oracle
for `soilc` (plan 05) compares against, and the surface the daemon
(plan 03) drives. Changing anything here is a contract change requiring
a user decision. The `soil0` *library* API is explicitly not covered
(§12).
References: impl plan 02 (§1 decisions, §8 micro-pins), plan 02
decision points, `docs/soil-syntax-spec.md`, tr-grammar §2.3 and §7,
lock-schema §4–§5, impl plan 01 §4.7 (descriptor JSON).*

---

## 1. Conventions

- Invocation: `soil0 <command> [flags] [input]`, commands `lex` |
  `parse` | `rename` | `infer` | `check` | `run` | `test` (v1.1
  additionally reserves `print`, the canonical printer — impl step 11).
  A global `--version` flag prints
  `{"soil0_cli":1,"soil0":"<crate version>"}` and exits 0; `soil0_cli`
  is this document's major version and does not bump for additive
  amendments.
- `lex` and `parse` take one `.soil` file; `rename`, `infer`, `check`,
  `run` take a program manifest (§7); `test` takes a bundle (§10). All
  inputs are UTF-8 files named by path — nothing is read from stdin.
- **stdout** carries exactly one JSON document on success: compact, no
  whitespace, single line, object keys in the field order this document
  declares. Golden tests byte-compare stdout.
- **stderr** carries exactly one diagnostics document (§2) on failure,
  same compactness rules.
- Exit codes: `0` success; `1` the *input program* violates the spec
  (any diagnostic), and for `run`/`test` the runtime outcomes noted in
  §8.6–§8.7; `2` usage or environment error — malformed manifest,
  env, bundle, or AST JSON, unreadable file, manifest order violation,
  or an internal bug.
- JSON documents embedded in inputs and outputs follow tr-grammar §7
  conventions wherever the data is Soil-shaped: internally tagged sums
  (`{"tag": …, "value": …}`, nullary tags bare), records with every
  field present, optionality via the `Option` encoding
  (`{"tag":"None"}` / `{"tag":"Some","value":…}`), booleans as JSON
  `true`/`false`. *Soil values* (run results, expected test values) are
  produced only by `soil-rt`'s canonical encoder.

## 2. Diagnostics

```json
{"errors":[{"code":"shadowing","message":"…","file":"median.soil",
  "span":{…},"notes":[{"message":"…","file":"…","span":{…}}]}]}
```

- `code` is a stable kebab-case string from the registry below — the
  rejection corpus keys on it; renaming a code is a contract change,
  adding one (for a new rule) is not.
- `file` and `span` are `Option`-encoded (a usage error has neither).
- `notes` attach secondary locations and facts: the prior binding site
  for `shadowing`, candidate types for `ambiguous-constructor`, the
  witness pattern for `non-exhaustive-match`.

### Error-code registry

| Phase | Codes |
|---|---|
| lex | `unterminated-string`, `bad-escape`, `escape-not-scalar`, `stray-character` |
| parse | `parse-expected` (generic expected/found), `nonassoc-comparison`, `name-mismatch` (signature vs equation defname), `wildcard-in-let-rec`, `misplaced-private-def` (private def outside `_private.soil`, or a non-private def inside one), `multiple-defs` (a non-private file with more than one definition) |
| rename | `shadowing`, `unknown-name`, `unknown-type`, `unknown-qualified`, `unknown-constructor`, `ambiguous-constructor`, `ambiguous-name` (definition names follow the same exactly-one-spelling rule, syntax spec §5.10), `needless-qualification`, `private-cross-module` |
| infer | `type-mismatch`, `effect-violation` (`io`/`ffi` deficits only — §8.5), `operator-polymorphic`, `non-derivable`, `annotation-needed` (unresolved record type at field access or constructor), `unknown-field`, `literal-out-of-range` |
| match | `non-exhaustive-match`, `redundant-arm`, `missing-record-rest`, `duplicate-field` |
| runtime | `runtime-panic` (§8.6), `unfilled-hole` (v1.1: `run`/`test` on a definition — or transitive callee — containing typed holes, §8.6) |
| class 2 | `usage`, `io`, `malformed-input`, `forward-reference`, `internal` |

## 3. Spans

```json
{"start":42,"end":47,"line":3,"col":5}
```

`start`/`end` are UTF-8 byte offsets into the file, end-exclusive —
the canonical span (plan 02 resolved). `line`/`col` are 1-based,
derived from `start`, display-only: oracle comparisons may not depend
on them beyond the derivation rule.

## 4. The AST JSON schema

The AST is specified as Soil type declarations; its JSON encoding
follows mechanically from tr-grammar §7 (this is the schema soilc's
Trellis type definitions must reproduce, plan 05). Every tree node is
wrapped:

```
type Span      = { start : U64, end : U64, line : U64, col : U64 }
type Spanned a = { span : Span, item : a }
type Binder    = { span : Span, name : Utf8 }
```

### 4.1 Files and definitions

```
type File = { defs : List (Spanned Def) }
type Def  = { name : Utf8
            , sig : Spanned Type
            , decreases : Option (Spanned Expr)
            , params : List Binder
            , body : Spanned Expr }
```

`parse` emits one `File`; a `<name>.soil` file has exactly one def, a
`_private.soil` any number (syntax spec §2).

### 4.2 Types, rows, predicates

```
type Effect = Div | Panic | Io | Ffi
type Row    = { effects : List Effect, var : Option Utf8 }

type Type =
  | Arrow   { param : Option Binder, dom : Spanned Type
            , row : Row, cod : Spanned Type }
  | Con     { name : Utf8, args : List (Spanned Type) }
  | TVar    { name : Utf8 }
  | Refined { binder : Binder, base : Spanned Type, pred : Spanned Pred }
```

- `effects` always in the fixed order `Div, Panic, Io, Ffi`; the empty
  row (`total`) is `{"effects":[],"var":{"tag":"None"}}`. A row
  attaches only to an arrow's codomain position, per the grammar.
- `Con` covers both bare (`I64`, args `[]`) and applied
  (`Result Utf8 FsError`) type constructors — one node.
- Refinement predicates are parsed and retained (plan 02 scope 4):

```
type Pred =
  | Implies { lhs : Spanned Pred, rhs : Spanned Pred }
  | POr     { lhs : Spanned Pred, rhs : Spanned Pred }
  | PAnd    { lhs : Spanned Pred, rhs : Spanned Pred }
  | PNot    { pred : Spanned Pred }
  | PCmp    { op : CmpOp, lhs : Spanned PExpr, rhs : Spanned PExpr }
  | Is      { scrutinee : Spanned PExpr, type_name : Option Utf8
            , ctor : Utf8, payload : Option Binder }
  | PCall   { name : Utf8, args : List (Spanned PExpr) }

type PExpr =
  | PLiteral { lit : Lit }
  | PPath    { root : Utf8, fields : List Utf8 }
  | PArith   { op : ArithOp, lhs : Spanned PExpr, rhs : Spanned PExpr }
  | PCallE   { name : Utf8, args : List (Spanned PExpr) }
```

`PArith` admits only `Add`/`Sub`/`Mul` (tr-grammar §2.3); the parser
enforces it. `Is.type_name` is the optional `Type::` qualification
(exactly-one-spelling rule, syntax spec §5.9).

### 4.3 Expressions

```
type Lit = LInt { digits : Utf8 }     -- underscores stripped; sign never included
         | LFloat { text : Utf8 }     -- source text, underscore-free
         | LStr { value : Utf8 }      -- escapes decoded

type CmpOp   = Eq | Ne | Lt | Le | Gt | Ge
type ArithOp = Add | Sub | Mul | Div | Mod

type Expr =
  | Let       { is_rec : Bool, bindings : List LetBinding, body : Spanned Expr }
  | Fun       { params : List Binder, body : Spanned Expr }
  | If        { cond : Spanned Expr, then_branch : Spanned Expr
              , else_branch : Spanned Expr }
  | Match     { scrutinee : Spanned Expr, arms : List (Spanned Arm) }
  | OrE       { lhs : Spanned Expr, rhs : Spanned Expr }
  | AndE      { lhs : Spanned Expr, rhs : Spanned Expr }
  | NotE      { operand : Spanned Expr }
  | Cmp       { op : CmpOp, lhs : Spanned Expr, rhs : Spanned Expr }
  | Arith     { op : ArithOp, lhs : Spanned Expr, rhs : Spanned Expr }
  | Neg       { operand : Spanned Expr }
  | App       { fn : Spanned Expr, arg : Spanned Expr }
  | Path      { root : Utf8, fields : List Utf8 }
  | Qualified { space : Utf8, name : Utf8 }
  | CtorE     { name : Utf8 }
  | RecordE   { update : Option PathBase, fields : List FieldInit }
  | Literal   { lit : Lit }
  | Annot     { expr : Spanned Expr, ty : Spanned Type }
  | Hole      { name : Utf8 }                          -- typed hole `?name` (v1.1, design §3.16)

type LetBinding = { binder : Option Binder, value : Spanned Expr }  -- None = "_"
type Arm        = { pattern : Spanned Pattern, body : Spanned Expr }
type PathBase   = { root : Utf8, fields : List Utf8 }
type FieldInit  = { name : Utf8, value : Spanned Expr }
```

- A bare variable is `Path` with `fields = []` — there is no separate
  `Var` node (one way). `root` may be a `private-ident`.
- `App` is binary and left-nested, faithful to juxtaposition.
- `Qualified` covers `module::def`, `Type::derived`, and `Type::Ctor`;
  the distinction is semantic (rename), not syntactic.
- Constructor application is `App` with a `CtorE` or `Qualified` head.
- Parenthesization is not represented; precedence is already resolved.

### 4.4 Patterns

```
type Pattern =
  | PWild
  | PBind   { binder : Binder }
  | PLit    { lit : Lit }
  | PCtor   { type_name : Option Utf8, name : Utf8
            , arg : Option (Spanned Pattern) }
  | PRecord { fields : List FieldPat, open : Bool }

type FieldPat = { name : Utf8, pattern : Option (Spanned Pattern) }  -- None = punning
```

`PRecord.open` is the trailing `..`; `PCtor.type_name` is the
`Type::` qualification (§5.9 rule applies in patterns identically).

### 4.5 Tokens (`lex` output)

```
type Token = TIdent { name : Utf8 } | TPrivate { name : Utf8 }
           | TTypeName { name : Utf8 } | TKeyword { word : Utf8 }
           | TOp { op : Utf8 }
           | TInt { digits : Utf8 } | TFloat { text : Utf8 }
           | TStr { value : Utf8 }
```

`lex` emits `{"tokens": List (Spanned Token)}`. Comments and whitespace
are dropped; there is no EOF token. Keywords are the §1 keyword list of
the syntax spec (effect names are *not* keywords — they lex as
`TIdent`).

## 5. Semantic types (signature output)

`infer`/`check` report signatures span-free, refinements erased:

```
type SigType = SArrow { param : Option Utf8, dom : SigType
                      , row : Row, cod : SigType }
             | SCon   { name : Utf8, args : List SigType }
             | SVar   { name : Utf8 }
```

Canonical renaming: type variables and row variables form one sequence
`a, b, c, …` (`…, z, a1, b1, …`) assigned in order of first appearance
in a pre-order walk of the signature. Named parameters (`(fs : Fs)`)
keep their declared names.

## 6. `env.json` — the type environment

```
type EnvFile  = { types : List TypeDef }
type TypeDef  = { name : Utf8, params : List Utf8
                , strategy : Strategy, body : TypeBody }
type Strategy = Structural | Opaque
type TypeBody = Record { fields : List FieldD }
              | Sum { variants : List VariantD }
              | OpaqueBody
              | Alias { ty : SigType }
type FieldD   = { name : Utf8, shape : SigType
                , ignored : Option IgnoredDefault }   -- Const | CopyField, impl plan 01 §8.14
type VariantD = { name : Utf8, payload : Option SigType }
```

Field, payload, and alias types use the **semantic type encoding**
(§5) — one type encoding for signatures and environments (resolved
2026-08-22, §13.3). `SCon` covers the scalars (`I64`, `Utf8`, `Unit`,
…), `List`/`Map`, and named types with arguments; `SVar` references a
`params` entry (`params` is required, `[]` for ground types). `SArrow`
shapes are **rejected in v1** (class-2 `malformed-input`): nothing
needs function-typed fields yet, and lifting the rejection later is a
behavior change, not a format change. soil0 lowers `SigType` to
`soil-rt` descriptor shapes internally when registering ground
instances on demand (an arrow would lower to the runtime's shapeless
`Closure`, which poisons derivation); the runtime's own descriptor JSON
(impl plan 01 §4.7) keeps its roles — fixtures and the C ABI —
unchanged. Aliases are checker-level and erased before registration.

### 6.1 The kernel

`soil0` pre-registers the kernel at startup; an `env.json` entry
reusing a kernel name is `malformed-input`. Normative declarations
(shapes provisional where marked — plan 04's prelude `.tr` specs must
adopt or revise them with the user, like the builtin names):

```
type Bool     = True | False            -- pre-registered by soil-rt
type Option a = None | Some a
type Result a e = Ok a | Err e          -- success-first (design §3.3)
type Path     = Utf8                    -- alias
type FsError  =                         -- provisional shape
  | NotFound   { path : Path }
  | ReadFailed { path : Path, message : Utf8 }
  | NotUtf8    { path : Path }
type Utf8Error = InvalidUtf8 { at : U64 }   -- provisional shape
type World = opaque
type Fs = opaque      type Net = opaque    type Clock = opaque
type Env = opaque     type Proc = opaque   type Rand = opaque
type Py = opaque
```

All seven capability types exist (design §3.5); only `Fs`, `Clock`,
`Rand` have primitives in v1 (§11). `Unit` and the numeric/string/bytes
scalars are built-in shapes, not declarations.

## 7. `program.json` — the manifest

```json
{"types":"env.json","defs":["stubs/list_wrappers.soil","sort_by.soil","median.soil"]}
```

- `types`: path to an `env.json`, relative to the manifest's directory
  (an empty environment is `{"types":[]}` in that file).
- `defs`: `.soil` paths relative to the manifest's directory, ordered
  **callee-before-caller**; a forward reference is class-2
  `forward-reference` (the caller owns the dependency tree — the daemon
  in production, the test harness before that; handwritten manifests
  are test fixtures only).
- A definition's *module*, for `_private` visibility and `module::def`
  resolution, is its file's parent directory name.

## 8. Commands

### 8.1 `soil0 lex <file.soil>`

Tokens per §4.5. Fails only with lex-phase diagnostics.

### 8.2 `soil0 parse <file.soil>`

`File` per §4.1–§4.4. File role (private or not) comes from the
filename.

### 8.3 `soil0 rename <program.json>`

Success: per-definition **reference sets** — the computed import set
(design §6.1) and the lock's call-edge data:

```json
{"defs":[{"name":"median","refs":{"defs":["len","nth","sort_by"],
  "types":["F64","List"],"builtins":[],"privates":[]}}]}
```

Definitions in manifest order; each list sorted lexicographically.
References are *surface* references — operator elaboration targets
(`F64::add`, …) appear only after `infer` and are not listed.

### 8.4 `soil0 infer <program.json>`

Success, per definition in manifest order:

```json
{"defs":[{"name":"median","type":{…SigType…},"row":{"effects":[],"var":{"tag":"None"}},
  "checks":{"termination":"unverified","panic_obligations":[
    {"kind":{"tag":"DivZero"},"span":{…}}]}}]}
```

`type`/`row` per §5 (the declared signature, checked). `checks` per
§8.5. The `--dump-ast` flag additionally writes the elaborated,
per-node-typed AST to stdout instead — **non-contractual**, free to
change without notice (impl plan 02 §1).

### 8.5 The check-facts model

`soil0` has no SMT solver and no termination checker, but the normative
examples claim `total`. Failing them would contradict the exit
criteria, and widening their signatures would contradict design §6.4
("the agent never silently widens"). The resolution is the demotion
philosophy plans 04–05 already adopted for the lock (lock-schema §4):

- **`io` and `ffi` subsumption is enforced.** A callee's declared
  `io`/`ffi` not covered by the caller's declared row is
  `effect-violation` — these effects are about capabilities and
  reality, not proof.
- **`div` and `panic` deficits are recorded, never errors.** They are
  proof obligations a later stage discharges (termination checker,
  refinement checker — plan 05); until then they are unproven claims:
  visible, runtime-checked, tests still gating.
  - `termination`: `"verified"` (no recursion and no `div` deficit) |
    `"unverified"` (self-recursion, `let rec`, or a call to a
    declared-`div` callee, while the row claims no `div`) | `"n/a"`
    (the declared row carries `div`) — lock-schema §4 vocabulary.
  - `panic_obligations`: the sites whose potential panic is *not*
    covered by the declared row, each
    `{"kind": Overflow | DivZero | CalleePanic { callee : Utf8 }, "span": …}`;
    empty when the row carries `panic` or no such site exists.
    Arithmetic sites are always-on runtime checks regardless (plan 02
    scope 6).
  - `holes` (v1.1): the definition's typed holes in source order, each
    `{"name": …, "span": …, "ty": <SigType>}` with the goal type's
    unresolved variables canonically renamed. Nonzero holes is the
    `partial` state (lock-schema §4): the definition checks but can
    never be tested, accepted, or built (design §3.16).

Declared signatures remain the modular truth for callers; facts do not
propagate (the daemon tracks them per definition in the lock).

### 8.6 `soil0 run <program.json> --entry <name> --args <json-array>`

Static pipeline first (any diagnostic aborts, exit 1), then:
capability-typed entry parameters are injected from a real `World` in
order; the remaining parameters decode type-directedly from the
`--args` array positionally (entry parameter types must be ground).
Real capabilities: `Fs` is the process filesystem with paths resolved
against the working directory, `Clock` the system clock, `Rand` OS
entropy. Mirrors `trellis call` (tr-grammar §3.5).

stdout on success: the result value's canonical JSON, nothing else. A
runtime `SoilError` exits 1 with a single `runtime-panic` diagnostic
whose message is the panic kind and message and whose `notes` carry the
definition-level trace, outermost first. If the entry — or any
transitive callee — contains typed holes, `run` (and `test`, for the
definition under test) refuses with `unfilled-hole` (v1.1, design
§3.16).

### 8.7 `soil0 check <program.json>`

The full static pipeline — parse, rename, infer, exhaustiveness and all
static rules. Success output is identical to `infer`'s (§8.4); failure
is the combined diagnostics of every phase that ran.

### 8.8 `soil0 test <bundle.json>`

See §10.

### 8.9 `soil0 print <file.soil>` (v1.1)

The canonical printer (syntax-spec §9): stdout is the file's canonical
**Soil text** — the one documented exception to the JSON-stdout rule
(§1), since the output *is* source code. `parse → print` is a fixpoint
and the checked-in examples are byte-identical under it; `soil_hash`
(plan 03) is computed over this text after alpha-normalization
(lock-schema §3).

## 9. Exhaustiveness diagnostics

`non-exhaustive-match` carries one note per missing case with a witness
pattern rendered in surface syntax (`Err _`, `{ kind = NotFound, .. }`);
`redundant-arm` spans the useless arm. Literal patterns (`LInt`,
`LStr`, `LFloat`) never exhaust their type; a match over them requires
a default arm.

## 10. Test bundles

Assembled by the caller (daemon; harness; fixtures), one bundle per
invocation:

```json
{"program":"program.json",
 "cases":[
   {"name":"found-and-missing#1","def":"read_file",
    "binds":[{"bind":"fs","ctor":"fake_fs",
      "args":[[{"key":"config.toml","value":"port = 8080"}]]}],
    "args":[{"tag":"Binding","value":{"name":"fs"}},
            {"tag":"Json","value":"config.toml"}],
    "expect":{"tag":"Value","value":{"tag":"Ok","value":"port = 8080"}},
    "xfail":false}]}
```

- `binds` construct fake capabilities: `ctor` names a builtin fake
  constructor, its `args` are JSON decoded against the constructor's
  parameter types (canonical §7 forms — a map is a key/value array).
- `args` fill the definition's parameters in order: `Binding`
  references a bind, `Json` decodes against the parameter's type.
- `expect` is `Value` (compared by canonical-JSON byte equality after
  decode/re-encode normalization, impl plan 02 §8.10) or `Panic`
  (matches any `SoilError` raised by the call).
- Output, cases in bundle order:

```json
{"cases":[{"name":"found-and-missing#1","result":"pass",
  "details":{"tag":"None"}}]}
```

`result` ∈ `pass` | `fail` | `xfail` | `xpass` (lock-schema §5);
`details` is `Some {expected, actual}` (both strings: canonical JSON,
or `"panic"` / the panic rendering) exactly when the result is `fail`
or `xpass`. Exit code: `0` iff no case is `fail` or `xpass`; the report
prints regardless. Bundle-level problems (unknown `def`, undecodable
args) are class-2, never case results.

## 11. Builtins

The native definitions visible to every program. Names and signatures
are **provisional prelude surface** (impl plan 02 §8.7, resolved
2026-08-22): plan 04's prelude `.tr` specs adopt these names, or revise
them with the user and update this table. Strict-minimum inventory
(2026-08-22): `Env`/`Proc`/`Net`/`Py` have no primitives yet; adding
one is a new decision point.

| Name | Signature | Semantics |
|---|---|---|
| `world_fs` | `World -> Fs` | derive the real filesystem capability |
| `world_clock` | `World -> Clock` | derive the real clock |
| `world_rand` | `World -> Rand` | derive real entropy |
| `fs_read_bytes` | `Fs -> Path -> io (Result Bytes FsError)` | read a whole file; `NotFound` / `ReadFailed` on error |
| `clock_now` | `Clock -> io I64` | seconds since the Unix epoch (provisional — design §3.5 sketches `now : Clock -> io Time`; plan 04 reconciles) |
| `rand_u64` | `Rand -> io U64` | next random value |
| `fake_fs` | `Map Utf8 Utf8 -> Fs` | in-memory fs: hit returns `Ok (utf8_encode contents)`, miss `Err (NotFound { path })` |
| `fake_clock` | `I64 -> Clock` | `clock_now` returns the given constant on every call |
| `fake_rand` | `U64 -> Rand` | deterministic SplitMix64 stream from the seed (§11.1) |
| `utf8_decode` | `Bytes -> Result Utf8 Utf8Error` | validate; `InvalidUtf8 { at }` gives the first bad byte offset |
| `utf8_encode` | `Utf8 -> Bytes` | the underlying bytes |
| `unit` | `Unit` | the unit value — there is no `()` literal; this is the one way to write `Unit` (§13.2) |
| `list_len` | `List a -> I64` | length |
| `list_nth` | `List a -> I64 -> panic a` | zero-based index; out of range panics |
| `list_empty` | `List a` | the empty list (a polymorphic value) |
| `list_append` | `List a -> a -> List a` | append one element |

**Numeric primitives** (the §5.4 elaboration targets, also reachable as
`Type::op`): for each fixed-width integer type `T` —
`T::add`, `T::sub`, `T::mul`, `T::neg` (`T -> T -> panic T` /
`T -> panic T`, overflow panics) and `T::div`, `T::mod`
(`T -> T -> panic T`, floor division and floor modulus — Python
semantics, syntax spec §5.6 — zero divisor and `MIN / -1` panic). All
`panic`s here are obligation sites under §8.5, runtime-checked always.
`F64` ops are total IEEE 754; `BigInt::add/sub/mul/neg` are total,
`BigInt::div/mod` panic on zero. Derived functions come from `soil-rt`,
are total (closure poisoning is rejected statically as
`non-derivable`), and have the signatures `T::eq : T -> T -> Bool`,
`T::show : T -> Utf8`, `T::hash : T -> U64`, and
`T::compare : T -> T -> I64` returning −1/0/1 (provisional like the
rest of the kernel surface — an `Ordering` sum is a plan-04 option,
scope 6 there). `and`/`or`/`not` elaborate to built-in short-circuit
forms on `Bool` and are **not** callable by name.

### 11.1 SplitMix64 (pinned)

`fake_rand` outcomes appear in expect tests, so the algorithm is
observable and must never drift. State starts at the seed; each
`rand_u64` call advances state by `0x9E3779B97F4A7C15` and returns
`mix(state)` where `mix(z)`: `z ^= z >> 30; z *= 0xBF58476D1CE4E5B9;
z ^= z >> 27; z *= 0x94D049BB133111EB; z ^= z >> 31`.

## 12. Non-contractual surfaces

Free to change without notice: the `soil0` Rust library API (the daemon
links it, but conformance is defined by the CLI alone — plan 02
resolved); `--dump-ast` output; diagnostic `message` and `notes`
*wording* (codes, spans, and note structure are contractual);
performance of everything.

## 13. Open questions raised by this draft

1. **The check-facts model (§8.5).** *Approved 2026-08-22.* It extends
   the impl-plan-02 §8.3 output (adds `checks`) and interprets plan
   02's "conservatively acquires `div`" as a recorded fact rather than
   a hard failure; impl plan 02 §8.3/step 6 and plan 02 scope 4 note
   the reconciliation.
2. **There is no way to write a `Unit` value.** *Resolved 2026-08-22:
   a kernel value* `unit : Unit` (§11) — the one way to produce `Unit`,
   like `list_empty` a builtin value. Chosen over a `()` literal or a
   grammar change: no new syntax, and the grammar stays as specified.
   Plan 04's prelude adopts it with the rest of the kernel surface.
3. **Function-typed fields in `env.json`.** *Resolved 2026-08-22:
   unify on the semantic type encoding now* — field/payload/alias types
   are `SigType` (§6), with `SArrow` rejected until a prelude type
   first needs a function field; lifting the rejection is behavior, not
   format, so the frozen contract survives. Rejected alternatives: a
   shapeless `Closure` shape (cannot type a call through the field —
   every access would need a local annotation) and adding an `Fn`
   variant to a separate Shape grammar later (a third type grammar
   duplicating SigType, and extending a frozen sum is a breaking
   change for consumers that match on shapes).
4. **`FsError`/`Utf8Error` shapes and `clock_now`** are provisional
   kernel surface for plan 04 to adopt or revise (§6.1, §11).
   *Resolved 2026-08-22: deferred to plan 04*, whose scope now carries
   an explicit kernel-reconciliation item.
5. **Inline-record variant payloads are not expressible in `env.json`
   v1.** The soil-type grammar (tr-grammar §4.1) gives variants inline
   record payloads (`NotFound { path : Path }`), but `VariantD.payload`
   is a `SigType` shape, which cannot carry fields. The kernel models
   its own record payloads internally; a user `env.json` cannot declare
   one yet. The encoding decision belongs to the daemon's env generator
   (plan 03), which is the first thing that will need it — raise it
   there rather than inventing a shape now.
6. **v1.1 amendments (2026-08-23, from the agentlanguages-survey
   adoptions; approved with them).** Typed holes: `Expr.Hole`,
   `checks.holes`, `unfilled-hole`, run/test refusal — design §3.16;
   syntax-spec §5.11. The `print` command is reserved for the canonical
   printer (one byte-exact form per AST, `parse → print → parse`
   idempotence), specified and implemented as impl step 11 before the
   daemon computes the first `soil_hash`.
