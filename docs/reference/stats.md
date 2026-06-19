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

## `stats.regress`

```
stats.regress(X, y)
```

Ordinary least squares **with the full inferential apparatus**, returned as a
fitted-model [struct](../language/structs.md). `X` is an n×k design matrix (or
a length-n vector for a single predictor); `y` is the n responses. An
intercept column is added automatically unless `X` already holds a constant
column.

What makes this exact where it can be: the coefficient covariance
σ̂²·(XᵀX)⁻¹ is a *rational matrix*, so standard errors and t-statistics are
exact surds — no float standard errors, ever. Only the quantities that need a
transcendental distribution (the p-values, `aic`, `bic`, `loglik`) come back
as symbolic expressions carrying a `tcdf`/`fcdf`/`ln`; take them to decimals
with [`N(...)`](numeric.md#n).

Fields of the result:

| field | meaning |
|-------|---------|
| `coefficients` | β̂, intercept first (column vector) |
| `se`, `tstat`, `pvalue` | per-coefficient standard error, t-statistic, two-sided p-value |
| `cov` | full coefficient covariance matrix σ̂²·(XᵀX)⁻¹ |
| `fitted`, `residuals` | ŷ and y − ŷ |
| `rss`, `sigma2` | residual sum of squares and its variance estimate σ̂² = RSS/df |
| `r2`, `adjr2` | R² and adjusted R² |
| `fstat`, `fpvalue` | overall-significance F and its p-value |
| `loglik`, `aic`, `bic` | Gaussian log-likelihood and information criteria |
| `leverage`, `studentized`, `cooks` | hat-matrix diagonal, internally studentized residuals, Cook's distance |
| `n`, `k`, `df`, `dfmodel` | observations, parameters, residual and model degrees of freedom |

```text
>> m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> m.coefficients
[ 11/5 ]
[  3/5 ]
>> m.r2
3/5
>> m.se
[ 1/5*sqrt(22) ]
[  1/5*sqrt(2) ]
>> N(m.pvalue)
[ 0.100743456085420036080062667873 ]
[ 0.124027062657554625225778493721 ]
>> m.cooks[1]
3/2
>> N(m.aic)
16.519539456645725222944251431
```

A perfect fit (zero residual variance), constant responses, or rank-deficient
regressors are errors — there is no honest inference to report in those cases.

## Probability distributions

The normal, Student-t, χ², and F distributions, each with a CDF, a PDF, and an
inverse CDF (quantile). Like every transcendental in surd, a distribution
value is a symbol until [`N(...)`](numeric.md#n) evaluates it — arbitrary
precision, computed through the regularized incomplete gamma/beta functions.

| distribution | CDF | PDF | quantile | parameters |
|--------------|-----|-----|----------|------------|
| Normal | `stats.normcdf(x[, μ, σ])` | `stats.normpdf(...)` | `stats.norminv(p[, μ, σ])` | mean, std (default 0, 1) |
| Student-t | `stats.tcdf(t, ν)` | `stats.tpdf(t, ν)` | `stats.tinv(p, ν)` | df ν |
| Chi-square | `stats.chisqcdf(x, k)` | `stats.chisqpdf(x, k)` | `stats.chisqinv(p, k)` | df k |
| F | `stats.fcdf(x, d1, d2)` | `stats.fpdf(x, d1, d2)` | `stats.finv(p, d1, d2)` | df d1, d2 |

```text
>> N(stats.normcdf(1.96))
0.975002104851779565863415730959
>> N(stats.norminv(0.975))
1.95996398454005423552459443052
>> N(stats.tcdf(2, 5))
0.949030260585070821877319447079
>> N(stats.chisqinv(0.95, 1))
3.84145882069412595836137543736
```

## Special functions

The mathematical machinery the distributions are built on is also exposed
*globally* (not in the `stats` namespace), alongside `sin`/`exp`: `erf(x)`,
`erfc(x)`, `gamma(x)`, `lgamma(x)` (log-gamma, x > 0), and `beta(a, b)`. They
fold to exact values where one exists — `gamma` of a positive integer is a
factorial, of a half-integer an exact multiple of √π, and `erf(0)` is `0` —
and otherwise evaluate under `N(...)`.

```text
>> gamma(5)
24
>> gamma(1/2)
sqrt(π)
>> N(erf(1))
0.842700792949714869341220635083
```
