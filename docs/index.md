# surd

**surd** is an exact-by-default computer algebra system — the engine for a
browser-based, no-install mathematical scratchpad. Try it live at
**<https://dariusjlukas.github.io/surd/>**.

A *surd* is an irrational root kept in exact symbolic form — `sqrt(2)` the
object, not `1.41421…` the approximation. That's the whole idea: results are
exact until *you* ask for a float.

```text
>> 1/3 + 1/6
1/2
>> sqrt(2)*sqrt(2)
2
>> inv([1/2, 1/3; 1/4, 1/5])
[  12  -20 ]
[ -15   30 ]
>> N(pi, 50)
3.1415926535897932384626433832795028841971693993751
```

## Design pillars

- **Exact by default.** `1/3` is the rational 1/3. `1.5` is `3/2`. `sqrt(2)`
  and `pi` are *symbolic objects*, not floats. You only get a float when you
  ask for one, via [`N(...)`](reference/numeric.md#n) — and that crossing is
  meant to be visible.
- **CAS first, numerics second.** Every value is a symbolic expression that
  may collapse to a number.
- **`:=` assigns, `=` builds an equation.** `=` never tests truth — equality
  of reals is undecidable in general (Richardson's theorem), so an equation is
  a piece of data you manipulate, not a boolean. Decidable equality is spelled
  `==`.
- **Honesty about undecidability.** Control-flow conditions must evaluate to a
  real `true`/`false`; a condition that can't be decided is an error, never a
  guess. `pi < 4` is an error — `N(pi) < 4` is `true`.
- **Bounded auto-simplification.** Construction applies cheap, terminating
  canonicalization (fold rationals, `sqrt(2)^2 → 2`, combine like terms). Deep
  simplification is reserved for explicit operations like
  [`expand`](reference/calculus.md#expand).

## Where to start

- [Getting started](getting-started.md) — run the REPL or the web app.
- [Syntax](language/syntax.md) — literals, operators, precedence, statements.
- [Exact numbers and floats](language/numbers.md) — the exact/approximate
  boundary, arbitrary precision, comparisons.
- [Variables, functions, control flow](language/programs.md) — it's a
  language, not just a calculator.
- [Built-in reference](reference/index.md) — every built-in function, with
  examples.

All example output in these pages is real engine output, captured verbatim.
