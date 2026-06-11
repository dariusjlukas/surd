//! The expression value type and all canonicalization.
//!
//! The whole "exact, correct by default" feel of the language lives here. The
//! key trick: build expressions only through the smart constructors
//! ([`add`], [`mul`], [`pow`]). They enforce a canonical form so that, for
//! example, `sqrt(2)^2` and `sqrt(2)*sqrt(2)` both collapse to `2` as a side
//! effect of two general rules rather than special cases:
//!   * `sqrt(x)` is represented as `x^(1/2)`.
//!   * [`mul`] collects like bases and adds their exponents.
//!   * [`pow`] flattens nested powers `(a^b)^c -> a^(b*c)` where it's sound.

use crate::ast::Node;
use astro_float::{BigFloat, Consts, Radix, RoundingMode};
use num_bigint::BigInt;
use num_rational::Ratio;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::fmt;
use std::rc::Rc;

/// Arbitrary-precision rational. We alias `Ratio<BigInt>` ourselves rather than
/// relying on num-rational's `BigRational` so we don't depend on its optional
/// feature flags.
pub type BigRational = Ratio<BigInt>;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// Exact integer.
    Int(BigInt),
    /// Exact non-integer rational (denominator != 1, always normalized).
    Rat(BigRational),
    /// An inexact, arbitrary-precision float — only ever produced by `N(...)`.
    /// This is the explicit boundary out of exact-land. The `usize` records how
    /// many significant decimal digits to show.
    Float(BigFloat, usize),
    /// A named mathematical constant kept symbolic (never auto-evaluated).
    Const(Constant),
    /// A free or bound variable.
    Symbol(String),
    /// n-ary sum, canonicalized: like terms combined, numeric part folded,
    /// operands sorted, length >= 2.
    Add(Vec<Expr>),
    /// n-ary product, canonicalized: like bases combined, numeric coefficient
    /// folded and placed first, length >= 2.
    Mul(Vec<Expr>),
    /// base ^ exponent.
    Pow(Box<Expr>, Box<Expr>),
    /// An applied function, e.g. `sin(x)`.
    Func(String, Vec<Expr>),
    /// A dense matrix of expression entries, stored row-major. Always
    /// rectangular and non-empty (enforced by `matrix::matrix`). Entries are
    /// general `Expr`, so symbolic matrices are allowed; the exact-ℚ linear
    /// algebra is just the case where every entry is a number.
    Matrix(Vec<Vec<Expr>>),
    /// A complex number re + im·i. Invariant: `im` is never the zero literal
    /// (such a value collapses to its real part), and neither part is itself
    /// `Complex`. Parts are general real expressions, so `x + I` is allowed.
    Complex(Box<Expr>, Box<Expr>),
    /// A boolean — produced by comparisons and logic, consumed by control flow.
    Bool(bool),
    /// A user-defined function: parameter names + body AST. `Rc` keeps `Expr`
    /// cheap to clone.
    Function { params: Vec<String>, body: Rc<Node> },
    /// `lhs = rhs`. A piece of data, not a boolean.
    Equation(Box<Expr>, Box<Expr>),
    /// Named fields holding arbitrary values: `struct(a = 1, b = [1; 2])`.
    /// Invariants (enforced by [`structure`]): non-empty, names unique and
    /// sorted — so derived equality is field-order-independent. Fields are
    /// read with `.name`; data imports land in the workspace as structs so
    /// imported names can't collide with existing bindings.
    Struct(Vec<(String, Expr)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constant {
    Pi,
    E,
}

// ---------------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------------

/// Largest exact power we'll actually evaluate, measured in *result* bits
/// (≈ base bits × exponent) — what bounds cost is the size of the answer, not
/// the exponent alone (`big^small` can still be enormous). ~1M bits ≈ 300k
/// digits. Beyond this a power stays symbolic.
const MAX_POW_RESULT_BITS: u128 = 1_000_000;

/// Cap on the exponent for a complex base, whose magnitude grows
/// multiplicatively per multiply.
const MAX_COMPLEX_POW_EXP: u64 = 100_000;

/// A small integer literal.
pub fn int(n: i64) -> Expr {
    Expr::Int(BigInt::from(n))
}

/// The rational 1/2, used to desugar `sqrt`.
pub fn half() -> Expr {
    Expr::Rat(BigRational::new(BigInt::from(1), BigInt::from(2)))
}

/// Build a complex number re + im·i, collapsing to a real when im is zero.
pub fn complex(re: Expr, im: Expr) -> Expr {
    if matches!(&im, Expr::Int(i) if i.is_zero()) {
        re
    } else {
        Expr::Complex(Box::new(re), Box::new(im))
    }
}

/// The imaginary unit, i.
pub fn imaginary_unit() -> Expr {
    Expr::Complex(Box::new(int(0)), Box::new(int(1)))
}

/// Build a struct value, enforcing its invariants: at least one field, names
/// unique, fields sorted by name (the canonical form).
pub fn structure(mut fields: Vec<(String, Expr)>) -> Result<Expr, String> {
    if fields.is_empty() {
        return Err("a struct needs at least one field".into());
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    for w in fields.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(format!("duplicate struct field '{}'", w[0].0));
        }
    }
    Ok(Expr::Struct(fields))
}

/// Collapse a rational to an `Int` when its denominator is 1.
pub fn rat_to_expr(r: BigRational) -> Expr {
    if r.is_integer() {
        Expr::Int(r.to_integer())
    } else {
        Expr::Rat(r)
    }
}

