//! Nonlinear least squares by Levenberg–Marquardt, built on an **exact
//! symbolic Jacobian**. The model is an ordinary surd expression in an
//! independent variable and some parameters; we differentiate it analytically
//! with respect to each parameter ([`differentiate`]) — the real derivative,
//! not a finite difference or an autodiff graph — and evaluate that Jacobian
//! numerically inside the iteration.
//!
//! The fit *itself* is necessarily numeric (it iterates), so the reported
//! estimates are floats. But the derivatives the steps are built from are
//! correct, which is exactly where finite-difference fitters lose accuracy and
//! robustness near a flat or stiff objective. The returned struct also carries
//! the Jacobian in symbolic form, so you can see the exact ∂f/∂θ it used.

use crate::expr::*;
use crate::f64eval::eval_f64;
use num_traits::FromPrimitive;

const MAX_ITERS: usize = 200;
const TOL: f64 = 1e-12;
/// Reported floats are f64-derived, so showing more than ~15 significant
/// digits would be inventing precision the fit doesn't have.
const RESULT_DIGITS: usize = 15;

/// Free symbols of an expression, in first-seen order. Used by the caller to
/// pick out the independent variable (everything that isn't a parameter).
pub fn free_symbols(e: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    collect_symbols(e, &mut out);
    out
}

fn collect_symbols(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Symbol(s) => {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        Expr::Add(ts) | Expr::Mul(ts) | Expr::Func(_, ts) => {
            ts.iter().for_each(|t| collect_symbols(t, out))
        }
        Expr::Pow(b, x) => {
            collect_symbols(b, out);
            collect_symbols(x, out);
        }
        Expr::Complex(a, b) | Expr::Equation(a, b) => {
            collect_symbols(a, out);
            collect_symbols(b, out);
        }
        Expr::Matrix(rows) => rows.iter().flatten().for_each(|t| collect_symbols(t, out)),
        _ => {}
    }
}

/// Fit `model` (in independent variable `xvar` and the named `params`) to the
/// data `(x, y)`, starting from `init`. Returns a fitted-model struct.
pub fn fit(
    model: &Expr,
    params: &[String],
    xvar: &str,
    x: &[f64],
    y: &[f64],
    init: &[f64],
) -> Result<Expr, String> {
    let n = x.len();
    let p = params.len();
    if y.len() != n {
        return Err(format!(
            "stats.nlfit: x and y differ in length ({} vs {})",
            n,
            y.len()
        ));
    }
    if init.len() != p {
        return Err(format!(
            "stats.nlfit: {} initial value(s) for {} parameter(s)",
            init.len(),
            p
        ));
    }
    if n <= p {
        return Err(format!(
            "stats.nlfit needs more data points ({}) than parameters ({})",
            n, p
        ));
    }

    // The Jacobian columns: one exact analytic derivative per parameter.
    let jac: Vec<Expr> = params.iter().map(|pj| differentiate(model, pj)).collect();

    let mut theta = init.to_vec();
    let mut cost = sse(model, xvar, params, &theta, x, y)?;
    if !cost.is_finite() {
        return Err(
            "stats.nlfit: the model is undefined at the initial parameters (try other initial values)"
                .into(),
        );
    }
    let mut lambda = 1e-3_f64;
    let mut converged = false;
    let mut iters = 0usize;

    while iters < MAX_ITERS {
        iters += 1;
        let (jm, r) = jac_resid(model, &jac, xvar, params, &theta, x, y)?;
        // Gauss–Newton normal-equation pieces: H = JᵀJ, g = Jᵀr.
        let mut h = vec![vec![0.0_f64; p]; p];
        let mut grad = vec![0.0_f64; p];
        for i in 0..n {
            for a in 0..p {
                grad[a] += jm[i][a] * r[i];
                for b in 0..p {
                    h[a][b] += jm[i][a] * jm[i][b];
                }
            }
        }
        // Damped step; grow λ (toward gradient descent) until the cost drops.
        let mut accepted = false;
        for _ in 0..40 {
            let mut hd = h.clone();
            for a in 0..p {
                hd[a][a] += lambda * h[a][a].max(1e-30);
            }
            let Some(delta) = solve_linear(&hd, &grad) else {
                lambda *= 10.0;
                if lambda > 1e15 {
                    return Err("stats.nlfit: the Jacobian is singular (parameters not \
                                identifiable, or try other initial values)"
                        .into());
                }
                continue;
            };
            let cand: Vec<f64> = theta.iter().zip(&delta).map(|(t, d)| t + d).collect();
            let new_cost = sse(model, xvar, params, &cand, x, y)?;
            if new_cost.is_finite() && new_cost < cost {
                let rel = (cost - new_cost) / cost.max(1e-300);
                let step = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
                let scale = theta.iter().map(|t| t.abs()).sum::<f64>() + TOL;
                theta = cand;
                cost = new_cost;
                lambda = (lambda * 0.4).max(1e-30);
                accepted = true;
                if rel < TOL || step < TOL * scale {
                    converged = true;
                }
                break;
            }
            lambda *= 10.0;
            if lambda > 1e15 {
                break;
            }
        }
        // No damped step reduced the cost → we are at a (local) minimum.
        if !accepted {
            converged = true;
            break;
        }
        if converged {
            break;
        }
    }

    finish(
        model, &jac, params, xvar, &theta, x, y, cost, iters, converged,
    )
}

