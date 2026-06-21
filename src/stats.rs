//! The `stats` built-in namespace: exact statistics.
//!
//! Every estimator runs in exact arithmetic: the mean of rationals is a
//! rational, a variance is a rational, and a standard deviation is an exact
//! surd — `stats.std([1; 2; 3; 4])` is `sqrt(5/3)`, with `N(...)` taking it
//! to floats only on request. `var`, `std`, `cov`, and `cor` are the
//! *sample* estimators (n−1 denominator). Symbolic entries flow through
//! everything that doesn't need ordering; `median` requires numeric data.

use crate::expr::*;
use crate::f64eval::eval_f64;
use crate::matrix;
use crate::nlfit;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &[
    "mean", "median", "quantile", "var", "std", "cov", "cor", "linfit", "polyfit", "polyval",
    "lsq", "regress", "wls", "ridge", "logit", "predict", "robustse", "anova", "bptest", "dwtest",
    "jbtest", "nlfit", "rmse", "r2", "normcdf", "normpdf", "norminv", "tcdf", "tpdf", "tinv",
    "chisqcdf", "chisqpdf", "chisqinv", "fcdf", "fpdf", "finv",
];

pub fn call(name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    match name {
        "mean" => {
            let xs = one_vector("stats.mean", &args)?;
            Ok(mean_of(&xs))
        }
        "median" => median(&one_vector("stats.median", &args)?),
        "quantile" => {
            if args.len() != 2 {
                return Err(format!(
                    "stats.quantile expects 2 argument(s), got {}",
                    args.len()
                ));
            }
            quantile(&entries("stats.quantile", &args[0])?, &args[1])
        }
        "rmse" => {
            let (a, b) = two_vectors("stats.rmse", &args)?;
            rmse(&a, &b)
        }
        "r2" => {
            let (y, yhat) = two_vectors("stats.r2", &args)?;
            r_squared(&y, &yhat)
        }
        "polyfit" => {
            if args.len() != 3 {
                return Err(format!(
                    "stats.polyfit expects 3 argument(s), got {}",
                    args.len()
                ));
            }
            let x = entries("stats.polyfit", &args[0])?;
            let y = entries("stats.polyfit", &args[1])?;
            polyfit(&x, &y, &args[2])
        }
        "polyval" => {
            if args.len() != 2 {
                return Err(format!(
                    "stats.polyval expects 2 argument(s), got {}",
                    args.len()
                ));
            }
            polyval(&entries("stats.polyval", &args[0])?, &args[1])
        }
        "lsq" => {
            if args.len() != 2 {
                return Err(format!(
                    "stats.lsq expects 2 argument(s), got {}",
                    args.len()
                ));
            }
            lsq(&args[0], &args[1])
        }
        "var" => variance(&one_vector("stats.var", &args)?, "stats.var"),
        "std" => {
            let v = variance(&one_vector("stats.std", &args)?, "stats.std")?;
            Ok(pow(v, half()))
        }
        "cov" => {
            let (a, b) = two_vectors("stats.cov", &args)?;
            covariance(&a, &b)
        }
        "cor" => {
            let (a, b) = two_vectors("stats.cor", &args)?;
            correlation(&a, &b)
        }
        "linfit" => {
            let (x, y) = two_vectors("stats.linfit", &args)?;
            linfit(&x, &y)
        }
        "regress" => regress(&args),
        "wls" => wls(&args),
        "ridge" => ridge(&args),
        "logit" => logit(&args),
        "predict" => predict(&args),
        "robustse" => robustse(&args),
        "anova" => anova(&args),
        "bptest" => bptest(&args),
        "dwtest" => dwtest(&args),
        "jbtest" => jbtest(&args),
        // Distributions are symbolic until N(...): an arity check here, the
        // arbitrary-precision evaluation in `crate::special`.
        "normcdf" | "normpdf" | "norminv" => dist(name, args, &[1, 3]),
        "tcdf" | "tpdf" | "tinv" | "chisqcdf" | "chisqpdf" | "chisqinv" => dist(name, args, &[2]),
        "fcdf" | "fpdf" | "finv" => dist(name, args, &[3]),
        _ => Err(format!(
            "unknown function 'stats.{}' (available: stats.{})",
            name,
            FUNCTIONS.join(", stats.")
        )),
    }
}

// -- estimators ---------------------------------------------------------------

fn mean_of(xs: &[Expr]) -> Expr {
    mul(vec![inv_int(xs.len()), add(xs.to_vec())])
}

/// Entries shifted by their mean, the common core of var/cov/linfit.
fn centered(xs: &[Expr]) -> Vec<Expr> {
    let m = mean_of(xs);
    xs.iter()
        .map(|x| add(vec![x.clone(), mul(vec![int(-1), m.clone()])]))
        .collect()
}

/// Σ aᵢ·bᵢ, each product expanded so symbolic centered terms tidy up.
fn sum_products(a: &[Expr], b: &[Expr]) -> Expr {
    add(a
        .iter()
        .zip(b)
        .map(|(x, y)| expand(&mul(vec![x.clone(), y.clone()])))
        .collect())
}

/// Sample variance, n−1 denominator.
fn variance(xs: &[Expr], name: &str) -> Result<Expr, String> {
    if xs.len() < 2 {
        return Err(format!("{} expects at least 2 data points", name));
    }
    let c = centered(xs);
    Ok(mul(vec![inv_int(xs.len() - 1), sum_products(&c, &c)]))
}

fn covariance(a: &[Expr], b: &[Expr]) -> Result<Expr, String> {
    if a.len() < 2 {
        return Err("stats.cov expects at least 2 data points".into());
    }
    Ok(mul(vec![
        inv_int(a.len() - 1),
        sum_products(&centered(a), &centered(b)),
    ]))
}

/// Pearson correlation: cov(a,b) / (std(a)·std(b)). For numeric data the
/// variances are nonnegative rationals, so the radicals merge and the result
/// is an exact surd (±1 exactly for perfectly linear data).
fn correlation(a: &[Expr], b: &[Expr]) -> Result<Expr, String> {
    let va = variance(a, "stats.cor")?;
    let vb = variance(b, "stats.cor")?;
    if is_known_zero(&va) || is_known_zero(&vb) {
        return Err("stats.cor is undefined for zero-variance data".into());
    }
    let cov = covariance(a, b)?;
    Ok(mul(vec![cov, pow(va, neg_half()), pow(vb, neg_half())]))
}

/// Exact least-squares line y = intercept + slope·x, as a struct.
fn linfit(x: &[Expr], y: &[Expr]) -> Result<Expr, String> {
    if x.len() < 2 {
        return Err("stats.linfit expects at least 2 data points".into());
    }
    let cx = centered(x);
    let sxx = sum_products(&cx, &cx);
    if is_known_zero(&sxx) {
        return Err("stats.linfit needs at least two distinct x values".into());
    }
    let sxy = sum_products(&cx, &centered(y));
    let slope = mul(vec![sxy, pow(sxx, int(-1))]);
    let intercept = add(vec![
        mean_of(y),
        mul(vec![int(-1), slope.clone(), mean_of(x)]),
    ]);
    // The fitted line as a real function `x ↦ intercept + slope·x`, so it can
    // be evaluated (`m.predict(2.5)`) and plotted (`plot(m.predict, x, a, b)`).
    let predict = crate::eval::function_from_expr(
        "x",
        &add(vec![
            intercept.clone(),
            mul(vec![slope.clone(), Expr::Symbol("x".to_string())]),
        ]),
    )?;
    structure(vec![
        ("intercept".to_string(), intercept),
        ("slope".to_string(), slope),
        ("predict".to_string(), predict),
    ])
}

