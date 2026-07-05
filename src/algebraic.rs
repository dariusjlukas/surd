//! Real algebraic numbers: exact sign, order, and equality for roots of
//! integer polynomials.
//!
//! This is the machinery behind deciding what certified interval refinement
//! cannot: whether two constants are *equal*. An enclosure can separate
//! `sqrt(2)+sqrt(3)` from `π`, but no finite precision separates
//! `(√2+√3)²` from `5+2√6` — they are the same number, and proving that
//! needs algebra, not bits. Here every value is represented as
//!
//!   * an exact rational, or
//!   * `Root { p, lo, hi }`: the unique root of a squarefree, primitive
//!     integer polynomial `p` (positive leading coefficient) inside the open
//!     interval `(lo, hi)`, with the invariant `sign p(lo) ≠ sign p(hi)`,
//!     both nonzero. Squarefree ⇒ every real root is simple ⇒ `p` changes
//!     sign at the root, so refinement is plain bisection on signs.
//!
//! Design decisions, and why:
//!
//! * **No minimal polynomials.** Minimality needs factorization over ℚ
//!   (Zassenhaus/LLL). Equality is decidable without it: two represented
//!   numbers are equal iff `gcd(p_a, p_b)` has a root in the intersection of
//!   their isolating intervals (a root there is a root of both defining
//!   polynomials inside both isolating intervals, hence — by uniqueness —
//!   equal to both numbers). Squarefree + gcd is all the algebra required.
//! * **Arithmetic via resultants, computed by evaluation/interpolation.**
//!   `a+b` is a root of `Res_y(p_a(y), p_b(x−y))`, `a·b` of
//!   `Res_y(p_a(y), y^n·p_b(x/y))`, `a^(1/q)` of `Res_x(p_a(x), y^q − x^p)`.
//!   Rather than a fraction-free determinant over ℤ[x], each resultant is
//!   evaluated at enough integer points (a scalar resultant over ℚ per
//!   point, by the Euclidean recurrence) and interpolated — far easier to
//!   get right, exactly as accurate, and fast enough under [`MAX_DEG`].
//!   The right root of the resultant is then re-isolated from the interval
//!   arithmetic of the operands, refining them until exactly one candidate
//!   root survives.
//! * **Sturm chains only where sign-change bisection is not enough**: root
//!   counting during isolation and the common-root test. The chain uses
//!   exact rational remainders scaled back to primitive integer polynomials
//!   by *positive* factors only — scaling by a negative would silently break
//!   the sign-variation count.
//! * **Degree cap, not a proof budget.** Every constructor refuses (returns
//!   `None`) past [`MAX_DEG`] or [`MAX_BITS_COEFF`]; callers fall back to
//!   the honest "may be equal" refusal. A refused simplification is fine; a
//!   slow or wrong one is not.
//!
//! π and e are transcendental: nothing here (correctly) represents them, so
//! `exp(1) == e`-class ties keep refusing exactly as before.

use crate::expr::{BigRational, Constant, Expr};
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, Zero};

/// Cap on the degree of any defining polynomial (inputs and resultant
/// outputs). 64 covers nested radicals four deep and cos(π/n) to n = 64
/// while keeping the exact arithmetic interactive.
pub const MAX_DEG: usize = 64;

/// Cap on coefficient size (bits) — combined with MAX_DEG this bounds every
/// Sturm/resultant computation.
pub const MAX_BITS_COEFF: u64 = 4096;

// ---------------------------------------------------------------------------
// Dense integer polynomials: index = degree, invariant: no trailing zeros
// (empty vec = the zero polynomial).
// ---------------------------------------------------------------------------

type Poly = Vec<BigInt>;

fn trim(mut p: Poly) -> Poly {
    while p.last().is_some_and(|c| c.is_zero()) {
        p.pop();
    }
    p
}

fn deg(p: &Poly) -> usize {
    p.len().saturating_sub(1)
}

fn poly_neg(p: &Poly) -> Poly {
    p.iter().map(|c| -c).collect()
}

fn poly_add(a: &Poly, b: &Poly) -> Poly {
    let mut out = vec![BigInt::zero(); a.len().max(b.len())];
    for (i, c) in a.iter().enumerate() {
        out[i] += c;
    }
    for (i, c) in b.iter().enumerate() {
        out[i] += c;
    }
    trim(out)
}

fn poly_mul(a: &Poly, b: &Poly) -> Poly {
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let mut out = vec![BigInt::zero(); a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    trim(out)
}

fn derivative(p: &Poly) -> Poly {
    if p.len() <= 1 {
        return vec![];
    }
    p.iter()
        .enumerate()
        .skip(1)
        .map(|(i, c)| c * BigInt::from(i))
        .collect()
}

/// Divide by the content and force a positive leading coefficient (the
/// canonical form for defining polynomials — root sets are unchanged).
fn primitive(p: &Poly) -> Poly {
    if p.is_empty() {
        return vec![];
    }
    let mut g = BigInt::zero();
    for c in p {
        g = num_bigint::BigInt::from(num_integer::gcd(g.clone(), c.clone()));
        if g.is_one() {
            break;
        }
    }
    let mut out: Poly = if g.is_zero() || g.is_one() {
        p.clone()
    } else {
        p.iter().map(|c| c / &g).collect()
    };
    if out.last().is_some_and(|c| c.is_negative()) {
        out = poly_neg(&out);
    }
    out
}

/// Divide by the positive content only — SIGNS PRESERVED. This, not
/// `primitive`, is the reduction the Sturm chain must use: `primitive`
/// flips the sign of a negative-leading polynomial, which silently breaks
/// the variation count (found the hard way by the chain's own unit test).
fn content_reduced(p: &Poly) -> Poly {
    if p.is_empty() {
        return vec![];
    }
    let mut g = BigInt::zero();
    for c in p {
        g = num_bigint::BigInt::from(num_integer::gcd(g.clone(), c.clone()));
        if g.is_one() {
            break;
        }
    }
    if g.is_zero() || g.is_one() {
        p.clone()
    } else {
        p.iter().map(|c| c / &g).collect()
    }
}

/// The sign of p(a/b) for b > 0, in exact integer arithmetic:
/// sign(Σ cᵢ·aⁱ·b^(n−i)) — no rationals materialized.
fn sign_at(p: &Poly, x: &BigRational) -> Sign {
    if p.is_empty() {
        return Sign::NoSign;
    }
    let (a, b) = (x.numer(), x.denom()); // denom > 0 by num-rational invariant
    let mut acc = BigInt::zero();
    // Horner in the homogenized form: acc = acc·a + cᵢ·b^(n−i) built from
    // the top coefficient down, multiplying a power of b in at each step.
    let mut bpow = BigInt::one();
    let mut terms: Vec<BigInt> = Vec::with_capacity(p.len());
    for c in p.iter().rev() {
        terms.push(c * &bpow);
        bpow *= b;
    }
    for t in terms {
        acc = acc * a + t;
    }
    acc.sign()
}

/// Exact polynomial remainder over ℚ, returned as a primitive integer
/// polynomial scaled by a POSITIVE rational only — sign structure preserved,
/// which is what the Sturm chain's variation count depends on.
fn rem_primitive(a: &Poly, b: &Poly) -> Poly {
    let mut r: Vec<BigRational> = a
        .iter()
        .map(|c| BigRational::from_integer(c.clone()))
        .collect();
    let bq: Vec<BigRational> = b
        .iter()
        .map(|c| BigRational::from_integer(c.clone()))
        .collect();
    let db = bq.len() - 1;
    let lead = bq.last().expect("nonzero divisor").clone();
    while r
        .iter()
        .rev()
        .position(|c| !c.is_zero())
        .map(|i| r.len() - 1 - i)
        >= Some(db)
        && r.iter().any(|c| !c.is_zero())
    {
        while r.last().is_some_and(|c| c.is_zero()) {
            r.pop();
        }
        if r.len() <= db {
            break;
        }
        let shift = r.len() - 1 - db;
        let f = r.last().unwrap() / &lead;
        for (i, c) in bq.iter().enumerate() {
            let v = &f * c;
            r[i + shift] -= v;
        }
        r.pop(); // leading term cancels exactly
    }
    while r.last().is_some_and(|c| c.is_zero()) {
        r.pop();
    }
    // Clear denominators with a positive multiplier, then primitive part.
    let mut den = BigInt::one();
    for c in &r {
        den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), c.denom().clone()));
    }
    let ints: Poly = r.iter().map(|c| (c * &den).to_integer()).collect();
    content_reduced(&trim(ints))
}

