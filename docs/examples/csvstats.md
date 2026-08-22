# csvstats

A hand-written module exercising the formats: a module header, two types
(`Row`, `ParseError`), and three functions (`parse_row`, `mean`, `median`). Not every definition has every layer — `median` is the
only one carried through all three (`.tr` → `.soil` → `.lock`); `_module`,
`row`, and `parse_row` have `.tr` + `.lock`; `parse_error` and `mean` are
spec-only. The canonical files live in
[`examples/csvstats/`](https://github.com/rctcwyvrn/trellis/tree/main/examples/csvstats);
these are included verbatim.

## _module (module header: `.tr` + `.lock`)

### _module.tr

````markdown
{{#include ../../examples/csvstats/_module.tr}}
````

### _module.lock

```json
{{#include ../../examples/csvstats/_module.lock}}
```

## row (type: `.tr` + `.lock`)

### row.tr

````markdown
{{#include ../../examples/csvstats/row.tr}}
````

### row.lock

```json
{{#include ../../examples/csvstats/row.lock}}
```

## parse_error (type: `.tr` only)

### parse_error.tr

````markdown
{{#include ../../examples/csvstats/parse_error.tr}}
````

## parse_row (function: `.tr` + `.lock`)

### parse_row.tr

````markdown
{{#include ../../examples/csvstats/parse_row.tr}}
````

### parse_row.lock

```json
{{#include ../../examples/csvstats/parse_row.lock}}
```

## mean (function: `.tr` only)

### mean.tr

````markdown
{{#include ../../examples/csvstats/mean.tr}}
````

## median (function: `.tr` + `.soil` + `.lock`)

### median.tr

````markdown
{{#include ../../examples/csvstats/median.tr}}
````

### median.soil

```
{{#include ../../examples/csvstats/median.soil}}
```

### median.lock

```json
{{#include ../../examples/csvstats/median.lock}}
```
