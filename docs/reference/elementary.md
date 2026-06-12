# Elementary functions

All of these stay **symbolic** until you cross to floats with
[`N(...)`](numeric.md#n); exact folds happen at construction
(`sqrt(4) → 2`, `exp(0) → 1`, `sin(pi/6) → 1/2`, `exp(I*pi) → -1`).

## `sqrt`

```
sqrt(x)
```

The square root, represented internally as `x^(1/2)` — so `sqrt(2)` and
`2^(1/2)` are the same object.

```text
>> sqrt(4)
2
>> sqrt(1/4)
1/2
>> sqrt(2)*sqrt(2)
2
>> sqrt(8)               # square factors extract
2*sqrt(2)
>> sqrt(8/9)
2/3*sqrt(2)
>> sqrt(2)*sqrt(3)       # provably nonnegative radicands combine
sqrt(6)
>> sqrt(-12)             # square roots of negatives are imaginary
2*sqrt(3)*I
>> N(sqrt(2), 10)
1.414213562
```

Exact perfect powers fold, square factors extract, and radicals over
*provably nonnegative* radicands combine (`√a·√b = √(a·b)` is only sound
when signs are known — `sqrt(x)*sqrt(y)` stays put). One deliberate
non-simplification:

- `sqrt(x^2)` stays `sqrt(x^2)` — collapsing it to `x` is the
  `sqrt(x^2) = |x|` branch-cut trap.

## `exp`

```
exp(x)
```

The exponential function `e^x`.

```text
>> exp(0)
1
>> N(exp(1), 10)
2.718281828
>> exp(I*pi)             # complex arguments unfold by Euler's formula — exactly
-1
>> exp(I*x)
cos(x) + sin(x)*I
>> diff(exp(x), x)
exp(x)
```

A complex argument unfolds symbolically into the canonical `re + im·I` form:
`exp(a + b*I)` is `exp(a)*(cos(b) + sin(b)*I)`. Combined with the exact trig
table below, `exp(I*pi)` is exactly `-1` — no floats involved.

## `ln`

```
ln(x)
```

The natural logarithm.

```text
>> ln(1)
0
>> N(ln(2), 10)
0.6931471806
>> N(ln(I), 20)          # ln(i) = iπ/2
1.5707963267948966192*I
>> diff(ln(x), x)
x^(-1)
```

## `sin` / `cos` / `tan`

```
sin(x)    cos(x)    tan(x)
```

The trigonometric functions, in radians.

```text
>> sin(pi/6)
1/2
>> cos(pi/4)
1/2*sqrt(2)
>> tan(pi/3)
sqrt(3)
>> cos(pi/12)            # the 15° grid
1/4*sqrt(2) + 1/4*sqrt(6)
>> sin(pi/8)             # the 22.5° grid, as a nested radical
1/2*sqrt(2 - sqrt(2))
>> cos(pi/5)             # the pentagonal grid: the golden ratio, halved
1/4 + 1/4*sqrt(5)
>> diff(sin(x), x)
cos(x)
>> diff(tan(x), x)
cos(x)^(-2)
```

At a rational multiple of π whose value has a surd form — denominators 1, 2,
3, 4, 5, 6, 8, 10, and 12, extended over the whole circle by symmetry — the
result folds to that exact value at construction. Everything else stays
symbolic: `cos(pi/7)` stays `cos(1/7*π)` (no surd form exists — its minimal
polynomial is a cubic), `sin(1)` stays `sin(1)`, and `tan(pi/2)` stays put
rather than inventing a value at a pole. `N(...)` evaluates any of them to
full requested precision.
Complex arguments evaluate numerically through Euler's formula and the
hyperbolic identities — see
[Complex numbers](../language/complex.md#complex-transcendentals).

## `abs`

```
abs(x)
```

Absolute value — the modulus, for complex `x`.

```text
>> abs(-2/3)
2/3
>> abs(3 + 4*I)
5
>> abs(x)                # symbolic: stays put (sign of x is unknown)
abs(x)
```
