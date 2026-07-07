//! Numeric discriminant-analysis classifiers: `stats.lda` and `stats.qda`,
//! plus their consumers (classifier dispatch inside `stats.predict`, and
//! `stats.project` for the LDA dimensionality reduction).
//!
//! This module is floating-point *by design*, and every value it derives from
//! f64 arithmetic comes back as an `Expr::Float` — certainty "approximate" —
//! never as an exact rational. That is deliberate: at the dimensionalities
//! discriminant analysis is used for (images, spectra, dozens-to-hundreds of
//! features), an exact eigendecomposition is intractable, and for the
//! irreducible characteristic polynomials real data produces it is not even
//! expressible in radicals, so the exact engine could only ever refuse.
//! What *can* stay exact does: class labels, counts, and priors come from
//! counting, not from floats. Degenerate inputs — a singular covariance, a
//! single class, non-convergence — are refusals with a suggested fix, never
//! a NaN or a silently regularized answer.

// Dense linear-algebra kernels: index loops that mirror the textbook
// formulas beat iterator chains for auditability here, and audits are the
// point of this file.
#![allow(clippy::needless_range_loop)]

use crate::expr::*;
use crate::f64eval::eval_f64;
use crate::nlfit;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

// ---------------------------------------------------------------------------
// Input parsing
// ---------------------------------------------------------------------------

/// Design rows as f64, refusing entries beyond floating-point range: a value
/// that reads as ±∞ would silently poison every covariance downstream.
fn rows_f64(caller: &str, rows: &[Vec<Expr>]) -> Result<Vec<Vec<f64>>, String> {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|e| {
                    let v = eval_f64(e, &[]).map_err(|err| format!("{}: {}", caller, err))?;
                    if !v.is_finite() {
                        return Err(format!(
                            "{}: an entry's magnitude exceeds the f64 range this numeric \
                             method computes in",
                            caller
                        ));
                    }
                    Ok(v)
                })
                .collect()
        })
        .collect()
}

/// The distinct class labels (in first-appearance order), each row's class
/// index, and the per-class counts. Labels are compared structurally, so
/// numeric labels and categorical symbols (how imports spell text cells)
/// both work.
struct Grouping {
    classes: Vec<Expr>,
    class_of: Vec<usize>,
    counts: Vec<usize>,
}

fn group_labels(caller: &str, y: &[Expr]) -> Result<Grouping, String> {
    let mut classes: Vec<Expr> = Vec::new();
    let mut class_of = Vec::with_capacity(y.len());
    for label in y {
        if is_missing(label) {
            return Err(format!(
                "{}: the labels contain a missing value (NA) — data.dropna(...) removes \
                 those rows",
                caller
            ));
        }
        let idx = match classes.iter().position(|c| c == label) {
            Some(i) => i,
            None => {
                classes.push(label.clone());
                classes.len() - 1
            }
        };
        class_of.push(idx);
    }
    if classes.len() < 2 {
        return Err(format!(
            "{} needs at least two classes in the labels, got {}",
            caller,
            classes.len()
        ));
    }
    let mut counts = vec![0usize; classes.len()];
    for &c in &class_of {
        counts[c] += 1;
    }
    Ok(Grouping {
        classes,
        class_of,
        counts,
    })
}

/// The optional shrinkage argument λ ∈ [0, 1]: the fitted covariance becomes
/// (1−λ)·Σ + λ·(tr Σ / d)·I, which is what makes n < d problems (images)
/// estimable at all. Returns the f64 to compute with and the user's exact
/// expression to store on the model.
fn shrinkage(caller: &str, arg: Option<&Expr>) -> Result<(f64, Expr), String> {
    let Some(e) = arg else {
        return Ok((0.0, int(0)));
    };
    let v = eval_f64(e, &[]).map_err(|err| format!("{}: {}", caller, err))?;
    if !(0.0..=1.0).contains(&v) {
        return Err(format!(
            "{}: the shrinkage argument must be between 0 and 1, got {}",
            caller, e
        ));
    }
    Ok((v, e.clone()))
}

