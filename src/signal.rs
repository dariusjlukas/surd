//! Packed, certified bulk data — the "exact for design, certified for bulk"
//! half of the engine.
//!
//! A signal stores one interval [lo, hi] per sample, computed with outward
//! rounding at every step, so the true value of every sample *provably* lies
//! inside its interval. Operations widen the intervals; they never lie. The
//! display always shows the worst half-width, so the certified error bound
//! is impossible to miss.
//!
//! Two substrates (chosen at `signal(...)` time, never mixed implicitly):
//!
//! * **f64** — hardware floats, audio-scale fast. Arithmetic (+, −, ×, ÷, √)
//!   is rigorous outright: IEEE 754 guarantees correct rounding, and every
//!   result is widened by one ulp on each side. Transcendentals (sin, cos,
//!   tan, exp, ln) lean on the platform libm being within 2 ulp — the
//!   standard assumption, made visible here: those results are widened by
//!   8 ulps.
//! * **arbitrary precision** — astro-float with *directed* rounding, so even
//!   transcendentals are rigorous end-to-end, at the cost of speed.
//!
//! Exactness boundary: packing (`signal`) and reading back (`mid`, `bound`,
//! indexing) are the only crossings; mixing a signal into exact arithmetic
//! is an error by design.

use crate::expr::{
    bf_lt, bf_strictly_neg, bf_strictly_pos, float_to_rational, numeric_value, with_consts,
    BigRational, Expr,
};
use astro_float::{BigFloat, Consts, RoundingMode};
use num_traits::{FromPrimitive, ToPrimitive};
use std::fmt;
use std::rc::Rc;

/// FFT length cap (per call). 2^22 samples ≈ 95 s of 44.1 kHz audio.
pub const MAX_FFT_LEN: usize = 1 << 22;
/// Pairwise-product cap for direct convolution of signals.
pub const MAX_PAIRWISE: u128 = 1 << 28;

const UP: RoundingMode = RoundingMode::Up;
const DOWN: RoundingMode = RoundingMode::Down;

/// Widening for f64 results of correctly-rounded operations (IEEE arith).
const ARITH_ULPS: u32 = 1;
/// Widening for f64 libm transcendentals (assumed within 2 ulp; see module
/// docs) plus argument-interval slack.
const LIBM_ULPS: u32 = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum SignalData {
    F64 {
        lo: Vec<f64>,
        hi: Vec<f64>,
    },
    Big {
        lo: Vec<BigFloat>,
        hi: Vec<BigFloat>,
        /// Display digits; working precision is `prec_bits` of this.
        digits: usize,
    },
}

pub type Signal = Rc<SignalData>;