/// The middle value by exact ordering; the mean of the two middle values for
/// even n. Ordering is undecidable for symbolic entries, so those error.
fn median(xs: &[Expr]) -> Result<Expr, String> {
    let sorted = sorted_numeric("stats.median", xs)?;
    let n = sorted.len();
    Ok(if n % 2 == 1 {
        sorted[n / 2].clone()
    } else {
        mul(vec![
            inv_int(2),
            add(vec![sorted[n / 2 - 1].clone(), sorted[n / 2].clone()]),
        ])
    })
}

/// The q-th quantile (0 ≤ q ≤ 1), by linear interpolation between order
/// statistics (the R type-7 / NumPy default) — exact, since the
/// interpolation weight (n−1)·q is an exact rational.
fn quantile(xs: &[Expr], q: &Expr) -> Result<Expr, String> {
    let q = numeric_value(q)
        .filter(|q| {
            *q >= BigRational::from_integer(0.into()) && *q <= BigRational::from_integer(1.into())
        })
        .ok_or("stats.quantile expects a rational q with 0 <= q <= 1")?;
    let sorted = sorted_numeric("stats.quantile", xs)?;
    let n = sorted.len();
    // h = (n−1)·q splits into an index and an exact fractional weight.
    let h = q * BigRational::from_integer(BigInt::from(n as i64 - 1));
    let lo = h.floor();
    let frac = &h - &lo;
    let lo = lo.to_integer().to_usize().expect("0 <= lo < n");
    if frac == BigRational::from_integer(0.into()) {
        return Ok(sorted[lo].clone());
    }
    // x_lo + frac·(x_{lo+1} − x_lo)
    Ok(add(vec![
        sorted[lo].clone(),
        mul(vec![
            rat_to_expr(frac),
            add(vec![
                sorted[lo + 1].clone(),
                mul(vec![int(-1), sorted[lo].clone()]),
            ]),
        ]),
    ]))
}

/// Root mean squared error: √(Σ(aᵢ−bᵢ)²/n) — an exact surd.
fn rmse(a: &[Expr], b: &[Expr]) -> Result<Expr, String> {
    let d: Vec<Expr> = a
        .iter()
        .zip(b)
        .map(|(x, y)| add(vec![x.clone(), mul(vec![int(-1), y.clone()])]))
        .collect();
    let mean_sq = mul(vec![inv_int(a.len()), sum_products(&d, &d)]);
    Ok(pow(mean_sq, half()))
}

/// Coefficient of determination R² = 1 − SSres/SStot for observations `y`
/// against predictions `yhat`. Exactly 1 for a perfect fit.
fn r_squared(y: &[Expr], yhat: &[Expr]) -> Result<Expr, String> {
    let res: Vec<Expr> = y
        .iter()
        .zip(yhat)
        .map(|(a, b)| add(vec![a.clone(), mul(vec![int(-1), b.clone()])]))
        .collect();
    let cy = centered(y);
    let ss_tot = sum_products(&cy, &cy);
    if is_known_zero(&ss_tot) {
        return Err("stats.r2 is undefined for constant observations (zero variance)".into());
    }
    let ss_res = sum_products(&res, &res);
    Ok(add(vec![
        int(1),
        mul(vec![int(-1), ss_res, pow(ss_tot, int(-1))]),
    ]))
}

/// Exact least-squares polynomial of degree `deg`: build the Vandermonde
/// matrix and solve the normal equations with exact elimination
/// (conditioning is a float problem — there is no rounding here).
/// Coefficients come back as a column vector, constant term first.
fn polyfit(x: &[Expr], y: &[Expr], deg: &Expr) -> Result<Expr, String> {
    let deg = numeric_value(deg)
        .filter(|d| d.is_integer())
        .and_then(|d| d.to_integer().to_usize())
        .filter(|&d| d <= 100)
        .ok_or("stats.polyfit expects a degree between 0 and 100")?;
    if x.len() != y.len() {
        return Err(format!(
            "stats.polyfit expects x and y of the same length, got {} and {}",
            x.len(),
            y.len()
        ));
    }
    if x.len() < deg + 1 {
        return Err(format!(
            "stats.polyfit needs at least {} points for degree {}, got {}",
            deg + 1,
            deg,
            x.len()
        ));
    }
    let vandermonde = Expr::Matrix(
        x.iter()
            .map(|xi| (0..=deg).map(|p| pow(xi.clone(), int(p as i64))).collect())
            .collect(),
    );
    let rhs = Expr::Matrix(y.iter().map(|yi| vec![yi.clone()]).collect());
    normal_equations(&vandermonde, &rhs).map_err(|_| {
        format!(
            "stats.polyfit needs at least {} distinct x values for degree {}",
            deg + 1,
            deg
        )
    })
}

/// Evaluate a polynomial (coefficient vector, constant term first) at `t` —
/// a scalar, a symbol, or elementwise over a vector. Horner, then `expand`
/// so a symbolic argument reads as a polynomial.
fn polyval(c: &[Expr], t: &Expr) -> Result<Expr, String> {
    let horner = |t: &Expr| -> Result<Expr, String> {
        let mut acc = c.last().cloned().unwrap_or_else(|| int(0));
        for coeff in c.iter().rev().skip(1) {
            acc = add(vec![mul(vec![acc, t.clone()]), coeff.clone()]);
        }
        Ok(expand(&acc))
    };
    if matrix::is_matrix(t) {
        matrix::try_map(t, |e| horner(e))
    } else {
        horner(t)
    }
}

/// Exact general least squares: the β minimizing ‖Aβ − b‖₂, via the normal
/// equations AᵀAβ = Aᵀb. No automatic intercept — `hcat` a ones column.
fn lsq(a: &Expr, b: &Expr) -> Result<Expr, String> {
    let Expr::Matrix(rows) = a else {
        return Err("stats.lsq expects a matrix of regressors".into());
    };
    let bv = entries("stats.lsq", b)?;
    if rows.len() != bv.len() {
        return Err(format!(
            "stats.lsq expects one observation per regressor row, got {} rows and {} observations",
            rows.len(),
            bv.len()
        ));
    }
    let rhs = Expr::Matrix(bv.into_iter().map(|e| vec![e]).collect());
    normal_equations(a, &rhs)
        .map_err(|_| "stats.lsq: the regressors are linearly dependent (rank-deficient)".into())
}

/// Solve AᵀAβ = Aᵀb exactly. Errors when AᵀA is singular — `matrix::solve`
/// reports an underdetermined system as a solution *set* (a struct), which
/// for a fit means the minimizer isn't unique.
fn normal_equations(a: &Expr, b: &Expr) -> Result<Expr, String> {
    let at = matrix::transpose(a);
    let ata = matrix::mat_mul(&at, a)?;
    let atb = matrix::mat_mul(&at, b)?;
    match matrix::solve(&ata, &atb)? {
        Expr::Struct(_) => Err("the least-squares minimizer is not unique".into()),
        unique => Ok(unique),
    }
}

// -- distributions ------------------------------------------------------------

/// A distribution function: validate its arity, then hand back the symbolic
/// application. It carries no exact value (a normal CDF is transcendental), so
/// it stays a symbol until `N(...)` evaluates it via `crate::special`.
fn dist(name: &str, args: Vec<Expr>, allowed: &[usize]) -> Result<Expr, String> {
    if !allowed.contains(&args.len()) {
        let counts: Vec<String> = allowed.iter().map(|n| n.to_string()).collect();
        return Err(format!(
            "stats.{} expects {} argument(s), got {}",
            name,
            counts.join(" or "),
            args.len()
        ));
    }
    Ok(func(name, args))
}

// -- linear regression with inference ----------------------------------------

