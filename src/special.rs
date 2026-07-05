//! Arbitrary-precision special functions and the statistical distributions
//! built on them. This is the numeric corner of `stats`: a normal CDF has no
//! exact closed form, so — exactly like `sin(2)` — these stay symbolic until
//! `N(...)` crosses into floats. That crossing lands here.
//!
//! Everything runs on `astro_float::BigFloat` at a working precision well
//! above what `N(...)` asks for (guard digits absorb the cancellation in the
//! gamma/beta machinery), then the caller rounds back down. The two load-
//! bearing primitives are the regularized incomplete gamma `P(a,x)` (χ², and
//! `erf` via `P(1/2, x²)`) and the regularized incomplete beta `Iₓ(a,b)`
//! (Student-t and F); `lgamma` (Spouge) normalizes both. Inverse CDFs are a
//! safeguarded Newton iteration on the forward CDF.

use crate::expr::{bf_lt, float_to_rational};
use astro_float::{BigFloat, Consts, RoundingMode};
use num_traits::ToPrimitive;

const RM: RoundingMode = RoundingMode::ToEven;

/// Hard cap on series / continued-fraction iterations — a convergence
/// backstop, set far above any well-conditioned input's real need.
const MAXIT: usize = 100_000;

// -- small BigFloat helpers ---------------------------------------------------

#[inline]
fn fi(i: i64, p: usize) -> BigFloat {
    BigFloat::from_i64(i, p)
}

#[inline]
fn ff(f: f64, p: usize) -> BigFloat {
    // Exact decode: astro-float's own from_f64 halves subnormal inputs.
    crate::expr::bf_from_f64_exact(f, p)
}

#[inline]
fn neg(x: &BigFloat, p: usize) -> BigFloat {
    fi(0, p).sub(x, p, RM)
}

/// `2^-n` as a positive BigFloat — the relative tolerance and the CF "tiny".
fn pow2_neg(n: usize, p: usize) -> BigFloat {
    fi(1, p).div(&fi(2, p).powi(n, p, RM), p, RM)
}

/// Best-effort f64 of a BigFloat, for seeding the f64 initial guesses. Loses
/// precision by design — Newton refines from here.
fn to_f64(x: &BigFloat) -> f64 {
    float_to_rational(x)
        .and_then(|r| r.to_f64())
        .unwrap_or(f64::NAN)
}

// -- log-gamma (Spouge) -------------------------------------------------------

/// `ln Γ(z)` for `z > 0` by Spouge's approximation. The number of terms `g`
/// scales with the working precision; the partial sums cancel, which is why
/// callers run this at roughly double the precision they need.
fn lgamma_pos(z: &BigFloat, w: usize, cc: &mut Consts) -> BigFloat {
    // g terms: the formula error is ~ (2π)^-(g+1/2); deliver more than w/2 good
    // bits (cancellation eats the rest) by taking g ≈ 0.21·w.
    let g = ((w as f64) * 0.21).ceil() as usize + 12;
    let half = ff(0.5, w);
    let gbf = fi(g as i64, w);

    // S = c₀ + Σ_{k=1}^{g-1} c_k/(z+k), with c₀ = √(2π) and c_k built in log
    // space: ln|c_k| = (k-½)·ln(g-k) + (g-k) - ln((k-1)!), sign = (-1)^{k-1}.
    let mut s = cc.pi(w, RM).mul(&fi(2, w), w, RM).sqrt(w, RM); // c₀ = √(2π)
    let mut lfact = fi(0, w); // ln((k-1)!), running
    let mut sign_pos = true; // k = 1 → +
    for k in 1..g {
        if k >= 2 {
            lfact = lfact.add(&fi((k - 1) as i64, w).ln(w, RM, cc), w, RM);
        }
        let gk = fi((g - k) as i64, w);
        let kk = fi(k as i64, w).sub(&half, w, RM);
        let lnc = kk
            .mul(&gk.ln(w, RM, cc), w, RM)
            .add(&gk, w, RM)
            .sub(&lfact, w, RM);
        let mut ck = lnc.exp(w, RM, cc);
        if !sign_pos {
            ck = neg(&ck, w);
        }
        let zk = z.add(&fi(k as i64, w), w, RM);
        s = s.add(&ck.div(&zk, w, RM), w, RM);
        sign_pos = !sign_pos;
    }

    let z_plus_g = z.add(&gbf, w, RM);
    // ln Γ(z+1) = (z+½)·ln(z+g) - (z+g) + ln(S); then ln Γ(z) = that - ln(z).
    z.add(&half, w, RM)
        .mul(&z_plus_g.ln(w, RM, cc), w, RM)
        .sub(&z_plus_g, w, RM)
        .add(&s.ln(w, RM, cc), w, RM)
        .sub(&z.ln(w, RM, cc), w, RM)
}