// ---------------------------------------------------------------------------
// Dense symmetric f64 kernels (Cholesky, Jacobi) — new numeric code, kept
// here because nothing exact can substitute for it
// ---------------------------------------------------------------------------

/// Lower-triangular Cholesky factor of a symmetric matrix, or `None` if it is
/// not numerically positive definite. The pivot must clear a threshold
/// *relative to its own diagonal entry*: a covariance that is singular in
/// exact arithmetic must read as singular here too, not factor through
/// roundoff noise into a garbage classifier.
fn cholesky(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let d = a.len();
    let mut l = vec![vec![0.0f64; d]; d];
    for i in 0..d {
        for j in 0..=i {
            let mut s = a[i][j];
            for t in 0..j {
                s -= l[i][t] * l[j][t];
            }
            if i == j {
                if !s.is_finite() || s <= a[i][i].max(0.0) * 1e-12 {
                    return None;
                }
                l[i][j] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }
    Some(l)
}

/// Solve L·y = b for lower-triangular L.
fn forward_solve(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let d = b.len();
    let mut y = vec![0.0f64; d];
    for i in 0..d {
        let mut s = b[i];
        for j in 0..i {
            s -= l[i][j] * y[j];
        }
        y[i] = s / l[i][i];
    }
    y
}

/// Solve Lᵀ·x = b for lower-triangular L.
fn back_solve(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let d = b.len();
    let mut x = vec![0.0f64; d];
    for i in (0..d).rev() {
        let mut s = b[i];
        for j in i + 1..d {
            s -= l[j][i] * x[j];
        }
        x[i] = s / l[i][i];
    }
    x
}

/// Solve (L·Lᵀ)·x = b.
fn chol_solve(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    back_solve(l, &forward_solve(l, b))
}

/// ln det(L·Lᵀ) = 2·Σ ln lᵢᵢ, without forming the (overflow-prone) product.
fn chol_logdet(l: &[Vec<f64>]) -> f64 {
    2.0 * l.iter().enumerate().map(|(i, r)| r[i].ln()).sum::<f64>()
}

/// (L·Lᵀ)⁻¹, column by column.
fn chol_inverse(l: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let d = l.len();
    let mut inv = vec![vec![0.0f64; d]; d];
    for j in 0..d {
        let mut e = vec![0.0f64; d];
        e[j] = 1.0;
        let col = chol_solve(l, &e);
        for i in 0..d {
            inv[i][j] = col[i];
        }
    }
    inv
}

/// Eigenvalues and eigenvectors of a symmetric matrix by cyclic Jacobi
/// rotations: eigenvalues descending, eigenvectors the columns of the second
/// return, orthonormal. `None` if the sweep cap is hit without the
/// off-diagonal mass dying — Jacobi converges quadratically, so that cap is
/// a refusal tripwire, not a tuning knob.
fn jacobi_eigh(mut a: Vec<Vec<f64>>) -> Option<(Vec<f64>, Vec<Vec<f64>>)> {
    let d = a.len();
    let mut v: Vec<Vec<f64>> = (0..d)
        .map(|i| (0..d).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    let fro2: f64 = a.iter().flatten().map(|x| x * x).sum();
    let tol = fro2 * 1e-28; // off-diagonal mass ≤ (1e-14)² of the matrix
    let mut converged = fro2 == 0.0;
    for _ in 0..100 {
        let off: f64 = (0..d)
            .flat_map(|p| (p + 1..d).map(move |q| (p, q)))
            .map(|(p, q)| a[p][q] * a[p][q])
            .sum();
        if off <= tol {
            converged = true;
            break;
        }
        for p in 0..d {
            for q in p + 1..d {
                let apq = a[p][q];
                if apq == 0.0 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for i in 0..d {
                    let (aip, aiq) = (a[i][p], a[i][q]);
                    a[i][p] = c * aip - s * aiq;
                    a[i][q] = s * aip + c * aiq;
                }
                for i in 0..d {
                    let (api, aqi) = (a[p][i], a[q][i]);
                    a[p][i] = c * api - s * aqi;
                    a[q][i] = s * api + c * aqi;
                }
                for i in 0..d {
                    let (vip, viq) = (v[i][p], v[i][q]);
                    v[i][p] = c * vip - s * viq;
                    v[i][q] = s * vip + c * viq;
                }
            }
        }
    }
    if !converged {
        return None;
    }
    let mut order: Vec<usize> = (0..d).collect();
    order.sort_by(|&i, &j| {
        a[j][j]
            .partial_cmp(&a[i][i])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let vals = order.iter().map(|&i| a[i][i]).collect();
    let vecs = (0..d)
        .map(|i| order.iter().map(|&j| v[i][j]).collect())
        .collect();
    Some((vals, vecs))
}

// ---------------------------------------------------------------------------
// Shared estimation pieces
// ---------------------------------------------------------------------------

/// Per-class means (k×d) of the data rows.
fn class_means(x: &[Vec<f64>], g: &Grouping, d: usize) -> Vec<Vec<f64>> {
    let k = g.classes.len();
    let mut means = vec![vec![0.0f64; d]; k];
    for (xi, &c) in x.iter().zip(&g.class_of) {
        for a in 0..d {
            means[c][a] += xi[a];
        }
    }
    for (m, &nc) in means.iter_mut().zip(&g.counts) {
        for e in m.iter_mut() {
            *e /= nc as f64;
        }
    }
    means
}

/// Apply shrinkage toward the spherical (tr Σ / d)·I in place.
fn shrink(sigma: &mut [Vec<f64>], lambda: f64) {
    if lambda == 0.0 {
        return;
    }
    let d = sigma.len();
    let mu = sigma.iter().enumerate().map(|(i, r)| r[i]).sum::<f64>() / d as f64;
    for (i, row) in sigma.iter_mut().enumerate() {
        for (j, e) in row.iter_mut().enumerate() {
            *e *= 1.0 - lambda;
            if i == j {
                *e += lambda * mu;
            }
        }
    }
}

/// Exact class priors n_c/n — these come from counting, so they are the one
/// part of a fitted classifier with no float in it.
fn exact_priors(counts: &[usize], n: usize) -> Vec<Expr> {
    counts
        .iter()
        .map(|&c| {
            rat_to_expr(BigRational::new(
                BigInt::from(c as u64),
                BigInt::from(n as u64),
            ))
        })
        .collect()
}

/// Pack an r×c block of f64s as a matrix of honest floats.
fn mat_expr(rows: &[Vec<f64>]) -> Result<Expr, String> {
    Ok(Expr::Matrix(
        rows.iter()
            .map(|r| nlfit::floats(r))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

/// Pack entries as an n×1 column vector.
fn col(v: Vec<Expr>) -> Expr {
    Expr::Matrix(v.into_iter().map(|e| vec![e]).collect())
}

// ---------------------------------------------------------------------------
// Fitting
// ---------------------------------------------------------------------------

/// `stats.lda(X, y[, shrinkage])`: linear discriminant analysis. One pooled
/// within-class covariance; linear decision scores x·aᶜ + bᶜ with
/// aᶜ = Σ⁻¹μᶜ and bᶜ = −½ μᶜᵀΣ⁻¹μᶜ + ln πᶜ; discriminant axes from the
/// generalized eigenproblem Σ_between·w = λ·Σ_within·w, reduced to a symmetric
/// one through the Cholesky factor and solved by Jacobi rotations.
pub(crate) fn lda(
    rows: &[Vec<Expr>],
    y: &[Expr],
    shrink_arg: Option<&Expr>,
) -> Result<Expr, String> {
    let caller = "stats.lda";
    let x = rows_f64(caller, rows)?;
    let g = group_labels(caller, y)?;
    let (n, d, k) = (x.len(), x[0].len(), g.classes.len());
    if n <= k {
        return Err(format!(
            "{} needs more observations ({}) than classes ({})",
            caller, n, k
        ));
    }
    let (lambda, shrink_expr) = shrinkage(caller, shrink_arg)?;
    let means = class_means(&x, &g, d);

    // Pooled within-class covariance (n−k denominator), then shrinkage.
    let mut sw = vec![vec![0.0f64; d]; d];
    for (xi, &c) in x.iter().zip(&g.class_of) {
        for a in 0..d {
            let da = xi[a] - means[c][a];
            for b in 0..d {
                sw[a][b] += da * (xi[b] - means[c][b]);
            }
        }
    }
    let df = (n - k) as f64;
    for row in sw.iter_mut() {
        for e in row.iter_mut() {
            *e /= df;
        }
    }
    shrink(&mut sw, lambda);
    let l = cholesky(&sw).ok_or_else(|| {
        format!(
            "{}: the pooled within-class covariance ({d}×{d}) is singular — this needs more \
             observations than features (plus classes) and no constant or collinear feature; \
             a shrinkage third argument regularizes it, e.g. stats.lda(X, y, 1/10)",
            caller
        )
    })?;

    // Linear decision scores.
    let mut coefs = Vec::with_capacity(k);
    let mut intercepts = Vec::with_capacity(k);
    for c in 0..k {
        let a = chol_solve(&l, &means[c]);
        let quad: f64 = means[c].iter().zip(&a).map(|(m, ai)| m * ai).sum();
        intercepts.push(-0.5 * quad + (g.counts[c] as f64 / n as f64).ln());
        coefs.push(a);
    }

    // Between-class scatter, whitened to the symmetric M = L⁻¹·Sb·L⁻ᵀ. The
    // eigenvalue *scale* depends on the df convention; the axes and the
    // explained ratios don't.
    let grand: Vec<f64> = (0..d)
        .map(|a| {
            (0..k)
                .map(|c| means[c][a] * g.counts[c] as f64)
                .sum::<f64>()
                / n as f64
        })
        .collect();
    let mut sb = vec![vec![0.0f64; d]; d];
    for c in 0..k {
        let w = g.counts[c] as f64 / df;
        for a in 0..d {
            let da = means[c][a] - grand[a];
            for b in 0..d {
                sb[a][b] += w * da * (means[c][b] - grand[b]);
            }
        }
    }
    let mut z = vec![vec![0.0f64; d]; d]; // Z = L⁻¹·Sb
    for j in 0..d {
        let cj = forward_solve(&l, &sb[j]); // Sb row j == column j (symmetric)
        for i in 0..d {
            z[i][j] = cj[i];
        }
    }
    let mut m = Vec::with_capacity(d); // M = Z·L⁻ᵀ, row i = (L⁻¹·zᵢ)ᵀ
    for i in 0..d {
        m.push(forward_solve(&l, &z[i]));
    }
    for i in 0..d {
        for j in 0..i {
            let s = 0.5 * (m[i][j] + m[j][i]);
            m[i][j] = s;
            m[j][i] = s;
        }
    }
    let (vals, vecs) = jacobi_eigh(m).ok_or_else(|| {
        format!(
            "{}: the eigendecomposition did not converge — refusing rather than returning \
             unreliable discriminant axes",
            caller
        )
    })?;

    // Back-transform the top min(d, k−1) axes; w = L⁻ᵀu, so wᵀ·Σ_within·w = 1
    // (projected data has unit within-class variance). Sign is fixed so the
    // largest-magnitude component is positive — eigenvector signs are
    // arbitrary and a printed model should be deterministic.
    let r = d.min(k - 1);
    let mut scalings = vec![vec![0.0f64; r]; d];
    for j in 0..r {
        let u: Vec<f64> = (0..d).map(|i| vecs[i][j]).collect();
        let mut w = back_solve(&l, &u);
        let lead = (0..d)
            .max_by(|&a, &b| {
                w[a].abs()
                    .partial_cmp(&w[b].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        if w[lead] < 0.0 {
            for e in w.iter_mut() {
                *e = -*e;
            }
        }
        for i in 0..d {
            scalings[i][j] = w[i];
        }
    }
    let clamped: Vec<f64> = vals.iter().map(|&v| v.max(0.0)).collect();
    let total: f64 = clamped.iter().sum();
    let explained: Vec<f64> = clamped[..r]
        .iter()
        .map(|&v| if total > 0.0 { v / total } else { 0.0 })
        .collect();

    structure(vec![
        ("kind".into(), Expr::Symbol("lda".into())),
        ("classes".into(), col(g.classes.clone())),
        (
            "counts".into(),
            col(g.counts.iter().map(|&c| int(c as i64)).collect()),
        ),
        ("priors".into(), col(exact_priors(&g.counts, n))),
        ("means".into(), mat_expr(&means)?),
        ("center".into(), mat_expr(&[grand])?),
        ("coefficients".into(), mat_expr(&coefs)?),
        ("intercepts".into(), col(nlfit::floats(&intercepts)?)),
        ("scalings".into(), mat_expr(&scalings)?),
        ("explained".into(), col(nlfit::floats(&explained)?)),
        ("shrinkage".into(), shrink_expr),
        ("n".into(), int(n as i64)),
        ("d".into(), int(d as i64)),
        ("k".into(), int(k as i64)),
    ])
}

/// `stats.qda(X, y[, shrinkage])`: quadratic discriminant analysis — one
/// covariance per class, so the decision boundaries are quadrics. The model
/// stores each class's precision (inverse covariance) stacked into one
/// (k·d)×d matrix, class c owning rows c·d+1 … (c+1)·d, plus the log-dets the
/// scores need.
pub(crate) fn qda(
    rows: &[Vec<Expr>],
    y: &[Expr],
    shrink_arg: Option<&Expr>,
) -> Result<Expr, String> {
    let caller = "stats.qda";
    let x = rows_f64(caller, rows)?;
    let g = group_labels(caller, y)?;
    let (n, d, k) = (x.len(), x[0].len(), g.classes.len());
    let (lambda, shrink_expr) = shrinkage(caller, shrink_arg)?;
    let means = class_means(&x, &g, d);

    let mut precisions = Vec::with_capacity(k * d);
    let mut logdets = Vec::with_capacity(k);
    for c in 0..k {
        if g.counts[c] < 2 {
            return Err(format!(
                "{}: class {} has only 1 observation — QDA estimates a covariance per class, \
                 which takes at least 2 (and realistically more than the {} features)",
                caller, g.classes[c], d
            ));
        }
        let mut cov = vec![vec![0.0f64; d]; d];
        for (xi, &ci) in x.iter().zip(&g.class_of) {
            if ci != c {
                continue;
            }
            for a in 0..d {
                let da = xi[a] - means[c][a];
                for b in 0..d {
                    cov[a][b] += da * (xi[b] - means[c][b]);
                }
            }
        }
        let dfc = (g.counts[c] - 1) as f64;
        for row in cov.iter_mut() {
            for e in row.iter_mut() {
                *e /= dfc;
            }
        }
        shrink(&mut cov, lambda);
        let l = cholesky(&cov).ok_or_else(|| {
            format!(
                "{}: the covariance of class {} is singular ({} observations for {} features) \
                 — that class needs more data, fewer features, or a shrinkage third argument, \
                 e.g. stats.qda(X, y, 1/10)",
                caller, g.classes[c], g.counts[c], d
            )
        })?;
        logdets.push(chol_logdet(&l));
        precisions.extend(chol_inverse(&l));
    }

    structure(vec![
        ("kind".into(), Expr::Symbol("qda".into())),
        ("classes".into(), col(g.classes.clone())),
        (
            "counts".into(),
            col(g.counts.iter().map(|&c| int(c as i64)).collect()),
        ),
        ("priors".into(), col(exact_priors(&g.counts, n))),
        ("means".into(), mat_expr(&means)?),
        ("precisions".into(), mat_expr(&precisions)?),
        ("logdets".into(), col(nlfit::floats(&logdets)?)),
        ("shrinkage".into(), shrink_expr),
        ("n".into(), int(n as i64)),
        ("d".into(), int(d as i64)),
        ("k".into(), int(k as i64)),
    ])
}

// ---------------------------------------------------------------------------
// Consuming a fitted model
// ---------------------------------------------------------------------------

/// Is this struct a classifier model this module fitted? (`stats.predict`
/// branches here before its regression path.)
pub(crate) fn is_classifier(model: &Expr) -> bool {
    let Expr::Struct(fields) = model else {
        return false;
    };
    fields
        .iter()
        .any(|(name, v)| name == "kind" && matches!(v, Expr::Symbol(s) if s == "lda" || s == "qda"))
}

fn field<'a>(model: &'a Expr, fname: &str, caller: &str) -> Result<&'a Expr, String> {
    let Expr::Struct(fields) = model else {
        return Err(format!("{}: expected a classifier model struct", caller));
    };
    fields
        .iter()
        .find(|(n, _)| n == fname)
        .map(|(_, v)| v)
        .ok_or_else(|| {
            format!(
                "{}: the model has no field '{}' (is it from stats.lda or stats.qda?)",
                caller, fname
            )
        })
}

fn field_usize(model: &Expr, fname: &str, caller: &str) -> Result<usize, String> {
    numeric_value(field(model, fname, caller)?)
        .and_then(|r| r.to_integer().to_usize())
        .ok_or_else(|| format!("{}: the model field '{}' is not a count", caller, fname))
}

fn field_matrix_f64(model: &Expr, fname: &str, caller: &str) -> Result<Vec<Vec<f64>>, String> {
    let Expr::Matrix(rows) = field(model, fname, caller)? else {
        return Err(format!(
            "{}: the model field '{}' is not a matrix",
            caller, fname
        ));
    };
    rows_f64(caller, rows)
}

fn field_column_f64(model: &Expr, fname: &str, caller: &str) -> Result<Vec<f64>, String> {
    Ok(field_matrix_f64(model, fname, caller)?
        .into_iter()
        .flatten()
        .collect())
}

/// The entries of an n×1 (or 1×n) matrix field, e.g. the class labels.
fn field_column(model: &Expr, fname: &str, caller: &str) -> Result<Vec<Expr>, String> {
    match field(model, fname, caller)? {
        Expr::Matrix(rows) if rows.len() == 1 => Ok(rows[0].clone()),
        Expr::Matrix(rows) => Ok(rows.iter().map(|r| r[0].clone()).collect()),
        _ => Err(format!(
            "{}: the model field '{}' is not a vector",
            caller, fname
        )),
    }
}

/// New observations as m rows of d features: an m×d matrix, or for a
/// single-feature model any vector.
fn new_rows(caller: &str, x: &Expr, d: usize) -> Result<Vec<Vec<Expr>>, String> {
    let Expr::Matrix(rows) = x else {
        return Err(format!(
            "{}: expected a matrix or vector of new rows",
            caller
        ));
    };
    if d == 1 {
        if rows.len() == 1 {
            return Ok(rows[0].iter().map(|e| vec![e.clone()]).collect());
        }
        if rows.iter().all(|r| r.len() == 1) {
            return Ok(rows.clone());
        }
        return Err(format!(
            "{}: this model has 1 feature; pass a vector of new values",
            caller
        ));
    }
    if rows.iter().all(|r| r.len() == d) {
        return Ok(rows.clone());
    }
    Err(format!(
        "{}: this model has {} features, so each new row needs {} values",
        caller, d, d
    ))
}

/// Classify new rows with a fitted LDA/QDA model: a struct of `labels` (the
/// argmax class per row) and `posterior` (the m×k softmax of the discriminant
/// scores — floats, because the whole decision is). Reached through
/// `stats.predict(model, Xnew)`.
pub(crate) fn classify(model: &Expr, args: &[Expr]) -> Result<Expr, String> {
    let caller = "stats.predict";
    if args.len() != 2 {
        return Err(format!(
            "{}: a classifier model takes exactly one argument of new rows (no confidence level)",
            caller
        ));
    }
    let d = field_usize(model, "d", caller)?;
    let classes = field_column(model, "classes", caller)?;
    let k = classes.len();
    let x = rows_f64(caller, &new_rows(caller, &args[1], d)?)?;

    let lda_model = matches!(field(model, "kind", caller)?, Expr::Symbol(s) if s == "lda");
    let scores: Vec<Vec<f64>> = if lda_model {
        let coefs = field_matrix_f64(model, "coefficients", caller)?;
        let intercepts = field_column_f64(model, "intercepts", caller)?;
        x.iter()
            .map(|xi| {
                (0..k)
                    .map(|c| {
                        xi.iter().zip(&coefs[c]).map(|(a, b)| a * b).sum::<f64>() + intercepts[c]
                    })
                    .collect()
            })
            .collect()
    } else {
        let means = field_matrix_f64(model, "means", caller)?;
        let precisions = field_matrix_f64(model, "precisions", caller)?;
        let logdets = field_column_f64(model, "logdets", caller)?;
        let priors = field_column_f64(model, "priors", caller)?;
        x.iter()
            .map(|xi| {
                (0..k)
                    .map(|c| {
                        let p = &precisions[c * d..(c + 1) * d];
                        let dx: Vec<f64> = xi.iter().zip(&means[c]).map(|(a, m)| a - m).collect();
                        let quad: f64 = (0..d)
                            .map(|a| dx[a] * (0..d).map(|b| p[a][b] * dx[b]).sum::<f64>())
                            .sum();
                        priors[c].ln() - 0.5 * logdets[c] - 0.5 * quad
                    })
                    .collect()
            })
            .collect()
    };

    let mut labels = Vec::with_capacity(scores.len());
    let mut posterior = Vec::with_capacity(scores.len());
    for s in &scores {
        let best = (0..k)
            .max_by(|&a, &b| s[a].partial_cmp(&s[b]).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0);
        labels.push(classes[best].clone());
        let top = s[best];
        let expd: Vec<f64> = s.iter().map(|&v| (v - top).exp()).collect();
        let sum: f64 = expd.iter().sum();
        posterior.push(expd.into_iter().map(|e| e / sum).collect::<Vec<f64>>());
    }
    structure(vec![
        ("labels".into(), col(labels)),
        ("posterior".into(), mat_expr(&posterior)?),
    ])
}

/// `stats.project(model, X)`: rows onto an LDA model's discriminant axes —
/// the numeric answer to "LDA as dimensionality reduction". Returns an m×r
/// float matrix, r = min(d, k−1); within-class variance is 1 on every axis.
pub(crate) fn project(args: &[Expr]) -> Result<Expr, String> {
    let caller = "stats.project";
    if args.len() != 2 {
        return Err(format!(
            "{} expects 2 argument(s) (model, X), got {}",
            caller,
            args.len()
        ));
    }
    let model = &args[0];
    match field(model, "kind", caller)? {
        Expr::Symbol(s) if s == "lda" => {}
        Expr::Symbol(s) if s == "qda" => {
            return Err(format!(
                "{} needs an LDA model — QDA has no shared discriminant axes to project onto",
                caller
            ));
        }
        _ => return Err(format!("{} expects a model struct from stats.lda", caller)),
    }
    let d = field_usize(model, "d", caller)?;
    let center = field_column_f64(model, "center", caller)?;
    let scalings = field_matrix_f64(model, "scalings", caller)?;
    let r = scalings[0].len();
    let x = rows_f64(caller, &new_rows(caller, &args[1], d)?)?;
    let out: Vec<Vec<f64>> = x
        .iter()
        .map(|xi| {
            (0..r)
                .map(|j| {
                    (0..d)
                        .map(|a| (xi[a] - center[a]) * scalings[a][j])
                        .sum::<f64>()
                })
                .collect()
        })
        .collect();
    mat_expr(&out)
}
