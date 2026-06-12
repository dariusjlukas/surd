# Linear algebra

All matrix operations are **exact** — over ℚ, over quadratic extensions
ℚ(√d) where eigenproblems demand it, and over full symbolic expressions.
Matrix entries can be anything: `det([p, q; r, s])` is `-q*r + p*s`. For
matrix literals and operators (`+`, `*`, `^`, …) see
[Matrices](../language/matrices.md).

All functions on this page take a matrix and error with `<name> expects a
matrix argument` otherwise.

## `det`

```
det(M)
```

The determinant. Fraction-free Bareiss elimination for numeric matrices (so
integer intermediates don't blow up), division-free cofactor expansion for
symbolic ones.

```text
>> det([1,2,3; 4,5,6; 7,8,10])
-3
>> det([p, q; r, s])
-q*r + p*s
>> det(eye(3))
1
```

## `inv`

```
inv(M)
```

The matrix inverse, by exact Gauss-Jordan elimination. A singular matrix is
an error. `A^(-1)` is equivalent.

```text
>> inv([1, 2; 3, 4])
[  -2     1 ]
[ 3/2  -1/2 ]
>> [1,2;3,4] * inv([1,2;3,4])      # exactly the identity
[ 1  0 ]
[ 0  1 ]
>> inv([1/2, 1/3; 1/4, 1/5])       # a float tool gives 11.9999…; this is exact
[  12  -20 ]
[ -15   30 ]
```

## `transpose` / `T`

```
transpose(M)
T(M)
```

The transpose. `T` is an alias.

```text
>> T([1, 2; 3, 4])
[ 1  3 ]
[ 2  4 ]
>> transpose([1, 2, 3])     # row vector → column vector
[ 1 ]
[ 2 ]
[ 3 ]
```

## `solve`

```
solve(A, b)
```

Solve the linear system A·x = b exactly (`b` is a column matrix).

```text
>> solve([2,1,-1; -3,-1,2; -2,1,2], [8; -11; -3])
[  2 ]
[  3 ]
[ -1 ]
```

An **underdetermined** system returns the *general* solution as a struct:
one particular solution plus a nullspace basis. Every solution is
`particular` plus a linear combination of the `nullspace` columns:

```text
>> solve([1,1;2,2], [3;6])
struct(nullspace = [ -1 ]
[  1 ], particular = [ 3 ]
[ 0 ])
```

An inconsistent system is an error, never a least-squares guess.

## `rref`

```
rref(M)
```

The reduced row echelon form, by exact Gauss-Jordan.

```text
>> rref([1,2,3; 4,5,6])
[ 1  0  -1 ]
[ 0  1   2 ]
```

## `rank`

```
rank(M)
```

The rank — exact, so it is the true rank, not a numerical-tolerance estimate.

```text
>> rank([1,2; 2,4])
1
>> rank(eye(4))
4
```

## `nullspace` / `kernel`

```
nullspace(M)
kernel(M)
```

A basis of the nullspace {x : A·x = 0}, returned as the columns of a matrix.
`kernel` is an alias.

```text
>> nullspace([1,2,3; 4,5,6])
[  1 ]
[ -2 ]
[  1 ]
>> nullspace(eye(2))
error: the null space is trivial: the matrix has full column rank (2), so A·x = 0 only for x = 0
```

A trivial nullspace is reported in those words rather than returned as a
zero column.

## `lu`

```
lu(M)
```

The LU decomposition with row pivoting (Doolittle): returns
`struct(L, U, P)` with **P·A = L·U**. Exact; singular matrices included.

```text
>> f := lu([4,3; 6,3])
>> f.L
[   1  0 ]
[ 3/2  1 ]
>> f.U
[ 4     3 ]
[ 0  -3/2 ]
>> f.P
[ 1  0 ]
[ 0  1 ]
```

## `qr`

```
qr(M)
```

The QR decomposition by exact Gram-Schmidt: returns `struct(Q, R)` with
A = Q·R. Projections run on the unnormalized orthogonal columns, so radicals
only enter at normalization — and Qᵀ·Q folds to the identity *exactly*,
rather than to within 1e-16:

```text
>> g := qr([1,1; 1,0])
>> T(g.Q) * g.Q              # orthonormal exactly, surd norms and all
[ 1  0 ]
[ 0  1 ]
```

## `eye` / `identity`

```
eye(n)
identity(n)
```

The n×n identity matrix. `eye` is an alias.

```text
>> eye(3)
[ 1  0  0 ]
[ 0  1  0 ]
[ 0  0  1 ]
```

## `charpoly`

```
charpoly(M)
charpoly(M, var)
```

The characteristic polynomial det(A − λI), computed symbolically. The
variable defaults to `lambda`; pass a second argument to choose another.

```text
>> charpoly([2,1; 1,2])
3 + lambda^2 - 4*lambda
>> charpoly([2,1; 1,2], t)
3 + t^2 - 4*t
```

## `eigenvalues` / `eig`

```
eigenvalues(M)
eig(M)
```

The eigenvalues, as a column. **Exact wherever a radical form exists**:
rational-root peeling, the quadratic formula (complex pairs included),
Cardano's formula for cubics, and biquadratic quartics with their nested
radicals.

```text
>> eigenvalues([1,1; 1,0])         # the golden ratio, exactly
[ 1/2 + 1/2*sqrt(5) ]
[ 1/2 - 1/2*sqrt(5) ]
>> eigenvalues([1,-1; 1,1])        # complex pairs, returned not refused
[ 1 + I ]
[ 1 - I ]
>> eigenvalues([0,0,2; 1,0,0; 0,1,0])    # Cardano: exact cube roots
[                              2^(1/3) ]
[ -1/2*2^(1/3) + 1/2*2^(1/3)*sqrt(3)*I ]
[ -1/2*2^(1/3) - 1/2*2^(1/3)*sqrt(3)*I ]
>> N(eigenvalues([1,1; 1,0]), 30)  # …or numeric, to any precision
[   1.61803398874989484820458683437 ]
[ -0.618033988749894848204586834366 ]
```

What provably has *no* radical form is **reported, never approximated**:
three real cubic roots need complex cube roots (casus irreducibilis — the
trigonometric form isn't implemented), general quartics await the Ferrari
reduction, and degree ≥ 5 has no radical formula at all (Abel–Ruffini).

## `eigenvectors`

```
eigenvectors(M)
```

The eigenvectors, as columns that **pair with the entries of
`eigenvalues(M)`, in order** — so A·V = V·diag(λ).

```text
>> eigenvectors([1,1; 1,0])
[ 1/2 + 1/2*sqrt(5)  1/2 - 1/2*sqrt(5) ]
[                 1                  1 ]
```

Gauss-Jordan runs in the field the eigenvalue actually lives in — ℚ, ℚ(√d),
or its complex extension — where the zero test is decidable, so
golden-ratio eigenvectors come out symbolically and `inv(V)·A·V`
diagonalizes complex rotations *exactly*.

A **defective** matrix (fewer independent eigenvectors than the
multiplicity) is reported in those words, never padded with zero columns.
Eigenvalues that need cubic or nested radicals are still exact via
`eigenvalues`, but `eigenvectors` doesn't follow into those fields yet.