/// Γ(x) for real x, by reflection for x ≤ 0. Errors at the non-positive-integer
/// poles.
fn gamma(x: &BigFloat, w: usize, cc: &mut Consts) -> Result<BigFloat, String> {
    if x.is_positive() && !x.is_zero() {
        return Ok(lgamma_pos(x, w, cc).exp(w, RM, cc));
    }
    // x ≤ 0: a pole at every non-positive integer.
    if float_to_rational(x).is_some_and(|r| r.is_integer()) {
        return Err("gamma has a pole at non-positive integers".into());
    }
    // Reflection: Γ(x) = π / (sin(πx)·Γ(1-x)).
    let pi = cc.pi(w, RM);
    let sin_pix = pi.mul(x, w, RM).sin(w, RM, cc);
    let one_minus_x = fi(1, w).sub(x, w, RM);
    let g1 = lgamma_pos(&one_minus_x, w, cc).exp(w, RM, cc);
    Ok(pi.div(&sin_pix.mul(&g1, w, RM), w, RM))
}

// -- regularized incomplete gamma --------------------------------------------

/// `(P(a,x), Q(a,x))` — the regularized lower and upper incomplete gamma, for
/// `a > 0`, `x ≥ 0`. Series below the `x = a+1` crossover, continued fraction
/// above it, so whichever of P/Q is the small tail stays accurate.
fn gamma_pq(
    a: &BigFloat,
    x: &BigFloat,
    w: usize,
    eps: &BigFloat,
    cc: &mut Consts,
) -> (BigFloat, BigFloat) {
    if x.is_zero() {
        return (fi(0, w), fi(1, w));
    }
    let one = fi(1, w);
    let lga = lgamma_pos(a, w, cc);
    // exp(a·ln x - x - lnΓ(a))
    let pref = a
        .mul(&x.ln(w, RM, cc), w, RM)
        .sub(x, w, RM)
        .sub(&lga, w, RM)
        .exp(w, RM, cc);

    if bf_lt(x, &a.add(&one, w, RM)) {
        // Series: Σ_{n≥0} xⁿ / (a(a+1)···(a+n)).
        let mut ap = a.clone();
        let mut term = one.div(a, w, RM);
        let mut sum = term.clone();
        for _ in 0..MAXIT {
            ap = ap.add(&one, w, RM);
            term = term.mul(x, w, RM).div(&ap, w, RM);
            sum = sum.add(&term, w, RM);
            if bf_lt(&term.div(&sum, w, RM).abs(), eps) {
                break;
            }
        }
        let p = sum.mul(&pref, w, RM);
        (p.clone(), one.sub(&p, w, RM))
    } else {
        // Continued fraction (Numerical Recipes `gcf`) for the upper tail.
        let tiny = pow2_neg(w / 2, w);
        let mut b = x.sub(a, w, RM).add(&one, w, RM);
        let mut c = one.div(&tiny, w, RM);
        let mut d = one.div(&b, w, RM);
        let mut h = d.clone();
        for i in 1..MAXIT {
            let ifb = fi(i as i64, w);
            let an = neg(&ifb.mul(&ifb.sub(a, w, RM), w, RM), w); // -i(i-a)
            b = b.add(&fi(2, w), w, RM);
            d = an.mul(&d, w, RM).add(&b, w, RM);
            if bf_lt(&d.abs(), &tiny) {
                d = tiny.clone();
            }
            d = one.div(&d, w, RM);
            c = b.add(&an.div(&c, w, RM), w, RM);
            if bf_lt(&c.abs(), &tiny) {
                c = tiny.clone();
            }
            let del = d.mul(&c, w, RM);
            h = h.mul(&del, w, RM);
            if bf_lt(&del.sub(&one, w, RM).abs(), eps) {
                break;
            }
        }
        let q = h.mul(&pref, w, RM);
        (one.sub(&q, w, RM), q)
    }
}

