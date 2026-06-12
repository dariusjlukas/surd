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

## Elementwise operators: `.*` `./` `.^`

Entrywise versions of `*` `/` `^`. Shapes must match when both sides are
matrices; a scalar broadcasts; two scalars degrade to the plain operation.

```text
>> [1, 2, 3] .* [4, 5, 6]
[ 4  10  18 ]
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
size(m)     # struct(rows, cols)
```

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
