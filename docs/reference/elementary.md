# Elementary functions

All of these stay **symbolic** until you cross to floats with
[`N(...)`](numeric.md#n); cheap exact folds happen at construction
(`sqrt(4) → 2`, `exp(0) → 1`, `sin(0) → 0`).

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
>> sqrt(-4)              # square roots of negatives are imaginary
2*I
>> N(sqrt(2), 10)
1.414213562
```

Exact perfect powers fold; everything else stays symbolic. Two deliberate
non-simplifications:

- `sqrt(x^2)` stays `sqrt(x^2)` — collapsing it to `x` is the
  `sqrt(x^2) = |x|` branch-cut trap.
- `sqrt(8)` stays `sqrt(8)` — square-factor extraction (`2*sqrt(2)`) is not
  implemented yet.

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
>> N(exp(I*pi))          # complex arguments work, via Euler's formula
-1
>> diff(exp(x), x)
exp(x)
```

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
>> sin(0)
0
>> N(sin(pi/6), 40)      # = 1/2, recovered numerically
0.5
>> N(tan(pi/4), 10)
1
>> diff(sin(x), x)
cos(x)
>> diff(tan(x), x)
cos(x)^(-2)
```

Special values like `sin(pi/6)` are *not* folded symbolically (that's
deferred deep simplification); they evaluate to full requested precision
under `N`. Complex arguments evaluate numerically through Euler's formula
and the hyperbolic identities — see
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