// -- regularized incomplete beta ---------------------------------------------

/// The Lentz continued fraction behind `Iₓ(a,b)` (Numerical Recipes `betacf`).
fn betacf(a: &BigFloat, b: &BigFloat, x: &BigFloat, w: usize, eps: &BigFloat) -> BigFloat {
    let one = fi(1, w);
    let two = fi(2, w);
    let tiny = pow2_neg(w / 2, w);
    let qab = a.add(b, w, RM);
    let qap = a.add(&one, w, RM);
    let qam = a.sub(&one, w, RM);
    let mut c = one.clone();
    let mut d = one.sub(&qab.mul(x, w, RM).div(&qap, w, RM), w, RM);
    if bf_lt(&d.abs(), &tiny) {
        d = tiny.clone();
    }
    d = one.div(&d, w, RM);
    let mut h = d.clone();
    for m in 1..MAXIT {
        let mf = fi(m as i64, w);
        let m2 = mf.mul(&two, w, RM);
        // even step
        let aa = mf.mul(&b.sub(&mf, w, RM), w, RM).mul(x, w, RM).div(
            &qam.add(&m2, w, RM).mul(&a.add(&m2, w, RM), w, RM),
            w,
            RM,
        );
        d = one.add(&aa.mul(&d, w, RM), w, RM);
        if bf_lt(&d.abs(), &tiny) {
            d = tiny.clone();
        }
        c = one.add(&aa.div(&c, w, RM), w, RM);
        if bf_lt(&c.abs(), &tiny) {
            c = tiny.clone();
        }
        d = one.div(&d, w, RM);
        h = h.mul(&d, w, RM).mul(&c, w, RM);
        // odd step
        let aa = neg(
            &a.add(&mf, w, RM)
                .mul(&qab.add(&mf, w, RM), w, RM)
                .mul(x, w, RM)
                .div(&a.add(&m2, w, RM).mul(&qap.add(&m2, w, RM), w, RM), w, RM),
            w,
        );
        d = one.add(&aa.mul(&d, w, RM), w, RM);
        if bf_lt(&d.abs(), &tiny) {
            d = tiny.clone();
        }
        c = one.add(&aa.div(&c, w, RM), w, RM);
        if bf_lt(&c.abs(), &tiny) {
            c = tiny.clone();
        }
        d = one.div(&d, w, RM);
        let del = d.mul(&c, w, RM);
        h = h.mul(&del, w, RM);
        if bf_lt(&del.sub(&one, w, RM).abs(), eps) {
            break;
        }
    }
    h
}

/// `Iₓ(a,b)`, the regularized incomplete beta, for `a,b > 0` and any real x
/// (clamped to `[0,1]`). The CF is evaluated on whichever of x / 1-x converges.
fn inc_beta(
    x: &BigFloat,
    a: &BigFloat,
    b: &BigFloat,
    w: usize,
    eps: &BigFloat,
    cc: &mut Consts,
) -> BigFloat {
    let one = fi(1, w);
    if !x.is_positive() || x.is_zero() {
        return fi(0, w);
    }
    if !bf_lt(x, &one) {
        return one;
    }
    let omx = one.sub(x, w, RM);
    // bt = exp(lnΓ(a+b) - lnΓ(a) - lnΓ(b) + a·ln x + b·ln(1-x))
    let bt = lgamma_pos(&a.add(b, w, RM), w, cc)
        .sub(&lgamma_pos(a, w, cc), w, RM)
        .sub(&lgamma_pos(b, w, cc), w, RM)
        .add(&a.mul(&x.ln(w, RM, cc), w, RM), w, RM)
        .add(&b.mul(&omx.ln(w, RM, cc), w, RM), w, RM)
        .exp(w, RM, cc);
    let thresh = a
        .add(&one, w, RM)
        .div(&a.add(b, w, RM).add(&fi(2, w), w, RM), w, RM);
    if bf_lt(x, &thresh) {
        bt.mul(&betacf(a, b, x, w, eps), w, RM).div(a, w, RM)
    } else {
        one.sub(
            &bt.mul(&betacf(b, a, &omx, w, eps), w, RM).div(b, w, RM),
            w,
            RM,
        )
    }
}

