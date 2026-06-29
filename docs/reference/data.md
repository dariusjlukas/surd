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
```

Apply a function entrywise, preserving shape. `f` is a function value or a
function's name — built-in or your own:

```text
>> f(x) := x^2 + 1
>> map(f, [1, 2, 3])
[ 2  5  10 ]
>> map(abs, dsp.freqz(h, w))     # magnitude response
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
**categorical** column (symbol-valued, like `origin`) — one-hot encodes it with
the first level dropped as the reference. The formula's names stay symbolic, so
a workspace binding of `weight` won't disturb `mpg ~ weight`. The same form
works for `stats.wls`, `stats.ridge`, and `stats.logit`. (Term order follows the
canonical ordering of the sum, and interactions like `a:b` are not yet
supported.)
