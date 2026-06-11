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

## Running it

Requires a Rust toolchain (`rustup` recommended).

```sh
cargo run            # interactive REPL
cargo test           # behavioral tests in tests/eval.rs
echo "sqrt(2)^2" | cargo run    # pipe mode
```

REPL meta-commands: `:vars` lists the workspace, `:q` quits.

### The web app

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
`src/state/store.ts` (Zustand: cells, transcript, engine status),
`src/plot/` (`LinePlot` — a framework-free ThreeJS painter — plus the React
wrapper that owns pan/zoom and triggers engine resampling), `src/components/`
(notebook, KaTeX output, REPL input bar with block continuation).

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
  an f64. Export saves any selection of workspace variables (anything a
  variable can hold, functions included) into one `surd-data` file; exact
  values round-trip losslessly. An import is a notebook *cell* carrying the
  file's text, so the replay model — and notebook export — keeps working with
  data in play.
- **The stack is set at link time** (`.cargo/config.toml`, 32 MiB) so the
  engine's recursion guards hold on wasm just like they do under
  `run_with_stack` natively.

`plot(f, x, a, b)` is a symbolic value in the engine (the variable is quoted,
like `diff`'s); the frontend samples it at f64 — pixels are already
approximate, results never are.

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
[ 1/2*(1 + sqrt(5)) ]
[ 1/2*(1 - sqrt(5)) ]
>> N(eigenvalues([1,1;1,0]), 30)   # ...or numeric, to any precision
[   1.61803398874989484820458683437 ]
[ -0.618033988749894848204586834366 ]
```

Eigenvalues are roots of the characteristic polynomial, found exactly when it
factors over ℚ into linear and quadratic pieces (rational-root search + the
quadratic formula). Genuinely complex roots or irreducible factors of degree ≥ 3
are *reported*, never approximated — `eigenvalues([0,-1;1,0])` says "complex
eigenvalues (not yet supported)" rather than lying.

Determinants use **fraction-free Bareiss elimination** for numeric matrices (so
integer intermediates don't blow up) and **cofactor expansion** for symbolic
ones (division-free, exact). Inverse / `solve` / `rref` / `rank` share one
exact Gauss-Jordan routine.

Builtins: `sqrt`, `sin`, `cos`, `tan`, `exp`, `ln`, `diff`/`D`, `subs`, `expand`,
`N`, `precision`, `conj`, `re`, `im`, `abs`, and matrix ops `det`, `inv`,
`transpose`/`T`, `solve`, `rref`, `rank`, `eye`/`identity`, `charpoly`,
`eigenvalues`/`eig`. Constants: `pi`, `e`, `I` (imaginary unit) — all three are
ordinary names that user bindings shadow, so `e` and `i` stay free for loop
counters and the like.

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
this is the design's core honesty about undecidability (Richardson's theorem):

```
>> if x then 1 else 2 end
error: expected a true/false condition, got 'x'
>> pi < 4
error: cannot order 'π' and '4'; both must be numbers (try N(...))
>> N(pi) < 4
true
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
- `src/{lexer,parser,ast}.rs` — front end.
- `src/latex.rs` — LaTeX rendering for the web UI (KaTeX). Cosmetic only.
- `src/f64eval.rs` — fast approximate `Expr → f64` for plot sampling, the one
  deliberate exception to arbitrary precision: pixels are already approximate.
- `wasm/` — `wasm-bindgen` bindings (`Session`, JSON results, plot sampling).
- `web/` — the browser UI: worker hosting, transcript persistence, KaTeX,
  canvas plots.

## Testing

`cargo test` runs ~100 tests across five layers (`cargo clippy` is clean of
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
  perfect powers, and decidable comparison. See `calcium`/`arb` for prior art.
- **The assumptions system** (is `x > 0`? integer?) — wants an SMT backend (Z3).
- **Conditionals on symbolic predicates / piecewise results.** Control flow is
  in, and (by design) requires a *decidable* boolean — symbolic/undecidable
  conditions error. A future assumptions system would let some of them resolve.
- **`return`/`break`/`continue`**, closures that capture locals, and `print`
  for in-loop output. (Today a function's value is its last statement; `if` is
  an expression, so recursion needs no early return.)
- **Deep simplification** via equality saturation (`egg`), **integration**
  (Risch), and **equation solving**.
- **More linear algebra**: eigen*vectors*, cubic/quartic eigenvalues, LU/QR
  decompositions, and a null-space basis for underdetermined `solve`.
  (Eigen*values* via rational + quadratic factoring, including complex pairs,
  are done.)
- **DSP, the original motivation**: a correct DFT/FFT now that `exp(I·θ)`
  evaluates numerically, plus FIR/IIR filter design. (Complex arithmetic, `sqrt`
  of negatives, conjugate/abs, and complex `exp`/`sin`/`cos`/`tan`/`ln`/powers
  are done.)
- **Symbolic** complex simplification (Euler expansion `exp(I*x) → cos x + I·sin x`,
  recognizing `exp(I*pi) → -1` exactly rather than numerically).
- **Square-factor extraction** (`sqrt(8) -> 2 sqrt(2)`) and degree-aware
  polynomial display ordering.
- ~~**Implicit multiplication**~~ — done, in the unambiguous cases: a number
  or `)` followed by `(` or an identifier multiplies (`2x`, `2pi`, `2sin(x)`,
  `2(x+1)`, `(x+1)(x-1)`, `(x+1)y`). Deliberately *not* implicit: `ident(…)`
  stays a function call, adjacent identifiers (`x y`) stay an error (they
  carry block grammar — `x then`), `1.5.5` stays an error, and `3e5` is
  rejected loudly rather than silently becoming `3*e5`. Exponents bind first:
  `x^2y` is `(x^2)·y`.
- **User-defined functions.**
- ~~**Float contagion**~~ — done: a float operand makes the numeric part of
  `+`/`*`/`^` float (`N(pi) + 1` is one float; `N(2) + x` keeps `x` symbolic),
  and floats compare/test equal by their exact binary value. Remaining gap:
  float *coefficients* don't merge like terms (`N(2.5)*x + x` stays two terms).
```