// -- error function -----------------------------------------------------------

/// erf(x) = sign(x)·P(½, x²).
fn erf(x: &BigFloat, w: usize, eps: &BigFloat, cc: &mut Consts) -> BigFloat {
    if x.is_zero() {
        return fi(0, w);
    }
    let half = ff(0.5, w);
    let x2 = x.mul(x, w, RM);
    let (p, _) = gamma_pq(&half, &x2, w, eps, cc);
    if x.is_negative() {
        neg(&p, w)
    } else {
        p
    }
}

/// erfc(x), kept tail-accurate by reaching for Q(½,x²) on the positive side.
fn erfc(x: &BigFloat, w: usize, eps: &BigFloat, cc: &mut Consts) -> BigFloat {
    let half = ff(0.5, w);
    let x2 = x.mul(x, w, RM);
    let (_, q) = gamma_pq(&half, &x2, w, eps, cc);
    if x.is_negative() {
        fi(2, w).sub(&q, w, RM) // erfc(-|x|) = 2 - Q(½,x²)
    } else {
        q
    }
}

// -- distributions ------------------------------------------------------------

/// A distribution and its (already validated, positive) shape parameters,
/// so one safeguarded-Newton inverter serves all four.
enum Dist {
    Normal(BigFloat, BigFloat), // (μ, σ)
    StudentT(BigFloat),         // ν
    ChiSquare(BigFloat),        // k
    FisherF(BigFloat, BigFloat),
}

fn dist_cdf(d: &Dist, x: &BigFloat, w: usize, eps: &BigFloat, cc: &mut Consts) -> BigFloat {
    let one = fi(1, w);
    let half = ff(0.5, w);
    match d {
        Dist::Normal(mu, sigma) => {
            // ½·erfc(-(x-μ)/(σ√2)) — uniform tail accuracy.
            let z = x.sub(mu, w, RM).div(sigma, w, RM);
            let arg = neg(&z.div(&fi(2, w).sqrt(w, RM), w, RM), w);
            half.mul(&erfc(&arg, w, eps, cc), w, RM)
        }
        Dist::StudentT(nu) => {
            let ib = inc_beta(
                &nu.div(&nu.add(&x.mul(x, w, RM), w, RM), w, RM),
                &nu.mul(&half, w, RM),
                &half,
                w,
                eps,
                cc,
            );
            if x.is_negative() {
                half.mul(&ib, w, RM)
            } else {
                one.sub(&half.mul(&ib, w, RM), w, RM)
            }
        }
        Dist::ChiSquare(k) => {
            if !x.is_positive() || x.is_zero() {
                return fi(0, w);
            }
            gamma_pq(&k.mul(&half, w, RM), &x.mul(&half, w, RM), w, eps, cc).0
        }
        Dist::FisherF(d1, d2) => {
            if !x.is_positive() || x.is_zero() {
                return fi(0, w);
            }
            let d1x = d1.mul(x, w, RM);
            let y = d1x.div(&d1x.add(d2, w, RM), w, RM);
            inc_beta(&y, &d1.mul(&half, w, RM), &d2.mul(&half, w, RM), w, eps, cc)
        }
    }
}

