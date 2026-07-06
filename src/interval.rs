//! Certified interval evaluation, for deciding the sign of a constant
//! expression exactly.
//!
//! A constant expression evaluates to an enclosure [lo, hi] computed with
//! directed rounding (lo rounds toward −∞, hi toward +∞ at every step), so
//! the true value provably lies inside. If the enclosure excludes zero, the
//! sign is *certain* — not a float guess. If it straddles zero, precision
//! doubles and we try again, up to [`MAX_BITS`]; past that the caller
//! reports "may be equal" rather than answering. The result is therefore
//! never wrong, merely sometimes refused — proving two different-looking
//! constants *equal* needs real algebraic numbers, which this deliberately
//! is not: that lives in `crate::algebraic`, and `eval::compare` falls back
//! to it exactly when this module refuses.
//!
//! Trig enclosures use the 1-Lipschitz bound: sin([a,b]) ⊆ sin(a) ± (b−a),
//! avoiding non-monotonic case analysis; tan goes through sin/cos with a
//! zero-excluding interval division (so poles refuse rather than lie).

use crate::expr::{
    bf_lt, bf_strictly_neg, bf_strictly_pos, known_nonneg, numeric_value, with_consts, Constant,
    Expr,
};
use astro_float::{BigFloat, Consts, Radix, RoundingMode};
use num_traits::ToPrimitive;

/// Refinement ceiling, in bits of working precision (≈ 2,466 decimal digits).
pub const MAX_BITS: usize = 8192;

/// `MAX_BITS` as decimal digits, for error messages.
pub fn max_digits() -> usize {
    (MAX_BITS as f64 * std::f64::consts::LOG10_2) as usize
}

/// Cap on integer exponents evaluated by interval squaring.
const MAX_IV_EXP: i64 = 10_000;

const UP: RoundingMode = RoundingMode::Up; // toward +∞
const DOWN: RoundingMode = RoundingMode::Down; // toward −∞
const NEAREST: RoundingMode = RoundingMode::ToEven;
const RADIX: Radix = Radix::Dec;

pub enum Sign {
    Negative,
    Zero,
    Positive,
    /// An enclosure touched zero from above ([0, w]): the value is provably
    /// ≥ 0, but may be exactly zero — `>= 0` is answerable, `> 0` is not.
    NonNegative,
    /// Mirror image: provably ≤ 0.
    NonPositive,
    /// Constant, but the enclosure still straddled zero at `MAX_BITS` —
    /// the values may be equal.
    Inseparable,
    /// Not a constant real expression (free symbols, complex values, …).
    Unsupported,
}

/// A certified rational enclosure of a constant expression at `p` bits of
/// working precision: the true value provably lies in [lo, hi]. `None` for
/// non-constant or unsupported expressions. (Used by the exact Remez design
/// to pin down band edges like cos(2π/5) that have no rational form.)
pub fn rational_enclosure(
    e: &Expr,
    p: usize,
) -> Option<(crate::expr::BigRational, crate::expr::BigRational)> {
    let iv = with_consts(|cc| eval_iv(e, p, cc)).ok()??;
    Some((
        crate::expr::float_to_rational(&iv.lo)?,
        crate::expr::float_to_rational(&iv.hi)?,
    ))
}

/// Precision ceiling for [`decimal_preview`] — a UI nicety, not a proof
/// obligation, so it gives up far below the comparison engine's `MAX_BITS`.
const PREVIEW_MAX_BITS: usize = 768;

/// A short certified decimal preview of a constant real expression, for the
/// UI's "≈" ghost next to exact results: refine until *both* enclosure
/// endpoints render to the same `digits`-significant-figure string, so every
/// digit shown is provably correct. `None` when the expression isn't a
/// supported real constant, or the endpoints still disagree at the (small)
/// ceiling — no preview is better than an uncertified one.
pub fn decimal_preview(e: &Expr, digits: usize) -> Option<String> {
    let mut p = 96;
    while p <= PREVIEW_MAX_BITS {
        if let Ok(Some(iv)) = with_consts(|cc| eval_iv(e, p, cc)) {
            let lo = crate::expr::format_bigfloat(&iv.lo, digits);
            if lo == crate::expr::format_bigfloat(&iv.hi, digits) {
                return Some(lo);
            }
        }
        p *= 2;
    }
    None
}

