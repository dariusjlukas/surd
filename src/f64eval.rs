//! Fast approximate evaluation: `Expr` → `f64`. The *pixel* path.
//!
//! Plotting samples an expression at hundreds of points, where arbitrary
//! precision would be wasted — pixels are already approximate. Anything
//! correctness-critical still goes through the exact engine and `N(...)`;
//! nothing here feeds back into symbolic results.

use crate::expr::{float_to_rational, Constant, Expr};
use num_traits::ToPrimitive;

/// Evaluate `e` to an `f64`, with free variables bound by `vars` (one for
/// curves, two for surfaces). Errors on anything symbolic, complex, or
/// non-scalar; IEEE non-finite results (poles, domain edges) are returned
/// as-is and dealt with by the caller.
pub fn eval_f64(e: &Expr, vars: &[(&str, f64)]) -> Result<f64, String> {
    match e {
        Expr::Int(i) => i
            .to_f64()
            .ok_or_else(|| "integer does not fit in f64".to_string()),
        Expr::Rat(r) => r
            .to_f64()
            .ok_or_else(|| "rational does not fit in f64".to_string()),
        Expr::Float(bf, _) => float_to_rational(bf)
            .and_then(|r| r.to_f64())
            .ok_or_else(|| "float is not finite".to_string()),
        Expr::Const(Constant::Pi) => Ok(std::f64::consts::PI),
        Expr::Const(Constant::E) => Ok(std::f64::consts::E),
        Expr::Symbol(s) => match vars.iter().find(|(name, _)| name == s) {
            Some((_, x)) => Ok(*x),
            None => Err(format!("cannot evaluate free symbol '{}'", s)),
        },
        Expr::Add(ts) => {
            let mut acc = 0.0;
            for t in ts {
                acc += eval_f64(t, vars)?;
            }
            Ok(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = 1.0;
            for f in fs {
                acc *= eval_f64(f, vars)?;
            }
            Ok(acc)
        }
        Expr::Pow(b, ex) => {
            let base = eval_f64(b, vars)?;
            let exp = eval_f64(ex, vars)?;
            // Integer exponents use powi so negative bases work ((-2)^3 = -8;
            // powf would give NaN).
            if exp.fract() == 0.0 && exp.abs() <= i32::MAX as f64 {
                Ok(base.powi(exp as i32))
            } else {
                Ok(base.powf(exp))
            }
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let x = eval_f64(&args[0], vars)?;
            match name.as_str() {
                "sin" => Ok(x.sin()),
                "cos" => Ok(x.cos()),
                "tan" => Ok(x.tan()),
                "exp" => Ok(x.exp()),
                "ln" => Ok(x.ln()),
                "abs" => Ok(x.abs()),
                _ => Err(format!("cannot evaluate '{}' numerically", name)),
            }
        }
        Expr::Func(name, _) => Err(format!("cannot evaluate '{}' numerically", name)),
        Expr::Complex(..) => Err("cannot plot a complex value on a real axis".to_string()),
        Expr::Matrix(..) => Err("cannot evaluate a matrix to a single number".to_string()),
        Expr::Bool(_) => Err("cannot evaluate a boolean to a number".to_string()),
        Expr::Function { .. } => Err("cannot evaluate a function value to a number".to_string()),
        Expr::Equation(..) => Err("cannot evaluate an equation to a number".to_string()),
        Expr::Struct(..) => Err("cannot evaluate a struct to a number".to_string()),
        Expr::Signal(_) => Err("cannot evaluate a signal to a single number".to_string()),
    }
}

/// Sample `e` at `n` evenly spaced values of `var` across [a, b], for plotting.
/// Points where evaluation fails or is non-finite come back as `None` — the
/// renderer draws a gap there (poles, log of negatives, …) rather than a lie.
pub fn sample(e: &Expr, var: &str, a: f64, b: f64, n: usize) -> Vec<(f64, Option<f64>)> {
    let n = n.clamp(2, 100_000);
    let step = (b - a) / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let x = a + step * i as f64;
            let y = match eval_f64(e, &[(var, x)]) {
                Ok(y) if y.is_finite() => Some(y),
                _ => None,
            };
            (x, y)
        })
        .collect()
}

