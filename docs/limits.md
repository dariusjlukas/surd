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

## Things that error on purpose

These are design decisions, not gaps:

- **Symbolic conditions** — `if x then … end` errors; conditions must be a
  real `true`/`false` (wrap comparisons in `N(...)`).
- **Ordering symbols** — `pi < 4` errors; ordering arbitrary reals is
  undecidable.
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
  roots beyond perfect powers, and decidable comparison.
- **An assumptions system** (is `x > 0`? an integer?) — wants an SMT backend.
- **Piecewise results / symbolic predicates** in conditionals.
- **`return` / `break` / `continue`**, closures capturing locals, `print`.
- **Deep simplification** (equality saturation), **integration** (Risch),
  **equation solving**.
- **Square-factor extraction** — `sqrt(8)` stays `sqrt(8)`, not `2*sqrt(2)`.
- **Symbolic complex simplification** — `exp(I*pi)` folds to −1 numerically
  (under `N`), not symbolically.
- **Eigenvectors beyond quadratic surds** — eigen*values* are exact through
  Cardano cubics and biquadratic quartics, but eigen*vectors* don't follow
  into those fields yet.
- **Float coefficients don't merge like terms** — `N(2.5)*x + x` stays two
  terms.
- **DSP toolkit** (the original motivation): DFT/FFT, FIR/IIR filter design —
  the complex groundwork is done.

## Testing philosophy

The engine is tested at five layers: unit tests per module, behavioral tests
for every feature, property-based tests (commutativity, distributivity,
display round-trip, and a differential test of exact-then-`N` against an
independent f64 evaluator), robustness fuzzing (thousands of random strings
must never panic), and one regression test per bug ever hit. Coverage-guided
fuzzing (`cargo-fuzz`) goes deeper on two targets: never-panic and
print-then-reparse round-tripping.
