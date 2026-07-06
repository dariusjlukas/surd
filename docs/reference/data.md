# Vectors and data

The primitives for working with data: indexing, elementwise operations, and
the functions below. A *vector* is a 1×n or n×1 matrix; everything here is
exact, like the rest of the engine.

## Indexing

Indexing is **1-based**, with `[...]` after any expression:

```text
>> v := [3; 1; 4; 1; 5]
>> v[2]
1
>> m := [1, 2; 3, 4]
>> m[2, 1]              # (row, column)
3
>> m[2]                 # one index on a matrix: the whole row
[ 3  4 ]
>> data.samples[3]      # chains through struct fields
```

Out-of-range and non-integer indices are clean errors. There is no indexed
*assignment* — values are immutable; build results with `map`, `vcat`, and
friends.

### Ranges with `:`

Any index position can be a **range** `lo:hi` (inclusive, 1-based) instead of a
single number. Either bound may be omitted — `lo:` runs to the end, `:hi` from
the start, and a bare `:` spans the whole axis. A **scalar index collapses its
axis; a range keeps it**, so you control the shape of the result:

```text
>> m := [1, 2, 3; 4, 5, 6; 7, 8, 9]
>> m[1:2, 2:3]         # rows 1–2, columns 2–3 → a submatrix
[ 2  3 ]
[ 5  6 ]
>> m[2, :]             # row 2, every column → a row
[ 4  5  6 ]
>> m[:, 2]             # every row, column 2 → a column
[ 2 ]
[ 5 ]
[ 8 ]
>> m[1:2, 3]           # the scalar column collapses → a column
[ 3 ]
[ 6 ]
>> v := [10, 20, 30, 40]
>> v[2:3]              # a sub-vector
[ 20  30 ]
```

Bounds are evaluated like any expression, so `v[2:n]` and `v[(k+1):]` work. A
range that runs past the end or reverses (`hi < lo`) is a clean error naming the
axis.

### Strided ranges: `lo:step:hi`

A range can carry a **stride** as a middle field — `lo:step:hi`, with the step
in the middle (MATLAB/Julia order). The step takes two forms:

- a **scalar** `k` keeps every `k`-th position (so `k = 1` is the plain range);
- a **`(take, skip)` pair** keeps `take` consecutive positions, then skips
  `skip`, and repeats — the general "take N, skip M" pattern.

A scalar step `k` is exactly the pair `(1, k - 1)`, and a plain `lo:hi` is
`(1, 0)`. The open forms still apply (`lo:step:`, `:step:hi`, `:step:`), the
stride works on either matrix axis, and the parenthesized scalar `(k)` stays a
scalar step — only the comma makes a pair.

```text
>> v := [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
>> v[1:2:]             # every 2nd element
[ 10  30  50  70  90 ]
>> v[2:2:8]            # every 2nd, from 2 through 8
[ 20  40  60  80 ]
>> v[1:(4, 1):]        # take 4, skip 1, repeat
[ 10  20  30  40  60  70  80  90 ]
>> m := [1, 2, 3, 4; 5, 6, 7, 8; 9, 10, 11, 12]
>> m[1:2:3, 1:2:4]     # every 2nd row and column
[  1   3 ]
[  9  11 ]
```

A step of `0` (or a take count of `0`) is a clean error; the skip count may be
`0` (a contiguous take). Strided indexing applies to signals too, producing a
decimated sub-signal that stays in the signal substrate.

## Elementwise operators: `.*` `./` `.^`

Entrywise versions of `*` `/` `^`. Shapes must match when both sides are
matrices; a scalar broadcasts; two scalars degrade to the plain operation.

```text
>> [1, 2, 3] .* [4, 5, 6]
[ 4  10  18 ]
>> [6, 8, 9] ./ [2, 4, 3]
[ 3  2  3 ]
>> [1, 2, 3] .^ 2
[ 1  4  9 ]
>> 2 .* [1, 2]
[ 2  4 ]
```

Scalar functions (`sin`, `cos`, `tan`, `exp`, `ln`, `sqrt`, `abs`, `conj`,
`re`, `im`) apply entrywise to a matrix argument automatically:

```text
>> sin([0; pi/6])
[   0 ]
[ 1/2 ]
```

## `len` / `size`

```
len(v)      # entries of a vector; rows of a matrix
size(m)     # dimensions, as struct(rows, cols)
```

```text
>> len([3; 1; 4; 1; 5])
5
>> len([1, 2, 3; 4, 5, 6])     # a matrix counts its rows
2
>> size([1, 2, 3; 4, 5, 6])    # fields are sorted, so cols prints before rows
struct(cols = 3, rows = 2)
```