/// Sample `e` on an `nx`×`ny` grid over `[a, b]`×`[c, d]`, for surface plots.
/// Returns heights row-major (`y` outer, `x` inner — `heights[j*nx + i]` is
/// the value at `(a + i·Δx, c + j·Δy)`); `None` marks poles / domain gaps,
/// same contract as [`sample`].
pub fn sample2d(
    e: &Expr,
    xvar: &str,
    yvar: &str,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    nx: usize,
    ny: usize,
) -> Vec<Option<f64>> {
    let nx = nx.clamp(2, 1000);
    let ny = ny.clamp(2, 1000);
    let step_x = (b - a) / (nx - 1) as f64;
    let step_y = (d - c) / (ny - 1) as f64;
    let mut heights = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        let y = c + step_y * j as f64;
        for i in 0..nx {
            let x = a + step_x * i as f64;
            heights.push(match eval_f64(e, &[(xvar, x), (yvar, y)]) {
                Ok(z) if z.is_finite() => Some(z),
                _ => None,
            });
        }
    }
    heights
}

/// Interpolation-residual tolerance driving *refinement* (curves and
/// surfaces alike), as a fraction of the robust value range. The tests below
/// compare each sample against linear interpolation of its 2×-coarser
/// neighbors; for smooth functions that residual is ~4× the drawn grid's own
/// interpolation error (second-order convergence), so 5% here certifies the
/// picture to ~1% of the drawn range.
const RESIDUAL_REFINE_TOL: f64 = 0.05;
/// Residual tolerance for the *undersampled* verdict at the resolution cap —
/// 4× looser than the refinement tolerance (the same Richardson factor), so
/// the flag fires only when the drawn curve/surface is genuinely off by ~5%
/// of the range, not merely uncertified to the refinement standard.
const RESIDUAL_WARN_TOL: f64 = 0.2;
/// Fraction of tested samples allowed to exceed a tolerance. Strictly zero
/// would force full refinement (and a permanent flag) for any pole or cliff —
/// a discontinuity fails the interpolation test at every resolution along a
/// measure-zero set, which is the renderer's gap/clamp problem, not a
/// sampling one.
const RESIDUAL_FRAC_TOL: f64 = 0.005;

/// An adaptively sampled curve: `(x, y)` points (same contract as
/// [`sample`]) plus the honesty flag — see [`Surface2d::undersampled`].
pub struct Curve1d {
    pub points: Vec<(f64, Option<f64>)>,
    pub undersampled: bool,
}

/// Sample `e` over `[a, b]` at the coarsest resolution that passes the
/// convergence test, doubling sample density (`n → 2n−1`, so sample sets
/// nest) from `base` up to `max` — the 1D sibling of [`sample2d_adaptive`],
/// same test, same tolerances.
pub fn sample_adaptive(e: &Expr, var: &str, a: f64, b: f64, base: usize, max: usize) -> Curve1d {
    let max = max.clamp(3, 100_000);
    // the even-index subset only reaches b when n is odd
    let base = base.clamp(3, max) | 1;
    let mut n = base;
    loop {
        let points = sample(e, var, a, b, n);
        let ys: Vec<Option<f64>> = points.iter().map(|p| p.1).collect();
        let (coarse, severe) = residual_fractions_1d(&ys);
        if coarse <= RESIDUAL_FRAC_TOL || 2 * n - 1 > max {
            return Curve1d {
                points,
                undersampled: severe > RESIDUAL_FRAC_TOL,
            };
        }
        n = 2 * n - 1;
    }
}

/// 1D counterpart of [`residual_fractions`]: each odd-indexed sample against
/// the midpoint of its even-indexed neighbors, clamped to the robust range.
fn residual_fractions_1d(ys: &[Option<f64>]) -> (f64, f64) {
    let Some((lo, hi)) = robust_range(ys) else {
        return (0.0, 0.0);
    };
    let at = |i: usize| ys[i].map(|y| y.clamp(lo, hi));
    let span = hi - lo;
    let (mut tested, mut coarse, mut severe) = (0u64, 0u64, 0u64);
    for i in (1..ys.len().saturating_sub(1)).step_by(2) {
        let (Some(v), Some(l), Some(r)) = (at(i), at(i - 1), at(i + 1)) else {
            continue;
        };
        tested += 1;
        let res = (v - (l + r) / 2.0).abs();
        if res > RESIDUAL_REFINE_TOL * span {
            coarse += 1;
        }
        if res > RESIDUAL_WARN_TOL * span {
            severe += 1;
        }
    }
    if tested == 0 {
        return (0.0, 0.0);
    }
    (coarse as f64 / tested as f64, severe as f64 / tested as f64)
}