/// The certified sign of `e`, by interval refinement.
pub fn certified_sign(e: &Expr) -> Sign {
    let mut p = 64;
    let mut evaluated = false;
    // One-sided knowledge accumulates across refinements: any enclosure with
    // lo ≥ 0 proves the value ≥ 0 forever (each enclosure is independently
    // valid). Both sides together prove exact zero.
    let mut known_nonneg = false;
    let mut known_nonpos = false;
    while p <= MAX_BITS {
        let iv = with_consts(|cc| eval_iv(e, p, cc));
        if let Ok(Some(iv)) = iv {
            evaluated = true;
            // Strictness matters: astro-float reports an exact zero as
            // "positive", and an enclosure [0, w] does NOT prove > 0 (the
            // value may be exactly zero — e.g. sqrt(exp(1) − e)).
            if bf_strictly_pos(&iv.lo) {
                return Sign::Positive;
            }
            if bf_strictly_neg(&iv.hi) {
                return Sign::Negative;
            }
            known_nonneg |= !bf_strictly_neg(&iv.lo);
            known_nonpos |= !bf_strictly_pos(&iv.hi);
            if known_nonneg && known_nonpos {
                return Sign::Zero;
            }
        }
        p *= 2;
    }
    match (evaluated, known_nonneg, known_nonpos) {
        (false, ..) => Sign::Unsupported,
        (true, true, false) => Sign::NonNegative,
        (true, false, true) => Sign::NonPositive,
        _ => Sign::Inseparable,
    }
}

/// An enclosure of a real value: lo ≤ value ≤ hi. Invariant: both finite.
struct Iv {
    lo: BigFloat,
    hi: BigFloat,
}

impl Iv {
    fn point(v: BigFloat) -> Option<Iv> {
        Iv::new(v.clone(), v)
    }

    fn new(lo: BigFloat, hi: BigFloat) -> Option<Iv> {
        if lo.is_nan() || hi.is_nan() || lo.is_inf() || hi.is_inf() {
            None
        } else {
            Some(Iv { lo, hi })
        }
    }

    fn contains_zero(&self) -> bool {
        !bf_strictly_pos(&self.lo) && !bf_strictly_neg(&self.hi)
    }
}

