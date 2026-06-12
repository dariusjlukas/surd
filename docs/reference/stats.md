# Statistics — the `stats` namespace

Exact statistics. These live in the
[`stats` namespace](../language/modules.md): `stats.mean(v)`, not `mean(v)`.

Every estimator runs in exact arithmetic: the mean of rationals is a
rational, a variance is a rational, a standard deviation is an exact surd —
floats appear only when you ask with [`N(...)`](numeric.md#n). `var`, `std`,
`cov`, and `cor` are the **sample** estimators (n−1 denominator). Symbolic
entries flow through everything that doesn't need ordering; `median` needs
numeric data (ordering symbolic reals is undecidable).

A *vector* argument is a 1×n or n×1 matrix.

## `stats.mean`

```
stats.mean(v)
```

```text
>> stats.mean([1; 2; 3; 4])
5/2
>> stats.mean([a; b])
1/2*a + 1/2*b
```

## `stats.median`

```
stats.median(v)
```

The middle value by exact ordering — `1/3` beats `0.3333` here; the mean of
the two middle values for even length.

```text
>> stats.median([1/2; 1/3; 1/4])
1/3
>> stats.median([1; 2; 3; 4])
5/2
```

## `stats.var` / `stats.std`

```
stats.var(v)
stats.std(v)
```

Sample variance and standard deviation (n−1 denominator).

```text
>> stats.var([1; 2; 3; 4])
5/3
>> stats.std([1; 2; 3; 4])
sqrt(5/3)
>> N(stats.std([1; 2; 3; 4]), 10)
1.290994449
```

## `stats.cov` / `stats.cor`

```
stats.cov(a, b)
stats.cor(a, b)
```

Sample covariance, and the Pearson correlation. Because the variances are
exact, perfectly linear data correlates to **exactly** ±1 — not 0.9999…:

```text
>> stats.cov([1; 2; 3], [2; 4; 6])
2
>> stats.cor([1; 2; 3], [2; 4; 6])
1
>> stats.cor([1; 2; 3], [5; 3; 1])
-1
```

Zero-variance data is an error (the correlation is undefined).

## `stats.linfit`

```
stats.linfit(x, y)
```

Exact least-squares line `y = intercept + slope·x`, as a struct.

```text
>> stats.linfit([0; 1; 2], [1; 2; 4])
struct(intercept = 5/6, slope = 3/2)
>> fit := stats.linfit([1; 2; 3; 4], [3; 5; 7; 9])
>> fit.slope
2
```

All x values equal is an error (the line is vertical).

## `stats.quantile`

```
stats.quantile(v, q)
```

The q-th quantile (0 ≤ q ≤ 1), by exact linear interpolation between order
statistics (the R type-7 / NumPy default — but with an exact weight, since
(n−1)·q is a rational here, not a float).

```text
>> stats.quantile([0; 10], 1/4)
5/2
>> stats.quantile([1; 2; 3; 4], 1/2)     # == stats.median
5/2
```

## `stats.rmse` / `stats.r2`

```
stats.rmse(a, b)
stats.r2(y, yhat)
```

Root mean squared error (an exact surd), and the coefficient of
determination R² = 1 − SSres/SStot. A perfect fit is *exactly* 1 — model
quality is never hidden inside float noise:

```text
>> stats.rmse([1, 2], [2, 4])
sqrt(5/2)
>> stats.r2([1, 2, 3, 4], [1, 2, 3, 5])
4/5
```

## `stats.polyfit` / `stats.polyval`

```
stats.polyfit(x, y, deg)
stats.polyval(c, t)
```

Exact least-squares polynomial fit: Vandermonde + normal equations solved
by exact elimination — Vandermonde *conditioning* is a float problem, and
there are no floats here. Coefficients are a column vector, constant term
first. `polyval` evaluates a coefficient vector at a scalar, a symbol (you
get the polynomial as an expression), or elementwise over a vector:

```text
>> c := stats.polyfit([0, 1, 2, 3], [0, 1, 4, 9], 2)
[ 0 ]
[ 0 ]
[ 1 ]
>> stats.polyval(c, t)
t^2
>> stats.r2([0, 1, 4, 9], stats.polyval(c, [0, 1, 2, 3]))
1
```

Too few distinct x values for the degree is an error.

## `stats.lsq`

```
stats.lsq(A, b)
```

General exact least squares: the β minimizing ‖Aβ − b‖₂, via the normal
equations. No automatic intercept — `hcat` a ones column for one. Linearly
dependent regressors (a non-unique minimizer) are an error:

```text
>> stats.lsq([1, 0; 0, 1; 1, 1], [1; 1; 2])
[ 1 ]
[ 1 ]
```
