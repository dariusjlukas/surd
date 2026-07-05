//! Exact Parks–McClellan (Remez exchange) FIR design.
//!
//! The classic algorithm, with the classic failure modes deleted. A float
//! implementation fights two numerics battles at once: the interpolation
//! system is ill-conditioned (hence barycentric Lagrange contortions), and
//! convergence is detected by tolerance (hence "failed to converge" on
//! perfectly reasonable specs). Here the whole exchange runs in exact
//! rational arithmetic:
//!
//! * The substitution x = cos ω turns the cosine polynomial
//!   A(ω) = Σ aₖ·cos(kω) into an ordinary polynomial Σ aₖ·Tₖ(x) in
//!   Chebyshev form, so a *rational* design grid in x keeps every quantity
//!   in ℚ end to end.
//! * The interpolation system solves exactly — conditioning is a rounding
//!   phenomenon, and there is no rounding.
//! * The minimax problem is solved **exactly on the design grid** (which is
//!   what float implementations actually iterate on too — they just don't
//!   solve even that exactly). The levelled error |δ| strictly increases
//!   every exchange over a finite grid, so termination is a theorem, not a
//!   tolerance. The returned ripple is the exact rational minimax error on
//!   the grid.
//!
//! All four linear-phase types are supported. Types II–IV multiply the
//! cosine polynomial by Q(ω) = cos(ω/2), sin(ω), or sin(ω/2) — irrational
//! in x — so each type designs in its own variable where the whole basis is
//! rational on a rational grid:
//!
//! * Type II (even n, symmetric):      u = cos(ω/2), basis u·Tₖ(2u²−1)
//! * Type III (odd n, antisymmetric):  t = tan(ω/2), basis
//!   (2t/(1+t²))·Tₖ((1−t²)/(1+t²)) — the Weierstrass substitution makes
//!   both sin ω and cos ω rational
//! * Type IV (even n, antisymmetric):  v = sin(ω/2), basis v·Tₖ(1−2v²)
//!
//! Each basis spans a Chebyshev (Haar) system on the open design domain, so
//! the alternation theory — and the exact-termination argument — carry over
//! unchanged. The types' forced zeros (II: ω=π, III: ω=0 and π, IV: ω=0)
//! are structural: a band demanding a nonzero response at a forced zero
//! gets the honest best approximation, not an error.

use crate::expr::{func, mul, numeric_value, rat_to_expr, BigRational, Expr};
use crate::interval;
use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};

/// Largest supported filter length (odd). The exact solve is O(r³) on
/// rationals whose size grows with r — past this it stops being interactive.
pub const MAX_TAPS: usize = 127;

/// The design lattice: every grid point (and every inexact band edge,
/// snapped inward) is a multiple of 2^-24 in x = cos ω (≈ 6e-8 — orders of
/// magnitude finer than any practical band spec). Small denominators keep
/// the exact arithmetic fast.
const EDGE_BITS: usize = 24;

/// Design-grid points per cosine coefficient (the usual firpm density).
const GRID_DENSITY: usize = 16;
/// Cap on total grid points.
const MAX_GRID: usize = 8192;
/// Exchange-iteration backstop. Termination is guaranteed by the strictly
/// increasing levelled error; this guards implementation bugs, not math.
const MAX_ITERATIONS: usize = 100;

/// Length cap for Types II–IV: their design variables enter the basis
/// squared, so rows carry ~2× the bits of a Type I row of the same order —
/// 48 taps designs in seconds, 64 takes tens of seconds.
pub const MAX_TAPS_II_IV: usize = 64;

/// One frequency band of the specification, already mapped to x = cos ω
/// (so `x_lo < x_hi`, and bands are sorted ascending in x).
struct Band {
    x_lo: BigRational,
    x_hi: BigRational,
    desired: BigRational,
    weight: BigRational,
}

pub struct Design {
    /// The filter taps h[0..n], symmetric, exact.
    pub taps: Vec<BigRational>,
    /// The exact minimax weighted ripple δ on the design grid.
    pub ripple: BigRational,
    /// Exchange iterations used.
    pub iterations: usize,
}

/// Design an n-tap Type I linear-phase FIR filter: minimize the maximum of
/// W(ω)·|D(ω) − A(ω)| over the bands, exactly, on the design grid.
///
/// `edges` are band edges in radians/sample (pairs: [lo, hi, lo, hi, …]),
/// ascending within [0, π]; `desired`/`weights` give one value per band.
pub fn design(
    n: usize,
    edges: &[Expr],
    desired: &[Expr],
    weights: &[Expr],
) -> Result<Design, String> {
    if n < 3 || n.is_multiple_of(2) {
        return Err(format!(
            "dsp.remez designs Type I filters: the tap count must be odd and at least 3, got {}",
            n
        ));
    }
    if n > MAX_TAPS {
        return Err(format!(
            "dsp.remez supports up to {} taps (the exact solve grows fast past that), got {}",
            MAX_TAPS, n
        ));
    }
    if edges.len() < 2 || !edges.len().is_multiple_of(2) {
        return Err("dsp.remez band edges come in pairs: [lo1, hi1, lo2, hi2, ...]".into());
    }
    let nbands = edges.len() / 2;
    if desired.len() != nbands {
        return Err(format!(
            "dsp.remez expects one desired value per band ({} bands, {} values)",
            nbands,
            desired.len()
        ));
    }
    if weights.len() != nbands {
        return Err(format!(
            "dsp.remez expects one weight per band ({} bands, {} weights)",
            nbands,
            weights.len()
        ));
    }

    let bands = resolve_bands(edges, desired, weights)?;
    let r = n.div_ceil(2); // cosine coefficients a_0..a_{r-1}; r = (n+1)/2
    let (grid, d, w) = build_grid(&bands, r)?;
    if grid.len() < r + 2 {
        return Err(
            "dsp.remez: the bands are too narrow for this filter order (not enough design \
             grid points) — reduce the order or widen the bands"
                .into(),
        );
    }

    // Initial extremals: spread across the whole grid.
    let mut extremals: Vec<usize> = (0..=r).map(|i| i * (grid.len() - 1) / r).collect();
    extremals.dedup();
    if extremals.len() != r + 1 {
        return Err("dsp.remez: grid too coarse to seed the exchange".into());
    }

    let ranges = band_ranges(&bands, &grid);
    let mut iterations = 0;
    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(
                "dsp.remez: the exchange did not settle (this should be impossible — \
                 please report it)"
                    .into(),
            );
        }

        // Solve for a_0..a_{r-1} and δ on the current extremals.
        let (a, delta) = solve_levelled(Ty::I, &grid, &d, &w, &extremals, r)?;

        // Weighted error over the grid, as *integer numerators* over one
        // shared denominator — comparisons stay in ℤ, with no per-op gcd
        // reduction (the dominant cost when the exact coefficients carry
        // determinant-sized denominators).
        let (err, q) = error_numerators(&grid, &d, &w, &a)?;

        // Converged exactly when no grid point beats the levelled error:
        // max|e| == |δ|  ⟺  max|N|·qδ == pδ·Q, all in ℤ.
        let abs_delta = delta.abs();
        let max_err_num = err.iter().map(|e| e.abs()).max().expect("grid nonempty");
        if &max_err_num * abs_delta.denom() == abs_delta.numer() * &q {
            let taps = taps_from_cosine(&a);
            return Ok(Design {
                taps,
                ripple: abs_delta,
                iterations,
            });
        }
        extremals = select_extremals(&err, &ranges, &extremals, r + 1)?;
    }
}

