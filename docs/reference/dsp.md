# DSP — the `dsp` namespace

Exact digital signal processing. These live in the
[`dsp` namespace](../language/modules.md): `dsp.dft(v)`, not `dft(v)`.

Everything follows the engine's exactness contract. DFT twiddle factors are
exact: for transform sizes whose angles have surd forms (1, 2, 3, 4, 5, 6,
8, 10, 12, 16, 20, 24), the DFT of a rational vector is a vector over
ℚ(i, √2, √3, √5, …) with no rounding anywhere — and `dsp.idft(dsp.dft(v))`
is *identically* `v`, not `v` up to epsilon. Other sizes stay exact but
symbolic: entries hold `cos`/`sin` of rational multiples of π, which
[`N(...)`](numeric.md#n) evaluates to any precision on demand.

A *vector* argument is a 1×n or n×1 matrix; results keep the orientation of
the (first) input. Transforms and convolutions are capped at 4,000,000
pairwise products per call (a DFT of length n costs n²) — past that, a clean
error rather than an effective hang.

## `dsp.dft`

```
dsp.dft(v)
```

The discrete Fourier transform, `X[k] = Σⱼ v[j]·e^(−2πi·kj/n)`
(unnormalized). Direct O(n²) summation — exactness is the point here, not
asymptotics.

```text
>> dsp.dft([1; 2; 3; 4])
[       10 ]
[ -2 + 2*I ]
[       -2 ]
[ -2 - 2*I ]
>> dsp.dft([1; 1; 0; 0; 0; 0; 0; 0])      # size 8: exact √2 surds, not floats
[                               2 ]
[ 1 + 1/2*sqrt(2) - 1/2*sqrt(2)*I ]
[                           1 - I ]
[ 1 - 1/2*sqrt(2) - 1/2*sqrt(2)*I ]
[                               0 ]
[ 1 - 1/2*sqrt(2) + 1/2*sqrt(2)*I ]
[                           1 + I ]
[ 1 + 1/2*sqrt(2) + 1/2*sqrt(2)*I ]
>> dsp.dft([a; b])                        # symbolic entries pass through
[ a + b ]
[ a - b ]
>> dsp.dft([1; 0; 0; 0; 1])               # size 5: golden-ratio surds
[                                              2 ]
[ 3/4 + 1/4*sqrt(5) + 1/4*sqrt(10 + 2*sqrt(5))*I ]
[ 3/4 - 1/4*sqrt(5) + 1/4*sqrt(10 - 2*sqrt(5))*I ]
[ 3/4 - 1/4*sqrt(5) - 1/4*sqrt(10 - 2*sqrt(5))*I ]
[ 3/4 + 1/4*sqrt(5) - 1/4*sqrt(10 + 2*sqrt(5))*I ]
```

## `dsp.idft`

```
dsp.idft(v)
```

The inverse transform, with the `+i` kernel and the 1/n factor. Inverts
`dsp.dft` exactly:

```text
>> dsp.idft(dsp.dft([1/3; -2; 5/7]))
[ 1/3 ]
[  -2 ]
[ 5/7 ]
```

## `dsp.dftmatrix`

```
dsp.dftmatrix(n)
```

The n×n Fourier matrix `F[j][k] = e^(−2πi·jk/n)`, so
`dsp.dftmatrix(n) * v` equals `dsp.dft(v)`.

```text
>> dsp.dftmatrix(4)
[ 1   1   1   1 ]
[ 1  -I  -1   I ]
[ 1  -1   1  -1 ]
[ 1   I  -1  -I ]
```

## `dsp.conv`

```
dsp.conv(a, b)
```

Linear convolution, length m+n−1 — equivalently, the coefficient product of
two polynomials, or FIR filtering of a finite signal.

```text
>> dsp.conv([1, 2], [1, 3])      # (1 + 2z)(1 + 3z) = 1 + 5z + 6z²
[ 1  5  6 ]
```

## `dsp.circconv`

```
dsp.circconv(a, b)
```

Circular (periodic) convolution of two equal-length vectors:
`c[i] = Σⱼ a[j]·b[(i−j) mod n]`.

```text
>> dsp.circconv([1, 2, 3], [0, 1, 0])     # convolving with a shifted impulse rotates
[ 3  1  2 ]
```

## `dsp.freqz`

Also accepts IIR forms: `dsp.freqz(b, a, w)` for a rational response
B/A, and `dsp.freqz(f, w)` with a filter struct (the SOS cascade).

```
dsp.freqz(h, w)
```

The frequency response H(ω) = Σₖ h[k]·e^(−iωk) of FIR taps `h`, at each ω
in the vector `w` (radians/sample). Exact at surd-table frequencies — a
grid like `linspace(0, pi, 9)` qualifies — and exact-symbolic elsewhere.
Magnitude via `map(abs, ...)`.

```text
>> dsp.freqz([1, 1], [0, pi/2, pi])
[ 2  1 - I  0 ]
>> map(abs, dsp.freqz([0, 1], [0, pi/3]))    # a pure delay: unit magnitude
[ 1  1 ]
```

The convolution theorem holds *structurally*:
`dsp.freqz(dsp.conv(a, b), w)` equals `dsp.freqz(a, w) .* dsp.freqz(b, w)`
exactly (it's a property test in the suite).

## `dsp.firlow`

```
dsp.firlow(n, wc)
```

An n-tap windowed-sinc lowpass prototype with cutoff `wc` radians/sample:
h[k] = sin(wc·(k−M))/(π·(k−M)), M = (n−1)/2, and wc/π at the center.
Rectangular by default — taper it elementwise:

```text
>> h := dsp.firlow(5, pi/2) .* dsp.hann(5)
[ 0  1/2*π^(-1)  1/2  1/2*π^(-1)  0 ]
>> dsp.freqz(h, [pi/2])     # exactly −1/2: magnitude 1/2 with the
[ -1/2 ]                    # linear-phase factor e^(−iπ) of the M = 2 delay
```

Highpass/bandpass come from the usual transforms (spectral inversion,
modulation) — they're one-liners with `.*` and `vcat`.

## `dsp.hann` / `dsp.hamming` / `dsp.blackman`

```
dsp.hann(n)    dsp.hamming(n)    dsp.blackman(n)
```

Symmetric cosine-sum windows with exact rational coefficients (Hamming
27/50, 23/50; Blackman 21/50, 1/2, 2/25). Exact at table angles:

```text
>> dsp.hann(4)
[ 0  3/4  3/4  0 ]
>> dsp.blackman(3)          # exactly 0 at the ends, not −1.4e-17
[ 0  1  0 ]
```

## `dsp.quantize`

```
dsp.quantize(v, bits)
```

Snap every entry to the fixed-point grid with `bits` fractional bits —
`round(x·2^bits)/2^bits`, ties away from zero — as **exact rationals**:

```text
>> dsp.quantize([1/3, 2/3], 4)                 # 4 fractional bits: round(x·16)/16
[ 5/16  11/16 ]
```

So the quantization error is an exact object you can measure before shipping
coefficients:

```text
>> h  := dsp.firlow(9, pi/4) .* dsp.hamming(9)
>> hq := dsp.quantize(N(h, 30), 15)            # Q1.15 tap values
>> 2^15 .* hq                                  # the integer register values
>> err := dsp.freqz(N(h, 30) - hq, linspace(0, pi, 16))
>> N(map(abs, err), 5)                         # exact-error response, to 5 digits
```

Overflow is the implementer's concern: `quantize` snaps, it never clamps.

## `dsp.remez`

```
dsp.remez(n, edges, desired)
dsp.remez(n, edges, desired, weights)
```

**Exact Parks–McClellan.** Designs an n-tap (odd, Type I) linear-phase FIR
filter minimizing the maximum weighted error over the specified bands —
with the float implementation's failure modes deleted:

* The interpolation system solves **exactly** — ill-conditioning is a
  rounding phenomenon, and there is no rounding.
* Termination is a **theorem, not a tolerance**: the levelled error strictly
  increases each exchange over a finite design grid, so "failed to
  converge" cannot happen.
* The minimax problem is solved exactly *on the design grid* (uniform in
  x = cos ω, ~16 points per coefficient — float implementations iterate on
  a grid too; they just don't solve even that exactly). The returned
  `ripple` is the exact rational minimax error on that grid.

Band `edges` come in ascending pairs in radians/sample within [0, π];
`desired` and optional `weights` (default 1) give one value per band.
Returns `struct(taps, ripple, iterations)` — taps and ripple as exact
rationals, so spec compliance is *decidable*:

```text
>> f := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0])
>> N(f.ripple, 6)
0.119476
>> abs(dsp.freqz(f.taps, [pi])[1]) <= f.ripple      # exact, not an eyeball
true
>> g := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0], [1, 10])
>> 10 * abs(dsp.freqz(g.taps, [pi])[1]) <= g.ripple # weights, exactly honored
true
>> dsp.remez(7, [0, pi], [1]).ripple                # the degenerate case is exact
0
```

Notes: up to 127 taps (the exact solve grows fast past that; large orders
take seconds). Band edges without a rational cosine (most of them) snap
*inward* by at most 2⁻²⁴ ≈ 6e-8 rad — the conservative direction, far below
any physical spec. Quantize the taps with `dsp.quantize` and measure the
exact quantization-error response before shipping, as usual.

## `dsp.window`

```
dsp.window(name, n)      # name: hann, hamming, or blackman
```

The certified-signal sibling of the exact `dsp.hann`/`hamming`/`blackman`
vectors: a window of length n whose samples are **certified enclosures**
computed in interval arithmetic, ready to taper bulk data elementwise. So it
returns a [signal](signals.md), not a vector — the endpoints are a tiny
interval around 0, not the exact `0` of `dsp.hann(4)`:

```text
>> dsp.window(hann, 4)
<signal: 4 samples, f64, max error ±2.9e-15>
```

(`signal(N(dsp.hann(n)))` would instead turn approximations into zero-error
points — this is the honest path.) Tapering a frame before an FFT is then one
line — `slice(clip.ch1, 1, 4096) .* dsp.window(hann, 4096)` — with the
window's enclosures carried into `dsp.fft` of the result.

## `dsp.butter`

```
dsp.butter(n, wc)
dsp.butter(n, wc, highpass)
```

Order-`n` Butterworth lowpass (or highpass) with cutoff `wc`
(radians/sample, 0 < wc < π), designed by the bilinear transform with exact
prewarp K = tan(wc/2). Everything stays exact: the prototype pole constants
are sines of rational multiples of π, and the bilinear map is rational.
Returns `struct(sos, order, kind)` where `sos` is a ⌈n/2⌉×6 matrix of
second-order sections `[b0 b1 b2 1 a1 a2]` (an odd order's first-order
section is zero-padded).

```text
>> f = dsp.butter(2, pi/2)
>> f.sos
[ (2 + sqrt(2))^(-1)  2*(2 + sqrt(2))^(-1)  (2 + sqrt(2))^(-1)  1  0  (2 + sqrt(2))^(-1)*(2 - sqrt(2)) ]
>> dsp.freqz(f, [0]) == [1]        # exact unity DC gain, proven
true
>> N(abs(dsp.freqz(f, [pi/2])[1])^2, 20)
0.5                                # the half-power point, exactly at wc
```

Deploy with `N(f.sos)` (floats) or `dsp.quantize(N(f.sos), bits)`
(fixed-point) — and then *prove the deployed filter stable* with
`dsp.stable`.

## `dsp.stable`

```
dsp.stable(f)          # filter struct
dsp.stable(sos)        # m×6 SOS matrix
dsp.stable(a)          # denominator coefficients [a0, a1, ..., an]
```

Certified strict stability: `true` exactly when every pole lies strictly
inside the unit circle, decided by the exact Schur–Cohn (reflection
coefficient) recursion — no complex root-finding, no floating point, just a
chain of certified sign decisions. A pole exactly *on* the circle answers
`false` (marginally stable is not stable). Because the test is exact on any
rational coefficients, it applies to the coefficients you actually deploy:

```text
>> dsp.stable(dsp.quantize(N(dsp.butter(6, 2/5*pi).sos), 15))
true                               # the 15-bit fixed-point filter, proven
>> dsp.stable([1, -3, 1])          # roots (3±sqrt(5))/2
false
>> dsp.stable([1, 0, 1/sqrt(2)])   # symbolic constants work too
true
```

## `dsp.filter`

```
dsp.filter(b, a, x)
dsp.filter(f, x)       # filter struct: SOS sections in cascade
```

Exact recursive filtering of a vector (zero initial state):
y[i] = (Σ bₖ·x[i−k] − Σ aₖ·y[i−k]) / a0. Rational in, rational out.

Bulk `signal(...)` data is refused on purpose: certified interval
arithmetic diverges through IIR feedback (widths grow geometrically even
for stable filters), and a blown-up enclosure presented as "certified"
would be worse than the honest error. Filter an exact stretch of interest
(`slice`), or use FIR taps with `dsp.conv`.

## `dsp.impz`

```
dsp.impz(f, n)
dsp.impz(b, a, n)
```

The first `n` samples of the impulse response, exactly.

```text
>> dsp.impz([1], [1, -1/2], 5)
[ 1  1/2  1/4  1/8  1/16 ]
```