fn dist_pdf(d: &Dist, x: &BigFloat, w: usize, cc: &mut Consts) -> BigFloat {
    let one = fi(1, w);
    let half = ff(0.5, w);
    let two = fi(2, w);
    let pi = cc.pi(w, RM);
    match d {
        Dist::Normal(mu, sigma) => {
            let z = x.sub(mu, w, RM).div(sigma, w, RM);
            let norm = sigma.mul(&two.mul(&pi, w, RM).sqrt(w, RM), w, RM);
            neg(&z.mul(&z, w, RM).mul(&half, w, RM), w)
                .exp(w, RM, cc)
                .div(&norm, w, RM)
        }
        Dist::StudentT(nu) => {
            // exp(lnΓ((ν+1)/2) - lnΓ(ν/2) - ½ln(νπ)) · (1+t²/ν)^(-(ν+1)/2)
            let lead = lgamma_pos(&nu.add(&one, w, RM).mul(&half, w, RM), w, cc)
                .sub(&lgamma_pos(&nu.mul(&half, w, RM), w, cc), w, RM)
                .sub(&nu.mul(&pi, w, RM).ln(w, RM, cc).mul(&half, w, RM), w, RM)
                .exp(w, RM, cc);
            let body = one.add(&x.mul(x, w, RM).div(nu, w, RM), w, RM).pow(
                &neg(&nu.add(&one, w, RM).mul(&half, w, RM), w),
                w,
                RM,
                cc,
            );
            lead.mul(&body, w, RM)
        }
        Dist::ChiSquare(k) => {
            if !x.is_positive() || x.is_zero() {
                return fi(0, w);
            }
            let kh = k.mul(&half, w, RM);
            // exp((k/2-1)ln x - x/2 - (k/2)ln2 - lnΓ(k/2))
            kh.sub(&one, w, RM)
                .mul(&x.ln(w, RM, cc), w, RM)
                .sub(&x.mul(&half, w, RM), w, RM)
                .sub(&kh.mul(&two.ln(w, RM, cc), w, RM), w, RM)
                .sub(&lgamma_pos(&kh, w, cc), w, RM)
                .exp(w, RM, cc)
        }
        Dist::FisherF(d1, d2) => {
            if !x.is_positive() || x.is_zero() {
                return fi(0, w);
            }
            let (d1h, d2h) = (d1.mul(&half, w, RM), d2.mul(&half, w, RM));
            let lbeta = lgamma_pos(&d1h, w, cc)
                .add(&lgamma_pos(&d2h, w, cc), w, RM)
                .sub(&lgamma_pos(&d1h.add(&d2h, w, RM), w, cc), w, RM);
            // exp((d1/2)ln(d1/d2) + (d1/2-1)ln x - ((d1+d2)/2)ln(1+d1·x/d2) - lnB)
            d1h.mul(&d1.div(d2, w, RM).ln(w, RM, cc), w, RM)
                .add(&d1h.sub(&one, w, RM).mul(&x.ln(w, RM, cc), w, RM), w, RM)
                .sub(
                    &d1h.add(&d2h, w, RM).mul(
                        &one.add(&d1.mul(x, w, RM).div(d2, w, RM), w, RM)
                            .ln(w, RM, cc),
                        w,
                        RM,
                    ),
                    w,
                    RM,
                )
                .sub(&lbeta, w, RM)
                .exp(w, RM, cc)
        }
    }
}

/// An f64 initial guess for the quantile, good enough to seed Newton.
fn initial_guess(d: &Dist, p: f64) -> f64 {
    match d {
        Dist::Normal(mu, sigma) => to_f64(mu) + to_f64(sigma) * acklam(p),
        Dist::StudentT(_) => acklam(p), // symmetric; Newton corrects the tails
        Dist::ChiSquare(k) => {
            let k = to_f64(k);
            // Wilson–Hilferty.
            let t = 1.0 - 2.0 / (9.0 * k) + acklam(p) * (2.0 / (9.0 * k)).sqrt();
            (k * t * t * t).max(1e-6)
        }
        Dist::FisherF(..) => 1.0, // median ≈ 1; bracket expansion handles the rest
    }
}

/// Lower bound of the support (None = -∞).
fn lower_bound(d: &Dist) -> Option<f64> {
    matches!(d, Dist::ChiSquare(_) | Dist::FisherF(..)).then_some(0.0)
}

