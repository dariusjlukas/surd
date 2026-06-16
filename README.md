# surd

A prototype of an **exact-by-default computer algebra system** — the engine for
a browser-based, no-install mathematical scratchpad. Written in Rust (pure-Rust
deps, so it cross-compiles to `wasm32` cleanly for the eventual web frontend).

(A *surd* is an irrational root kept in exact symbolic form — `sqrt(2)` the
object, not `1.41421…` the approximation. That's the whole idea.)

The thesis: most "plot a curve / linear regression / matrix math / FIR filter"
work doesn't need heavyweight licensed software, and the popular numeric stacks
quietly trade mathematical correctness for engineering pragmatism. This aims the
other way — **correctness above all, compute time be damned.**

## Design pillars

- **Exact by default.** `1/3` is the rational 1/3. `1.5` is `3/2`. `sqrt(2)` and
  `pi` are *symbolic objects*, not floats. You only get a float when you ask for
  one, via `N(...)` — and that crossing is meant to be visible.
- **CAS first, numerics second.** Every value is a symbolic expression that may
  collapse to a number.
- **`:=` assigns, `=` builds an equation.** `=` never tests truth — equality of
  reals is undecidable in general (Richardson's theorem), so an equation is a
  piece of data you manipulate, not a boolean.
- **Bounded auto-simplification.** Construction applies cheap, terminating
  canonicalization (fold rationals, `sqrt(2)^2 -> 2`, combine like terms). Deep
  simplification is reserved for explicit operations.

## Documentation

User documentation — the language guide and a reference page for every
built-in, with verified examples — is published at
**<https://dariusjlukas.github.io/surd/docs/>** (the web app itself lives at
<https://dariusjlukas.github.io/surd/>). It is built from `docs/` as a
[MkDocs](https://www.mkdocs.org) (Material) site and deployed alongside the
web app by the Pages workflow. To work on it locally:

```sh
pip install mkdocs-material
mkdocs serve         # live-reloading preview at http://localhost:8000
mkdocs build         # static site in site/
```

## Running it

Requires a Rust toolchain (`rustup` recommended).

```sh
cargo run            # interactive REPL
cargo test           # the full suite (unit + behavioral + property + fuzz + regression)
echo "sqrt(2)^2" | cargo run    # pipe mode
```

REPL meta-commands: `:vars` lists the workspace, `:q` quits.

### The web app

A live deployment runs at **<https://dariusjlukas.github.io/surd/>** —
no install needed.

The real frontend lives in `app/` — React + TypeScript + Vite + Tailwind +
Zustand, with custom ThreeJS plots — on top of a `wasm-bindgen` crate in
`wasm/`. (`web/` is the earlier no-build harness; its `smoke.mjs` is still the
headless check of the wasm bundle.)

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build wasm --target web --out-dir ../app/src/engine/pkg
cd app && npm install && npm run dev      # → http://localhost:5173
npm run build                             # production bundle in app/dist/

# headless engine check (uses the web/pkg copy):
wasm-pack build wasm --target web --out-dir ../web/pkg && node web/smoke.mjs
```

App structure: `src/engine/` (typed worker protocol + `EngineClient`),
`src/editor/` (a CodeMirror input with surd syntax highlighting,
autocompletion, and block continuation), `src/state/store.ts` (Zustand: cells,
transcript, engine status), `src/plot/` (`LinePlot` and `SurfacePlot` —
framework-free ThreeJS painters for 2D curves and 3D surfaces — plus the React
wrappers that own pan/zoom and trigger engine resampling), `src/components/`
(notebook, KaTeX output, workspace panel).

How it holds together:

- **The worker is the cancellation boundary.** The engine runs in a Web
  Worker; *cancel* terminates the worker. Because the engine is
  deterministic, the transcript of successful inputs **is** the serialized
  workspace — a fresh worker replays it to restore state. The transcript and
  the rendered cells both persist in `localStorage`: a reload paints the
  notebook instantly from cached results while the engine replays in the
  background.
- **Results are structured.** `Session::eval` returns JSON (kind, plain text,
  LaTeX, plot samples, error); the UI renders math with KaTeX and draws
  `plot(...)` values with ThreeJS — gaps at poles, never a lie bridged across
  an asymptote.
- **Plots resample on pan/zoom.** A plot result carries the re-parseable text
  of its expression (workspace bindings already substituted), so the frontend
  asks the worker to re-sample any window at full resolution — zooming reveals
  detail instead of stretching 600 stale samples. Drag pans, wheel zooms x,
  shift+wheel zooms y; touching the y-axis switches it from auto-fit
  (quantile) to manual until reset.
- **A workspace panel** lists every binding (name, value as typeset math),
  refreshed from `Session::workspace()` after each successful evaluation.
- **Raw data imports/exports** live in the workspace panel. Import a file
  (`surd-data` JSON, generic JSON, or CSV — sniffed) and it lands in a fresh
  variable; files with named members (CSV columns, JSON keys, exported
  variables) arrive inside a *struct* (`sensor.temp`, see below), so imported
  names can never collide with existing bindings. Numbers are read from their
  literal text — `0.1` in a sensor log becomes the exact rational 1/10, never
  an f64. A separate waveform button imports bulk data — WAV, raw binary
  (`f64`/`f32`/`i16`), or packed CSV — straight into [signals](#certified-bulk-data-signals).
  Export saves any selection of workspace variables (anything a variable can
  hold, functions and signals included) into one `surd-data` file; exact
  values round-trip losslessly. An import is a notebook *cell* carrying the
  file's text, so the replay model — and notebook export — keeps working with
  data in play.
- **The stack is set at link time** (`.cargo/config.toml`, 32 MiB) so the
  engine's recursion guards hold on wasm just like they do under
  `run_with_stack` natively.

`plot(f, x, a, b)` is a symbolic value in the engine (the variable is quoted,
like `diff`'s); the frontend samples it at f64 — pixels are already
approximate, results never are.

### The desktop app (offline)

The same frontend ships as a fully offline desktop app via [Tauri](https://tauri.app/).
Tauri (rather than Electron) is the natural fit here: it's Rust-native like the
rest of the project, hosts the web UI in the OS's own webview, and produces
~10 MB installers instead of bundling a ~150 MB Chromium. The engine still runs
as wasm inside the webview — nothing talks to a server, so the app works with no
network at all. The shell lives in `app/src-tauri/` and is a detached Cargo
workspace, keeping Tauri's dependency tree out of the lean native/`wasm` builds.

```sh
cd app
npm install
npm run tauri:dev      # dev window with HMR (builds the wasm engine first)
npm run tauri:build    # native bundle in app/src-tauri/target/release/bundle/
```

On macOS, `tauri:build`'s final `.dmg` step styles the disk-image window with
AppleScript, which needs the terminal to hold Automation permission for Finder
— without it the step fails (`bundle_dmg.sh` exits 64). Use
`npm run tauri:build:dmg` (sets `CI=1`, which skips the cosmetic styling) for a
plain working `.dmg`, or `npm run tauri:build -- --bundles app` for just the
`.app`. CI sets `CI` automatically, so release builds are unaffected.

The desktop build adds two thin platform shims over the web build (see
`app/src/platform/desktop.ts`), both no-ops in a browser: external links (the
docs button) open in the system browser, and notebook/data/plot exports go
through a native **Save** dialog (the `save_export` command in
`app/src-tauri/src/lib.rs`) instead of a browser download.

Installers for macOS (universal `.dmg`), Windows (`.msi`/`.exe`), and Linux
(`.rpm` for Fedora, plus `.AppImage` and `.deb`) are built by
`.github/workflows/release.yml` — push a `v*` tag (or run it manually) and it
attaches them to a draft GitHub Release.
Builds are **unsigned by default** (macOS Gatekeeper / Windows SmartScreen will
warn). macOS signing + notarization is opt-in: set the `MACOS_SIGNING`
repository variable to `true` and add the `APPLE_*` secrets, and the workflow
signs the build. Linux builds ship a `SHA256SUMS.txt` for verification. The
workflow header documents the required secrets and the Windows signing options
(SignPath / Azure Trusted Signing).

## What works today

```
>> 1/3 + 1/6
1/2
>> 1.5
3/2
>> sqrt(2)*sqrt(2)
2
>> (2*x)^2
4*x^2
>> 2*pi + pi
3*π
>> diff(sin(x), x)
cos(x)
>> expand((x+1)^2)
1 + x^2 + 2*x
>> x := 3
3
>> x^2 + 1
10
>> diff(x^2, x)        # diff takes x by *name*: 2x, evaluated at x = 3
6
>> N(1/3)
0.333333333333333333333333333333
>> y = x + 1
y = x + 1
```

### Arbitrary-precision floats

`N(x)` crosses from exact to approximate at a default of 30 significant digits;
`N(x, d)` asks for `d`, and `precision(d)` changes the default. `N` also maps
over matrices. Constants and transcendentals are computed to whatever precision
you request (via `astro-float`), and the numeric engine reaches right through
symbolic structure:

```
>> N(pi, 100)
3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628034825342117068
>> N(sqrt(2), 60)
1.41421356237309504880168872420969807856967187537694807317668
>> N(sin(pi/6), 40)        # = 1/2, recovered numerically
0.5
>> fib(n) := if n < 2 then n else fib(n-1) + fib(n-2) end
>> N(fib(30)/fib(29), 40)  # Fibonacci ratio: only ~11 digits of φ
1.618033988748203621343798191078293911856
>> N((1 + sqrt(5))/2, 40)  # closed form: exact to all 40
1.61803398874989484820458683436563811772
```

### Exact linear algebra over ℚ

Matrix literals use `,` between entries and `;` between rows. Every operation is
exact — no rounding, ever:

```
>> A := [1, 2; 3, 4]
[ 1  2 ]
[ 3  4 ]
>> inv(A)
[  -2     1 ]
[ 3/2  -1/2 ]
>> A * inv(A)            # exactly the identity, not "approximately"
[ 1  0 ]
[ 0  1 ]
>> inv([1/2, 1/3; 1/4, 1/5])    # a float tool gives you 11.9999…; this is exact
[  12  -20 ]
[ -15   30 ]
>> det([1,2,3; 4,5,6; 7,8,10])
-3
>> solve([2,1,-1; -3,-1,2; -2,1,2], [8; -11; -3])
[  2 ]
[  3 ]
[ -1 ]
>> det([a, b; c, d])    # symbolic entries work too
-b*c + a*d
>> diff([x^2, sin(x); x, 1], x)    # differentiation distributes entrywise
[ 2*x  cos(x) ]
[   1       0 ]
>> charpoly([2,1;1,2])             # det(A - λI), symbolically
3 + lambda^2 - 4*lambda
>> eigenvalues([1,1;1,0])          # exact — irrational roots kept as sqrt
[ 1/2 + 1/2*sqrt(5) ]
[ 1/2 - 1/2*sqrt(5) ]
>> N(eigenvalues([1,1;1,0]), 30)   # ...or numeric, to any precision
[   1.61803398874989484820458683437 ]
[ -0.618033988749894848204586834366 ]
>> eigenvectors([1,1;1,0])         # columns pair with eigenvalues(A), in order
[ 1/2 + 1/2*sqrt(5)  1/2 - 1/2*sqrt(5) ]
[                 1                  1 ]
>> nullspace([1,2,3; 4,5,6])       # kernel basis, as columns
[  1 ]
[ -2 ]
[  1 ]
>> solve([1,1;2,2], [3;6])         # underdetermined: the *general* solution
struct(nullspace = [ -1 ]
[  1 ], particular = [ 3 ]
[ 0 ])
>> f := qr([1,1;1,0])              # exact Gram-Schmidt: A = Q·R
>> T(f.Q) * f.Q                    # orthonormal *exactly*, surd norms and all
[ 1  0 ]
[ 0  1 ]
>> eigenvalues([0,0,2; 1,0,0; 0,1,0])    # Cardano: exact cube roots
[                              2^(1/3) ]
[ -1/2*2^(1/3) + 1/2*2^(1/3)*sqrt(3)*I ]
[ -1/2*2^(1/3) - 1/2*2^(1/3)*sqrt(3)*I ]
```

Eigenvalues are exact wherever a radical form exists: rational-root peeling,
the quadratic formula (complex pairs included), Cardano's formula for cubics,
and biquadratic quartics with their nested radicals (`±sqrt(1 + sqrt(2))`).
What provably has no such form is *reported*, never approximated: three real
cubic roots need complex cube roots (casus irreducibilis — the trigonometric
form isn't implemented), general quartics await the Ferrari reduction, and
degree ≥ 5 has no radical formula at all (Abel–Ruffini).

`lu(A)` returns `struct(L, U, P)` with P·A = L·U (Doolittle, row pivoting;
exact, singular matrices included). `qr(A)` returns `struct(Q, R)` by exact
Gram-Schmidt — projections run on the unnormalized orthogonal columns, so
radicals only enter at normalization, and Qᵀ·Q folds to the identity *exactly*
rather than to within 1e-16.

Eigen*vectors* run Gauss-Jordan in the field the eigenvalue actually lives in —
ℚ, ℚ(√d), or its complex extension — where the zero test is decidable, so
`eigenvectors([1,1;1,0])` produces the golden-ratio eigenvector symbolically
and `inv(V)·A·V` diagonalizes complex rotations *exactly*. The columns of
`eigenvectors(A)` pair with the entries of `eigenvalues(A)`, so A·V = V·diag(λ);
a defective matrix (fewer independent eigenvectors than the multiplicity) is
reported in those words, never padded with zero columns. (Eigenvalues that
need cubic or nested radicals are still exact via `eigenvalues`, but
`eigenvectors` doesn't follow into those fields yet.) An underdetermined
`solve` returns a struct: one particular solution plus a `nullspace` basis —
every solution is `particular` + a combination of the basis columns.

Determinants use **fraction-free Bareiss elimination** for numeric matrices (so
integer intermediates don't blow up) and **cofactor expansion** for symbolic
ones (division-free, exact). Inverse / `solve` / `rref` / `rank` share one
exact Gauss-Jordan routine.

Builtins: `sqrt`, `sin`, `cos`, `tan`, `exp`, `ln`, `diff`/`D`, `subs`, `expand`,
`N`, `precision`, `conj`, `re`, `im`, `abs`; data ops `map`, `len`, `size`,
`slice`, `dot`, `vcat`/`hcat`, `linspace`, `plot`/`plot3d`, `struct`, `signal`,
`mid`, `bound`; and matrix ops `det`, `inv`, `transpose`/`T`, `solve`, `rref`,
`rank`, `nullspace`/`kernel`, `lu`, `qr`, `eye`/`identity`, `charpoly`,
`eigenvalues`/`eig`, `eigenvectors`. Domain toolkits live behind **namespaces**
(`dsp.dft(v)`, `stats.mean(v)`) so they don't claim bare names. Constants:
`pi`, `e`, `I` (imaginary unit) — all three are ordinary names that user
bindings shadow, so `e` and `i` stay free for loop counters and the like.

`diff`/`D` and `subs` take their variable argument by *name* and keep it
symbolic while the expression argument evaluates, so a workspace binding
doesn't collapse the expression first: after `x := 3`, `diff(x^2, x)` is the
derivative 2·x evaluated at x = 3, i.e. `6` — not an error about `diff(9, 3)`.

### Complex numbers

`I` (capital — so `i` stays free for loop counters) is the imaginary unit.
Complex numbers behave like *numbers*: arithmetic folds eagerly rather than
staying factored.

```
>> (1 + I)*(1 - I)
2
>> (1 + I)^2
2*I
>> sqrt(-4)                 # square roots of negatives are imaginary
2*I
>> (2 + 3*I)/(1 + I)
5/2 + 1/2*I
>> (2 + 3*I) * conj(2 + 3*I)   # = |z|^2
13
>> abs(3 + 4*I)
5
>> eigenvalues([1,-1; 1,1])    # complex eigenvalues, returned not refused
[ 1 + I ]
[ 1 - I ]
>> diff(x^2 + I*x, x)
2*x + I
```

Builtins: `conj`, `re`/`real`, `im`/`imag`, `abs`. Real and imaginary parts may
themselves be symbolic (`x + I` is fine). Symbols are assumed real (so
`conj(x) = x`).

**Complex transcendentals**, to arbitrary precision via `N`. `exp`/`sin`/`cos`/
`tan`/`ln` of a complex argument are evaluated through Euler's formula and the
hyperbolic identities, and complex powers via `exp(w·ln z)`:

```
>> N(exp(I*pi))                 # Euler's identity, recovered exactly
-1
>> N(exp(I*pi/2), 20)           # e^(iπ/2) = i
I
>> N(exp(2*pi*I/3), 25)         # a primitive cube root of unity
-0.5 + 0.8660254037844386467637232*I
>> N(ln(I), 20)                 # ln(i) = iπ/2
1.5707963267948966192*I
>> N(exp(1 + I), 20)
1.4686939399158851571 + 2.2873552871788423912*I
```

(`N(exp(I*pi) + 1)` shows a ~1e-60·i residue rather than `0` — the honest
precision floor of a numeric computation, since proving it's exactly zero is
undecidable.)

The residue snapping that makes `N(exp(I*pi))` read `-1` applies **only to
transcendental results**, where a component that is mathematically zero can
only come back as cancellation residue. A purely arithmetic complex value has
full relative precision in each component, so a genuinely tiny part survives:
`N(1 + 10^(-50)*I, 30)` is `1 + 1e-50*I`, not `1`. (The remaining caveat: a
tiny *exact* component fed through a transcendental expression — say
`1 + sin(10^(-50))*I` — is still indistinguishable from residue and gets
snapped.)

### Signal processing, exactly

DSP — the original motivation — has landed. It lives in the `dsp` namespace,
so it doesn't claim bare names like `dft` or `conv` for everyone. DFT twiddle
factors are exact wherever the angle has a surd form, so the DFT of a rational
vector is a vector of exact surds and `dsp.idft(dsp.dft(v))` is *identically*
`v`, not `v` up to epsilon:

```
>> dsp.dft([1; 2; 3; 4])
[       10 ]
[ -2 + 2*I ]
[       -2 ]
[ -2 - 2*I ]
>> dsp.conv([1, 2], [1, 3])           # (1 + 2z)(1 + 3z) = 1 + 5z + 6z²
[ 1  5  6 ]
>> dsp.freqz([1, 1], [0, pi/2, pi])   # FIR frequency response, exact
[ 2  1 - I  0 ]
>> dsp.hann(4)                        # cosine-sum windows, exact rationals
[ 0  3/4  3/4  0 ]
```

The namespace covers the DFT/IDFT and the Fourier matrix, linear and circular
convolution, FIR frequency response (`freqz`), windowed-sinc (`firlow`) and
**exact Parks–McClellan** (`remez`) filter design, the Hann/Hamming/Blackman
windows, and fixed-point `quantize`. `remez` is the showpiece: the
interpolation system solves *exactly* (so ill-conditioning, a rounding
phenomenon, cannot happen) and termination is a *theorem*, not a tolerance —
it returns the exact rational minimax ripple, so spec compliance is
**decidable**, not eyeballed.

### Statistics

The `stats` namespace runs every estimator in exact arithmetic: the mean of
rationals is a rational, a standard deviation is an exact surd, and perfectly
linear data correlates to *exactly* ±1 — model quality is never hidden inside
float noise.

```
>> stats.mean([1; 2; 3; 4])
5/2
>> stats.std([1; 2; 3; 4])              # an exact surd, not a rounded decimal
sqrt(5/3)
>> stats.cor([1; 2; 3], [2; 4; 6])
1
>> stats.linfit([0; 1; 2], [1; 2; 4])   # exact least-squares line
struct(intercept = 5/6, slope = 3/2)
```

Also `median`, `var`, `cov`, `quantile`, `rmse`/`r2`, and exact least-squares
`polyfit`/`polyval`/`lsq` — Vandermonde *conditioning* is a float problem, and
there are no floats here.

### Certified bulk data: signals

Exact arithmetic is the right tool for *designing* a filter; it is the wrong
tool for running it over a million samples. **Signals** bridge the gap: packed
bulk data where every sample carries a certified error enclosure — an interval
computed with outward rounding at every step, so the true value provably lies
inside. The worst-case error is part of the value, and the display refuses to
hide it:

```
>> s := signal([1/3; -2; 5/7; 1])
<signal: 4 samples, f64, max error ±1.1e-16>
>> s .* s
<signal: 4 samples, f64, max error ±8.9e-16>
>> r := dsp.ifft(dsp.fft(s)).re
>> dsp.peak(r - s) < 1/10^12            # the round-trip error is *provably* tiny
true
```

There are two substrates — hardware `f64` (audio-scale fast) and arbitrary
precision (bounds shrink at will), both rigorous, never mixed implicitly.
`signal(...)` is the only way in; `mid`/`bound`/indexing/the reductions are the
only ways out; mixing a signal into exact arithmetic is an error. The `dsp`
operations (interval FFT, convolution, `peak`/`rms`) carry the enclosures
through, so every number in a pipeline is either exact or comes with a proven
bound — there is no third category to debug. The web app imports WAV / raw
binary / CSV into signals, and exports them losslessly in both substrates.

### Vectors, data, and plotting

Vectors are 1×n / n×1 matrices; indexing is 1-based (`v[2]`, `m[2, 1]`, with
`m[i]` the whole row), elementwise operators are `.*` `./` `.^`, and scalar
functions map over a matrix automatically. Helpers: `map`, `len`/`size`,
`slice`, `dot`, `vcat`/`hcat`, and `linspace` (with an exact rational step).

```
>> map(abs, dsp.freqz([0, 1], [0, pi/3]))    # a pure delay: unit magnitude
[ 1  1 ]
>> linspace(0, pi, 5)
[ 0  1/4*π  1/2*π  3/4*π  π ]
```

`plot(f1, …, fk, x, a, b)` and `plot3d(f, x, a, b, y, c, d)` are symbolic
values in the engine (the plot variable is quoted, like `diff`'s); the web
frontend samples them at f64 and draws interactive curves and surfaces that
**resample at full resolution** as you pan and zoom — sampling is the one
deliberate exception to arbitrary precision, since pixels are already
approximate and results never are.

### It's a language, not just a calculator

Comparisons produce booleans; `if`/`while`/functions give Turing-complete
control flow. Programs are statements separated by newlines or `;`, and the
REPL keeps reading until a block closes.

```
>> fact(n) := if n == 0 then 1 else n*fact(n-1) end
>> fact(20)                       # exact arbitrary precision — no overflow
2432902008176640000
>> function newton(steps)         # √2 by Newton's method, in exact rationals
..     x := 1
..     i := 0
..     while i < steps do
..         x := (x + 2/x) / 2
..         i := i + 1
..     end
..     x
.. end
>> newton(5)
886731088897/627013566048
>> N(newton(5))
1.4142135623730951
```

**The decidable-boolean rule.** Control-flow conditions must evaluate to a real
`true`/`false`. A condition that can't be decided is an error, never a guess —
this is the design's core honesty about undecidability (Richardson's theorem).
*Constant* comparisons are decided by certified interval refinement (directed
rounding, precision doubling until the enclosures provably separate), so the
answer is exact knowledge, not a float guess:

```
>> pi < 4
true
>> sqrt(2) + sqrt(3) > pi
true
>> if x then 1 else 2 end
error: expected a true/false condition, got 'x'
>> x < 4
error: cannot order 'x' and '4'; both must be constant real values (a free
symbol has no fixed value — try subs(...) or N(...))
>> (sqrt(2)+sqrt(3))^2 < 5 + 2*sqrt(6)     # exactly equal: refuses, never lies
error: cannot order '(sqrt(2) + sqrt(3))^2' and '5 + 2*sqrt(6)': they agree
to at least 2466 significant digits — the values may be equal
```

Floats *do* compare, and exactly: a binary float is the rational m·2^k, so a
float-vs-exact comparison is decided losslessly on that value — never by
rounding the other side. (Corollary: `N(2) == 2` is `true`, but `N(1/3) == 1/3`
is `false` — the float is genuinely not 1/3, and saying so is the point.)

For non-numbers, `==`/`!=` test *decidable structural* equality (after
canonicalization), not mathematical equality — `(x-1)*(x+1) == x^2 - 1` is
`false` until you `expand`.

Functions: `f(x) := expr` for one-liners, or `function f(x) … end` for blocks.
Bodies have their own local scope; recursion works. Operators: `< > <= >= == !=`,
`and`/`or`/`not`. Reserved words: `if then else end while do function and or not
true false`.

Structs group named values; fields hold anything a variable can and are read
with `.` (which binds tighter than `^`). Data imports arrive as structs:

```
>> s := struct(gain = 1/3, taps = [1, 2; 3, 4])
struct(gain = 1/3, taps = [ 1  2 ]
[ 3  4 ])
>> s.gain * det(s.taps)
-2/3
```

Field names are taken from the syntax (a binding for `gain` elsewhere doesn't
interfere), fields are kept sorted (so `==` is field-order-independent), and
structs are opaque to arithmetic — `s + 1` is an error, not a guess.

## Architecture

```
source ─▶ lexer ─▶ parser ─▶ ast ─▶ eval ─▶ Expr   (canonical)
                                      │
                          smart constructors in expr.rs
                          (add / mul / pow enforce canonical form)
```

- `src/expr.rs` — the `Expr` value type and **all** canonicalization. The
  cleverness is concentrated here: `sqrt(x)` is `x^(1/2)`, `mul` collects like
  bases and sums exponents, `pow` flattens `(a^b)^c -> a^(b*c)` only where it's
  sound (guarding against the `sqrt(x^2) = |x|` branch-cut trap).
- `src/eval.rs` — tree-walking evaluator + builtins + the workspace.
- `src/matrix.rs` — exact linear algebra (Bareiss/cofactor determinants,
  Gauss-Jordan inverse/solve/rref/rank). Operates on `Expr`, so it's symbolic
  too; exact ℚ is just the all-numeric case.
- `src/dsp.rs` / `src/remez.rs` — the `dsp` namespace: exact DFT/IDFT and
  Fourier matrix, linear/circular convolution, `freqz`, windowed-sinc and
  exact Parks–McClellan (`remez`) FIR design, windows, and `quantize`.
- `src/stats.rs` — the `stats` namespace: exact mean/variance/correlation,
  quantiles, and least-squares (`linfit`/`polyfit`/`lsq`).
- `src/signal.rs` — certified bulk-data signals: interval enclosures in two
  substrates, the interval FFT/convolution, and reductions.
- `src/interval.rs` — the certified interval arithmetic underneath signals
  and the decidable constant comparisons (directed rounding, precision
  doubling).
- `src/dataio.rs` — the `surd-data` JSON format plus best-effort JSON/CSV
  import; decimal literals are read as exact rationals, never round-tripped
  through f64.
- `src/{lexer,parser,ast}.rs` — front end.
- `src/latex.rs` — LaTeX rendering for the web UI (KaTeX). Cosmetic only.
- `src/f64eval.rs` — fast approximate `Expr → f64` for plot sampling, the one
  deliberate exception to arbitrary precision: pixels are already approximate.
- `wasm/` — `wasm-bindgen` bindings (`Session`, JSON results, plot sampling,
  bulk imports).
- `app/` — the real browser UI (React + TypeScript + Vite + ThreeJS):
  notebook, KaTeX, interactive 2D/3D plots, workspace panel, data
  import/export. `web/` is the earlier no-build harness, kept for its
  headless `smoke.mjs` engine check.

## Testing

A pre-commit hook (`.githooks/pre-commit`) runs `cargo test`, the app's
prettier/eslint/tsc checks, and the wasm smoke test before every commit. Enable it once per clone with
`git config core.hooksPath .githooks` (bypass a single commit with
`git commit --no-verify`).

`cargo test` runs ~190 tests across five layers (`cargo clippy` is clean of
errors):

- **Unit** (`#[cfg(test)]` in each module) — lexer tokenization, parser
  precedence/associativity, and canonicalization on the smart constructors.
- **Behavioral** (`tests/eval.rs`) — end-to-end results for every feature.
- **Property-based** (`tests/properties.rs`, via `proptest`) — invariants that
  must hold for *every* generated expression: commutativity/associativity,
  distributivity, additive inverse/identity, differentiation linearity, and
  display **round-trip** (re-parsing a printed result is a fixed point). The
  centerpiece is a **differential test**: exact-then-`N` is checked against an
  independent `f64` evaluation, which catches precedence/sign/canonicalization
  bugs wholesale. (It already found one: `(x+1) − (x+1)` not cancelling to 0.)
  The DSP and signal layers are held to the same bar — the convolution theorem
  (`freqz(conv(a,b)) = freqz(a).*freqz(b)`) is a property test, and another
  convolves random rationals exactly as an oracle and checks every coefficient
  lands inside its certified signal enclosure, in both substrates.
- **Robustness / fuzz** (`tests/robustness.rs`) — `proptest` throws thousands of
  random and "math-soup" strings at the engine asserting it *never panics*, plus
  curated adversarial inputs and checks that the resource guards (below) turn
  pathological input into clean errors.
- **Regression** (`tests/regression.rs`) — one test per bug ever hit.

**Resource guards** keep untrusted input from crashing or hanging the engine:
a token-count cap and parser-nesting cap (deeply nested input → error, not
stack overflow), an evaluation-depth cap and recursion-frame cap, a loop
iteration cap, and a ceiling on exact exponents (`2^(10^15)` stays symbolic
instead of building a gigabyte bignum). Evaluation runs via
`surd::run_with_stack` so legitimate deep work has room before the guards trip;
the WASM target should set its stack size at link time.

**Coverage-guided fuzzing** (`fuzz/`, via `cargo-fuzz`/libFuzzer, nightly) goes
deeper than proptest's random sampling. Two targets:

- `eval` — arbitrary bytes through lex → parse → eval; asserts it never panics,
  hangs, or overflows.
- `roundtrip` — a printed result must re-evaluate to itself (a canonical form is
  a fixed point), catching display/canonicalization bugs.

Both evaluate through `run_with_stack`, so they exercise the real production
configuration. Run them with:

```sh
cargo +nightly fuzz run eval --sanitizer none
cargo +nightly fuzz run roundtrip --sanitizer none
```

(`--sanitizer none` sidesteps Apple-Silicon ASan friction — we're hunting
panics/overflows in safe Rust, not memory errors.) The `roundtrip` target
immediately earned its keep, finding that `(11/5)^x` printed as `11/5^x`
(which re-parses as `11/(5^x)`).

## Deliberately deferred (the disciplined-MVP line)

These were scoped out on purpose; they're where an exact CAS balloons.

- **Real algebraic numbers** (poly + isolating interval) for exact roots beyond
  perfect powers, and for *proving equality* of constants — interval refinement
  (shipped) certifies any strict ordering, but can never prove two
  different-looking constants equal. See `calcium`/`arb` for prior art.
- **The assumptions system** (is `x > 0`? integer?) — wants an SMT backend (Z3).
- **Conditionals on symbolic predicates / piecewise results.** Control flow is
  in, and (by design) requires a *decidable* boolean — symbolic/undecidable
  conditions error. A future assumptions system would let some of them resolve.
- **`return`/`break`/`continue`**, closures that capture locals, and `print`
  for in-loop output. (Today a function's value is its last statement; `if` is
  an expression, so recursion needs no early return.)
- **Deep simplification** via equality saturation (`egg`), **integration**
  (Risch), and **equation solving**.
- **More linear algebra**: the general (non-biquadratic) quartic eigenvalue
  via Ferrari's resolvent, the trigonometric closed form for the casus
  irreducibilis, and eigen*vectors* for eigenvalues beyond quadratic surds
  (wants field arithmetic in ℚ(∛·)). (Done: eigenvalues through Cardano
  cubics and biquadratic quartics, eigenvectors over ℚ(√d) and its complex
  extension, `nullspace`, LU and QR, and the particular + null-space form of
  underdetermined `solve`.)
- **More DSP and statistics**: STFT/spectrograms, Type II–IV Remez, IIR
  design, and z-transforms; weighted and iterative regression (logistic,
  optimizers). (Done: the `dsp` namespace — DFT/IDFT, convolution, `freqz`,
  windowed-sinc and exact Parks–McClellan FIR design, windows, quantization —
  the certified signal substrate, and the `stats` namespace through exact
  least squares.)
- **Degree-aware polynomial display ordering** — `expand((x+1)^3)` prints
  `1 + x^3 + 3*x + 3*x^2`, not in descending degree.
- ~~**DSP, the original motivation**~~ — done: a correct DFT now that complex
  arithmetic, `sqrt` of negatives, conjugate/abs, and complex
  `exp`/`sin`/`cos`/`tan`/`ln`/powers all evaluate, plus exact FIR design and
  the certified signal substrate (see above; IIR and z-transforms remain).
- ~~**Symbolic complex simplification**~~ — done: `exp(I*x)` unfolds to
  `cos(x) + sin(x)*I`, and `exp(I*pi)` folds to `-1` exactly, no floats.
- ~~**Square-factor extraction**~~ — done (best-effort): `sqrt(8) → 2*sqrt(2)`,
  `sqrt(8/9) → 2/3*sqrt(2)`, and `√a·√b → √(a·b)` fires where nonnegativity is
  provable (`sqrt(2)*sqrt(3) → sqrt(6)`, while `sqrt(x)*sqrt(y)` stays put).
- ~~**User-defined functions**~~ — done: `f(x) := …` one-liners and
  `function … end` blocks, with local scope and recursion.
- ~~**Implicit multiplication**~~ — done, in the unambiguous cases: a number
  or `)` followed by `(` or an identifier multiplies (`2x`, `2pi`, `2sin(x)`,
  `2(x+1)`, `(x+1)(x-1)`, `(x+1)y`). Deliberately *not* implicit: `ident(…)`
  stays a function call, adjacent identifiers (`x y`) stay an error (they
  carry block grammar — `x then`), `1.5.5` stays an error, and `3e5` is
  rejected loudly rather than silently becoming `3*e5`. Exponents bind first:
  `x^2y` is `(x^2)·y`.
- ~~**Float contagion**~~ — done: a float operand makes the numeric part of
  `+`/`*`/`^` float (`N(pi) + 1` is one float; `N(2) + x` keeps `x` symbolic),
  and floats compare/test equal by their exact binary value. Remaining gap:
  float *coefficients* don't merge like terms (`N(2.5)*x + x` stays two terms).
