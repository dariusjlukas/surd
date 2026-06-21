# Limits and design notes

## Resource guards

"Compute time be damned" still shouldn't mean "hang forever." Pathological
or adversarial input turns into a clean error, never a crash:

| Guard | Limit |
| --- | --- |
| Tokens per program | 15,000 |
| Parser nesting depth | 512 |
| Expression-evaluation depth | 8,000 |
| Function-call recursion frames | 1,500 |
| `while` loop iterations | 10,000,000 |
| Exact exponent ceiling | `2^(10^15)` stays symbolic instead of building a gigabyte bignum |
| `precision(d)` | clamped to 1…100,000 digits |
| `dsp` pairwise products per call | 4,000,000 (a DFT of length n costs n²) |
| Signal FFT length | 2²² samples (power of two) |
| `dsp.remez` | ≤ 127 taps; band edges snap inward ≤ 2⁻²⁴ rad |
| Signal convolution | 2²⁸ pairwise products |
| Bulk import size | 2²⁴ samples per file |
| Comparison interval refinement | 8,192 bits (≈ 2,466 digits), then "may be equal" |
| `plot` sampling | adaptive 601 → 4,801 points per curve; windows the cap can't resolve are labeled "undersampled", never silently aliased |
| `plot3d` sampling grid | adaptive 81×81 → 641×641, same undersampled labeling |

## Things that error on purpose

These are design decisions, not gaps:

- **Symbolic conditions** — `if x then … end` errors; conditions must be a
  real `true`/`false`.
- **Ordering free symbols** — `x < 4` errors; a free symbol has no fixed
  value. Constant comparisons (`pi < 4`, `sqrt(2)+sqrt(3) > pi`) are decided
  by certified interval refinement — answered only when the enclosures
  provably separate, so never a guess. Equal-valued constants written
  differently (`(sqrt(2)+sqrt(3))^2` vs `5+2*sqrt(6)`) refuse at the
  refinement cap: *proving* equality needs real algebraic numbers
  (deliberately deferred, below).
- **`=` as a truth test** — `=` builds an equation; use `==` for decidable
  equality.
- **Scientific notation** — `3e5` is rejected with a hint (`3*10^5`), rather
  than silently parsing as `3*e5`.
- **Matrix + scalar** — `A + 1` errors; only matrices add to matrices.
- **Non-integer matrix powers** — `A^(1/2)` errors.
- **Arithmetic on booleans, functions, structs** — opaque values.
- **Division by zero** — always an error, even symbolically detectable cases.
- **Eigenvalues with no radical form** — reported (casus irreducibilis,
  general quartics, degree ≥ 5), never silently approximated.

## Known gaps (deliberately deferred)

Scoped out of the prototype on purpose — this is where an exact CAS balloons:

- **Real algebraic numbers** (polynomial + isolating interval) for exact
  roots beyond perfect powers, and for *proving equality* of constants —
  interval refinement (shipped) can certify any strict ordering, but can
  never prove two different-looking constants equal.
- **An assumptions system** (is `x > 0`? an integer?) — wants an SMT backend.
- **Piecewise results / symbolic predicates** in conditionals.
- **`return` / `break` / `continue`**, closures capturing locals, `print`.
- **Deep simplification** (equality saturation), **integration** (Risch),
  **equation solving**.
- **Exact trig beyond the surd table** — `sin(pi/6)` folds to `1/2` and
  `cos(pi/5)` to the golden-ratio surd, but denominators outside
  {1, 2, 3, 4, 5, 6, 8, 10, 12} stay symbolic (`cos(pi/7)` has no surd
  form at all — its minimal polynomial is a cubic).
- **Radical combining beyond provable signs** — `sqrt(x)*sqrt(y)` stays put;
  `√a·√b → √(a·b)` fires only where nonnegativity is provable (numbers,
  π/e, and quadratic-surd sums like `10 − 2*sqrt(5)`).
- **Square-factor extraction is best-effort** — bounded trial division plus
  a perfect-square check; a square of two huge primes stays under the
  radical (never wrong, occasionally incomplete).
- **Eigenvectors beyond quadratic surds** — eigen*values* are exact through
  Cardano cubics and biquadratic quartics, but eigen*vectors* don't follow
  into those fields yet.
- **Float coefficients don't merge like terms** — `N(2.5)*x + x` stays two
  terms.
- **A module system for user code** — modules are
  [structs of functions](language/modules.md); there is no file import or
  package mechanism.
- **More DSP** — the [`dsp` namespace](reference/dsp.md) covers DFT,
  convolution, FIR design (windowed-sinc and exact Parks–McClellan),
  frequency response, windows (exact and certified), and fixed-point
  quantization; STFT/spectrograms, Type II–IV Remez, IIR design, and
  z-transforms are future work.
- **Signal gaps** — [signals](reference/signals.md) cover the certified
  bulk pipeline end to end (FFT, convolution, reductions, import/export in
  both substrates, plotting with zoom refinement, slicing); the substrates
  still never convert into each other implicitly, and zoom refinement needs
  a live session (a reloaded notebook's plots refine again after any
  evaluation replays them).
- **No indexed assignment** — `v[1] := 5` is not a thing; values are
  immutable. Build with `map`, `vcat`, `hcat`.
- **More statistics** — the [`stats` namespace](reference/stats.md) now covers
  the univariate basics, quantiles, fit metrics, exact least squares
  (`linfit`/`polyfit`/`lsq`), OLS with full inference (`regress`), weighted and
  penalized regression (`wls`/`ridge`), logistic regression (`logit`), nonlinear
  least squares (`nlfit`), the standard probability distributions, and
  regression diagnostics; mixed/hierarchical models and time-series methods are
  still future work.

## Testing philosophy

The engine is tested at five layers: unit tests per module, behavioral tests
for every feature, property-based tests (commutativity, distributivity,
display round-trip, and a differential test of exact-then-`N` against an
independent f64 evaluator), robustness fuzzing (thousands of random strings
must never panic), and one regression test per bug ever hit. Coverage-guided
fuzzing (`cargo-fuzz`) goes deeper on two targets: never-panic and
print-then-reparse round-tripping.
