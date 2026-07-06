# Syntax

## Programs and statements

A program is a sequence of statements separated by **newlines or `;`**. The
value of a program (or any block) is the value of its **last statement**.

Newlines are significant only at bracket depth 0 — inside `(...)` or `[...]`
a newline is line continuation:

```text
>> (1
..  + 2)
3
```

`#` starts a comment that runs to the end of the line:

```text
>> 1 + 1   # this is ignored
2
```

## Suppressing output with `;`

End a statement with `;` to **suppress its echoed result** — the
MATLAB/Julia convention. The value is still computed and any binding still
made; only the display is silenced. This keeps a large matrix or vector from
flooding the screen when all you wanted was to store it:

```text
>> big := [1, 2, 3; 4, 5, 6];   # bound, but nothing printed
>> big[2, 3]                    # …and still usable
6
```

A trailing `;` is unambiguous: it can only be a statement terminator at the
end of a program (a `;` *inside* `[...]` is a matrix row separator, with the
`]` still to come). A trailing comment or newline doesn't change this, so
`big := …;  # note` is suppressed too. Errors are never suppressed.

In the notebook a suppressed cell collapses to a faint, clickable shape hint
(e.g. `; 6-vector`) that expands the full output on demand; the value is also
always listed in the workspace panel.

## Identifiers and reserved words

Identifiers start with a letter or `_` and continue with letters, digits, or
`_`. These words are reserved and carry grammar:

```
if  then  else  elseif  end  while  do  for  in  function
and  or  not  true  false
```

`true` and `false` are literals and can never be assigned to. The constants
`pi`, `e`, and `I` are *ordinary names* that user bindings shadow — so `e`
and `i` stay free for loop counters and the like (see
[Constants](../reference/constants.md)).

An unbound identifier is simply a **symbol** — `x` evaluates to the symbolic
variable `x`, no declaration needed.

## Numeric literals

- Integers: `42`, arbitrary precision (`2^100` is the full 31-digit integer).
- Decimals: `1.5`, `.5` — read as **exact rationals** (`1.5` is `3/2`, `0.1`
  is `1/10`), never floats. Floats are opt-in via
  [`N(...)`](../reference/numeric.md#n).
- Scientific notation is **rejected loudly** rather than misread: `3e5` is an
  error suggesting `3*10^5` (otherwise it would silently parse as `3*e5`, a
  free symbol).

## String literals

`"..."` is an inert piece of data — it labels things (plot titles and axis
labels, see [plotting](../reference/plotting.md#titles-and-axis-labels)).
Arithmetic on a string is an error, never a coercion; `==`/`!=` compare
strings decidably. Strings are built with
[`str(a, b, ...)`](../reference/data.md#str), which renders each argument to
its canonical printed form and concatenates — `str("r = ", 3/7)` is
`"r = 3/7"` — and measured with [`len`](../reference/data.md#len-size).

Backslashes stay **literal** so LaTeX needs no doubling — `"$\omega$"` holds
`$\omega$`. The only escapes are `\"` (a quote) and `\\` (a backslash).

## Operators and precedence

From loosest to tightest binding:

| Precedence | Operators | Notes |
| --- | --- | --- |
| 1 (loosest) | `=` | builds an [equation](#equations-vs-assignment-vs-equality), not a truth test |
| 2 | `or` | short-circuits |
| 3 | `and` | short-circuits |
| 4 | `not` | logical negation (there is no `!`) |
| 5 | `<` `>` `<=` `>=` `==` `!=` | comparisons — see [Exact numbers and floats](numbers.md#comparisons-are-decidable-or-they-are-errors) |
| 6 | `+` `-` | |
| 7 | `*` `/` | |
| 8 | unary `-` | |
| 9 | `^` | **right-associative**: `2^3^2` is `2^(3^2)` |
| 10 (tightest) | `.` | struct field access — binds tighter than `^`, so `s.a^2` is `(s.a)^2` |

Assignment `:=` and function definition are statement forms, not operators.
A lambda `params -> body` sits alongside `=` at the loosest level: the body
extends as far right as possible, so `x -> x + 1` is `x -> (x + 1)` and
`x -> y -> x + y` nests right-associatively.

## Implicit multiplication

A number or `)` followed by `(` or an identifier multiplies:

```text
>> 2x + 2(x+1)
2 + 4*x
>> x^2y          # exponents bind first: (x^2)·y
y*x^2
```

So `2x`, `2pi`, `2sin(x)`, `2(x+1)`, `(x+1)(x-1)`, and `(x+1)y` all work.
Deliberately *not* implicit:

- `ident(…)` stays a function call (`f(x)` never means `f*x`),
- adjacent identifiers (`x y`) stay an error — they carry block grammar
  (`x then …`),
- `3e5` is rejected (see above), and `1.5.5` is an error.

## Matrix literals

`[...]` with `,` between entries and `;` between rows:

```text
>> [1, 2; 3, 4]
[ 1  2 ]
[ 3  4 ]
>> [1; 2; 3]        # a column vector is a 3×1 matrix
[ 1 ]
[ 2 ]
[ 3 ]
```

Entries can be any expression, symbols included. See
[Matrices](matrices.md).

## Equations vs. assignment vs. equality

Three distinct spellings, three distinct meanings:

| Spelling | Meaning |
| --- | --- |
| `x := 3` | **Assignment** — bind `x` in the workspace |
| `y = x + 1` | **Equation** — a piece of data; both sides evaluate, nothing is tested |
| `x == 3` | **Decidable equality test** — produces `true`/`false` |

```text
>> a := 5
5
>> y = a + 1        # an equation; the right side evaluates to 6
y = 6
>> a == 5
true
```

`=` never tests truth: equality of reals is undecidable in general
(Richardson's theorem), so an equation is something you manipulate, not a
boolean. For what `==` does (and does not) decide, see
[Exact numbers and floats](numbers.md#comparisons-are-decidable-or-they-are-errors).

## Function calls

`name(arg, ...)`. If `name` is bound to a user-defined function it is called;
otherwise a [built-in](../reference/index.md) is dispatched. A name that is
neither stays a **symbolic, unevaluated application**:

```text
>> unknownfn(3)
unknownfn(3)
```

A few built-ins ([`diff`](../reference/calculus.md#diff-d),
[`subs`](../reference/calculus.md#subs),
[`plot`](../reference/plotting.md), [`struct`](../reference/structs.md))
treat certain arguments as *names* taken from the syntax rather than values —
their pages spell this out.