/// Ordinary least squares with the full inferential apparatus, as a fitted-
/// model struct. The point estimates and their covariance are *exact* (the
/// coefficient covariance σ̂²·(XᵀX)⁻¹ is a rational matrix, so standard errors
/// and t-statistics are exact surds); only the p-values, information criteria,
/// and log-likelihood carry a symbolic `tcdf`/`fcdf`/`ln` to be taken to
/// decimals with `N(...)`. An intercept column is added automatically unless
/// the design already holds a constant column.
fn regress(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 2 {
        return Err(format!(
            "stats.regress expects 2 argument(s), got {}",
            args.len()
        ));
    }
    let (x, y) = model_data("stats.regress", &args[0], &args[1])?;
    let w = vec![int(1); y.len()];
    fit_linear("stats.regress", &x, y, w)
}

/// Weighted least squares: exactly `stats.regress`, but minimizing
/// Σ wᵢ·(yᵢ − xᵢβ)² for per-observation weights `w` (inverse-variance weights
/// for heteroskedastic data, say). The same exact covariance machinery runs
/// weighted, so the result is a regression model with all the usual fields.
fn wls(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 3 {
        return Err(format!(
            "stats.wls expects 3 argument(s) (X, y, weights), got {}",
            args.len()
        ));
    }
    let (x, y) = model_data("stats.wls", &args[0], &args[1])?;
    let w = entries("stats.wls", &args[2])?;
    if w.len() != y.len() {
        return Err(format!(
            "stats.wls: {} weights for {} observations",
            w.len(),
            y.len()
        ));
    }
    if w.iter()
        .any(|wi| numeric_value(wi).is_some_and(|v| v <= BigRational::from_integer(0.into())))
    {
        return Err("stats.wls: weights must be positive".into());
    }
    fit_linear("stats.wls", &x, y, w)
}

