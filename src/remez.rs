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
//! v1 designs Type I filters (odd length, even symmetry) — lowpass,
//! highpass, bandpass, and arbitrary multiband all fit.

use crate::expr::{func, numeric_value, BigRational, Expr};
use crate::interval;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};

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
        let (a, delta) = solve_levelled(&grid, &d, &w, &extremals, r)?;

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
        let mut row_rat: Vec<BigRational> = Vec::with_capacity(m + 1);
        // T_0..T_{r-1} by the Chebyshev recurrence.
        let mut t_prev = BigRational::from_integer(1.into());
        let mut t_curr = x.clone();
        for k in 0..r {
            let t_k = match k {
                0 => t_prev.clone(),
                1 => t_curr.clone(),
                _ => {
                    let t_next = BigRational::from_integer(2.into()) * x * &t_curr - &t_prev;
                    t_prev = std::mem::replace(&mut t_curr, t_next);
                    t_curr.clone()
                }
            };
            row_rat.push(t_k);
        }
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
fn select_extremals(
    err: &[BigInt],
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
