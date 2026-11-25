# Agent test (8.D acceptance)

Ten small programs written from `templates/common/AGENTS.md` (the version before this test, plus the
loop). Each round is one `oxi check` run over the file. `first_try/` holds round 1, `final/` the programs
that pass. Caveat: the author had read grammar.md and the research report earlier in the same session, so
this is not a clean-room test of a model that knows nothing else; the programs were written as a
TypeScript-fluent model would, without consulting anything but AGENTS.md.

| # | Program | Rounds | Diagnostics seen |
|---|---|---|---|
| 01 | fizzbuzz | 1 | none |
| 02 | clamp / lerp (f64) | 2 | E0100: `n / 10` passed where `f64` is expected |
| 03 | Stack class with `T \| null` | 1 | none |
| 04 | temperature struct | 1 | none |
| 05 | parse digits over `s.chars()` | 2 | E0110: `total * 10 + (ch - 48) as i32` is `(total*10 + (ch-48)) as i32` (`as` binds looser than `+`) |
| 06 | shape union, exhaustive switch | 1 | none |
| 07 | event counter with `main(log, state)` | 1 | none |
| 08 | array stats / mean | 2 | E0100: integer `/` gives an integer, `f64` expected |
| 09 | safe divide with `?` chain | 1 | none |
| 10 | enum + switch expression + law | 1 | none |

Result: 7 of 10 passed on the first check, 3 needed one fix each. 13 `oxi check` runs for 10 programs
(1.3 per program), 0 programs needed a third round. None of the three fixes was machine-applicable (all
are `maybe-incorrect` type-level repairs), so the "apply fixes" half of the loop was exercised by hand.

Eight further programs (`extra_first_try/`, 11-18; wrapping `+%=`, generics, interface bound, closures,
code points, linked list with `T | null`, `range`) found two more gaps: an exported function could not be
generic (E0121; lifted by the 2026-11-26 `@host` decision, `export` is now module visibility only) and `?` needs the very same error type (E0104).

Guidance that was missing and is now in AGENTS.md / the skill:

1. `as` binds looser than arithmetic: parenthesise each operand (`(a as f64) + (b as f64)`).
2. Integer `/` yields an integer; convert both operands before dividing when a float is wanted.
3. A literal adapts to its context but a typed variable never converts implicitly.
4. (Superseded 2026-11-26.) `export` is module visibility only; only `@host export function` is limited to the ABI types and cannot be generic.
5. `?` needs the same `E`; there is no automatic error conversion.
6. `range(a, b)` exists (it was missing from the list of built-ins).

`cargo test -p oxiscript-cli` re-checks the record: `final/` passes, `first_try/` fails exactly on 02, 05 and 08 with the codes above, and extra 16 fails with E0104 (extra 12 failed with E0121 until 2026-11-26, now passes).