/// Assemble the result struct: estimates, asymptotic inference (from the
/// linearized covariance σ̂²·(JᵀJ)⁻¹ at the solution), residuals, and the
/// symbolic Jacobian.
#[allow(clippy::too_many_arguments)]
fn finish(
    model: &Expr,
    jac: &[Expr],
    params: &[String],
    xvar: &str,
    theta: &[f64],
    x: &[f64],
    y: &[f64],
    rss: f64,
    iters: usize,
    converged: bool,
) -> Result<Expr, String> {
    let (n, p) = (x.len(), params.len());
    let (jm, resid) = jac_resid(model, jac, xvar, params, theta, x, y)?;
    let mut h = vec![vec![0.0_f64; p]; p];
    for row in &jm {
        for a in 0..p {
            for b in 0..p {
                h[a][b] += row[a] * row[b];
            }
        }
    }
    let sigma2 = rss / (n - p) as f64;
    let hinv = inverse(&h).ok_or(
        "stats.nlfit: the Jacobian is singular at the solution (parameters not identifiable)",
    )?;
    let df = (n - p) as i64;

    let coeffs = floats(theta)?;
    let mut se = Vec::with_capacity(p);
    let mut tstat = Vec::with_capacity(p);
    let mut pvalue = Vec::with_capacity(p);
    for (j, &t) in theta.iter().enumerate() {
        let s = (sigma2 * hinv[j][j]).max(0.0).sqrt();
        se.push(float_expr(s)?);
        // A perfect fit makes se = 0 (the parameter is pinned exactly); report
        // a saturated t rather than an infinity surd can't hold — its p-value
        // then evaluates to 0, which is the right limit.
        let t_ratio = if s > 0.0 {
            t / s
        } else if t == 0.0 {
            0.0
        } else {
            t.signum() * 1e308
        };
        let tv = float_expr(t_ratio)?;
        pvalue.push(mul(vec![
            int(2),
            add(vec![
                int(1),
                mul(vec![
                    int(-1),
                    func("tcdf", vec![func("abs", vec![tv.clone()]), int(df)]),
                ]),
            ]),
        ]));
        tstat.push(tv);
    }

    structure(vec![
        ("coefficients".into(), col(coeffs)),
        ("se".into(), col(se)),
        ("tstat".into(), col(tstat)),
        ("pvalue".into(), col(pvalue)),
        ("residuals".into(), col(floats(&resid)?)),
        ("rss".into(), float_expr(rss)?),
        ("sigma2".into(), float_expr(sigma2)?),
        // The exact analytic derivatives the fit was built on.
        ("jacobian".into(), col(jac.to_vec())),
        ("iterations".into(), int(iters as i64)),
        ("converged".into(), Expr::Bool(converged)),
    ])
}

// -- objective and Jacobian ---------------------------------------------------

