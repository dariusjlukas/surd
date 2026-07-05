//! Certified IIR filter design: exact Butterworth biquads, an exact
//! Schur–Cohn stability test, and exact recursive filtering of vectors.
//!
//! The exactness story, end to end:
//!
//! * Butterworth prototype pole angles are rational multiples of π, so the
//!   section constants σ = sin((2k−1)π/2n) are real algebraic numbers the
//!   engine already handles exactly (surds where a surd exists, symbolic
//!   otherwise, certified everywhere).
//! * The bilinear prewarp K = tan(ω/2) stays exact-symbolic for any
//!   constant cutoff, and the bilinear map itself is rational — nothing in
//!   the design ever leaves exact-land. `N(...)` produces deployable
//!   coefficients; `dsp.quantize` produces the fixed-point ones.
//! * Stability is decided by the Schur–Cohn (reflection-coefficient)
//!   step-down, which needs no complex root-finding: it is a chain of SIGN
//!   decisions on constants, each settled by certified interval refinement
//!   with the real-algebraic engine as the tie-breaker. It therefore works
//!   on the *quantized* coefficients you will actually run — "this exact
//!   f64/fixed-point filter is provably stable" is the headline feature.
//!   A pole exactly ON the unit circle refuses loudly (marginal ≠ stable).
//!
//! Deliberately absent: certified IIR filtering of bulk `signal(...)` data.
//! Naive interval arithmetic through feedback diverges — each step
//! multiplies interval widths by roughly Σ|aᵢ| ≥ 1, so enclosures explode
//! geometrically even for perfectly stable filters, and a uselessly wide
//! bound dressed up as "certified" would be worse than an honest refusal.
//! Doing it right needs a decay argument from the pole radii (‖h‖₁-style
//! error transport), which is future work. Exact vectors filter exactly.

use crate::algebraic;
use crate::expr::*;
use crate::interval;
use num_bigint::BigInt;
use num_traits::Zero;
use std::cmp::Ordering;

/// Degree cap for the general Schur–Cohn recursion (each stage is exact
/// division and two certified sign decisions; symbolic coefficients grow).
const MAX_STABLE_DEG: usize = 32;
/// Cap when the coefficients are symbolic constants (surd towers from an
/// expanded dsp.tf): the factored trees still square per stage.
const MAX_STABLE_DEG_SYMBOLIC: usize = 6;

/// Cap on filter order for design (each section is cheap, but symbolic
/// coefficient size grows with the trig table misses).
const MAX_IIR_ORDER: usize = 24;

// ---------------------------------------------------------------------------
// Certified sign of a constant expression (the compare-machinery core,
// reused outside `eval`): interval refinement first, exact algebra on ties.
// ---------------------------------------------------------------------------