/// The weighted error w·(d − P(x)) at every grid point, as integer
/// numerators over one shared positive denominator Q. P(x) evaluates by a
/// *scaled* Chebyshev recurrence: with x = m/s (the lattice), Cₖ = sᵏ·Tₖ(x)
/// satisfies Cₖ = 2m·Cₖ₋₁ − s²·Cₖ₋₂ — integers all the way down.
fn error_numerators(
    grid: &[BigRational],
    d: &[BigRational],
    w: &[BigRational],
    a: &[BigRational],
) -> Result<(Vec<BigInt>, BigInt), String> {
    let r = a.len();
    // Common denominator of the coefficients: aₖ = αₖ / da.
    let mut da = BigInt::from(1);
    for v in a {
        da = num_integer::lcm(da, v.denom().clone());
    }
    let alpha: Vec<BigInt> = a
        .iter()
        .map(|v| (v * BigRational::from_integer(da.clone())).to_integer())
        .collect();
    // Common scale of grid points and band constants: x = m/s with one s
    // (the lattice), w = wn/wd, d = dn/dd — fold wd·dd into the shared Q.
    let mut s = BigInt::from(1);
    for x in grid {
        s = num_integer::lcm(s, x.denom().clone());
    }
    let mut wd_all = BigInt::from(1);
    for j in 0..grid.len() {
        wd_all = num_integer::lcm(wd_all, w[j].denom() * d[j].denom());
    }
    // s^k powers, descending exponents for Σ αₖ·Cₖ·s^{r−1−k}.
    let mut s_pow = vec![BigInt::from(1); r];
    for k in 1..r {
        s_pow[k] = &s_pow[k - 1] * &s;
    }
    let s2 = &s * &s;
    // Q = wd_all · da · s^{r−1}; e_j = N_j / Q.
    let q = &wd_all * &da * &s_pow[r - 1];
    let mut out = Vec::with_capacity(grid.len());
    for j in 0..grid.len() {
        let m = (&grid[j] * BigRational::from_integer(s.clone())).to_integer();
        // Scaled Chebyshev sweep: S = Σ αₖ·Cₖ(m)·s^{r−1−k}  (= da·s^{r−1}·P).
        let mut c_prev = BigInt::from(1); // C_0
        let mut c_curr = m.clone(); // C_1
        let mut sum = &alpha[0] * &s_pow[r - 1];
        if r > 1 {
            sum += &alpha[1] * &c_curr * &s_pow[r - 2];
        }
        for k in 2..r {
            let c_next = BigInt::from(2) * &m * &c_curr - &s2 * &c_prev;
            c_prev = std::mem::replace(&mut c_curr, c_next);
            sum += &alpha[k] * &c_curr * &s_pow[r - 1 - k];
        }
        // N = wn·(dn·da·s^{r−1} − dd·S)·(wd_all / (wd·dd))
        let scale = &wd_all / (w[j].denom() * d[j].denom());
        let n = w[j].numer()
            * (d[j].numer() * d[j].denom() * &da * &s_pow[r - 1] / d[j].denom()
                - d[j].denom() * &sum)
            * &scale;
        out.push(n);
    }
    Ok((out, q))
}

