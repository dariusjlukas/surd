//! Property-based tests: algebraic invariants that must hold for *every* input,
//! plus a differential check of exact-then-`N` against an independent f64
//! oracle (which catches precedence / sign / canonicalization bugs wholesale).

mod common;
use common::*;
use proptest::prelude::*;

use num_bigint::BigInt;
use num_traits::{FromPrimitive, Signed};
use surd::expr::{float_to_rational, rat_to_expr, BigRational, Expr};
use surd::signal::{self, SignalData};

// ---------------------------------------------------------------------------
// A closed (variable-free) expression with a parallel f64 semantics, for the
// differential numeric test.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum G {
    Num(i64),
    Pi,
    E,
    Add(Box<G>, Box<G>),
    Sub(Box<G>, Box<G>),
    Mul(Box<G>, Box<G>),
    Neg(Box<G>),
    Pow(Box<G>, u32),
    Sin(Box<G>),
    Cos(Box<G>),
    /// tan(sin(a)): |sin| ≤ 1 < π/2 keeps the argument pole-free by
    /// construction, while still driving tan's sin/cos-quotient kernel.
    TanSin(Box<G>),
    /// a / (b² + 1): always defined, drives recip/division kernels.
    DivSafe(Box<G>, Box<G>),
    /// sqrt(abs(a)): drives abs and the half-integer power path.
    SqrtAbs(Box<G>),
    /// exp(sin(a)): bounded argument — overflow behavior is not what this
    /// differential test measures.
    ExpSin(Box<G>),
    /// ln(1 + abs(a)): domain-safe logarithm.
    Ln1p(Box<G>),
}

fn arb_g() -> impl Strategy<Value = G> {
    let leaf = prop_oneof![
        4 => (-5i64..6).prop_map(G::Num),
        1 => Just(G::Pi),
        1 => Just(G::E),
    ];
    leaf.prop_recursive(4, 24, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Add(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Sub(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Mul(Box::new(a), Box::new(b))),
            inner.clone().prop_map(|a| G::Neg(Box::new(a))),
            (inner.clone(), 0u32..4).prop_map(|(a, n)| G::Pow(Box::new(a), n)),
            inner.clone().prop_map(|a| G::Sin(Box::new(a))),
            inner.clone().prop_map(|a| G::Cos(Box::new(a))),
            inner.clone().prop_map(|a| G::TanSin(Box::new(a))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::DivSafe(Box::new(a), Box::new(b))),
            inner.clone().prop_map(|a| G::SqrtAbs(Box::new(a))),
            inner.clone().prop_map(|a| G::ExpSin(Box::new(a))),
            inner.prop_map(|a| G::Ln1p(Box::new(a))),
        ]
    })
}

fn render_g(g: &G) -> String {
    match g {
        G::Num(n) => format!("({})", n),
        G::Add(a, b) => format!("({} + {})", render_g(a), render_g(b)),
        G::Sub(a, b) => format!("({} - {})", render_g(a), render_g(b)),
        G::Mul(a, b) => format!("({} * {})", render_g(a), render_g(b)),
        G::Neg(a) => format!("(-{})", render_g(a)),
        G::Pow(a, n) => format!("({})^{}", render_g(a), n),
        G::Sin(a) => format!("sin({})", render_g(a)),
        G::Cos(a) => format!("cos({})", render_g(a)),
        G::Pi => "pi".to_string(),
        G::E => "e".to_string(),
        G::TanSin(a) => format!("tan(sin({}))", render_g(a)),
        G::DivSafe(a, b) => format!("({} / (({})^2 + 1))", render_g(a), render_g(b)),
        G::SqrtAbs(a) => format!("sqrt(abs({}))", render_g(a)),
        G::ExpSin(a) => format!("exp(sin({}))", render_g(a)),
        G::Ln1p(a) => format!("ln(1 + abs({}))", render_g(a)),
    }
}

fn contains_sqrt(g: &G) -> bool {
    match g {
        G::SqrtAbs(_) => true,
        G::Num(_) | G::Pi | G::E => false,
        G::Add(a, b) | G::Sub(a, b) | G::Mul(a, b) | G::DivSafe(a, b) => {
            contains_sqrt(a) || contains_sqrt(b)
        }
        G::Neg(a)
        | G::Pow(a, _)
        | G::Sin(a)
        | G::Cos(a)
        | G::TanSin(a)
        | G::ExpSin(a)
        | G::Ln1p(a) => contains_sqrt(a),
    }
}

fn eval_f64(g: &G) -> f64 {
    match g {
        G::Num(n) => *n as f64,
        G::Add(a, b) => eval_f64(a) + eval_f64(b),
        G::Sub(a, b) => eval_f64(a) - eval_f64(b),
        G::Mul(a, b) => eval_f64(a) * eval_f64(b),
        G::Neg(a) => -eval_f64(a),
        G::Pow(a, n) => eval_f64(a).powi(*n as i32),
        G::Sin(a) => eval_f64(a).sin(),
        G::Cos(a) => eval_f64(a).cos(),
        G::Pi => std::f64::consts::PI,
        G::E => std::f64::consts::E,
        G::TanSin(a) => eval_f64(a).sin().tan(),
        G::DivSafe(a, b) => {
            let d = eval_f64(b);
            eval_f64(a) / (d * d + 1.0)
        }
        G::SqrtAbs(a) => eval_f64(a).abs().sqrt(),
        G::ExpSin(a) => eval_f64(a).sin().exp(),
        G::Ln1p(a) => (1.0 + eval_f64(a).abs()).ln(),
    }
}

// ---------------------------------------------------------------------------
// A symbolic expression with variables, for the structural-algebra invariants.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum S {
    Num(i64),
    Var(char),
    Add(Box<S>, Box<S>),
    Sub(Box<S>, Box<S>),
    Mul(Box<S>, Box<S>),
    Neg(Box<S>),
    Pow(Box<S>, u32),
}