fn certified_cmp_zero(e: &Expr) -> Result<Ordering, String> {
    match interval::certified_sign(e) {
        interval::Sign::Positive => Ok(Ordering::Greater),
        interval::Sign::Negative => Ok(Ordering::Less),
        interval::Sign::Zero => Ok(Ordering::Equal),
        _ => algebraic::certified_sign(e).ok_or_else(|| {
            format!(
                "cannot decide the sign of '{}' — it must be a constant real value",
                e
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// Design: dsp.butter(n, wc[, highpass])
// ---------------------------------------------------------------------------

/// `dsp.butter(n, wc)` / `dsp.butter(n, wc, highpass)`: order-n Butterworth
/// lowpass (or highpass) with cutoff `wc` (radians/sample, 0 < wc < π),
/// bilinear transform with exact prewarp K = tan(wc/2). Returns
/// struct(sos, order, kind): `sos` is a ⌈n/2⌉×6 matrix of second-order
/// sections `[b0 b1 b2 1 a1 a2]` with exact coefficients; a first-order
/// section (odd n) is padded with zeros (b2 = a2 = 0).
pub fn butter(args: Vec<Expr>) -> Result<Expr, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "dsp.butter expects butter(n, wc[, highpass]), got {} argument(s)",
            args.len()
        ));
    }
    let n = numeric_value(&args[0])
        .filter(|r| r.is_integer())
        .and_then(|r| num_traits::ToPrimitive::to_usize(&r.to_integer()))
        .filter(|&n| (1..=MAX_IIR_ORDER).contains(&n))
        .ok_or_else(|| format!("dsp.butter expects an order from 1 to {}", MAX_IIR_ORDER))?;
    let wc = args[1].clone();
    let highpass = match args.get(2) {
        None => false,
        Some(Expr::Symbol(s)) if s == "highpass" => true,
        Some(Expr::Symbol(s)) if s == "lowpass" => false,
        Some(other) => {
            return Err(format!(
                "dsp.butter's third argument selects the kind: lowpass or highpass, got '{}'",
                other
            ))
        }
    };
    // The cutoff must be a constant strictly inside (0, π) — certified.
    let in_domain = certified_cmp_zero(&wc)
        .map(|s| s == Ordering::Greater)
        .and_then(|pos| {
            let head = add(vec![
                Expr::Const(Constant::Pi),
                mul(vec![int(-1), wc.clone()]),
            ]);
            Ok(pos && certified_cmp_zero(&head)? == Ordering::Greater)
        });
    match in_domain {
        Ok(true) => {}
        Ok(false) => return Err(format!("dsp.butter's cutoff '{}' is outside (0, π)", wc)),
        Err(_) => {
            return Err(format!(
                "dsp.butter's cutoff must be a constant frequency in (0, π), got '{}'",
                wc
            ))
        }
    }
    // K = tan(wc/2), exact-symbolic (a surd when wc/2 is on the trig table).
    let k = func("tan", vec![mul(vec![rat_expr(1, 2), wc.clone()])]);
    let k2 = pow(k.clone(), int(2));
    let mut rows: Vec<Vec<Expr>> = Vec::new();
    // Conjugate-pair sections: prototype factor s² + 2σs + 1 with
    // σ = sin((2j−1)π/(2n)); bilinear (lowpass) gives
    //   a0 = 1 + 2σK + K²,  a1 = 2(K²−1),  a2 = 1 − 2σK + K²,  b = K²·[1,2,1]
    // and highpass (s ← K(1+z⁻¹)/(1−z⁻¹)) swaps the roles of 1 and K²:
    //   a0 = K² + 2σK + 1,  a1 = 2(K²−1),  a2 = K² − 2σK + 1,  b = [1,−2,1].
    // (Identical a-side; the numerator carries the kind.)
    for j in 1..=(n / 2) {
        let angle = mul(vec![
            rat_to_expr(BigRational::new(
                BigInt::from(2 * j as i64 - 1),
                BigInt::from(2 * n as i64),
            )),
            Expr::Const(Constant::Pi),
        ]);
        let sigma = func("sin", vec![angle]);
        let two_sigma_k = mul(vec![int(2), sigma, k.clone()]);
        let a0 = add(vec![int(1), two_sigma_k.clone(), k2.clone()]);
        let a1 = mul(vec![int(2), add(vec![k2.clone(), int(-1)])]);
        let a2 = add(vec![int(1), mul(vec![int(-1), two_sigma_k]), k2.clone()]);
        let inv_a0 = pow(a0, int(-1));
        let (b0, b1, b2) = if highpass {
            (
                inv_a0.clone(),
                mul(vec![int(-2), inv_a0.clone()]),
                inv_a0.clone(),
            )
        } else {
            let g = mul(vec![k2.clone(), inv_a0.clone()]);
            (g.clone(), mul(vec![int(2), g.clone()]), g)
        };
        rows.push(vec![
            b0,
            b1,
            b2,
            int(1),
            mul(vec![a1, inv_a0.clone()]),
            mul(vec![a2, inv_a0]),
        ]);
    }
    // Odd order: one real prototype pole at s = −1.
    //   lowpass:  b = K·[1, 1]/(1+K),   a1 = (K−1)/(1+K)
    //   highpass: b = [1, −1]/(1+K),    a1 = (K−1)/(1+K)
    if n % 2 == 1 {
        let a0 = add(vec![int(1), k.clone()]);
        let inv_a0 = pow(a0, int(-1));
        let a1 = mul(vec![add(vec![k.clone(), int(-1)]), inv_a0.clone()]);
        let (b0, b1) = if highpass {
            (inv_a0.clone(), mul(vec![int(-1), inv_a0.clone()]))
        } else {
            let g = mul(vec![k.clone(), inv_a0]);
            (g.clone(), g)
        };
        rows.push(vec![b0, b1, int(0), int(1), a1, int(0)]);
    }
    structure(vec![
        ("sos".to_string(), Expr::Matrix(rows)),
        ("order".to_string(), int(n as i64)),
        (
            "kind".to_string(),
            Expr::Symbol(if highpass { "highpass" } else { "lowpass" }.to_string()),
        ),
    ])
}

fn rat_expr(n: i64, d: i64) -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(n), BigInt::from(d)))
}

