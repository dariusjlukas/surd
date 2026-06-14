# Variables, functions, control flow

surd is a language, not just a calculator: comparisons produce booleans, and
`if`/`while`/functions give Turing-complete control flow.

## Variables

`name := expr` binds a value in the workspace; the assignment itself
evaluates to the value:

```text
>> a := 5
5
>> a^2 + 1
26
```

Anything a value can be is bindable: numbers, symbols, matrices, structs,
functions, equations. `pi`, `e`, and `I` may be shadowed by user bindings;
`true`/`false` may not.

An unbound name is a free **symbol**, so symbolic algebra needs no
declarations:

```text
>> x + x + 1
1 + 2*x
```

## Functions

Two forms — a one-liner and a block:

```text
>> square(t) := t^2
<function(t)>
>> square(7)
49
```

```text
>> function newton(steps)         # √2 by Newton's method, in exact rationals
..     x := 1
..     i := 0
..     while i < steps do
..         x := (x + 2/x) / 2
..         i := i + 1
..     end
..     x
.. end
>> newton(4)
665857/470832
>> N(newton(4), 20)
1.4142135623746899106
```

- A function's value is its **last statement** — there is no `return`.
- Bodies get their own **local scope**: parameters and any `:=` inside the
  body bind locally; reads fall back to the global workspace.
- **Recursion works** (depth-capped at 1500 frames, as a guard against
  runaways):

```text
>> fact(n) := if n == 0 then 1 else n*fact(n-1) end
>> fact(20)                       # exact arbitrary precision — no overflow
2432902008176640000
```

A user-defined function **shadows a built-in** of the same name.

## Booleans and logic

Comparisons produce real booleans. `and`/`or` short-circuit; negation is the
word `not` (there is no `!`):

```text
>> 1 < 2 and 2 < 3
true
>> not (1 < 2)
false
```

Booleans are opaque to arithmetic — `true + 1` is an error, not a `2`.

## `if`

`if cond then a else b end` — an **expression**, usable anywhere a value is:

```text
>> if 2 < 3 then 10 else 20 end
10
```

The `else` branch is optional; without it, a false condition yields `0`:

```text
>> if 1 > 2 then 10 end
0
```

**The decidable-boolean rule.** Conditions must evaluate to a real
`true`/`false`. A condition that can't be decided is an error, never a guess
— this is the design's core honesty about undecidability:

```text
>> if x then 1 else 2 end
error: expected a true/false condition, got 'x'
```

Wrap symbolic comparisons in [`N(...)`](../reference/numeric.md#n) to decide
them numerically.

## `while`

`while cond do body end`. The loop's value is the last body evaluation (`0`
if the body never ran). Iterations are capped at 10,000,000 so an accidental
infinite loop errors instead of hanging.

```text
>> i := 0
>> while i < 5 do i := i + 1 end
5
```

## Blocks

Statements are separated by newlines or `;`; the value of a block is its
last statement. `if`/`while`/`function` blocks close with `end`, and the
REPL keeps reading lines until every block is closed.

## Resource guards

Untrusted or pathological input turns into clean errors, never a hang or a
crash: a token cap (15,000) and parser nesting cap (512), an
expression-depth cap (8,000) and recursion-frame cap (1,500), a loop
iteration cap (10,000,000), and a ceiling on exact exponents (`2^(10^15)`
stays symbolic instead of building a gigabyte bignum).

## Not in the language (yet)

`return`/`break`/`continue`, closures that capture locals, and `print` are
deliberately deferred — a function's value is its last statement, and since
`if` is an expression, recursion needs no early return. See
[Limits and design notes](../limits.md).