/// Polynomial gcd over ℚ (primitive representative), by Euclid with
/// primitive-part reduction.
fn poly_gcd(a: &Poly, b: &Poly) -> Poly {
    let (mut x, mut y) = (primitive(a), primitive(b));
    while !y.is_empty() {
        let r = rem_primitive(&x, &y);
        x = y;
        y = r;
    }
    primitive(&x)
}

/// Exact division p / d over ℚ when it divides evenly (used for the
/// squarefree part, where divisibility is a theorem). Returns the primitive
/// integer quotient.
fn div_exact(p: &Poly, d: &Poly) -> Poly {
    let mut r: Vec<BigRational> = p
        .iter()
        .map(|c| BigRational::from_integer(c.clone()))
        .collect();
    let dq: Vec<BigRational> = d
        .iter()
        .map(|c| BigRational::from_integer(c.clone()))
        .collect();
    let dd = dq.len() - 1;
    let lead = dq.last().expect("nonzero divisor").clone();
    let mut q = vec![BigRational::from_integer(BigInt::zero()); p.len() - d.len() + 1];
    while r.len() > dd {
        let shift = r.len() - 1 - dd;
        let f = r.last().unwrap() / &lead;
        q[shift] = f.clone();
        for (i, c) in dq.iter().enumerate() {
            let v = &f * c;
            r[i + shift] -= v;
        }
        r.pop();
        while r.last().is_some_and(|c| c.is_zero()) {
            r.pop();
        }
        if r.is_empty() {
            break;
        }
    }
    debug_assert!(r.is_empty(), "div_exact called on a non-divisor");
    let mut den = BigInt::one();
    for c in &q {
        den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), c.denom().clone()));
    }
    let ints: Poly = q.iter().map(|c| (c * &den).to_integer()).collect();
    primitive(&ints)
}

/// Squarefree part: p / gcd(p, p′). Same real roots, all simple.
fn squarefree(p: &Poly) -> Poly {
    let p = primitive(p);
    if deg(&p) <= 1 {
        return p;
    }
    let g = poly_gcd(&p, &derivative(&p));
    if deg(&g) == 0 {
        p
    } else {
        div_exact(&p, &g)
    }
}

fn coeff_bits(p: &Poly) -> u64 {
    p.iter().map(|c| c.bits()).max().unwrap_or(0)
}

fn within_caps(p: &Poly) -> bool {
    deg(p) <= MAX_DEG && coeff_bits(p) <= MAX_BITS_COEFF
}

// ---------------------------------------------------------------------------
// Sturm chains: exact root counting.
// ---------------------------------------------------------------------------

/// Sturm chain of a squarefree polynomial: p, p′, then negated remainders,
/// each scaled to primitive by positive factors.
fn sturm_chain(p: &Poly) -> Vec<Poly> {
    let p0 = primitive(p);
    let p1 = content_reduced(&derivative(&p0));
    let mut chain = vec![p0, p1];
    loop {
        let n = chain.len();
        if chain[n - 1].is_empty() {
            chain.pop();
            break;
        }
        if deg(&chain[n - 1]) == 0 {
            break;
        }
        let r = rem_primitive(&chain[n - 2], &chain[n - 1]);
        if r.is_empty() {
            break;
        }
        chain.push(poly_neg(&r));
    }
    chain
}

/// Sign variations of the chain at x (zeros skipped).
fn variations(chain: &[Poly], x: &BigRational) -> usize {
    let mut count = 0;
    let mut last = Sign::NoSign;
    for p in chain {
        let s = sign_at(p, x);
        if s == Sign::NoSign {
            continue;
        }
        if last != Sign::NoSign && s != last {
            count += 1;
        }
        last = s;
    }
    count
}

/// Number of distinct real roots of the (squarefree) chain's polynomial in
/// the open interval (lo, hi). Requires p(lo) ≠ 0 and p(hi) ≠ 0, which makes
/// (lo, hi) and (lo, hi] coincide.
fn count_roots(chain: &[Poly], lo: &BigRational, hi: &BigRational) -> usize {
    variations(chain, lo).saturating_sub(variations(chain, hi))
}

/// Cauchy root bound: every real root lies in (−B, B), B = 1 + max|cᵢ|/|cₙ|.
fn root_bound(p: &Poly) -> BigRational {
    let lead = p.last().expect("nonzero poly").magnitude().clone();
    let max = p
        .iter()
        .take(p.len() - 1)
        .map(|c| c.magnitude().clone())
        .max()
        .unwrap_or_default();
    BigRational::from_integer(BigInt::one())
        + BigRational::new(BigInt::from(max), BigInt::from(lead))
}

/// Isolate all real roots of a squarefree p: disjoint open intervals with a
/// sign change and exactly one root each, ascending. Rational roots hit by a
/// bisection midpoint come back as exact points.
fn isolate_roots(p: &Poly) -> Vec<RealAlg> {
    if deg(p) == 0 {
        return vec![];
    }
    let chain = sturm_chain(p);
    let b = root_bound(p);
    let mut out = Vec::new();
    // Endpoints ±B are safely beyond every root, so p(±B) ≠ 0.
    let mut stack = vec![(-b.clone(), b.clone())];
    while let Some((lo, hi)) = stack.pop() {
        let n = count_roots(&chain, &lo, &hi);
        if n == 0 {
            continue;
        }
        if n == 1 {
            out.push(make_root(p, lo, hi));
            continue;
        }
        let mid = (&lo + &hi) / BigRational::from_integer(BigInt::from(2));
        if sign_at(p, &mid) == Sign::NoSign {
            // The midpoint IS a (rational) root; recurse on both sides of a
            // small punctured neighborhood that provably contains only it.
            out.push(RealAlg::Rational(mid.clone()));
            let eps = smallest_gap_escape(p, &lo, &hi);
            let (m_lo, m_hi) = (&mid - &eps, &mid + &eps);
            debug_assert!(sign_at(p, &m_lo) != Sign::NoSign);
            debug_assert!(sign_at(p, &m_hi) != Sign::NoSign);
            if count_roots(&chain, &lo, &m_lo) > 0 {
                stack.push((lo, m_lo));
            }
            if count_roots(&chain, &m_hi, &hi) > 0 {
                stack.push((m_hi, hi));
            }
        } else {
            stack.push((lo, mid.clone()));
            stack.push((mid, hi));
        }
    }
    out.sort_by(|a, b| a.cmp_alg(b));
    out
}

/// A positive rational ε below half the minimum distance between distinct
/// roots in (lo, hi) — found by shrinking until the Sturm counts confirm the
/// puncture isolates the midpoint root. Cheap and always terminates because
/// root gaps are positive.
fn smallest_gap_escape(p: &Poly, lo: &BigRational, hi: &BigRational) -> BigRational {
    let chain = sturm_chain(p);
    let mut eps = (hi - lo) / BigRational::from_integer(BigInt::from(4));
    let mid = (lo + hi) / BigRational::from_integer(BigInt::from(2));
    loop {
        let (a, b) = (&mid - &eps, &mid + &eps);
        if sign_at(p, &a) != Sign::NoSign
            && sign_at(p, &b) != Sign::NoSign
            && count_roots(&chain, &a, &b) == 1
        {
            return eps;
        }
        eps /= BigRational::from_integer(BigInt::from(2));
    }
}