/// Map ω-band edges to exact rational x = cos ω bounds, snapping inward
/// where the cosine has no rational value. Validates ordering as it goes.
fn resolve_bands(edges: &[Expr], desired: &[Expr], weights: &[Expr]) -> Result<Vec<Band>, String> {
    let snap = BigInt::from(1u64) << EDGE_BITS;
    // cos ω per edge: exact when canonicalization folds it to a rational,
    // otherwise a certified enclosure. Which side of the enclosure to use —
    // and which way to snap — depends on whether the edge bounds a band from
    // below or above, so that the band only ever *shrinks* (conservative).
    let mut xs: Vec<(BigRational, BigRational)> = Vec::with_capacity(edges.len());
    for e in edges {
        // Validate the edge ITSELF lies in [0, π]. Checking |cos ω| ≤ 1
        // instead is vacuous — it holds for every real ω — and an
        // out-of-domain edge (e.g. 5, or 7π/3) would silently design for
        // the cosine-folded band instead of the stated spec. The enclosure
        // check is conservative on the correct side: 0 and π themselves are
        // exact and pass.
        let edge_ok = |e: &Expr| -> Option<bool> {
            let (lo, _) = interval::rational_enclosure(e, 128)?;
            if lo < BigRational::zero() {
                return Some(false);
            }
            let pi_minus = crate::expr::add(vec![
                Expr::Const(crate::expr::Constant::Pi),
                crate::expr::mul(vec![crate::expr::int(-1), e.clone()]),
            ]);
            let (lo, _) = interval::rational_enclosure(&pi_minus, 128)?;
            Some(lo >= BigRational::zero())
        };
        match edge_ok(e) {
            Some(false) => return Err(format!("band edge '{}' is outside [0, π]", e)),
            Some(true) => {}
            // Not a constant: fall through, the enclosure path below
            // produces the precise error.
            None => {}
        }
        let c = func("cos", vec![e.clone()]);
        if let Some(x) = numeric_value(&c) {
            if x.abs() > BigRational::from_integer(1.into()) {
                return Err(format!("band edge '{}' is outside [0, π]", e));
            }
            xs.push((x.clone(), x));
        } else {
            let (lo, hi) = interval::rational_enclosure(&c, 128)
                .ok_or_else(|| format!("band edge '{}' is not a constant frequency", e))?;
            if hi > BigRational::from_integer(1.into())
                || lo < BigRational::from_integer((-1).into())
            {
                return Err(format!("band edge '{}' is outside [0, π]", e));
            }
            xs.push((lo, hi));
        }
    }
    // ω ascending ⇒ x descending. Validate strict ordering in x using the
    // conservative sides, then emit bands ascending in x (reverse order).
    for pair in xs.windows(2) {
        // next edge must be strictly below in x (strictly above in ω)
        if pair[1].1 >= pair[0].0 {
            return Err(
                "dsp.remez band edges must be strictly increasing within [0, π] \
                 (and separated by more than ~4e-15)"
                    .into(),
            );
        }
    }
    let mut bands = Vec::with_capacity(edges.len() / 2);
    for b in (0..edges.len() / 2).rev() {
        let (omega_lo, omega_hi) = (&xs[2 * b], &xs[2 * b + 1]);
        // Band [ω_lo, ω_hi] → x ∈ [cos ω_hi, cos ω_lo]. An inexact edge uses
        // the *inner* side of its enclosure, snapped further inward — the
        // band can only shrink, never claim frequencies outside the spec.
        let x_lo = if omega_hi.0 == omega_hi.1 {
            omega_hi.0.clone()
        } else {
            ceil_to(&omega_hi.1, &snap)
        }
        .max(BigRational::from_integer((-1).into()));
        let x_hi = if omega_lo.0 == omega_lo.1 {
            omega_lo.0.clone()
        } else {
            floor_to(&omega_lo.0, &snap)
        }
        .min(BigRational::from_integer(1.into()));
        if x_lo >= x_hi {
            return Err("dsp.remez: a band is too narrow (its edges collapse)".into());
        }
        let dv = numeric_value(&desired[b])
            .ok_or_else(|| format!("desired value '{}' must be a number", desired[b]))?;
        let wv = numeric_value(&weights[b])
            .filter(|v| v > &BigRational::zero())
            .ok_or_else(|| format!("weight '{}' must be a positive number", weights[b]))?;
        bands.push(Band {
            x_lo,
            x_hi,
            desired: dv,
            weight: wv,
        });
    }
    Ok(bands)
}

/// Round up / down to a multiple of 1/snap.
fn ceil_to(r: &BigRational, snap: &BigInt) -> BigRational {
    let scaled = r * BigRational::from_integer(snap.clone());
    BigRational::new(scaled.ceil().to_integer(), snap.clone())
}

fn floor_to(r: &BigRational, snap: &BigInt) -> BigRational {
    let scaled = r * BigRational::from_integer(snap.clone());
    BigRational::new(scaled.floor().to_integer(), snap.clone())
}

/// The design grid: rational x points per band (uniform within each band,
/// allotted by span), with the desired value and weight at each point.
#[allow(clippy::type_complexity)]
fn build_grid(
    bands: &[Band],
    r: usize,
) -> Result<(Vec<BigRational>, Vec<BigRational>, Vec<BigRational>), String> {
    let total_span: BigRational = bands
        .iter()
        .map(|b| &b.x_hi - &b.x_lo)
        .fold(BigRational::zero(), |acc, s| acc + s);
    if total_span <= BigRational::zero() {
        return Err("dsp.remez: empty design bands".into());
    }
    let target = (GRID_DENSITY * r).min(MAX_GRID);
    let mut grid = Vec::new();
    let mut d = Vec::new();
    let mut w = Vec::new();
    for b in bands {
        let span = &b.x_hi - &b.x_lo;
        // Points proportional to span, with a healthy floor per band.
        let share = (&span / &total_span * BigRational::from_integer((target as i64).into()))
            .to_integer()
            .to_usize()
            .unwrap_or(0);
        let m = share.clamp(8, MAX_GRID);
        let step = span / BigRational::from_integer((m as i64 - 1).into());
        let snap = BigInt::from(1u64) << EDGE_BITS;
        for j in 0..m {
            let x = &b.x_lo + &step * BigRational::from_integer((j as i64).into());
            // Interior points carry no spec meaning — snap them to the
            // lattice so every grid denominator stays at 2^EDGE_BITS. Edges
            // (j = 0, m−1) are already lattice points or exact rationals.
            let x = if j == 0 || j == m - 1 {
                x
            } else {
                floor_to(&x, &snap)
            };
            if grid.last() == Some(&x) {
                continue; // narrow bands can snap two points together
            }
            grid.push(x);
            d.push(b.desired.clone());
            w.push(b.weight.clone());
        }
    }
    Ok((grid, d, w))
}

/// Index ranges of each band within the concatenated grid (for the
/// per-band local-extremum scan).
fn band_ranges(bands: &[Band], grid: &[BigRational]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(bands.len());
    let mut start = 0;
    for b in bands {
        let mut end = start;
        while end < grid.len() && grid[end] <= b.x_hi {
            end += 1;
        }
        ranges.push((start, end));
        start = end;
    }
    ranges
}

