# Matrices

Exact linear algebra over ℚ — and beyond: entries are full symbolic
expressions, so exact rationals are just the all-numeric case. Every
operation is exact. No rounding, ever.

## Literals

`,` separates entries, `;` separates rows:

```text
>> A := [1, 2; 3, 4]
[ 1  2 ]
[ 3  4 ]
>> [1; 2; 3]                 # column vector = n×1 matrix
[ 1 ]
[ 2 ]
[ 3 ]
>> [p, q; r, s]              # symbolic entries are fine
[ p  q ]
[ r  s ]
```

## Operators

| Expression | Meaning |
| --- | --- |
| `A + B`, `A - B` | entrywise sum/difference (two matrices only — a matrix and a scalar don't add) |
| `A * B` | matrix product |
| `2 * A`, `A * 2` | scalar product |
| `A / B` | `A · B⁻¹` |
| `A / 2`, `2 / A` | scalar division / `2 · A⁻¹` |
| `A ^ n` | matrix power, **integer** exponents only; `A^(-1)` is the inverse |

```text
>> A^2
[  7  10 ]
[ 15  22 ]
>> A * inv(A)            # exactly the identity, not "approximately"
[ 1  0 ]
[ 0  1 ]
```

## Why exactness matters here

```text
>> inv([1/2, 1/3; 1/4, 1/5])    # a float tool gives you 11.9999…; this is exact
[  12  -20 ]
[ -15   30 ]
```

Determinants use fraction-free Bareiss elimination for numeric matrices (so
integer intermediates don't blow up) and division-free cofactor expansion for
symbolic ones. Inverse, `solve`, `rref`, and `rank` share one exact
Gauss-Jordan routine.

## Symbolic entries and calculus

```text
>> det([p, q; r, s])
-q*r + p*s
>> diff([x^2, sin(x); x, 1], x)    # differentiation distributes entrywise
[ 2*x  cos(x) ]
[   1       0 ]
>> N([1/3, 1/7; 2/3, 1], 5)        # N maps entrywise too
[ 0.33333  0.14286 ]
[ 0.66667        1 ]
```

## Solving systems

```text
>> solve([2,1,-1; -3,-1,2; -2,1,2], [8; -11; -3])
[  2 ]
[  3 ]
[ -1 ]
```

An underdetermined system returns the *general* solution as a struct — one
particular solution plus a nullspace basis; every solution is `particular`
plus a combination of the basis columns:

```text
>> solve([1,1;2,2], [3;6])
struct(nullspace = [ -1 ]
[  1 ], particular = [ 3 ]
[ 0 ])
```

## Eigenproblems

Exact wherever a radical form exists — irrational and complex eigenvalues are
kept symbolic, never approximated:

```text
>> eigenvalues([1,1;1,0])          # the golden ratio, exactly
[ 1/2 + 1/2*sqrt(5) ]
[ 1/2 - 1/2*sqrt(5) ]
>> eigenvectors([1,1;1,0])         # columns pair with eigenvalues(A), in order
[ 1/2 + 1/2*sqrt(5)  1/2 - 1/2*sqrt(5) ]
[                 1                  1 ]
>> charpoly([2,1;1,2])             # det(A - λI), symbolically
3 + lambda^2 - 4*lambda
```

What provably has no radical form is *reported*, never approximated — see
the [linear-algebra reference](../reference/linear-algebra.md#eigenvalues-eig)
for exactly which cases are covered.

## Decompositions

`lu(A)` returns `struct(L, U, P)` with P·A = L·U; `qr(A)` returns
`struct(Q, R)` by exact Gram-Schmidt, where Qᵀ·Q folds to the identity
*exactly* rather than to within 1e-16:

```text
>> g := qr([1,1;1,0])
>> T(g.Q) * g.Q                    # orthonormal exactly, surd norms and all
[ 1  0 ]
[ 0  1 ]
```

## Reference

Full signatures and examples for every matrix built-in — `det`, `inv`,
`transpose`/`T`, `solve`, `rref`, `rank`, `nullspace`/`kernel`, `lu`, `qr`,
`eye`/`identity`, `charpoly`, `eigenvalues`/`eig`, `eigenvectors` — are in
the [linear-algebra reference](../reference/linear-algebra.md).

## Indexing and elementwise operations

`m[i, j]` reads an element (1-based), `v[i]` a vector element, `m[i]` a whole
row. Any position can also be a **range** `lo:hi` (with `lo:`, `:hi`, and `:`
as the open and whole-axis forms) — a scalar collapses that axis, a range keeps
it, so `m[2, :]` is a row, `m[:, 2]` a column, and `m[1:2, 1:3]` a submatrix.
`.*` `./` `.^` operate entrywise; scalar functions map over entries
automatically. See [Vectors and data](../reference/data.md).