// ---------------------------------------------------------------------------
// Stability: dsp.stable(x) — exact Schur–Cohn.
// ---------------------------------------------------------------------------

/// `dsp.stable(x)`: certified strict stability (every pole strictly inside
/// the unit circle). Accepts a filter struct (checks each section's
/// denominator), an SOS matrix, or a denominator coefficient vector
/// `[a0, a1, …, an]` for A(z) = Σ aₖ·z^(−k). A pole exactly on the circle
/// (or one the engine cannot separate from it) errors rather than guesses.
pub fn stable(args: Vec<Expr>) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err("dsp.stable expects one argument: a filter, SOS matrix, or denominator coefficient vector".into());
    }
    for den in denominators("dsp.stable", &args[0])? {
        if !schur_cohn(&den)? {
            return Ok(Expr::Bool(false));
        }
    }
    Ok(Expr::Bool(true))
}

/// The denominator coefficient vectors ([a0, a1, …]) behind any accepted
/// filter shape.
fn denominators(name: &str, e: &Expr) -> Result<Vec<Vec<Expr>>, String> {
    match e {
        // Filter struct: read its sos matrix.
        Expr::Struct(fields) => match fields.iter().find(|(k, _)| k == "sos") {
            Some((_, sos)) => denominators(name, sos),
            None => Err(format!(
                "{} expects a filter struct with an 'sos' field",
                name
            )),
        },
        Expr::Matrix(rows) if rows.iter().all(|r| r.len() == 6) => Ok(rows
            .iter()
            .map(|r| vec![r[3].clone(), r[4].clone(), r[5].clone()])
            .collect()),
        Expr::Matrix(_) => {
            let (a, _) = crate::dsp::as_vector(name, e)?;
            if a.len() < 2 {
                return Err(format!("{} needs a denominator of degree at least 1", name));
            }
            Ok(vec![a])
        }
        _ => Err(format!(
            "{} expects a filter struct, an m×6 SOS matrix, or a coefficient vector",
            name
        )),
    }
}