/// Solve the levelled-error interpolation: Σₖ aₖ·Tₖ(xᵢ) + (−1)ⁱ·δ/Wᵢ = Dᵢ
/// over the r+1 extremals — exactly. Rows clear to integers (the lattice
/// keeps denominators small) and fraction-free Bareiss elimination keeps
/// intermediate growth determinant-bounded; the system is a Chebyshev
/// alternation system, so it is provably nonsingular for distinct nodes.
fn solve_levelled(
    ty: Ty,
    grid: &[BigRational],
    d: &[BigRational],
    w: &[BigRational],
    extremals: &[usize],
    r: usize,
) -> Result<(Vec<BigRational>, BigRational), String> {
    let m = extremals.len(); // r + 1
                             // Build the augmented matrix [A | b] over the rationals, then scale each
                             // row by the lcm of its denominators to integers.
    let mut aug: Vec<Vec<BigInt>> = Vec::with_capacity(m);
    for (i, &j) in extremals.iter().enumerate() {
        let x = &grid[j];
        let mut row_rat: Vec<BigRational> = basis_row(ty, x, r);
        row_rat.reserve(2);
        let sign = if i % 2 == 0 { 1 } else { -1 };
        row_rat.push(BigRational::from_integer(sign.into()) / &w[j]);
        row_rat.push(d[j].clone());
        // Clear denominators.
        let mut lcm = BigInt::from(1);
        for v in &row_rat {
            lcm = num_integer::lcm(lcm, v.denom().clone());
        }
        aug.push(
            row_rat
                .into_iter()
                .map(|v| (v * BigRational::from_integer(lcm.clone())).to_integer())
                .collect(),
        );
    }

    // Fraction-free (Bareiss) forward elimination on the augmented matrix.
    let cols = m + 1;
    let mut prev_pivot = BigInt::from(1);
    for k in 0..m {
        if aug[k][k].is_zero() {
            // Pivot: find a nonzero below (nonsingular system ⇒ one exists).
            let swap = (k + 1..m)
                .find(|&i| !aug[i][k].is_zero())
                .ok_or("dsp.remez: internal error (singular levelled system)")?;
            aug.swap(k, swap);
            // A row swap flips the determinant's sign; Bareiss stays exact
            // either way since divisions remain exact minors.
        }
        for i in k + 1..m {
            for j in k + 1..cols {
                let num = &aug[k][k] * &aug[i][j] - &aug[i][k] * &aug[k][j];
                aug[i][j] = num / &prev_pivot; // exact by Sylvester's identity
            }
            aug[i][k] = BigInt::from(0);
        }
        prev_pivot = aug[k][k].clone();
    }

    // Back substitution in rationals.
    let mut sol = vec![BigRational::zero(); m];
    for i in (0..m).rev() {
        let mut acc = BigRational::from_integer(aug[i][m].clone());
        for j in i + 1..m {
            acc -= BigRational::from_integer(aug[i][j].clone()) * &sol[j];
        }
        sol[i] = acc / BigRational::from_integer(aug[i][i].clone());
    }
    let delta = sol.pop().expect("r+1 unknowns");
    Ok((sol, delta))
}

/// The exchange step: pick the new extremal set — local maxima of |error|
/// per band, *unioned with the current extremals* (which sit at exactly ±δ
/// in alternation, so at least `want` alternating candidates always exist),
/// alternation enforced, trimmed to `want` keeping the largest.
fn select_extremals<T: Signed + Ord + Clone>(
    err: &[T],
    ranges: &[(usize, usize)],
    current: &[usize],
    want: usize,
) -> Result<Vec<usize>, String> {
    let mut candidates: Vec<usize> = Vec::new();
    for &(s, e) in ranges {
        for j in s..e {
            if err[j].is_zero() {
                continue;
            }
            let left_ok = j == s || err[j].abs() >= err[j - 1].abs();
            let right_ok = j + 1 == e || err[j].abs() >= err[j + 1].abs();
            if left_ok && right_ok {
                candidates.push(j);
            }
        }
    }
    // Merge in the current extremals (sorted union, dedup).
    candidates.extend(current.iter().copied());
    candidates.sort_unstable();
    candidates.dedup();
    // Alternation: consecutive same-sign candidates collapse to the larger.
    let mut alt: Vec<usize> = Vec::with_capacity(candidates.len());
    for j in candidates {
        match alt.last() {
            Some(&p) if err[p].is_positive() == err[j].is_positive() => {
                if err[j].abs() > err[p].abs() {
                    *alt.last_mut().expect("nonempty") = j;
                }
            }
            _ => alt.push(j),
        }
    }
    // Trim to `want`, dropping the smaller endpoint each time — this never
    // drops the global maximum, which the convergence argument needs.
    while alt.len() > want {
        let first = err[*alt.first().expect("nonempty")].abs();
        let last = err[*alt.last().expect("nonempty")].abs();
        if first <= last {
            alt.remove(0);
        } else {
            alt.pop();
        }
    }
    if alt.len() < want {
        return Err(
            "dsp.remez: the design grid is too coarse for this filter order — \
             reduce the order or widen the bands"
                .into(),
        );
    }
    Ok(alt)
}

/// Type I taps from the cosine coefficients: h[M] = a₀, h[M±k] = aₖ/2.
fn taps_from_cosine(a: &[BigRational]) -> Vec<BigRational> {
    let m = a.len() - 1; // middle index; n = 2m+1
    let mut h = vec![BigRational::zero(); 2 * m + 1];
    h[m] = a[0].clone();
    let half = BigRational::new(1.into(), 2.into());
    for (k, ak) in a.iter().enumerate().skip(1) {
        let v = ak * &half;
        h[m - k] = v.clone();
        h[m + k] = v;
    }
    h
}

// ---------------------------------------------------------------------------
// Types II–IV: the generalized-basis exchange.
// ---------------------------------------------------------------------------

/// The four linear-phase FIR types, keyed by length parity and symmetry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    I,
    II,
    III,
    IV,
}

impl Ty {
    pub fn classify(n: usize, antisymmetric: bool) -> Ty {
        match (n % 2 == 1, antisymmetric) {
            (true, false) => Ty::I,
            (false, false) => Ty::II,
            (true, true) => Ty::III,
            (false, true) => Ty::IV,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Ty::I => "I",
            Ty::II => "II",
            Ty::III => "III",
            Ty::IV => "IV",
        }
    }

    pub fn number(self) -> i64 {
        match self {
            Ty::I => 1,
            Ty::II => 2,
            Ty::III => 3,
            Ty::IV => 4,
        }
    }

    /// Number of basis coefficients for an n-tap filter of this type.
    fn r_of(self, n: usize) -> usize {
        match self {
            Ty::I => n.div_ceil(2),
            Ty::II | Ty::IV => n / 2,
            Ty::III => (n - 1) / 2,
        }
    }