/// The shared (weighted) least-squares engine behind `regress` and `wls`. With
/// `w` all ones it reproduces ordinary least squares exactly (`1·x → x`,
/// `1^½ → 1`, `ln 1 → 0` all fold away), so OLS pays nothing for the generality.
fn fit_linear(caller: &str, x: &Expr, y: Vec<Expr>, w: Vec<Expr>) -> Result<Expr, String> {
    let n = y.len();
    if n < 3 {
        return Err(format!("{} needs at least 3 observations", caller));
    }
    let mut rows = design_rows(caller, x, n)?;
    // Record whether we prepend the intercept, so `predict`/`robustse` can
    // rebuild a matching design for new data.
    let added_intercept = !has_constant_col(&rows);
    if added_intercept {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    let k = rows[0].len();
    if n <= k {
        return Err(format!(
            "{} needs more observations ({}) than parameters ({})",
            caller, n, k
        ));
    }
    let dfmodel = k - 1;
    if dfmodel == 0 {
        return Err(format!(
            "{} needs at least one non-constant regressor",
            caller
        ));
    }
    let df = n - k;

    // Weighted normal equations: β̂ = (XᵀWX)⁻¹XᵀWy, with W = diag(w). The
    // weight scales each row of X and each y, then XᵀW = (WX)ᵀ.
    let wx_rows: Vec<Vec<Expr>> = rows
        .iter()
        .zip(&w)
        .map(|(row, wi)| {
            row.iter()
                .map(|x| mul(vec![wi.clone(), x.clone()]))
                .collect()
        })
        .collect();
    let wy: Vec<Expr> = y
        .iter()
        .zip(&w)
        .map(|(yi, wi)| mul(vec![wi.clone(), yi.clone()]))
        .collect();

    let xmat = Expr::Matrix(rows.clone());
    let xt = matrix::transpose(&xmat);
    let wxmat = Expr::Matrix(wx_rows);
    let xtwx = matrix::mat_mul(&xt, &wxmat)?;
    let xtwy = matrix::mat_mul(&xt, &col(wy.clone()))?;
    let beta = match matrix::solve(&xtwx, &xtwy)? {
        Expr::Struct(_) => {
            return Err(format!(
                "{}: the regressors are linearly dependent (rank-deficient)",
                caller
            ))
        }
        b => b,
    };
    let xtwx_inv = matrix::inverse(&xtwx)?;
    let inv_diag = diagonal(&xtwx_inv)?;

    let fitted = matrix::mat_mul(&xmat, &beta)?;
    let fitted_v = entries(caller, &fitted)?;
    let beta_v = entries(caller, &beta)?;
    let resid: Vec<Expr> = y
        .iter()
        .zip(&fitted_v)
        .map(|(yi, fi)| add(vec![yi.clone(), mul(vec![int(-1), fi.clone()])]))
        .collect();

    // Weighted residual sum of squares Σ wᵢrᵢ².
    let rss = add(resid
        .iter()
        .zip(&w)
        .map(|(ri, wi)| expand(&mul(vec![wi.clone(), ri.clone(), ri.clone()])))
        .collect());
    if is_known_zero(&rss) {
        return Err(format!(
            "{}: residuals are exactly zero (a perfect fit leaves no residual variance \
             to do inference with)",
            caller
        ));
    }
    let sigma2 = mul(vec![inv_int(df), rss.clone()]);

    // Weighted total sum of squares, about the weighted mean ȳ_w = Σwy/Σw.
    let ybar = mul(vec![add(wy), pow(add(w.clone()), int(-1))]);
    let ss_tot = add(y
        .iter()
        .zip(&w)
        .map(|(yi, wi)| {
            let c = add(vec![yi.clone(), mul(vec![int(-1), ybar.clone()])]);
            expand(&mul(vec![wi.clone(), c.clone(), c]))
        })
        .collect());
    if is_known_zero(&ss_tot) {
        return Err(format!(
            "{} is undefined for constant observations (zero variance)",
            caller
        ));
    }

    // se_j = √(σ̂²·(XᵀWX)⁻¹_jj); t_j = β_j/se_j; two-sided p via the t CDF.
    let mut se = Vec::with_capacity(k);
    let mut tstat = Vec::with_capacity(k);
    let mut pvalue = Vec::with_capacity(k);
    for (bj, vjj) in beta_v.iter().zip(&inv_diag) {
        let var_j = mul(vec![sigma2.clone(), vjj.clone()]);
        se.push(pow(var_j.clone(), half()));
        let t = mul(vec![bj.clone(), pow(var_j, neg_half())]);
        let p = mul(vec![
            int(2),
            add(vec![
                int(1),
                mul(vec![
                    int(-1),
                    func("tcdf", vec![func("abs", vec![t.clone()]), int(df as i64)]),
                ]),
            ]),
        ]);
        tstat.push(t);
        pvalue.push(p);
    }

    // 95% confidence intervals: β_j ± t*·se_j, with t* = tinv(0.975, df).
    let tstar = func("tinv", vec![rat(39, 40), int(df as i64)]);
    let confint: Vec<Vec<Expr>> = beta_v
        .iter()
        .zip(&se)
        .map(|(bj, sj)| {
            let hw = mul(vec![tstar.clone(), sj.clone()]);
            vec![
                add(vec![bj.clone(), mul(vec![int(-1), hw.clone()])]),
                add(vec![bj.clone(), hw]),
            ]
        })
        .collect();

    // R², adjusted R², and the overall-significance F.
    let r2 = add(vec![
        int(1),
        mul(vec![int(-1), rss.clone(), pow(ss_tot.clone(), int(-1))]),
    ]);
    let adjr2 = add(vec![
        int(1),
        mul(vec![
            int(-1),
            rss.clone(),
            int(n as i64 - 1),
            inv_int(df),
            pow(ss_tot.clone(), int(-1)),
        ]),
    ]);
    let explained = add(vec![ss_tot, mul(vec![int(-1), rss.clone()])]);
    let fstat = mul(vec![
        explained,
        int(df as i64),
        inv_int(dfmodel),
        pow(rss.clone(), int(-1)),
    ]);
    let fpvalue = add(vec![
        int(1),
        mul(vec![
            int(-1),
            func(
                "fcdf",
                vec![fstat.clone(), int(dfmodel as i64), int(df as i64)],
            ),
        ]),
    ]);

    // Leverage hᵢ — the weighted hat-matrix diagonal, H = X·(XᵀWX)⁻¹·(WX)ᵀ —
    // then internally studentized residuals and Cook's distance (the weight
    // enters as √wᵢ / wᵢ respectively).
    let xtw = matrix::transpose(&wxmat);
    let hat = matrix::mat_mul(&matrix::mat_mul(&xmat, &xtwx_inv)?, &xtw)?;
    let lev = diagonal(&hat)?;
    let mut studentized = Vec::with_capacity(n);
    let mut cooks = Vec::with_capacity(n);
    for ((ei, hi), wi) in resid.iter().zip(&lev).zip(&w) {
        let one_minus_h = add(vec![int(1), mul(vec![int(-1), hi.clone()])]);
        studentized.push(mul(vec![
            ei.clone(),
            pow(wi.clone(), half()),
            pow(mul(vec![sigma2.clone(), one_minus_h.clone()]), neg_half()),
        ]));
        cooks.push(mul(vec![
            wi.clone(),
            pow(ei.clone(), int(2)),
            inv_int(k),
            pow(sigma2.clone(), int(-1)),
            hi.clone(),
            pow(one_minus_h, int(-2)),
        ]));
    }

    // Gaussian log-likelihood and the information criteria (symbolic: each
    // carries an `ln`). loglik = -(n/2)·(ln 2π + ln(RSS/n) + 1) + ½·Σ ln wᵢ;
    // the weight term vanishes for OLS (ln 1 = 0).
    let loglik = add(vec![
        mul(vec![
            rat_to_expr(BigRational::new(BigInt::from(-(n as i64)), BigInt::from(2))),
            add(vec![
                func("ln", vec![mul(vec![int(2), Expr::Const(Constant::Pi)])]),
                func("ln", vec![mul(vec![inv_int(n), rss.clone()])]),
                int(1),
            ]),
        ]),
        mul(vec![
            inv_int(2),
            add(w.iter().map(|wi| func("ln", vec![wi.clone()])).collect()),
        ]),
    ]);
    let neg2ll = mul(vec![int(-2), loglik.clone()]);
    let aic = add(vec![neg2ll.clone(), int(2 * (k as i64 + 1))]);
    let bic = add(vec![
        neg2ll,
        mul(vec![func("ln", vec![int(n as i64)]), int(k as i64 + 1)]),
    ]);

    structure(vec![
        ("coefficients".into(), beta),
        ("se".into(), col(se)),
        ("tstat".into(), col(tstat)),
        ("pvalue".into(), col(pvalue)),
        ("confint".into(), Expr::Matrix(confint)),
        ("intercept".into(), Expr::Bool(added_intercept)),
        ("fitted".into(), fitted),
        ("residuals".into(), col(resid)),
        ("leverage".into(), col(lev)),
        ("studentized".into(), col(studentized)),
        ("cooks".into(), col(cooks)),
        ("cov".into(), matrix::scalar_mul(&sigma2, &xtwx_inv)),
        ("sigma2".into(), sigma2),
        ("rss".into(), rss),
        ("r2".into(), r2),
        ("adjr2".into(), adjr2),
        ("fstat".into(), fstat),
        ("fpvalue".into(), fpvalue),
        ("loglik".into(), loglik),
        ("aic".into(), aic),
        ("bic".into(), bic),
        ("n".into(), int(n as i64)),
        ("k".into(), int(k as i64)),
        ("df".into(), int(df as i64)),
        ("dfmodel".into(), int(dfmodel as i64)),
    ])
}

/// Ridge regression: β̂ = (XᵀX + λP)⁻¹Xᵀy, the L2-penalized estimator that
/// trades a little bias for variance — the standard cure for multicollinearity.
/// Exact in λ (rational ⇒ rational β). The intercept is never penalized
/// (`P` is the identity with a 0 on the intercept). The estimator is biased, so
/// classical standard errors don't apply; the result reports the coefficients,
/// fit, and the effective degrees of freedom trace(X(XᵀX+λP)⁻¹Xᵀ). Predictors
/// on very different scales should be standardized first.
fn ridge(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 3 {
        return Err(format!(
            "stats.ridge expects 3 argument(s) (X, y, lambda), got {}",
            args.len()
        ));
    }
    let (x, y) = model_data("stats.ridge", &args[0], &args[1])?;
    let n = y.len();
    if n < 2 {
        return Err("stats.ridge needs at least 2 observations".into());
    }
    let lambda = &args[2];
    if numeric_value(lambda).is_some_and(|v| v < BigRational::from_integer(0.into())) {
        return Err("stats.ridge: the penalty lambda must be nonnegative".into());
    }
    let mut rows = design_rows("stats.ridge", &x, n)?;
    let added_intercept = !has_constant_col(&rows);
    if added_intercept {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    let k = rows[0].len();
    let intercept_idx = constant_col_index(&rows);

    let xmat = Expr::Matrix(rows.clone());
    let xt = matrix::transpose(&xmat);
    let xtx = matrix::mat_mul(&xt, &xmat)?;
    let xty = matrix::mat_mul(&xt, &col(y.clone()))?;

    // Add λ to the diagonal, skipping the intercept column.
    let Expr::Matrix(xtx_rows) = &xtx else {
        return Err("stats.ridge: internal error forming XᵀX".into());
    };
    let penalized = Expr::Matrix(
        xtx_rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .map(|(j, e)| {
                        if i == j && Some(i) != intercept_idx {
                            add(vec![e.clone(), lambda.clone()])
                        } else {
                            e.clone()
                        }
                    })
                    .collect()
            })
            .collect(),
    );

    let beta = match matrix::solve(&penalized, &xty)? {
        Expr::Struct(_) => return Err("stats.ridge: the penalized system is singular".into()),
        b => b,
    };
    let fitted = matrix::mat_mul(&xmat, &beta)?;
    let fitted_v = entries("stats.ridge", &fitted)?;
    let resid: Vec<Expr> = y
        .iter()
        .zip(&fitted_v)
        .map(|(yi, fi)| add(vec![yi.clone(), mul(vec![int(-1), fi.clone()])]))
        .collect();
    let rss = sum_products(&resid, &resid);

    // Effective degrees of freedom: trace((XᵀX+λP)⁻¹·XᵀX).
    let edf = {
        let prod = matrix::mat_mul(&matrix::inverse(&penalized)?, &xtx)?;
        add(diagonal(&prod)?)
    };

    let cy = centered(&y);
    let ss_tot = sum_products(&cy, &cy);
    let mut fields = vec![
        ("coefficients".into(), beta),
        ("fitted".into(), fitted),
        ("residuals".into(), col(resid)),
        ("rss".into(), rss.clone()),
        ("lambda".into(), lambda.clone()),
        ("edf".into(), edf),
        ("intercept".into(), Expr::Bool(added_intercept)),
        ("n".into(), int(n as i64)),
        ("k".into(), int(k as i64)),
    ];
    if !is_known_zero(&ss_tot) {
        fields.push((
            "r2".into(),
            add(vec![int(1), mul(vec![int(-1), rss, pow(ss_tot, int(-1))])]),
        ));
    }
    structure(fields)
}

/// The index of the first constant (intercept-spanning) column, if any.
fn constant_col_index(rows: &[Vec<Expr>]) -> Option<usize> {
    (0..rows[0].len()).find(|&j| rows.iter().all(|r| r[j] == rows[0][j]))
}

