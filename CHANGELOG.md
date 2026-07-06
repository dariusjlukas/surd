# Changelog

All notable changes to surd are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under `[Unreleased]`; `scripts/bump-version.sh` rolls that
section into a dated, versioned release.

## [Unreleased]

### Added

- **Certainty badges.** Every evaluated result now carries its trust class
  across the engine boundary and shows it as a badge in the app: **exact**
  (no approximation anywhere in the value), **certified** (inexact but
  carrying a proven error enclosure), **symbolic** (contains free
  variables), or **approximate** (contains `N(...)` floats —
  round-to-nearest, not certified). Data imports report it too, so a bulk
  signal import visibly lands as *certified* while a CSV of rationals lands
  as *exact*.
- **Refusals are a first-class outcome.** An honest refusal ("the values may
  be equal") now renders as its own amber outcome with an explanation —
  distinct from red errors. Refusing is the engine working as designed, and
  the UI finally says so.
- **Certified numeric preview.** Exact, non-literal scalar results (`1/3`,
  `sqrt(2)`, `sin(1)`) show a faint `≈ 0.333333` ghost. The preview digits
  are *certified*, not floating-point guesses: both endpoints of a
  directed-rounding enclosure must round to the same 6-significant-digit
  string, or no preview is shown at all. Toggleable in Settings → Notebook.
- **`=` vs `:=` guard.** A cell like `x = 3` — which builds an *equation*
  and binds nothing, while echoing exactly like an assignment confirmation —
  now gets an inline hint pointing at `x := 3`.
- **Undo for destructive notebook operations.** Deleting a cell, deleting a
  notebook, and clearing a notebook now offer a transient Undo toast
  (Cmd/Ctrl+Z also works outside an editor). Restoring replays the affected
  cells through the engine, so the workspace comes back exactly — replay is
  the consistency model. Notebooks remain the only copy of your work, so
  the recovery path matters.
- **Example gallery.** The empty-notebook Welcome screen now offers three
  worked notebooks — an exact-by-default tour (ending in an honest
  refusal), certified DSP (FFT round-trip proof, exact Remez), and exact
  statistics. They are shipped as *sources only* and evaluated live on
  open, so every result, badge, and refusal on screen comes from your own
  engine, never a canned screenshot.
- **Per-cell timing.** Cells that took ≥ 100 ms show a faint duration under
  their output.
- **CI test gate.** A new workflow runs the native suite plus the wasm
  substrate smoke test on every push/PR, and both the Pages deploy and the
  release pipeline now refuse to ship a wasm bundle the smoke test rejects
  — the astro-float wasm32 bug class is invisible to native tests, so the
  gates run on the real bundle.
- **`str` and string length.** `str(a, b, ...)` renders each argument to its
  canonical printed form and concatenates — conversion, concatenation, and
  formatting in one primitive (precision composes with `N(x, digits)`:
  `str("pi is ", N(pi, 5))` is `"pi is 3.1416"`). The result feeds plot
  labels, so titles can be computed: `plot(..., title = str("r = ", r))`.
  `len(s)` counts a string's characters.
- **Anonymous functions and closures.** `x -> x^2` and `(a, b) -> a + b` are
  function values usable anywhere a value goes. Functions created inside
  another function capture the locals they mention **by value** at creation,
  so the closure factory `make(k) := (x -> k*x)` works; top-level free names
  stay late-bound against the workspace (recursion keeps working). Local
  functions can call themselves by name.
- **`for` loops.** `for x in lo:hi do ... end` / `lo:step:hi` iterates an
  inclusive range of exact values (rational and negative steps included —
  endpoints and step must be exact so the stopping comparison is decidable);
  `for x in m do ... end` iterates a vector's elements or a matrix's rows.
- **`elseif`.** `if a then ... elseif b then ... else ... end` chains cases
  under a single `end` (the spelled-out nested `else if ... end end` still
  works).
- **`filter` and `fold`.** `filter(pred, v)` keeps the elements where the
  predicate is `true` (a non-boolean verdict refuses; so does keeping
  nothing — there is no empty matrix). `fold(f, init, v)` is the left fold
  over a vector's elements or a matrix's entries.
- **Multi-argument `map`.** `map(f, a, b, ...)` over same-shape matrices
  passes one entry from each — the elementwise zip.

### Fixed

- **SOUNDNESS (wasm32, browser/desktop app only): astro-float's 64-bit words
  broke `exp`-family functions.** The library only special-cases x86 for its
  32-bit word type, so on wasm32 it computes with `u64` words while `usize`
  is 32-bit — and every truncating `as usize` inside it misbehaved:
  `exp` dropped the integer part of its argument (`exp(1)` → `1`, so the
  certified comparison engine **asserted `exp(1) ≠ e` and
  `exp(1) − e < −10⁻¹⁰⁰⁰`** — false certified orderings, the worst bug class);
  `powi` returned its base unchanged for n ≥ 2, which made the
  special-function convergence eps 2⁻¹ instead of 2⁻¹⁴⁸ and silently
  truncated every CDF/PDF series to two terms (`normcdf(1)` ≈ 0.8226 instead
  of 0.8413); `pow` (`2^π` ≈ 3.25 instead of 8.82), `sinh`, `cosh`, and
  complex trig (`sin(2+3i)` collapsed to `sin(2)`) inherited it. Native
  builds were never affected; the already-documented wasm32 fractional
  `BigFloat::parse` bug is the same root cause. All routes now go through
  guarded wrappers (`expr::bf_exp` / `bf_pow_round` / `bf_sinh` / `bf_cosh`,
  and an exact exponent shift for the eps) that never hand the library an
  argument with an integer part and never call `powi`. Pinned by native
  identity tests and wasm-substrate tripwires in `web/smoke.mjs`.

### Changed

- **A false `if` without `else` no longer yields a silent `0` in value
  position.** Using the value of such an `if` (assignment right side,
  function result, arithmetic operand) is now an error; in statement
  position — a conditional assignment for its side effect — it remains
  allowed. The silent `0` was the same bug family as defaulting an
  undecidable `==` to false (soundness audit #12).

- **3D axis labels.** `plot3d(..., xlabel = "...", ylabel = "...",
  zlabel = "...")` — the labels replace the default axis names (the plot
  variables and `z`) on the box edges and turn with the box as you orbit.
  Mathtext like the 2D labels (`$...$` renders LaTeX). PNG/PDF exports of 3D
  plots now composite the projected axis names *and* tick values around the
  frame, which previously exported without either.

## [0.9.0] - 2026-07-05

### Added

- **Plot titles and axis labels.** `plot(..., title = "...", xlabel = "...",
  ylabel = "...")` (and `plot3d(..., title = "...")`) accept trailing
  keyword-string arguments on every plot form — curves, signals, and scatter.
  Labels are mathtext: plain text where `$...$` segments render as LaTeX
  (`title = "response of $H(\omega)$"`). They render in the live view and are
  baked into PNG/PDF exports — which now also composite the theme background,
  tick numbers, and (for multi-curve plots) the legend around the frame,
  where they previously exported the bare transparent canvas.
- **String literals.** `"..."` is a new inert value type (it can't be
  computed with — arithmetic on it refuses), carried for plot labels.
  Backslashes stay literal so LaTeX needs no doubling; only `\"` and `\\`
  escape.

### Fixed

- **`==`/`!=` on constant values now refuse instead of answering `false` when
  equality is undecidable.** Previously the fall-through defaulted to `false`,
  so e.g. `dsp.idft(dsp.dft([1;2;3;4;5;6;7]))[5] == 5` — two exactly equal
  values whose difference exceeds the algebraic engine's degree caps —
  asserted a false disequality. Constant comparisons now go through the same
  certified machinery as `<`/`>`: interval separation proves `≠`, exact
  algebra proves `=` (the heptagon identity
  `cos(π/7) − cos(2π/7) + cos(3π/7) == 1/2` now answers `true`), and anything
  undecided errors with "the values may be equal". Comparisons involving free
  symbols keep their structural semantics.

### Changed

- Trig of rational multiples of π now canonicalizes the *angle* into
  [0, π/2] even when no surd form exists: `cos(10/7·π)`, `cos(4/7·π)` and
  `cos(−3/7·π)` all render as `−cos(3/7·π)` (periodicity, antipode, and
  reflection — exact identities). Equal values get equal canonical forms, so
  differences cancel structurally, mirrored structures (window entries,
  conjugate DFT twiddles) become recognizably identical, and huge angles
  collapse without needing digits of π.
- Performance, without any change in results: `expand` collects like terms
  between distribution rounds instead of building the full cartesian mountain
  (`expand((x+y+1)^12)`: 1.55 s → 3.3 ms); signal packing verifies f64
  enclosures by integer cross-multiplication instead of bignum gcd + division
  (6×); sum canonicalization no longer re-canonicalizes each term's basis
  (20–40% across exact matrix algebra, stats, and filter design); symbolic
  windows are built as half + mirror and DFT/dftmatrix twiddles are built
  once per residue class instead of once per matrix cell (exact 16-point DFT:
  25 ms → 4 ms; 256-point exact Hann window: 7.8 ms → 2.6 ms).

### Added

- A criterion benchmark suite (`cargo bench`, benches/engine.rs) driving the
  engine through `Interpreter::eval_line` — parsing, canonicalization,
  certified comparison, `N(...)`, algebraic numbers, both signal substrates,
  exact linear algebra, stats, and filter design. `--save-baseline` /
  `--baseline` give before/after comparisons.
- Formula transforms and interactions. A model-formula term can now be any
  scalar expression in column names — `y ~ x + x^2`, `y ~ ln(x) + z + x*z`
  (a product term is what R writes `a:b`), and the response side transforms
  too (`ln(y) ~ x`). Terms are evaluated row by row by exact substitution, so
  the design matrix stays exact — symbolic, like `ln(35)`, where no closed
  numeric form exists; a log-linear fit of `y = 2^x` recovers `N(exp(slope))`
  as exactly 2. Columns used inside a transform or interaction must be
  numeric — a categorical column errors with a pointer at `data.dummy` — and
  a constant term is rejected (the intercept is automatic). Works everywhere
  formulas do: `regress`, `wls`, `ridge`, `lasso`, `logit`, and `cv`.
- Classical hypothesis tests, in the house style — exact statistics, symbolic
  p-values:
  - `stats.ttest(x, mu)` / `stats.ttest(x, y)` / `stats.ttest(x, y, paired)` —
    one-sample, two-sample, and paired t-tests. The two-sample form is
    Welch's (the safe default), with the Welch–Satterthwaite degrees of
    freedom kept as an exact rational and handed to the symbolic `tcdf`,
    which evaluates at non-integer ν. Reports statistic, df, se, p-value,
    95% confidence interval, and estimate.
  - `stats.chisqtest(table)` / `stats.chisqtest(x, y)` — Pearson's
    chi-square test of independence on a contingency table or two
    categorical columns (cross-tabulated, levels reported). The statistic
    and expected counts are exact rationals; no continuity correction
    (matches R's `chisq.test(..., correct = FALSE)`).
  - `stats.cortest(x, y)` — tests whether the Pearson correlation is zero;
    the estimate is the exact surd `stats.cor` computes.
- Categorical columns import from CSV. A word-like text cell (`us`,
  `treated`) now imports as a symbol — a categorical level, the same value a
  hand-built `[us; eu; us]` column holds — instead of failing the whole file,
  so `data.dummy`, `data.groupby`, and model formulas
  (`stats.regress(mpg ~ weight + origin, cars)`) work on text columns straight
  from a file. The import summary flags each categorical column with its
  level count ("origin (392×1 matrix, categorical (3 levels))"), so a typo'd
  file is visible at a glance. A cell that *looks* numeric but doesn't parse
  (`3.4O`, `1.2.3`, a date) is still a loud, located error — a typo'd number
  must never silently become a category. The header rule is unchanged: a
  first row containing word-like text is a header.
- Missing-value handling for real-world data. A blank CSV cell — or one
  spelled `NA`, `N/A`, `NaN`, `null`, or `?` in any case, or a generic-JSON
  `null` — now imports as the marker symbol `NA` instead of failing the whole
  import, and the import summary counts what came in ("… — 3 missing values
  (NA)"). surd does no NA arithmetic, silent or otherwise: every `stats` and
  `data` function refuses NA data with a pointed error, and the new
  `data.dropna` is the explicit fix — it removes NA entries from a vector,
  NA-carrying rows from a matrix, and (listwise) every row of a table where
  any column is NA, keeping columns aligned.
- `data.split(x, frac[, seed])` — a reproducible random train/test split of a
  table, matrix, or vector into `struct(train, test)`. Membership comes from
  a seeded Fisher–Yates shuffle (SplitMix64, exactly uniform via rejection
  sampling), so the engine stays deterministic: the same call always produces
  the same split, and a different seed a different one. Each side keeps the
  original row order.
- `stats.cv(X_or_formula, y_or_data, k[, opts])` — k-fold cross-validation
  for `regress`, `ridge`, and `lasso`, the out-of-sample counterpart to the
  in-sample `r2`/`aic`. The design matrix is built once from the full data
  (so categorical encodings agree across folds), a seeded shuffle deals the
  folds, and each fold is predicted by a model fitted on the rest. Refits are
  exact for regress/ridge — noiseless linear data scores a CV error of
  exactly 0 — and f64 for lasso, like the fitters themselves. `opts` is
  `struct(model, lambda, seed)`; a `lambda` *vector* scores every candidate
  on the same folds and reports the `best`, the standard way to choose a
  penalty.

## [0.8.0] - 2026-07-01

### Fixed

- Container values can no longer reach scalar positions. A matrix (or signal,
  boolean, function, struct) slipped into scalar arithmetic came out of
  canonicalization as well-sorted nonsense — `dot([[1,2], 2], [3,4])` evaluated
  to `8 + 3*[ 1  2 ]`, `[[1,2], 3]` built a nested matrix, and
  `linspace(1, [1,2], 3)` put matrices inside entries. Every path that feeds a
  user value into a scalar position — matrix literals, `map`/`fill` results,
  `linspace` endpoints, `subs` replacements, evaluating `D` at a bound
  variable, and the scalar pieces of `vcat`/`hcat` — now rejects non-scalar
  values with a clear error. Symbolic entries are untouched (they're the
  point).
- `stats.wls` now insists every weight is a positive *number*. A symbolic
  weight used to pass the positivity check (which only tested numeric weights)
  and surface later as a confusing downstream error.
- The `dsp.window` unknown-window error message no longer contains a run of
  stray spaces from a wrapped string literal.

### Changed

- Canonical operand ordering (the sorted form of sums and products) now uses a
  structural comparison instead of rendering each operand to a string. The old
  key re-formatted entire subtrees on every `add`/`mul` — quadratic in
  expression depth on the hottest path in the engine. Output order is unchanged
  except in corner cases where the string order was an artifact (e.g. numeric
  values now compare numerically, so `x^2` sorts before `x^10`).
- The lasso `df` convention is now documented: it counts every nonzero
  coefficient including the unpenalized intercept (R's glmnet reports one
  less).

### Added

- `fill`, a matrix builder. `fill(v, n)` makes an n×n matrix with every entry
  `v` (square, like `eye`), and `fill(v, rows, cols)` a rows×cols one; a `1×n`
  fill is the easy way to a constant row vector. The value is any scalar
  expression, so `fill(x, 2)` is a symbolic matrix. When the first argument is a
  function, each entry is `f(row, col)` at its 1-based coordinate (matching
  `m[row, col]`), so `fill(g, 3)` with `g(i, j) := (i-1)*3 + j` numbers the grid
  1–9. Zero dimensions and non-scalar constant values (a matrix, a signal) are
  rejected.
- `stats.sum`, `stats.min`, and `stats.max`. `stats.sum(v)` adds every element
  exactly, flowing symbolic entries through like `stats.mean` (so
  `stats.sum([a; b; 2])` is `2 + a + b`). `stats.min`/`stats.max` return the
  smallest/largest element by the same exact ordering as `stats.median` — a
  rational beats a nearby float — with the matching entry returned verbatim;
  symbolic entries can't be ordered, so they error.
- Scalar broadcast over matrix `+` and `-`, mirroring `*` and `/`. `A + 2`,
  `2 + A`, `A - 2`, and `2 - A` now add or subtract the scalar from every entry
  (with `2 - A` negating each entry first), instead of raising "a matrix and a
  scalar don't add". Matrix-with-matrix `+`/`-` still require equal shapes.
- Strided slicing. An index range can carry a step as a middle field,
  `lo:step:hi` (MATLAB/Julia order): a scalar `step` keeps every `step`-th
  position (`v[1:2:]`), and a `(take, skip)` pair keeps `take` consecutive
  positions then skips `skip`, repeating — the general "take N, skip M" pattern
  (`v[1:(4, 1):]`). A scalar `step` of `k` is the pair `(1, k - 1)` and a plain
  `lo:hi` is `(1, 0)`, so the existing range forms are unchanged. The stride
  works on either matrix axis and on signals (producing a decimated sub-signal),
  and the open forms (`lo:step:`, `:step:hi`, `:step:`) still apply. A step or
  take count of `0` is a clean error.
- Output suppression with a trailing `;`, the MATLAB/Julia convention: ending a
  statement with `;` computes and binds the value as usual but suppresses the
  echoed result — so assigning a large matrix or vector no longer floods the
  screen. The REPL prints nothing; the notebook collapses the cell to a faint,
  clickable shape hint (`; 5×3 matrix`) that expands the full output on demand,
  and the value is always still listed in the workspace panel. Errors are never
  suppressed.

## [0.7.0] - 2026-06-29

## [0.6.0] - 2026-06-25

## [0.5.0] - 2026-06-23

## [0.4.0] - 2026-06-22

## [0.3.0] - 2026-06-22

### Added

- 3D scatter plots: `scatter3d(x, y, z)` draws three equal-length vectors as
  markers in the surface view. Overlay it in `plot3d(...)` to compare a fitted
  surface against measured `(x, y, z)` data —
  e.g. `plot3d(b0 + b1*x + b2*y, scatter3d(xs, ys, zs), x, 0, 10, y, 0, 10)` —
  or `plot3d(scatter3d(xs, ys, zs))` on its own, auto-boxed to the data. Points
  are static (orbit/zoom without resampling) and the hover probe snaps to the
  nearest marker.

## [0.2.0] - 2026-06-21

### Added

- Scatter plots: `scatter(x, y)` draws two equal-length vectors as markers.
  Overlay it in `plot(...)` to compare measured data against a curve —
  e.g. `plot(scatter(xs, ys), m.predict, x, a, b)` — or `plot(scatter(xs, ys))`
  on its own, auto-windowed to the data. Points are static (pan/zoom re-windows
  client-side) and the hover probe snaps to the nearest point.
- Fitted models are directly plottable: `stats.linfit` and `stats.nlfit` now
  return a `predict` field holding the fitted curve as a function — evaluate it
  (`m.predict(2.5)`) or plot it (`plot(scatter(xs, ys), m.predict, x, a, b)`).
  Relatedly, `plot` now accepts any one-argument function as a curve, so
  `plot(f, x, a, b)` draws a user-defined `f` directly.
- Offline documentation in the desktop app: the Help button now opens a copy
  of the docs bundled into the build (in its own window, no network), falling
  back to the hosted site only when a build shipped without them. The web build
  still links to the hosted docs.
- Version reporting: `surd --version` on the CLI, a `version()` binding in the
  wasm engine, and a version line (with a link to the releases page) in the
  desktop/web app's Settings → About.

## [0.1.0]

First tagged release. Exact-by-default computer-algebra engine with:

- Exact arithmetic over arbitrary-precision rationals, radicals, and symbolic
  constants; floats only on explicit `N(x)`.
- A REPL/CLI front end and a wasm-powered notebook UI (web + Tauri desktop).
- Numerical tooling: GLMs, penalized/weighted regression, nonlinear least
  squares with an exact symbolic Jacobian, plus DSP and statistics namespaces.

[Unreleased]: https://github.com/dariusjlukas/surd/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.9.0
[0.8.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.8.0
[0.7.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.7.0
[0.6.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.6.0
[0.5.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.5.0
[0.4.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.4.0
[0.3.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.3.0
[0.2.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.2.0
[0.1.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.1.0