/// Variable bindings (parameters then the independent variable) for `eval_f64`.
fn bindings<'a>(
    params: &'a [String],
    theta: &[f64],
    xvar: &'a str,
    xi: f64,
) -> Vec<(&'a str, f64)> {
    let mut v: Vec<(&str, f64)> = params
        .iter()
        .map(String::as_str)
        .zip(theta.iter().copied())
        .collect();
    v.push((xvar, xi));
    v
}

/// Residual sum of squares Σ(yᵢ − f(xᵢ; θ))².
fn sse(
    model: &Expr,
    xvar: &str,
    params: &[String],
    theta: &[f64],
    x: &[f64],
    y: &[f64],
) -> Result<f64, String> {
    let mut s = 0.0;
    for (&xi, &yi) in x.iter().zip(y) {
        let r = yi - eval_f64(model, &bindings(params, theta, xvar, xi))?;
        s += r * r;
    }
    Ok(s)
}

/// The Jacobian matrix J[i][j] = ∂f/∂θⱼ at (xᵢ, θ) and residuals rᵢ.
fn jac_resid(
    model: &Expr,
    jac: &[Expr],
    xvar: &str,
    params: &[String],
    theta: &[f64],
    x: &[f64],
    y: &[f64],
) -> Result<(Vec<Vec<f64>>, Vec<f64>), String> {
    let (n, p) = (x.len(), jac.len());
    let mut jm = vec![vec![0.0; p]; n];
    let mut r = vec![0.0; n];
    for i in 0..n {
        let b = bindings(params, theta, xvar, x[i]);
        r[i] = y[i] - eval_f64(model, &b)?;
        for (j, jcol) in jac.iter().enumerate() {
            jm[i][j] = eval_f64(jcol, &b)?;
        }
    }
    Ok((jm, r))
}

// -- small dense f64 linear algebra (p is the parameter count, tiny) ----------

/// Solve `A x = b` by Gauss–Jordan with partial pivoting. `None` if singular.
pub(crate) fn solve_linear(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = a.len();
    let mut m = a.to_vec();
    let mut x = b.to_vec();
    for col in 0..n {
        let piv = (col..n).max_by(|&r, &s| {
            m[r][col]
                .abs()
                .partial_cmp(&m[s][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if m[piv][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, piv);
        x.swap(col, piv);
        for r in 0..n {
            if r != col {
                let f = m[r][col] / m[col][col];
                for c in col..n {
                    m[r][c] -= f * m[col][c];
                }
                x[r] -= f * x[col];
            }
        }
    }
    Some((0..n).map(|i| x[i] / m[i][i]).collect())
}

/// Invert a square matrix by Gauss–Jordan. `None` if singular.
pub(crate) fn inverse(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut m = a.to_vec();
    let mut inv: Vec<Vec<f64>> = (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    for col in 0..n {
        let piv = (col..n).max_by(|&r, &s| {
            m[r][col]
                .abs()
                .partial_cmp(&m[s][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if m[piv][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, piv);
        inv.swap(col, piv);
        let d = m[col][col];
        for c in 0..n {
            m[col][c] /= d;
            inv[col][c] /= d;
        }
        for r in 0..n {
            if r != col {
                let f = m[r][col];
                for c in 0..n {
                    m[r][c] -= f * m[col][c];
                    inv[r][c] -= f * inv[col][c];
                }
            }
        }
    }
    Some(inv)
}

// -- result construction ------------------------------------------------------

/// An f64 as an exact-decimal float `Expr`, rounded to honest f64 precision.
pub(crate) fn float_expr(v: f64) -> Result<Expr, String> {
    let r = BigRational::from_f64(v)
        .ok_or_else(|| "stats.nlfit produced a non-finite value".to_string())?;
    numeric_eval(&rat_to_expr(r), RESULT_DIGITS)
}

pub(crate) fn floats(vs: &[f64]) -> Result<Vec<Expr>, String> {
    vs.iter().map(|&v| float_expr(v)).collect()
}

/// Pack entries as an n×1 column vector.
fn col(v: Vec<Expr>) -> Expr {
    Expr::Matrix(v.into_iter().map(|e| vec![e]).collect())
}