// -- the formula interface ----------------------------------------------------

/// Resolve the `(design X, response y)` for a model, supporting both the matrix
/// form `(X, y)` and the formula form `(response ~ terms, data)`.
fn model_data(caller: &str, a0: &Expr, a1: &Expr) -> Result<(Expr, Vec<Expr>), String> {
    if let Expr::Formula(lhs, rhs) = a0 {
        build_from_formula(caller, lhs, rhs, a1)
    } else {
        Ok((a0.clone(), entries(caller, a1)?))
    }
}

/// Build a design matrix and response from `response ~ terms` against a data
/// struct (columns named by the formula's symbols). Numeric columns enter
/// directly; categorical (symbol-valued) columns are one-hot encoded with the
/// first level dropped as the reference, since `fit_linear` supplies the
/// intercept.
fn build_from_formula(
    caller: &str,
    lhs: &Expr,
    rhs: &Expr,
    data: &Expr,
) -> Result<(Expr, Vec<Expr>), String> {
    let Expr::Struct(fields) = data else {
        return Err(format!(
            "{}: the formula form needs a data struct as the second argument",
            caller
        ));
    };
    let Expr::Symbol(resp) = lhs else {
        return Err(format!(
            "{}: the formula response (left of ~) must be a single column name",
            caller
        ));
    };
    let y = lookup_column(caller, fields, resp)?;
    let n = y.len();

    let mut cols: Vec<Vec<Expr>> = Vec::new();
    for term in formula_terms(caller, rhs)? {
        let column = lookup_column(caller, fields, &term)?;
        if column.len() != n {
            return Err(format!(
                "{}: column '{}' has {} rows but the response has {}",
                caller,
                term,
                column.len(),
                n
            ));
        }
        if column.iter().any(|e| !is_numeric(e)) {
            // Categorical: one indicator column per level past the reference.
            let levels = distinct(&column);
            for lv in levels.iter().skip(1) {
                cols.push(
                    column
                        .iter()
                        .map(|x| if x == lv { int(1) } else { int(0) })
                        .collect(),
                );
            }
        } else {
            cols.push(column);
        }
    }
    if cols.is_empty() {
        return Err(format!("{}: the formula has no usable predictors", caller));
    }
    let rows: Vec<Vec<Expr>> = (0..n)
        .map(|i| cols.iter().map(|c| c[i].clone()).collect())
        .collect();
    Ok((Expr::Matrix(rows), y))
}

/// The additive terms on the right of `~`, each a bare column name. (`a + b`
/// has already canonicalized to `Add([a, b])`.)
fn formula_terms(caller: &str, rhs: &Expr) -> Result<Vec<String>, String> {
    let name = |e: &Expr| match e {
        Expr::Symbol(s) => Ok(s.clone()),
        other => Err(format!(
            "{}: formula predictors must be column names, got '{}'",
            caller, other
        )),
    };
    match rhs {
        Expr::Add(ts) => ts.iter().map(name).collect(),
        other => Ok(vec![name(other)?]),
    }
}

/// A named column of a data struct, as a flat vector of entries.
fn lookup_column(caller: &str, fields: &[(String, Expr)], name: &str) -> Result<Vec<Expr>, String> {
    let v = fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .ok_or_else(|| format!("{}: the data has no column '{}'", caller, name))?;
    entries(caller, v)
}

fn is_numeric(e: &Expr) -> bool {
    numeric_value(e).is_some() || matches!(e, Expr::Float(..))
}