fn arb_s() -> impl Strategy<Value = S> {
    let leaf = prop_oneof![
        (-4i64..5).prop_map(S::Num),
        prop_oneof![Just('x'), Just('y'), Just('z')].prop_map(S::Var),
    ];
    leaf.prop_recursive(3, 16, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| S::Add(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| S::Sub(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| S::Mul(Box::new(a), Box::new(b))),
            inner.clone().prop_map(|a| S::Neg(Box::new(a))),
            (inner.clone(), 0u32..4).prop_map(|(a, n)| S::Pow(Box::new(a), n)),
        ]
    })
}

fn render_s(s: &S) -> String {
    match s {
        S::Num(n) => format!("({})", n),
        S::Var(c) => c.to_string(),
        S::Add(a, b) => format!("({} + {})", render_s(a), render_s(b)),
        S::Sub(a, b) => format!("({} - {})", render_s(a), render_s(b)),
        S::Mul(a, b) => format!("({} * {})", render_s(a), render_s(b)),
        S::Neg(a) => format!("(-{})", render_s(a)),
        S::Pow(a, n) => format!("({})^{}", render_s(a), n),
    }
}

proptest! {
    // The headline test: exact arithmetic then N(...) must agree with a naive
    // f64 evaluation. Validates lexing, parser precedence, canonicalization, and
    // numeric evaluation all at once.
    #[test]
    fn numeric_agrees_with_f64_oracle(g in arb_g()) {
        let oracle = eval_f64(&g);
        prop_assume!(oracle.is_finite() && oracle.abs() < 1e12);
        let out = ev(&format!("N(({}), 30)", render_g(&g)));
        prop_assume!(!is_err(&out));
        let got: f64 = match out.parse() { Ok(v) => v, Err(_) => return Ok(()) };
        prop_assume!(got.is_finite());
        // sqrt amplifies the ORACLE's own rounding noise: ε residue under a
        // root becomes √ε (sqrt(abs(tan(sin(pi)))) is exactly 0 in the
        // engine but ~1e-8 in f64). Trees containing a root get the wider
        // tolerance; everything else keeps the tight one.
        let tol = if contains_sqrt(&g) {
            1e-6 * (1.0 + oracle.abs())
        } else {
            1e-9 * (1.0 + oracle.abs())
        };
        prop_assert!(
            (got - oracle).abs() <= tol,
            "{} => engine {} vs oracle {}", render_g(&g), got, oracle
        );
    }

    // Ring axioms: the canonical form is order-independent, so equivalent
    // expressions render identically.
    #[test]
    fn addition_is_commutative(a in arb_s(), b in arb_s()) {
        prop_assert_eq!(ev(&format!("({}) + ({})", render_s(&a), render_s(&b))),
                        ev(&format!("({}) + ({})", render_s(&b), render_s(&a))));
    }

    #[test]
    fn multiplication_is_commutative(a in arb_s(), b in arb_s()) {
        prop_assert_eq!(ev(&format!("({}) * ({})", render_s(&a), render_s(&b))),
                        ev(&format!("({}) * ({})", render_s(&b), render_s(&a))));
    }

    #[test]
    fn addition_is_associative(a in arb_s(), b in arb_s(), c in arb_s()) {
        let (x, y, z) = (render_s(&a), render_s(&b), render_s(&c));
        prop_assert_eq!(ev(&format!("(({}) + ({})) + ({})", x, y, z)),
                        ev(&format!("({}) + (({}) + ({}))", x, y, z)));
    }

    #[test]
    fn distributivity_holds_after_expand(a in arb_s(), b in arb_s(), c in arb_s()) {
        let (x, y, z) = (render_s(&a), render_s(&b), render_s(&c));
        prop_assert_eq!(ev(&format!("expand(({}) * (({}) + ({})))", x, y, z)),
                        ev(&format!("expand(({})*({}) + ({})*({}))", x, y, x, z)));
    }

    #[test]
    fn additive_inverse_and_identity(a in arb_s()) {
        let x = render_s(&a);
        prop_assert_eq!(ev(&format!("({}) - ({})", x, x)), "0");
        prop_assert_eq!(ev(&format!("({}) + 0", x)), ev(&x));
        prop_assert_eq!(ev(&format!("({}) * 1", x)), ev(&x));
        prop_assert_eq!(ev(&format!("({}) * 0", x)), "0");
    }

    // Differentiation is linear.
    #[test]
    fn differentiation_is_linear(a in arb_s(), b in arb_s()) {
        let (x, y) = (render_s(&a), render_s(&b));
        prop_assert_eq!(ev(&format!("diff(({}) + ({}), x)", x, y)),
                        ev(&format!("diff(({}), x) + diff(({}), x)", x, y)));
    }

    // Display round-trips: re-evaluating a printed result is a fixed point.
    #[test]
    fn display_reparses_to_itself(a in arb_s()) {
        let once = ev(&render_s(&a));
        prop_assume!(!is_err(&once));
        prop_assert_eq!(ev(&once), once);
    }

    // Nothing the generator produces ever panics or hangs.
    #[test]
    fn generated_expressions_never_panic(a in arb_s()) {
        let _ = ev(&render_s(&a));
        let _ = ev(&format!("N(diff(({}), x), 20)", render_s(&a)));
    }

    // dsp.idft inverts dsp.dft *exactly* — not to within epsilon — for any
    // rational vector whose size has surd-form twiddles. The round trip runs
    // through complex surd arithmetic and must land back on the input.
    #[test]
    fn dft_idft_roundtrip_is_exact(v in arb_rational_vector()) {
        let vec = render_vector(&v);
        prop_assert_eq!(
            normalized(&format!("dsp.idft(dsp.dft({}))", vec)),
            normalized(&vec)
        );
    }

    // Certified comparisons must agree with the f64 oracle whenever the gap
    // is comfortably above f64 noise. The engine only answers when enclosures
    // provably separate, so any disagreement here is a soundness bug in the
    // interval evaluator — this is the differential test for src/interval.rs.
    #[test]
    fn certified_comparison_agrees_with_f64_oracle(a in arb_g(), b in arb_g()) {
        let (va, vb) = (eval_f64(&a), eval_f64(&b));
        prop_assume!(va.is_finite() && vb.is_finite());
        prop_assume!((va - vb).abs() > 1e-6 * (1.0 + va.abs() + vb.abs()));
        let out = ev(&format!("({}) < ({})", render_g(&a), render_g(&b)));
        prop_assert_eq!(out, (va < vb).to_string());
    }

    // The convolution theorem, exactly: the frequency response of a cascade
    // is the elementwise product of the responses. At surd-table frequencies
    // everything folds to canonical exact complex numbers, so the two sides
    // must be *structurally identical* — no epsilon anywhere.
    #[test]
    fn convolution_theorem_holds_exactly(
        a in prop::collection::vec((-9i64..10, 1i64..6), 1..5),
        b in prop::collection::vec((-9i64..10, 1i64..6), 1..5),
    ) {
        let grid = "[0, pi/2, pi]";
        let cascade = normalized(&format!(
            "dsp.freqz(dsp.conv({a}, {b}), {grid})",
            a = render_vector(&a), b = render_vector(&b),
        ));
        let product = normalized(&format!(
            "dsp.freqz({a}, {grid}) .* dsp.freqz({b}, {grid})",
            a = render_vector(&a), b = render_vector(&b),
        ));
        prop_assert_eq!(cascade, product);
    }

    // Exact least squares recovers an exact polynomial *identically*:
    // sample y = p(x) on deg+2 integer points, fit degree deg, and the
    // coefficients must come back unchanged — Vandermonde conditioning is a
    // float problem, and there are no floats here.
    #[test]
    fn polyfit_recovers_exact_polynomials(
        c in prop::collection::vec((-9i64..10, 1i64..6), 1..5),
    ) {
        let deg = c.len() - 1;
        let coeffs = {
            let entries: Vec<String> = c.iter().map(|(n, d)| format!("({}/{})", n, d)).collect();
            format!("[{}]", entries.join("; "))
        };
        let xs: Vec<String> = (0..=(deg as i64 + 1)).map(|x| x.to_string()).collect();
        let grid = format!("[{}]", xs.join(", "));
        let fitted = normalized(&format!(
            "stats.polyfit({grid}, stats.polyval({coeffs}, {grid}), {deg})"
        ));
        prop_assert_eq!(fitted, normalized(&coeffs));
    }

    // Exact Remez: every design must (a) be symmetric, (b) report a
    // non-negative rational ripple, and (c) satisfy its spec at the DC and
    // Nyquist grid points as *decidable* exact comparisons — |H(0)−1| ≤ δ
    // and |H(π)| ≤ δ. These hold by the discrete-minimax construction; any
    // failure is an implementation bug.
    #[test]
    fn remez_designs_meet_their_spec_exactly(
        half_taps in 3usize..7,
        cut in 2i64..5,
    ) {
        let n = 2 * half_taps + 1;
        // Passband [0, cut/10·π], stopband [(cut+2)/10·π, π].
        let spec = format!(
            "f := dsp.remez({n}, [0, {cut}/10*pi, {hi}/10*pi, pi], [1, 0])",
            n = n, cut = cut, hi = cut + 2,
        );
        let checks = ev_all(&[
            &spec,
            "sym := f.taps[1] == f.taps[len(f.taps)] and f.taps[2] == f.taps[len(f.taps) - 1]",
            "dc := abs(dsp.freqz(f.taps, [0])[1] - 1) <= f.ripple",
            "ny := abs(dsp.freqz(f.taps, [pi])[1]) <= f.ripple",
            "pos := f.ripple >= 0",
            "sym and dc and ny and pos",
        ]);
        prop_assert_eq!(checks, "true");
    }

    // Linear convolution is commutative (it's polynomial multiplication).
    #[test]
    fn convolution_is_commutative(
        a in prop::collection::vec((-9i64..10, 1i64..6), 1..7),
        b in prop::collection::vec((-9i64..10, 1i64..6), 1..7),
    ) {
        prop_assert_eq!(
            normalized(&format!("dsp.conv({}, {})", render_vector(&a), render_vector(&b))),
            normalized(&format!("dsp.conv({}, {})", render_vector(&b), render_vector(&a)))
        );
    }
}