/// Invert the CDF: `x` with `F(x) = p`, by Newton steps safeguarded inside a
/// bracket that bisection always shrinks. Robust for every monotone CDF here.
fn dist_invert(
    d: &Dist,
    ptarget: &BigFloat,
    w: usize,
    eps: &BigFloat,
    cc: &mut Consts,
) -> BigFloat {
    let one = fi(1, w);
    let two = fi(2, w);
    let pf = to_f64(ptarget).clamp(1e-300, 1.0 - 1e-16);
    let mut x = ff(initial_guess(d, pf), w);

    // Bracket [a, b] with F(a) < p < F(b).
    let bounded = lower_bound(d).is_some();
    let mut a = if bounded {
        pow2_neg(w / 4, w)
    } else {
        x.sub(&one, w, RM)
    };
    let mut b = if bounded {
        x.abs().add(&one, w, RM)
    } else {
        x.add(&one, w, RM)
    };
    // Push the ends out until they straddle p (capped — guesses are close).
    for _ in 0..200 {
        if bf_lt(&dist_cdf(d, &a, w, eps, cc), ptarget) {
            break;
        }
        a = if bounded {
            a.div(&two, w, RM)
        } else {
            a.sub(&b.sub(&a, w, RM), w, RM)
        };
    }
    for _ in 0..200 {
        if bf_lt(ptarget, &dist_cdf(d, &b, w, eps, cc)) {
            break;
        }
        b = b.add(&b.sub(&a, w, RM), w, RM);
    }
    if !bf_lt(&a, &x) || !bf_lt(&x, &b) {
        x = a.add(&b, w, RM).div(&two, w, RM);
    }

    for _ in 0..200 {
        let fx = dist_cdf(d, &x, w, eps, cc).sub(ptarget, w, RM);
        if fx.is_positive() && !fx.is_zero() {
            b = x.clone();
        } else {
            a = x.clone();
        }
        let fpx = dist_pdf(d, &x, w, cc);
        let mut xn = a.add(&b, w, RM).div(&two, w, RM); // bisection fallback
        if fpx.is_positive() && !fpx.is_zero() {
            let cand = x.sub(&fx.div(&fpx, w, RM), w, RM);
            if bf_lt(&a, &cand) && bf_lt(&cand, &b) {
                xn = cand;
            }
        }
        let diff = xn.sub(&x, w, RM).abs();
        let scale = xn.abs().add(&one, w, RM);
        x = xn;
        if bf_lt(&diff, &eps.mul(&scale, w, RM)) {
            break;
        }
    }
    x
}