/// Wrap an isolated single-root interval as a RealAlg, tightening to the
/// sign-change invariant (bisect once if an endpoint sign matches — with one
/// simple root inside, signs at the ends must differ once endpoints are
/// root-free, which isolate_roots guarantees).
fn make_root(p: &Poly, lo: BigRational, hi: BigRational) -> RealAlg {
    debug_assert!(sign_at(p, &lo) != Sign::NoSign && sign_at(p, &hi) != Sign::NoSign);
    debug_assert!(
        sign_at(p, &lo) != sign_at(p, &hi),
        "single simple root ⇒ sign change"
    );
    // A linear polynomial has the exact rational root −c₀/c₁.
    if deg(p) == 1 {
        return RealAlg::Rational(BigRational::new(-p[0].clone(), p[1].clone()));
    }
    RealAlg::Root {
        p: primitive(p),
        lo,
        hi,
    }
}

// ---------------------------------------------------------------------------
// The number type.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum RealAlg {
    Rational(BigRational),
    Root {
        /// Squarefree, primitive, positive leading coefficient.
        p: Poly,
        /// Open isolating interval: exactly one root of `p` inside,
        /// `sign p(lo) ≠ sign p(hi)`, both nonzero.
        lo: BigRational,
        hi: BigRational,
    },
}

impl RealAlg {
    pub fn from_rational(r: BigRational) -> Self {
        RealAlg::Rational(r)
    }

    /// The k-th real root (1-based, ascending) of the polynomial with the
    /// given rational coefficients (index 0 = constant term). `None` if the
    /// polynomial is zero/constant, over the caps, or k is out of range.
    pub fn nth_root_of(coeffs: &[BigRational], k: usize) -> Option<RealAlg> {
        let mut den = BigInt::one();
        for c in coeffs {
            den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), c.denom().clone()));
        }
        let ints: Poly = trim(coeffs.iter().map(|c| (c * &den).to_integer()).collect());
        if deg(&ints) == 0 || ints.is_empty() || !within_caps(&ints) {
            return None;
        }
        let sf = squarefree(&ints);
        let roots = isolate_roots(&sf);
        roots.into_iter().nth(k.checked_sub(1)?)
    }

    /// How many distinct real roots the polynomial has (for error messages).
    pub fn real_root_count(coeffs: &[BigRational]) -> Option<usize> {
        let mut den = BigInt::one();
        for c in coeffs {
            den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), c.denom().clone()));
        }
        let ints: Poly = trim(coeffs.iter().map(|c| (c * &den).to_integer()).collect());
        if deg(&ints) == 0 || ints.is_empty() || !within_caps(&ints) {
            return None;
        }
        Some(isolate_roots(&squarefree(&ints)).len())
    }

    /// Halve the isolating interval once (no-op for rationals).
    fn refine(&mut self) {
        if let RealAlg::Root { p, lo, hi } = self {
            let mid = (&*lo + &*hi) / BigRational::from_integer(BigInt::from(2));
            match sign_at(p, &mid) {
                Sign::NoSign => {
                    // The midpoint is the root: collapse to an exact rational.
                    *self = RealAlg::Rational(mid);
                }
                s => {
                    if s == sign_at(p, lo) {
                        *lo = mid;
                    } else {
                        *hi = mid;
                    }
                }
            }
        }
    }

    /// Refine until the interval width is at most `target` (> 0).
    pub fn refine_to(&mut self, target: &BigRational) {
        while let RealAlg::Root { lo, hi, .. } = self {
            if &(&*hi - &*lo) <= target {
                break;
            }
            self.refine();
        }
    }

    /// Current rational bounds (point interval for rationals).
    pub fn bounds(&self) -> (BigRational, BigRational) {
        match self {
            RealAlg::Rational(r) => (r.clone(), r.clone()),
            RealAlg::Root { lo, hi, .. } => (lo.clone(), hi.clone()),
        }
    }

    /// Exact sign.
    pub fn sign(&self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match self {
            RealAlg::Rational(r) => r.cmp(&BigRational::from_integer(BigInt::zero())),
            RealAlg::Root { .. } => {
                match self.cmp_rational(&BigRational::from_integer(BigInt::zero())) {
                    Less => Less,
                    Greater => Greater,
                    Equal => Equal,
                }
            }
        }
    }

    /// Exact comparison against a rational — sign tests only, no Sturm.
    fn cmp_rational(&self, r: &BigRational) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match self {
            RealAlg::Rational(v) => v.cmp(r),
            RealAlg::Root { p, lo, hi } => {
                if r <= lo {
                    return Greater;
                }
                if r >= hi {
                    return Less;
                }
                match sign_at(p, r) {
                    Sign::NoSign => Equal, // r is a root inside ⇒ THE root
                    s => {
                        if s == sign_at(p, lo) {
                            Greater // sign unchanged from lo ⇒ root in (r, hi)
                        } else {
                            Less
                        }
                    }
                }
            }
        }
    }

    /// Exact comparison. Equality is the gcd common-root test; inequality
    /// falls out of interval refinement (which terminates because the values
    /// are then genuinely distinct).
    pub fn cmp_alg(&self, other: &RealAlg) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match (self, other) {
            (RealAlg::Rational(a), RealAlg::Rational(b)) => a.cmp(b),
            (a, RealAlg::Rational(r)) => a.cmp_rational(r),
            (RealAlg::Rational(r), b) => b.cmp_rational(r).reverse(),
            (RealAlg::Root { p: pa, .. }, RealAlg::Root { p: pb, .. }) => {
                let (mut a, mut b) = (self.clone(), other.clone());
                // Fast path + equality certificate.
                let g = poly_gcd(pa, pb);
                let g_chain = (deg(&g) >= 1).then(|| sturm_chain(&g));
                loop {
                    let (alo, ahi) = a.bounds();
                    let (blo, bhi) = b.bounds();
                    if ahi <= blo {
                        // Disjoint (touching endpoints are open-interval safe:
                        // neither value equals its own endpoint).
                        return Less;
                    }
                    if bhi <= alo {
                        return Greater;
                    }
                    // Overlapping. Equal iff gcd has a root in the overlap.
                    if let Some(chain) = &g_chain {
                        let ilo = alo.clone().max(blo.clone());
                        let ihi = ahi.clone().min(bhi.clone());
                        // Endpoints of the overlap are endpoints of isolating
                        // intervals, where the defining polys are nonzero —
                        // but g(endpoint) = 0 is still possible for other
                        // factors of g. Perturbation: refine and retry; a
                        // fixed root can only coincide with the moving
                        // endpoints finitely often.
                        if sign_at(&g, &ilo) != Sign::NoSign && sign_at(&g, &ihi) != Sign::NoSign {
                            if count_roots(chain, &ilo, &ihi) >= 1 {
                                return Equal;
                            }
                            // No common root in the overlap: they differ.
                            // Keep refining until the intervals separate.
                        }
                    }
                    a.refine();
                    b.refine();
                    if let (RealAlg::Rational(_), _) | (_, RealAlg::Rational(_)) = (&a, &b) {
                        // A refinement hit an exact rational: restart with
                        // the simpler shape.
                        return a.cmp_alg(&b);
                    }
                }
            }
        }
    }

    /// An f64 approximation (for display/plot convenience).
    pub fn approx_f64(&self) -> f64 {
        let mut me = self.clone();
        me.refine_to(&BigRational::new(BigInt::one(), BigInt::from(10u8).pow(20)));
        let (lo, hi) = me.bounds();
        let mid = (lo + hi) / BigRational::from_integer(BigInt::from(2));
        num_traits::ToPrimitive::to_f64(&mid).unwrap_or(f64::NAN)
    }
}

// ---------------------------------------------------------------------------
// Resultant arithmetic (evaluation / interpolation).
// ---------------------------------------------------------------------------