/// Distinct entries in first-appearance order.
fn distinct(xs: &[Expr]) -> Vec<Expr> {
    let mut out: Vec<Expr> = Vec::new();
    for x in xs {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// Logistic regression by iteratively reweighted least squares (IRLS). The
/// response `y` is binary (0/1); the fit models P(y = 1) = 1/(1 + e^{−xβ}).
/// IRLS iterates — so the estimates are floats — but it's the same weighted
/// least squares we already do, looped. Inference is Wald: standard errors from
/// (XᵀWX)⁻¹ at convergence, two-sided p-values from the normal CDF.
fn logit(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 2 {
        return Err(format!(
            "stats.logit expects 2 argument(s) (X, y), got {}",
            args.len()
        ));
    }
    let (xdesign, y_expr) = model_data("stats.logit", &args[0], &args[1])?;
    let n = y_expr.len();
    if n < 3 {
        return Err("stats.logit needs at least 3 observations".into());
    }
    let mut y = Vec::with_capacity(n);
    for yi in &y_expr {
        let v = eval_f64(yi, &[])?;
        if v != 0.0 && v != 1.0 {
            return Err("stats.logit: the response must be binary (every value 0 or 1)".into());
        }
        y.push(v);
    }
    let mut rows = design_rows("stats.logit", &xdesign, n)?;
    let added_intercept = !has_constant_col(&rows);
    if added_intercept {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    let k = rows[0].len();
    if n <= k {
        return Err(format!(
            "stats.logit needs more observations ({}) than parameters ({})",
            n, k
        ));
    }
    let x: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|e| eval_f64(e, &[]))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let (beta, mu, wts, iters, converged) = irls(&x, &y, k)?;
    let cov = nlfit::inverse(&xtwx_f64(&x, &wts, k)).ok_or(
        "stats.logit: the information matrix is singular (perfect separation, or a collinear \
         regressor)",
    )?;

    // Wald standard errors, z-statistics, and normal-tail p-values.
    let mut se = Vec::with_capacity(k);
    let mut zstat = Vec::with_capacity(k);
    let mut pvalue = Vec::with_capacity(k);
    for (j, &b) in beta.iter().enumerate() {
        let s = cov[j][j].max(0.0).sqrt();
        se.push(nlfit::float_expr(s)?);
        let zv = if s > 0.0 {
            b / s
        } else if b == 0.0 {
            0.0
        } else {
            b.signum() * 1e308
        };
        let ze = nlfit::float_expr(zv)?;
        pvalue.push(mul(vec![
            int(2),
            add(vec![
                int(1),
                mul(vec![
                    int(-1),
                    func("normcdf", vec![func("abs", vec![ze.clone()])]),
                ]),
            ]),
        ]));
        zstat.push(ze);
    }

    let dev = deviance(&y, &mu);
    let ybar = y.iter().sum::<f64>() / n as f64;
    let null_dev = deviance(&y, &vec![ybar; n]);
    let resid: Vec<f64> = y.iter().zip(&mu).map(|(yi, mi)| yi - mi).collect();

    structure(vec![
        ("coefficients".into(), col(nlfit::floats(&beta)?)),
        ("se".into(), col(se)),
        ("zstat".into(), col(zstat)),
        ("pvalue".into(), col(pvalue)),
        ("fitted".into(), col(nlfit::floats(&mu)?)),
        ("residuals".into(), col(nlfit::floats(&resid)?)),
        ("deviance".into(), nlfit::float_expr(dev)?),
        ("nulldeviance".into(), nlfit::float_expr(null_dev)?),
        ("pseudor2".into(), nlfit::float_expr(1.0 - dev / null_dev)?),
        ("intercept".into(), Expr::Bool(added_intercept)),
        ("iterations".into(), int(iters as i64)),
        ("converged".into(), Expr::Bool(converged)),
        ("n".into(), int(n as i64)),
        ("k".into(), int(k as i64)),
    ])
}

/// One IRLS run. Returns (β, fitted probabilities μ, IRLS weights, iterations,
/// converged). The working response z = η + (y−μ)/w turns each step into a
/// weighted least-squares solve.
#[allow(clippy::type_complexity)]
fn irls(
    x: &[Vec<f64>],
    y: &[f64],
    k: usize,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>, usize, bool), String> {
    let mut beta = vec![0.0_f64; k];
    let mut dev = f64::INFINITY;
    let mut converged = false;
    let mut iters = 0;
    for it in 0..100 {
        iters = it + 1;
        let mut h = vec![vec![0.0_f64; k]; k];
        let mut g = vec![0.0_f64; k];
        for (xi, &yi) in x.iter().zip(y) {
            let eta: f64 = (0..k).map(|j| xi[j] * beta[j]).sum();
            let m = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
            let w = (m * (1.0 - m)).max(1e-12);
            let z = eta + (yi - m) / w; // working response
            for a in 0..k {
                g[a] += w * xi[a] * z;
                for b in 0..k {
                    h[a][b] += w * xi[a] * xi[b];
                }
            }
        }
        beta = nlfit::solve_linear(&h, &g).ok_or(
            "stats.logit: the information matrix is singular (perfect separation, or a collinear \
             regressor)",
        )?;
        let (mu, _) = predict_probs(x, &beta, k);
        let new_dev = deviance(y, &mu);
        if (dev - new_dev).abs() < 1e-12 * (new_dev.abs() + 1e-12) {
            converged = true;
            break;
        }
        dev = new_dev;
    }
    let (mu, wts) = predict_probs(x, &beta, k);
    Ok((beta, mu, wts, iters, converged))
}

/// Fitted probabilities μ and IRLS weights μ(1−μ) at the given coefficients.
fn predict_probs(x: &[Vec<f64>], beta: &[f64], k: usize) -> (Vec<f64>, Vec<f64>) {
    let mut mu = Vec::with_capacity(x.len());
    let mut w = Vec::with_capacity(x.len());
    for xi in x {
        let eta: f64 = (0..k).map(|j| xi[j] * beta[j]).sum();
        let m = (1.0 / (1.0 + (-eta).exp())).clamp(1e-12, 1.0 - 1e-12);
        mu.push(m);
        w.push((m * (1.0 - m)).max(1e-12));
    }
    (mu, w)
}

/// XᵀWX for the binomial weights W = diag(μ(1−μ)).
fn xtwx_f64(x: &[Vec<f64>], w: &[f64], k: usize) -> Vec<Vec<f64>> {
    let mut h = vec![vec![0.0_f64; k]; k];
    for (xi, &wi) in x.iter().zip(w) {
        for a in 0..k {
            for b in 0..k {
                h[a][b] += wi * xi[a] * xi[b];
            }
        }
    }
    h
}

/// Binomial deviance −2·Σ[yᵢ ln μᵢ + (1−yᵢ) ln(1−μᵢ)].
fn deviance(y: &[f64], mu: &[f64]) -> f64 {
    y.iter()
        .zip(mu)
        .map(|(&yi, mi)| {
            let m = mi.clamp(1e-12, 1.0 - 1e-12);
            -2.0 * (yi * m.ln() + (1.0 - yi) * (1.0 - m).ln())
        })
        .sum()
}

/// Interpret the regressor argument as `n` rows of predictors: an n×k design
/// matrix as-is, or a length-n vector (row or column) as a single predictor.
fn design_rows(caller: &str, x: &Expr, n: usize) -> Result<Vec<Vec<Expr>>, String> {
    let Expr::Matrix(rows) = x else {
        return Err(format!(
            "{} expects a matrix or vector of regressors",
            caller
        ));
    };
    if rows.len() == 1 && rows[0].len() == n {
        return Ok(rows[0].iter().map(|e| vec![e.clone()]).collect());
    }
    if rows.len() == n {
        return Ok(rows.clone());
    }
    Err(format!(
        "{}: {} regressor rows but {} observations",
        caller,
        rows.len(),
        n
    ))
}

/// Does any column hold one repeated value? Such a column already spans the
/// intercept, so `regress` won't add another (which would be rank-deficient).
fn has_constant_col(rows: &[Vec<Expr>]) -> bool {
    (0..rows[0].len()).any(|j| rows.iter().all(|r| r[j] == rows[0][j]))
}

/// Pack a list of entries as an n×1 column vector.
fn col(v: Vec<Expr>) -> Expr {
    Expr::Matrix(v.into_iter().map(|e| vec![e]).collect())
}

/// The main diagonal of a square matrix.
fn diagonal(m: &Expr) -> Result<Vec<Expr>, String> {
    let Expr::Matrix(rows) = m else {
        return Err("expected a matrix".into());
    };
    Ok((0..rows.len()).map(|i| rows[i][i].clone()).collect())
}

// -- post-estimation ----------------------------------------------------------

/// Read a named field out of a `stats.regress` model struct.
fn model_field<'a>(model: &'a Expr, fname: &str, caller: &str) -> Result<&'a Expr, String> {
    let Expr::Struct(fields) = model else {
        return Err(format!(
            "{} expects a model struct from stats.regress",
            caller
        ));
    };
    fields
        .iter()
        .find(|(n, _)| n == fname)
        .map(|(_, v)| v)
        .ok_or_else(|| {
            format!(
                "{}: model has no field '{}' (is it from stats.regress?)",
                caller, fname
            )
        })
}

/// A model field that should be a non-negative integer (n, k, df).
fn field_usize(model: &Expr, fname: &str, caller: &str) -> Result<usize, String> {
    numeric_value(model_field(model, fname, caller)?)
        .and_then(|r| r.to_integer().to_usize())
        .ok_or_else(|| format!("{}: field '{}' is not an integer", caller, fname))
}

/// Confidence level → tail probability (1+level)/2, as an exact rational.
/// Defaults to 95% when omitted.
fn tail_prob(caller: &str, level: Option<&Expr>) -> Result<Expr, String> {
    let l = match level {
        None => BigRational::new(BigInt::from(95), BigInt::from(100)),
        Some(e) => numeric_value(e)
            .filter(|r| {
                *r > BigRational::from_integer(0.into()) && *r < BigRational::from_integer(1.into())
            })
            .ok_or_else(|| format!("{}: confidence level must be between 0 and 1", caller))?,
    };
    Ok(rat_to_expr(
        (BigRational::from_integer(1.into()) + l) / BigRational::from_integer(2.into()),
    ))
}

/// `stats.predict(model, Xnew[, level])`: point predictions at new regressor
/// rows, with a confidence interval for the mean response and a (wider)
/// prediction interval for a new observation. `Xnew` carries the same raw
/// predictors as the design given to `regress` (the intercept is reattached
/// automatically). Intervals are symbolic — `N(...)` for decimals.
fn predict(args: &[Expr]) -> Result<Expr, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "stats.predict expects 2 or 3 argument(s), got {}",
            args.len()
        ));
    }
    let model = &args[0];
    let beta = model_field(model, "coefficients", "stats.predict")?.clone();
    let k = entries("stats.predict", &beta)?.len();
    let cov = model_field(model, "cov", "stats.predict")?.clone();
    let sigma2 = model_field(model, "sigma2", "stats.predict")?.clone();
    let df = model_field(model, "df", "stats.predict")?.clone();
    let intercept = matches!(
        model_field(model, "intercept", "stats.predict")?,
        Expr::Bool(true)
    );
    let p_raw = if intercept { k - 1 } else { k };
    let tstar = func("tinv", vec![tail_prob("stats.predict", args.get(2))?, df]);

    let mut rows = predict_design(&args[1], p_raw)?;
    if intercept {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    let xn = Expr::Matrix(rows);
    let fit = matrix::mat_mul(&xn, &beta)?;
    let fit_v = entries("stats.predict", &fit)?;
    // var of the mean response = diag(Xnew·cov·Xnewᵀ); a new draw adds σ̂².
    let covx = matrix::mat_mul(&xn, &cov)?;
    let vmean = diagonal(&matrix::mat_mul(&covx, &matrix::transpose(&xn))?)?;

    let mut se = Vec::with_capacity(fit_v.len());
    let mut ci = Vec::with_capacity(fit_v.len());
    let mut pi = Vec::with_capacity(fit_v.len());
    for (fi, vm) in fit_v.iter().zip(&vmean) {
        let se_mean = pow(vm.clone(), half());
        let se_pred = pow(add(vec![sigma2.clone(), vm.clone()]), half());
        se.push(se_mean.clone());
        ci.push(interval(fi, &mul(vec![tstar.clone(), se_mean])));
        pi.push(interval(fi, &mul(vec![tstar.clone(), se_pred])));
    }
    structure(vec![
        ("fit".into(), fit),
        ("se".into(), col(se)),
        ("ci".into(), Expr::Matrix(ci)),
        ("pi".into(), Expr::Matrix(pi)),
    ])
}