/// An adaptively sampled surface: a square `n`×`n` heights grid (row-major,
/// same contract as [`sample2d`]) plus an honesty flag. `undersampled` means
/// even the finest grid tried still disagrees with its own coarse-grid
/// interpolation over more than a sliver of the window — the drawn surface
/// would alias, and the UI must say so rather than present it as truth.
pub struct Surface2d {
    pub n: usize,
    pub heights: Vec<Option<f64>>,
    pub undersampled: bool,
}

/// Sample `e` over `[a, b]`×`[c, d]` on the coarsest grid that passes a
/// convergence test, doubling cell density (`n → 2n−1`, so grids nest) from
/// `base` up to `max`.
///
/// The test is self-contained per grid: every sample with an odd index is
/// compared against linear interpolation of its even-indexed neighbors —
/// i.e. against the surface the 2×-coarser subgrid would have drawn. If the
/// two agree, refinement has converged and the grid is faithful; if not,
/// the function has structure between the coarse samples and we refine.
/// Values are clamped to the robust z-range first (mirroring the renderer,
/// which clamps spikes to the box), so one pole doesn't drown the test.
#[allow(clippy::too_many_arguments)]
pub fn sample2d_adaptive(
    e: &Expr,
    xvar: &str,
    yvar: &str,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    base: usize,
    max: usize,
) -> Surface2d {
    let max = max.clamp(3, 1000);
    // the even-index subgrid only reaches the far edge when n is odd
    let base = base.clamp(3, max) | 1;
    let mut n = base;
    loop {
        let heights = sample2d(e, xvar, yvar, a, b, c, d, n, n);
        let (coarse, severe) = residual_fractions(&heights, n);
        if coarse <= RESIDUAL_FRAC_TOL || 2 * n - 1 > max {
            return Surface2d {
                n,
                heights,
                undersampled: severe > RESIDUAL_FRAC_TOL,
            };
        }
        n = 2 * n - 1;
    }
}

/// The 2%–98% quantile range of the finite heights, padded — the same robust
/// z-range the frontend draws against (spikes beyond it clamp to the box).
/// `None` when no sample is finite.
fn robust_range(heights: &[Option<f64>]) -> Option<(f64, f64)> {
    let mut zs: Vec<f64> = heights.iter().filter_map(|h| *h).collect();
    if zs.is_empty() {
        return None;
    }
    zs.sort_unstable_by(f64::total_cmp);
    let mut lo = zs[zs.len() * 2 / 100];
    let mut hi = zs[(zs.len() * 98 / 100).min(zs.len() - 1)];
    if lo == hi {
        lo -= 1.0;
        hi += 1.0;
    }
    let pad = (hi - lo) * 0.02;
    Some((lo - pad, hi + pad))
}

