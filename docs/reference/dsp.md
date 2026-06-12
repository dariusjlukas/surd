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
`round(x·2^bits)/2^bits`, ties away from zero — as **exact rationals**. So
the quantization error is an exact object you can measure before shipping
coefficients:

```text
>> h  := dsp.firlow(9, pi/4) .* dsp.hamming(9)
>> hq := dsp.quantize(N(h, 30), 15)            # Q1.15 tap values
>> 2^15 .* hq                                  # the integer register values
>> err := dsp.freqz(N(h, 30) - hq, linspace(0, pi, 16))
>> N(map(abs, err), 5)                         # exact-error response, to 5 digits
```

Overflow is the implementer's concern: `quantize` snaps, it never clamps.