/// `[center − half, center + half]` as a 2-element interval row.
fn interval(center: &Expr, half_width: &Expr) -> Vec<Expr> {
    vec![
        add(vec![center.clone(), mul(vec![int(-1), half_width.clone()])]),
        add(vec![center.clone(), half_width.clone()]),
    ]
}

/// Interpret new predictor data for `predict`: a length-m vector for a single-
/// predictor model, or an m×p matrix otherwise. Returns m rows of p values.
fn predict_design(x: &Expr, p: usize) -> Result<Vec<Vec<Expr>>, String> {
    let Expr::Matrix(rows) = x else {
        return Err("stats.predict expects a matrix or vector of new predictor values".into());
    };
    if p == 1 {
        if rows.len() == 1 {
            return Ok(rows[0].iter().map(|e| vec![e.clone()]).collect());
        }
        if rows.iter().all(|r| r.len() == 1) {
            return Ok(rows.clone());
        }
        return Err(
            "stats.predict: this model has 1 predictor; pass a vector of new values".into(),
        );
    }
    if rows.iter().all(|r| r.len() == p) {
        return Ok(rows.clone());
    }
    Err(format!(
        "stats.predict: this model has {} predictors, so each new row needs {} values",
        p, p
    ))
}

/// `stats.robustse(model, X[, type])`: heteroskedasticity-consistent (White
/// sandwich) standard errors, with the same `X` passed to `regress`. `type` is
/// 0–3 for HC0–HC3 (default HC1). The meat matrix needs the design, hence the
/// re-passed `X`; everything is exact. Returns robust se/t/p.
fn robustse(args: &[Expr]) -> Result<Expr, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "stats.robustse expects 2 or 3 argument(s), got {}",
            args.len()
        ));
    }
    let model = &args[0];
    let n = field_usize(model, "n", "stats.robustse")?;
    let df = field_usize(model, "df", "stats.robustse")?;
    let beta = model_field(model, "coefficients", "stats.robustse")?.clone();
    let beta_v = entries("stats.robustse", &beta)?;
    let resid = entries(
        "stats.robustse",
        model_field(model, "residuals", "stats.robustse")?,
    )?;
    let lev = entries(
        "stats.robustse",
        model_field(model, "leverage", "stats.robustse")?,
    )?;
    let cov = model_field(model, "cov", "stats.robustse")?.clone();
    let sigma2 = model_field(model, "sigma2", "stats.robustse")?.clone();
    let intercept = matches!(
        model_field(model, "intercept", "stats.robustse")?,
        Expr::Bool(true)
    );
    let hc = match args.get(2) {
        None => 1,
        Some(e) => numeric_value(e)
            .and_then(|r| r.to_integer().to_i64())
            .filter(|t| (0..=3).contains(t))
            .ok_or("stats.robustse: type must be 0, 1, 2, or 3 (HC0–HC3)")?,
    };

    // Rebuild the fitted design (with intercept) to match the coefficients.
    let mut rows = design_rows("stats.robustse", &args[1], n)?;
    if intercept {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    if rows[0].len() != beta_v.len() {
        return Err("stats.robustse: X does not match the model's regressors".into());
    }

    // (XᵀX)⁻¹ = cov / σ̂²; meat = Xᵀ·diag(ωᵢ)·X with ωᵢ a weighted squared
    // residual; sandwich V = (XᵀX)⁻¹·meat·(XᵀX)⁻¹.
    let xtx_inv = matrix::scalar_mul(&pow(sigma2, int(-1)), &cov);
    let wx: Vec<Vec<Expr>> = rows
        .iter()
        .zip(&resid)
        .zip(&lev)
        .map(|((row, e), h)| {
            let w = hc_weight(hc, e, h, n, df);
            row.iter()
                .map(|x| mul(vec![w.clone(), x.clone()]))
                .collect()
        })
        .collect();
    let xmat = Expr::Matrix(rows);
    let meat = matrix::mat_mul(&matrix::transpose(&xmat), &Expr::Matrix(wx))?;
    let vmat = matrix::mat_mul(&matrix::mat_mul(&xtx_inv, &meat)?, &xtx_inv)?;

    let mut se = Vec::new();
    let mut tstat = Vec::new();
    let mut pvalue = Vec::new();
    for (bj, vjj) in beta_v.iter().zip(diagonal(&vmat)?) {
        let s = pow(vjj.clone(), half());
        let t = mul(vec![bj.clone(), pow(vjj, neg_half())]);
        pvalue.push(mul(vec![
            int(2),
            add(vec![
                int(1),
                mul(vec![
                    int(-1),
                    func("tcdf", vec![func("abs", vec![t.clone()]), int(df as i64)]),
                ]),
            ]),
        ]));
        tstat.push(t);
        se.push(s);
    }
    structure(vec![
        ("se".into(), col(se)),
        ("tstat".into(), col(tstat)),
        ("pvalue".into(), col(pvalue)),
    ])
}

/// The per-observation weight ωᵢ = cᵢ·eᵢ² for the HC sandwich meat.
fn hc_weight(hc: i64, e: &Expr, h: &Expr, n: usize, df: usize) -> Expr {
    let e2 = pow(e.clone(), int(2));
    let one_minus_h = || add(vec![int(1), mul(vec![int(-1), h.clone()])]);
    match hc {
        0 => e2,
        1 => mul(vec![rat(n as i64, df as i64), e2]),
        2 => mul(vec![e2, pow(one_minus_h(), int(-1))]),
        _ => mul(vec![e2, pow(one_minus_h(), int(-2))]),
    }
}

/// `stats.anova(reduced, full)`: an F-test comparing two nested OLS models
/// (order-independent — the one with fewer residual degrees of freedom is the
/// fuller model). F = [(RSSᵣ − RSSf)/Δdf] / [RSSf/dff].
fn anova(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 2 {
        return Err(format!(
            "stats.anova expects 2 argument(s), got {}",
            args.len()
        ));
    }
    let (rss1, df1) = (
        model_field(&args[0], "rss", "stats.anova")?.clone(),
        field_usize(&args[0], "df", "stats.anova")?,
    );
    let (rss2, df2) = (
        model_field(&args[1], "rss", "stats.anova")?.clone(),
        field_usize(&args[1], "df", "stats.anova")?,
    );
    if df1 == df2 {
        return Err("stats.anova: the models have equal residual df (are they nested?)".into());
    }
    // Fewer residual df = more parameters = the fuller model.
    let ((rss_r, df_r), (rss_f, df_f)) = if df1 > df2 {
        ((rss1, df1), (rss2, df2))
    } else {
        ((rss2, df2), (rss1, df1))
    };
    let ddf = df_r - df_f;
    let num = mul(vec![
        add(vec![rss_r, mul(vec![int(-1), rss_f.clone()])]),
        inv_int(ddf),
    ]);
    let den = mul(vec![rss_f, inv_int(df_f)]);
    let fstat = mul(vec![num, pow(den, int(-1))]);
    let pvalue = add(vec![
        int(1),
        mul(vec![
            int(-1),
            func(
                "fcdf",
                vec![fstat.clone(), int(ddf as i64), int(df_f as i64)],
            ),
        ]),
    ]);
    structure(vec![
        ("fstat".into(), fstat),
        ("pvalue".into(), pvalue),
        ("df1".into(), int(ddf as i64)),
        ("df2".into(), int(df_f as i64)),
    ])
}