// ---------------------------------------------------------------------------
// Signal soundness: certified enclosures must contain the exact result
// ---------------------------------------------------------------------------

/// Exact rational convolution — an oracle independent of both the symbolic
/// dsp.conv and the interval kernel.
fn exact_conv(a: &[BigRational], b: &[BigRational]) -> Vec<BigRational> {
    let zero = BigRational::from_integer(BigInt::from(0));
    let mut out = vec![zero; a.len() + b.len() - 1];
    for (j, x) in a.iter().enumerate() {
        for (k, y) in b.iter().enumerate() {
            out[j + k] += x * y;
        }
    }
    out
}

fn rats(v: &[(i64, i64)]) -> Vec<BigRational> {
    v.iter()
        .map(|(n, d)| BigRational::new(BigInt::from(*n), BigInt::from(*d)))
        .collect()
}

fn exprs(v: &[BigRational]) -> Vec<Expr> {
    v.iter().map(|r| rat_to_expr(r.clone())).collect()
}

/// The exact rational value of one enclosure endpoint.
fn endpoint(s: &SignalData, i: usize, high: bool) -> BigRational {
    match s {
        SignalData::F64 { lo, hi } => {
            BigRational::from_f64(if high { hi[i] } else { lo[i] }).expect("finite endpoint")
        }
        SignalData::Big { lo, hi, .. } => {
            float_to_rational(if high { &hi[i] } else { &lo[i] }).expect("finite endpoint")
        }
        SignalData::Complex { .. } => unreachable!("these properties exercise real signals"),
    }
}