/// Schur–Cohn step-down: A(z) = Σ aₖ z^(−k) has all roots strictly inside
/// the unit circle iff every reflection coefficient kₘ = αₘ of the
/// normalized (α₀ = 1) polynomial satisfies |kₘ| < 1, where
/// A_{m−1} = (A_m − kₘ·rev(A_m)) / (1 − kₘ²). Exact arithmetic, certified
/// signs; `Err` on marginal/undecidable.
fn schur_cohn(a: &[Expr]) -> Result<bool, String> {
    if a.len() < 2 {
        return Ok(true); // constant: no poles
    }
    if a.len() - 1 > MAX_STABLE_DEG {
        return Err(format!(
            "dsp.stable handles denominators up to degree {}",
            MAX_STABLE_DEG
        ));
    }
    // Symbolic-constant coefficients square the expression tree per
    // step-down stage; past a modest degree that stops being interactive.
    // (Rational coefficients — every deployed/quantized filter — have no
    // such growth; and the SOS form checks per-section regardless of order.)
    let symbolic = a.iter().any(|c| crate::expr::numeric_value(c).is_none());
    if symbolic && a.len() - 1 > MAX_STABLE_DEG_SYMBOLIC {
        return Err(format!(
            "dsp.stable: symbolic coefficients are supported to degree {} — check the filter in SOS form (dsp.stable(f)), or its quantized coefficients",
            MAX_STABLE_DEG_SYMBOLIC
        ));
    }
    // Normalize a0 to 1 (a0 must be provably nonzero).
    if certified_cmp_zero(&a[0])? == Ordering::Equal {
        return Err("dsp.stable: the leading denominator coefficient is zero".into());
    }
    let inv_a0 = pow(a[0].clone(), int(-1));
    let mut coeffs: Vec<Expr> = a
        .iter()
        .map(|c| mul(vec![c.clone(), inv_a0.clone()]))
        .collect();
    while coeffs.len() > 1 {
        let m = coeffs.len() - 1;
        let k = coeffs[m].clone();
        // Strict stability requires |k| < 1 at every stage (|k| is the
        // magnitude of a product of the remaining roots, so |k| = 1 already
        // certifies some root has modulus ≥ 1 — the answer is false, not
        // marginal-maybe). Only an undecidable sign propagates as an error.
        if certified_cmp_zero(&add(vec![int(1), mul(vec![int(-1), k.clone()])]))?
            != Ordering::Greater
        {
            return Ok(false);
        }
        if certified_cmp_zero(&add(vec![int(1), k.clone()]))? != Ordering::Greater {
            return Ok(false);
        }
        // Step down: (A − k·rev A) / (1 − k²), degree m−1. The leading
        // entry is exactly 1 again by construction.
        let denom = add(vec![int(1), mul(vec![int(-1), pow(k.clone(), int(2))])]);
        let inv = pow(denom, int(-1));
        let next: Vec<Expr> = (0..m)
            .map(|i| {
                let stepped = add(vec![
                    coeffs[i].clone(),
                    mul(vec![int(-1), k.clone(), coeffs[m - i].clone()]),
                ]);
                // No expand: on symbolic-constant coefficients (an expanded
                // dsp.tf denominator), expanding products of large surd sums
                // grows exponentially across stages and effectively hangs.
                // The certified sign decisions walk factored trees fine.
                mul(vec![stepped, inv.clone()])
            })
            .collect();
        coeffs = next;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Exact recursive filtering: dsp.filter / dsp.impz
// ---------------------------------------------------------------------------

/// Cap on len(x)·(len(b)+len(a)) — same philosophy as dsp's pairwise cap.
const MAX_FILTER_OPS: usize = 1_000_000;

/// `dsp.filter(b, a, x)` or `dsp.filter(f, x)` with a filter struct (the
/// SOS sections apply in cascade): exact direct-form-I recursion
/// y[i] = (Σ bₖ·x[i−k] − Σ_{k≥1} aₖ·y[i−k]) / a0 on an exact vector, zero
/// initial state. Bulk signals are refused on purpose: interval feedback
/// diverges, and this engine does not hand out useless-but-"certified"
/// enclosures. (FIR taps on signals: dsp.conv.)
pub fn filter(args: Vec<Expr>) -> Result<Expr, String> {
    match args.len() {
        2 => {
            let sections = sos_sections("dsp.filter", &args[0])?;
            reject_signal("dsp.filter", &args[1])?;
            let (mut x, shape) = crate::dsp::as_vector("dsp.filter", &args[1])?;
            for (b, a) in sections {
                x = apply_df1(&b, &a, &x)?;
            }
            Ok(crate::dsp::from_vector(x, shape))
        }
        3 => {
            reject_signal("dsp.filter", &args[2])?;
            let (b, _) = crate::dsp::as_vector("dsp.filter", &args[0])?;
            let (a, _) = crate::dsp::as_vector("dsp.filter", &args[1])?;
            let (x, shape) = crate::dsp::as_vector("dsp.filter", &args[2])?;
            Ok(crate::dsp::from_vector(apply_df1(&b, &a, &x)?, shape))
        }
        n => Err(format!(
            "dsp.filter expects filter(b, a, x) or filter(f, x), got {} argument(s)",
            n
        )),
    }
}

/// `dsp.impz(f, n)` / `dsp.impz(b, a, n)`: the first n samples of the
/// impulse response, exactly.
pub fn impz(mut args: Vec<Expr>) -> Result<Expr, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "dsp.impz expects impz(f, n) or impz(b, a, n), got {} argument(s)",
            args.len()
        ));
    }
    let n = crate::dsp::as_size("dsp.impz", args.last().expect("length checked"))?;
    let mut delta = vec![int(0); n];
    delta[0] = int(1);
    args.pop();
    args.push(Expr::Matrix(vec![delta]));
    filter(args)
}

