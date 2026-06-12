# Signals — certified bulk data

Exact arithmetic is the right tool for *designing* a model; it is the wrong
tool for running one over a million samples. Signals are the bridge:
**packed bulk data where every sample carries a certified error enclosure**.
Each sample is an interval [lo, hi] computed with outward rounding at every
step, so the true value provably lies inside. The result of a pipeline is
never silently wrong — its worst-case error is part of the value, and the
display refuses to hide it:

```text
>> s := signal([1/3; -2; 5/7; 1])
<signal: 4 samples, f64, max error ±5.6e-17>
>> s .* s
<signal: 4 samples, f64, max error ±6.7e-16>
```

## Two substrates

```
signal(v)            # hardware f64 — audio-scale fast
signal(v, digits)    # arbitrary precision — slower, bounds shrink at will
```

* **f64**: arithmetic (`+ − × ÷ √`) is rigorous outright — IEEE 754
  guarantees correctly rounded operations, and every result is widened one
  ulp outward. Transcendentals (`sin cos tan exp ln`) additionally assume
  the platform libm is within 2 ulp (the standard assumption, made visible:
  they widen by 8 ulps).
* **Arbitrary precision**: astro-float with *directed* rounding — rigorous
  end to end with no libm assumption.

```text
>> bound(signal([1/3; 2/7]))
2.78e-17
>> bound(signal([1/3; 2/7], 50))
4.32e-78
```

The substrates never mix implicitly; repack to convert.

## The boundary is explicit

`signal(...)` is the only way in; `mid`, `bound`, indexing, and the
reductions are the only ways out. Mixing a signal into exact arithmetic is
an error, exactly like the design thesis demands:

```text
>> mid(s)          # column matrix of midpoints (floats)
>> bound(s)        # certified max |true − mid| over all samples
>> bound(s, i)     # …for sample i
>> s[i]            # the midpoint of sample i
>> s + [1; 2]      # error: cannot mix an exact matrix with a signal
```

`mid(s) ± bound(s)` is always a true statement — the bound covers the
midpoint's own representation error, not just the enclosure width.

## Operations

Elementwise `+ − .* ./ .^`(integer), scalar broadcast, and the scalar
functions (`sin cos tan exp ln sqrt abs`) all work and all widen honestly.
Division by a sample interval containing zero is an error naming the sample.

In `dsp`:

| Function | Description |
| --- | --- |
| `dsp.fft(s)` / `dsp.ifft(f)` | Radix-2 interval FFT (power-of-two lengths) → `struct(re, im)` |
| `dsp.conv(a, b)` | Certified bulk convolution |
| `dsp.pad(s, n)` | Zero-pad (never truncates) |
| `dsp.peak(s)` | Certified upper bound on max │x│ |
| `dsp.rms(s)` | Certified upper bound on the RMS |

```text
>> s := signal([1; 2; 3; 4; 5; 6; 7; 8])
>> r := dsp.ifft(dsp.fft(s)).re
>> dsp.peak(r - s) < 1/10^12     # the round-trip error is *provably* tiny
true
```

The test suite holds signals to the same standard as the exact engine: a
property test convolves random rational vectors exactly (an independent
oracle) and verifies every exact coefficient lies inside its certified
enclosure, compared as exact rationals — in both substrates.

## Plotting and slicing

`plot(s)` (or `plot(s1, s2)` to overlay) draws a signal's samples over the
1-based index. Signals longer than the point cap draw as a min/max
*envelope* (extremes survive, never aliased away) and are flagged as
decimated — and **zooming refines**: the session re-decimates the zoomed
window from the full-resolution data, so detail appears as you go in. (If
the session has restarted since the plot rendered, the shipped envelope
stands.)

`slice(s, start, n)` cuts `n` samples from 1-based `start` — handy for
trimming to a power of two before `dsp.fft`. (`slice` works on exact vectors
too.)

## Bulk imports

| Source | Result |
| --- | --- |
| WAV (PCM 16/24/32, float 32/64) | `struct(rate, ch1[, ch2…])`, normalized to [−1, 1) exactly |
| Raw binary (`f64`/`f32`/`i16`, little-endian) | one signal, unnormalized |
| Packed CSV | `struct` of one signal per column |

Integer PCM and IEEE floats convert to f64 *exactly*, so imported data
starts with certified error **zero**; CSV decimals start within ±1 ulp of
their correctly-rounded parse. (Import caps: 2²⁴ samples per file.)

In the web app, the waveform button in the workspace panel imports any of
these — the format follows the file extension (`.wav`, `.csv`, and raw
binary as `.f64`/`.f32`/`.i16`). Bulk imports replay with the notebook like
any other data cell.

Signals **export** through the normal workspace export, in both substrates:
f64 bounds as plain numbers (serde round-trips them exactly), and
arbitrary-precision bounds as exact decimal strings (a binary float's
decimal expansion terminates) — re-import is bit-identical either way.

## Putting it together: model vs. data

```text
>> taps := dsp.firlow(9, pi/4) .* dsp.hamming(9)     # design: exact
>> hq := dsp.quantize(N(taps, 30), 15)               # quantize: exact
>> y := dsp.conv(data.ch1, signal(hq))               # run: certified
>> resid := y - predicted                            # compare: certified
>> dsp.rms(resid)                                    # measure: an upper bound
```

Every number in that pipeline is either exact or carries a proven bound —
there is no third category to debug.
