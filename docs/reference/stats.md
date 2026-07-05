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

## `stats.sum`

```
stats.sum(v)
```

The exact sum of every element. Like `mean`, it flows symbolic entries
through, so it stays exact whatever the data:

```text
>> stats.sum([1; 2; 3; 4])
10
>> stats.sum([1/2; 1/3; 1/4])
13/12
>> stats.sum([a; b; 2])
2 + a + b
```

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

## `stats.min` / `stats.max`

```
stats.min(v)
stats.max(v)
```

The smallest and largest element, by the same exact ordering as `median` —
`1/3` beats `0.34` here. The matching entry comes back verbatim, so its exact
form is preserved. Symbolic entries can't be ordered, so they error.

```text
>> stats.min([1/2; 1/3; 1/4])
1/4
>> stats.max([1/3; 0.34; 3/8])
3/8
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

## `stats.covmat` / `stats.cormat`

```
stats.covmat(M)
stats.cormat(M)
```

The full sample covariance and Pearson correlation matrices of a data matrix
`M` — **columns are variables, rows are observations** (so an n×k matrix gives a
k×k result). Both are exact: every entry is the same surd `stats.cov` / `stats.cor`
would give for that pair of columns, so `cormat` has an exactly-1 diagonal and is
exactly symmetric.

```text
>> stats.cormat([1, 2; 2, 4; 3, 6])
[ 1  1 ]
[ 1  1 ]
>> stats.covmat([1, 2; 2, 4; 3, 6])
[ 1  2 ]
[ 2  4 ]
```

Wrap in `N(...)` for decimals, e.g. `N(stats.cormat(M))`. To *see* the pairwise
relationships, draw the matrix with [`pairs`](plotting.md#scatterplot-matrices).
Needs at least 2 observations; a constant column makes the correlation matrix
undefined (an error).

## `stats.linfit`

```
stats.linfit(x, y)
```

Exact least-squares line `y = intercept + slope·x`, as a struct with fields
`intercept`, `slope`, and `predict` — the fitted line as a function (see
[Fitted models](#fitted-models)).

```text
>> fit := stats.linfit([1; 2; 3; 4], [3; 5; 7; 9])
struct(intercept = 1, predict = <function(x)>, slope = 2)
>> fit.slope
2
>> fit.predict(10)        # the line at x = 10
21
```

All x values equal is an error (the line is vertical).

### Fitted models

A single-predictor fit — `stats.linfit` and `stats.nlfit` — returns a `predict`
field holding the fitted curve as an ordinary **function** of the predictor.
That makes the model directly usable two ways:

```text
>> m := stats.linfit(xs, ys)
>> m.predict(2.5)                                  # predict at a new point
>> plot(scatter(xs, ys), m.predict, x, 0, 10)      # overlay on the data
```

`m.predict` plots like any curve (bare, or applied as `m.predict(x)`). For
`nlfit` the fitted coefficients are f64, so `predict` carries their exact
rational form — wrap a point prediction in `N(…)` for a decimal. The
multi-predictor models (`regress`, `wls`, `ridge`, `logit`) instead predict
through [`stats.predict`](#statspredict), which takes a design matrix.

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
column. You can also fit from a data table with a
[formula](data.md#model-formulas-the-operator): `stats.regress(y ~ x1 + x2,
data)` (the same form works for `wls`, `ridge`, `lasso`, and `logit`).

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
| `confint` | 95% confidence interval per coefficient, `[lower, upper]` rows |
| `cov` | full coefficient covariance matrix σ̂²·(XᵀX)⁻¹ |
| `fitted`, `residuals` | ŷ and y − ŷ |
| `rss`, `sigma2` | residual sum of squares and its variance estimate σ̂² = RSS/df |
| `r2`, `adjr2` | R² and adjusted R² |
| `fstat`, `fpvalue` | overall-significance F and its p-value |
| `loglik`, `aic`, `bic` | Gaussian log-likelihood and information criteria |
| `leverage`, `studentized`, `cooks` | hat-matrix diagonal, internally studentized residuals, Cook's distance |
| `intercept` | whether `regress` added an intercept column (so `predict` can match it) |
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

## `stats.wls`

```
stats.wls(X, y, weights)
```

Weighted least squares — exactly `stats.regress`, but minimizing
Σ wᵢ·(yᵢ − xᵢβ)² for per-observation `weights` (inverse-variance weights for
heteroskedastic data, replication counts, …). The result is a regression model
with all the same fields, computed from the weighted normal equations
β̂ = (XᵀWX)⁻¹XᵀWy, still exact. Weights must be positive; with all weights
equal it is ordinary least squares.

```text
>> stats.wls([1; 2; 3], [1; 2; 2], [1; 1; 2]).coefficients
[ 8/11 ]
[ 5/11 ]
```

## `stats.ridge`

```
stats.ridge(X, y, lambda)
```

Ridge regression — the L2-penalized estimator β̂ = (XᵀX + λP)⁻¹Xᵀy, which
trades a little bias for a large drop in variance and is the standard remedy
for multicollinearity. Exact in `lambda` (rational λ ⇒ rational coefficients).
The intercept is never penalized. Because ridge is biased, classical standard
errors don't apply, so the result reports point estimates and fit only:
`coefficients`, `fitted`, `residuals`, `rss`, `r2`, `lambda`, and the effective
degrees of freedom `edf` = trace(X(XᵀX+λP)⁻¹Xᵀ). λ = 0 recovers OLS; larger λ
shrinks the slopes and lowers `edf` toward 1. Standardize predictors of very
different scales first.

```text
>> r := stats.ridge([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 1)
>> r.coefficients
[ 26/11 ]
[  6/11 ]
>> r.edf
21/11
```

## `stats.lasso`

```
stats.lasso(X, y, lambda)
```

Lasso regression — the L1-penalized estimator minimizing
½n⁻¹‖y − Xβ‖₂² + λ‖β‖₁ over the slope coefficients. Where [ridge](#statsridge)
only shrinks, lasso's L1 penalty drives coefficients **exactly to zero**, so the
fit doubles as variable selection. There's no closed form: the solver is cyclic
coordinate descent with soft-thresholding, so — like [logit](#statslogit) — the
coefficients come back as floats. The intercept is added automatically (unless
`X` already has a constant column) and is never penalized. Unlike OLS, lasso
fits happily with more predictors than observations. Standardize predictors of
very different scales first, since one shared `lambda` penalizes every
coefficient equally.

The result reports point estimates and fit only (the L1 estimator is biased, so
classical standard errors don't apply): `coefficients`, `fitted`, `residuals`,
`rss`, `r2`, `lambda`, `intercept`, the number of active (nonzero) coefficients
`df`, the coordinate-descent `iterations` and whether they `converged`, and the
counts `n`, `k`. `lambda = 0` recovers OLS; as `lambda` grows, coefficients drop
out one by one and `df` falls. `df` counts *every* nonzero coefficient,
including the (unpenalized) intercept — so an intercept-only fit has `df = 1`.
(R's glmnet reports only the penalized coefficients; its `df` is one less.)

```text
>> l := stats.lasso([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 1/5)
>> l.coefficients
[ 2.50000000038866 ]
[ 0.499999999894001 ]
>> stats.lasso([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 2).coefficients   # slope zeroed out
[ 4 ]
[ 0 ]
```

A negative or non-numeric `lambda` is an error.

## `stats.cv`

```
stats.cv(X, y, k)
stats.cv(response ~ terms, data, k)
stats.cv(..., k, struct(model = ..., lambda = ..., seed = ...))
```

k-fold cross-validation — the **out-of-sample** counterpart to the in-sample
`r2`/`aic` a fitted model reports, and the honest way to compare models or
choose a penalty. The rows are dealt into `k` near-equal folds by a seeded
shuffle (reproducible, like [`data.split`](data.md#datasplit) — same call,
same folds, forever); each fold is then predicted by a model fitted on the
other `k − 1`, and the squared prediction errors pool into the CV
mean-squared error. The design matrix is built **once** from the full data,
so a categorical column one-hot encodes identically in every fold.

The options struct selects what gets refit per fold:

| option | meaning |
|--------|---------|
| `model` | `regress` (default), `ridge`, or `lasso` |
| `lambda` | the penalty for ridge/lasso — a scalar, or a **vector of candidates** |
| `seed` | fold-assignment seed (nonnegative integer, default 0) |

For `regress` and `ridge` the per-fold refits run in **exact arithmetic**:
`mse` is an exact rational and `rmse` an exact surd — a model that predicts
its held-out folds perfectly scores *exactly* 0, not approximately. `lasso`
refits are floats, like `stats.lasso` itself.

With a scalar (or no) `lambda` the result is
`struct(model, k, n, seed, mse, rmse, foldmse)` — `foldmse` holds the
per-fold MSEs, `mse` the pooled MSE over all n held-out predictions. With a
`lambda` **vector**, every candidate is scored against the *same* folds and
the result is `struct(model, k, n, seed, lambda, mse, rmse, best)`, with
`mse`/`rmse` aligned to `lambda` and `best` the candidate with the smallest
CV error — the standard way to pick a ridge/lasso penalty:

```text
>> x := [1; 2; 3; 4; 5; 6; 7; 8]
>> y := [2.1; 3.9; 6.2; 7.8; 10.1; 12.2; 13.8; 16.1]
>> c := stats.cv(x, y, 4)
>> N(c.rmse)
0.179017251396614708699479205606
>> r := stats.cv(x, y, 4, struct(model = ridge, lambda = [0; 1/10; 1; 10]))
>> r.best
0
>> stats.cv([1; 2; 3; 4; 5; 6], [3; 5; 7; 9; 11; 13], 3).rmse   # exact fit, exact zero
0
```

The formula form works against a data struct exactly as in
[`stats.regress`](#statsregress): `stats.cv(mpg ~ weight + origin, cars, 5)`.
A fold whose training rows become linearly dependent (a categorical level
concentrated in one fold, say) is reported with its fold number — try fewer
folds or another `seed`. Data carrying [missing values](data.md#missing-values-na)
is refused; `data.dropna` first.

## `stats.logit`

```
stats.logit(X, y)
```

Logistic regression by iteratively reweighted least squares — for a binary
response `y` (each value 0 or 1), modeling P(y = 1) = 1/(1 + e^{−xβ}). IRLS
iterates a weighted least squares step, so the estimates are floats; inference
is Wald (standard errors from the information matrix (XᵀWX)⁻¹ at convergence,
two-sided p-values from the **normal** CDF, not t).

| field | meaning |
|-------|---------|
| `coefficients` | log-odds coefficients β̂ |
| `se`, `zstat`, `pvalue` | Wald standard error, z-statistic, two-sided p-value |
| `fitted`, `residuals` | fitted probabilities μ and response residuals y − μ |
| `deviance`, `nulldeviance` | model and intercept-only deviance |
| `pseudor2` | McFadden's pseudo-R² = 1 − deviance/nulldeviance |
| `iterations`, `converged` | IRLS iterations and whether it converged |

```text
>> m := stats.logit([1; 2; 3; 4; 5; 6; 7; 8], [0; 0; 0; 1; 0; 1; 1; 1])
>> m.coefficients
[ -5.77032035229123 ]
[  1.28229341162027 ]
>> N(m.pvalue)
[ 0.152781530183485 ]
[ 0.136139154333700 ]
```

Non-binary responses, more parameters than observations, and perfectly
separated data (a singular information matrix) are errors.

## `stats.predict`

```
stats.predict(model, Xnew)
stats.predict(model, Xnew, level)
```

Predict the response at new regressor rows from a `stats.regress` model.
`Xnew` carries the same raw predictors you gave `regress` (a length-m vector
for a single-predictor model, otherwise an m×k matrix); the intercept is
reattached automatically. The optional `level` is the confidence level
(default `0.95`). Returns a struct:

| field | meaning |
|-------|---------|
| `fit` | point predictions ŷ = Xnew·β̂ |
| `se` | standard error of the **mean** response per row |
| `ci` | confidence interval for the mean response, `[lower, upper]` rows |
| `pi` | prediction interval for a **new observation** (wider — adds σ̂²) |

```text
>> m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> p := stats.predict(m, [6; 7])
>> p.fit
[ 29/5 ]
[ 32/5 ]
>> N(p.ci[1])
[ 2.81460073898108864334454228784  8.78539926101891135665545771216 ]
>> N(p.pi[1])
[ 1.67507814177002750418315696512  9.92492185822997249581684303488 ]
```

## `stats.robustse`

```
stats.robustse(model, X)
stats.robustse(model, X, type)
```

Heteroskedasticity-consistent (White sandwich) standard errors, for inference
that doesn't assume constant error variance. Pass the same `X` you gave
`regress` (the sandwich's *meat* needs the design matrix). `type` selects the
small-sample correction: `0`–`3` for HC0–HC3, default **HC1**. Everything stays
exact — the robust covariance is a rational matrix, the standard errors exact
surds. Returns `se`, `tstat`, `pvalue` recomputed robustly.

```text
>> m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> stats.robustse(m, [1; 2; 3; 4; 5]).se
[ 1/5*sqrt(229/10) ]
[  1/5*sqrt(43/30) ]
```

## `stats.anova`

```
stats.anova(reduced, full)
```

Compare two **nested** OLS models with an F-test — does the fuller model
explain significantly more variance? Order-independent: the model with fewer
residual degrees of freedom is treated as the fuller one.
F = [(RSSᵣ − RSS_f)/Δdf] / [RSS_f/df_f]. Returns `fstat`, `pvalue` (carrying an
`fcdf`), and the two degrees of freedom `df1`, `df2`.

```text
>> red  := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> full := stats.regress([1, 1; 2, 4; 3, 9; 4, 16; 5, 25], [2; 4; 5; 4; 5])
>> stats.anova(red, full).fstat
20/11
```

## Regression assumption tests

Each takes a `stats.regress` model and returns a struct. The test statistics
are exact rationals (built from the residuals); the p-values stay symbolic.

| function | tests for | statistic ~ |
|----------|-----------|-------------|
| `stats.dwtest(model)` | first-order autocorrelation (Durbin–Watson, ≈2 means none) | — (`statistic` only) |
| `stats.bptest(model)` | heteroskedasticity (Breusch–Pagan / Koenker, vs. fitted) | χ²(1) |
| `stats.jbtest(model)` | non-normal residuals (Jarque–Bera, skew + kurtosis) | χ²(2) |

```text
>> m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> stats.dwtest(m).statistic
121/60
>> stats.jbtest(m).statistic
3283/5760
>> N(stats.jbtest(m).pvalue)
0.752027310235741287995200868876
```

## `stats.ttest`

```
stats.ttest(x, mu)          # one-sample, against a hypothesized mean
stats.ttest(x, y)           # two-sample (Welch)
stats.ttest(x, y, paired)   # paired
```

Student's t-tests. The two-sample form is **Welch's** unequal-variance test
(the safe default — it never assumes the equal variances it is in no position
to check), with the Welch–Satterthwaite degrees of freedom as an **exact
rational**; the paired form is a one-sample test on the pairwise differences.
The statistic and standard error are exact surds; the p-value and the 95%
confidence interval carry symbolic `tcdf`/`tinv` — `N(...)` for decimals.

Returns `struct(statistic, df, se, pvalue, confint, estimate, n, kind)` —
`estimate` is the sample mean (one-sample/paired) or the difference of means
(two-sample), and `confint` is the 95% interval around it. The one-sample
form also records `mu`.

```text
>> t := stats.ttest([1; 2; 3; 4], [2; 4; 6; 8; 10])
>> t.df                       # Welch–Satterthwaite, exactly
2523/457
>> t.statistic
-7*(29/3)^(-1/2)
>> N(t.pvalue)
0.0691335931923924129724721915329
>> stats.ttest([2; 4; 7], [1; 3; 5], paired).statistic
4
```

Zero-variance data (and paired samples of different lengths) are errors.

## `stats.chisqtest`

```
stats.chisqtest(table)      # an r×c contingency table of counts
stats.chisqtest(x, y)       # two same-length categorical columns
```

Pearson's chi-square test of independence. The two-column form cross-tabulates
the levels of `x` and `y` (in first-appearance order, reported as `rows` and
`cols`); the statistic Σ(O−E)²/E and the matrix of `expected` counts are exact
rationals, with the p-value a symbolic `chisqcdf` at (r−1)(c−1) degrees of
freedom. No continuity correction is applied (matching R's
`chisq.test(..., correct = FALSE)`).

```text
>> c := stats.chisqtest([10, 20; 30, 40])
>> c.statistic
50/63
>> c.expected
[ 12  18 ]
[ 28  42 ]
>> N(c.pvalue)
0.372998483613487117046496052219
```

Counts must be nonnegative numbers, the table at least 2×2, and no row or
column may sum to zero.

## `stats.cortest`

```
stats.cortest(x, y)
```

Is the Pearson correlation zero? The `estimate` is the exact surd
[`stats.cor`](#statscov-statscor) computes, the statistic is
t = r·√((n−2)/(1−r²)) at n−2 degrees of freedom, and the p-value stays
symbolic. Perfectly correlated data (|r| = 1) is an error — there is nothing
left to test.

```text
>> r := stats.cortest([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])
>> r.estimate^2
3/5
>> N(r.statistic)
2.12132034355964257320253308631
>> N(r.pvalue)
0.124027062657554625225778493721
```

## `stats.nlfit`

```
stats.nlfit(model, [params], x, y)
stats.nlfit(model, [params], x, y, initial)
```

Nonlinear least squares — fit an arbitrary model `y ≈ f(x; θ)` by
Levenberg–Marquardt. `model` is any expression in an independent variable and
the parameters; `[params]` lists the parameter names to fit (held symbolic, so
a workspace binding won't collapse them); the remaining free symbol is the
independent variable, matched to `x`. `initial` is one starting guess per
parameter (default `1`); good guesses matter for nonlinear fits.

The distinctive part: the Jacobian ∂f/∂θⱼ is built by **exact symbolic
differentiation** — the true derivative, not a finite-difference approximation
— so steps stay accurate where difference-based fitters degrade. The result
exposes that Jacobian in symbolic form. The fit iterates, so the *estimates*
are floats (reported to f64 precision); the asymptotic standard errors come
from the linearized covariance σ̂²·(JᵀJ)⁻¹ at the solution.

| field | meaning |
|-------|---------|
| `coefficients` | fitted parameters, in the order of `[params]` |
| `se`, `tstat`, `pvalue` | asymptotic standard error, t-statistic, two-sided p-value |
| `residuals`, `rss`, `sigma2` | residuals, residual sum of squares, σ̂² = RSS/(n−p) |
| `jacobian` | the **exact symbolic** derivatives ∂f/∂θⱼ used by the fit |
| `predict` | the fitted model as a function of the predictor (see [Fitted models](#fitted-models)) |
| `iterations`, `converged` | iterations taken and whether the step/cost tolerance was met |

```text
>> f := stats.nlfit(a*exp(b*x), [a, b], [0; 1; 2; 3; 4],
                    [2; 3.29744; 5.43656; 8.96338; 14.7781], [1, 1])
>> f.coefficients
[ 2.00000040860740 ]
[ 0.49999977885331 ]
>> f.jacobian
[     exp(b*x) ]
[ a*x*exp(b*x) ]
>> f.converged
true
```

Constants resolve from the workspace, so `c := 2; stats.nlfit(a*x^c, [a], …)`
fits `a` with the exponent fixed at 2. A parameter that never appears in the
model, more than one leftover free variable, or a divergent start are errors.

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
