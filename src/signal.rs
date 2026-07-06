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
    bf_from_f64_exact, bf_lt, bf_strictly_neg, bf_strictly_pos, float_to_rational, numeric_value,
    with_consts, BigRational, Expr,
};
use astro_float::{BigFloat, Consts, RoundingMode};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
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
    /// A complex signal: two real sub-signals (re + i·im). Invariant
    /// (enforced by [`complex`]): `re` and `im` are themselves real (`F64` or
    /// `Big`, never `Complex`), the same length, and the same substrate. This
    /// reuses every real kernel — complex add/mul/abs are composed from the
    /// real `binop`/`unary` on the two parts, so both substrates come for free.
    Complex {
        re: Box<SignalData>,
        im: Box<SignalData>,
    },
}

pub type Signal = Rc<SignalData>;

impl SignalData {
    pub fn len(&self) -> usize {
        match self {
            SignalData::F64 { lo, .. } => lo.len(),
            SignalData::Big { lo, .. } => lo.len(),
            SignalData::Complex { re, .. } => re.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Build a complex signal from real parts. They must be real (not themselves
/// complex), the same length, and share a substrate (both `F64`, or both `Big`
/// with the same `digits`).
pub fn complex(re: SignalData, im: SignalData) -> Result<SignalData, String> {
    if matches!(re, SignalData::Complex { .. }) || matches!(im, SignalData::Complex { .. }) {
        return Err("a complex signal's parts must be real, not complex".into());
    }
    if re.len() != im.len() {
        return Err(format!(
            "complex signal parts must have the same length, got {} and {}",
            re.len(),
            im.len()
        ));
    }
    let same_substrate = match (&re, &im) {
        (SignalData::F64 { .. }, SignalData::F64 { .. }) => true,
        (SignalData::Big { digits: a, .. }, SignalData::Big { digits: b, .. }) => a == b,
        _ => false,
    };
    if !same_substrate {
        return Err(
            "complex signal parts must share a substrate — both f64, or both \
                    arbitrary-precision with the same digits"
                .into(),
        );
    }
    Ok(SignalData::Complex {
        re: Box::new(re),
        im: Box::new(im),
    })
}

/// The real signal whose substrate/length a constant should match: a complex
/// signal's real part, or the signal itself. (A real `Complex`'s parts are
/// real, so the result is always `F64`/`Big`.)
fn real_substrate(s: &SignalData) -> &SignalData {
    match s {
        SignalData::Complex { re, .. } => re,
        real => real,
    }
}

/// Decompose into owned (re, im) real parts; a real signal gets a zero
/// imaginary part of the matching substrate and length.
fn split_complex(s: &SignalData) -> Result<(SignalData, SignalData), String> {
    match s {
        SignalData::Complex { re, im } => Ok(((**re).clone(), (**im).clone())),
        real => {
            let zero = constant(real, &BigRational::from_integer(0.into()))?;
            Ok((real.clone(), zero))
        }
    }
}

/// Complex product (a_re + i·a_im)(b_re + i·b_im), over real-signal parts.
fn cmul(
    a: (&SignalData, &SignalData),
    b: (&SignalData, &SignalData),
) -> Result<(SignalData, SignalData), String> {
    let re = binop("-", &binop("*", a.0, b.0)?, &binop("*", a.1, b.1)?)?;
    let im = binop("+", &binop("*", a.0, b.1)?, &binop("*", a.1, b.0)?)?;
    Ok((re, im))
}

/// Complex quotient a/b = a·conj(b) / |b|². Divides by the (real) |b|²; the
/// real `binop("/")` rejects any sample whose divisor interval reaches zero.
fn cdiv(
    a: (&SignalData, &SignalData),
    b: (&SignalData, &SignalData),
) -> Result<(SignalData, SignalData), String> {
    let denom = binop("+", &binop("*", b.0, b.0)?, &binop("*", b.1, b.1)?)?;
    let num_re = binop("+", &binop("*", a.0, b.0)?, &binop("*", a.1, b.1)?)?;
    let num_im = binop("-", &binop("*", a.1, b.0)?, &binop("*", a.0, b.1)?)?;
    Ok((binop("/", &num_re, &denom)?, binop("/", &num_im, &denom)?))
}

/// Per-sample magnitude |z| = √(re² + im²), as a real signal.
fn cmag(re: &SignalData, im: &SignalData) -> Result<SignalData, String> {
    let sumsq = binop("+", &binop("*", re, re)?, &binop("*", im, im)?)?;
    unary("sqrt", &sumsq)
}

/// The real part of a signal (the signal itself when already real).
pub fn re_part(s: &SignalData) -> SignalData {
    match s {
        SignalData::Complex { re, .. } => (**re).clone(),
        real => real.clone(),
    }
}

/// The imaginary part of a signal (an all-zero real signal when already real).
pub fn im_part(s: &SignalData) -> SignalData {
    match s {
        SignalData::Complex { im, .. } => (**im).clone(),
        real => constant(real, &BigRational::from_integer(0.into()))
            .expect("zero is always representable"),
    }
}

/// Whether a signal is complex-valued.
pub fn is_complex(s: &SignalData) -> bool {
    matches!(s, SignalData::Complex { .. })
}

/// The complex conjugate (identity on a real signal).
pub fn conj(s: &SignalData) -> SignalData {
    match s {
        SignalData::Complex { re, im } => SignalData::Complex {
            re: re.clone(),
            im: Box::new(unary("neg", im).expect("negation is total on real signals")),
        },
        real => real.clone(),
    }
}

/// FFT/IFFT producing a single complex signal. Accepts a real or complex
/// signal; the heavy lifting reuses the interval-complex [`fft`] kernels.
pub fn fft_signal(s: &SignalData, inverse: bool) -> Result<SignalData, String> {
    let (re, im) = match s {
        SignalData::Complex { re, im } => fft(re, Some(im), inverse)?,
        real => fft(real, None, inverse)?,
    };
    complex(re, im)
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
            SignalData::Complex { re, .. } => {
                let hw = max_half_width_f64(self);
                let sub = match re.as_ref() {
                    SignalData::F64 { .. } => "complex f64".to_string(),
                    SignalData::Big { digits, .. } => format!("complex, {} digits", digits),
                    SignalData::Complex { .. } => unreachable!("complex parts are real"),
                };
                if hw == 0.0 {
                    write!(f, "<signal: {} {}, {}, exact>", n, samples, sub)
                } else {
                    write!(
                        f,
                        "<signal: {} {}, {}, max error ±{:.1e}>",
                        n, samples, sub, hw
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
            // The Lipschitz slack (width, at the argument's scale) and the
            // libm error (ulps of sin(a.0), at the result's scale) are
            // different magnitudes; widening their round-to-nearest SUM at
            // its own — possibly cancelled-to-tiny — scale covers neither
            // (confirmed containment break for intervals near π). Widen each
            // contribution at its own scale, plus 1 ulp for the final add.
            // next_up on the width covers its round-to-nearest underestimate.
            let width = (a.1 - a.0).next_up();
            let v = if name == "sin" { a.0.sin() } else { a.0.cos() };
            let lo = (widen_down(v, LIBM_ULPS) - width).next_down();
            let hi = (widen_up(v, LIBM_ULPS) + width).next_up();
            // The clamp keeps a huge-width interval finite; [-1, 1] is
            // always a sound enclosure for sin/cos.
            Ok((lo.max(-1.0), hi.min(1.0)))
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

fn big_add(
    a: (&BigFloat, &BigFloat),
    b: (&BigFloat, &BigFloat),
    p: usize,
) -> Result<(BigFloat, BigFloat), String> {
    big_check(a.0.add(b.0, p, DOWN), a.1.add(b.1, p, UP))
}

fn big_sub(
    a: (&BigFloat, &BigFloat),
    b: (&BigFloat, &BigFloat),
    p: usize,
) -> Result<(BigFloat, BigFloat), String> {
    big_check(a.0.sub(b.1, p, DOWN), a.1.sub(b.0, p, UP))
}

fn big_mul(
    a: (&BigFloat, &BigFloat),
    b: (&BigFloat, &BigFloat),
    p: usize,
) -> Result<(BigFloat, BigFloat), String> {
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

fn big_div(
    a: (&BigFloat, &BigFloat),
    b: (&BigFloat, &BigFloat),
    p: usize,
) -> Result<(BigFloat, BigFloat), String> {
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
                let mag = if bf_lt(a.1, &neg_lo) {
                    neg_lo
                } else {
                    a.1.clone()
                };
                big_check(zero, mag)
            }
        }
        "sqrt" => {
            if a.1.is_negative() {
                return Err("sqrt of a negative sample (signals are real-valued)".into());
            }
            let lo = if a.0.is_negative() {
                zero
            } else {
                a.0.sqrt(p, DOWN)
            };
            big_check(lo, a.1.sqrt(p, UP))
        }
        "exp" => {
            // bf_exp, never raw `.exp` (wasm32 drops the argument's integer
            // part — see the wrapper). astro-float also flushes exp
            // underflow to exact +0 even rounding Up; a zero upper bound
            // sits BELOW the strictly positive true value. Substitute the
            // smallest representable positive (a flushed lower bound of 0
            // is already sound).
            let mut hi = crate::expr::bf_exp(a.1, p, UP, cc);
            if hi.is_zero() {
                hi = BigFloat::min_positive(p);
            }
            big_check(crate::expr::bf_exp(a.0, p, DOWN, cc), hi)
        }
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

/// Exact comparison of a finite f64 against an exact rational, without
/// building a `BigRational` from the float: `x = ±m·2^e` (from
/// `integer_decode`, exact for every finite f64 including subnormals) is
/// compared against `r = n/d` by integer cross-multiplication,
/// `±m·2^e ⋚ n/d  ⇔  ±m·d·2^e ⋚ n` (num-rational keeps `d > 0`). Shifts and
/// one multiply — no gcd reduction, no division ladder; the
/// `BigRational::from_f64(x).cmp(r)` equivalent of this check dominated the
/// signal-packing profile. `None` for non-finite `x` (mirroring `from_f64`),
/// which callers must treat as "containment not shown".
fn cmp_f64_rat(x: f64, r: &BigRational) -> Option<std::cmp::Ordering> {
    if !x.is_finite() {
        return None;
    }
    let (m, e, s) = num_traits::Float::integer_decode(x);
    let mut lhs = BigInt::from(m);
    if s < 0 {
        lhs = -lhs;
    }
    lhs *= r.denom();
    let mut rhs = r.numer().clone();
    if e >= 0 {
        lhs <<= e as usize;
    } else {
        // Keep both sides integral by scaling the right side up instead:
        // m·d·2^e ⋚ n  ⇔  m·d ⋚ n·2^{-e}. e ≥ −1074, so ≤ ~1.1 kbit — cheap.
        rhs <<= (-e) as usize;
    }
    Some(lhs.cmp(&rhs))
}

/// A sound f64 enclosure of an exact rational, by nudging a nearest-ish
/// approximation outward until exact containment is verified.
fn rat_to_f64_iv(r: &BigRational) -> Result<(f64, f64), String> {
    use std::cmp::Ordering;
    let approx = r.to_f64().filter(|v| v.is_finite()).ok_or_else(|| {
        "an entry exceeds the f64 range — use signal(v, digits) for arbitrary precision".to_string()
    })?;
    // `x ≤ r` / `x ≥ r`, exactly; false when the comparison is unavailable
    // (non-finite x), so an unverified bound is never accepted.
    let le = |x: f64| cmp_f64_rat(x, r).is_some_and(|o| o != Ordering::Greater);
    let ge = |x: f64| cmp_f64_rat(x, r).is_some_and(|o| o != Ordering::Less);
    let mut lo = approx;
    let mut hi = approx;
    for _ in 0..64 {
        if le(lo) {
            break;
        }
        lo = lo.next_down();
    }
    for _ in 0..64 {
        if ge(hi) {
            break;
        }
        hi = hi.next_up();
    }
    if le(lo) && ge(hi) && lo.is_finite() && hi.is_finite() {
        Ok((lo, hi))
    } else {
        Err("could not enclose an entry in f64 (use signal(v, digits))".into())
    }
}

fn rat_to_big_iv(
    r: &BigRational,
    p: usize,
    cc: &mut Consts,
) -> Result<(BigFloat, BigFloat), String> {
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
        Expr::Float(bf, _) => {
            float_to_rational(bf).ok_or_else(|| "cannot pack a non-finite float".to_string())
        }
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
    // Any complex entry makes the whole signal complex: split into real and
    // imaginary entry lists and pack each separately (same substrate).
    if entries.iter().any(|e| matches!(e, Expr::Complex(..))) {
        let mut re = Vec::with_capacity(entries.len());
        let mut im = Vec::with_capacity(entries.len());
        for e in entries {
            match e {
                Expr::Complex(r, i) => {
                    re.push((**r).clone());
                    im.push((**i).clone());
                }
                other => {
                    re.push(other.clone());
                    im.push(Expr::Int(BigInt::from(0)));
                }
            }
        }
        return complex(pack(&re, digits)?, pack(&im, digits)?);
    }
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
            // bf_from_f64_exact, not from_f64: astro-float halves subnormals.
            Expr::Float(bf_from_f64_exact(m, 64), 17)
        }
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            let two = BigFloat::from_i64(2, p);
            let m = lo[i]
                .add(&hi[i], p, RoundingMode::ToEven)
                .div(&two, p, RoundingMode::ToEven);
            Expr::Float(m, *digits)
        }
        SignalData::Complex { re, im } => crate::expr::complex(midpoint(re, i), midpoint(im, i)),
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
            let m = lo[i] / 2.0 + hi[i] / 2.0;
            // Deviation is exactly 0 only when the computed midpoint lands
            // exactly on a point interval — `lo == hi` alone is NOT enough:
            // `lo/2 + hi/2` is inexact for odd subnormals (3·2⁻¹⁰⁷⁴ has
            // midpoint 4·2⁻¹⁰⁷⁴), leaving a real |mid − true| gap that a
            // "0 (exact)" bound would deny.
            if m == lo[i] && m == hi[i] {
                return 0.0;
            }
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
        // With component deviations dr, di the modulus deviation reaches
        // √(dr² + di²) ≤ √2·max(dr, di) — the max alone understates it by up
        // to √2 (re, im each within d of mid puts z up to d·√2 away).
        SignalData::Complex { re, im } => {
            let d = deviation_f64(re, i).max(deviation_f64(im, i));
            if d == 0.0 {
                0.0 // both components exact ⇒ z is exactly mid
            } else {
                widen_up(d * std::f64::consts::SQRT_2.next_up(), 1)
            }
        }
    }
}

fn max_half_width_f64(s: &SignalData) -> f64 {
    (0..s.len())
        .map(|i| deviation_f64(s, i))
        .fold(0.0, f64::max)
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
    // bf_from_f64_exact, not from_f64: astro-float halves subnormals, which
    // would report a certified bound at HALF its true value.
    Expr::Float(bf_from_f64_exact(hw, 64), 3)
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
            SignalData::Big {
                lo: al,
                hi: ah,
                digits,
            },
            SignalData::Big {
                lo: bl,
                hi: bh,
                digits: bd,
            },
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
        // Complex on either side: promote the real operand to (x, 0) and work
        // component-wise, reusing the real kernels above (and so both substrates).
        _ if matches!(a, SignalData::Complex { .. }) || matches!(b, SignalData::Complex { .. }) => {
            let (are, aim) = split_complex(a)?;
            let (bre, bim) = split_complex(b)?;
            match op {
                "+" | "-" => complex(binop(op, &are, &bre)?, binop(op, &aim, &bim)?),
                "*" => {
                    let (re, im) = cmul((&are, &aim), (&bre, &bim))?;
                    complex(re, im)
                }
                "/" => {
                    let (re, im) = cdiv((&are, &aim), (&bre, &bim))?;
                    complex(re, im)
                }
                _ => unreachable!("unknown signal binop"),
            }
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

/// Broadcast an exact scalar (real or complex) against a signal.
pub fn scalar_binop(
    op: &str,
    s: &SignalData,
    scalar: &Expr,
    scalar_on_left: bool,
) -> Result<SignalData, String> {
    let broadcast = match scalar {
        // A complex scalar broadcasts to a complex constant; binop then
        // promotes a real `s` as needed.
        Expr::Complex(r, i) => {
            let base = real_substrate(s);
            complex(
                constant(base, &entry_rational(r)?)?,
                constant(base, &entry_rational(i)?)?,
            )?
        }
        _ => {
            let r = entry_rational(scalar).map_err(|_| {
                format!(
                    "cannot mix '{}' into a signal — only numbers broadcast",
                    scalar
                )
            })?;
            constant(s, &r)?
        }
    };
    if scalar_on_left {
        binop(op, &broadcast, s)
    } else {
        binop(op, s, &broadcast)
    }
}

/// A real constant signal matching the substrate/length of `like` (a complex
/// `like` matches its real part — the same length and substrate).
fn constant(like: &SignalData, r: &BigRational) -> Result<SignalData, String> {
    match real_substrate(like) {
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
        SignalData::Complex { .. } => unreachable!("real_substrate yields a real signal"),
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
        SignalData::Complex { re, im } => match name {
            // |z| collapses to a real magnitude signal.
            "abs" => cmag(re, im),
            "neg" => complex(unary("neg", re)?, unary("neg", im)?),
            "conj" => Ok(conj(s)),
            _ => Err(format!("'{}' is not defined on complex signals", name)),
        },
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
            SignalData::Big {
                lo: al,
                hi: ah,
                digits,
            },
            SignalData::Big {
                lo: bl,
                hi: bh,
                digits: bd,
            },
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
        // Complex convolution: combine the real component convolutions.
        _ if matches!(a, SignalData::Complex { .. }) || matches!(b, SignalData::Complex { .. }) => {
            let (are, aim) = split_complex(a)?;
            let (bre, bim) = split_complex(b)?;
            let re = binop("-", &conv(&are, &bre)?, &conv(&aim, &bim)?)?;
            let im = binop("+", &conv(&are, &bim)?, &conv(&aim, &bre)?)?;
            complex(re, im)
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
        SignalData::Complex { re, im } => complex(pad(re, n)?, pad(im, n)?),
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
        SignalData::Complex { .. } => {
            Err("fft: real and imaginary parts must each be real signals".into())
        }
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
    let mut buf: Vec<(f64, f64, f64, f64)> = (0..n).map(|i| (rl[i], rh[i], il[i], ih[i])).collect();
    bit_reverse_permute(&mut buf);

    let mut len = 2;
    while len <= n {
        let half = len / 2;
        for k in 0..half {
            // Twiddle e^(∓2πi·k/len). Widening a *point* twiddle by ulps of
            // the function value is unsound where cos/sin ≈ 0 — the angle
            // error (π's ½ ulp) is absolute, not relative (confirmed
            // containment break at n = 4, bin 1). Instead: k/len is dyadic,
            // hence exact in f64; enclose the angle 2π·(k/len) as an
            // interval and take cos/sin of the interval via the Lipschitz
            // kernel, exactly as fft_big and window() do.
            let (w_re, w_im) = if k == 0 {
                ((1.0, 1.0), (0.0, 0.0)) // e^0 exactly
            } else {
                let frac = (k as f64) / (len as f64); // exact: len is 2^m
                let t_lo = ((2.0 * std::f64::consts::PI.next_down()) * frac).next_down();
                let t_hi = ((2.0 * std::f64::consts::PI.next_up()) * frac).next_up();
                let c = f64_unary("cos", (t_lo, t_hi))?;
                let s = f64_unary("sin", (t_lo, t_hi))?;
                let s = if inverse { s } else { (-s.1, -s.0) };
                (c, s)
            };
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
        SignalData::F64 {
            lo: out.0,
            hi: out.1,
        },
        SignalData::F64 {
            lo: out.2,
            hi: out.3,
        },
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
        SignalData::Big {
            lo: out.0,
            hi: out.1,
            digits,
        },
        SignalData::Big {
            lo: out.2,
            hi: out.3,
            digits,
        },
    ))
}

// ---------------------------------------------------------------------------
// Certified reductions
// ---------------------------------------------------------------------------

/// Certified upper bound on max |x[i]| — the peak (max |z| for complex).
pub fn peak(s: &SignalData) -> Result<Expr, String> {
    match s {
        SignalData::F64 { lo, hi } => {
            let v = lo
                .iter()
                .zip(hi)
                .map(|(l, h)| l.abs().max(h.abs()))
                .fold(0.0, f64::max);
            Ok(Expr::Float(bf_from_f64_exact(v, 64), 17))
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
            Ok(Expr::Float(max, *digits))
        }
        SignalData::Complex { re, im } => peak(&cmag(re, im)?),
    }
}

/// Certified upper bound on the RMS √(Σx²/n) — √(Σ|z|²/n) for complex.
pub fn rms(s: &SignalData) -> Result<Expr, String> {
    if let SignalData::Complex { re, im } = s {
        return rms(&cmag(re, im)?);
    }
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
            Ok(Expr::Float(bf_from_f64_exact(upper, 64), 17))
        }
        SignalData::Big { hi, digits, .. } => {
            let p = prec_bits(*digits);
            let mut acc = BigFloat::from_i64(0, p);
            for h in hi {
                acc = acc.add(h, p, UP);
            }
            let n = BigFloat::from_i64(hi.len() as i64, p);
            let upper = acc.div(&n, p, UP).sqrt(p, UP);
            // The accumulation above never passes through big_check; a
            // silent Inf/NaN must not escape as a "certified" bound.
            let (_, upper) = big_check(BigFloat::from_i64(0, p), upper)?;
            Ok(Expr::Float(upper, *digits))
        }
        // `s` is real here (complex handled above), so `s*s` is real too.
        SignalData::Complex { .. } => unreachable!("rms squares a real signal"),
    }
}

// ---------------------------------------------------------------------------
// Slicing and plotting support
// ---------------------------------------------------------------------------

/// A contiguous sub-signal: `n` samples starting at 0-based `start`.
pub fn slice(s: &SignalData, start: usize, n: usize) -> Result<SignalData, String> {
    if n == 0 {
        return Err("slice needs at least 1 sample".into());
    }
    let end = start
        .checked_add(n)
        .filter(|e| *e <= s.len())
        .ok_or_else(|| {
            format!(
                "slice of {} samples from position {} runs past the end (the signal has {})",
                n,
                start + 1,
                s.len()
            )
        })?;
    Ok(match s {
        SignalData::F64 { lo, hi } => SignalData::F64 {
            lo: lo[start..end].to_vec(),
            hi: hi[start..end].to_vec(),
        },
        SignalData::Big { lo, hi, digits } => SignalData::Big {
            lo: lo[start..end].to_vec(),
            hi: hi[start..end].to_vec(),
            digits: *digits,
        },
        SignalData::Complex { re, im } => complex(slice(re, start, n)?, slice(im, start, n)?)?,
    })
}

/// A sub-signal gathered from arbitrary 0-based positions, in order — the
/// strided counterpart of [`slice`]. Callers validate the indices against the
/// length (they must all be `< s.len()`).
pub fn gather(s: &SignalData, idx: &[usize]) -> Result<SignalData, String> {
    if idx.is_empty() {
        return Err("slice needs at least 1 sample".into());
    }
    Ok(match s {
        SignalData::F64 { lo, hi } => SignalData::F64 {
            lo: idx.iter().map(|&i| lo[i]).collect(),
            hi: idx.iter().map(|&i| hi[i]).collect(),
        },
        SignalData::Big { lo, hi, digits } => SignalData::Big {
            lo: idx.iter().map(|&i| lo[i].clone()).collect(),
            hi: idx.iter().map(|&i| hi[i].clone()).collect(),
            digits: *digits,
        },
        SignalData::Complex { re, im } => complex(gather(re, idx)?, gather(im, idx)?)?,
    })
}

/// Midpoint of sample `i` as a display-grade f64 (plotting only — the
/// rigorous readback is [`midpoint`]/[`half_width`]).
fn mid_f64(s: &SignalData, i: usize) -> f64 {
    match s {
        SignalData::F64 { lo, hi } => lo[i] / 2.0 + hi[i] / 2.0,
        SignalData::Big { lo, hi, digits } => {
            let p = prec_bits(*digits);
            let two = BigFloat::from_i64(2, p);
            let m = lo[i]
                .add(&hi[i], p, RoundingMode::ToEven)
                .div(&two, p, RoundingMode::ToEven);
            float_to_rational(&m)
                .and_then(|r| r.to_f64())
                .unwrap_or(f64::NAN)
        }
        // Fallback when a complex signal is plotted without being split into
        // its parts first: the magnitude midpoint. (The wasm plot path splits
        // into separate re/im series, so this is rarely hit.)
        SignalData::Complex { re, im } => {
            let (r, m) = (mid_f64(re, i), mid_f64(im, i));
            (r * r + m * m).sqrt()
        }
    }
}

/// Per-sample midpoints as plain `f64` — the representative used when leaving
/// the certified world (e.g. raw binary export). A complex signal returns its
/// real and imaginary midpoint streams; a real signal returns `(values, None)`.
pub fn midpoints_f64(s: &SignalData) -> (Vec<f64>, Option<Vec<f64>>) {
    match s {
        SignalData::Complex { re, im } => (
            (0..re.len()).map(|i| mid_f64(re, i)).collect(),
            Some((0..im.len()).map(|i| mid_f64(im, i)).collect()),
        ),
        real => ((0..real.len()).map(|i| mid_f64(real, i)).collect(), None),
    }
}

/// (x, y) plot points over the 1-based sample index. Signals longer than
/// `max_points` decimate to a min/max *envelope* (two points per bucket, the
/// audio-editor waveform display) — extremes are preserved, never aliased
/// away; the caller should still flag the curve as decimated.
pub fn plot_points(s: &SignalData, max_points: usize) -> Vec<(f64, Option<f64>)> {
    plot_points_range(s, 0, s.len(), max_points)
}

/// Plot points for the 0-based sample range [from, to) — the zoom-refinement
/// primitive: a narrower window over the same data redecimates at full
/// resolution. The emitted x values stay global (1-based original indices).
pub fn plot_points_range(
    s: &SignalData,
    from: usize,
    to: usize,
    max_points: usize,
) -> Vec<(f64, Option<f64>)> {
    let (from, to) = (from.min(s.len()), to.min(s.len()));
    if from >= to {
        return Vec::new();
    }
    let n = to - from;
    let point = |i: usize| {
        let y = mid_f64(s, i);
        ((i + 1) as f64, y.is_finite().then_some(y))
    };
    if n <= max_points {
        return (from..to).map(point).collect();
    }
    let buckets = (max_points / 2).max(1);
    let mut out = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let lo_i = from + b * n / buckets;
        let hi_i = (from + ((b + 1) * n / buckets).max(b * n / buckets + 1)).min(to);
        let (mut min_i, mut max_i) = (lo_i, lo_i);
        let (mut min_v, mut max_v) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in lo_i..hi_i {
            let v = mid_f64(s, i);
            if v < min_v {
                min_v = v;
                min_i = i;
            }
            if v > max_v {
                max_v = v;
                max_i = i;
            }
        }
        // Emit the extremes in index order so the polyline sweeps left→right.
        let (first, second) = if min_i <= max_i {
            (min_i, max_i)
        } else {
            (max_i, min_i)
        };
        out.push(point(first));
        if second != first {
            out.push(point(second));
        }
    }
    out
}

/// Whether plotting `[from, to)` at `max_points` will decimate.
pub fn range_decimated(from: usize, to: usize, max_points: usize) -> bool {
    to.saturating_sub(from) > max_points
}

/// Whether plotting at `max_points` will decimate.
pub fn plot_decimated(s: &SignalData, max_points: usize) -> bool {
    s.len() > max_points
}

// ---------------------------------------------------------------------------
// Lossless serialization of arbitrary-precision signals
// ---------------------------------------------------------------------------

/// Exact decimal bounds of a Big signal, ready to serialize.
pub struct DecimalBounds {
    pub lo: Vec<String>,
    pub hi: Vec<String>,
    pub digits: usize,
}

/// The bounds of a Big signal as exact decimal strings (a binary float's
/// decimal expansion terminates, so this is lossless). `None` for f64
/// signals, which serialize as plain numbers.
pub fn big_decimal_bounds(s: &SignalData) -> Option<Result<DecimalBounds, String>> {
    let SignalData::Big { lo, hi, digits } = s else {
        return None;
    };
    let dec = |bf: &BigFloat| -> Result<String, String> {
        let r = float_to_rational(bf).ok_or("cannot export a non-finite bound")?;
        crate::dataio::rat_to_decimal(&r)
            .ok_or_else(|| "a binary float always terminates in decimal".to_string())
    };
    let run = || -> Result<_, String> {
        Ok(DecimalBounds {
            lo: lo.iter().map(dec).collect::<Result<Vec<_>, _>>()?,
            hi: hi.iter().map(dec).collect::<Result<Vec<_>, _>>()?,
            digits: *digits,
        })
    };
    Some(run())
}

/// Rebuild a Big signal from exact decimal bounds. Parsing an exactly
/// representable decimal at the signal's working precision is exact, so
/// export → import is the identity.
pub fn big_from_decimal_bounds(
    lo: &[String],
    hi: &[String],
    digits: usize,
) -> Result<SignalData, String> {
    if lo.len() != hi.len() {
        return Err("'signal' lo and hi must have the same length".into());
    }
    let digits = digits.clamp(1, 100_000);
    let p = prec_bits(digits);
    with_consts(|cc| -> Result<SignalData, String> {
        // Decimal → exact rational → directed division. Never astro-float's
        // string parse: it mispositions the decimal point of long fractional
        // strings on wasm32. Going through the rational also keeps outward
        // rounding per side, so even a hand-edited file can only *widen* the
        // enclosure, never tighten it. An exported bound is representable at
        // p bits, so both directions agree and the round trip is exact.
        let mut conv = |s: &str, low_side: bool| -> Result<BigFloat, String> {
            if s.len() > 1_000_000 {
                return Err("'signal' bound is too long".into());
            }
            let r =
                crate::dataio::decimal_to_rat(s).map_err(|e| format!("bad signal bound: {}", e))?;
            let (d, u) = rat_to_big_iv(&r, p, cc)?;
            Ok(if low_side { d } else { u })
        };
        let mut lo_v = Vec::with_capacity(lo.len());
        for s in lo {
            lo_v.push(conv(s, true)?);
        }
        let mut hi_v = Vec::with_capacity(hi.len());
        for s in hi {
            hi_v.push(conv(s, false)?);
        }
        let (lo, hi) = (lo_v, hi_v);
        for (l, h) in lo.iter().zip(&hi) {
            if bf_lt(h, l) {
                return Err("'signal' bounds must satisfy lo <= hi".into());
            }
        }
        Ok(SignalData::Big { lo, hi, digits })
    })?
}

// ---------------------------------------------------------------------------
// Certified window generation
// ---------------------------------------------------------------------------

/// A cosine-sum window as a *certified* f64 signal: every sample is an
/// enclosure of the true window value, computed in interval arithmetic
/// (π as an interval, cos via the Lipschitz bound). This is the honest way
/// to taper bulk data — `signal(N(dsp.hann(n)))` would launder uncertified
/// approximations into point intervals.
pub fn window(name: &str, n: usize) -> Result<SignalData, String> {
    let (a0, a1, a2): ((i64, i64), (i64, i64), (i64, i64)) = match name {
        "hann" => ((1, 2), (1, 2), (0, 1)),
        "hamming" => ((27, 50), (23, 50), (0, 1)),
        "blackman" => ((21, 50), (1, 2), (2, 25)),
        other => {
            return Err(format!(
                "unknown window '{}' (available: hann, hamming, blackman)",
                other
            ))
        }
    };
    if n == 0 {
        return Err("a window needs at least 1 sample".into());
    }
    let rat = |(p, q): (i64, i64)| BigRational::new(BigInt::from(p), BigInt::from(q));
    let (c0, c1, c2) = (
        rat_to_f64_iv(&rat(a0))?,
        rat_to_f64_iv(&rat(a1))?,
        rat_to_f64_iv(&rat(a2))?,
    );
    // π as a certified f64 interval.
    let pi = (
        widen_down(std::f64::consts::PI, 1),
        widen_up(std::f64::consts::PI, 1),
    );
    let mut lo = Vec::with_capacity(n);
    let mut hi = Vec::with_capacity(n);
    if n == 1 {
        lo.push(1.0);
        hi.push(1.0);
        return Ok(SignalData::F64 { lo, hi });
    }
    for k in 0..n {
        // w[k] = a0 − a1·cos(2πk/(n−1)) + a2·cos(4πk/(n−1))
        let ratio = rat_to_f64_iv(&rat((2 * k as i64, n as i64 - 1)))?;
        let angle = f64_mul(ratio, pi)?;
        let cos1 = f64_unary("cos", angle)?;
        let cos2 = f64_unary("cos", f64_mul(angle, (2.0, 2.0))?)?;
        let w = f64_add(f64_sub(c0, f64_mul(c1, cos1)?)?, f64_mul(c2, cos2)?)?;
        // The window is mathematically within [0, 1] for these families'
        // coefficient signs at the sampled points — but the enclosure is the
        // claim, so no clamping beyond what interval math produced.
        lo.push(w.0);
        hi.push(w.1);
    }
    Ok(SignalData::F64 { lo, hi })
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_traits::FromPrimitive;
    use std::cmp::Ordering;

    /// `cmp_f64_rat` replaced `BigRational::from_f64(x).map(|v| v.cmp(r))` on
    /// the packing hot path; a disagreement with that oracle would silently
    /// break enclosure containment, so the equivalence is pinned here across
    /// the regions where f64 decompositions go wrong: subnormals, ±0,
    /// near-overflow, exact dyadic ties, and both signs throughout.
    #[test]
    fn cmp_f64_rat_matches_the_bigrational_oracle() {
        let mut cases: Vec<f64> = vec![
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            1.5,
            f64::MIN_POSITIVE,       // smallest normal
            f64::MIN_POSITIVE / 2.0, // subnormal
            5e-324,                  // smallest subnormal
            -5e-324,
            f64::MAX,
            f64::MIN,
            1.0f64.next_up(),
            1.0f64.next_down(),
            0.1,
            -0.1,
            1e300,
            -1e300,
            1e-300,
            (1u64 << 53) as f64, // integer at the mantissa edge
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        // A deterministic sweep of pseudo-random bit patterns (LCG) pushes the
        // check across arbitrary exponent/mantissa combinations.
        let mut state: u64 = 0x243F6A8885A308D3;
        for _ in 0..4000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            cases.push(f64::from_bits(state));
        }
        // Rationals to compare against: small, huge, tiny, negative, and the
        // exact values of some of the floats themselves (tie cases).
        let mut rats: Vec<BigRational> = [
            (0i64, 1i64),
            (1, 1),
            (-1, 1),
            (1, 3),
            (-22, 7),
            (1, i64::MAX),
            (i64::MAX, 2),
            (-i64::MAX, 3),
        ]
        .into_iter()
        .map(|(n, d)| BigRational::new(BigInt::from(n), BigInt::from(d)))
        .collect();
        for x in [0.5f64, -0.75, 5e-324, f64::MAX, 0.1, 3.0] {
            rats.push(BigRational::from_f64(x).unwrap()); // exact tie candidates
        }
        for &x in &cases {
            for r in &rats {
                let oracle: Option<Ordering> = BigRational::from_f64(x).map(|v| v.cmp(r));
                assert_eq!(
                    cmp_f64_rat(x, r),
                    oracle,
                    "cmp_f64_rat disagrees with BigRational::from_f64 for x = {x:e} ({:#x}), r = {r}",
                    x.to_bits()
                );
            }
        }
    }
}