fn reject_signal(name: &str, e: &Expr) -> Result<(), String> {
    if matches!(e, Expr::Signal(_)) {
        return Err(format!(
            "{} does not run on bulk signals: certified interval arithmetic diverges \
             through IIR feedback, and a blown-up enclosure would be worthless. Filter \
             an exact vector (slice(...) a stretch of interest), or use FIR taps with \
             dsp.conv.",
            name
        ));
    }
    Ok(())
}

/// The (b, a) pairs of a filter struct's SOS matrix (or a bare SOS matrix).
pub(crate) fn sos_sections(name: &str, e: &Expr) -> Result<Vec<(Vec<Expr>, Vec<Expr>)>, String> {
    let sos = match e {
        Expr::Struct(fields) => match fields.iter().find(|(k, _)| k == "sos") {
            Some((_, sos)) => sos.clone(),
            None => {
                return Err(format!(
                    "{} expects a filter struct with an 'sos' field",
                    name
                ))
            }
        },
        other => other.clone(),
    };
    let Expr::Matrix(rows) = &sos else {
        return Err(format!("{}: the sos field must be an m×6 matrix", name));
    };
    if !rows.iter().all(|r| r.len() == 6) {
        return Err(format!("{}: the sos field must be an m×6 matrix", name));
    }
    Ok(rows
        .iter()
        .map(|r| {
            (
                vec![r[0].clone(), r[1].clone(), r[2].clone()],
                vec![r[3].clone(), r[4].clone(), r[5].clone()],
            )
        })
        .collect())
}

fn apply_df1(b: &[Expr], a: &[Expr], x: &[Expr]) -> Result<Vec<Expr>, String> {
    if a.is_empty() || b.is_empty() {
        return Err("dsp.filter needs nonempty coefficient vectors".into());
    }
    if x.len().saturating_mul(b.len() + a.len()) > MAX_FILTER_OPS {
        return Err(format!(
            "dsp.filter: input is too large for exact recursion (cap {} products)",
            MAX_FILTER_OPS
        ));
    }
    if certified_cmp_zero(&a[0])? == Ordering::Equal {
        return Err("dsp.filter: the leading denominator coefficient is zero".into());
    }
    let inv_a0 = pow(a[0].clone(), int(-1));
    let mut y: Vec<Expr> = Vec::with_capacity(x.len());
    for i in 0..x.len() {
        let mut terms: Vec<Expr> = Vec::new();
        for (k, bk) in b.iter().enumerate() {
            if i >= k {
                terms.push(mul(vec![bk.clone(), x[i - k].clone()]));
            }
        }
        for (k, ak) in a.iter().enumerate().skip(1) {
            if i >= k {
                terms.push(mul(vec![int(-1), ak.clone(), y[i - k].clone()]));
            }
        }
        let sum = if terms.is_empty() { int(0) } else { add(terms) };
        y.push(expand(&mul(vec![sum, inv_a0.clone()])));
    }
    Ok(y)
}

// ---------------------------------------------------------------------------
// Frequency response of a rational filter (dsp.freqz extension).
// ---------------------------------------------------------------------------