    /// The design variable for an ω band edge, as an Expr: x = cos ω,
    /// u = cos(ω/2), t = tan(ω/2), v = sin(ω/2).
    fn var_expr(self, e: &Expr) -> Expr {
        let half = rat_to_expr(BigRational::new(BigInt::from(1), BigInt::from(2)));
        match self {
            Ty::I => func("cos", vec![e.clone()]),
            Ty::II => func("cos", vec![mul(vec![half, e.clone()])]),
            Ty::III => func("tan", vec![mul(vec![half, e.clone()])]),
            Ty::IV => func("sin", vec![mul(vec![half, e.clone()])]),
        }
    }

    /// Whether the design variable decreases as ω increases.
    fn descending(self) -> bool {
        matches!(self, Ty::I | Ty::II)
    }
}

/// The r basis-function values [φ₀(x) … φ_{r−1}(x)] at one design point, in
/// the type's own variable — rational everywhere:
///   I:  Tₖ(x)                    II: u·Tₖ(2u²−1)
///   III: (2t/(1+t²))·Tₖ(y), y = (1−t²)/(1+t²)
///   IV: v·Tₖ(1−2v²)
fn basis_row(ty: Ty, var: &BigRational, r: usize) -> Vec<BigRational> {
    let one = BigRational::one();
    let two = BigRational::from_integer(2.into());
    let (y, pref) = match ty {
        Ty::I => (var.clone(), one.clone()),
        Ty::II => (&two * var * var - &one, var.clone()),
        Ty::IV => (&one - &two * var * var, var.clone()),
        Ty::III => {
            let t2 = var * var;
            let den = &one + &t2;
            ((&one - &t2) / &den, &two * var / &den)
        }
    };
    let mut row = Vec::with_capacity(r);
    let mut t_prev = one;
    let mut t_curr = y.clone();
    for k in 0..r {
        let t_k = match k {
            0 => t_prev.clone(),
            1 => t_curr.clone(),
            _ => {
                let t_next = BigRational::from_integer(2.into()) * &y * &t_curr - &t_prev;
                t_prev = std::mem::replace(&mut t_curr, t_next);
                t_curr.clone()
            }
        };
        row.push(&pref * t_k);
    }
    row
}

/// Σₖ αₖ·φₖ(var) for integer-cleared coefficients α (= a·da), evaluated by
/// a scaled integer Chebyshev recurrence — one BigRational materializes per
/// point. For var = m/s: II/IV run Cₖ = s^{2k}·Tₖ(y) with y-numerator
/// ±(2m²−s²) over s²; III runs Cₖ = d^k·Tₖ(y) with d = s²+m², y-numerator
/// s²−m², prefactor 2ms/d.
fn eval_p_scaled(ty: Ty, var: &BigRational, alpha: &[BigInt]) -> (BigInt, BigInt) {
    let r = alpha.len();
    let m = var.numer().clone();
    let s = var.denom().clone();
    let (y_num, modulus, pref_num): (BigInt, BigInt, BigInt) = match ty {
        Ty::I => (m.clone(), s.clone(), BigInt::from(1)),
        Ty::II => (BigInt::from(2) * &m * &m - &s * &s, &s * &s, m.clone()),
        Ty::IV => (&s * &s - BigInt::from(2) * &m * &m, &s * &s, m.clone()),
        Ty::III => (
            &s * &s - &m * &m,
            &s * &s + &m * &m,
            BigInt::from(2) * &m * &s,
        ),
    };
    // Cₖ = modulusᵏ·Tₖ(y_num/modulus): Cₖ = 2·y_num·Cₖ₋₁ − modulus²·Cₖ₋₂.
    let mod2 = &modulus * &modulus;
    let mut mod_pow = vec![BigInt::from(1); r];
    for k in 1..r {
        mod_pow[k] = &mod_pow[k - 1] * &modulus;
    }
    let mut c_prev = BigInt::from(1);
    let mut c_curr = y_num.clone();
    let mut sum = &alpha[0] * &mod_pow[r - 1];
    if r > 1 {
        sum += &alpha[1] * &c_curr * &mod_pow[r - 2];
    }
    for (k, ak) in alpha.iter().enumerate().skip(2) {
        let c_next = BigInt::from(2) * &y_num * &c_curr - &mod2 * &c_prev;
        c_prev = std::mem::replace(&mut c_curr, c_next);
        sum += ak * &c_curr * &mod_pow[r - 1 - k];
    }
    // Total = pref · Σ αₖTₖ, as a RAW (numerator, positive denominator)
    // pair — callers fold it into per-point error numerators; nothing here
    // may construct a reducing BigRational (gcds on determinant-sized
    // integers were a 60×+ slowdown):
    //   I:   sum / s^{r−1}
    //   II:  (m/s)·sum / s^{2(r−1)}          = m·sum / s^{2r−1}
    //   IV:  same shape as II
    //   III: (2ms/d)·sum / d^{r−1}           = 2ms·sum / d^r
    match ty {
        Ty::I => (sum, mod_pow[r - 1].clone()),
        // u = m/s times sum/(s²)^{r−1}: m·sum over s^{2r−1}.
        Ty::II | Ty::IV => (&pref_num * &sum, &mod_pow[r - 1] * &s),
        // (2ms/d) times sum/d^{r−1}: 2ms·sum over d^r.
        Ty::III => (&pref_num * &sum, &mod_pow[r - 1] * &modulus),
    }
}