/// If `e` is an exact number, return it as a rational.
pub fn numeric_value(e: &Expr) -> Option<BigRational> {
    match e {
        Expr::Int(i) => Some(BigRational::from_integer(i.clone())),
        Expr::Rat(r) => Some(r.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Canonicalizing smart constructors
// ---------------------------------------------------------------------------

/// Build a canonical sum (complex-aware).
pub fn add(terms: Vec<Expr>) -> Expr {
    let mut flat = Vec::new();
    for t in terms {
        flatten_add(t, &mut flat);
    }
    // If any term is complex, split into real and imaginary parts and combine
    // each separately, keeping the real canonicalizer below complex-free.
    if flat.iter().any(|t| matches!(t, Expr::Complex(..))) {
        let mut reals = Vec::new();
        let mut imags = Vec::new();
        for t in flat {
            match t {
                Expr::Complex(re, im) => {
                    reals.push(*re);
                    imags.push(*im);
                }
                other => reals.push(other),
            }
        }
        return complex(add_real(reals), add_real(imags));
    }
    add_real(flat)
}

/// Build a canonical sum of guaranteed-real terms.
fn add_real(terms: Vec<Expr>) -> Expr {
    let mut flat = Vec::new();
    for t in terms {
        flatten_add(t, &mut flat);
    }

    let mut constant = BigRational::zero();
    // Float contagion: float terms accumulate separately, and any exact
    // constant folds into them at the end. (running sum, max display digits)
    let mut float_sum: Option<(BigFloat, usize)> = None;
    // Each non-numeric term is split into (coefficient, basis) and like bases
    // are combined. Linear scan: term counts are small in practice.
    let mut parts: Vec<(BigRational, Expr)> = Vec::new();
    for t in flat {
        if let Some(r) = numeric_value(&t) {
            constant += r;
            continue;
        }
        if let Expr::Float(bf, d) = &t {
            float_sum = Some(match float_sum {
                None => (bf.clone(), *d),
                Some((acc, ad)) => {
                    let nd = ad.max(*d);
                    (acc.add(bf, prec_bits_for(nd), ROUND), nd)
                }
            });
            continue;
        }
        let (c, basis) = split_coeff(&t);
        if let Some(slot) = parts.iter_mut().find(|(_, b)| *b == basis) {
            slot.0 += c;
        } else {
            parts.push((c, basis));
        }
    }

    let mut result: Vec<Expr> = Vec::new();
    for (c, basis) in parts {
        if c.is_zero() {
            continue;
        }
        result.push(mul(vec![rat_to_expr(c), basis]));
    }
    if let Some((sum, d)) = float_sum {
        // A float makes the whole numeric part inexact: N(pi) + 1 is one float.
        let p = prec_bits_for(d);
        let total = if constant.is_zero() {
            sum
        } else {
            sum.add(&rat_to_bigfloat(&constant, p), p, ROUND)
        };
        // A zero float term vanishes next to other terms (like exact 0); it
        // survives only as the value of an all-numeric sum, e.g. N(1) - N(1).
        if result.is_empty() || !total.is_zero() {
            result.push(Expr::Float(total, d));
        }
    } else if !constant.is_zero() {
        result.push(rat_to_expr(constant));
    }

    sort_operands(&mut result);
    match result.len() {
        0 => int(0),
        1 => result.pop().unwrap(),
        _ => Expr::Add(result),
    }
}

/// Build a canonical product (complex-aware).
pub fn mul(factors: Vec<Expr>) -> Expr {
    let mut flat = Vec::new();
    for f in factors {
        flatten_mul(f, &mut flat);
    }
    if flat.iter().any(|f| matches!(f, Expr::Complex(..))) {
        let mut reals = Vec::new();
        let mut complexes = Vec::new();
        for f in flat {
            match f {
                Expr::Complex(re, im) => complexes.push((*re, *im)),
                other => reals.push(other),
            }
        }
        let scalar = mul_real(reals);
        // Multiply the complex factors: (a+bi)(c+di) = (ac − bd) + (ad + bc)i.
        let mut cre = int(1);
        let mut cim = int(0);
        for (fre, fim) in complexes {
            let nre = add_real(vec![
                mul_real(vec![cre.clone(), fre.clone()]),
                mul_real(vec![int(-1), cim.clone(), fim.clone()]),
            ]);
            let nim = add_real(vec![
                mul_real(vec![cre.clone(), fim]),
                mul_real(vec![cim, fre]),
            ]);
            cre = nre;
            cim = nim;
        }
        return complex(
            mul_real(vec![scalar.clone(), cre]),
            mul_real(vec![scalar, cim]),
        );
    }
    mul_real(flat)
}

/// Build a canonical product of guaranteed-real factors.
fn mul_real(factors: Vec<Expr>) -> Expr {
    let mut flat = Vec::new();
    for f in factors {
        flatten_mul(f, &mut flat);
    }

    let mut coeff = BigRational::one();
    // Float contagion: float factors accumulate separately; the exact
    // coefficient folds into them at the end. (running product, max digits)
    let mut float_coeff: Option<(BigFloat, usize)> = None;
    // Each factor is split into (base, exponent); like bases get their
    // exponents summed. This is what makes sqrt(2)*sqrt(2) -> 2.
    let mut parts: Vec<(Expr, Expr)> = Vec::new();
    for f in flat {
        if let Some(r) = numeric_value(&f) {
            if r.is_zero() {
                return int(0);
            }
            coeff *= r;
            continue;
        }
        if let Expr::Float(bf, d) = &f {
            float_coeff = Some(match float_coeff {
                None => (bf.clone(), *d),
                Some((acc, ad)) => {
                    let nd = ad.max(*d);
                    (acc.mul(bf, prec_bits_for(nd), ROUND), nd)
                }
            });
            continue;
        }
        let (base, exp) = split_pow(&f);
        if let Some(slot) = parts.iter_mut().find(|(b, _)| *b == base) {
            let prev = std::mem::replace(&mut slot.1, int(0));
            slot.1 = add(vec![prev, exp]);
        } else {
            parts.push((base, exp));
        }
    }

    let mut result: Vec<Expr> = Vec::new();
    for (base, exp) in parts {
        let p = pow(base, exp);
        // Combining exponents can produce a number again (e.g. 2^(1/2+1/2)=2);
        // fold any such number back into the coefficient.
        if let Some(r) = numeric_value(&p) {
            if r.is_zero() {
                return int(0);
            }
            coeff *= r;
        } else {
            result.push(p);
        }
    }

    sort_operands(&mut result);

    if let Some((fc, d)) = float_coeff {
        // Float contagion: the exact coefficient folds into the float one.
        let p = prec_bits_for(d);
        let total = if coeff.is_one() {
            fc
        } else {
            fc.mul(&rat_to_bigfloat(&coeff, p), p, ROUND)
        };
        // A zero float coefficient annihilates the product — but inexactly,
        // so the zero stays a float.
        if total.is_zero() || result.is_empty() {
            return Expr::Float(total, d);
        }
        result.insert(0, Expr::Float(total, d));
        return Expr::Mul(result);
    }

    // Distribute a numeric coefficient over a lone sum: c·(a + b) → c·a + c·b.
    // Without this, −(a + b) stays an opaque `Mul([-1, Add])` while +(a + b)
    // flattens, so `(a+b) − (a+b)` would never cancel to 0.
    if !coeff.is_one() && result.len() == 1 {
        if let Expr::Add(terms) = &result[0] {
            let c = rat_to_expr(coeff);
            return add(terms.iter().map(|t| mul(vec![c.clone(), t.clone()])).collect());
        }
    }

    if !coeff.is_one() {
        result.insert(0, rat_to_expr(coeff));
    }
    match result.len() {
        0 => int(1),
        1 => result.pop().unwrap(),
        _ => Expr::Mul(result),
    }
}

/// Build a canonical power.
pub fn pow(base: Expr, exp: Expr) -> Expr {
    if let Some(e) = numeric_value(&exp) {
        if e.is_zero() {
            return int(1); // includes 0^0 := 1, a deliberate convention
        }
        if e.is_one() {
            return base;
        }
    }
    // Float contagion: a float anywhere in an all-numeric power folds
    // numerically at the float's precision — when the result is a finite real.
    if matches!(base, Expr::Float(..)) || matches!(exp, Expr::Float(..)) {
        if let Some(folded) = float_pow(&base, &exp) {
            return folded;
        }
        // A symbolic other side, or a non-real / non-finite result
        // (e.g. (-2.0)^(1/2), 0.0^-1): stay symbolic rather than NaN.
        return Expr::Pow(Box::new(base), Box::new(exp));
    }
    // A complex base raised to a (modest) integer power: repeated multiplication.
    if let Expr::Complex(re, im) = &base {
        if let Some(e) = numeric_value(&exp) {
            if e.is_integer() {
                if let Some(n) = e.to_integer().to_i64() {
                    if n.unsigned_abs() <= MAX_COMPLEX_POW_EXP {
                        return complex_powi(re, im, n);
                    }
                }
            }
        }
        // Non-integer / huge powers of a complex number stay symbolic.
        return Expr::Pow(Box::new(base), Box::new(exp));
    }
    if let Expr::Int(i) = &base {
        if i.is_one() {
            return int(1);
        }
        if i.is_zero() {
            // 0^0 was already returned as 1 above, so any exponent here is
            // nonzero. 0^positive = 0; 0^negative is undefined — leave it
            // symbolic rather than dividing by zero (the `/` operator reports
            // the user-facing error before we ever get here).
            match numeric_value(&exp) {
                Some(e) if e > BigRational::zero() => return int(0),
                _ => return Expr::Pow(Box::new(base), Box::new(exp)),
            }
        }
    }

    // (a^b)^c -> a^(b*c). Only sound in general when c is an integer, or when
    // the inner base is a positive real (so no |x| / branch-cut surprise like
    // sqrt(x^2) = |x|). We honor that guard — accuracy over convenience.
    if let Expr::Pow(inner_base, inner_exp) = &base {
        let exp_is_int = numeric_value(&exp).is_some_and(|e| e.is_integer());
        let base_positive = numeric_value(inner_base).is_some_and(|v| v > BigRational::zero());
        if exp_is_int || base_positive {
            return pow(
                (**inner_base).clone(),
                mul(vec![(**inner_exp).clone(), exp]),
            );
        }
    }

    // (a*b)^n -> a^n * b^n for integer n.
    if let Some(e) = numeric_value(&exp) {
        if e.is_integer() {
            if let Expr::Mul(fs) = &base {
                let parts = fs
                    .iter()
                    .cloned()
                    .map(|f| pow(f, Expr::Int(e.numer().clone())))
                    .collect();
                return mul(parts);
            }
        }
    }

    // Both numeric: fold exactly where we can.
    if let (Some(b), Some(e)) = (numeric_value(&base), numeric_value(&exp)) {
        if e.is_integer() {
            let n = e.to_integer();
            // Estimate the result size (base bits × |exponent|) and refuse to
            // build it if it'd be enormous — keep the power symbolic instead.
            if let Some(v) = n.to_i64() {
                let base_bits = b.numer().bits().max(b.denom().bits()).max(1) as u128;
                let cost = base_bits.saturating_mul(v.unsigned_abs() as u128);
                if cost <= MAX_POW_RESULT_BITS {
                    return rat_to_expr(rat_pow(&b, &n));
                }
            }
            return Expr::Pow(Box::new(base), Box::new(exp));
        }
        // Rational exponent on a positive base: take the exact root if one
        // exists (e.g. sqrt(4)=2, 8^(1/3)=2); otherwise stay symbolic.
        if b > BigRational::zero() {
            if let Some(v) = exact_rational_root(&b, &e) {
                return rat_to_expr(v);
            }
        }
        // Principal square root of a negative real is imaginary:
        // sqrt(−a) = sqrt(a)·i.
        if b < BigRational::zero() && e == BigRational::new(BigInt::from(1), BigInt::from(2)) {
            return complex(int(0), pow(rat_to_expr(-b), half()));
        }
    }

    Expr::Pow(Box::new(base), Box::new(exp))
}

/// Exact number → BigFloat at precision `p` bits (the float-contagion bridge).
fn rat_to_bigfloat(r: &BigRational, p: usize) -> BigFloat {
    with_consts(|cc| {
        let n = BigFloat::parse(&r.numer().to_string(), RADIX, p, ROUND, cc);
        if r.is_integer() {
            n
        } else {
            let d = BigFloat::parse(&r.denom().to_string(), RADIX, p, ROUND, cc);
            n.div(&d, p, ROUND)
        }
    })
    .expect("constants cache unavailable (allocation failure)")
}

/// A side of a numeric power, as a float. `None` if it isn't a number.
fn to_float_value(e: &Expr, p: usize) -> Option<BigFloat> {
    match e {
        Expr::Float(bf, _) => Some(bf.clone()),
        _ => numeric_value(e).map(|r| rat_to_bigfloat(&r, p)),
    }
}

/// base^exp where at least one side is a Float: fold to a float when both
/// sides are numbers and the result is a finite real, else `None`.
fn float_pow(base: &Expr, exp: &Expr) -> Option<Expr> {
    let digits = match (base, exp) {
        (Expr::Float(_, a), Expr::Float(_, b)) => *a.max(b),
        (Expr::Float(_, a), _) | (_, Expr::Float(_, a)) => *a,
        _ => unreachable!("float_pow requires a float side"),
    };
    let p = prec_bits_for(digits);
    let b = to_float_value(base, p)?;
    let result = match exact_int_value(exp) {
        // Integer exponents use repeated multiplication, which is also
        // correct for negative bases (the general real pow is not).
        Some(n) => bf_powi(&b, n, p),
        None => {
            let e = to_float_value(exp, p)?;
            with_consts(|cc| b.pow(&e, p, ROUND, cc)).ok()?
        }
    };
    if result.is_nan() || result.is_inf() {
        return None;
    }
    Some(Expr::Float(result, digits))
}

/// If `e` is an exact integer that fits i64, return it.
fn exact_int_value(e: &Expr) -> Option<i64> {
    let r = numeric_value(e)?;
    if r.is_integer() {
        r.to_integer().to_i64()
    } else {
        None
    }
}

/// 1/(a+bi) = (a − bi)/(a² + b²).
fn complex_reciprocal(re: &Expr, im: &Expr) -> Expr {
    let denom = add_real(vec![
        mul_real(vec![re.clone(), re.clone()]),
        mul_real(vec![im.clone(), im.clone()]),
    ]);
    let inv = pow(denom, int(-1));
    complex(
        mul(vec![re.clone(), inv.clone()]),
        mul(vec![int(-1), im.clone(), inv]),
    )
}

/// A complex number to an integer power, by repeated complex multiplication.
fn complex_powi(re: &Expr, im: &Expr, n: i64) -> Expr {
    if n == 0 {
        return int(1);
    }
    let base = if n < 0 {
        complex_reciprocal(re, im)
    } else {
        complex(re.clone(), im.clone())
    };
    let mut acc = int(1);
    for _ in 0..n.unsigned_abs() {
        acc = mul(vec![acc, base.clone()]);
    }
    acc
}

/// Complex conjugate. Real values (including symbols, assumed real) are
/// returned unchanged.
pub fn conjugate(e: &Expr) -> Expr {
    match e {
        Expr::Complex(re, im) => complex((**re).clone(), mul(vec![int(-1), (**im).clone()])),
        _ => e.clone(),
    }
}

/// Real part (symbols assumed real).
pub fn real_part(e: &Expr) -> Expr {
    match e {
        Expr::Complex(re, _) => (**re).clone(),
        _ => e.clone(),
    }
}

/// Imaginary part (symbols assumed real, so 0).
pub fn imag_part(e: &Expr) -> Expr {
    match e {
        Expr::Complex(_, im) => (**im).clone(),
        _ => int(0),
    }
}

/// Modulus |z|: |a+bi| = sqrt(a²+b²); |x| for reals.
pub fn absolute_value(e: &Expr) -> Expr {
    match e {
        Expr::Complex(re, im) => {
            let sum = add(vec![
                mul(vec![(**re).clone(), (**re).clone()]),
                mul(vec![(**im).clone(), (**im).clone()]),
            ]);
            pow(sum, half())
        }
        _ => match numeric_value(e) {
            Some(r) => rat_to_expr(r.abs()),
            None => Expr::Func("abs".to_string(), vec![e.clone()]),
        },
    }
}

/// Build an applied function, with a few zero-cost identities.
pub fn func(name: &str, args: Vec<Expr>) -> Expr {
    if args.len() == 1 {
        let a = &args[0];
        match name {
            "sin" if is_zero(a) => return int(0),
            "cos" if is_zero(a) => return int(1),
            "exp" if is_zero(a) => return int(1),
            "ln" if is_one(a) => return int(0),
            _ => {}
        }
    }
    Expr::Func(name.to_string(), args)
}

// ---------------------------------------------------------------------------
// Symbolic operations
// ---------------------------------------------------------------------------

/// Symbolic differentiation w.r.t. a variable name.
pub fn differentiate(e: &Expr, var: &str) -> Expr {
    match e {
        Expr::Int(_) | Expr::Rat(_) | Expr::Float(..) | Expr::Const(_) => int(0), // d/dx of a constant
        Expr::Symbol(s) => {
            if s == var {
                int(1)
            } else {
                int(0)
            }
        }
        Expr::Add(ts) => add(ts.iter().map(|t| differentiate(t, var)).collect()),
        Expr::Mul(fs) => {
            // Product rule: d(∏ fᵢ) = Σᵢ (fᵢ' · ∏_{j≠i} fⱼ)
            let mut terms = Vec::new();
            for i in 0..fs.len() {
                let mut prod = Vec::with_capacity(fs.len());
                for (j, f) in fs.iter().enumerate() {
                    prod.push(if i == j {
                        differentiate(f, var)
                    } else {
                        f.clone()
                    });
                }
                terms.push(mul(prod));
            }
            add(terms)
        }
        Expr::Pow(b, ex) => {
            if !contains_symbol(ex, var) {
                // d(u^n) = n·u^(n-1)·u'  (n constant w.r.t. var)
                mul(vec![
                    (**ex).clone(),
                    pow((**b).clone(), add(vec![(**ex).clone(), int(-1)])),
                    differentiate(b, var),
                ])
            } else {
                // General: d(u^v) = u^v·(v'·ln u + v·u'/u)
                let term1 = mul(vec![differentiate(ex, var), func("ln", vec![(**b).clone()])]);
                let term2 = mul(vec![
                    (**ex).clone(),
                    differentiate(b, var),
                    pow((**b).clone(), int(-1)),
                ]);
                mul(vec![
                    pow((**b).clone(), (**ex).clone()),
                    add(vec![term1, term2]),
                ])
            }
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let u = &args[0];
            let du = differentiate(u, var);
            match name.as_str() {
                "sin" => mul(vec![func("cos", vec![u.clone()]), du]),
                "cos" => mul(vec![int(-1), func("sin", vec![u.clone()]), du]),
                "exp" => mul(vec![func("exp", vec![u.clone()]), du]),
                "ln" => mul(vec![du, pow(u.clone(), int(-1))]),
                "tan" => mul(vec![pow(func("cos", vec![u.clone()]), int(-2)), du]),
                _ => unknown_derivative(e, var),
            }
        }
        Expr::Func(..) => unknown_derivative(e, var),
        // Differentiation distributes over a matrix entrywise.
        Expr::Matrix(rows) => Expr::Matrix(map_entries(rows, |x| differentiate(x, var))),
        // Differentiation distributes over the real and imaginary parts.
        Expr::Complex(re, im) => complex(differentiate(re, var), differentiate(im, var)),
        // Booleans, functions, and structs are opaque to differentiation.
        Expr::Bool(_) | Expr::Function { .. } | Expr::Struct(_) => e.clone(),
        Expr::Equation(l, r) => Expr::Equation(
            Box::new(differentiate(l, var)),
            Box::new(differentiate(r, var)),
        ),
    }
}

fn unknown_derivative(e: &Expr, var: &str) -> Expr {
    // Leave it symbolic rather than guessing.
    Expr::Func("D".into(), vec![e.clone(), Expr::Symbol(var.to_string())])
}

/// Substitute every free occurrence of `var` with `val`, re-canonicalizing.
pub fn substitute(e: &Expr, var: &str, val: &Expr) -> Expr {
    match e {
        Expr::Symbol(s) if s == var => val.clone(),
        Expr::Int(_)
        | Expr::Rat(_)
        | Expr::Float(..)
        | Expr::Const(_)
        | Expr::Symbol(_)
        | Expr::Bool(_)
        | Expr::Function { .. } => e.clone(),
        Expr::Add(ts) => add(ts.iter().map(|t| substitute(t, var, val)).collect()),
        Expr::Mul(fs) => mul(fs.iter().map(|f| substitute(f, var, val)).collect()),
        Expr::Pow(b, ex) => pow(substitute(b, var, val), substitute(ex, var, val)),
        Expr::Func(name, args) => func(name, args.iter().map(|a| substitute(a, var, val)).collect()),
        Expr::Complex(re, im) => complex(substitute(re, var, val), substitute(im, var, val)),
        Expr::Matrix(rows) => Expr::Matrix(map_entries(rows, |x| substitute(x, var, val))),
        // Substitution reaches into struct fields (names are not symbols).
        Expr::Struct(fields) => Expr::Struct(
            fields
                .iter()
                .map(|(n, v)| (n.clone(), substitute(v, var, val)))
                .collect(),
        ),
        Expr::Equation(l, r) => Expr::Equation(
            Box::new(substitute(l, var, val)),
            Box::new(substitute(r, var, val)),
        ),
    }
}

/// Apply `f` to every entry of a matrix's rows, preserving shape.
pub fn map_entries(rows: &[Vec<Expr>], mut f: impl FnMut(&Expr) -> Expr) -> Vec<Vec<Expr>> {
    rows.iter()
        .map(|row| row.iter().map(&mut f).collect())
        .collect()
}

/// Distribute products over sums (and non-negative integer powers of sums),
/// then let the smart constructors recombine like terms.
/// `expand((x+1)^2)` -> `1 + x^2 + 2*x`.
pub fn expand(e: &Expr) -> Expr {
    if let Expr::Matrix(rows) = e {
        return Expr::Matrix(map_entries(rows, expand));
    }
    if let Expr::Complex(re, im) = e {
        return complex(expand(re), expand(im));
    }
    add(expand_terms(e))
}

/// Expand `e` into a flat list of additive terms, each with no top-level sum.
///
/// Distributing at the *term* level is the load-bearing detail: it guarantees
/// `mul` is never handed two sum-shaped operands, so it can't canonicalize
/// `(x+1)*(x+1)` back into `(x+1)^2` and send expansion into an infinite loop.
fn expand_terms(e: &Expr) -> Vec<Expr> {
    match e {
        Expr::Add(ts) => ts.iter().flat_map(expand_terms).collect(),
        Expr::Mul(fs) => {
            let mut acc = vec![int(1)];
            for f in fs {
                let terms = expand_terms(f);
                acc = cartesian_mul(&acc, &terms);
            }
            acc
        }
        Expr::Pow(b, ex) => {
            if let Some(r) = numeric_value(ex) {
                if r.is_integer() && r >= BigRational::zero() {
                    if let Some(n) = r.to_integer().to_usize() {
                        if n <= 64 {
                            let base_terms = expand_terms(b);
                            let mut acc = vec![int(1)];
                            for _ in 0..n {
                                acc = cartesian_mul(&acc, &base_terms);
                            }
                            return acc;
                        }
                    }
                }
            }
            // Not an expandable power: expand inside the base, keep the power.
            vec![pow(expand(b), (**ex).clone())]
        }
        _ => vec![e.clone()],
    }
}

/// Pairwise products of two term lists. Each input term is sum-free, and `mul`
/// of sum-free operands stays sum-free, so the result is too.
fn cartesian_mul(a: &[Expr], b: &[Expr]) -> Vec<Expr> {
    let mut out = Vec::with_capacity(a.len() * b.len());
    for x in a {
        for y in b {
            out.push(mul(vec![x.clone(), y.clone()]));
        }
    }
    out
}

/// Does `var` appear anywhere in `e`?
pub fn contains_symbol(e: &Expr, var: &str) -> bool {
    match e {
        Expr::Symbol(s) => s == var,
        Expr::Int(_)
        | Expr::Rat(_)
        | Expr::Float(..)
        | Expr::Const(_)
        | Expr::Bool(_)
        | Expr::Function { .. } => false,
        Expr::Add(ts) | Expr::Mul(ts) => ts.iter().any(|t| contains_symbol(t, var)),
        Expr::Pow(b, ex) => contains_symbol(b, var) || contains_symbol(ex, var),
        Expr::Func(_, args) => args.iter().any(|a| contains_symbol(a, var)),
        Expr::Complex(re, im) => contains_symbol(re, var) || contains_symbol(im, var),
        Expr::Matrix(rows) => rows.iter().flatten().any(|e| contains_symbol(e, var)),
        Expr::Struct(fields) => fields.iter().any(|(_, v)| contains_symbol(v, var)),
        Expr::Equation(l, r) => contains_symbol(l, var) || contains_symbol(r, var),
    }
}

const ROUND: RoundingMode = RoundingMode::ToEven;
const RADIX: Radix = Radix::Dec;

/// Cross the exact -> inexact boundary: evaluate `e` to `digits` significant
/// decimal digits of arbitrary-precision floating point.
///
/// We compute with guard bits beyond what's requested, then round for display.
/// This is the *only* place floats enter the system — exactness is the default
/// everywhere else.
pub fn numeric_eval(e: &Expr, digits: usize) -> Result<Expr, String> {
    let digits = digits.clamp(1, 100_000);
    // N over a matrix evaluates each entry — e.g. numeric eigenvalues.
    if let Expr::Matrix(rows) = e {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut new_row = Vec::with_capacity(row.len());
            for entry in row {
                new_row.push(numeric_eval(entry, digits)?);
            }
            out.push(new_row);
        }
        return Ok(Expr::Matrix(out));
    }
    let prec_bits = prec_bits_for(digits);

    // Complex-valued expressions (incl. transcendentals of complex arguments)
    // go through the complex evaluator; everything else stays on the real path.
    if contains_complex(e) {
        let (re, im) = with_consts(|cc| to_complex(e, prec_bits, cc))??;
        if re.is_nan() || im.is_nan() {
            return Err("numeric result is undefined".into());
        }
        // Snap only results of transcendental evaluation, where a component
        // that is mathematically zero comes back as cancellation residue
        // (exp(iπ) → −1 + 1e−40·i). A purely arithmetic value has full
        // relative precision in each component, so a tiny part there is
        // genuine data (1 + 10^-50·I) and must survive.
        let (re, im) = if involves_transcendentals(e) {
            snap_negligible(re, im, digits, prec_bits)
        } else {
            (re, im)
        };
        return Ok(if im.is_zero() {
            Expr::Float(re, digits)
        } else {
            Expr::Complex(Box::new(Expr::Float(re, digits)), Box::new(Expr::Float(im, digits)))
        });
    }

    let value = with_consts(|cc| to_bigfloat(e, prec_bits, cc))??;
    if value.is_nan() {
        return Err("numeric result is undefined (e.g. a real power of a negative number)".into());
    }
    Ok(Expr::Float(value, digits))
}

/// Working precision in bits for `digits` significant decimal digits, with
/// guard bits beyond what's displayed.
fn prec_bits_for(digits: usize) -> usize {
    ((digits as f64) * std::f64::consts::LOG2_10).ceil() as usize + 32
}

thread_local! {
    /// astro-float's constants machinery (π, e). Its internal cache grows
    /// monotonically with the precision requested, so one long-lived instance
    /// per thread amortizes the cost across every `N(...)` call — rebuilding
    /// it per call recomputes π from scratch each time.
    static CONSTS: std::cell::RefCell<Option<Consts>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with the thread's cached `Consts`. Closures must be pure BigFloat
/// math: building `Expr`s through the smart constructors inside `f` could
/// re-enter this cell and panic on the double borrow.
fn with_consts<T>(f: impl FnOnce(&mut Consts) -> T) -> Result<T, String> {
    CONSTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot =
                Some(Consts::new().map_err(|_| "could not initialise constants".to_string())?);
        }
        Ok(f(slot.as_mut().expect("just initialised")))
    })
}

