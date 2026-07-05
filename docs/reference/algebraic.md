# Algebraic numbers

Every rational, every radical, and every polynomial root is a *real
algebraic number* — a root of some integer polynomial. surd represents these
exactly (a squarefree defining polynomial plus an isolating interval), which
makes one more class of question decidable: **equality**.

Certified interval comparison can separate any two *different* constants,
but no amount of precision proves two equal constants equal — `(√2+√3)²` and
`5+2√6` agree to every digit there is. When interval refinement gives up,
the algebraic engine takes over and decides exactly:

```text
>> (sqrt(2)+sqrt(3))^2 == 5 + 2*sqrt(6)
true
>> (sqrt(2)+sqrt(3))^2 < 5 + 2*sqrt(6)
false
>> sqrt(3 + 2*sqrt(2)) == 1 + sqrt(2)
true
```

This covers expressions built from rationals, radicals (any rational
power), field arithmetic, `root(...)` values, and sine/cosine/tangent of
rational multiples of π. It does *not* cover π and e themselves — they are
transcendental, and ties against them still refuse honestly:

```text
>> exp(1) <= e
error: cannot order 'exp(1)' and 'e': ... the values may be equal
```

## `root`

```
root(p, k)
```

The k-th real root (ascending, 1-based) of a univariate polynomial `p` with
rational coefficients — including roots that have **no radical form at
all**:

```text
>> r = root(x^5 - x - 1, 1)      # quintic: unsolvable by radicals
root(-1 - x + x^5, 1)
>> N(r, 20)
1.1673039782614186843
>> r > 1
true
>> r^5 - r - 1 == 0
true
```

The value stays symbolic; `N(...)` refines it to any precision, comparisons
are exact, and arithmetic on it participates in the algebraic engine.
Rational roots collapse to plain numbers when the isolator pins them
(`root(2*x - 4, 1)` is just `2`). Out-of-range indices and rootless
polynomials error loudly:

```text
>> root(x^2 + 1, 1)
error: the polynomial has no real roots
```

## Trig of rational multiples of π

`cos(kπ/n)` beyond the classic surd grid has no radical form (its minimal
polynomial is a cubic or worse), but it is still algebraic — via Chebyshev
polynomials, the engine knows exactly which root it is:

```text
>> 8*cos(pi/7)^3 - 4*cos(pi/7)^2 - 4*cos(pi/7) + 1 == 0
true
>> cos(pi/7) == root(8*x^3 - 4*x^2 - 4*x + 1, 3)
true
>> tan(pi/5)^2 == 5 - 2*sqrt(5)
true
```

## Limits

The engine refuses (falling back to the usual "may be equal") past a
defining-polynomial degree of 64 or coefficients beyond 4,096 bits — caps
chosen so every decision stays interactive. Sums and products of algebraic
numbers multiply degrees, so deeply nested combinations can hit the cap;
plain radicals, `root(...)` values, and trig values are comfortably inside
it.