/// H(e^{iω}) for a cascade of (b, a) sections at each ω in `w`, exactly:
/// per section, (Σ bₖ e^{−iωk}) · (Σ aₖ e^{−iωk})^(−1).
pub fn freqz_rational(
    sections: &[(Vec<Expr>, Vec<Expr>)],
    w: &[Expr],
) -> Result<Vec<Expr>, String> {
    let mut out = Vec::with_capacity(w.len());
    for wi in w {
        let mut h = int(1);
        for (b, a) in sections {
            let num = poly_response(b, wi);
            let den = poly_response(a, wi);
            h = mul(vec![h, num, pow(den, int(-1))]);
        }
        out.push(expand(&h));
    }
    Ok(out)
}

/// Σₖ cₖ·e^{−iωk}, built through the smart constructors.
fn poly_response(c: &[Expr], w: &Expr) -> Expr {
    let terms = c
        .iter()
        .enumerate()
        .map(|(k, ck)| {
            if k == 0 {
                ck.clone()
            } else {
                let arg = mul(vec![int(k as i64), w.clone()]);
                let kernel = complex(
                    func("cos", vec![arg.clone()]),
                    mul(vec![int(-1), func("sin", vec![arg])]),
                );
                mul(vec![ck.clone(), kernel])
            }
        })
        .collect();
    expand(&add(terms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schur_cohn_matches_the_stability_triangle_for_biquads() {
        // Monic z² + a1·z + a2 is stable iff |a2| < 1 and |a1| < 1 + a2.
        let cases: &[(i64, i64, i64, i64, bool)] = &[
            // (a1_num, a1_den, a2_num, a2_den, stable)
            (0, 1, 0, 1, true),   // double pole at 0
            (-3, 2, 9, 16, true), // poles 3/4, 3/4
            (-3, 1, 1, 1, false), // poles 2.618, 0.382
            (0, 1, -2, 1, false), // |a2| > 1
            (5, 2, 1, 2, false),  // |a1| > 1 + a2
            (1, 2, -1, 4, true),
        ];
        for &(n1, d1, n2, d2, want) in cases {
            let a = vec![
                int(1),
                rat_to_expr(BigRational::new(BigInt::from(n1), BigInt::from(d1))),
                rat_to_expr(BigRational::new(BigInt::from(n2), BigInt::from(d2))),
            ];
            assert_eq!(
                schur_cohn(&a).unwrap(),
                want,
                "a1 = {}/{}, a2 = {}/{}",
                n1,
                d1,
                n2,
                d2
            );
        }
        // A pole exactly on the circle is not strictly stable: false.
        let marginal = vec![int(1), int(-2), int(1)]; // (z−1)²
        assert_eq!(schur_cohn(&marginal).unwrap(), false);
    }

    #[test]
    fn exact_geometric_impulse_response() {
        // y[n] = x[n] + 1/2·y[n−1]: impulse response (1/2)^n exactly.
        let b = vec![int(1)];
        let a = vec![int(1), rat_expr(-1, 2)];
        let mut x = vec![int(0); 5];
        x[0] = int(1);
        let y = apply_df1(&b, &a, &x).unwrap();
        let expect = ["1", "1/2", "1/4", "1/8", "1/16"];
        for (yi, e) in y.iter().zip(expect) {
            assert_eq!(format!("{}", yi), *e);
        }
    }
}

// ---------------------------------------------------------------------------
// z-domain utilities: dsp.tf / dsp.poles / dsp.zeros.
// ---------------------------------------------------------------------------

/// `dsp.tf(f)`: expand a filter's SOS cascade into one transfer function
/// B(z)/A(z) — exact polynomial products of the section coefficients.
/// Returns struct(b, a) in delay form ([c0, c1, …] for Σ cₖ·z^(−k)), ready
/// for `dsp.freqz(b, a, w)`, `dsp.stable(a)`, `dsp.filter(b, a, x)`,
/// `dsp.poles`/`dsp.zeros`.
pub fn tf(args: Vec<Expr>) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err("dsp.tf expects one argument: a filter struct or SOS matrix".into());
    }
    let sections = sos_sections("dsp.tf", &args[0])?;
    let mut b = vec![int(1)];
    let mut a = vec![int(1)];
    for (sb, sa) in &sections {
        b = poly_mul_expr(&b, &strip_trailing_zeros(sb));
        a = poly_mul_expr(&a, &strip_trailing_zeros(sa));
    }
    structure(vec![
        ("b".to_string(), Expr::Matrix(vec![b])),
        ("a".to_string(), Expr::Matrix(vec![a])),
    ])
}