fn eval_iv(e: &Expr, p: usize, cc: &mut Consts) -> Option<Iv> {
    match e {
        Expr::Int(i) => Iv::point(exact_int(&i.to_string(), cc)?),
        Expr::Rat(r) => {
            let n = exact_int(&r.numer().to_string(), cc)?;
            let d = exact_int(&r.denom().to_string(), cc)?;
            Iv::new(n.div(&d, p, DOWN), n.div(&d, p, UP))
        }
        // A float is its exact binary value.
        Expr::Float(bf, _) => Iv::point(bf.clone()),
        Expr::Const(Constant::Pi) => Iv::new(cc.pi(p, DOWN), cc.pi(p, UP)),
        Expr::Const(Constant::E) => Iv::new(cc.e(p, DOWN), cc.e(p, UP)),
        Expr::Add(ts) => {
            let mut acc = Iv::point(BigFloat::from_i64(0, p.max(64)))?;
            for t in ts {
                acc = add_iv(&acc, &eval_iv(t, p, cc)?, p)?;
            }
            Some(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = Iv::point(BigFloat::from_i64(1, p.max(64)))?;
            for f in fs {
                acc = mul_iv(&acc, &eval_iv(f, p, cc)?, p)?;
            }
            Some(acc)
        }
        Expr::Pow(b, ex) => pow_iv(b, ex, p, cc),
        // An exact algebraic root: its isolating interval, refined to p
        // bits, IS a certified enclosure — endpoints convert with directed
        // rounding.
        Expr::Func(name, args) if name == "root" && args.len() == 2 => {
            let mut v = crate::algebraic::from_expr(e)?;
            crate::algebraic::refine_bits(&mut v, p);
            let (lo, hi) = v.bounds();
            let lo = {
                let n = exact_int(&lo.numer().to_string(), cc)?;
                let d = exact_int(&lo.denom().to_string(), cc)?;
                n.div(&d, p, DOWN)
            };
            let hi = {
                let n = exact_int(&hi.numer().to_string(), cc)?;
                let d = exact_int(&hi.denom().to_string(), cc)?;
                n.div(&d, p, UP)
            };
            Iv::new(lo, hi)
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let x = eval_iv(&args[0], p, cc)?;
            match name.as_str() {
                "sin" => lipschitz_iv(&x, p, |v, rm| v.sin(p, rm, cc)),
                "cos" => lipschitz_iv(&x, p, |v, rm| v.cos(p, rm, cc)),
                "tan" => {
                    let s = lipschitz_iv(&x, p, |v, rm| v.sin(p, rm, cc))?;
                    let c = lipschitz_iv(&x, p, |v, rm| v.cos(p, rm, cc))?;
                    mul_iv(&s, &recip_iv(&c, p)?, p)
                }
                "exp" => exp_iv(&x, p, cc),
                "ln" => {
                    if !bf_strictly_pos(&x.lo) {
                        return None;
                    }
                    Iv::new(x.lo.ln(p, DOWN, cc), x.hi.ln(p, UP, cc))
                }
                "abs" => abs_iv(&x, p),
                _ => None,
            }
        }
        // Symbols have no value; complex values have no order; matrices,
        // booleans, functions, structs, and equations are not real numbers.
        _ => None,
    }
}

/// Parse an integer exactly (enough mantissa bits for every digit). `None`
/// only for absurdly large inputs.
fn exact_int(digits: &str, cc: &mut Consts) -> Option<BigFloat> {
    // 4 bits per decimal digit over-covers; floor at one word (astro-float
    // returns NaN below 64 bits — see prec_bits_for in expr.rs).
    let bits = (digits.len() * 4 + 64).max(64);
    if bits > 4_000_000 {
        return None;
    }
    let v = BigFloat::parse(digits, RADIX, bits, NEAREST, cc);
    if v.is_nan() || v.is_inf() {
        None
    } else {
        Some(v)
    }
}

fn add_iv(a: &Iv, b: &Iv, p: usize) -> Option<Iv> {
    Iv::new(a.lo.add(&b.lo, p, DOWN), a.hi.add(&b.hi, p, UP))
}

fn neg_iv(a: &Iv) -> Option<Iv> {
    Iv::new(a.hi.neg(), a.lo.neg())
}

/// Interval product: extremes over the four endpoint products, each rounded
/// outward. Sound for any sign pattern.
fn mul_iv(a: &Iv, b: &Iv, p: usize) -> Option<Iv> {
    let mut lo: Option<BigFloat> = None;
    let mut hi: Option<BigFloat> = None;
    for x in [&a.lo, &a.hi] {
        for y in [&b.lo, &b.hi] {
            let down = x.mul(y, p, DOWN);
            let up = x.mul(y, p, UP);
            lo = Some(match lo {
                Some(m) if bf_lt(&m, &down) => m,
                _ => down,
            });
            hi = Some(match hi {
                Some(m) if bf_lt(&up, &m) => m,
                _ => up,
            });
        }
    }
    Iv::new(lo?, hi?)
}

/// 1/x, refusing intervals that straddle zero (a pole may hide inside).
fn recip_iv(a: &Iv, p: usize) -> Option<Iv> {
    if a.contains_zero() {
        return None;
    }
    let one = BigFloat::from_i64(1, p.max(64));
    Iv::new(one.div(&a.hi, p, DOWN), one.div(&a.lo, p, UP))
}

/// x^n for n ≥ 0 by interval squaring (generic interval multiplication
/// handles the even/odd sign cases on its own, just more loosely).
fn powi_iv(a: &Iv, n: u64, p: usize) -> Option<Iv> {
    let mut result = Iv::point(BigFloat::from_i64(1, p.max(64)))?;
    let mut base = Iv::new(a.lo.clone(), a.hi.clone())?;
    let mut n = n;
    while n > 0 {
        if n & 1 == 1 {
            result = mul_iv(&result, &base, p)?;
        }
        n >>= 1;
        if n > 0 {
            base = mul_iv(&base, &base, p)?;
        }
    }
    Some(result)
}

/// √x. A lower endpoint that dips below zero from outward rounding clamps to
/// zero — but ONLY when the radicand is provably nonnegative (symbolically):
/// a straddling enclosure whose true value is negative makes the sqrt
/// imaginary, and clamping would let the engine certify an ordering on a
/// complex number (e.g. `sqrt(pi − q) < 1` for a q agreeing with π to 20
/// digits). Without the proof we refuse at this precision; refinement will
/// separate a genuinely positive radicand on its own.
fn sqrt_iv(a: &Iv, p: usize, radicand_known_nonneg: bool) -> Option<Iv> {
    if a.hi.is_negative() {
        return None;
    }
    let lo = if a.lo.is_negative() {
        if !radicand_known_nonneg {
            return None;
        }
        BigFloat::from_i64(0, p.max(64))
    } else {
        a.lo.sqrt(p, DOWN)
    };
    Iv::new(lo, a.hi.sqrt(p, UP))
}

/// e^x. astro-float flushes underflow to *exact +0 even when rounding Up*
/// (inputs below ≈ −1.4885·10⁹ = EXPONENT_MIN·ln 2), which would put the
/// "upper bound" below the strictly positive true value and let the engine
/// certify `exp(-2200000000) <= 0`. A flushed upper endpoint becomes the
/// smallest representable positive instead; a flushed lower endpoint is
/// already sound (0 < e^x).
fn exp_iv(x: &Iv, p: usize, cc: &mut Consts) -> Option<Iv> {
    // bf_exp, never raw `.exp` — astro-float's exp drops the integer part of
    // its argument on wasm32 (see the wrapper's docs).
    let lo = crate::expr::bf_exp(&x.lo, p, DOWN, cc);
    let mut hi = crate::expr::bf_exp(&x.hi, p, UP, cc);
    if hi.is_zero() {
        hi = BigFloat::min_positive(p.max(64));
    }
    Iv::new(lo, hi)
}

fn abs_iv(a: &Iv, p: usize) -> Option<Iv> {
    if !a.lo.is_negative() {
        Iv::new(a.lo.clone(), a.hi.clone())
    } else if a.hi.is_negative() {
        neg_iv(a)
    } else {
        let mag = if bf_lt(&a.hi, &a.lo.neg()) {
            a.lo.neg()
        } else {
            a.hi.clone()
        };
        Iv::new(BigFloat::from_i64(0, p.max(64)), mag)
    }
}

/// Enclosure for a 1-Lipschitz function f (sin, cos): f([a,b]) lies within
/// f(a) ± (b−a), with f(a) itself bracketed by directed rounding.
fn lipschitz_iv(
    a: &Iv,
    p: usize,
    mut f: impl FnMut(&BigFloat, RoundingMode) -> BigFloat,
) -> Option<Iv> {
    let width = a.hi.sub(&a.lo, p, UP);
    let lo = f(&a.lo, DOWN).sub(&width, p, DOWN);
    let hi = f(&a.lo, UP).add(&width, p, UP);
    Iv::new(lo, hi)
}

/// base^exp: integer and half-integer exponents get tight monotone handling;
/// anything else goes through exp(exp·ln(base)), which needs base > 0.
fn pow_iv(base: &Expr, exp: &Expr, p: usize, cc: &mut Consts) -> Option<Iv> {
    let b = eval_iv(base, p, cc)?;
    if let Some(r) = numeric_value(exp) {
        if r.is_integer() {
            // unsigned_abs: `.abs()` would panic (or wrap past the cap in
            // release) on an exponent of exactly i64::MIN.
            let n = r
                .to_integer()
                .to_i64()
                .filter(|n| n.unsigned_abs() <= MAX_IV_EXP as u64)?;
            let m = powi_iv(&b, n.unsigned_abs(), p)?;
            return if n < 0 { recip_iv(&m, p) } else { Some(m) };
        }
        if *r.denom() == 2.into() {
            // x^(k/2) = (√x)^k — monotone, and covers all surds tightly.
            let k = r
                .numer()
                .to_i64()
                .filter(|k| k.unsigned_abs() <= MAX_IV_EXP as u64)?;
            let s = sqrt_iv(&b, p, known_nonneg(base))?;
            let m = powi_iv(&s, k.unsigned_abs(), p)?;
            return if k < 0 { recip_iv(&m, p) } else { Some(m) };
        }
    }
    // General real power: x^y = exp(y·ln x), defined for x > 0.
    if !bf_strictly_pos(&b.lo) {
        return None;
    }
    let ln_b = Iv::new(b.lo.ln(p, DOWN, cc), b.hi.ln(p, UP, cc))?;
    let y = eval_iv(exp, p, cc)?;
    let prod = mul_iv(&y, &ln_b, p)?;
    exp_iv(&prod, p, cc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{add, func, int, mul, pow, rat_to_expr, BigRational};
    use num_bigint::BigInt;

    fn rat(n: i64, d: i64) -> Expr {
        rat_to_expr(BigRational::new(BigInt::from(n), BigInt::from(d)))
    }

    /// The soundness of every enclosure rests on Up/Down meaning "toward
    /// ±∞" in astro-float. Pin that empirically: 1/3 must round to distinct
    /// bounds in the right order, on both signs.
    #[test]
    fn directed_rounding_semantics() {
        let mut cc = Consts::new().unwrap();
        let one = BigFloat::from_i64(1, 64);
        let neg_one = BigFloat::from_i64(-1, 64);
        let three = BigFloat::from_i64(3, 64);
        let lo = one.div(&three, 64, DOWN);
        let hi = one.div(&three, 64, UP);
        assert!(lo < hi, "Down must round below Up for positive values");
        let nlo = neg_one.div(&three, 64, DOWN);
        let nhi = neg_one.div(&three, 64, UP);
        assert!(nlo < nhi, "Down must round below Up for negative values");
        let pi_lo = cc.pi(64, DOWN);
        let pi_hi = cc.pi(64, UP);
        assert!(pi_lo < pi_hi);
    }

    fn sign_of(e: &Expr) -> Sign {
        certified_sign(e)
    }

    #[test]
    fn separates_clear_signs() {
        // π − 3 > 0
        let d = add(vec![Expr::Const(Constant::Pi), int(-3)]);
        assert!(matches!(sign_of(&d), Sign::Positive));
        // √2 + √3 − π > 0 (3.1462… vs 3.1415…)
        let s = add(vec![
            pow(int(2), rat(1, 2)),
            pow(int(3), rat(1, 2)),
            mul(vec![int(-1), Expr::Const(Constant::Pi)]),
        ]);
        assert!(matches!(sign_of(&s), Sign::Positive));
        // sin(1) − cos(1) > 0
        let t = add(vec![
            func("sin", vec![int(1)]),
            mul(vec![int(-1), func("cos", vec![int(1)])]),
        ]);
        assert!(matches!(sign_of(&t), Sign::Positive));
    }

    #[test]
    fn refuses_what_it_cannot_know() {
        assert!(matches!(
            sign_of(&Expr::Symbol("x".into())),
            Sign::Unsupported
        ));
        // (√2+√3)² − (5+2√6) is exactly 0 but not structurally 0: the
        // enclosure can never exclude zero, so the answer is "inseparable",
        // not a wrong sign.
        let lhs = pow(
            add(vec![pow(int(2), rat(1, 2)), pow(int(3), rat(1, 2))]),
            int(2),
        );
        let rhs = add(vec![int(5), mul(vec![int(2), pow(int(6), rat(1, 2))])]);
        let d = add(vec![lhs, mul(vec![int(-1), rhs])]);
        assert!(matches!(sign_of(&d), Sign::Inseparable));
    }

    #[test]
    fn decimal_preview_certifies_its_digits() {
        // Both endpoints of the enclosure round to the same string, so every
        // shown digit is proven — spot-check against known expansions.
        assert_eq!(
            decimal_preview(&Expr::Const(Constant::Pi), 6).as_deref(),
            Some("3.14159")
        );
        assert_eq!(decimal_preview(&rat(1, 3), 6).as_deref(), Some("0.333333"));
        assert_eq!(
            decimal_preview(&pow(int(2), rat(1, 2)), 6).as_deref(),
            Some("1.41421")
        );
        // Negative values keep their sign.
        assert_eq!(
            decimal_preview(&rat(-1, 7), 6).as_deref(),
            Some("-0.142857")
        );
    }

    #[test]
    fn decimal_preview_refuses_what_it_cannot_certify() {
        // Free symbols are not constants.
        assert_eq!(decimal_preview(&Expr::Symbol("x".into()), 6), None);
        // A value whose 6-digit rounding the small preview ceiling cannot
        // settle (an exact-but-not-structural zero straddles 0 forever:
        // "0" vs "-0.0000…" never agree).
        let lhs = pow(
            add(vec![pow(int(2), rat(1, 2)), pow(int(3), rat(1, 2))]),
            int(2),
        );
        let rhs = add(vec![int(5), mul(vec![int(2), pow(int(6), rat(1, 2))])]);
        let d = add(vec![lhs, mul(vec![int(-1), rhs])]);
        assert_eq!(decimal_preview(&d, 6), None);
    }
}