fn bf_int(i: i64, p: usize) -> BigFloat {
    BigFloat::from_i64(i, p)
}

fn to_bigfloat(e: &Expr, p: usize, cc: &mut Consts) -> Result<BigFloat, String> {
    match e {
        // Parse via decimal string so arbitrarily large integers convert exactly.
        Expr::Int(i) => Ok(BigFloat::parse(&i.to_string(), RADIX, p, ROUND, cc)),
        Expr::Rat(r) => {
            let n = BigFloat::parse(&r.numer().to_string(), RADIX, p, ROUND, cc);
            let d = BigFloat::parse(&r.denom().to_string(), RADIX, p, ROUND, cc);
            Ok(n.div(&d, p, ROUND))
        }
        Expr::Float(bf, _) => Ok(bf.clone()),
        Expr::Const(Constant::Pi) => Ok(cc.pi(p, ROUND)),
        Expr::Const(Constant::E) => Ok(cc.e(p, ROUND)),
        Expr::Symbol(s) => Err(format!("cannot numerically evaluate free symbol '{}'", s)),
        Expr::Add(ts) => {
            let mut acc = bf_int(0, p);
            for t in ts {
                acc = acc.add(&to_bigfloat(t, p, cc)?, p, ROUND);
            }
            Ok(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = bf_int(1, p);
            for f in fs {
                acc = acc.mul(&to_bigfloat(f, p, cc)?, p, ROUND);
            }
            Ok(acc)
        }
        Expr::Pow(b, ex) => {
            let base = to_bigfloat(b, p, cc)?;
            // Integer exponents use exact repeated multiplication, which also
            // handles negative bases correctly (a *real* power of a negative
            // base, e.g. (-1)^(1/2), is genuinely not real → NaN).
            if let Some(r) = numeric_value(ex) {
                if r.is_integer() {
                    if let Some(n) = r.to_integer().to_i64() {
                        return Ok(bf_powi(&base, n, p));
                    }
                }
            }
            let exp = to_bigfloat(ex, p, cc)?;
            Ok(base.pow(&exp, p, ROUND, cc))
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let x = to_bigfloat(&args[0], p, cc)?;
            match name.as_str() {
                "sin" => Ok(x.sin(p, ROUND, cc)),
                "cos" => Ok(x.cos(p, ROUND, cc)),
                "tan" => Ok(x.tan(p, ROUND, cc)),
                "exp" => Ok(x.exp(p, ROUND, cc)),
                "ln" => Ok(x.ln(p, ROUND, cc)),
                _ => Err(format!("cannot numerically evaluate '{}'", name)),
            }
        }
        Expr::Func(name, _) => Err(format!("cannot numerically evaluate '{}'", name)),
        Expr::Matrix(..) => {
            Err("cannot collapse a matrix to a single number (try N on its entries)".to_string())
        }
        Expr::Complex(..) => {
            Err("cannot evaluate a complex number to a single real float".to_string())
        }
        Expr::Bool(_) => Err("cannot numerically evaluate a boolean".to_string()),
        Expr::Function { .. } => Err("cannot numerically evaluate a function".to_string()),
        Expr::Equation(..) => Err("cannot numerically evaluate an equation".to_string()),
        Expr::Struct(..) => Err("cannot numerically evaluate a struct".to_string()),
    }
}

/// Exact integer power by square-and-multiply (handles negative exponents and
/// negative bases, unlike the general real `pow`).
fn bf_powi(base: &BigFloat, n: i64, p: usize) -> BigFloat {
    if n == 0 {
        return bf_int(1, p);
    }
    let mut result = bf_int(1, p);
    let mut b = base.clone();
    let mut e = n.unsigned_abs();
    while e > 0 {
        if e & 1 == 1 {
            result = result.mul(&b, p, ROUND);
        }
        e >>= 1;
        if e > 0 {
            b = b.mul(&b, p, ROUND);
        }
    }
    if n < 0 {
        bf_int(1, p).div(&result, p, ROUND)
    } else {
        result
    }
}

/// The exact rational value of a float — lossless, since a binary float is
/// exactly ±m·2^k. `None` for NaN and infinities. This is what makes
/// float-vs-exact comparison decidable: no rounding is involved, we compare
/// the value the float actually holds.
pub fn float_to_rational(bf: &BigFloat) -> Option<BigRational> {
    if bf.is_zero() {
        return Some(BigRational::zero());
    }
    let (words, _, sign, exp, _) = bf.as_raw_parts()?; // None for NaN/Inf
    let mut m = BigInt::zero();
    for w in words.iter().rev() {
        m = (m << astro_float::WORD_BIT_SIZE) + w;
    }
    if sign.is_negative() {
        m = -m;
    }
    // The mantissa reads as a binary fraction in [1/2, 1): value = m·2^(e−bits).
    let shift = exp as i64 - (words.len() * astro_float::WORD_BIT_SIZE) as i64;
    Some(if shift >= 0 {
        BigRational::from_integer(m << shift as usize)
    } else {
        BigRational::new(m, BigInt::one() << (-shift) as usize)
    })
}

// ---------------------------------------------------------------------------
// Complex numeric evaluation (transcendentals via polar form)
// ---------------------------------------------------------------------------

/// A numeric complex value (real, imaginary) in arbitrary-precision floats.
type Cpx = (BigFloat, BigFloat);

/// Does `e` contain any complex subexpression? (Used to pick the real vs.
/// complex numeric evaluator, keeping the common real path fast.)
fn contains_complex(e: &Expr) -> bool {
    match e {
        Expr::Complex(..) => true,
        Expr::Add(ts) | Expr::Mul(ts) | Expr::Func(_, ts) => ts.iter().any(contains_complex),
        Expr::Pow(b, x) => contains_complex(b) || contains_complex(x),
        Expr::Equation(l, r) => contains_complex(l) || contains_complex(r),
        Expr::Matrix(rows) => rows.iter().flatten().any(contains_complex),
        _ => false,
    }
}

fn bf_neg(x: &BigFloat, p: usize) -> BigFloat {
    BigFloat::from_i64(0, p).sub(x, p, ROUND)
}

fn c_add(x: &Cpx, y: &Cpx, p: usize) -> Cpx {
    (x.0.add(&y.0, p, ROUND), x.1.add(&y.1, p, ROUND))
}

fn c_mul(x: &Cpx, y: &Cpx, p: usize) -> Cpx {
    let re = x.0.mul(&y.0, p, ROUND).sub(&x.1.mul(&y.1, p, ROUND), p, ROUND);
    let im = x.0.mul(&y.1, p, ROUND).add(&x.1.mul(&y.0, p, ROUND), p, ROUND);
    (re, im)
}

fn c_div(x: &Cpx, y: &Cpx, p: usize) -> Cpx {
    let denom = y.0.mul(&y.0, p, ROUND).add(&y.1.mul(&y.1, p, ROUND), p, ROUND);
    let re = x
        .0
        .mul(&y.0, p, ROUND)
        .add(&x.1.mul(&y.1, p, ROUND), p, ROUND)
        .div(&denom, p, ROUND);
    let im = x
        .1
        .mul(&y.0, p, ROUND)
        .sub(&x.0.mul(&y.1, p, ROUND), p, ROUND)
        .div(&denom, p, ROUND);
    (re, im)
}

/// z to an integer power, by repeated squaring (negative → reciprocal).
fn c_powi(z: &Cpx, n: i64, p: usize) -> Cpx {
    let mut acc = (BigFloat::from_i64(1, p), BigFloat::from_i64(0, p));
    let mut base = z.clone();
    let mut e = n.unsigned_abs();
    while e > 0 {
        if e & 1 == 1 {
            acc = c_mul(&acc, &base, p);
        }
        e >>= 1;
        if e > 0 {
            base = c_mul(&base, &base, p);
        }
    }
    if n < 0 {
        let one = (BigFloat::from_i64(1, p), BigFloat::from_i64(0, p));
        c_div(&one, &acc, p)
    } else {
        acc
    }
}

/// exp(a+bi) = e^a·(cos b + i·sin b).
fn c_exp(z: &Cpx, p: usize, cc: &mut Consts) -> Cpx {
    let ea = z.0.exp(p, ROUND, cc);
    let cos_b = z.1.cos(p, ROUND, cc);
    let sin_b = z.1.sin(p, ROUND, cc);
    (ea.mul(&cos_b, p, ROUND), ea.mul(&sin_b, p, ROUND))
}

/// sin(a+bi) = sin a·cosh b + i·cos a·sinh b.
fn c_sin(z: &Cpx, p: usize, cc: &mut Consts) -> Cpx {
    let re = z.0.sin(p, ROUND, cc).mul(&z.1.cosh(p, ROUND, cc), p, ROUND);
    let im = z.0.cos(p, ROUND, cc).mul(&z.1.sinh(p, ROUND, cc), p, ROUND);
    (re, im)
}

/// cos(a+bi) = cos a·cosh b − i·sin a·sinh b.
fn c_cos(z: &Cpx, p: usize, cc: &mut Consts) -> Cpx {
    let re = z.0.cos(p, ROUND, cc).mul(&z.1.cosh(p, ROUND, cc), p, ROUND);
    let im = z.0.sin(p, ROUND, cc).mul(&z.1.sinh(p, ROUND, cc), p, ROUND);
    (re, bf_neg(&im, p))
}

fn c_tan(z: &Cpx, p: usize, cc: &mut Consts) -> Cpx {
    c_div(&c_sin(z, p, cc), &c_cos(z, p, cc), p)
}

/// arg(a+bi) ∈ (−π, π], computed as atan2(b, a).
fn c_arg(a: &BigFloat, b: &BigFloat, p: usize, cc: &mut Consts) -> BigFloat {
    let pi = cc.pi(p, ROUND);
    if a.is_zero() {
        let half_pi = pi.div(&BigFloat::from_i64(2, p), p, ROUND);
        return if b.is_negative() {
            bf_neg(&half_pi, p)
        } else if b.is_zero() {
            BigFloat::from_i64(0, p)
        } else {
            half_pi
        };
    }
    let base = b.div(a, p, ROUND).atan(p, ROUND, cc);
    if a.is_positive() {
        base
    } else if b.is_negative() {
        base.sub(&pi, p, ROUND)
    } else {
        base.add(&pi, p, ROUND)
    }
}

/// ln(z) = ln|z| + i·arg(z).
fn c_ln(z: &Cpx, p: usize, cc: &mut Consts) -> Result<Cpx, String> {
    let mod_sq = z.0.mul(&z.0, p, ROUND).add(&z.1.mul(&z.1, p, ROUND), p, ROUND);
    if mod_sq.is_zero() {
        return Err("ln(0) is undefined".to_string());
    }
    let ln_mod = mod_sq.sqrt(p, ROUND).ln(p, ROUND, cc);
    Ok((ln_mod, c_arg(&z.0, &z.1, p, cc)))
}

/// General complex power z^w = exp(w·ln z).
fn c_pow(z: &Cpx, w: &Cpx, p: usize, cc: &mut Consts) -> Result<Cpx, String> {
    if z.0.is_zero() && z.1.is_zero() {
        return Ok((BigFloat::from_i64(0, p), BigFloat::from_i64(0, p)));
    }
    let ln_z = c_ln(z, p, cc)?;
    Ok(c_exp(&c_mul(w, &ln_z, p), p, cc))
}

/// Evaluate any expression to a numeric complex value.
fn to_complex(e: &Expr, p: usize, cc: &mut Consts) -> Result<Cpx, String> {
    match e {
        Expr::Int(_) | Expr::Rat(_) | Expr::Float(..) | Expr::Const(_) => {
            Ok((to_bigfloat(e, p, cc)?, BigFloat::from_i64(0, p)))
        }
        Expr::Complex(re, im) => Ok((to_bigfloat(re, p, cc)?, to_bigfloat(im, p, cc)?)),
        Expr::Symbol(s) => Err(format!("cannot numerically evaluate free symbol '{}'", s)),
        Expr::Add(ts) => {
            let mut acc = (BigFloat::from_i64(0, p), BigFloat::from_i64(0, p));
            for t in ts {
                acc = c_add(&acc, &to_complex(t, p, cc)?, p);
            }
            Ok(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = (BigFloat::from_i64(1, p), BigFloat::from_i64(0, p));
            for f in fs {
                acc = c_mul(&acc, &to_complex(f, p, cc)?, p);
            }
            Ok(acc)
        }
        Expr::Pow(b, ex) => {
            let z = to_complex(b, p, cc)?;
            if let Some(r) = numeric_value(ex) {
                if r.is_integer() {
                    if let Some(n) = r.to_integer().to_i64() {
                        return Ok(c_powi(&z, n, p));
                    }
                }
            }
            let w = to_complex(ex, p, cc)?;
            c_pow(&z, &w, p, cc)
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let z = to_complex(&args[0], p, cc)?;
            match name.as_str() {
                "exp" => Ok(c_exp(&z, p, cc)),
                "sin" => Ok(c_sin(&z, p, cc)),
                "cos" => Ok(c_cos(&z, p, cc)),
                "tan" => Ok(c_tan(&z, p, cc)),
                "ln" => c_ln(&z, p, cc),
                _ => Err(format!("cannot numerically evaluate '{}'", name)),
            }
        }
        Expr::Func(name, _) => Err(format!("cannot numerically evaluate '{}'", name)),
        Expr::Bool(_) => Err("cannot numerically evaluate a boolean".to_string()),
        Expr::Function { .. } => Err("cannot numerically evaluate a function".to_string()),
        Expr::Matrix(..) => Err("cannot collapse a matrix to a single number".to_string()),
        Expr::Equation(..) => Err("cannot numerically evaluate an equation".to_string()),
        Expr::Struct(..) => Err("cannot numerically evaluate a struct".to_string()),
    }
}

/// Does numeric evaluation of `e` pass through transcendental operations —
/// function applications, symbolic constants, or non-integer powers (which
/// evaluate via exp(w·ln z))? Only those can leave cancellation residue, so
/// only their results are eligible for `snap_negligible`.
fn involves_transcendentals(e: &Expr) -> bool {
    match e {
        Expr::Const(_) | Expr::Func(..) => true,
        Expr::Add(ts) | Expr::Mul(ts) => ts.iter().any(involves_transcendentals),
        Expr::Pow(b, x) => {
            involves_transcendentals(b)
                || involves_transcendentals(x)
                || !numeric_value(x).is_some_and(|r| r.is_integer())
        }
        Expr::Complex(re, im) => involves_transcendentals(re) || involves_transcendentals(im),
        Expr::Matrix(rows) => rows.iter().flatten().any(involves_transcendentals),
        Expr::Equation(l, r) => involves_transcendentals(l) || involves_transcendentals(r),
        _ => false,
    }
}

/// Zero out a real or imaginary part that is negligible relative to the other
/// at the requested precision — so e.g. exp(iπ) reads as exactly −1 rather than
/// −1 + 1e−40·i. This is a *display* convenience on an already-approximate value.
fn snap_negligible(re: BigFloat, im: BigFloat, digits: usize, p: usize) -> Cpx {
    let abs_re = re.abs();
    let abs_im = im.abs();
    let bf_lt = |x: &BigFloat, y: &BigFloat| x.sub(y, p, ROUND).is_negative();
    let scale = if bf_lt(&abs_re, &abs_im) {
        abs_im.clone()
    } else {
        abs_re.clone()
    };
    if scale.is_zero() {
        return (re, im);
    }
    let ten = BigFloat::from_i64(10, p);
    let threshold = scale.mul(&bf_powi(&ten, -(digits as i64), p), p, ROUND);
    let zero = BigFloat::from_i64(0, p);
    let re = if bf_lt(&abs_re, &threshold) { zero.clone() } else { re };
    let im = if bf_lt(&abs_im, &threshold) { zero } else { im };
    (re, im)
}

/// Render a `BigFloat` as a clean decimal string with `digits` significant
/// figures.
///
/// The binary->decimal conversion is done in exact `BigInt` arithmetic on the
/// float's true rational value. astro-float's own `Display`/`format` is never
/// used: its radix conversion has an arithmetic-overflow panic on 32-bit
/// targets, which took down the wasm32 build for every float it printed
/// (https://github.com/stencillogic/astro-float/issues/43).
pub(crate) fn format_bigfloat(bf: &BigFloat, digits: usize) -> String {
    if bf.is_nan() {
        return "NaN".to_string();
    }
    if bf.is_inf_pos() {
        return "Inf".to_string();
    }
    if bf.is_inf_neg() {
        return "-Inf".to_string();
    }
    let v = float_to_rational(bf).expect("NaN/Inf were handled above");
    if v.is_zero() {
        return "0".to_string();
    }
    let neg = v.is_negative();
    let (mut d, lead) = decimal_digits(&v.abs(), digits.max(1));
    while d.len() > 1 && *d.last().unwrap() == 0 {
        d.pop();
    }

    let text = render_decimal(&d, lead);
    if neg {
        format!("-{}", text)
    } else {
        text
    }
}

/// The first `k` significant decimal digits of a positive rational, rounded
/// half-up on the exact value, plus the decimal exponent of the leading digit.
fn decimal_digits(v: &BigRational, k: usize) -> (Vec<u8>, i64) {
    // n-bit / m-bit puts v in (2^(b-1), 2^(b+1)), so this floor(log10) estimate
    // is within one of the truth; the loop below settles the difference.
    let b = v.numer().bits() as i64 - v.denom().bits() as i64;
    let mut lead = ((b - 1) as i128 * 301_029_995_663_981).div_euclid(1_000_000_000_000_000) as i64;
    loop {
        // Scale so the last kept digit (decimal exponent `lead - k + 1`) lands
        // at the units place, then floor-divide: q = floor(v / 10^s).
        let s = lead - k as i64 + 1;
        let mut num = v.numer().clone();
        let mut den = v.denom().clone();
        if s >= 0 {
            den *= num_traits::pow(BigInt::from(10), s as usize);
        } else {
            num *= num_traits::pow(BigInt::from(10), (-s) as usize);
        }
        let mut q = &num / &den;
        let len = if q.is_zero() { 0 } else { q.to_string().len() };
        if len < k {
            lead -= 1;
            continue;
        }
        if len > k {
            lead += 1;
            continue;
        }
        // Half-up against the exact remainder; a carry that grows 99…9 into
        // 100…0 shifts the leading digit up one place.
        if (num - &q * &den) * 2 >= den {
            q += 1;
            if q.to_string().len() > k {
                q /= 10;
                lead += 1;
            }
        }
        return (q.to_string().bytes().map(|c| c - b'0').collect(), lead);
    }
}

fn render_decimal(d: &[u8], lead: i64) -> String {
    let digits: String = d.iter().map(|x| (b'0' + x) as char).collect();
    // Plain decimal in a friendly range; scientific otherwise.
    if !(-10..=20).contains(&lead) {
        let mant = if d.len() == 1 {
            digits
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        return format!("{}e{}", mant, lead);
    }
    if lead < 0 {
        format!("0.{}{}", "0".repeat((-lead - 1) as usize), digits)
    } else {
        let lead = lead as usize;
        if lead + 1 >= d.len() {
            format!("{}{}", digits, "0".repeat(lead + 1 - d.len()))
        } else {
            format!("{}.{}", &digits[..lead + 1], &digits[lead + 1..])
        }
    }
}

// ---------------------------------------------------------------------------
// Internal numeric / structural helpers
// ---------------------------------------------------------------------------

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Int(i) if i.is_zero())
}

fn is_one(e: &Expr) -> bool {
    matches!(e, Expr::Int(i) if i.is_one())
}

fn flatten_add(e: Expr, out: &mut Vec<Expr>) {
    if let Expr::Add(ts) = e {
        for t in ts {
            flatten_add(t, out);
        }
    } else {
        out.push(e);
    }
}

fn flatten_mul(e: Expr, out: &mut Vec<Expr>) {
    if let Expr::Mul(fs) = e {
        for f in fs {
            flatten_mul(f, out);
        }
    } else {
        out.push(e);
    }
}

/// Split a term into (numeric coefficient, basis). Relies on canonical `Mul`
/// putting any numeric coefficient first.
fn split_coeff(e: &Expr) -> (BigRational, Expr) {
    if let Expr::Mul(fs) = e {
        if let Some(first) = fs.first() {
            if let Some(c) = numeric_value(first) {
                return (c, mul(fs[1..].to_vec()));
            }
        }
    }
    (BigRational::one(), e.clone())
}

/// Split a factor into (base, exponent).
fn split_pow(e: &Expr) -> (Expr, Expr) {
    if let Expr::Pow(b, ex) = e {
        ((**b).clone(), (**ex).clone())
    } else {
        (e.clone(), int(1))
    }
}

/// Exact integer/rational power with a (possibly negative) integer exponent.
fn rat_pow(base: &BigRational, n: &BigInt) -> BigRational {
    if n.is_zero() {
        return BigRational::one();
    }
    let mag = n
        .abs()
        .to_usize()
        .expect("exponent magnitude too large for this prototype");
    let p = num_traits::pow::pow(base.clone(), mag);
    if n.is_negative() {
        p.recip()
    } else {
        p
    }
}

/// `b^e` for positive rational `b` and rational `e`, returning a rational iff
/// the result is exactly rational (e.g. 4^(1/2)=2). Otherwise `None`.
fn exact_rational_root(b: &BigRational, e: &BigRational) -> Option<BigRational> {
    let q = e.denom().to_u32()?; // denominator is positive after normalization
    let bn = b.numer();
    let bd = b.denom();
    let rn = bn.nth_root(q);
    if num_traits::pow::pow(rn.clone(), q as usize) != *bn {
        return None;
    }
    let rd = bd.nth_root(q);
    if num_traits::pow::pow(rd.clone(), q as usize) != *bd {
        return None;
    }
    let root = BigRational::new(rn, rd);
    // The result is root^numer — apply the same result-size cap as the
    // integer-exponent path in `pow`. Without it, 8^(10^15/3) would build a
    // petabyte bignum, and a numerator beyond usize would panic in rat_pow.
    let n = e.numer().to_i64()?;
    let root_bits = root.numer().bits().max(root.denom().bits()).max(1) as u128;
    if root_bits.saturating_mul(n.unsigned_abs() as u128) > MAX_POW_RESULT_BITS {
        return None;
    }
    Some(rat_pow(&root, e.numer()))
}

/// Deterministic total order for canonical operand sorting. The key is
/// (type rank, rendered string) — cheap, stable, and good enough to make
/// structurally-equal expressions compare equal.
fn sort_operands(v: &mut [Expr]) {
    v.sort_by_key(sort_key);
}

fn sort_key(e: &Expr) -> (u8, String) {
    (type_rank(e), format!("{}", e))
}

fn type_rank(e: &Expr) -> u8 {
    match e {
        Expr::Int(_) | Expr::Rat(_) | Expr::Float(..) | Expr::Complex(..) => 0,
        Expr::Const(_) => 1,
        Expr::Symbol(_) => 2,
        Expr::Pow(..) => 3,
        Expr::Func(..) => 4,
        Expr::Mul(_) => 5,
        Expr::Add(_) => 6,
        Expr::Matrix(_) => 7,
        Expr::Bool(_) => 8,
        Expr::Function { .. } => 9,
        Expr::Equation(..) => 10,
        Expr::Struct(_) => 11,
    }
}

// ---------------------------------------------------------------------------
// Display — precedence-aware pretty printer
// ---------------------------------------------------------------------------

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render(0))
    }
}

