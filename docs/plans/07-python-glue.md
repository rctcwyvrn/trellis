# Plan 07 — the Python-glue project: the second Trellis project

*References: design §11 step 6, §1.2–1.3 (users and the pitch),
bootstrap plan §0 (the bias this milestone exists to correct).*

## Goal

Write a real program the author would otherwise have vibed in pure
Python: Rust crates through `soil-rs-std`, a dozen Python functions
through `trellis bind`, real work done in Soil. The compiler validated
the pure core; this validates FFI, capabilities, bindings, and — the
actual product question — whether writing `.tr` files and reading
generated Soil is *pleasant*. This milestone's deliverable is as much a
verdict as a program.

## Scope

1. **Pick the program with the user.** Criteria: genuinely wanted (not a
   demo), touches files/network/clock (exercises three capabilities),
   needs ~a dozen foreign symbols across 2–3 Python packages plus at
   least one Rust crate, small enough to finish (order of 30–60
   definitions).
2. **Work disciplined-tier by the book.** Human-written `.tr` prose and
   tests, agent lowerings, `accepted` gates, export pins, no hand-edited
   Soil unless the escape is genuinely needed (and then noted). The point
   is to feel the friction a real user feels; do not use insider
   shortcuts.
3. **Bind on demand.** Every foreign symbol through `trellis bind` as
   encountered, never batched up front — this tests the assistant's
   real cadence (design §3.11's "a dozen bindings is a week of friction"
   claim, now measured).
4. **Keep a friction log.** A running document (not memory, not code):
   every point where the format, the tooling, the corpus, or the agent
   made the wrong thing easy or the right thing hard, with enough
   context to act on. This log is the primary input to the next round of
   design changes.
5. **Ship it.** The program builds via `trellis build` to a native
   binary, runs cram-tested against the real world, and gets used.

## Non-goals

New toolchain features mid-flight (log them instead — resist fixing the
tool from inside the project except for outright blockers); team
features; `trellis derive`; performance work beyond what debug-mode
flame graphs reveal for free.

## Testing

The project's own tests are the milestone's tests: expect/property on
pure logic, fake-capability tests on `io`, contract tests on every
binding, cram on the entry point. CI policy line in `soil.toml`: every
exported definition `accepted`.

## Exit criteria

- The program works and is actually used by the author.
- Every binding came through `trellis bind`; every definition is
  `accepted`; the lock audit view shows exactly which trust levels and
  escape hatches exist.
- The friction log is reviewed with the user and triaged into design
  changes, tooling issues, and corpus fixes — closing the loop that the
  compiler-first ordering deliberately left open.

## Decision points — resolved 2026-08-22

- **The program is chosen at milestone kickoff**, not now — the right
  project is whatever the author genuinely wants built when the tooling
  is real. The criteria in §1 are the filter; a stale pre-commitment
  would defeat the "genuinely wanted" requirement.
- **Friction cadence: fix after shipping, except outright blockers.**
  The project is a measurement of real-user friction; mid-flight fixes
  with insider knowledge would contaminate it. Log and work around
  during; review and triage (design changes / tooling issues / corpus
  fixes) after.