/// Fractions of odd-indexed samples whose residual against the even-subgrid
/// interpolation exceeds ([`RESIDUAL_REFINE_TOL`], [`RESIDUAL_WARN_TOL`]) of
/// the robust z-range. Comparisons touching a `None` (pole / domain gap) are
/// skipped — gaps are already drawn honestly as holes. Requires odd `n`.
fn residual_fractions(heights: &[Option<f64>], n: usize) -> (f64, f64) {
    let Some((lo, hi)) = robust_range(heights) else {
        return (0.0, 0.0);
    };
    let at = |j: usize, i: usize| heights[j * n + i].map(|h| h.clamp(lo, hi));
    let avg2 = |p: Option<f64>, q: Option<f64>| Some((p? + q?) / 2.0);
    let span = hi - lo;
    let (mut tested, mut coarse, mut severe) = (0u64, 0u64, 0u64);
    for j in 0..n {
        for i in 0..n {
            let interp = match (i % 2, j % 2) {
                (0, 0) => continue,
                (1, 0) => avg2(at(j, i - 1), at(j, i + 1)),
                (0, 1) => avg2(at(j - 1, i), at(j + 1, i)),
                _ => avg2(
                    avg2(at(j - 1, i - 1), at(j - 1, i + 1)),
                    avg2(at(j + 1, i - 1), at(j + 1, i + 1)),
                ),
            };
            let (Some(v), Some(p)) = (at(j, i), interp) else {
                continue;
            };
            tested += 1;
            let r = (v - p).abs();
            if r > RESIDUAL_REFINE_TOL * span {
                coarse += 1;
            }
            if r > RESIDUAL_WARN_TOL * span {
                severe += 1;
            }
        }
    }
    if tested == 0 {
        return (0.0, 0.0);
    }
    (coarse as f64 / tested as f64, severe as f64 / tested as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    fn expr_of(src: &str) -> Expr {
        Interpreter::new().eval_line(src).unwrap()
    }

    #[test]
    fn agrees_with_the_exact_engine() {
        // Same oracle idea as the property suite, pointed the other way: the
        // f64 path must match exact-then-N to f64 precision.
        for src in ["1/3 + sin(1)", "exp(2) - pi", "2^10 + 1/7", "cos(pi)"] {
            let e = expr_of(src);
            let fast = eval_f64(&e, &[]).unwrap();
            let exact = expr_of(&format!("N({}, 25)", src));
            let exact_str = format!("{}", exact);
            let slow: f64 = exact_str.parse().unwrap();
            assert!(
                (fast - slow).abs() <= 1e-12 * slow.abs().max(1.0),
                "{}: fast {} vs exact {}",
                src,
                fast,
                slow
            );
        }
    }

    #[test]
    fn variable_binding_and_gaps() {
        let e = expr_of("x^2 + 1");
        assert_eq!(eval_f64(&e, &[("x", 3.0)]).unwrap(), 10.0);
        assert!(eval_f64(&e, &[]).is_err()); // free symbol

        // 1/x has a pole at 0: the sample there is a gap, not infinity.
        let inv = expr_of("x^(-1)");
        let pts = sample(&inv, "x", -1.0, 1.0, 3);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].1, Some(-1.0));
        assert_eq!(pts[1].1, None); // x = 0
        assert_eq!(pts[2].1, Some(1.0));
    }

    #[test]
    fn negative_base_integer_power() {
        let e = expr_of("y^3");
        assert_eq!(eval_f64(&e, &[("y", -2.0)]).unwrap(), -8.0);
    }

    #[test]
    fn adaptive_curves_mirror_surfaces() {
        // smooth: stays at the base resolution
        let e = expr_of("sin(x)");
        let c = sample_adaptive(&e, "x", 0.0, 6.28, 601, 4801);
        assert_eq!(c.points.len(), 601);
        assert!(!c.undersampled);

        // oscillatory but resolvable: refines past the base, unflagged
        let e = expr_of("sin(50*x)");
        let c = sample_adaptive(&e, "x", 0.0, 10.0, 601, 4801);
        assert!(c.points.len() > 601, "expected refinement past the base");
        assert!(!c.undersampled);

        // ~1600 oscillations across the window exceed the cap: flagged,
        // never silently drawn
        let c = sample_adaptive(&e, "x", -100.0, 100.0, 601, 4801);
        assert_eq!(c.points.len(), 4801);
        assert!(c.undersampled);
    }

    #[test]
    fn adaptive_keeps_smooth_surfaces_at_the_base_grid() {
        // x² + y² is essentially linear at 81×81 scale: converges immediately.
        let e = expr_of("x^2 + y^2");
        let s = sample2d_adaptive(&e, "x", "y", -1.0, 1.0, -1.0, 1.0, 81, 641);
        assert_eq!(s.n, 81);
        assert!(!s.undersampled);
        assert_eq!(s.heights.len(), 81 * 81);
    }

    #[test]
    fn adaptive_refines_oscillatory_surfaces() {
        // sin(x·y) over [-9, 9]² aliases badly at 81×81 (the local frequency
        // grows with |∇(xy)| = √(x²+y²), up to ~12.7 at the corners) but is
        // resolved within the cap — refined, and not flagged.
        let e = expr_of("sin(x*y)");
        let s = sample2d_adaptive(&e, "x", "y", -9.0, 9.0, -9.0, 9.0, 81, 641);
        assert!(s.n > 81, "expected refinement past the base grid");
        assert!(!s.undersampled);

        // ×5 the frequency exceeds what even the capped grid can certify:
        // the result must be flagged, never silently drawn.
        let e = expr_of("sin(5*x*y)");
        let s = sample2d_adaptive(&e, "x", "y", -9.0, 9.0, -9.0, 9.0, 81, 641);
        assert_eq!(s.n, 641);
        assert!(s.undersampled);
    }

    #[test]
    fn adaptive_skips_gaps_like_the_renderer_does() {
        // A pole line crosses the window; the comparisons that touch its
        // None samples are skipped, the rest converge.
        let e = expr_of("(x - y)^(-1)");
        let s = sample2d_adaptive(&e, "x", "y", -1.0, 1.0, -1.0, 1.0, 81, 641);
        assert!(s.heights.iter().any(|h| h.is_some()));
    }
}