// Precedence levels used only for parenthesization while printing.
const PREC_EQ: u8 = 1;
const PREC_ADD: u8 = 2;
const PREC_MUL: u8 = 3;
const PREC_POW: u8 = 4;
const PREC_ATOM: u8 = 10;

impl Expr {
    fn render(&self, parent: u8) -> String {
        let (prec, s) = self.render_inner();
        if prec < parent {
            format!("({})", s)
        } else {
            s
        }
    }

    fn render_inner(&self) -> (u8, String) {
        match self {
            Expr::Int(i) => (PREC_ATOM, i.to_string()),
            // A rational prints as a division, so it needs the precedence of one
            // — otherwise `(11/5)^x` would print as `11/5^x` (= 11/(5^x)).
            Expr::Rat(r) => (PREC_MUL, format!("{}/{}", r.numer(), r.denom())),
            Expr::Float(bf, digits) => (PREC_ATOM, format_bigfloat(bf, *digits)),
            Expr::Const(Constant::Pi) => (PREC_ATOM, "π".to_string()),
            Expr::Const(Constant::E) => (PREC_ATOM, "e".to_string()),
            Expr::Symbol(s) => (PREC_ATOM, s.clone()),
            Expr::Func(name, args) => {
                let inner = args
                    .iter()
                    .map(|a| a.render(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                (PREC_ATOM, format!("{}({})", name, inner))
            }
            Expr::Pow(b, e) => {
                if is_one_half(e) {
                    (PREC_ATOM, format!("sqrt({})", b.render(0)))
                } else {
                    let base = b.render(PREC_POW + 1);
                    let exp = if needs_paren_in_exponent(e) {
                        format!("({})", e.render(0))
                    } else {
                        e.render(PREC_POW + 1)
                    };
                    (PREC_POW, format!("{}^{}", base, exp))
                }
            }
            Expr::Mul(fs) => {
                let mut factors = fs.clone();
                let mut negative = false;
                if let Some(Expr::Int(i)) = factors.first() {
                    if *i == BigInt::from(-1) {
                        negative = true;
                        factors.remove(0);
                    }
                }
                let body = if factors.is_empty() {
                    "1".to_string()
                } else {
                    factors
                        .iter()
                        .map(|x| x.render(PREC_MUL))
                        .collect::<Vec<_>>()
                        .join("*")
                };
                (PREC_MUL, if negative { format!("-{}", body) } else { body })
            }
            Expr::Add(ts) => {
                let mut out = String::new();
                for (i, t) in ts.iter().enumerate() {
                    if i == 0 {
                        out.push_str(&t.render(PREC_ADD));
                    } else if let Some(pos) = negative_part(t) {
                        out.push_str(" - ");
                        out.push_str(&pos.render(PREC_ADD));
                    } else {
                        out.push_str(" + ");
                        out.push_str(&t.render(PREC_ADD));
                    }
                }
                (PREC_ADD, out)
            }
            Expr::Matrix(rows) => (PREC_ATOM, render_matrix(rows)),
            Expr::Complex(re, im) => render_complex(re, im),
            Expr::Bool(b) => (PREC_ATOM, if *b { "true" } else { "false" }.to_string()),
            Expr::Function { params, .. } => {
                (PREC_ATOM, format!("<function({})>", params.join(", ")))
            }
            Expr::Equation(l, r) => (
                PREC_EQ,
                format!("{} = {}", l.render(PREC_ADD), r.render(PREC_ADD)),
            ),
            // Re-parseable through the `struct(...)` builtin: equation-valued
            // fields render above PREC_EQ, so they come back parenthesized.
            Expr::Struct(fields) => (
                PREC_ATOM,
                format!(
                    "struct({})",
                    fields
                        .iter()
                        .map(|(n, v)| format!("{} = {}", n, v.render(PREC_ADD)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        }
    }
}

/// Render a matrix as a column-aligned, multi-line block:
/// ```text
/// [  1   2 ]
/// [ 10  12 ]
/// ```
fn render_matrix(rows: &[Vec<Expr>]) -> String {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|e| e.render(0)).collect())
        .collect();
    let cols = cells[0].len();
    let mut widths = vec![0usize; cols];
    for row in &cells {
        for (j, c) in row.iter().enumerate() {
            widths[j] = widths[j].max(c.chars().count());
        }
    }
    cells
        .iter()
        .map(|row| {
            let inner = row
                .iter()
                .enumerate()
                .map(|(j, c)| format!("{:>w$}", c, w = widths[j]))
                .collect::<Vec<_>>()
                .join("  ");
            format!("[ {} ]", inner)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a complex number as `re + im*I`, `re - |im|*I`, or just the imaginary
/// part when the real part is zero. Works on strings so it's uniform across
/// integer / rational / float / symbolic parts.
fn render_complex(re: &Expr, im: &Expr) -> (u8, String) {
    let re_s = re.render(PREC_ADD);
    // Split the imaginary coefficient into sign and magnitude.
    let coeff = im.render(PREC_MUL);
    let (neg, mag) = match coeff.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, coeff),
    };
    let imag = if mag == "1" {
        "I".to_string()
    } else {
        format!("{}*I", mag)
    };

    if re_s == "0" {
        let s = if neg { format!("-{}", imag) } else { imag };
        return (PREC_MUL, s);
    }
    let s = if neg {
        format!("{} - {}", re_s, imag)
    } else {
        format!("{} + {}", re_s, imag)
    };
    (PREC_ADD, s)
}

pub(crate) fn is_one_half(e: &Expr) -> bool {
    matches!(e, Expr::Rat(r) if *r.numer() == BigInt::from(1) && *r.denom() == BigInt::from(2))
}

fn needs_paren_in_exponent(e: &Expr) -> bool {
    match e {
        Expr::Rat(_) | Expr::Add(_) | Expr::Mul(_) | Expr::Float(..) => true,
        Expr::Int(i) => i.is_negative(),
        _ => false,
    }
}

/// If `e` is "negative", return its positive counterpart (so `Add` can print
/// `a - b` instead of `a + -b`).
pub(crate) fn negative_part(e: &Expr) -> Option<Expr> {
    match e {
        Expr::Int(i) if i.is_negative() => Some(Expr::Int(-i.clone())),
        Expr::Rat(r) if r.is_negative() => Some(Expr::Rat(-r.clone())),
        Expr::Float(bf, d) if bf.is_negative() => Some(Expr::Float(bf.neg(), *d)),
        Expr::Mul(fs) => {
            // A product is "negative" when its leading numeric coefficient is.
            let negated = match fs.first()? {
                Expr::Int(i) if i.is_negative() => Expr::Int(-i.clone()),
                Expr::Rat(r) if r.is_negative() => Expr::Rat(-r.clone()),
                Expr::Float(bf, d) if bf.is_negative() => Expr::Float(bf.neg(), *d),
                _ => return None,
            };
            let mut nf = fs.clone();
            nf[0] = negated;
            if matches!(&nf[0], Expr::Int(i) if i.is_one()) {
                nf.remove(0); // drop a leading coefficient of 1
            }
            Some(if nf.len() == 1 {
                nf.pop().unwrap()
            } else {
                Expr::Mul(nf)
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> Expr {
        Expr::Symbol(s.to_string())
    }

    #[test]
    fn add_combines_like_terms() {
        assert_eq!(add(vec![sym("x"), sym("x")]), mul(vec![int(2), sym("x")]));
        assert_eq!(add(vec![sym("x"), mul(vec![int(-1), sym("x")])]), int(0));
    }

    #[test]
    fn add_is_canonical_regardless_of_order() {
        // Commutative canonical form: same multiset of terms ⇒ equal Expr.
        let a = add(vec![int(1), sym("x"), sym("y")]);
        let b = add(vec![sym("y"), int(1), sym("x")]);
        assert_eq!(a, b);
    }

    #[test]
    fn mul_collects_powers_of_the_same_base() {
        // x * x == x^2
        assert_eq!(mul(vec![sym("x"), sym("x")]), pow(sym("x"), int(2)));
        // numeric coefficient is folded
        assert_eq!(mul(vec![int(2), int(3), sym("x")]), mul(vec![int(6), sym("x")]));
        // zero annihilates
        assert_eq!(mul(vec![int(0), sym("x")]), int(0));
    }

    #[test]
    fn pow_identities() {
        assert_eq!(pow(sym("x"), int(0)), int(1));
        assert_eq!(pow(sym("x"), int(1)), sym("x"));
        assert_eq!(pow(int(1), sym("x")), int(1));
        assert_eq!(pow(int(2), int(10)), int(1024));
    }

    #[test]
    fn radicals_reduce_via_general_rules() {
        // sqrt(2)*sqrt(2) == 2 and sqrt(2)^2 == 2 fall out of mul/pow rules.
        let sqrt2 = pow(int(2), half());
        assert_eq!(mul(vec![sqrt2.clone(), sqrt2.clone()]), int(2));
        assert_eq!(pow(sqrt2, int(2)), int(2));
    }

    #[test]
    fn canonicalization_is_idempotent() {
        // Rebuilding a result through the constructors changes nothing.
        let e = add(vec![
            mul(vec![int(3), pow(sym("x"), int(2))]),
            mul(vec![int(2), sym("x")]),
            int(1),
        ]);
        let rebuilt = add(vec![e.clone()]);
        assert_eq!(e, rebuilt);
    }

    #[test]
    fn huge_exponents_stay_symbolic_instead_of_hanging() {
        // 2^(10^15) must not try to build the bignum.
        let big = Expr::Int(num_traits::pow::pow(BigInt::from(10), 15));
        assert!(matches!(pow(int(2), big), Expr::Pow(..)));
    }

    #[test]
    fn sqrt_of_negative_is_imaginary() {
        assert_eq!(pow(int(-4), half()), complex(int(0), int(2)));
    }

    #[test]
    fn float_to_rational_is_lossless() {
        let mut cc = Consts::new().unwrap();
        // 0.375 = 3/8 is exactly representable in binary.
        let f = BigFloat::parse("0.375", RADIX, 128, ROUND, &mut cc);
        assert_eq!(
            float_to_rational(&f).unwrap(),
            BigRational::new(BigInt::from(3), BigInt::from(8))
        );
        let g = BigFloat::from_i64(-12345, 128);
        assert_eq!(
            float_to_rational(&g).unwrap(),
            BigRational::from_integer(BigInt::from(-12345))
        );
        assert_eq!(
            float_to_rational(&BigFloat::from_i64(0, 64)).unwrap(),
            BigRational::zero()
        );
        assert!(float_to_rational(&astro_float::NAN).is_none());
    }

    #[test]
    fn format_bigfloat_rounds_on_the_exact_value() {
        let mut cc = Consts::new().unwrap();
        // Carry propagates through all displayed digits: 99.96 -> 100.
        let f = BigFloat::parse("99.96", RADIX, 128, ROUND, &mut cc);
        assert_eq!(format_bigfloat(&f, 3), "100");
        // Half-up at the cut digit, leading zeros preserved.
        let f = BigFloat::parse("0.0001234567", RADIX, 128, ROUND, &mut cc);
        assert_eq!(format_bigfloat(&f, 4), "0.0001235");
        // Non-values don't reach the rational path.
        assert_eq!(format_bigfloat(&astro_float::NAN, 5), "NaN");
        assert_eq!(format_bigfloat(&astro_float::INF_POS, 5), "Inf");
        assert_eq!(format_bigfloat(&astro_float::INF_NEG, 5), "-Inf");
    }
}
