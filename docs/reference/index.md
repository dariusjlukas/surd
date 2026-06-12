# Built-in reference

Every built-in function, grouped by area. A user-defined function with the
same name shadows the built-in; a call to a name that is neither stays a
symbolic, unevaluated application.

## Elementary functions

| Function | Description |
| --- | --- |
| [`sqrt(x)`](elementary.md#sqrt) | Square root (`x^(1/2)`) |
| [`exp(x)`](elementary.md#exp) | Exponential function |
| [`ln(x)`](elementary.md#ln) | Natural logarithm |
| [`sin(x)`](elementary.md#sin-cos-tan) | Sine |
| [`cos(x)`](elementary.md#sin-cos-tan) | Cosine |
| [`tan(x)`](elementary.md#sin-cos-tan) | Tangent |
| [`abs(x)`](elementary.md#abs) | Absolute value (modulus for complex `x`) |

## Calculus and symbolic manipulation

| Function | Description |
| --- | --- |
| [`diff(expr, x)` / `D(expr, x)`](calculus.md#diff-d) | Derivative of `expr` with respect to `x` |
| [`subs(expr, x, val)`](calculus.md#subs) | Substitute `val` for `x` in `expr` |
| [`expand(expr)`](calculus.md#expand) | Expand products and integer powers |

## Numeric evaluation

| Function | Description |
| --- | --- |
| [`N(x, digits?)`](numeric.md#n) | Numeric value of `x` to `digits` significant digits (default 30) |
| [`precision(digits?)`](numeric.md#precision) | Query or set the default precision |

## Complex numbers

| Function | Description |
| --- | --- |
| [`conj(z)`](complex.md#conj) | Complex conjugate |
| [`re(z)` / `real(z)`](complex.md#re-real) | Real part |
| [`im(z)` / `imag(z)`](complex.md#im-imag) | Imaginary part |
| [`abs(z)`](complex.md#abs) | Modulus |

## Linear algebra

| Function | Description |
| --- | --- |
| [`det(M)`](linear-algebra.md#det) | Determinant |
| [`inv(M)`](linear-algebra.md#inv) | Inverse |
| [`transpose(M)` / `T(M)`](linear-algebra.md#transpose-t) | Transpose |
| [`solve(A, b)`](linear-algebra.md#solve) | Solve the linear system A·x = b |
| [`rref(M)`](linear-algebra.md#rref) | Reduced row echelon form |
| [`rank(M)`](linear-algebra.md#rank) | Rank |
| [`nullspace(M)` / `kernel(M)`](linear-algebra.md#nullspace-kernel) | Nullspace basis, as columns |
| [`lu(M)`](linear-algebra.md#lu) | LU decomposition → `struct(L, U, P)` |
| [`qr(M)`](linear-algebra.md#qr) | QR decomposition → `struct(Q, R)` |
| [`eye(n)` / `identity(n)`](linear-algebra.md#eye-identity) | n×n identity matrix |
| [`charpoly(M, var?)`](linear-algebra.md#charpoly) | Characteristic polynomial |
| [`eigenvalues(M)` / `eig(M)`](linear-algebra.md#eigenvalues-eig) | Eigenvalues, exact |
| [`eigenvectors(M)`](linear-algebra.md#eigenvectors) | Eigenvectors, paired with `eigenvalues(M)` |

## Plotting

| Function | Description |
| --- | --- |
| [`plot(f1, ..., fk, x, a, b)`](plotting.md#plot) | One or more curves in `x` over `[a, b]` |
| [`plot3d(f, x, a, b, y, c, d)`](plotting.md#plot3d) | Surface z = f(x, y) over `[a, b]` × `[c, d]` |

## Structs

| Function | Description |
| --- | --- |
| [`struct(name = value, ...)`](structs.md) | Build a struct from named fields |

## Constants

| Name | Description |
| --- | --- |
| [`pi`](constants.md#pi) | The circle constant π |
| [`e`](constants.md#e) | Euler's number |
| [`I`](constants.md#i) | The imaginary unit |
| [`true` / `false`](constants.md#true-false) | Boolean literals |