impl SignalData {
    pub fn len(&self) -> usize {
        match self {
            SignalData::F64 { lo, .. } => lo.len(),
            SignalData::Big { lo, .. } => lo.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One-line description for display: `<signal: 1024 samples, f64, max error ±2.2e-16>`.
impl fmt::Display for SignalData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.len();
        let samples = if n == 1 { "sample" } else { "samples" };
        match self {
            SignalData::F64 { .. } => {
                let hw = max_half_width_f64(self);
                if hw == 0.0 {
                    write!(f, "<signal: {} {}, f64, exact>", n, samples)
                } else {
                    write!(f, "<signal: {} {}, f64, max error ±{:.1e}>", n, samples, hw)
                }
            }
            SignalData::Big { digits, .. } => {
                let hw = max_half_width_f64(self);
                if hw == 0.0 {
                    write!(f, "<signal: {} {}, {} digits, exact>", n, samples, digits)
                } else {
                    write!(
                        f,
                        "<signal: {} {}, {} digits, max error ±{:.1e}>",
                        n, samples, digits, hw
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// f64 interval scalars
// ---------------------------------------------------------------------------

fn widen_down(mut x: f64, ulps: u32) -> f64 {
    for _ in 0..ulps {
        x = x.next_down();
    }
    x
}

fn widen_up(mut x: f64, ulps: u32) -> f64 {
    for _ in 0..ulps {
        x = x.next_up();
    }
    x
}

/// A sound f64 enclosure from a round-to-nearest computation.
fn iv_around(lo: f64, hi: f64, ulps: u32) -> Result<(f64, f64), String> {
    let (lo, hi) = (widen_down(lo, ulps), widen_up(hi, ulps));
    if lo.is_finite() && hi.is_finite() {
        Ok((lo, hi))
    } else {
        Err("overflow in signal computation (a sample left the f64 range)".into())
    }
}

fn f64_add(a: (f64, f64), b: (f64, f64)) -> Result<(f64, f64), String> {
    iv_around(a.0 + b.0, a.1 + b.1, ARITH_ULPS)
}

fn f64_sub(a: (f64, f64), b: (f64, f64)) -> Result<(f64, f64), String> {
    iv_around(a.0 - b.1, a.1 - b.0, ARITH_ULPS)
}

fn f64_mul(a: (f64, f64), b: (f64, f64)) -> Result<(f64, f64), String> {
    let p = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
    let lo = p.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = p.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    iv_around(lo, hi, ARITH_ULPS)
}

fn f64_div(a: (f64, f64), b: (f64, f64)) -> Result<(f64, f64), String> {
    if b.0 <= 0.0 && b.1 >= 0.0 {
        return Err("division by an interval containing zero (a sample's divisor may be 0)".into());
    }
    let p = [a.0 / b.0, a.0 / b.1, a.1 / b.0, a.1 / b.1];
    let lo = p.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = p.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    iv_around(lo, hi, ARITH_ULPS)
}

/// Unary f64 interval functions. Monotone ones map endpoints; sin/cos use
/// the 1-Lipschitz midpoint bound (sound for any interval width).
fn f64_unary(name: &str, a: (f64, f64)) -> Result<(f64, f64), String> {
    match name {
        "neg" => Ok((-a.1, -a.0)),
        "abs" => Ok(if a.0 >= 0.0 {
            a
        } else if a.1 <= 0.0 {
            (-a.1, -a.0)
        } else {
            (0.0, (-a.0).max(a.1))
        }),
        "sqrt" => {
            if a.1 < 0.0 {
                return Err("sqrt of a negative sample (signals are real-valued)".into());
            }
            iv_around(a.0.max(0.0).sqrt(), a.1.sqrt(), ARITH_ULPS)
        }
        "exp" => iv_around(a.0.exp(), a.1.exp(), LIBM_ULPS),
        "ln" => {
            if a.0 <= 0.0 {
                return Err("ln of a sample interval reaching zero or below".into());
            }
            iv_around(a.0.ln(), a.1.ln(), LIBM_ULPS)
        }
        "sin" | "cos" => {
            let width = a.1 - a.0;
            let v = if name == "sin" { a.0.sin() } else { a.0.cos() };
            let enc = iv_around(v - width, v + width, LIBM_ULPS)?;
            Ok((enc.0.max(-1.0), enc.1.min(1.0)))
        }
        "tan" => {
            let s = f64_unary("sin", a)?;
            let c = f64_unary("cos", a)?;
            f64_div(s, c).map_err(|_| "tan of a sample interval containing a pole".into())
        }
        _ => Err(format!("'{}' is not defined on signals", name)),
    }
}

// ---------------------------------------------------------------------------
// BigFloat interval scalars (directed rounding — rigorous end to end)
// ---------------------------------------------------------------------------

fn prec_bits(digits: usize) -> usize {
    (((digits as f64) * std::f64::consts::LOG2_10).ceil() as usize + 32).max(64)
}

fn big_check(lo: BigFloat, hi: BigFloat) -> Result<(BigFloat, BigFloat), String> {
    if lo.is_nan() || hi.is_nan() || lo.is_inf() || hi.is_inf() {
        Err("overflow or undefined value in signal computation".into())
    } else {
        Ok((lo, hi))
    }
}

fn big_add(a: (&BigFloat, &BigFloat), b: (&BigFloat, &BigFloat), p: usize) -> Result<(BigFloat, BigFloat), String> {
    big_check(a.0.add(b.0, p, DOWN), a.1.add(b.1, p, UP))
}

fn big_sub(a: (&BigFloat, &BigFloat), b: (&BigFloat, &BigFloat), p: usize) -> Result<(BigFloat, BigFloat), String> {
    big_check(a.0.sub(b.1, p, DOWN), a.1.sub(b.0, p, UP))
}

fn big_mul(a: (&BigFloat, &BigFloat), b: (&BigFloat, &BigFloat), p: usize) -> Result<(BigFloat, BigFloat), String> {
    let mut lo: Option<BigFloat> = None;
    let mut hi: Option<BigFloat> = None;
    for x in [a.0, a.1] {
        for y in [b.0, b.1] {
            let d = x.mul(y, p, DOWN);
            let u = x.mul(y, p, UP);
            lo = Some(match lo {
                Some(m) if bf_lt(&m, &d) => m,
                _ => d,
            });
            hi = Some(match hi {
                Some(m) if bf_lt(&u, &m) => m,
                _ => u,
            });
        }
    }
    big_check(lo.unwrap(), hi.unwrap())
}

fn big_div(a: (&BigFloat, &BigFloat), b: (&BigFloat, &BigFloat), p: usize) -> Result<(BigFloat, BigFloat), String> {
    if !bf_strictly_pos(b.0) && !bf_strictly_neg(b.1) {
        return Err("division by an interval containing zero (a sample's divisor may be 0)".into());
    }
    let mut lo: Option<BigFloat> = None;
    let mut hi: Option<BigFloat> = None;
    for x in [a.0, a.1] {
        for y in [b.0, b.1] {
            let d = x.div(y, p, DOWN);
            let u = x.div(y, p, UP);
            lo = Some(match lo {
                Some(m) if bf_lt(&m, &d) => m,
                _ => d,
            });
            hi = Some(match hi {
                Some(m) if bf_lt(&u, &m) => m,
                _ => u,
            });
        }
    }
    big_check(lo.unwrap(), hi.unwrap())
}

fn big_unary(
    name: &str,
    a: (&BigFloat, &BigFloat),
    p: usize,
    cc: &mut Consts,
) -> Result<(BigFloat, BigFloat), String> {
    let zero = BigFloat::from_i64(0, p.max(64));
    match name {
        "neg" => big_check(a.1.neg(), a.0.neg()),
        "abs" => {
            if !a.0.is_negative() {
                big_check(a.0.clone(), a.1.clone())
            } else if a.1.is_negative() {
                big_check(a.1.neg(), a.0.neg())
            } else {
                let neg_lo = a.0.neg();
                let mag = if bf_lt(a.1, &neg_lo) { neg_lo } else { a.1.clone() };
                big_check(zero, mag)
            }
        }
        "sqrt" => {
            if a.1.is_negative() {
                return Err("sqrt of a negative sample (signals are real-valued)".into());
            }
            let lo = if a.0.is_negative() { zero } else { a.0.sqrt(p, DOWN) };
            big_check(lo, a.1.sqrt(p, UP))
        }
        "exp" => big_check(a.0.exp(p, DOWN, cc), a.1.exp(p, UP, cc)),
        "ln" => {
            if !bf_strictly_pos(a.0) {
                return Err("ln of a sample interval reaching zero or below".into());
            }
            big_check(a.0.ln(p, DOWN, cc), a.1.ln(p, UP, cc))
        }
        "sin" | "cos" => {
            // 1-Lipschitz midpoint bound, directed at every step.
            let width = a.1.sub(a.0, p, UP);
            let (d, u) = if name == "sin" {
                (a.0.sin(p, DOWN, cc), a.0.sin(p, UP, cc))
            } else {
                (a.0.cos(p, DOWN, cc), a.0.cos(p, UP, cc))
            };
            big_check(d.sub(&width, p, DOWN), u.add(&width, p, UP))
        }
        "tan" => {
            let s = big_unary("sin", a, p, cc)?;
            let c = big_unary("cos", a, p, cc)?;
            big_div((&s.0, &s.1), (&c.0, &c.1), p)
                .map_err(|_| "tan of a sample interval containing a pole".into())
        }
        _ => Err(format!("'{}' is not defined on signals", name)),
    }
}

// ---------------------------------------------------------------------------
// Packing and reading back (the exact↔certified boundary)
// ---------------------------------------------------------------------------

/// A sound f64 enclosure of an exact rational, by nudging a nearest-ish
/// approximation outward until exact containment is verified.
fn rat_to_f64_iv(r: &BigRational) -> Result<(f64, f64), String> {
    let approx = r.to_f64().filter(|v| v.is_finite()).ok_or_else(|| {
        "an entry exceeds the f64 range — use signal(v, digits) for arbitrary precision"
            .to_string()
    })?;
    let mut lo = approx;
    let mut hi = approx;
    for _ in 0..64 {
        if BigRational::from_f64(lo).is_some_and(|v| v <= *r) {
            break;
        }
        lo = lo.next_down();
    }
    for _ in 0..64 {
        if BigRational::from_f64(hi).is_some_and(|v| v >= *r) {
            break;
        }
        hi = hi.next_up();
    }
    let contained = BigRational::from_f64(lo).is_some_and(|v| v <= *r)
        && BigRational::from_f64(hi).is_some_and(|v| v >= *r);
    if contained && lo.is_finite() && hi.is_finite() {
        Ok((lo, hi))
    } else {
        Err("could not enclose an entry in f64 (use signal(v, digits))".into())
    }
}

fn rat_to_big_iv(r: &BigRational, p: usize, cc: &mut Consts) -> Result<(BigFloat, BigFloat), String> {
    let radix = astro_float::Radix::Dec;
    let nearest = RoundingMode::ToEven;
    // Numerator and denominator parse exactly given enough bits, then a
    // directed division gives the enclosure.
    let bits = |s: &str| (s.len() * 4 + 64).max(64);
    let ns = r.numer().to_string();
    let ds = r.denom().to_string();
    if bits(&ns).max(bits(&ds)) > 4_000_000 {
        return Err("entry too large to pack".into());
    }
    let n = BigFloat::parse(&ns, radix, bits(&ns), nearest, cc);
    let d = BigFloat::parse(&ds, radix, bits(&ds), nearest, cc);
    big_check(n.div(&d, p, DOWN), n.div(&d, p, UP))
}

/// The exact value of an entry, for packing: rationals directly, floats via
/// their exact binary value. Symbolic entries refuse.
fn entry_rational(e: &Expr) -> Result<BigRational, String> {
    match e {
        Expr::Float(bf, _) => float_to_rational(bf)
            .ok_or_else(|| "cannot pack a non-finite float".to_string()),
        other => numeric_value(other).ok_or_else(|| {
            format!(
                "signal needs numeric entries, got '{}' — evaluate symbolic values first (N, subs)",
                other
            )
        }),
    }
}

/// Pack exact entries into a signal: f64 substrate by default, arbitrary
/// precision when `digits` is given.
pub fn pack(entries: &[Expr], digits: Option<usize>) -> Result<SignalData, String> {
    match digits {
        None => {
            let mut lo = Vec::with_capacity(entries.len());
            let mut hi = Vec::with_capacity(entries.len());
            for e in entries {
                let (l, h) = rat_to_f64_iv(&entry_rational(e)?)?;
                lo.push(l);
                hi.push(h);
            }
            Ok(SignalData::F64 { lo, hi })
        }
        Some(digits) => {
            let digits = digits.clamp(1, 100_000);
            let p = prec_bits(digits);
            with_consts(|cc| -> Result<SignalData, String> {
                let mut lo = Vec::with_capacity(entries.len());
                let mut hi = Vec::with_capacity(entries.len());
                for e in entries {
                    let (l, h) = rat_to_big_iv(&entry_rational(e)?, p, cc)?;
                    lo.push(l);
                    hi.push(h);
                }
                Ok(SignalData::Big { lo, hi, digits })
            })?
        }
    }
}

/// The midpoint of sample `i` (0-based), as a Float expression.
pub fn midpoint(s: &SignalData, i: usize) -> Expr {
    match s {
        SignalData::F64 { lo, hi } => {
            let m = lo[i] / 2.0 + hi[i] / 2.0;
            Expr::Float(BigFloat::from_f64(m, 64), 17)
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            let two = BigFloat::from_i64(2, p);
            let m = lo[i]
                .add(&hi[i], p, RoundingMode::ToEven)
                .div(&two, p, RoundingMode::ToEven);
            Expr::Float(m, *digits)
        }
    }
}

/// Column matrix of all midpoints — back to exact-land for export/printing.
pub fn mid_matrix(s: &SignalData) -> Expr {
    let rows = (0..s.len()).map(|i| vec![midpoint(s, i)]).collect();
    Expr::Matrix(rows)
}

/// Certified bound on |true value − midpoint| for sample `i`: the larger of
/// the two distances from the *rounded* midpoint to the interval ends,
/// computed upward — so the bound also covers the midpoint's own
/// representation error, not just the enclosure half-width.
fn deviation_f64(s: &SignalData, i: usize) -> f64 {
    match s {
        SignalData::F64 { lo, hi } => {
            // A point interval has an exact midpoint: deviation exactly 0.
            if lo[i] == hi[i] {
                return 0.0;
            }
            let m = lo[i] / 2.0 + hi[i] / 2.0;
            widen_up((m - lo[i]).max(hi[i] - m).max(0.0), 1)
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            if hi[i].sub(&lo[i], p, UP).is_zero() {
                return 0.0;
            }
            let Expr::Float(m, _) = midpoint(s, i) else {
                unreachable!("midpoint is always a float");
            };
            let a = m.sub(&lo[i], p, UP);
            let b = hi[i].sub(&m, p, UP);
            let d = if bf_lt(&a, &b) { b } else { a };
            // Display approximation only — the rigorous bound is `d`.
            big_to_f64_upper(&d)
        }
    }
}

fn max_half_width_f64(s: &SignalData) -> f64 {
    (0..s.len()).map(|i| deviation_f64(s, i)).fold(0.0, f64::max)
}

/// An f64 ≥ the BigFloat value, for display purposes. Round-trips through
/// the decimal renderer used by Float display.
fn big_to_f64_upper(x: &BigFloat) -> f64 {
    float_to_rational(x)
        .and_then(|r| r.to_f64())
        .map(|v| widen_up(v, 4))
        .unwrap_or(f64::INFINITY)
}

/// The certified bound on |true value − mid| for sample `i`, or the maximum
/// over all samples — so `mid(s)` ± `bound(s)` is always a true statement.
pub fn half_width(s: &SignalData, i: Option<usize>) -> Expr {
    let hw = match i {
        Some(i) => deviation_f64(s, i),
        None => max_half_width_f64(s),
    };
    Expr::Float(BigFloat::from_f64(hw, 64), 3)
}

// ---------------------------------------------------------------------------
// Elementwise operations
// ---------------------------------------------------------------------------

/// Elementwise binary op between two signals of the same substrate & length.
pub fn binop(op: &str, a: &SignalData, b: &SignalData) -> Result<SignalData, String> {
    if a.len() != b.len() {
        return Err(format!(
            "signals must have the same length, got {} and {}",
            a.len(),
            b.len()
        ));
    }
    match (a, b) {
        (SignalData::F64 { lo: al, hi: ah }, SignalData::F64 { lo: bl, hi: bh }) => {
            let mut lo = Vec::with_capacity(a.len());
            let mut hi = Vec::with_capacity(a.len());
            for i in 0..a.len() {
                let (l, h) = f64_binop_one(op, (al[i], ah[i]), (bl[i], bh[i]))
                    .map_err(|e| format!("{} (sample {})", e, i + 1))?;
                lo.push(l);
                hi.push(h);
            }
            Ok(SignalData::F64 { lo, hi })
        }
        (
            SignalData::Big { lo: al, hi: ah, digits },
            SignalData::Big { lo: bl, hi: bh, digits: bd },
        ) => {
            let digits = (*digits).max(*bd);
            let p = prec_bits(digits);
            let mut lo = Vec::with_capacity(a.len());
            let mut hi = Vec::with_capacity(a.len());
            for i in 0..a.len() {
                let (l, h) = big_binop_one(op, (&al[i], &ah[i]), (&bl[i], &bh[i]), p)
                    .map_err(|e| format!("{} (sample {})", e, i + 1))?;
                lo.push(l);
                hi.push(h);
            }
            Ok(SignalData::Big { lo, hi, digits })
        }
        _ => Err(
            "cannot mix f64 and arbitrary-precision signals — repack one side with signal(...)"
                .into(),
        ),
    }
}

fn f64_binop_one(op: &str, a: (f64, f64), b: (f64, f64)) -> Result<(f64, f64), String> {
    match op {
        "+" => f64_add(a, b),
        "-" => f64_sub(a, b),
        "*" => f64_mul(a, b),
        "/" => f64_div(a, b),
        _ => unreachable!("unknown signal binop"),
    }
}

fn big_binop_one(
    op: &str,
    a: (&BigFloat, &BigFloat),
    b: (&BigFloat, &BigFloat),
    p: usize,
) -> Result<(BigFloat, BigFloat), String> {
    match op {
        "+" => big_add(a, b, p),
        "-" => big_sub(a, b, p),
        "*" => big_mul(a, b, p),
        "/" => big_div(a, b, p),
        _ => unreachable!("unknown signal binop"),
    }
}

/// Broadcast an exact scalar against a signal.
pub fn scalar_binop(
    op: &str,
    s: &SignalData,
    scalar: &Expr,
    scalar_on_left: bool,
) -> Result<SignalData, String> {
    let r = entry_rational(scalar)
        .map_err(|_| format!("cannot mix '{}' into a signal — only numbers broadcast", scalar))?;
    let broadcast = constant(s, &r)?;
    if scalar_on_left {
        binop(op, &broadcast, s)
    } else {
        binop(op, s, &broadcast)
    }
}

/// A constant signal in the same substrate/length as `like`.
fn constant(like: &SignalData, r: &BigRational) -> Result<SignalData, String> {
    match like {
        SignalData::F64 { lo, .. } => {
            let (l, h) = rat_to_f64_iv(r)?;
            Ok(SignalData::F64 {
                lo: vec![l; lo.len()],
                hi: vec![h; lo.len()],
            })
        }
        SignalData::Big { lo, digits, .. } => {
            let p = prec_bits(*digits);
            let (l, h) = with_consts(|cc| rat_to_big_iv(r, p, cc))??;
            Ok(SignalData::Big {
                lo: vec![l; lo.len()],
                hi: vec![h; lo.len()],
                digits: *digits,
            })
        }
    }
}

/// Elementwise unary function over a signal.
pub fn unary(name: &str, s: &SignalData) -> Result<SignalData, String> {
    match s {
        SignalData::F64 { lo, hi } => {
            let mut nlo = Vec::with_capacity(lo.len());
            let mut nhi = Vec::with_capacity(lo.len());
            for i in 0..lo.len() {
                let (l, h) = f64_unary(name, (lo[i], hi[i]))
                    .map_err(|e| format!("{} (sample {})", e, i + 1))?;
                nlo.push(l);
                nhi.push(h);
            }
            Ok(SignalData::F64 { lo: nlo, hi: nhi })
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            with_consts(|cc| -> Result<SignalData, String> {
                let mut nlo = Vec::with_capacity(lo.len());
                let mut nhi = Vec::with_capacity(lo.len());
                for i in 0..lo.len() {
                    let (l, h) = big_unary(name, (&lo[i], &hi[i]), p, cc)
                        .map_err(|e| format!("{} (sample {})", e, i + 1))?;
                    nlo.push(l);
                    nhi.push(h);
                }
                Ok(SignalData::Big {
                    lo: nlo,
                    hi: nhi,
                    digits: *digits,
                })
            })?
        }
    }
}

/// Integer power, elementwise, by repeated interval squaring.
pub fn powi(s: &SignalData, n: i64) -> Result<SignalData, String> {
    if !(0..=64).contains(&n) {
        return Err("signal exponents must be integers in 0..=64".into());
    }
    // Square-and-multiply over whole signals (a few elementwise passes).
    let mut result = constant(s, &BigRational::from_integer(1.into()))?;
    let mut base = s.clone();
    let mut n = n as u64;
    while n > 0 {
        if n & 1 == 1 {
            result = binop("*", &result, &base)?;
        }
        n >>= 1;
        if n > 0 {
            base = binop("*", &base, &base)?;
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Bulk algorithms: convolution and FFT
// ---------------------------------------------------------------------------

/// Direct interval convolution, length m+n−1.
pub fn conv(a: &SignalData, b: &SignalData) -> Result<SignalData, String> {
    let (m, n) = (a.len(), b.len());
    if (m as u128).saturating_mul(n as u128) > MAX_PAIRWISE {
        return Err(format!(
            "convolution too large ({}×{} products; cap {})",
            m, n, MAX_PAIRWISE
        ));
    }
    match (a, b) {
        (SignalData::F64 { lo: al, hi: ah }, SignalData::F64 { lo: bl, hi: bh }) => {
            let len = m + n - 1;
            let mut lo = vec![0.0f64; len];
            let mut hi = vec![0.0f64; len];
            for j in 0..m {
                for k in 0..n {
                    let prod = f64_mul((al[j], ah[j]), (bl[k], bh[k]))?;
                    let acc = f64_add((lo[j + k], hi[j + k]), prod)?;
                    lo[j + k] = acc.0;
                    hi[j + k] = acc.1;
                }
            }
            Ok(SignalData::F64 { lo, hi })
        }
        (
            SignalData::Big { lo: al, hi: ah, digits },
            SignalData::Big { lo: bl, hi: bh, digits: bd },
        ) => {
            let digits = (*digits).max(*bd);
            let p = prec_bits(digits);
            let len = m + n - 1;
            let zero = BigFloat::from_i64(0, p);
            let mut lo = vec![zero.clone(); len];
            let mut hi = vec![zero; len];
            for j in 0..m {
                for k in 0..n {
                    let prod = big_mul((&al[j], &ah[j]), (&bl[k], &bh[k]), p)?;
                    let acc = big_add((&lo[j + k], &hi[j + k]), (&prod.0, &prod.1), p)?;
                    lo[j + k] = acc.0;
                    hi[j + k] = acc.1;
                }
            }
            Ok(SignalData::Big { lo, hi, digits })
        }
        _ => Err(
            "cannot mix f64 and arbitrary-precision signals — repack one side with signal(...)"
                .into(),
        ),
    }
}

/// Zero-pad (or refuse to truncate) to length `n`.
pub fn pad(s: &SignalData, n: usize) -> Result<SignalData, String> {
    if n < s.len() {
        return Err(format!(
            "dsp.pad never truncates: the signal has {} samples, asked for {}",
            s.len(),
            n
        ));
    }
    match s {
        SignalData::F64 { lo, hi } => {
            let mut lo = lo.clone();
            let mut hi = hi.clone();
            lo.resize(n, 0.0);
            hi.resize(n, 0.0);
            Ok(SignalData::F64 { lo, hi })
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            let zero = BigFloat::from_i64(0, p);
            let mut lo = lo.clone();
            let mut hi = hi.clone();
            lo.resize(n, zero.clone());
            hi.resize(n, zero);
            Ok(SignalData::Big {
                lo,
                hi,
                digits: *digits,
            })
        }
    }
}

/// Radix-2 FFT over interval complex pairs. Returns (re, im) signals.
/// Forward kernel e^(−2πi·kj/n); inverse applies the +i kernel and the 1/n
/// factor (exact — n is a power of two, so the division is lossless).
pub fn fft(
    re: &SignalData,
    im: Option<&SignalData>,
    inverse: bool,
) -> Result<(SignalData, SignalData), String> {
    let n = re.len();
    if !n.is_power_of_two() {
        return Err(format!(
            "fft length must be a power of two, got {} (zero-pad with dsp.pad)",
            n
        ));
    }
    if n > MAX_FFT_LEN {
        return Err(format!("fft length {} exceeds the cap {}", n, MAX_FFT_LEN));
    }
    if let Some(im) = im {
        if im.len() != n {
            return Err("fft: real and imaginary parts must have the same length".into());
        }
    }
    match re {
        SignalData::F64 { .. } => fft_f64(re, im, inverse),
        SignalData::Big { .. } => fft_big(re, im, inverse),
    }
}

fn bit_reverse_permute<T>(v: &mut [T]) {
    let n = v.len();
    if n <= 1 {
        return; // a length-1 transform is the identity (and the shift below would be 64)
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            v.swap(i, j);
        }
    }
}

fn fft_f64(
    re: &SignalData,
    im: Option<&SignalData>,
    inverse: bool,
) -> Result<(SignalData, SignalData), String> {
    let n = re.len();
    let (rl, rh) = match re {
        SignalData::F64 { lo, hi } => (lo, hi),
        _ => unreachable!(),
    };
    let zeros = (vec![0.0; n], vec![0.0; n]);
    let (il, ih) = match im {
        Some(SignalData::F64 { lo, hi }) => (lo, hi),
        None => (&zeros.0, &zeros.1),
        _ => return Err("fft: real and imaginary parts must share a substrate".into()),
    };
    // (re_lo, re_hi, im_lo, im_hi) per element.
    let mut buf: Vec<(f64, f64, f64, f64)> = (0..n)
        .map(|i| (rl[i], rh[i], il[i], ih[i]))
        .collect();
    bit_reverse_permute(&mut buf);

    let mut len = 2;
    while len <= n {
        let half = len / 2;
        for k in 0..half {
            // Twiddle e^(∓2πi·k/len), widened for libm slack.
            let theta = 2.0 * std::f64::consts::PI * (k as f64) / (len as f64);
            let (c, s) = (theta.cos(), theta.sin());
            let s = if inverse { s } else { -s };
            let w_re = (widen_down(c, LIBM_ULPS), widen_up(c, LIBM_ULPS));
            let w_im = (widen_down(s, LIBM_ULPS), widen_up(s, LIBM_ULPS));
            let mut i = k;
            while i < n {
                let j = i + half;
                let (bre, bim) = ((buf[j].0, buf[j].1), (buf[j].2, buf[j].3));
                // t = w · buf[j]  (complex interval product)
                let t_re = f64_sub(f64_mul(w_re, bre)?, f64_mul(w_im, bim)?)?;
                let t_im = f64_add(f64_mul(w_re, bim)?, f64_mul(w_im, bre)?)?;
                let (are, aim) = ((buf[i].0, buf[i].1), (buf[i].2, buf[i].3));
                let sum_re = f64_add(are, t_re)?;
                let sum_im = f64_add(aim, t_im)?;
                let dif_re = f64_sub(are, t_re)?;
                let dif_im = f64_sub(aim, t_im)?;
                buf[i] = (sum_re.0, sum_re.1, sum_im.0, sum_im.1);
                buf[j] = (dif_re.0, dif_re.1, dif_im.0, dif_im.1);
                i += len;
            }
        }
        len *= 2;
    }

    let scale = if inverse { 1.0 / (n as f64) } else { 1.0 }; // exact: n is 2^k
    let mut out = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    for (a, b, c, d) in buf {
        out.0.push(a * scale);
        out.1.push(b * scale);
        out.2.push(c * scale);
        out.3.push(d * scale);
    }
    Ok((
        SignalData::F64 { lo: out.0, hi: out.1 },
        SignalData::F64 { lo: out.2, hi: out.3 },
    ))
}

fn fft_big(
    re: &SignalData,
    im: Option<&SignalData>,
    inverse: bool,
) -> Result<(SignalData, SignalData), String> {
    let n = re.len();
    let (rl, rh, digits) = match re {
        SignalData::Big { lo, hi, digits } => (lo, hi, *digits),
        _ => unreachable!(),
    };
    let p = prec_bits(digits);
    let zero = BigFloat::from_i64(0, p);
    let zeros = (vec![zero.clone(); n], vec![zero.clone(); n]);
    let (il, ih) = match im {
        Some(SignalData::Big { lo, hi, .. }) => (lo, hi),
        None => (&zeros.0, &zeros.1),
        _ => return Err("fft: real and imaginary parts must share a substrate".into()),
    };
    let mut buf: Vec<(BigFloat, BigFloat, BigFloat, BigFloat)> = (0..n)
        .map(|i| (rl[i].clone(), rh[i].clone(), il[i].clone(), ih[i].clone()))
        .collect();
    bit_reverse_permute(&mut buf);

    with_consts(|cc| -> Result<(), String> {
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            for k in 0..half {
                // Angle 2πk/len as a directed interval, then Lipschitz trig.
                let pi_lo = cc.pi(p, DOWN);
                let pi_hi = cc.pi(p, UP);
                let ratio = BigRational::new((2 * k as i64).into(), (len as i64).into());
                let (q_lo, q_hi) = rat_to_big_iv(&ratio, p, cc)?;
                let ang = big_mul((&q_lo, &q_hi), (&pi_lo, &pi_hi), p)?;
                let w_re = big_unary("cos", (&ang.0, &ang.1), p, cc)?;
                let mut w_im = big_unary("sin", (&ang.0, &ang.1), p, cc)?;
                if !inverse {
                    w_im = (w_im.1.neg(), w_im.0.neg());
                }
                let mut i = k;
                while i < n {
                    let j = i + half;
                    let bre = (&buf[j].0, &buf[j].1);
                    let bim = (&buf[j].2, &buf[j].3);
                    let m1 = big_mul((&w_re.0, &w_re.1), bre, p)?;
                    let m2 = big_mul((&w_im.0, &w_im.1), bim, p)?;
                    let m3 = big_mul((&w_re.0, &w_re.1), bim, p)?;
                    let m4 = big_mul((&w_im.0, &w_im.1), bre, p)?;
                    let t_re = big_sub((&m1.0, &m1.1), (&m2.0, &m2.1), p)?;
                    let t_im = big_add((&m3.0, &m3.1), (&m4.0, &m4.1), p)?;
                    let are = (buf[i].0.clone(), buf[i].1.clone());
                    let aim = (buf[i].2.clone(), buf[i].3.clone());
                    let sum_re = big_add((&are.0, &are.1), (&t_re.0, &t_re.1), p)?;
                    let sum_im = big_add((&aim.0, &aim.1), (&t_im.0, &t_im.1), p)?;
                    let dif_re = big_sub((&are.0, &are.1), (&t_re.0, &t_re.1), p)?;
                    let dif_im = big_sub((&aim.0, &aim.1), (&t_im.0, &t_im.1), p)?;
                    buf[i] = (sum_re.0, sum_re.1, sum_im.0, sum_im.1);
                    buf[j] = (dif_re.0, dif_re.1, dif_im.0, dif_im.1);
                    i += len;
                }
            }
            len *= 2;
        }
        Ok(())
    })??;

    let mut out = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let nn = BigFloat::from_i64(n as i64, p);
    for (a, b, c, d) in buf {
        if inverse {
            // n = 2^k: lossless in binary floating point.
            out.0.push(a.div(&nn, p, DOWN));
            out.1.push(b.div(&nn, p, UP));
            out.2.push(c.div(&nn, p, DOWN));
            out.3.push(d.div(&nn, p, UP));
        } else {
            out.0.push(a);
            out.1.push(b);
            out.2.push(c);
            out.3.push(d);
        }
    }
    Ok((
        SignalData::Big { lo: out.0, hi: out.1, digits },
        SignalData::Big { lo: out.2, hi: out.3, digits },
    ))
}

// ---------------------------------------------------------------------------
// Certified reductions
// ---------------------------------------------------------------------------

/// Certified upper bound on max |x[i]| — the peak.
pub fn peak(s: &SignalData) -> Expr {
    match s {
        SignalData::F64 { lo, hi } => {
            let v = lo
                .iter()
                .zip(hi)
                .map(|(l, h)| l.abs().max(h.abs()))
                .fold(0.0, f64::max);
            Expr::Float(BigFloat::from_f64(v, 64), 17)
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            let mut max = BigFloat::from_i64(0, p);
            for (l, h) in lo.iter().zip(hi) {
                let neg_l = l.neg();
                let m = if bf_lt(h, &neg_l) { neg_l } else { h.clone() };
                if bf_lt(&max, &m) {
                    max = m;
                }
            }
            Expr::Float(max, *digits)
        }
    }
}

/// Certified upper bound on the RMS √(Σx²/n).
pub fn rms(s: &SignalData) -> Result<Expr, String> {
    let sq = binop("*", s, s)?;
    match &sq {
        SignalData::F64 { hi, .. } => {
            let mut acc = (0.0f64, 0.0f64);
            let lo_v = match &sq {
                SignalData::F64 { lo, .. } => lo,
                _ => unreachable!(),
            };
            for i in 0..hi.len() {
                acc = f64_add(acc, (lo_v[i], hi[i]))?;
            }
            let n = hi.len() as f64;
            let upper = widen_up((acc.1 / n).sqrt(), 2);
            Ok(Expr::Float(BigFloat::from_f64(upper, 64), 17))
        }
        SignalData::Big { hi, digits, .. } => {
            let p = prec_bits(*digits);
            let mut acc = BigFloat::from_i64(0, p);
            for h in hi {
                acc = acc.add(h, p, UP);
            }
            let n = BigFloat::from_i64(hi.len() as i64, p);
            let upper = acc.div(&n, p, UP).sqrt(p, UP);
            Ok(Expr::Float(upper, *digits))
        }
    }
}