/// Scalar resultant of two rational polynomials by the Euclidean recurrence:
/// res(f, g) = (−1)^(df·dg) · lc(g)^(df − dr) · res(g, r), res(f, c) = c^df.
fn scalar_resultant(f: &[BigRational], g: &[BigRational]) -> BigRational {
    fn go(f: &mut Vec<BigRational>, g: &mut Vec<BigRational>) -> BigRational {
        let trim_q = |v: &mut Vec<BigRational>| {
            while v.last().is_some_and(|c| c.is_zero()) {
                v.pop();
            }
        };
        trim_q(f);
        trim_q(g);
        if f.is_empty() || g.is_empty() {
            return BigRational::from_integer(BigInt::zero());
        }
        let (df, dg) = (f.len() - 1, g.len() - 1);
        if dg == 0 {
            return pow_rat(&g[0], df as u32);
        }
        if df < dg {
            let sign = if (df * dg) % 2 == 1 {
                -BigRational::from_integer(BigInt::one())
            } else {
                BigRational::from_integer(BigInt::one())
            };
            return sign * go(g, f);
        }
        // r = f mod g
        let lead = g.last().unwrap().clone();
        let mut r = f.clone();
        while r.len() > dg {
            let shift = r.len() - 1 - dg;
            let c = r.last().unwrap() / &lead;
            for (i, gc) in g.iter().enumerate() {
                let v = &c * gc;
                r[i + shift] -= v;
            }
            r.pop();
            trim_q(&mut r);
            if r.is_empty() {
                break;
            }
        }
        if r.is_empty() {
            return BigRational::from_integer(BigInt::zero());
        }
        let dr = r.len() - 1;
        let sign = if (df * dg) % 2 == 1 {
            -BigRational::from_integer(BigInt::one())
        } else {
            BigRational::from_integer(BigInt::one())
        };
        sign * pow_rat(&lead, (df - dr) as u32) * go(g, &mut r)
    }
    go(&mut f.to_vec(), &mut g.to_vec())
}

fn pow_rat(b: &BigRational, e: u32) -> BigRational {
    let mut acc = BigRational::from_integer(BigInt::one());
    for _ in 0..e {
        acc *= b;
    }
    acc
}

/// Lagrange interpolation through (i, values[i]) for integer nodes 0..n,
/// returned as a primitive integer polynomial (positive scaling only).
fn interpolate(values: &[BigRational]) -> Poly {
    let n = values.len();
    let mut acc = vec![BigRational::from_integer(BigInt::zero()); n];
    for (i, yi) in values.iter().enumerate() {
        if yi.is_zero() {
            continue;
        }
        // Basis polynomial ∏_{j≠i} (x − j) / (i − j), built incrementally.
        let mut basis = vec![BigRational::from_integer(BigInt::one())];
        let mut denom = BigRational::from_integer(BigInt::one());
        for j in 0..n {
            if j == i {
                continue;
            }
            let jq = BigRational::from_integer(BigInt::from(j));
            // basis *= (x − j)
            let mut next = vec![BigRational::from_integer(BigInt::zero()); basis.len() + 1];
            for (k, c) in basis.iter().enumerate() {
                next[k + 1] += c;
                next[k] -= c * &jq;
            }
            basis = next;
            denom *= BigRational::from_integer(BigInt::from(i as i64 - j as i64));
        }
        let w = yi / denom;
        for (k, c) in basis.iter().enumerate() {
            acc[k] += c * &w;
        }
    }
    let mut den = BigInt::one();
    for c in &acc {
        den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), c.denom().clone()));
    }
    primitive(&trim(acc.iter().map(|c| (c * &den).to_integer()).collect()))
}

fn to_q(p: &Poly) -> Vec<BigRational> {
    p.iter()
        .map(|c| BigRational::from_integer(c.clone()))
        .collect()
}

/// p_b(t − y) as a polynomial in y (for the sum resultant), t rational.
fn shifted_reversed(pb: &Poly, t: &BigRational) -> Vec<BigRational> {
    // Horner: result = Σ coeffs of pb evaluated at (t − y).
    let mut acc = vec![BigRational::from_integer(BigInt::zero())];
    for c in pb.iter().rev() {
        // acc = acc·(t − y) + c
        let mut next = vec![BigRational::from_integer(BigInt::zero()); acc.len() + 1];
        for (k, a) in acc.iter().enumerate() {
            next[k] += a * t; // ·t
            next[k + 1] -= a; // ·(−y)
        }
        next[0] += BigRational::from_integer(c.clone());
        acc = next;
    }
    acc
}

/// y^n · p_b(t / y) as a polynomial in y (for the product resultant).
fn homogenized_at(pb: &Poly, t: &BigRational) -> Vec<BigRational> {
    // Σ c_j t^j y^(n−j)
    let n = deg(pb);
    let mut out = vec![BigRational::from_integer(BigInt::zero()); n + 1];
    let mut tp = BigRational::from_integer(BigInt::one());
    for (j, c) in pb.iter().enumerate() {
        out[n - j] = BigRational::from_integer(c.clone()) * &tp;
        tp *= t;
    }
    out
}

enum BinOpKind {
    Add,
    Mul,
}

/// Interval arithmetic on the operands' rational bounds — used to pick the
/// right root of the resultant.
fn op_interval(
    kind: &BinOpKind,
    a: &(BigRational, BigRational),
    b: &(BigRational, BigRational),
) -> (BigRational, BigRational) {
    match kind {
        BinOpKind::Add => (&a.0 + &b.0, &a.1 + &b.1),
        BinOpKind::Mul => {
            let products = [&a.0 * &b.0, &a.0 * &b.1, &a.1 * &b.0, &a.1 * &b.1];
            (
                products.iter().min().unwrap().clone(),
                products.iter().max().unwrap().clone(),
            )
        }
    }
}

fn alg_binop(kind: BinOpKind, a: &RealAlg, b: &RealAlg) -> Option<RealAlg> {
    use RealAlg::*;
    // Rational short-circuits: exact, and they keep degrees down.
    match (a, b) {
        (Rational(x), Rational(y)) => {
            return Some(Rational(match kind {
                BinOpKind::Add => x + y,
                BinOpKind::Mul => x * y,
            }))
        }
        (Rational(x), root @ Root { .. }) | (root @ Root { .. }, Rational(x)) => {
            return rational_op(&kind, root, x)
        }
        _ => {}
    }
    let (Root { p: pa, .. }, Root { p: pb, .. }) = (a, b) else {
        unreachable!()
    };
    if matches!(kind, BinOpKind::Mul) {
        // A zero operand can't be a Root (0 would be its unique root and
        // refinement collapses rationals), but guard anyway.
        if a.sign() == std::cmp::Ordering::Equal || b.sign() == std::cmp::Ordering::Equal {
            return Some(Rational(BigRational::from_integer(BigInt::zero())));
        }
    }
    let (m, n) = (deg(pa), deg(pb));
    let dmax = m.checked_mul(n)?;
    if dmax > MAX_DEG {
        return None;
    }
    // Evaluate R(t) = Res_y(pa(y), q_t(y)) at t = 0..dmax, interpolate.
    let paq = to_q(pa);
    let mut vals = Vec::with_capacity(dmax + 1);
    for t in 0..=dmax {
        let tq = BigRational::from_integer(BigInt::from(t));
        let qt = match kind {
            BinOpKind::Add => shifted_reversed(pb, &tq),
            BinOpKind::Mul => homogenized_at(pb, &tq),
        };
        vals.push(scalar_resultant(&paq, &qt));
    }
    let r = interpolate(&vals);
    if r.is_empty() || deg(&r) == 0 || !within_caps(&r) {
        return None;
    }
    let sf = squarefree(&r);
    select_root(&sf, kind, a, b)
}

