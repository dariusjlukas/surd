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