proptest! {
    // THE signal soundness property: convolve exactly (independent oracle)
    // and on packed signals, in both substrates; every exact coefficient
    // must lie inside its certified enclosure — compared as exact rationals,
    // no epsilons anywhere.
    #[test]
    fn signal_conv_encloses_the_exact_result(
        pairs in prop::collection::vec(
            ((-99i64..100, 1i64..10), (-99i64..100, 1i64..10)),
            1..7,
        ),
    ) {
        let (av, bv): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        let (ar, br) = (rats(&av), rats(&bv));
        let exact = exact_conv(&ar, &br);
        for digits in [None, Some(5)] {
            let sa = signal::pack(&exprs(&ar), digits).unwrap();
            let sb = signal::pack(&exprs(&br), digits).unwrap();
            let sc = signal::conv(&sa, &sb).unwrap();
            for (i, want) in exact.iter().enumerate() {
                prop_assert!(
                    endpoint(&sc, i, false) <= *want && *want <= endpoint(&sc, i, true),
                    "sample {} of {:?}-digit conv: {} not in enclosure",
                    i, digits, want
                );
            }
        }
    }

    // Elementwise ops keep the same contract.
    #[test]
    fn signal_elementwise_encloses_the_exact_result(
        pairs in prop::collection::vec(
            ((-99i64..100, 1i64..10), (-99i64..100, 1i64..10)),
            1..9,
        ),
    ) {
        let (av, bv): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        let (ar, br) = (rats(&av), rats(&bv));
        for digits in [None, Some(5)] {
            let sa = signal::pack(&exprs(&ar), digits).unwrap();
            let sb = signal::pack(&exprs(&br), digits).unwrap();
            let prod = signal::binop("*", &sa, &sb).unwrap();
            let sum = signal::binop("+", &sa, &sb).unwrap();
            for i in 0..ar.len() {
                let p = &ar[i] * &br[i];
                let s = &ar[i] + &br[i];
                prop_assert!(endpoint(&prod, i, false) <= p && p <= endpoint(&prod, i, true));
                prop_assert!(endpoint(&sum, i, false) <= s && s <= endpoint(&sum, i, true));
            }
        }
    }

    // FFT round trip: ifft(fft(s)) must enclose the original exact samples.
    #[test]
    fn signal_fft_roundtrip_encloses_the_input(
        v in prop::collection::vec((-99i64..100, 1i64..10), 1..5),
    ) {
        // Pad with zeros to the next power of two.
        let mut vr = rats(&v);
        while !vr.len().is_power_of_two() {
            vr.push(BigRational::from_integer(BigInt::from(0)));
        }
        for digits in [None, Some(8)] {
            let s = signal::pack(&exprs(&vr), digits).unwrap();
            let (fre, fim) = signal::fft(&s, None, false).unwrap();
            let (rre, _rim) = signal::fft(&fre, Some(&fim), true).unwrap();
            for (i, want) in vr.iter().enumerate() {
                prop_assert!(
                    endpoint(&rre, i, false) <= *want && *want <= endpoint(&rre, i, true),
                    "fft roundtrip sample {}: {} escaped its enclosure", i, want
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// dsp helpers
// ---------------------------------------------------------------------------

/// A rational column vector whose length has exact (surd-form) DFT twiddles.
/// Size 5 exercises the pentagonal (golden-ratio) grid and the radical
/// merging + square-factor extraction the round trip depends on.
fn arb_rational_vector() -> impl Strategy<Value = Vec<(i64, i64)>> {
    prop_oneof![
        Just(1usize),
        Just(2),
        Just(3),
        Just(4),
        Just(5),
        Just(6),
        Just(8)
    ]
    .prop_flat_map(|n| prop::collection::vec((-9i64..10, 1i64..6), n))
}

/// Rendered as a row vector: a 1-element vector is a 1×1 matrix, which the
/// dsp functions classify as a row, and output orientation follows the first
/// argument — rows keep the orientation uniform across operand orders.
fn render_vector(v: &[(i64, i64)]) -> String {
    let entries: Vec<String> = v.iter().map(|(n, d)| format!("({}/{})", n, d)).collect();
    format!("[{}]", entries.join(", "))
}

/// Evaluate and collapse whitespace, so multi-line matrices compare cleanly.
fn normalized(src: &str) -> String {
    ev(src).split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Signal soundness, round 2 (post-audit): the kernels the original properties
// never reached — transcendentals, sub/div, complex kernels, and the forward
// FFT against an exact DFT oracle. The confirmed containment breaks (point
// twiddles, cancelled sin widths, subnormal readbacks) all lived here.
// ---------------------------------------------------------------------------

/// High-precision certified oracle from the *independent* interval evaluator
/// (512 bits — its enclosure is ~2^-500 wide, hundreds of orders of magnitude
/// tighter than any signal-kernel width). Containment is asserted via the
/// sufficient condition: kernel_lo ≤ oracle_lo && oracle_hi ≤ kernel_hi.
fn oracle512(ex: &Expr) -> (BigRational, BigRational) {
    surd::interval::rational_enclosure(ex, 512).expect("oracle must evaluate")
}

/// π to 20 decimal digits, as an exact rational — a cancellation hotspot for
/// sin (sin(π+δ) ≈ −δ, comparable to the enclosure width itself).
fn pi_ish() -> BigRational {
    BigRational::new(
        "314159265358979323846".parse().unwrap(),
        BigInt::from(10u64).pow(20).into(),
    )
}

proptest! {
    // Unary transcendental kernels, both substrates, against the 512-bit
    // oracle — including samples parked right next to π where the Lipschitz
    // width and the function value cancel.
    #[test]
    fn signal_transcendentals_enclose_the_true_value(
        v in prop::collection::vec((-400i64..400, 1i64..100), 1..6),
        near_pi in any::<bool>(),
    ) {
        let mut vr = rats(&v);
        if near_pi {
            let scale = BigRational::from_integer(BigInt::from(10u64).pow(15).into());
            vr = vr.iter().map(|r| pi_ish() + r / &scale).collect();
        }
        // Strictly positive twins for the ln / sqrt domains.
        let seventh = BigRational::new(BigInt::from(1), BigInt::from(7));
        let vp: Vec<BigRational> = vr.iter().map(|r| r.abs() + &seventh).collect();
        for digits in [None, Some(8)] {
            let s_any = signal::pack(&exprs(&vr), digits).unwrap();
            let s_pos = signal::pack(&exprs(&vp), digits).unwrap();
            for (name, s, inputs) in [
                ("sin", &s_any, &vr),
                ("cos", &s_any, &vr),
                ("exp", &s_any, &vr),
                ("abs", &s_any, &vr),
                ("ln", &s_pos, &vp),
                ("sqrt", &s_pos, &vp),
            ] {
                let out = signal::unary(name, s).unwrap();
                for (i, r) in inputs.iter().enumerate() {
                    let arg = rat_to_expr(r.clone());
                    let ex = if name == "sqrt" {
                        surd::expr::pow(arg, rat_to_expr(BigRational::new(1.into(), 2.into())))
                    } else {
                        surd::expr::func(name, vec![arg])
                    };
                    let (olo, ohi) = oracle512(&ex);
                    prop_assert!(
                        endpoint(&out, i, false) <= olo && ohi <= endpoint(&out, i, true),
                        "{}({}) escaped its {:?}-digit enclosure (sample {})",
                        name, r, digits, i
                    );
                }
            }
        }
    }

    // Subtraction and division — the two elementwise kernels the original
    // property never generated. Divisors are kept away from zero (division
    // by a zero-straddling interval refuses by design; that path is pinned
    // by an eval test).
    #[test]
    fn signal_sub_div_enclose_the_exact_result(
        pairs in prop::collection::vec(
            ((-99i64..100, 1i64..10), (1i64..100, 1i64..10), any::<bool>()),
            1..9,
        ),
    ) {
        let (av, bsigned): (Vec<_>, Vec<_>) =
            pairs.into_iter().map(|(a, b, neg)| (a, (b, neg))).unzip();
        let ar = rats(&av);
        let br: Vec<BigRational> = bsigned
            .iter()
            .map(|((n, d), neg)| {
                let r = BigRational::new(BigInt::from(*n), BigInt::from(*d));
                if *neg { -r } else { r }
            })
            .collect();
        for digits in [None, Some(5)] {
            let sa = signal::pack(&exprs(&ar), digits).unwrap();
            let sb = signal::pack(&exprs(&br), digits).unwrap();
            let dif = signal::binop("-", &sa, &sb).unwrap();
            let quo = signal::binop("/", &sa, &sb).unwrap();
            for i in 0..ar.len() {
                let d = &ar[i] - &br[i];
                let q = &ar[i] / &br[i];
                prop_assert!(endpoint(&dif, i, false) <= d && d <= endpoint(&dif, i, true));
                prop_assert!(endpoint(&quo, i, false) <= q && q <= endpoint(&quo, i, true));
            }
        }
    }

    // Complex kernels (cmul / cdiv / cmag) against exact complex rational
    // arithmetic. The old complex checks went through dsp.peak — computed
    // from the same enclosures under test, hence blind to containment.
    #[test]
    fn complex_kernels_enclose_the_exact_result(
        quads in prop::collection::vec(
            (
                (-49i64..50, 1i64..8),
                (-49i64..50, 1i64..8),
                (1i64..50, 1i64..8),
                (-49i64..50, 1i64..8),
            ),
            1..6,
        ),
    ) {
        // z1 = a + bi arbitrary; z2 = c + di with c ≥ 1/7 so |z2|² is
        // bounded away from zero and cdiv cannot refuse.
        let (mut ar, mut br, mut cr, mut dr) = (vec![], vec![], vec![], vec![]);
        for (a, b, c, d) in &quads {
            ar.push(BigRational::new(BigInt::from(a.0), BigInt::from(a.1)));
            br.push(BigRational::new(BigInt::from(b.0), BigInt::from(b.1)));
            cr.push(BigRational::new(BigInt::from(c.0), BigInt::from(c.1)));
            dr.push(BigRational::new(BigInt::from(d.0), BigInt::from(d.1)));
        }
        for digits in [None, Some(6)] {
            let z1 = signal::complex(
                signal::pack(&exprs(&ar), digits).unwrap(),
                signal::pack(&exprs(&br), digits).unwrap(),
            )
            .unwrap();
            let z2 = signal::complex(
                signal::pack(&exprs(&cr), digits).unwrap(),
                signal::pack(&exprs(&dr), digits).unwrap(),
            )
            .unwrap();
            let contains = |s: &SignalData, i: usize, want: &BigRational| {
                endpoint(s, i, false) <= *want && *want <= endpoint(s, i, true)
            };
            let prod = signal::binop("*", &z1, &z2).unwrap();
            let quo = signal::binop("/", &z1, &z2).unwrap();
            let mag = signal::unary("abs", &z1).unwrap();
            let (pre, pim) = (signal::re_part(&prod), signal::im_part(&prod));
            let (qre, qim) = (signal::re_part(&quo), signal::im_part(&quo));
            for i in 0..ar.len() {
                let (a, b, c, d) = (&ar[i], &br[i], &cr[i], &dr[i]);
                // (a+bi)(c+di) = (ac−bd) + (ad+bc)i
                prop_assert!(contains(&pre, i, &(a * c - b * d)));
                prop_assert!(contains(&pim, i, &(a * d + b * c)));
                // (a+bi)/(c+di) = ((ac+bd) + (bc−ad)i) / (c²+d²)
                let den = c * c + d * d;
                prop_assert!(contains(&qre, i, &((a * c + b * d) / &den)));
                prop_assert!(contains(&qim, i, &((b * c - a * d) / &den)));
                // |z1| = √(a²+b²): compare squares, exactly in ℚ.
                let s2 = a * a + b * b;
                let (ml, mh) = (endpoint(&mag, i, false), endpoint(&mag, i, true));
                prop_assert!(&mh * &mh >= s2 && !mh.is_negative());
                prop_assert!(ml.is_negative() || &ml * &ml <= s2);
            }
        }
    }

    // Forward FFT against the exact DFT: at n = 4 every twiddle is 0 or ±1,
    // so X[k] = Σ xⱼ·(−i)^(jk) is exact rational arithmetic. The roundtrip
    // property cannot see a wrong-but-invertible transform (and cancels
    // twiddle bias) — this can. Bin 1 of [0,1,0,−1] was a confirmed break.
    #[test]
    fn forward_fft_encloses_the_exact_dft(
        v in prop::collection::vec((-99i64..100, 1i64..10), 4..=4),
    ) {
        let vr = rats(&v);
        // (−i)^m: re/im lookup for m mod 4 → (1,0), (0,−1), (−1,0), (0,1)
        let tw_re = [1i64, 0, -1, 0];
        let tw_im = [0i64, -1, 0, 1];
        for digits in [None, Some(8)] {
            let s = signal::pack(&exprs(&vr), digits).unwrap();
            let (fre, fim) = signal::fft(&s, None, false).unwrap();
            for k in 0..4 {
                let mut xre = BigRational::from_integer(BigInt::from(0));
                let mut xim = BigRational::from_integer(BigInt::from(0));
                for (j, x) in vr.iter().enumerate() {
                    let m = (j * k) % 4;
                    xre += x * BigRational::from_integer(BigInt::from(tw_re[m]));
                    xim += x * BigRational::from_integer(BigInt::from(tw_im[m]));
                }
                prop_assert!(
                    endpoint(&fre, k, false) <= xre && xre <= endpoint(&fre, k, true),
                    "re X[{}] = {} escaped", k, xre
                );
                prop_assert!(
                    endpoint(&fim, k, false) <= xim && xim <= endpoint(&fim, k, true),
                    "im X[{}] = {} escaped", k, xim
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Real algebraic numbers: exact-root recovery and radical identities
// ---------------------------------------------------------------------------

proptest! {
    // ∏(x − rᵢ) has exactly the rᵢ as roots: root(p, i) must equal the i-th
    // smallest, EXACTLY (decided by the algebraic engine through `==`).
    #[test]
    fn root_recovers_factored_polynomial_roots(
        roots in prop::collection::btree_set(-20i64..21, 1..4),
    ) {
        let rs: Vec<i64> = roots.into_iter().collect(); // sorted, distinct
        let poly = rs
            .iter()
            .map(|r| format!("(x - ({r}))"))
            .collect::<Vec<_>>()
            .join("*");
        for (i, r) in rs.iter().enumerate() {
            prop_assert_eq!(
                ev(&format!("root(expand({poly}), {}) == {r}", i + 1)),
                "true",
                "root {} of {} should be {}", i + 1, poly, r
            );
        }
    }

    // (√a+√b)² = a + b + 2√(ab) for positive integers — structurally
    // different once ab has square factors, so this exercises the exact
    // algebraic equality path, not canonicalization.
    #[test]
    fn radical_square_identity_is_decided_exactly(a in 2i64..30, b in 2i64..30) {
        prop_assert_eq!(
            ev(&format!("(sqrt({a})+sqrt({b}))^2 == {a} + {b} + 2*sqrt({})", a * b)),
            "true"
        );
        // And the strict inequality against a perturbed rhs is false-free:
        prop_assert_eq!(
            ev(&format!("(sqrt({a})+sqrt({b}))^2 < {a} + {b} + 2*sqrt({})", a * b)),
            "false"
        );
    }
}

// ---------------------------------------------------------------------------
// Certified IIR: Schur–Cohn agrees with the closed-form biquad triangle
// ---------------------------------------------------------------------------

proptest! {
    // Monic z² + a1·z + a2 is strictly stable iff |a2| < 1 and |a1| < 1 + a2
    // (the stability triangle) — an independent closed form to test the
    // step-down recursion against, exactly, over rationals.
    #[test]
    fn schur_cohn_agrees_with_the_stability_triangle(
        a1 in (-40i64..41, 1i64..8),
        a2 in (-40i64..41, 1i64..8),
    ) {
        let a1 = BigRational::new(BigInt::from(a1.0), BigInt::from(a1.1));
        let a2 = BigRational::new(BigInt::from(a2.0), BigInt::from(a2.1));
        let one = BigRational::from_integer(BigInt::from(1));
        let triangle = a2.clone().abs() < one && a1.clone().abs() < &one + &a2;
        // Skip exact boundary cases: dsp.stable answers false there (not
        // strictly stable), while a strict triangle can't distinguish
        // |a2| == 1 from |a1| == 1 + a2; both mean "false" anyway.
        let got = ev(&format!(
            "dsp.stable([1, {}/{}, {}/{}])",
            a1.numer(), a1.denom(), a2.numer(), a2.denom()
        ));
        prop_assert_eq!(got, if triangle { "true" } else { "false" });
    }
}

// ---------------------------------------------------------------------------
// astro-float directed rounding, pinned for EVERY operation the certified
// evaluators use (the old pin covered div and π only). Each op at 64 bits
// must bracket a 512-bit nearest reference: Down ≤ ref ≤ Up.
// ---------------------------------------------------------------------------

mod astro_pin {
    use super::*;
    use astro_float::{BigFloat, Consts, Radix, RoundingMode};

    const UP: RoundingMode = RoundingMode::Up;
    const DOWN: RoundingMode = RoundingMode::Down;
    const NEAREST: RoundingMode = RoundingMode::ToEven;

    /// a ≤ b, by the sign of the difference — astro-float's `PartialOrd`
    /// is unreliable near zero, so no test may use raw `<=` either.
    fn bf_le(a: &BigFloat, b: &BigFloat) -> bool {
        let d = b.sub(a, 1024, NEAREST);
        !(d.is_negative() && !d.is_zero())
    }

    fn from_ratio(n: i64, d: i64, p: usize, rm: RoundingMode, cc: &mut Consts) -> BigFloat {
        let nn = BigFloat::parse(&n.to_string(), Radix::Dec, p, NEAREST, cc);
        let dd = BigFloat::parse(&d.to_string(), Radix::Dec, p, NEAREST, cc);
        nn.div(&dd, p, rm)
    }

    proptest! {
        #[test]
        fn directed_rounding_brackets_a_high_precision_reference(
            an in -9999i64..10000, ad in 1i64..1000,
            bn in -9999i64..10000, bd in 1i64..1000,
            op in 0usize..9,
        ) {
            let mut cc = Consts::new().unwrap();
            // Exact 512-bit operands (i64 ratios are exact at 512 bits far
            // beyond these magnitudes), then each op Down/Up at 64 bits vs
            // nearest at 512.
            let a64 = from_ratio(an, ad, 64, NEAREST, &mut cc);
            let b64 = from_ratio(bn, bd, 64, NEAREST, &mut cc);
            let a512 = from_ratio(an, ad, 512, NEAREST, &mut cc);
            let b512 = from_ratio(bn, bd, 512, NEAREST, &mut cc);
            // A BigFloat VALUE is exact regardless of the precision an op
            // is asked to produce, so the 64-bit operands feed both sides:
            // the op under test at 64 bits Down/Up, the reference at 512
            // bits nearest — same exact inputs, no truncation mismatch.
            let _ = (a512, b512);
            let (a, b) = (a64.clone(), b64.clone());
            let (a_hi, b_hi) = (a.clone(), b.clone());
            let (lo, hi, reference) = match op {
                0 => (a.add(&b, 64, DOWN), a.add(&b, 64, UP), a_hi.add(&b_hi, 512, NEAREST)),
                1 => (a.sub(&b, 64, DOWN), a.sub(&b, 64, UP), a_hi.sub(&b_hi, 512, NEAREST)),
                2 => (a.mul(&b, 64, DOWN), a.mul(&b, 64, UP), a_hi.mul(&b_hi, 512, NEAREST)),
                3 => {
                    prop_assume!(bn != 0);
                    (a.div(&b, 64, DOWN), a.div(&b, 64, UP), a_hi.div(&b_hi, 512, NEAREST))
                }
                4 => {
                    let (aa, ah) = (a.abs(), a_hi.abs());
                    (aa.sqrt(64, DOWN), aa.sqrt(64, UP), ah.sqrt(512, NEAREST))
                }
                5 => {
                    // exp on a damped argument (avoid the known underflow
                    // flush, pinned separately below).
                    prop_assume!(an.abs() < 50 * ad);
                    (a.exp(64, DOWN, &mut cc), a.exp(64, UP, &mut cc), a_hi.exp(512, NEAREST, &mut cc))
                }
                6 => {
                    prop_assume!(an > 0);
                    (a.ln(64, DOWN, &mut cc), a.ln(64, UP, &mut cc), a_hi.ln(512, NEAREST, &mut cc))
                }
                7 => (a.sin(64, DOWN, &mut cc), a.sin(64, UP, &mut cc), a_hi.sin(512, NEAREST, &mut cc)),
                _ => (a.cos(64, DOWN, &mut cc), a.cos(64, UP, &mut cc), a_hi.cos(512, NEAREST, &mut cc)),
            };
            prop_assert!(!lo.is_nan() && !hi.is_nan() && !reference.is_nan());
            prop_assert!(bf_le(&lo, &reference), "Down endpoint above the reference (op {op})");
            prop_assert!(bf_le(&reference, &hi), "reference above the Up endpoint (op {op})");
        }
    }

    /// Canaries for the two upstream astro-float bugs the engine guards
    /// against. If astro ever FIXES them, these fail — the cue to simplify
    /// the guards (interval::exp_iv, expr::bf_from_f64_exact).
    #[test]
    fn upstream_bug_canaries_still_present() {
        let mut cc = Consts::new().unwrap();
        // 1. exp underflow flushes to exact +0 even rounding Up.
        let x = BigFloat::parse("-2200000000", Radix::Dec, 64, NEAREST, &mut cc);
        assert!(
            x.exp(64, RoundingMode::Up, &mut cc).is_zero(),
            "astro-float exp underflow flush is FIXED upstream — simplify interval::exp_iv"
        );
        // 2. from_f64 halves subnormals: f64::from_bits(2) is 2⁻¹⁰⁷³, but
        // it comes back stored as 0.5·2⁻¹⁰⁷³ = 2⁻¹⁰⁷⁴ (exponent −1073 with
        // a normalized mantissa; a correct conversion would carry −1072).
        let tiny = BigFloat::from_f64(f64::from_bits(2), 64);
        assert_eq!(
            tiny.exponent(),
            Some(-1073),
            "astro-float from_f64 subnormal halving is FIXED upstream — simplify bf_from_f64_exact"
        );
    }
}

// ---------------------------------------------------------------------------
// D10: containment at the f64 floor — subnormal and near-overflow samples,
// where two audit-confirmed bugs lived and no generator ever went.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn signal_ops_contain_exactly_at_extreme_magnitudes(
        pairs in prop::collection::vec(
            ((1i64..1000, 0u32..3), (1i64..1000, 0u32..3), any::<bool>(), any::<bool>()),
            1..5,
        ),
        scale_pow in prop::sample::select(vec![-1074i64, -1060, -1030, -520, 0, 500, 1000]),
    ) {
        // Values m·2^scale ± tiny offsets: subnormal region (−1074..−1023),
        // deep-normal, and near-overflow. Exact rational oracle throughout.
        let two = BigRational::from_integer(BigInt::from(2));
        let scale = if scale_pow >= 0 {
            BigRational::from_integer(BigInt::from(1) << (scale_pow as usize))
        } else {
            BigRational::new(BigInt::from(1), BigInt::from(1) << ((-scale_pow) as usize))
        };
        let (mut ar, mut br) = (vec![], vec![]);
        for ((am, ash), (bm, bsh), aneg, bneg) in &pairs {
            let mut a = BigRational::from_integer(BigInt::from(*am)) * &scale
                * num_traits::pow::pow(two.clone(), *ash as usize);
            let mut b = BigRational::from_integer(BigInt::from(*bm)) * &scale
                * num_traits::pow::pow(two.clone(), *bsh as usize);
            if *aneg { a = -a; }
            if *bneg { b = -b; }
            ar.push(a);
            br.push(b);
        }
        // f64 substrate only: the Big substrate has no subnormal regime.
        let sa = match signal::pack(&exprs(&ar), None) { Ok(s) => s, Err(_) => return Ok(()) };
        let sb = match signal::pack(&exprs(&br), None) { Ok(s) => s, Err(_) => return Ok(()) };
        for op in ["+", "-", "*"] {
            // Products at 2^1000 scales overflow: a loud error is sound,
            // a wrong enclosure is not.
            let out = match signal::binop(op, &sa, &sb) { Ok(o) => o, Err(_) => continue };
            for i in 0..ar.len() {
                let want = match op {
                    "+" => &ar[i] + &br[i],
                    "-" => &ar[i] - &br[i],
                    _ => &ar[i] * &br[i],
                };
                prop_assert!(
                    endpoint(&out, i, false) <= want && want <= endpoint(&out, i, true),
                    "{} at scale 2^{}: sample {} escaped", op, scale_pow, i
                );
            }
        }
        // The certified bound must cover |mid − true| even down here (the
        // audit's subnormal bound()-lie regression, as a property).
        for (i, a) in ar.iter().enumerate() {
            let mid = surd::expr::numeric_value(&signal::midpoint(&sa, i))
                .or_else(|| match signal::midpoint(&sa, i) {
                    surd::expr::Expr::Float(bf, _) => float_to_rational(&bf),
                    _ => None,
                })
                .expect("midpoint is numeric");
            let bound = match signal::half_width(&sa, Some(i)) {
                surd::expr::Expr::Float(bf, _) => float_to_rational(&bf).expect("finite"),
                other => surd::expr::numeric_value(&other).expect("numeric"),
            };
            let dev = (a - &mid).abs();
            prop_assert!(dev <= bound, "bound() understates at scale 2^{}", scale_pow);
        }
    }

    // D11: certified windows must enclose the exact cosine-sum values —
    // previously pinned at a single sample of hann(4).
    #[test]
    fn window_signals_enclose_their_exact_formula(
        n in 2usize..24,
        which in 0usize..3,
    ) {
        let name = ["hann", "hamming", "blackman"][which];
        let (a0, a1, a2): ((i64, i64), (i64, i64), (i64, i64)) = match which {
            0 => ((1, 2), (1, 2), (0, 1)),
            1 => ((27, 50), (23, 50), (0, 1)),
            _ => ((21, 50), (1, 2), (2, 25)),
        };
        let s = signal::window(name, n).unwrap();
        for k in 0..n {
            // w[k] = a0 − a1·cos(2πk/(n−1)) + a2·cos(4πk/(n−1)), exactly,
            // through the independent 512-bit interval oracle.
            let rat = |(p, q): (i64, i64)| {
                rat_to_expr(BigRational::new(BigInt::from(p), BigInt::from(q)))
            };
            let angle = |mult: i64| {
                surd::expr::mul(vec![
                    rat_to_expr(BigRational::new(
                        BigInt::from(mult * k as i64),
                        BigInt::from(n as i64 - 1),
                    )),
                    surd::expr::Expr::Const(surd::expr::Constant::Pi),
                ])
            };
            let exact = surd::expr::add(vec![
                rat(a0),
                surd::expr::mul(vec![
                    rat((-a1.0, a1.1)),
                    surd::expr::func("cos", vec![angle(2)]),
                ]),
                surd::expr::mul(vec![rat(a2), surd::expr::func("cos", vec![angle(4)])]),
            ]);
            let (olo, ohi) = oracle512(&exact);
            prop_assert!(
                endpoint(&s, k, false) <= olo && ohi <= endpoint(&s, k, true),
                "{}({}) sample {} enclosure misses the exact value", name, n, k
            );
        }
    }
}