/// c + root / c · root: substitution keeps the degree unchanged.
/// (x ↦ x − c gives the sum's polynomial; x ↦ x/c the product's.)
fn rational_op(kind: &BinOpKind, root: &RealAlg, c: &BigRational) -> Option<RealAlg> {
    let RealAlg::Root { p, lo, hi } = root else {
        unreachable!()
    };
    match kind {
        BinOpKind::Add => {
            let shifted = shifted_poly(p, c)?;
            Some(RealAlg::Root {
                p: shifted,
                lo: lo + c,
                hi: hi + c,
            })
        }
        BinOpKind::Mul => {
            if c.is_zero() {
                return Some(RealAlg::Rational(BigRational::from_integer(BigInt::zero())));
            }
            // Roots scale by c when x ↦ x/c: coefficient i picks up c^(n−i).
            let n = deg(p);
            let mut coeffs: Vec<BigRational> =
                vec![BigRational::from_integer(BigInt::zero()); n + 1];
            let mut cp = BigRational::from_integer(BigInt::one());
            for i in (0..=n).rev() {
                coeffs[i] = BigRational::from_integer(p[i].clone()) * &cp;
                cp *= c;
            }
            let mut den = BigInt::one();
            for q in &coeffs {
                den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), q.denom().clone()));
            }
            let ints: Poly = coeffs.iter().map(|q| (q * &den).to_integer()).collect();
            let p2 = primitive(&trim(ints));
            let (a, b) = (lo * c, hi * c);
            let (lo2, hi2) = if c.is_negative() { (b, a) } else { (a, b) };
            debug_assert!(sign_at(&p2, &lo2) != Sign::NoSign);
            Some(RealAlg::Root {
                p: p2,
                lo: lo2,
                hi: hi2,
            })
        }
    }
}

/// p(x − c) as a primitive integer polynomial (roots shifted by +c).
fn shifted_poly(p: &Poly, c: &BigRational) -> Option<Poly> {
    let mut acc = vec![BigRational::from_integer(BigInt::zero())];
    for coef in p.iter().rev() {
        // acc = acc·(x − c) + coef
        let mut next = vec![BigRational::from_integer(BigInt::zero()); acc.len() + 1];
        for (k, a) in acc.iter().enumerate() {
            next[k + 1] += a;
            next[k] -= a * c;
        }
        next[0] += BigRational::from_integer(coef.clone());
        acc = next;
    }
    let mut den = BigInt::one();
    for q in &acc {
        den = num_bigint::BigInt::from(num_integer::lcm(den.clone(), q.denom().clone()));
    }
    Some(primitive(&trim(
        acc.iter().map(|q| (q * &den).to_integer()).collect(),
    )))
}

/// Pick the root of `sf` that is the true value of `a ∘ b`: refine the
/// operands until their op-interval contains exactly one root of `sf` with
/// root-free endpoints.
fn select_root(sf: &Poly, kind: BinOpKind, a: &RealAlg, b: &RealAlg) -> Option<RealAlg> {
    let chain = sturm_chain(sf);
    let (mut a, mut b) = (a.clone(), b.clone());
    for _round in 0..4096 {
        let (mut jlo, mut jhi) = op_interval(&kind, &a.bounds(), &b.bounds());
        // Nudge endpoints off roots of sf (the true value is strictly
        // interior once the operands are refined, since it differs from
        // every OTHER root and the interval shrinks to it; an endpoint
        // *equal* to the true value only happens when both operands are
        // rational — handled before we get here).
        let width = &jhi - &jlo;
        if width.is_zero() {
            // Point interval: the value is this exact rational.
            return Some(RealAlg::Rational(jlo));
        }
        let nudge = &width / BigRational::from_integer(BigInt::from(1024));
        if sign_at(sf, &jlo) == Sign::NoSign {
            jlo -= &nudge;
        }
        if sign_at(sf, &jhi) == Sign::NoSign {
            jhi += &nudge;
        }
        if sign_at(sf, &jlo) != Sign::NoSign && sign_at(sf, &jhi) != Sign::NoSign {
            match count_roots(&chain, &jlo, &jhi) {
                1 => {
                    // One candidate — but it may be rational: bisect down via
                    // the standard Root representation only if signs differ.
                    if sign_at(sf, &jlo) != sign_at(sf, &jhi) {
                        return Some(RealAlg::Root {
                            p: sf.clone(),
                            lo: jlo,
                            hi: jhi,
                        });
                    }
                    // Same sign at both ends with one root inside would
                    // contradict simple roots — can only be a nudge artifact;
                    // fall through to refine.
                }
                0 => return None, // cannot happen for a correct resultant
                _ => {}
            }
        }
        a.refine();
        b.refine();
        if let (RealAlg::Rational(_), RealAlg::Rational(_)) = (&a, &b) {
            let (jlo, _) = op_interval(&kind, &a.bounds(), &b.bounds());
            return Some(RealAlg::Rational(jlo));
        }
    }
    None // refinement budget exhausted (astronomically unlikely)
}

impl RealAlg {
    pub fn add_alg(&self, other: &RealAlg) -> Option<RealAlg> {
        alg_binop(BinOpKind::Add, self, other)
    }

    pub fn mul_alg(&self, other: &RealAlg) -> Option<RealAlg> {
        alg_binop(BinOpKind::Mul, self, other)
    }

    pub fn neg_alg(&self) -> RealAlg {
        match self {
            RealAlg::Rational(r) => RealAlg::Rational(-r),
            RealAlg::Root { p, lo, hi } => {
                // p(−x), sign-normalized by primitive().
                let flipped: Poly = p
                    .iter()
                    .enumerate()
                    .map(|(i, c)| if i % 2 == 1 { -c } else { c.clone() })
                    .collect();
                RealAlg::Root {
                    p: primitive(&flipped),
                    lo: -hi,
                    hi: -lo,
                }
            }
        }
    }

    pub fn recip_alg(&self) -> Option<RealAlg> {
        match self {
            RealAlg::Rational(r) => {
                if r.is_zero() {
                    None
                } else {
                    Some(RealAlg::Rational(r.recip()))
                }
            }
            RealAlg::Root { p, .. } => {
                if self.sign() == std::cmp::Ordering::Equal {
                    return None;
                }
                let mut me = self.clone();
                // Refine until the interval excludes 0, so 1/x is monotone
                // on it.
                loop {
                    let (lo, hi) = me.bounds();
                    if lo.is_negative() != hi.is_negative() || lo.is_zero() || hi.is_zero() {
                        me.refine();
                        if let RealAlg::Rational(r) = &me {
                            return if r.is_zero() {
                                None
                            } else {
                                Some(RealAlg::Rational(r.recip()))
                            };
                        }
                        continue;
                    }
                    break;
                }
                let (lo, hi) = me.bounds();
                // Reversed coefficients: roots become reciprocals. Constant
                // term of p is nonzero (0 is not the root — just proven).
                let mut rev: Poly = p.iter().rev().cloned().collect();
                rev = primitive(&trim(rev.drain(..).collect()));
                let sf = squarefree(&rev);
                let (rlo, rhi) = (hi.recip(), lo.recip());
                // The reciprocal interval isolates 1/x among the reciprocals
                // of p's roots, but sf may have endpoint-root collisions:
                // reuse the generic selector against a rational "identity"
                // operand to stay in one code path.
                let chain = sturm_chain(&sf);
                let (mut rlo, mut rhi) = (rlo, rhi);
                let mut guard = self.clone();
                for _ in 0..4096 {
                    if sign_at(&sf, &rlo) != Sign::NoSign
                        && sign_at(&sf, &rhi) != Sign::NoSign
                        && count_roots(&chain, &rlo, &rhi) == 1
                        && sign_at(&sf, &rlo) != sign_at(&sf, &rhi)
                    {
                        return Some(RealAlg::Root {
                            p: sf,
                            lo: rlo,
                            hi: rhi,
                        });
                    }
                    guard.refine();
                    if let RealAlg::Rational(r) = &guard {
                        return Some(RealAlg::Rational(r.recip()));
                    }
                    let (glo, ghi) = guard.bounds();
                    rlo = ghi.recip();
                    rhi = glo.recip();
                }
                None
            }
        }
    }