/// `dsp.poles(x)` / `dsp.zeros(x)`: the exact poles (roots of A) or zeros
/// (roots of B) of a filter. Section-structured input (a filter struct or
/// SOS matrix) roots each biquad by the quadratic formula — complex pairs
/// come back as exact `a ± b·i` radical expressions. A bare coefficient
/// vector of degree ≤ 2 does the same; higher degrees go through the real
/// algebraic engine when every root is real (squarefree), and refuse
/// otherwise — exact complex roots of high-degree polynomials would need a
/// complex algebraic engine, and the SOS form already carries the exact
/// factorization, so keep filters in sections.
pub fn poles_or_zeros(name: &str, args: Vec<Expr>, poles: bool) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err(format!(
            "{} expects one argument: a filter, SOS matrix, or coefficient vector",
            name
        ));
    }
    let mut out: Vec<Expr> = Vec::new();
    match &args[0] {
        Expr::Struct(_) => {
            for (b, a) in sos_sections(name, &args[0])? {
                section_roots(name, &b, &a, poles, &mut out)?;
            }
        }
        Expr::Matrix(rows) if rows.iter().all(|r| r.len() == 6) && rows.len() > 1 => {
            for (b, a) in sos_sections(name, &args[0])? {
                section_roots(name, &b, &a, poles, &mut out)?;
            }
        }
        Expr::Matrix(rows) if rows.len() == 1 && rows[0].len() == 6 => {
            for (b, a) in sos_sections(name, &args[0])? {
                section_roots(name, &b, &a, poles, &mut out)?;
            }
        }
        Expr::Matrix(_) => {
            let (c, _) = crate::dsp::as_vector(name, &args[0])?;
            let c = strip_trailing_zeros(&c);
            vector_roots(name, &c, &mut out)?;
        }
        _ => {
            return Err(format!(
                "{} expects a filter struct, an m×6 SOS matrix, or a coefficient vector",
                name
            ))
        }
    }
    Ok(Expr::Matrix(out.into_iter().map(|e| vec![e]).collect()))
}

/// The roots contributed by one section: both polynomials lift to the same
/// z-degree D = max(deg B, deg A), so a shorter polynomial's missing powers
/// become genuine roots at the origin (a pure delay), never padding
/// artifacts.
fn section_roots(
    name: &str,
    b: &[Expr],
    a: &[Expr],
    poles: bool,
    out: &mut Vec<Expr>,
) -> Result<(), String> {
    let (b, a) = (strip_trailing_zeros(b), strip_trailing_zeros(a));
    let d = b.len().max(a.len()) - 1;
    let c = if poles { &a } else { &b };
    // z^D·C(z⁻¹) = c0·z^D + c1·z^(D−1) + … : degree-D z-polynomial whose
    // low-order missing terms are exact zeros at the origin.
    for _ in 0..(d + 1 - c.len()) {
        out.push(int(0));
    }
    vector_roots(name, c, out)
}

