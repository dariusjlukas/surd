//! Engine benchmarks. Every benchmark drives the public REPL surface
//! (`Interpreter::eval_line`), so timings reflect what a user at the REPL or
//! the web app actually pays — parse + eval + canonicalization — and stay
//! valid across internal refactors.
//!
//! Usage:
//!   cargo bench                                  # run everything
//!   cargo bench -- fft                           # run matching benchmarks
//!   cargo bench -- --save-baseline main          # record a baseline
//!   cargo bench -- --baseline main               # compare against it
//!
//! `cargo bench` builds with the release profile (opt-level = "s", lto), the
//! same trade the shipping wasm bundle makes, so wins here are wins there.
//!
//! Convention: setup lines (bindings) run once on a shared interpreter; the
//! benchmarked line is a self-contained expression evaluated repeatedly on
//! that interpreter. Benchmarked lines must not rebind names the next
//! iteration depends on.

use criterion::Criterion;
use surd::Interpreter;

/// A fresh interpreter with `setup` lines already evaluated. Panics on any
/// setup error so a broken benchmark fails loudly instead of timing an error
/// path.
fn interp_with(setup: &[&str]) -> Interpreter {
    let mut interp = Interpreter::new();
    for line in setup {
        if let Err(e) = interp.eval_line(line) {
            panic!("benchmark setup failed on {line:?}: {e}");
        }
    }
    interp
}

/// Benchmark evaluating `line` on an interpreter preloaded with `setup`.
/// The result is asserted to be `Ok` once up front — a benchmark that times
/// an error message being formatted would be silently meaningless.
fn bench_line(c: &mut Criterion, name: &str, setup: &[&str], line: &str) {
    let mut interp = interp_with(setup);
    if let Err(e) = interp.eval_line(line) {
        panic!("benchmark line failed {line:?}: {e}");
    }
    c.bench_function(name, |b| {
        b.iter(|| interp.eval_line(std::hint::black_box(line)).unwrap())
    });
}

/// Deterministic pseudo-random values in (-1, 1) — a fixed LCG, so every run
/// and every machine benches identical input. (The engine's own `rng` is not
/// used: benches must not depend on engine behavior for their inputs.)
fn lcg_values(n: usize) -> Vec<f64> {
    let mut state: u64 = 0x9E3779B97F4A7C15;
    (0..n)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Top 53 bits → [0,1), then center. 6 decimal digits is plenty
            // (the literals are formatted with {:.6} below anyway).
            ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        })
        .collect()
}

/// Render values as a surd column-vector literal: `[v1; v2; …]`.
fn column(values: &[f64]) -> String {
    let body: Vec<String> = values.iter().map(|v| format!("{v:.6}")).collect();
    format!("[{}]", body.join("; "))
}

// ---------------------------------------------------------------------------
// Parse + canonicalize
// ---------------------------------------------------------------------------

fn bench_parse_eval(c: &mut Criterion) {
    bench_line(
        c,
        "parse/rational_arith",
        &[],
        "1/3 + 1/6 + 2^10 - 355/113 + 17/23 * 46/34",
    );
    bench_line(
        c,
        "parse/decimal_literals",
        &[],
        "0.1 + 0.2 + 3.14159265358979 - 2.71828182845905",
    );
    bench_line(
        c,
        "parse/surd_simplify",
        &[],
        "sqrt(2)*sqrt(2) + sqrt(8) - sqrt(18) + sqrt(50)",
    );
    bench_line(c, "parse/expand_poly", &[], "expand((x + y + 1)^12)");
    bench_line(c, "parse/diff", &[], "diff(sin(x)*exp(x^2) + x^5/ln(x), x)");

    // Parser + lexer alone, no evaluation: how much of a cheap line is
    // front-end cost.
    let src = "expand((x + y + 1)^12) + 1/3 + sqrt(50) - sin(pi/12)";
    c.bench_function("parse/lex_parse_only", |b| {
        b.iter(|| {
            let toks = surd::lexer::lex(std::hint::black_box(src)).unwrap();
            surd::parser::parse(toks).unwrap()
        })
    });

    // Printing: the canonical form must re-parse (printer invariant), and the
    // frontend renders every result — formatting cost is user-visible.
    let mut interp = Interpreter::new();
    let big = interp.eval_line("expand((x + y + 1)^12)").unwrap();
    c.bench_function("print/expanded_poly", |b| {
        b.iter(|| format!("{}", std::hint::black_box(&big)))
    });
}

// ---------------------------------------------------------------------------
// Interpreter control flow
// ---------------------------------------------------------------------------

fn bench_interpreter(c: &mut Criterion) {
    bench_line(
        c,
        "interp/recursive_fact_60",
        &["fact(n) := if n == 0 then 1 else n*fact(n-1) end"],
        "fact(60)",
    );
    bench_line(
        c,
        "interp/loop_sum_1000",
        &[],
        "s := 0; k := 1; while k <= 1000 do s := s + k; k := k + 1 end; s",
    );
}

// ---------------------------------------------------------------------------
// Numeric evaluation (N) and certified comparison
// ---------------------------------------------------------------------------