    /// Integer power. NOT repeated multiplication: pairwise products pile
    /// the degree up multiplicatively (deg⁴ for a cube), while the single
    /// resultant Res_x(p(x), y − xⁿ) keeps the result's degree at deg p —
    /// each root α contributes exactly one factor (y − αⁿ).
    pub fn powi_alg(&self, n: i64) -> Option<RealAlg> {
        if n == 0 {
            return Some(RealAlg::Rational(BigRational::from_integer(BigInt::one())));
        }
        let base = if n < 0 {
            self.recip_alg()?
        } else {
            self.clone()
        };
        let n = n.unsigned_abs();
        if n == 1 {
            return Some(base);
        }
        if let RealAlg::Rational(r) = &base {
            let n32 = u32::try_from(n).ok()?;
            return Some(RealAlg::Rational(pow_rat(r, n32)));
        }
        if n > MAX_DEG as u64 {
            return None;
        }
        let n = n as usize;
        // Make the operand sign-definite first so the power interval is
        // monotone (an exactly-zero value is rational and handled above via
        // sign(); a Root representing 0 collapses in cmp_rational).
        let mut me = base.clone();
        if me.sign() == std::cmp::Ordering::Equal {
            return Some(RealAlg::Rational(BigRational::from_integer(BigInt::zero())));
        }
        loop {
            let (lo, hi) = me.bounds();
            if lo.is_positive() == hi.is_positive() && !lo.is_zero() && !hi.is_zero() {
                break;
            }
            me.refine();
            if let RealAlg::Rational(r) = &me {
                let n32 = u32::try_from(n as u64).ok()?;
                return Some(RealAlg::Rational(pow_rat(r, n32)));
            }
        }
        let RealAlg::Root { p, .. } = &me else {
            unreachable!()
        };
        // R(y) = Res_x(p(x), y − xⁿ), degree ≤ deg p in y.
        let m = deg(p);
        let paq = to_q(p);
        let mut vals = Vec::with_capacity(m + 1);
        for t in 0..=m {
            // g(x) = t − xⁿ
            let mut g = vec![BigRational::from_integer(BigInt::zero()); n + 1];
            g[0] = BigRational::from_integer(BigInt::from(t));
            g[n] = -BigRational::from_integer(BigInt::one());
            vals.push(scalar_resultant(&paq, &g));
        }
        let r = interpolate(&vals);
        if r.is_empty() || deg(&r) == 0 || !within_caps(&r) {
            return None;
        }
        let sf = squarefree(&r);
        let chain = sturm_chain(&sf);
        let n32 = u32::try_from(n as u64).ok()?;
        for _ in 0..4096 {
            let (lo, hi) = me.bounds();
            // Monotone power interval on a sign-definite operand interval.
            let (a, b) = (pow_rat(&lo, n32), pow_rat(&hi, n32));
            let (mut jlo, mut jhi) = if a <= b { (a, b) } else { (b, a) };
            let width = &jhi - &jlo;
            if !width.is_zero() {
                let nudge = &width / BigRational::from_integer(BigInt::from(1024));
                if sign_at(&sf, &jlo) == Sign::NoSign {
                    jlo -= &nudge;
                }
                if sign_at(&sf, &jhi) == Sign::NoSign {
                    jhi += &nudge;
                }
                if sign_at(&sf, &jlo) != Sign::NoSign
                    && sign_at(&sf, &jhi) != Sign::NoSign
                    && count_roots(&chain, &jlo, &jhi) == 1
                    && sign_at(&sf, &jlo) != sign_at(&sf, &jhi)
                {
                    return Some(RealAlg::Root {
                        p: sf,
                        lo: jlo,
                        hi: jhi,
                    });
                }
            }
            me.refine();
            if let RealAlg::Rational(r) = &me {
                return Some(RealAlg::Rational(pow_rat(r, n32)));
            }
        }
        None
    }

    /// The real q-th root of this value raised to p (i.e. x^(p/q)) when it
    /// exists: x ≥ 0, or x < 0 with odd q.
    pub fn rational_pow(&self, pnum: i64, q: u32) -> Option<RealAlg> {
        if q == 0 {
            return None;
        }
        if q == 1 {
            return self.powi_alg(pnum);
        }
        let xp = self.powi_alg(pnum.abs())?;
        let sign = xp.sign();
        if sign == std::cmp::Ordering::Less && q % 2 == 0 {
            return None; // even root of a negative: not real
        }
        let root = xp.qth_root(q)?;
        if pnum < 0 {
            root.recip_alg()
        } else {
            Some(root)
        }
    }

    /// The real q-th root with the same sign as self (unique for x ≥ 0, and
    /// for x < 0 with odd q).
    fn qth_root(&self, q: u32) -> Option<RealAlg> {
        if let RealAlg::Rational(r) = self {
            if r.is_zero() {
                return Some(RealAlg::Rational(BigRational::from_integer(BigInt::zero())));
            }
        }
        // y is a root of Res_x(p(x), y^q − x): evaluate at integer y-points
        // and interpolate. For rationals, p(x) = den·x − num.
        let (pa, _) = self.defining_poly();
        let m = deg(&pa);
        let dmax = m.checked_mul(q as usize)?;
        if dmax > MAX_DEG {
            return None;
        }
        let paq = to_q(&pa);
        let mut vals = Vec::with_capacity(dmax + 1);
        for t in 0..=dmax {
            // Res_x(p(x), t^q − x) = ± p(t^q) up to lc(p)-powers; but going
            // through the generic scalar resultant keeps one code path.
            let tq = pow_rat(&BigRational::from_integer(BigInt::from(t)), q);
            // poly in x: (t^q) − x  →  coeffs [t^q, −1]
            let g = vec![tq, -BigRational::from_integer(BigInt::one())];
            vals.push(scalar_resultant(&paq, &g));
        }
        let r = interpolate(&vals);
        if r.is_empty() || deg(&r) == 0 || !within_caps(&r) {
            return None;
        }
        let sf = squarefree(&r);
        // Select by refining our own bounds and taking rational q-th root
        // enclosures of them.
        let chain = sturm_chain(&sf);
        let mut me = self.clone();
        for _ in 0..4096 {
            let (lo, hi) = me.bounds();
            let (rlo, rhi) = (rational_root_lower(&lo, q), rational_root_upper(&hi, q));
            if let (Some(mut rlo), Some(mut rhi)) = (rlo, rhi) {
                let width = &rhi - &rlo;
                if !width.is_zero() {
                    let nudge = &width / BigRational::from_integer(BigInt::from(1024));
                    if sign_at(&sf, &rlo) == Sign::NoSign {
                        rlo -= &nudge;
                    }
                    if sign_at(&sf, &rhi) == Sign::NoSign {
                        rhi += &nudge;
                    }
                    if sign_at(&sf, &rlo) != Sign::NoSign
                        && sign_at(&sf, &rhi) != Sign::NoSign
                        && count_roots(&chain, &rlo, &rhi) == 1
                        && sign_at(&sf, &rlo) != sign_at(&sf, &rhi)
                    {
                        return Some(RealAlg::Root {
                            p: sf,
                            lo: rlo,
                            hi: rhi,
                        });
                    }
                }
            }
            me.refine();
        }
        None
    }

    /// A defining integer polynomial (denominator-cleared linear one for
    /// rationals) — for resultant inputs.
    fn defining_poly(&self) -> (Poly, ()) {
        match self {
            RealAlg::Rational(r) => (
                primitive(&trim(vec![-r.numer().clone(), r.denom().clone()])),
                (),
            ),
            RealAlg::Root { p, .. } => (p.clone(), ()),
        }
    }
}

/// A rational lower bound for x^(1/q) (x may be any sign; odd q handles
/// negatives by symmetry). Cheap: refine via f64 then verify by powering.
fn rational_root_lower(x: &BigRational, q: u32) -> Option<BigRational> {
    rational_root_bound(x, q, false)
}

fn rational_root_upper(x: &BigRational, q: u32) -> Option<BigRational> {
    rational_root_bound(x, q, true)
}