/// Exact roots of the z-polynomial c0·z^m + c1·z^(m−1) + … + cm (delay-form
/// coefficients c, already trailing-stripped).
fn vector_roots(name: &str, c: &[Expr], out: &mut Vec<Expr>) -> Result<(), String> {
    match c.len() {
        0 | 1 => Ok(()), // constant: no roots
        2 => {
            // c0·z + c1 = 0.
            out.push(mul(vec![int(-1), c[1].clone(), pow(c[0].clone(), int(-1))]));
            Ok(())
        }
        3 => {
            // Quadratic formula on c0·z² + c1·z + c2, exactly; the
            // discriminant's sign is a certified decision, so complex pairs
            // are recognized — not guessed.
            let disc = add(vec![
                pow(c[1].clone(), int(2)),
                mul(vec![int(-4), c[0].clone(), c[2].clone()]),
            ]);
            let inv_2c0 = pow(mul(vec![int(2), c[0].clone()]), int(-1));
            let neg_c1 = mul(vec![int(-1), c[1].clone()]);
            match certified_cmp_zero(&disc)? {
                Ordering::Equal => {
                    let r = mul(vec![neg_c1, inv_2c0]);
                    out.push(r.clone());
                    out.push(r);
                }
                Ordering::Greater => {
                    let s = pow(disc, rat_expr(1, 2));
                    out.push(mul(vec![
                        add(vec![neg_c1.clone(), mul(vec![int(-1), s.clone()])]),
                        inv_2c0.clone(),
                    ]));
                    out.push(mul(vec![add(vec![neg_c1, s]), inv_2c0]));
                }
                Ordering::Less => {
                    let s = pow(mul(vec![int(-1), disc]), rat_expr(1, 2));
                    let re = mul(vec![neg_c1, inv_2c0.clone()]);
                    let im = mul(vec![s, inv_2c0]);
                    out.push(crate::expr::complex(
                        re.clone(),
                        mul(vec![int(-1), im.clone()]),
                    ));
                    out.push(crate::expr::complex(re, im));
                }
            }
            Ok(())
        }
        n => {
            // Degree ≥ 3: exact only when the polynomial is squarefree with
            // every root real — then the k-th roots are root(p, k) values.
            let coeffs: Vec<crate::expr::BigRational> = c
                .iter()
                .map(|e| {
                    crate::expr::numeric_value(e).ok_or_else(|| {
                        format!(
                            "{}: degree ≥ 3 needs rational coefficients — keep the filter \
                             in second-order sections for exact symbolic roots",
                            name
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
            // Ascending for the algebraic engine (delay form is descending
            // in z).
            let asc: Vec<crate::expr::BigRational> = coeffs.iter().rev().cloned().collect();
            let deg = n - 1;
            let count = crate::algebraic::RealAlg::real_root_count(&asc).ok_or_else(|| {
                format!(
                    "{}: this polynomial is beyond the algebraic engine's caps",
                    name
                )
            })?;
            if count != deg {
                return Err(format!(
                    "{}: a degree-{} polynomial with complex (or repeated) roots — keep \
                     the filter in second-order sections (dsp.butter output), where every \
                     root is exact by the quadratic formula",
                    name, deg
                ));
            }
            // Build the poly expression in z and return root(p, k) values.
            let z = Expr::Symbol("z".to_string());
            let terms: Vec<Expr> = c
                .iter()
                .enumerate()
                .map(|(i, ci)| mul(vec![ci.clone(), pow(z.clone(), int((deg - i) as i64))]))
                .collect();
            let p = add(terms);
            for k in 1..=deg {
                out.push(Expr::Func(
                    "root".to_string(),
                    vec![p.clone(), int(k as i64)],
                ));
            }
            Ok(())
        }
    }
}

fn strip_trailing_zeros(c: &[Expr]) -> Vec<Expr> {
    let mut v = c.to_vec();
    while v.len() > 1
        && matches!(v.last(), Some(e) if crate::expr::numeric_value(e).is_some_and(|r| r.is_zero()))
    {
        v.pop();
    }
    v
}

/// Exact product of two delay-form coefficient polynomials.
fn poly_mul_expr(a: &[Expr], b: &[Expr]) -> Vec<Expr> {
    let mut out = vec![Vec::new(); a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j].push(mul(vec![x.clone(), y.clone()]));
        }
    }
    out.into_iter()
        .map(|terms| {
            if terms.is_empty() {
                int(0)
            } else {
                crate::expr::expand(&add(terms))
            }
        })
        .collect()
}
