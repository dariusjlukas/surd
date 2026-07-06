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

End the line with `;` to bind **without echoing** the value — handy for a
large matrix or vector you only mean to store (see
[Suppressing output](syntax.md#suppressing-output-with)):

```text
>> m := [1, 2, 3; 4, 5, 6];   # bound silently
>> m[1, 2]
2
```

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

Chain cases with `elseif` — one `end` closes the whole chain:

```text
>> grade(x) := if x >= 90 then 4 elseif x >= 80 then 3 else 0 end
>> grade(85)
3
```

The `else` branch is optional **in statement position** — running an `if`
purely for its side effect (a conditional assignment) is fine. But using the
*value* of an `if` that has no `else` is an error when the condition is
false: there is no value to use, and inventing one (an earlier version
yielded a silent `0`) is exactly the kind of guess surd refuses to make.

```text
>> if 1 > 2 then x := 10 end      # fine: nothing consumes the value
>> y := if 1 > 2 then 10 end
error: this 'if' has no 'else', so it has no value when the condition is false — add an else branch
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

## `for`

`for x in lo:hi do body end` iterates an **inclusive range** of exact values
(`lo:step:hi` with a step in the middle, MATLAB/Julia order — steps may be
negative or rational). Endpoints and step must be exact numbers, so the
stopping comparison is always decidable; there is no float drift, and a
rational step lands exactly on the endpoint:

```text
>> s := 0
>> for x in 0:1/4:1 do s := s + x end
>> s
5/2
```

`for x in m do ... end` over a matrix iterates a vector's elements, or an
m×n matrix's rows (matching `m[i]`). The loop variable stays bound after the
loop, like a `while` counter, and the same 10,000,000-iteration cap applies.

## Anonymous functions and closures

`x -> expr` is a function value with no name; multiple parameters take
parentheses, `(a, b) -> a + b`. Lambdas go anywhere a value goes — most
usefully straight into [`map`/`filter`/`fold`](../reference/data.md#map):

```text
>> map(x -> x^2, [1, 2, 3])
[ 1  4  9 ]
>> filter(x -> x > 2, [1, 2, 3, 4])
[ 3  4 ]
>> fold((acc, x) -> acc + x, 0, [1, 2, 3, 4])
10
```

Functions created inside another function **capture the locals they mention,
by value**, at the moment they are created — the classic closure factory
works:

```text
>> make(k) := (x -> k*x)
>> double := make(2)
>> double(21)
42
```

Capture is a snapshot: later changes to the original variable don't reach
the closure (values in surd are immutable, and closures are values too).
Free names at the *top level* are not captured — they stay late-bound
against the live workspace, which is what lets `fact(n) := ... fact(n-1) ...`
refer to itself before it exists. Named `function`/`:=` definitions written
inside another function capture the same way, and a local function can call
itself by name.

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

`return`/`break`/`continue` and `print` are deliberately deferred — a
function's value is its last statement, and since `if` is an expression,
recursion needs no early return; `for` plus `filter`/`fold` cover most of
what `break` is for. See [Limits and design notes](../limits.md).