/// `stats.bptest(model)`: Breusch–Pagan / Koenker test for heteroskedasticity,
/// regressing the squared residuals on the fitted values. LM = n·R² ~ χ²(1).
fn bptest(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err(format!(
            "stats.bptest expects 1 argument, got {}",
            args.len()
        ));
    }
    let resid = entries(
        "stats.bptest",
        model_field(&args[0], "residuals", "stats.bptest")?,
    )?;
    let fitted = entries(
        "stats.bptest",
        model_field(&args[0], "fitted", "stats.bptest")?,
    )?;
    let n = resid.len();
    let esq: Vec<Expr> = resid
        .iter()
        .map(|e| expand(&pow(e.clone(), int(2))))
        .collect();
    let var_e = variance(&esq, "stats.bptest")?;
    let var_f = variance(&fitted, "stats.bptest")?;
    if is_known_zero(&var_e) || is_known_zero(&var_f) {
        return Err("stats.bptest: no variation in squared residuals or fitted values".into());
    }
    // R² of the auxiliary simple regression = cov(e²,ŷ)² / (var(e²)·var(ŷ)).
    let cov_ef = covariance(&esq, &fitted)?;
    let r2_aux = mul(vec![
        pow(cov_ef, int(2)),
        pow(var_e, int(-1)),
        pow(var_f, int(-1)),
    ]);
    let lm = mul(vec![int(n as i64), r2_aux]);
    chisq_test(lm, 1)
}

/// `stats.dwtest(model)`: the Durbin–Watson statistic for first-order residual
/// autocorrelation, Σ(eᵢ−eᵢ₋₁)² / Σeᵢ² — exact, in [0,4], ≈2 meaning none.
fn dwtest(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err(format!(
            "stats.dwtest expects 1 argument, got {}",
            args.len()
        ));
    }
    let e = entries(
        "stats.dwtest",
        model_field(&args[0], "residuals", "stats.dwtest")?,
    )?;
    if e.len() < 2 {
        return Err("stats.dwtest needs at least 2 residuals".into());
    }
    let diffs: Vec<Expr> = (1..e.len())
        .map(|i| add(vec![e[i].clone(), mul(vec![int(-1), e[i - 1].clone()])]))
        .collect();
    let dw = mul(vec![
        sum_products(&diffs, &diffs),
        pow(sum_products(&e, &e), int(-1)),
    ]);
    structure(vec![("statistic".into(), dw)])
}

/// `stats.jbtest(model)`: the Jarque–Bera test of residual normality from
/// sample skewness S and kurtosis K, JB = (n/6)(S² + (K−3)²/4) ~ χ²(2). The
/// statistic is exact (OLS residuals sum to zero, so the moments are rational).
fn jbtest(args: &[Expr]) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err(format!(
            "stats.jbtest expects 1 argument, got {}",
            args.len()
        ));
    }
    let e = entries(
        "stats.jbtest",
        model_field(&args[0], "residuals", "stats.jbtest")?,
    )?;
    let n = e.len();
    let moment = |p: i64| {
        mul(vec![
            inv_int(n),
            add(e.iter().map(|x| expand(&pow(x.clone(), int(p)))).collect()),
        ])
    };
    let (m2, m3, m4) = (moment(2), moment(3), moment(4));
    if is_known_zero(&m2) {
        return Err("stats.jbtest: residuals have zero variance".into());
    }
    let skew_sq = mul(vec![pow(m3, int(2)), pow(m2.clone(), int(-3))]);
    let kurt_excess = add(vec![mul(vec![m4, pow(m2, int(-2))]), int(-3)]);
    let jb = mul(vec![
        rat(n as i64, 6),
        add(vec![
            skew_sq,
            mul(vec![inv_int(4), pow(kurt_excess, int(2))]),
        ]),
    ]);
    chisq_test(jb, 2)
}

/// Wrap a test statistic with its upper-tail χ²(df) p-value as a struct.
fn chisq_test(statistic: Expr, df: i64) -> Result<Expr, String> {
    let pvalue = add(vec![
        int(1),
        mul(vec![
            int(-1),
            func("chisqcdf", vec![statistic.clone(), int(df)]),
        ]),
    ]);
    structure(vec![
        ("statistic".into(), statistic),
        ("pvalue".into(), pvalue),
    ])
}

/// Entries sorted by exact numeric value (floats by their exact binary
/// value). Errors on anything unorderable.
fn sorted_numeric(name: &str, xs: &[Expr]) -> Result<Vec<Expr>, String> {
    let mut keyed: Vec<(BigRational, &Expr)> = Vec::with_capacity(xs.len());
    for x in xs {
        let key = match x {
            Expr::Float(bf, _) => float_to_rational(bf),
            other => numeric_value(other),
        }
        .ok_or_else(|| {
            format!(
                "{} needs numeric entries; '{}' can't be ordered (try N(...))",
                name, x
            )
        })?;
        keyed.push((key, x));
    }
    keyed.sort_by(|p, q| p.0.cmp(&q.0));
    Ok(keyed.into_iter().map(|(_, x)| x.clone()).collect())
}

// -- argument plumbing --------------------------------------------------------

/// 1/n as an exact rational.
fn inv_int(n: usize) -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(1), BigInt::from(n)))
}

/// n/d as an exact rational literal.
fn rat(n: i64, d: i64) -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(n), BigInt::from(d)))
}

fn neg_half() -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(-1), BigInt::from(2)))
}

fn is_known_zero(e: &Expr) -> bool {
    numeric_value(e).is_some_and(|r| r == BigRational::new(BigInt::from(0), BigInt::from(1)))
}

fn entries(name: &str, e: &Expr) -> Result<Vec<Expr>, String> {
    let Expr::Matrix(rows) = e else {
        return Err(format!("{} expects a vector (a 1×n or n×1 matrix)", name));
    };
    if rows.len() == 1 {
        Ok(rows[0].clone())
    } else if rows.iter().all(|r| r.len() == 1) {
        Ok(rows.iter().map(|r| r[0].clone()).collect())
    } else {
        Err(format!(
            "{} expects a vector (a 1×n or n×1 matrix), got a {}×{} matrix",
            name,
            rows.len(),
            rows[0].len()
        ))
    }
}

fn one_vector(name: &str, args: &[Expr]) -> Result<Vec<Expr>, String> {
    if args.len() != 1 {
        return Err(format!(
            "{} expects 1 argument(s), got {}",
            name,
            args.len()
        ));
    }
    entries(name, &args[0])
}

fn two_vectors(name: &str, args: &[Expr]) -> Result<(Vec<Expr>, Vec<Expr>), String> {
    if args.len() != 2 {
        return Err(format!(
            "{} expects 2 argument(s), got {}",
            name,
            args.len()
        ));
    }
    let a = entries(name, &args[0])?;
    let b = entries(name, &args[1])?;
    if a.len() != b.len() {
        return Err(format!(
            "{} expects two vectors of the same length, got {} and {}",
            name,
            a.len(),
            b.len()
        ));
    }
    Ok((a, b))
}