/// Design any linear-phase type. `antisymmetric` selects Types III/IV.
pub fn design_typed(
    n: usize,
    antisymmetric: bool,
    edges: &[Expr],
    desired: &[Expr],
    weights: &[Expr],
) -> Result<Design, String> {
    let ty = Ty::classify(n, antisymmetric);
    if ty == Ty::I {
        return design(n, edges, desired, weights);
    }
    let min_taps = if ty == Ty::III { 3 } else { 2 };
    if n < min_taps {
        return Err(format!(
            "dsp.remez: a Type {} filter needs at least {} taps",
            ty.name(),
            min_taps
        ));
    }
    if n > MAX_TAPS_II_IV {
        return Err(format!(
            "dsp.remez supports up to {} taps for Type {} designs (their exact solve \
             carries twice the lattice precision per row), got {}",
            MAX_TAPS_II_IV,
            ty.name(),
            n
        ));
    }
    if edges.len() < 2 || !edges.len().is_multiple_of(2) {
        return Err("dsp.remez band edges come in pairs: [lo1, hi1, lo2, hi2, ...]".into());
    }
    let nbands = edges.len() / 2;
    if desired.len() != nbands || weights.len() != nbands {
        return Err(format!(
            "dsp.remez expects one desired value and one weight per band ({} bands)",
            nbands
        ));
    }
    let r = ty.r_of(n);
    let bands = resolve_bands_var(ty, edges, desired, weights)?;
    let (grid, d, w) = build_grid(&bands, r)?;
    if grid.len() < r + 2 {
        return Err(
            "dsp.remez: the bands are too narrow for this filter order (not enough design \
             grid points) — reduce the order or widen the bands"
                .into(),
        );
    }
    let mut extremals: Vec<usize> = (0..=r).map(|i| i * (grid.len() - 1) / r).collect();
    extremals.dedup();
    if extremals.len() != r + 1 {
        return Err("dsp.remez: grid too coarse to seed the exchange".into());
    }
    let ranges = band_ranges(&bands, &grid);
    let mut iterations = 0;
    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(
                "dsp.remez: the exchange did not settle (this should be impossible — \
                 please report it)"
                    .into(),
            );
        }
        let (a, delta) = solve_levelled(ty, &grid, &d, &w, &extremals, r)?;
        // Integer-cleared coefficients for the scaled sweep.
        let mut da = BigInt::from(1);
        for v in &a {
            da = num_integer::lcm(da, v.denom().clone());
        }
        let alpha: Vec<BigInt> = a
            .iter()
            .map(|v| (v * BigRational::from_integer(da.clone())).to_integer())
            .collect();
        // Error per point as a RAW ratio (never reduced: num-rational's
        // comparisons cross-multiply, so `new_raw` keeps the exchange
        // gcd-free), and *scaled by da*: the numerator carries the
        // determinant-sized da, so leaving da OUT of the denominator keeps
        // every cross-multiplied comparison at da×small instead of da×da —
        // the difference between interactive and minutes at r = 32. All
        // errors share the same scale, so extremal selection is unchanged;
        // only the termination threshold must scale to match:
        // e·da = wn·(dn·da·pd − dd·pn) / (wd·dd·pd), |δ|·da likewise.
        let err: Vec<BigRational> = (0..grid.len())
            .map(|j| {
                let (pn, pd) = eval_p_scaled(ty, &grid[j], &alpha);
                let n = w[j].numer() * (d[j].numer() * &da * &pd - d[j].denom() * &pn);
                let q = w[j].denom() * d[j].denom() * &pd;
                BigRational::new_raw(n, q)
            })
            .collect();
        let abs_delta = delta.abs();
        let scaled_delta = BigRational::new_raw(abs_delta.numer() * &da, abs_delta.denom().clone());
        let max_err = err.iter().map(|e| e.abs()).max().expect("grid nonempty");
        if max_err == scaled_delta {
            let taps = taps_of_type(ty, &a);
            return Ok(Design {
                taps,
                ripple: abs_delta,
                iterations,
            });
        }
        extremals = select_extremals(&err, &ranges, &extremals, r + 1)?;
    }
}

/// Map ω-band edges into the type's design variable, snapping inexact
/// values inward on the 2^-EDGE_BITS lattice, validating order, and nudging
/// a forced-zero endpoint (u = 0, v = 0, t = 0) one lattice step inside —
/// the basis vanishes identically there, so the point carries no design
/// freedom.
fn resolve_bands_var(
    ty: Ty,
    edges: &[Expr],
    desired: &[Expr],
    weights: &[Expr],
) -> Result<Vec<Band>, String> {
    let snap = BigInt::from(1u64) << EDGE_BITS;
    let one_step = BigRational::new(BigInt::from(1), snap.clone());
    let mut vals: Vec<(BigRational, BigRational)> = Vec::with_capacity(edges.len());
    for e in edges {
        // Domain [0, π], checked on the edge itself (see resolve_bands).
        let edge_ok = |e: &Expr| -> Option<bool> {
            let (lo, _) = interval::rational_enclosure(e, 128)?;
            if lo < BigRational::zero() {
                return Some(false);
            }
            let pi_minus = crate::expr::add(vec![
                Expr::Const(crate::expr::Constant::Pi),
                crate::expr::mul(vec![crate::expr::int(-1), e.clone()]),
            ]);
            let (lo, _) = interval::rational_enclosure(&pi_minus, 128)?;
            Some(lo >= BigRational::zero())
        };
        match edge_ok(e) {
            Some(false) => return Err(format!("band edge '{}' is outside [0, π]", e)),
            Some(true) => {}
            None => {}
        }
        let v = ty.var_expr(e);
        if let Some(x) = numeric_value(&v) {
            vals.push((x.clone(), x));
        } else {
            let (lo, hi) = interval::rational_enclosure(&v, 128).ok_or_else(|| {
                if ty == Ty::III {
                    format!(
                        "band edge '{}' is not usable for a Type III design (tan(ω/2) must be \
                         finite — end the band strictly before π, where the response is \
                         structurally zero anyway)",
                        e
                    )
                } else {
                    format!("band edge '{}' is not a constant frequency", e)
                }
            })?;
            vals.push((lo, hi));
        }
    }
    // Ascending ω ⇒ var strictly descending (I, II) or ascending (III, IV).
    for pair in vals.windows(2) {
        let ok = if ty.descending() {
            pair[1].1 < pair[0].0
        } else {
            pair[1].0 > pair[0].1
        };
        if !ok {
            return Err(
                "dsp.remez band edges must be strictly increasing within [0, π] \
                 (and separated by more than the design lattice)"
                    .into(),
            );
        }
    }
    let nbands = edges.len() / 2;
    let mut bands = Vec::with_capacity(nbands);
    let band_order: Vec<usize> = if ty.descending() {
        (0..nbands).rev().collect()
    } else {
        (0..nbands).collect()
    };
    for b in band_order {
        let (omega_lo, omega_hi) = (&vals[2 * b], &vals[2 * b + 1]);
        // Inward snap per side; which ω edge is the var-low side depends on
        // the variable's direction.
        let (raw_lo, raw_hi) = if ty.descending() {
            (omega_hi, omega_lo)
        } else {
            (omega_lo, omega_hi)
        };
        let mut lo = if raw_lo.0 == raw_lo.1 {
            raw_lo.0.clone()
        } else {
            ceil_to(&raw_lo.1, &snap)
        };
        let mut hi = if raw_hi.0 == raw_hi.1 {
            raw_hi.0.clone()
        } else {
            floor_to(&raw_hi.0, &snap)
        };
        // A forced-zero endpoint contributes nothing: step inside.
        if lo.is_zero() {
            lo = one_step.clone();
        }
        if ty == Ty::II && hi.is_zero() {
            // (descending var: hi is the ω-low edge; u = 0 only at ω = π,
            // which lands in `lo` — but guard both ends anyway.)
            hi = -one_step.clone();
        }
        if lo >= hi {
            return Err("dsp.remez: a band is too narrow (its edges collapse)".into());
        }
        let dv = numeric_value(&desired[b])
            .ok_or_else(|| format!("desired value '{}' must be a number", desired[b]))?;
        let wv = numeric_value(&weights[b])
            .filter(|v| v > &BigRational::zero())
            .ok_or_else(|| format!("weight '{}' must be a positive number", weights[b]))?;
        bands.push(Band {
            x_lo: lo,
            x_hi: hi,
            desired: dv,
            weight: wv,
        });
    }
    Ok(bands)
}