/// Acklam's rational approximation to the standard-normal quantile (f64,
/// ~1e-9 relative). Only ever a Newton seed, so this accuracy is ample.
#[allow(clippy::excessive_precision)] // published coefficients, kept verbatim
fn acklam(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    let pl = 0.02425;
    if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

// -- dispatch -----------------------------------------------------------------

/// Names this module evaluates numerically. `to_bigfloat` routes these here
/// instead of treating them as opaque applications.
pub fn is_special(name: &str) -> bool {
    matches!(
        name,
        "erf"
            | "erfc"
            | "gamma"
            | "lgamma"
            | "beta"
            | "normcdf"
            | "normpdf"
            | "norminv"
            | "tcdf"
            | "tpdf"
            | "tinv"
            | "chisqcdf"
            | "chisqpdf"
            | "chisqinv"
            | "fcdf"
            | "fpdf"
            | "finv"
    )
}

fn require_pos(name: &str, what: &str, v: &BigFloat) -> Result<(), String> {
    if v.is_positive() && !v.is_zero() {
        Ok(())
    } else {
        Err(format!("{}: {} must be positive", name, what))
    }
}

fn require_prob(name: &str, p: &BigFloat, w: usize) -> Result<(), String> {
    if (p.is_positive() && !p.is_zero()) && bf_lt(p, &fi(1, w)) {
        Ok(())
    } else {
        Err(format!(
            "{}: probability must be strictly between 0 and 1",
            name
        ))
    }
}

/// Evaluate a special function on already-numeric arguments. `p` is the
/// caller's target precision in bits; we work at roughly double that.
pub fn eval(name: &str, xs: &[BigFloat], p: usize, cc: &mut Consts) -> Result<BigFloat, String> {
    let w = 2 * p + 128;
    let eps = pow2_neg(p + 24, w);
    let arity = |n: usize| -> Result<(), String> {
        if xs.len() == n {
            Ok(())
        } else {
            Err(format!(
                "{} expects {} argument(s), got {}",
                name,
                n,
                xs.len()
            ))
        }
    };
    // Normal accepts either the standard form or explicit (μ, σ).
    let normal = |xs: &[BigFloat], extra: usize| -> Result<Dist, String> {
        match xs.len() {
            n if n == extra => Ok(Dist::Normal(fi(0, w), fi(1, w))),
            n if n == extra + 2 => {
                require_pos(name, "sigma", &xs[extra + 1])?;
                Ok(Dist::Normal(xs[extra].clone(), xs[extra + 1].clone()))
            }
            got => Err(format!(
                "{} expects {} or {} arguments, got {}",
                name,
                extra,
                extra + 2,
                got
            )),
        }
    };

    match name {
        "erf" => arity(1).map(|_| erf(&xs[0], w, &eps, cc)),
        "erfc" => arity(1).map(|_| erfc(&xs[0], w, &eps, cc)),
        "gamma" => arity(1).and_then(|_| gamma(&xs[0], w, cc)),
        "lgamma" => arity(1)
            .and_then(|_| require_pos(name, "argument", &xs[0]).map(|_| lgamma_pos(&xs[0], w, cc))),
        "beta" => arity(2).and_then(|_| {
            require_pos(name, "a", &xs[0])?;
            require_pos(name, "b", &xs[1])?;
            Ok(lgamma_pos(&xs[0], w, cc)
                .add(&lgamma_pos(&xs[1], w, cc), w, RM)
                .sub(&lgamma_pos(&xs[0].add(&xs[1], w, RM), w, cc), w, RM)
                .exp(w, RM, cc))
        }),

        "normcdf" => normal(xs, 1).map(|d| dist_cdf(&d, &xs[0], w, &eps, cc)),
        "normpdf" => normal(xs, 1).map(|d| dist_pdf(&d, &xs[0], w, cc)),
        "norminv" => {
            let d = normal(xs, 1)?;
            require_prob(name, &xs[0], w)?;
            Ok(dist_invert(&d, &xs[0], w, &eps, cc))
        }

        "tcdf" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            Ok(dist_cdf(
                &Dist::StudentT(xs[1].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),
        "tpdf" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            Ok(dist_pdf(&Dist::StudentT(xs[1].clone()), &xs[0], w, cc))
        }),
        "tinv" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            require_prob(name, &xs[0], w)?;
            Ok(dist_invert(
                &Dist::StudentT(xs[1].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),

        "chisqcdf" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            Ok(dist_cdf(
                &Dist::ChiSquare(xs[1].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),
        "chisqpdf" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            Ok(dist_pdf(&Dist::ChiSquare(xs[1].clone()), &xs[0], w, cc))
        }),
        "chisqinv" => arity(2).and_then(|_| {
            require_pos(name, "degrees of freedom", &xs[1])?;
            require_prob(name, &xs[0], w)?;
            Ok(dist_invert(
                &Dist::ChiSquare(xs[1].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),

        "fcdf" => arity(3).and_then(|_| {
            require_pos(name, "d1", &xs[1])?;
            require_pos(name, "d2", &xs[2])?;
            Ok(dist_cdf(
                &Dist::FisherF(xs[1].clone(), xs[2].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),
        "fpdf" => arity(3).and_then(|_| {
            require_pos(name, "d1", &xs[1])?;
            require_pos(name, "d2", &xs[2])?;
            Ok(dist_pdf(
                &Dist::FisherF(xs[1].clone(), xs[2].clone()),
                &xs[0],
                w,
                cc,
            ))
        }),
        "finv" => arity(3).and_then(|_| {
            require_pos(name, "d1", &xs[1])?;
            require_pos(name, "d2", &xs[2])?;
            require_prob(name, &xs[0], w)?;
            Ok(dist_invert(
                &Dist::FisherF(xs[1].clone(), xs[2].clone()),
                &xs[0],
                w,
                &eps,
                cc,
            ))
        }),

        _ => Err(format!("cannot numerically evaluate '{}'", name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluate `name(args)` to an f64, at enough precision that the f64 is
    /// fully determined.
    fn ev(name: &str, args: &[f64]) -> f64 {
        let mut cc = Consts::new().unwrap();
        let xs: Vec<BigFloat> = args.iter().map(|&a| ff(a, 200)).collect();
        to_f64(&eval(name, &xs, 80, &mut cc).unwrap())
    }

    fn close(a: f64, b: f64) {
        assert!(
            (a - b).abs() <= 1e-11 * (1.0 + b.abs()),
            "got {}, want {}",
            a,
            b
        );
    }

    #[test]
    fn gamma_and_lgamma() {
        close(ev("gamma", &[5.0]), 24.0); // 4!
        close(ev("gamma", &[0.5]), std::f64::consts::PI.sqrt());
        close(ev("gamma", &[10.5]), 1133278.3889487854);
        close(ev("gamma", &[-1.5]), 2.3632718012073544); // reflection
        close(ev("lgamma", &[100.0]), 359.1342053695754);
        close(ev("beta", &[2.0, 3.0]), 1.0 / 12.0);
    }

    #[test]
    fn error_function() {
        close(ev("erf", &[1.0]), 0.8427007929497148);
        close(ev("erf", &[-0.5]), -0.5204998778130465);
        close(ev("erfc", &[2.0]), 0.0046777349810472645);
    }

    #[test]
    fn normal_distribution() {
        close(ev("normcdf", &[0.0]), 0.5);
        close(ev("normcdf", &[1.96]), 0.9750021048517795);
        close(ev("normcdf", &[-3.0]), 0.0013498980316300933);
        close(ev("normpdf", &[0.0]), 0.3989422804014327);
        close(ev("norminv", &[0.975]), 1.959963984540054);
        close(ev("norminv", &[0.025]), -1.959963984540054);
        close(ev("normcdf", &[ev("norminv", &[0.3])]), 0.3); // round trip
    }

    #[test]
    fn student_t() {
        // pt(2,5) by independent fine quadrature; pt(0,nu)=1/2 exactly;
        // qt(0.975,1) = tan(pi*0.475) exactly (Cauchy).
        close(ev("tcdf", &[2.0, 5.0]), 0.9490302605850676);
        close(ev("tcdf", &[0.0, 10.0]), 0.5);
        close(ev("tinv", &[0.975, 1.0]), 12.706204736174696);
        close(ev("tcdf", &[ev("tinv", &[0.975, 10.0]), 10.0]), 0.975); // round trip
        close(ev("tinv", &[0.975, 10.0]), 2.2281388519649385);
    }

    #[test]
    fn chi_square() {
        // pchisq(x,1) = erf(sqrt(x/2)) exactly.
        close(ev("chisqcdf", &[3.84, 1.0]), 0.9499564787512949);
        close(ev("chisqinv", &[0.95, 1.0]), 3.841458820694124);
        close(ev("chisqinv", &[0.95, 10.0]), 18.307038053275146);
        close(ev("chisqcdf", &[ev("chisqinv", &[0.95, 10.0]), 10.0]), 0.95); // round trip
    }

    #[test]
    fn fisher_f() {
        close(ev("fcdf", &[1.0, 10.0, 10.0]), 0.5); // symmetry: F(1;d,d)=1/2
        close(
            ev("fcdf", &[ev("finv", &[0.95, 5.0, 10.0]), 5.0, 10.0]),
            0.95,
        ); // round trip
        close(ev("finv", &[0.95, 5.0, 10.0]), 3.325834530413011);
    }
}