fn rational_root_bound(x: &BigRational, q: u32, upper: bool) -> Option<BigRational> {
    if x.is_negative() && q % 2 == 0 {
        return None;
    }
    let negate = x.is_negative();
    let ax = x.abs();
    // f64 seed, then step outward until the q-th power provably brackets.
    let f = num_traits::ToPrimitive::to_f64(&ax)?;
    let seed = f.powf(1.0 / q as f64);
    let mut r = BigRational::from_f64_approx(seed);
    let step = BigRational::new(BigInt::one(), BigInt::from(1u64 << 20));
    let want_upper = upper != negate; // for negatives the roles swap
    for _ in 0..256 {
        let powed = pow_rat(&r, q);
        let ok = if want_upper { powed >= ax } else { powed <= ax };
        if ok {
            let signed = if negate { -r.clone() } else { r.clone() };
            return Some(signed);
        }
        if want_upper {
            r += &step;
        } else {
            r -= &step;
        }
        if r.is_negative() {
            r = BigRational::from_integer(BigInt::zero());
        }
    }
    None
}

trait F64Approx {
    fn from_f64_approx(f: f64) -> Self;
}

impl F64Approx for BigRational {
    fn from_f64_approx(f: f64) -> Self {
        num_rational::Ratio::from_float(f)
            .unwrap_or_else(|| BigRational::from_integer(BigInt::zero()))
    }
}

// ---------------------------------------------------------------------------
// Chebyshev: exact trig of rational multiples of π.
// ---------------------------------------------------------------------------

/// T_n(x) over ℤ by the recurrence T₀ = 1, T₁ = x, T_{k+1} = 2x·T_k − T_{k−1}.
fn chebyshev_t(n: u32) -> Poly {
    let mut t0: Poly = vec![BigInt::one()];
    if n == 0 {
        return t0;
    }
    let mut t1: Poly = vec![BigInt::zero(), BigInt::one()];
    let two_x: Poly = vec![BigInt::zero(), BigInt::from(2)];
    for _ in 1..n {
        let next = poly_add(&poly_mul(&two_x, &t1), &poly_neg(&t0));
        t0 = t1;
        t1 = next;
    }
    t1
}

/// cos(r·π) for rational r, as a real algebraic number: with r = k/n in
/// lowest terms, T_n(cos(kπ/n)) = cos(kπ) = ±1, so the value is a root of
/// T_n(x) ∓ 1 — selected by an f64-seeded interval, verified by Sturm.
pub fn cos_pi_rational(r: &BigRational) -> Option<RealAlg> {
    // Reduce r mod 2 (cos period), fold to [0, 1] (cos(−θ) = cos θ,
    // cos(2π − θ) = cos θ).
    let two = BigRational::from_integer(BigInt::from(2));
    let mut r = r - (r / &two).floor() * &two; // r ∈ [0, 2)
    if r > BigRational::from_integer(BigInt::one()) {
        r = &two - &r; // ∈ (0, 1]
    }
    let n = num_traits::ToPrimitive::to_u32(r.denom())?;
    let k = num_traits::ToPrimitive::to_i64(r.numer())?;
    if n as usize > MAX_DEG {
        return None;
    }
    // Easy exact points.
    if n == 1 {
        return Some(RealAlg::Rational(BigRational::from_integer(
            if k % 2 == 0 {
                BigInt::one()
            } else {
                -BigInt::one()
            },
        )));
    }
    if n == 2 {
        return Some(RealAlg::Rational(BigRational::from_integer(BigInt::zero())));
    }
    let tn = chebyshev_t(n);
    let rhs = if k % 2 == 0 {
        BigInt::one()
    } else {
        -BigInt::one()
    };
    let mut p = tn.clone();
    p[0] -= rhs;
    let sf = squarefree(&p);
    // Seed interval: cos(kπ/n) via f64, padded by 1e-9 — root gaps of
    // T_n ∓ 1 are Ω(1/n²) ≫ that for n ≤ 64, and Sturm verifies anyway.
    let approx = ((k as f64) * std::f64::consts::PI / (n as f64)).cos();
    isolate_near(&sf, approx)
}

/// sin(r·π) = cos((1/2 − r)·π).
pub fn sin_pi_rational(r: &BigRational) -> Option<RealAlg> {
    let half = BigRational::new(BigInt::one(), BigInt::from(2));
    cos_pi_rational(&(half - r))
}

/// The unique root of sf near the (trusted-for-seeding-only) f64 value:
/// grow a rational interval around it until Sturm confirms exactly one root
/// with sign-changing, root-free endpoints. Sound regardless of seed
/// quality — a bad seed returns the wrong root only if the seed was wrong
/// by more than the verified isolation, which Sturm rules out by
/// construction here (we *verify*, never assume).
fn isolate_near(sf: &Poly, seed: f64) -> Option<RealAlg> {
    let chain = sturm_chain(sf);
    let center = BigRational::from_f64_approx(seed);
    let mut radius = BigRational::new(BigInt::one(), BigInt::from(1u64 << 40));
    for _ in 0..80 {
        let (mut lo, mut hi) = (&center - &radius, &center + &radius);
        let width = &hi - &lo;
        let nudge = &width / BigRational::from_integer(BigInt::from(1024));
        if sign_at(sf, &lo) == Sign::NoSign {
            lo -= &nudge;
        }
        if sign_at(sf, &hi) == Sign::NoSign {
            hi += &nudge;
        }
        if sign_at(sf, &lo) != Sign::NoSign && sign_at(sf, &hi) != Sign::NoSign {
            match count_roots(&chain, &lo, &hi) {
                1 if sign_at(sf, &lo) != sign_at(sf, &hi) => {
                    return Some(RealAlg::Root {
                        p: sf.clone(),
                        lo,
                        hi,
                    })
                }
                0 => {
                    radius *= BigRational::from_integer(BigInt::from(2));
                    continue;
                }
                _ => {
                    // More than one root inside: the seed sits between close
                    // roots — shrink instead.
                    radius /= BigRational::from_integer(BigInt::from(2));
                    continue;
                }
            }
        }
        radius *= BigRational::from_integer(BigInt::from(2));
    }
    None
}

// ---------------------------------------------------------------------------
// Expr → RealAlg conversion.
// ---------------------------------------------------------------------------