/// Taps from the exchange coefficients, per type (standard Parks–McClellan
/// coefficient maps; α indexes the cosine-basis solution).
///   II:  A = Σ bₙ·cos((n−½)ω),  b₁ = α₀ + α₁/2,  bₙ = (αₙ₋₁ + αₙ)/2
///        h[r−n] = h[r+n−1] = bₙ/2
///   III: A = Σ cₙ·sin(nω),      c₁ = α₀ − α₂/2,  cₙ = (αₙ₋₁ − αₙ₊₁)/2
///        h[M−n] = cₙ/2 = −h[M+n], h[M] = 0
///   IV:  A = Σ dₙ·sin((n−½)ω),  d₁ = α₀ − α₁/2,  dₙ = (αₙ₋₁ − αₙ)/2
///        h[r−n] = dₙ/2 = −h[r+n−1]
/// (out-of-range α are zero; H carries e^{−iωM̃} — times i for III/IV.)
fn taps_of_type(ty: Ty, a: &[BigRational]) -> Vec<BigRational> {
    let r = a.len();
    let at = |k: usize| -> BigRational {
        if k < r {
            a[k].clone()
        } else {
            BigRational::zero()
        }
    };
    let half = BigRational::new(1.into(), 2.into());
    match ty {
        Ty::I => taps_from_cosine(a),
        Ty::II => {
            let mut h = vec![BigRational::zero(); 2 * r];
            for nn in 1..=r {
                let b_n = if nn == 1 {
                    at(0) + at(1) * &half
                } else {
                    (at(nn - 1) + at(nn)) * &half
                };
                let v = &b_n * &half;
                h[r - nn] = v.clone();
                h[r + nn - 1] = v;
            }
            h
        }
        Ty::III => {
            let m = r; // middle index; n = 2r + 1
            let mut h = vec![BigRational::zero(); 2 * r + 1];
            for nn in 1..=r {
                let c_n = if nn == 1 {
                    at(0) - at(2) * &half
                } else {
                    (at(nn - 1) - at(nn + 1)) * &half
                };
                let v = &c_n * &half;
                h[m - nn] = v.clone();
                h[m + nn] = -v;
            }
            h
        }
        Ty::IV => {
            let mut h = vec![BigRational::zero(); 2 * r];
            for nn in 1..=r {
                let d_n = if nn == 1 {
                    at(0) - at(1) * &half
                } else {
                    (at(nn - 1) - at(nn)) * &half
                };
                let v = &d_n * &half;
                h[r - nn] = v.clone();
                h[r + nn - 1] = -v;
            }
            h
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{int, mul, rat_to_expr, Constant};

    fn pi_frac(n: i64, d: i64) -> Expr {
        mul(vec![
            rat_to_expr(BigRational::new(BigInt::from(n), BigInt::from(d))),
            Expr::Const(Constant::Pi),
        ])
    }

    /// Evaluate the amplitude A at a design-variable point FROM THE TAPS —
    /// an independent path (Chebyshev identities per type), never touching
    /// the exchange's own P/error machinery:
    ///   I:   A(x) = h[M] + 2·Σ h[M−k]·Tₖ(x)
    ///   II:  A(u) = Σ 2h[r−n]·T_{2n−1}(u)
    ///   III: A(t) = Σ 2h[M−n]·sin(nω),  sin/cos ω rational in t
    ///   IV:  A(v) = Σ 2h[r−n]·(−1)^{n−1}·T_{2n−1}(v)
    fn amplitude_from_taps(ty: Ty, h: &[BigRational], var: &BigRational) -> BigRational {
        let cheb = |k: usize, x: &BigRational| -> BigRational {
            let mut t_prev = BigRational::from_integer(1.into());
            let mut t_curr = x.clone();
            if k == 0 {
                return t_prev;
            }
            for _ in 1..k {
                let next = BigRational::from_integer(2.into()) * x * &t_curr - &t_prev;
                t_prev = std::mem::replace(&mut t_curr, next);
            }
            t_curr
        };
        let two = BigRational::from_integer(2.into());
        match ty {
            Ty::I => {
                let m = h.len() / 2;
                let mut acc = h[m].clone();
                for k in 1..=m {
                    acc += &two * &h[m - k] * cheb(k, var);
                }
                acc
            }
            Ty::II => {
                let r = h.len() / 2;
                let mut acc = BigRational::zero();
                for n in 1..=r {
                    acc += &two * &h[r - n] * cheb(2 * n - 1, var);
                }
                acc
            }
            Ty::IV => {
                let r = h.len() / 2;
                let mut acc = BigRational::zero();
                for n in 1..=r {
                    let sign = if n % 2 == 1 {
                        BigRational::from_integer(1.into())
                    } else {
                        BigRational::from_integer((-1).into())
                    };
                    acc += &two * &h[r - n] * sign * cheb(2 * n - 1, var);
                }
                acc
            }
            Ty::III => {
                let m = h.len() / 2;
                let t2 = var * var;
                let den = BigRational::from_integer(1.into()) + &t2;
                let cosw = (BigRational::from_integer(1.into()) - &t2) / &den;
                let sinw = &two * var / &den;
                // sin(nω) by the complex-power recurrence on (cos ω, sin ω).
                let (mut c_n, mut s_n) = (cosw.clone(), sinw.clone());
                let mut acc = BigRational::zero();
                for n in 1..=m {
                    if n > 1 {
                        let c_next = &c_n * &cosw - &s_n * &sinw;
                        let s_next = &c_n * &sinw + &s_n * &cosw;
                        c_n = c_next;
                        s_n = s_next;
                    }
                    acc += &two * &h[m - n] * &s_n;
                }
                acc
            }
        }
    }

    /// The audit's D6 gap: spec compliance was only ever asserted at DC and
    /// Nyquist. This re-derives the error at EVERY design-grid point from
    /// the taps and asserts the two halves of the equioscillation
    /// certificate exactly: W·|D − A| ≤ δ everywhere on the grid, with at
    /// least r+1 alternating touches of ±δ (the alternation count for an
    /// r-dimensional Haar system — the filter literature's "L+2" counts
    /// the polynomial degree L = r−1).
    #[test]
    fn whole_grid_compliance_and_alternation() {
        let cases: &[(usize, bool, Vec<Expr>, Vec<Expr>)] = &[
            (
                15,
                false,
                vec![pi_frac(0, 1), pi_frac(2, 5), pi_frac(1, 2), pi_frac(1, 1)],
                vec![int(1), int(0)],
            ),
            (
                10,
                false,
                vec![pi_frac(0, 1), pi_frac(2, 5), pi_frac(3, 5), pi_frac(1, 1)],
                vec![int(1), int(0)],
            ),
            (11, true, vec![pi_frac(1, 5), pi_frac(4, 5)], vec![int(1)]),
            (8, true, vec![pi_frac(1, 3), pi_frac(1, 1)], vec![int(1)]),
        ];
        for (n, anti, edges, desired) in cases {
            let ty = Ty::classify(*n, *anti);
            let weights: Vec<Expr> = vec![int(1); desired.len()];
            let d = design_typed(*n, *anti, edges, desired, &weights).unwrap();
            // Rebuild the grid exactly as the design did.
            let r = ty.r_of(*n);
            let bands = if ty == Ty::I {
                resolve_bands(edges, desired, &weights).unwrap()
            } else {
                resolve_bands_var(ty, edges, desired, &weights).unwrap()
            };
            let (grid, dd, ww) = build_grid(&bands, r).unwrap();
            let mut touches: Vec<bool> = Vec::new(); // sign of e at |e| == δ
            for j in 0..grid.len() {
                let a = amplitude_from_taps(ty, &d.taps, &grid[j]);
                let e = &ww[j] * (&dd[j] - &a);
                let mag = e.clone().abs();
                assert!(
                    mag <= d.ripple,
                    "Type {} n={}: grid point {} exceeds the ripple",
                    ty.number(),
                    n,
                    j
                );
                if mag == d.ripple {
                    touches.push(e >= BigRational::zero());
                }
            }
            if d.ripple > BigRational::zero() {
                let mut alternations = 1;
                for w in touches.windows(2) {
                    if w[0] != w[1] {
                        alternations += 1;
                    }
                }
                assert!(
                    alternations >= r + 1,
                    "Type {} n={}: only {} alternating extremals (need {})",
                    ty.number(),
                    n,
                    alternations,
                    r + 1
                );
            }
        }
    }

    /// The structural invariants of each type, checked on exact taps:
    /// symmetry/antisymmetry, forced zeros (as exact tap-sum identities),
    /// and the middle tap of Type III.
    #[test]
    fn tap_structure_per_type() {
        // Type II: even length, symmetric; H(π) = Σ (−1)^k h[k] = 0.
        let d2 = design_typed(
            10,
            false,
            &[pi_frac(0, 1), pi_frac(2, 5), pi_frac(3, 5), pi_frac(1, 1)],
            &[int(1), int(0)],
            &[int(1), int(1)],
        )
        .unwrap();
        assert_eq!(d2.taps.len(), 10);
        for k in 0..10 {
            assert_eq!(d2.taps[k], d2.taps[9 - k], "Type II symmetry at {k}");
        }
        let nyq: BigRational = d2
            .taps
            .iter()
            .enumerate()
            .map(|(k, h)| if k % 2 == 0 { h.clone() } else { -h.clone() })
            .sum();
        assert!(nyq.is_zero(), "Type II forces H(π) = 0");

        // Type III: odd length, antisymmetric, zero middle tap;
        // H(0) = Σ h = 0 and H(π) = Σ (−1)^k h[k] = 0.
        let d3 = design_typed(
            11,
            true,
            &[pi_frac(1, 5), pi_frac(4, 5)],
            &[int(1)],
            &[int(1)],
        )
        .unwrap();
        assert_eq!(d3.taps.len(), 11);
        for k in 0..11 {
            assert_eq!(
                d3.taps[k],
                -d3.taps[10 - k].clone(),
                "Type III antisymmetry"
            );
        }
        assert!(d3.taps[5].is_zero(), "Type III middle tap is 0");
        let dc: BigRational = d3.taps.iter().cloned().sum();
        assert!(dc.is_zero(), "Type III forces H(0) = 0");

        // Type IV: even length, antisymmetric; H(0) = 0, H(π) free.
        let d4 = design_typed(
            8,
            true,
            &[pi_frac(1, 3), pi_frac(1, 1)],
            &[int(1)],
            &[int(1)],
        )
        .unwrap();
        assert_eq!(d4.taps.len(), 8);
        for k in 0..8 {
            assert_eq!(d4.taps[k], -d4.taps[7 - k].clone(), "Type IV antisymmetry");
        }
        let dc: BigRational = d4.taps.iter().cloned().sum();
        assert!(dc.is_zero(), "Type IV forces H(0) = 0");
        let nyq: BigRational = d4
            .taps
            .iter()
            .enumerate()
            .map(|(k, h)| if k % 2 == 0 { h.clone() } else { -h.clone() })
            .sum();
        assert!(!nyq.is_zero(), "Type IV does NOT force H(π) = 0");
    }
}
