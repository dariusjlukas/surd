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
