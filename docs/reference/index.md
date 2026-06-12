# Built-in reference

Every built-in function, grouped by area. A user-defined function with the
same name shadows the built-in; a call to a name that is neither stays a
symbolic, unevaluated application. Domain toolkits live behind
[namespaces](../language/modules.md) (`dsp.dft(v)`), so they don't claim
bare names.

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

## Vectors and data

Indexing `v[i]` / `m[i, j]` (1-based) and the elementwise operators
`.*` `./` `.^` are part of the [syntax](data.md); scalar functions apply
entrywise to matrices automatically.

| Function | Description |
| --- | --- |
| [`len(v)`](data.md#len-size) | Entries of a vector (rows of a matrix) |
| [`size(m)`](data.md#len-size) | Dimensions, as `struct(rows, cols)` |
| [`map(f, m)`](data.md#map) | Apply a function entrywise |
| [`dot(a, b)`](data.md#dot) | Σ aᵢ·bᵢ |
| [`vcat(a, ...)` / `hcat(a, ...)`](data.md#vcat-hcat) | Stack vertically / horizontally |
| [`linspace(a, b, n)`](data.md#linspace) | n evenly spaced points, exact step |
| [`slice(v, start, n)`](data.md#slice) | n elements from 1-based start (vectors and signals) |

## Signals (certified bulk data)

Packed data at scale, every sample carrying a certified error enclosure —
see [Signals](signals.md).

| Function | Description |
| --- | --- |
| [`signal(v, digits?)`](signals.md#two-substrates) | Pack a vector (f64, or arbitrary precision) |
| [`mid(s)`](signals.md#the-boundary-is-explicit) | Midpoints, back to exact-land |
| [`bound(s, i?)`](signals.md#the-boundary-is-explicit) | Certified max │true − mid│ |
| [`dsp.fft(s)` / `dsp.ifft(f)`](signals.md#operations) | Certified radix-2 FFT |
| [`dsp.pad(s, n)`](signals.md#operations) | Zero-pad to a power of two |
| [`dsp.peak(s)` / `dsp.rms(s)`](signals.md#operations) | Certified reductions |

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

## DSP (the `dsp` namespace)

| Function | Description |
| --- | --- |
| [`dsp.dft(v)`](dsp.md#dspdft) | Discrete Fourier transform, exact |
| [`dsp.idft(v)`](dsp.md#dspidft) | Inverse DFT (exactly inverts `dsp.dft`) |
| [`dsp.dftmatrix(n)`](dsp.md#dspdftmatrix) | The n×n Fourier matrix |
| [`dsp.conv(a, b)`](dsp.md#dspconv) | Linear convolution |
| [`dsp.circconv(a, b)`](dsp.md#dspcircconv) | Circular convolution |
| [`dsp.freqz(h, w)`](dsp.md#dspfreqz) | FIR frequency response at frequencies `w` |
| [`dsp.firlow(n, wc)`](dsp.md#dspfirlow) | Windowed-sinc lowpass prototype |
| [`dsp.hann(n)` / `dsp.hamming(n)` / `dsp.blackman(n)`](dsp.md#dsphann-dsphamming-dspblackman) | Cosine-sum windows, exact |
| [`dsp.quantize(v, bits)`](dsp.md#dspquantize) | Snap to a fixed-point grid (exact rationals) |

## Statistics (the `stats` namespace)

| Function | Description |
| --- | --- |
| [`stats.mean(v)`](stats.md#statsmean) | Mean, exact |
| [`stats.median(v)`](stats.md#statsmedian) | Median by exact ordering |
| [`stats.var(v)`](stats.md#statsvar-statsstd) | Sample variance |
| [`stats.std(v)`](stats.md#statsvar-statsstd) | Sample standard deviation (an exact surd) |
| [`stats.cov(a, b)`](stats.md#statscov-statscor) | Sample covariance |
| [`stats.cor(a, b)`](stats.md#statscov-statscor) | Pearson correlation (exactly ±1 for linear data) |
| [`stats.linfit(x, y)`](stats.md#statslinfit) | Exact least-squares line → `struct(intercept, slope)` |
| [`stats.quantile(v, q)`](stats.md#statsquantile) | q-th quantile, exact interpolation |
| [`stats.rmse(a, b)`](stats.md#statsrmse-statsr2) | Root mean squared error (exact surd) |
| [`stats.r2(y, yhat)`](stats.md#statsrmse-statsr2) | Coefficient of determination, exact |
| [`stats.polyfit(x, y, deg)`](stats.md#statspolyfit-statspolyval) | Exact least-squares polynomial |
| [`stats.polyval(c, t)`](stats.md#statspolyfit-statspolyval) | Evaluate a coefficient vector (scalar, symbol, or elementwise) |
| [`stats.lsq(A, b)`](stats.md#statslsq) | General exact least squares |

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