/// Convert a constant expression to a real algebraic number, when it is one
/// (within caps). π, e, transcendental function values, free symbols, and
/// complex values return `None` — the caller keeps its refusal behavior.
pub fn from_expr(e: &Expr) -> Option<RealAlg> {
    match e {
        Expr::Int(i) => Some(RealAlg::Rational(BigRational::from_integer(i.clone()))),
        Expr::Rat(r) => Some(RealAlg::Rational(r.clone())),
        Expr::Float(bf, _) => Some(RealAlg::Rational(crate::expr::float_to_rational(bf)?)),
        Expr::Add(ts) => {
            let mut acc = RealAlg::Rational(BigRational::from_integer(BigInt::zero()));
            for t in ts {
                acc = acc.add_alg(&from_expr(t)?)?;
            }
            Some(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = RealAlg::Rational(BigRational::from_integer(BigInt::one()));
            for f in fs {
                acc = acc.mul_alg(&from_expr(f)?)?;
            }
            Some(acc)
        }
        Expr::Pow(b, x) => {
            let r = crate::expr::numeric_value(x)?;
            let base = from_expr(b)?;
            let pnum = num_traits::ToPrimitive::to_i64(r.numer())?;
            let q = num_traits::ToPrimitive::to_u32(r.denom())?;
            base.rational_pow(pnum, q)
        }
        Expr::Func(name, args) => match (name.as_str(), args.as_slice()) {
            ("root", [poly, k]) => {
                let (coeffs, _) = root_call_coeffs(poly)?;
                let k = num_traits::ToPrimitive::to_usize(
                    crate::expr::numeric_value(k)?.to_integer().magnitude(),
                )?;
                RealAlg::nth_root_of(&coeffs, k)
            }
            ("cos", [arg]) => cos_pi_rational(&pi_multiple(arg)?),
            ("sin", [arg]) => sin_pi_rational(&pi_multiple(arg)?),
            ("tan", [arg]) => {
                let r = pi_multiple(arg)?;
                let s = sin_pi_rational(&r)?;
                let c = cos_pi_rational(&r)?;
                if c.sign() == std::cmp::Ordering::Equal {
                    return None; // pole
                }
                s.mul_alg(&c.recip_alg()?)
            }
            ("abs", [arg]) => {
                let v = from_expr(arg)?;
                Some(if v.sign() == std::cmp::Ordering::Less {
                    v.neg_alg()
                } else {
                    v
                })
            }
            _ => None,
        },
        _ => None,
    }
}

/// `arg` as a rational multiple of π: matches `π`, `r·π` (canonical Mul with
/// the numeric coefficient first), and plain rationals only when zero.
fn pi_multiple(arg: &Expr) -> Option<BigRational> {
    match arg {
        Expr::Const(Constant::Pi) => Some(BigRational::from_integer(BigInt::one())),
        Expr::Mul(fs) if fs.len() == 2 => {
            if matches!(fs[1], Expr::Const(Constant::Pi)) {
                crate::expr::numeric_value(&fs[0])
            } else {
                None
            }
        }
        Expr::Int(i) if i.is_zero() => Some(BigRational::from_integer(BigInt::zero())),
        _ => None,
    }
}

/// The rational coefficient vector (ascending) of a univariate polynomial
/// expression, plus its variable name — shared by the `root` builtin and
/// `from_expr`.
pub fn root_call_coeffs(poly: &Expr) -> Option<(Vec<BigRational>, String)> {
    let vars = collect_symbols(poly);
    if vars.len() != 1 {
        return None;
    }
    let var = vars.into_iter().next()?;
    let coeffs = crate::matrix::poly_coeffs(poly, &var)?;
    Some((coeffs, var))
}

fn collect_symbols(e: &Expr) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Expr::Symbol(s) => {
                out.insert(s.clone());
            }
            Expr::Add(ts) | Expr::Mul(ts) | Expr::Func(_, ts) => {
                ts.iter().for_each(|t| walk(t, out))
            }
            Expr::Pow(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Complex(a, b) | Expr::Equation(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Expr::Matrix(rows) => rows.iter().flatten().for_each(|t| walk(t, out)),
            _ => {}
        }
    }
    walk(e, &mut out);
    out
}

/// Exact sign of a constant expression via algebra, where interval
/// refinement had to refuse. `None` when the expression isn't (provably) a
/// real algebraic number within caps.
pub fn certified_sign(e: &Expr) -> Option<std::cmp::Ordering> {
    Some(from_expr(e)?.sign())
}

/// Refine `v` until its interval width is at most scale·2^−(bits+2), with
/// scale = max(1, |v|) — i.e. `bits` of relative precision (plus guard
/// bits), matching how the certified evaluators size their enclosures.
pub fn refine_bits(v: &mut RealAlg, bits: usize) {
    let (lo, hi) = v.bounds();
    let scale = lo
        .abs()
        .max(hi.abs())
        .max(BigRational::from_integer(BigInt::one()));
    let target = scale / BigRational::from_integer(BigInt::one() << (bits + 2));
    v.refine_to(&target);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rat(n: i64, d: i64) -> BigRational {
        BigRational::new(BigInt::from(n), BigInt::from(d))
    }

    fn from_ints(v: &[i64]) -> Poly {
        trim(v.iter().map(|&c| BigInt::from(c)).collect())
    }

    #[test]
    fn sturm_counts_roots_of_wilkinsonish_products() {
        // (x−1)(x−2)(x−3) = x³ − 6x² + 11x − 6
        let p = from_ints(&[-6, 11, -6, 1]);
        let chain = sturm_chain(&p);
        assert_eq!(count_roots(&chain, &rat(0, 1), &rat(4, 1)), 3);
        assert_eq!(count_roots(&chain, &rat(3, 2), &rat(5, 2)), 1);
        assert_eq!(count_roots(&chain, &rat(7, 2), &rat(9, 2)), 0);
    }

    #[test]
    fn isolation_finds_rational_and_irrational_roots() {
        // x³ − 2x² − x + 2 = (x−2)(x−1)(x+1)
        let roots = isolate_roots(&from_ints(&[2, -1, -2, 1]));
        assert_eq!(roots.len(), 3);
        // x² − 2: two irrational roots.
        let roots = isolate_roots(&from_ints(&[-2, 0, 1]));
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[1].sign(), std::cmp::Ordering::Greater);
        let a = roots[1].approx_f64();
        assert!((a - std::f64::consts::SQRT_2).abs() < 1e-12);
    }

    #[test]
    fn sqrt2_plus_sqrt3_squared_equals_5_plus_2_sqrt6() {
        let sqrt2 = RealAlg::nth_root_of(&[rat(-2, 1), rat(0, 1), rat(1, 1)], 2).unwrap();
        let sqrt3 = RealAlg::nth_root_of(&[rat(-3, 1), rat(0, 1), rat(1, 1)], 2).unwrap();
        let sqrt6 = RealAlg::nth_root_of(&[rat(-6, 1), rat(0, 1), rat(1, 1)], 2).unwrap();
        let lhs = sqrt2.add_alg(&sqrt3).unwrap().powi_alg(2).unwrap();
        let rhs = RealAlg::Rational(rat(5, 1))
            .add_alg(&RealAlg::Rational(rat(2, 1)).mul_alg(&sqrt6).unwrap())
            .unwrap();
        assert_eq!(lhs.cmp_alg(&rhs), std::cmp::Ordering::Equal);
        // And a near-miss stays unequal.
        let off = rhs.add_alg(&RealAlg::Rational(rat(1, 1 << 30))).unwrap();
        assert_eq!(lhs.cmp_alg(&off), std::cmp::Ordering::Less);
    }

    #[test]
    fn cbrt2_cubed_is_2() {
        let cbrt2 = RealAlg::Rational(rat(2, 1)).rational_pow(1, 3).unwrap();
        let cubed = cbrt2.powi_alg(3).unwrap();
        assert_eq!(
            cubed.cmp_alg(&RealAlg::Rational(rat(2, 1))),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn cos_pi_over_7_satisfies_its_cubic() {
        // 8c³ − 4c² − 4c + 1 = 0 for c = cos(π/7).
        let c = cos_pi_rational(&rat(1, 7)).unwrap();
        let c2 = c.powi_alg(2).unwrap();
        let c3 = c.powi_alg(3).unwrap();
        let sum = RealAlg::Rational(rat(8, 1))
            .mul_alg(&c3)
            .unwrap()
            .add_alg(&RealAlg::Rational(rat(-4, 1)).mul_alg(&c2).unwrap())
            .unwrap()
            .add_alg(&RealAlg::Rational(rat(-4, 1)).mul_alg(&c).unwrap())
            .unwrap()
            .add_alg(&RealAlg::Rational(rat(1, 1)))
            .unwrap();
        assert_eq!(sum.sign(), std::cmp::Ordering::Equal);
        // Sanity: the numeric value is cos(π/7) ≈ 0.9009688679.
        assert!((c.approx_f64() - (std::f64::consts::PI / 7.0).cos()).abs() < 1e-12);
    }

    #[test]
    fn ordering_of_close_algebraics_is_exact() {
        // 2^(1/3) vs 2^(1/3) + 1/2^40: distinct, order decided exactly.
        let a = RealAlg::Rational(rat(2, 1)).rational_pow(1, 3).unwrap();
        let b = a.add_alg(&RealAlg::Rational(rat(1, 1 << 40))).unwrap();
        assert_eq!(a.cmp_alg(&b), std::cmp::Ordering::Less);
        assert_eq!(b.cmp_alg(&a), std::cmp::Ordering::Greater);
        assert_eq!(a.cmp_alg(&a.clone()), std::cmp::Ordering::Equal);
    }
}