## `slice`

```
slice(v, start, n)
```

`n` consecutive elements from 1-based `start` — vectors and
[signals](signals.md) alike:

```text
>> slice([10, 20, 30, 40], 2, 2)
[ 20  30 ]
```

The `:` range form above is the more general way to say the same thing
(`v[2:3]`); `slice` is kept for when the count, not the endpoint, is what you
have.

## `map`

```
map(f, m)
map(f, m1, ..., mk)
```

Apply a function entrywise, preserving shape. `f` is a function value — a
name, built-in or your own, or a [lambda](../language/programs.md#anonymous-functions-and-closures).
With several same-shape matrices the function receives one entry from each,
so `map(f, a, b)` is the elementwise zip of `a` and `b`:

```text
>> map(x -> x^2 + 1, [1, 2, 3])
[ 2  5  10 ]
>> map(abs, dsp.freqz(h, w))     # magnitude response
>> map((a, b) -> a*b, [1, 2, 3], [4, 5, 6])
[ 4  10  18 ]
```

## `filter`

```
filter(pred, v)
```

The elements of a vector for which `pred` returns `true`, preserving
orientation (row in, row out). The predicate must return an actual boolean
for every element — a symbolic comparison refuses rather than guessing. And
because there is no empty matrix, keeping *no* elements is an error too.

```text
>> filter(x -> x > 2, [1, 2, 3, 4])
[ 3  4 ]
```

## `fold`

```
fold(f, init, v)
```

Left fold: starting from `init`, apply `acc := f(acc, x)` over the elements
of a vector (or the entries of a matrix, row-major). The accumulator may be
any value — a scalar sum, a growing vector via `vcat`, a struct.

```text
>> fold((acc, x) -> acc + x, 0, [1, 2, 3, 4])
10
>> fold((acc, x) -> acc*10 + x, 0, [1, 9, 8, 4])
1984
```

## `dot`

```
dot(a, b)
```

Σ aᵢ·bᵢ for two same-length vectors (bilinear — no conjugation; apply
`conj` yourself for the Hermitian inner product).

```text
>> dot([1, 2, 3], [4, 5, 6])
32
```

## `vcat` / `hcat`

```
vcat(a, b, ...)    # stack vertically; scalars join as 1×1
hcat(a, b, ...)    # stack horizontally
```

```text
>> vcat([1; 2], 9)
[ 1 ]
[ 2 ]
[ 9 ]
>> hcat([1; 2], [3; 4])     # columns side by side
[ 1  3 ]
[ 2  4 ]
```

## `linspace`

```
linspace(a, b, n)
```

n evenly spaced points from a to b inclusive, as a row vector — with an
**exact** rational step, so grids land precisely where you think:

```text
>> linspace(0, pi, 5)
[ 0  1/4*π  1/2*π  3/4*π  π ]
```

## The `data` namespace — preparing data for a model

These helpers sit in front of the [`stats`](stats.md) models. The column
transforms stay **exact** — a z-score is `(x − μ)/σ` with `μ` rational and `σ`
a surd, so the result is an exact surd, not a rounded float.

| function | result |
|----------|--------|
| `data.center(v)` | `v` minus its mean |
| `data.standardize(v)` | z-scores `(vᵢ − μ)/σ` (sample σ), exact surds |
| `data.rescale(v)` | min–max rescaled to `[0, 1]` (numeric data) |
| `data.dummy(v)` | one-hot encode a categorical column |
| `data.groupby(keys, values)` | aggregate `values` by the levels of `keys` |
| `data.dropna(x)` | remove rows with [missing values](#missing-values-na) |
| `data.split(x, frac[, seed])` | seeded random [train/test split](#datasplit) |

`data.dummy(v)` treats each distinct entry (symbol or number) as a level and
returns `struct(levels, indicators)` — an indicator (0/1) column per level.
`data.groupby` returns `struct(levels, count, sum, mean)`, one row per level.

```text
>> N(data.standardize([1; 2; 3; 4; 5]))
[ -1.26491106406735173279955741777 ]
[ ... ]
>> data.groupby([a; b; a; b; a], [1; 2; 3; 4; 5]).mean
[ 3 ]
[ 3 ]
>> data.dummy([red; blue; red]).indicators
[ 1  0 ]
[ 0  1 ]
[ 1  0 ]
```

## Missing values (`NA`)

Real files have holes. A blank CSV cell — or one spelled `NA`, `N/A`, `NaN`,
`null`, or `?` in any letter case, or a JSON `null` — imports as the symbol
`NA`, and the import summary counts what came in
(`value (120×1 matrix) — 3 missing values (NA)`).

surd does **no** NA arithmetic, silent or otherwise. To the algebra `NA` is
an ordinary free symbol; a mean computed "through" one would be well-formed
nonsense, so every `stats` and `data` function refuses NA data outright:

```text
>> stats.mean([1; NA; 3])
error: stats.mean: the data has 1 missing value (NA) — drop the affected
rows first with data.dropna(...)
```

Missingness is handled by you, explicitly, or not at all. `data.dropna`
is the explicit handler — listwise deletion:

- `data.dropna(v)` — a vector minus its `NA` entries.
- `data.dropna(m)` — a matrix minus the rows containing any `NA`.
- `data.dropna(t)` — a table (a struct of equal-length column vectors, as
  CSV import produces) minus every row where *any* column is `NA`, keeping
  the columns aligned.

```text
>> stats.mean(data.dropna([1; NA; 3; NA; 5]))
3
>> t := struct(x = [1; 2; 3; 4], y = [10; NA; 30; 40])
>> data.dropna(t).x
[ 1 ]
[ 3 ]
[ 4 ]
```

Dropping every row is an error, not an empty value.

## `data.split`

```
data.split(x, frac)
data.split(x, frac, seed)
```

A reproducible random train/test split — the first step toward evaluating a
model on data it wasn't fitted to (see [`stats.cv`](stats.md#statscv) for the
k-fold version). `x` is a table (struct of equal-length columns), a matrix
(split by rows), or a vector; `frac` is the **train** fraction, exact in
(0, 1). The result is `struct(train, test)` in `x`'s own shape, so the
formula interface works on either side directly.

Membership is chosen by a seeded shuffle: the engine stays deterministic —
the same call always produces the same split — and passing a different
`seed` (a nonnegative integer, default 0) produces a different one. Each
side keeps the original row order. The train side gets `⌊frac·n + 1/2⌋`
rows; a fraction that would leave either side empty is an error.

```text
>> cars := struct(mpg = [18; 21; 30; 25; 28], weight = [35; 31; 22; 26; 24])
>> s := data.split(cars, 4/5);
>> m := stats.regress(mpg ~ weight, s.train);
>> stats.rmse(s.test.mpg, stats.predict(m, s.test.weight).fit)
233/179
```

## Model formulas: the `~` operator

`response ~ term1 + term2` builds a **model formula** — a piece of data whose
operands name columns of a data struct (a table from
[CSV import](../getting-started.md), say, or a hand-built `struct`). Pass it to
a `stats` model in place of an explicit `(X, y)`:

```text
>> cars := struct(mpg = [...], weight = [...], origin = [us; eu; us; ...])
>> m := stats.regress(mpg ~ weight + origin, cars)
```

The builder looks each term up as a column, adds an intercept, and — for a
**categorical** column (symbol-valued, like `origin`, which is exactly what a
text column in a CSV imports as) — one-hot encodes it with the first level
dropped as the reference. The formula's names stay symbolic, so
a workspace binding of `weight` won't disturb `mpg ~ weight`. The same form
works for `stats.wls`, `stats.ridge`, `stats.lasso`, `stats.logit`, and
`stats.cv`. (Term order follows the canonical ordering of the sum; the
intercept is always supplied, so a constant term is an error.)

### Transforms and interactions

A term doesn't have to be a bare name: it can be **any scalar expression in
column names** — a power, a transform, or a product (what R writes `a:b`).
Each such term is evaluated row by row with the column values substituted
*exactly*, so the design matrix entries stay exact — symbolic, like `ln(35)`,
where no closed numeric form exists.

```text
>> m := stats.regress(y ~ x + x^2, d)          # polynomial terms
>> m := stats.regress(y ~ ln(x) + z + x*z, d)  # a transform and an interaction
>> m := stats.regress(ln(y) ~ x, growth)       # the response transforms too
```

A log-linear fit stays exact end to end: on `y = [2; 4; 8; 16; 32; 64]`
against `x = [1; ...; 6]`, the slope of `ln(y) ~ x` is an exact combination
of logarithms, and `N(exp(slope))` collapses to exactly `2` — the growth
factor, recovered without a decimal in sight.

Columns used inside a transform or interaction must be **numeric** — a
categorical column has no arithmetic, so `y ~ ln(origin)` or `y ~ weight*origin`
is an error pointing at [`data.dummy`](#the-data-namespace-preparing-data-for-a-model)
(encode first, then interact with the indicator columns you mean).