fn bench_numeric(c: &mut Criterion) {
    bench_line(c, "N/pi_1000_digits", &[], "N(pi, 1000)");
    bench_line(
        c,
        "N/transcendental_mix_100",
        &[],
        "N(sin(1) + exp(1/3) * ln(7) - cos(2/5), 100)",
    );
    bench_line(c, "N/sqrt2_10000_digits", &[], "N(sqrt(2), 10000)");

    // Certified comparisons. The easy case answers at the first precision
    // rung; the hard case is the classic near-integer e^(pi*sqrt(163)),
    // ~7.5e-13 away from an integer, forcing interval refinement.
    bench_line(c, "compare/easy_rational", &[], "355/113 < 22/7");
    bench_line(
        c,
        "compare/symbolic_first_rung",
        &[],
        "sqrt(2) + sqrt(3) < pi",
    );
    bench_line(
        c,
        "compare/near_tie_refinement",
        &[],
        "exp(pi*sqrt(163)) < 262537412640768744",
    );
}

// ---------------------------------------------------------------------------
// Algebraic numbers
// ---------------------------------------------------------------------------

fn bench_algebraic(c: &mut Criterion) {
    bench_line(
        c,
        "algebraic/construct_quintic_root",
        &[],
        "root(x^5 - x - 1, 1)",
    );
    bench_line(
        c,
        "algebraic/sum_of_radicals",
        &["a := root(x^2 - 2, 2)", "b := root(x^2 - 3, 2)"],
        "a + b",
    );
    bench_line(
        c,
        "algebraic/equality_gcd_test",
        &["a := root(x^2 - 2, 2)", "b := root(x^2 - 3, 2)"],
        "(a + b)^2 == 5 + 2*root(x^2 - 6, 2)",
    );
}

// ---------------------------------------------------------------------------
// Signals: both substrates, plus the exact symbolic DFT
// ---------------------------------------------------------------------------

fn bench_signals(c: &mut Criterion) {
    let v1024 = column(&lcg_values(1024));
    let v256 = column(&lcg_values(256));
    let v64 = column(&lcg_values(64));

    // Packing = parse the literal + decimal→exact-rational→substrate. This is
    // what every import/entry pays, so parse cost is deliberately included.
    bench_line(c, "signal/pack_f64_1024", &[], &format!("signal({v1024})"));
    bench_line(
        c,
        "signal/pack_big30_256",
        &[],
        &format!("signal({v256}, 30)"),
    );

    let f64_setup = format!("s := signal({v1024})");
    bench_line(c, "signal/fft_f64_1024", &[&f64_setup], "dsp.fft(s)");
    bench_line(c, "signal/rms_f64_1024", &[&f64_setup], "dsp.rms(s)");

    let conv_setup = format!("u := signal({v256})");
    bench_line(c, "signal/conv_f64_256", &[&conv_setup], "dsp.conv(u, u)");

    let big_setup = format!("t := signal({v64}, 30)");
    bench_line(c, "signal/fft_big30_64", &[&big_setup], "dsp.fft(t)");

    // Exact symbolic DFT over ℚ(i, √2, …): n² smart-constructor products.
    let v16: Vec<String> = (1..=16).map(|k| format!("{k}/{}", k + 1)).collect();
    bench_line(
        c,
        "signal/exact_dft_16",
        &[],
        &format!("dsp.dft([{}])", v16.join("; ")),
    );

    bench_line(c, "signal/window_hann_256", &[], "dsp.hann(256)");
}

// ---------------------------------------------------------------------------
// Matrices and stats (exact rational linear algebra)
// ---------------------------------------------------------------------------

fn bench_matrix_stats(c: &mut Criterion) {
    // 8×8 Hilbert matrix: dense exact-rational elimination with coefficient
    // growth — the honest worst case for exact linear algebra.
    let hilbert: Vec<String> = (1..=8)
        .map(|i| {
            (1..=8)
                .map(|j| format!("1/{}", i + j - 1))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect();
    let h = format!("[{}]", hilbert.join("; "));
    bench_line(c, "matrix/inv_hilbert_8", &[&format!("H := {h}")], "inv(H)");
    bench_line(c, "matrix/det_hilbert_8", &[&format!("H := {h}")], "det(H)");

    // Regression on 100 exact points (decimals → rationals → normal equations).
    let xs = column(&lcg_values(100));
    let ys = column(&lcg_values(100));
    bench_line(
        c,
        "stats/linfit_100",
        &[&format!("xv := {xs}"), &format!("yv := {ys}")],
        "stats.linfit(xv, yv)",
    );
    bench_line(
        c,
        "stats/ttest_30",
        &[&format!("m := {}", column(&lcg_values(30)))],
        "stats.ttest(m, 0)",
    );
}

// ---------------------------------------------------------------------------
// Filter design
// ---------------------------------------------------------------------------

fn bench_filters(c: &mut Criterion) {
    bench_line(
        c,
        "filter/remez_15",
        &[],
        "dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0])",
    );
    bench_line(c, "filter/butter_4", &[], "dsp.butter(4, 2/5*pi)");
    bench_line(c, "filter/firlow_31", &[], "dsp.firlow(31, pi/2)");
}

fn run_benches() {
    let mut c = Criterion::default()
        .configure_from_args()
        // The suite spans ~µs parses to multi-second exact algebra; the
        // minimum sample count and a short warm-up keep the whole suite in
        // the minutes range so it actually gets run. For a high-confidence
        // read on one benchmark, override from the CLI:
        //   cargo bench -- <filter> --sample-size 100
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(1));
    bench_parse_eval(&mut c);
    bench_interpreter(&mut c);
    bench_numeric(&mut c);
    bench_algebraic(&mut c);
    bench_signals(&mut c);
    bench_matrix_stats(&mut c);
    bench_filters(&mut c);
    c.final_summary();
}

fn main() {
    // Same large-stack wrapper the REPL and tests use: deep recursion must hit
    // the engine's depth guards, not the OS stack.
    surd::run_with_stack(run_benches);
}
