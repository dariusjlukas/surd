//! Property-based tests: algebraic invariants that must hold for *every* input,
//! plus a differential check of exact-then-`N` against an independent f64
//! oracle (which catches precedence / sign / canonicalization bugs wholesale).

mod common;
use common::*;
use proptest::prelude::*;

use num_bigint::BigInt;
use num_traits::FromPrimitive;
use surd::expr::{float_to_rational, rat_to_expr, BigRational, Expr};
use surd::signal::{self, SignalData};

// ---------------------------------------------------------------------------
// A closed (variable-free) expression with a parallel f64 semantics, for the
// differential numeric test.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum G {
    Num(i64),
    Add(Box<G>, Box<G>),
    Sub(Box<G>, Box<G>),
    Mul(Box<G>, Box<G>),
    Neg(Box<G>),
    Pow(Box<G>, u32),
    Sin(Box<G>),
    Cos(Box<G>),
}

fn arb_g() -> impl Strategy<Value = G> {
    let leaf = (-5i64..6).prop_map(G::Num);
    leaf.prop_recursive(4, 24, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Add(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Sub(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| G::Mul(Box::new(a), Box::new(b))),
            inner.clone().prop_map(|a| G::Neg(Box::new(a))),
            (inner.clone(), 0u32..4).prop_map(|(a, n)| G::Pow(Box::new(a), n)),
            inner.clone().prop_map(|a| G::Sin(Box::new(a))),
            inner.prop_map(|a| G::Cos(Box::new(a))),
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
        let tol = 1e-9 * (1.0 + oracle.abs());
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
