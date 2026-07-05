# surd

Exact/certified CAS + DSP engine (Rust core, wasm + web/desktop app). The
product thesis: every result is exact, a certified enclosure, or a refusal —
never a silent approximation. Users trust results so they only have to debug
their model, not the tool. **A wrong certified/exact answer is the worst
possible bug; refusing ("may be equal", an error) is always acceptable.**

See SOUNDNESS-AUDIT-2026-07-04.md for the current known-defect list.

## Soundness rules (each one exists because violating it shipped a real bug)

**Certified enclosures [lo, hi] must contain the true real value.** Certified
comparisons must never assert a false ordering. When in doubt, widen or refuse.

**Never order BigFloats with raw `<`/`>`/`partial_cmp`/`.min()`/`.max()`/
`is_positive`.** astro-float's PartialOrd is unreliable near zero (exact zero
compares by raw exponent, so 2.7e-20 < 0) and `is_positive(+0) == true`. Use
`expr::bf_lt` and `bf_strictly_pos`/`bf_strictly_neg` — nothing else.

**Never parse a fractional decimal string with astro-float.** `BigFloat::parse`
mispositions the decimal point of fractional strings > ~19 digits on wasm32
(integers are fine). Route: `dataio::decimal_to_rat` → integer parses →
directed division.

**Keep astro-float precision ≥ 64 bits.** Below that it returns NaN.

**Do not trust astro-float at the exponent extremes.**
- `exp` flushes underflow to *exact +0 even with RoundingMode::Up* (below
  ≈ e^(−1.4885e9)) — an Up-rounded 0 from exp is NOT a valid upper bound.
- `BigFloat::from_f64` halves every subnormal f64 (2^−1073 → 2^−1074) — guard
  any f64→Big readback that can see magnitudes < ~2.2e-308.
- The docs describe Up/Down as "round half toward ±∞"; the implementation is
  true directed rounding (verified against a 512-bit reference). Trust the
  behavior, not the doc — and don't "fix" code based on the doc.

**Widen at the scale of the error, not the scale of the result.** N-ulp
widening at the result's magnitude only covers *relative* error. Absolute-scale
error sources — a point-valued angle fed to trig (FFT twiddles), a
round-to-nearest interval width under cancellation — need interval-valued
inputs plus Lipschitz bounds. `fft_big` and `signal::window` are the correct
pattern; `fft_f64`'s point twiddles were a confirmed containment break.

**Realness must be proven, never assumed.** An expression that is not
`Expr::Complex` can still be complex-valued: `pi^(2*I)` is a real base with a
complex exponent. Any rule gated on realness or nonnegativity — `known_nonneg`,
`(a^b)^c` collapse, `re`/`im`/`conj`, sqrt clamping a negative lo to 0 — must
verify every exponent inside is provably real, or refuse.

**src/special.rs is approximate, not certified.** Never present its output
(p-values, gamma/beta) as exact/certified; keep it symbolic until `N()`.

**src/algebraic.rs (real algebraic numbers) invariants:**
- Sturm-chain reductions must preserve signs: use `content_reduced` (divide
  by positive content only), NEVER `primitive()` (it flips negative leading
  coefficients and silently breaks the variation count — a shipped bug
  caught by the chain's own unit test).
- No minimal polynomials, by design: equality is the gcd common-root test
  over intersected isolating intervals. Sums/products multiply defining
  degrees; the MAX_DEG/MAX_BITS_COEFF caps make constructors return `None`,
  which callers must turn into the honest "may be equal" refusal.
- Powers go through the single resultant `Res_x(p(x), y − xⁿ)` (degree
  preserved), never repeated multiplication (degree explodes as degⁿ).
- π and e are transcendental: `from_expr` must keep returning `None` for
  them — an "algebraic" representation of either would be a lie.

**Never call a `with_consts`-using helper (e.g. `rat_to_bigfloat`) from
inside a `with_consts` closure** — the RefCell re-borrow panics at runtime
(`to_bigfloat` and `eval_iv` bodies run inside one; use the passed `cc`).

## Testing conventions

- The gold standard is a **containment property test against an exact
  BigRational oracle, on both substrates** (see tests/properties.rs
  `signal_conv_encloses_the_exact_result`). A new certified kernel is not done
  until it has one.
- **Roundtrip oracles don't count** (ifft∘fft is blind to an
  invertible-but-wrong transform and cancels twiddle bias). Neither do checks
  computed *from the enclosures under test* (e.g. asserting via `dsp.peak` of
  the same signal) — those are tautological for containment.
- Push generators into the ugly regions: subnormals, near-overflow, intervals
  straddling zero, division, near-tie comparisons. Every confirmed kernel bug
  lived in an untested region.
- `cargo test` runs with overflow checks; release does not. Integer arithmetic
  on user-controlled magnitudes (exponents, digit counts) needs explicit
  guards, not wrapping luck.

## Architecture notes

- Certified comparison engine: src/interval.rs (64→8192-bit refinement;
  refuses at the ceiling, never guesses). Certified bulk signals:
  src/signal.rs, two substrates (f64 ulp-widened / Big directed-rounded);
  `signal()`/`mid`/`bound`/indexing are the only boundary crossings, substrate
  mixing must error. Exact Remez: src/remez.rs (all claims are scoped to the
  2^-24 design grid, not the continuous band — keep it that way or close the
  gap honestly).
- Built-in namespaces (`dsp.`, `stats.`, `dataio.`) dispatch via
  `is_namespace`/`call_namespace` in src/eval.rs; user modules are structs of
  functions; **no file import/package mechanism — deliberate**.
- Printer invariant: the printed canonical form must re-parse and evaluate to
  a fixed point (fuzz/roundtrip enforces it). Watch operator precedence when
  touching rendering — `-8^(1/3)` reads as `-(8^(1/3))`.
- Missing-data sentinels on import: `NA`/`nan`/`null`/`?` etc.
  (dataio.rs `is_missing_cell`) — collides with real categorical data
  (ISO code `NA`); check there before changing import semantics.

## Build & dev

- `cargo test` — full suite, ~6 s. Fuzz targets live in fuzz/ (separate crate).
- `cargo bench` — criterion suite in benches/engine.rs, driven entirely through
  `Interpreter::eval_line` so numbers reflect real REPL/wasm cost.
  `-- --save-baseline main` records, `-- --baseline main` compares; benches
  build with the shipping profile (opt-level "s"). Performance work must not
  change results: same canonical forms, same enclosures, same refusals —
  optimize by doing less work, never by weakening a check.
- REPL: `./target/release/surd`, reads stdin. **Assignment is `:=`** —
  `x = 3` builds an *equation* (a value, displayed as `x = 3`, easily
  mistaken for an assignment echo) and binds nothing.
- After any wasm API change, rebuild BOTH `web/pkg` and `app/src/engine/pkg`
  (the app pkg is built by deploy-pages.yml via wasm-pack `--out-dir` there;
  locally `npm run build:wasm`).
- Frontend drawable recipe (new plot kind end-to-end): engine returns a tagged
  `Expr::Func("name", …)` → wasm extractor + `EvalResult` field + `kind` string
  → app/src/engine/types.ts mirror → CellView case → painter in app/src/plot +
  autocomplete in app/src/editor/surdLang.ts. web/smoke.mjs is the tripwire.
