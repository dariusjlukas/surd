# Exact numbers and floats

## Exact by default

Every number you type is exact:

```text
>> 1/3 + 1/6
1/2
>> 1.5                # a decimal literal is an exact rational
3/2
>> 0.1 + 0.2          # no 0.30000000000000004 here
3/10
>> 2^100              # integers are arbitrary precision
1267650600228229401496703205376
```

Irrational values stay symbolic instead of rounding:

```text
>> sqrt(2)*sqrt(2)
2
>> 2*pi + pi
3*π
>> sqrt(8)            # no hidden float — and no square-factor extraction (yet)
sqrt(8)
```

Construction applies *bounded* canonicalization: rationals fold, like terms
combine, `sqrt(2)^2` becomes `2`. Anything deeper (expanding products,
trigonometric identities) is an explicit operation —
[`expand`](../reference/calculus.md#expand).

## Crossing to floats: `N`

[`N(x)`](../reference/numeric.md#n) evaluates numerically, to 30 significant
digits by default; `N(x, d)` asks for `d` digits, and
[`precision(d)`](../reference/numeric.md#precision) changes the default.
Constants and transcendentals are computed to whatever precision you request:

```text
>> N(1/3)
0.333333333333333333333333333333
>> N(pi, 50)
3.1415926535897932384626433832795028841971693993751
>> N(sin(pi/6), 40)        # = 1/2, recovered numerically
0.5
```

The numeric engine reaches right through symbolic structure — compare a
recursively computed Fibonacci ratio with the closed form:

```text
>> fib(n) := if n < 2 then n else fib(n-1) + fib(n-2) end
>> N(fib(30)/fib(29), 40)  # Fibonacci ratio: only ~11 digits of φ
1.618033988748203621343798191078293911856
>> N((1 + sqrt(5))/2, 40)  # closed form: exact to all 40
1.61803398874989484820458683436563811772
```

`N` also maps entrywise over [matrices](matrices.md).

### Float contagion

A float operand makes the numeric part of `+`/`*`/`^` float; symbols stay
symbolic. `N(pi) + 1` is one float; `N(2) + x` keeps `x` symbolic. (Known
gap: float *coefficients* don't merge like terms — `N(2.5)*x + x` stays two
terms.)

## Comparisons are decidable, or they are errors

Ordering (`<` `>` `<=` `>=`) works on numbers and on **constant** symbolic
expressions — the latter decided by *certified interval refinement*: both
sides are enclosed in intervals computed with directed rounding, and the
precision doubles until the intervals provably separate. The answer is
therefore never a float guess:

```text
>> 1/2 < 2/3
true
>> pi < 4
true
>> sqrt(2) + sqrt(3) > pi          # 3.1462… vs 3.1415…
true
>> exp(pi) > pi^e                  # the classic
true
```

A comparison that can't be decided is an **error, never a guess** — free
symbols have no fixed value, and constants whose enclosures never separate
(they may be equal) refuse at ~2,400 digits:

```text
>> x < 4
error: cannot order 'x' and '4'; both must be constant real values (a free
symbol has no fixed value — try subs(...) or N(...))
>> (sqrt(2)+sqrt(3))^2 < 5 + 2*sqrt(6)    # the two sides are exactly equal
error: cannot order '(sqrt(2) + sqrt(3))^2' and '5 + 2*sqrt(6)': they agree
to at least 2466 significant digits — the values may be equal
```

One exception cuts through even free symbols: when the *difference*
canonicalizes to an exact number, the answer holds for every real value —
`x + 1 > x` is `true`, `x < x` is `false`.

Floats *do* compare, and exactly: a binary float is the rational m·2^k, so a
float-vs-exact comparison is decided losslessly on that value — never by
rounding the other side:

```text
>> N(2) == 2
true
>> N(1/3) == 1/3       # the float is genuinely not 1/3 — saying so is the point
false
```

For non-numbers, `==`/`!=` test *decidable structural* equality after
canonicalization — **not** mathematical equality (which is undecidable):

```text
>> (x-1)*(x+1) == x^2 - 1
false
>> expand((x-1)*(x+1)) == x^2 - 1
true
```

Struct equality is field-order-independent (fields are kept sorted).
