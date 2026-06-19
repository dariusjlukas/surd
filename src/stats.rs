//! The `stats` built-in namespace: exact statistics.
//!
//! Every estimator runs in exact arithmetic: the mean of rationals is a
//! rational, a variance is a rational, and a standard deviation is an exact
//! surd — `stats.std([1; 2; 3; 4])` is `sqrt(5/3)`, with `N(...)` taking it
//! to floats only on request. `var`, `std`, `cov`, and `cor` are the
//! *sample* estimators (n−1 denominator). Symbolic entries flow through
//! everything that doesn't need ordering; `median` requires numeric data.

use crate::expr::*;
use crate::matrix;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &[
    "mean", "median", "quantile", "var", "std", "cov", "cor", "linfit", "polyfit", "polyval",
    "lsq", "regress", "rmse", "r2", "normcdf", "normpdf", "norminv", "tcdf", "tpdf", "tinv",
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
    structure(vec![
        ("intercept".to_string(), intercept),
        ("slope".to_string(), slope),
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
    let y = entries("stats.regress", &args[1])?;
    let n = y.len();
    if n < 3 {
        return Err("stats.regress needs at least 3 observations".into());
    }
    let mut rows = design_rows(&args[0], n)?;
    if !has_constant_col(&rows) {
        for r in rows.iter_mut() {
            r.insert(0, int(1));
        }
    }
    let k = rows[0].len();
    if n <= k {
        return Err(format!(
            "stats.regress needs more observations ({}) than parameters ({})",
            n, k
        ));
    }
    let dfmodel = k - 1;
    if dfmodel == 0 {
        return Err("stats.regress needs at least one non-constant regressor".into());
    }
    let df = n - k;

    // β̂ = (XᵀX)⁻¹Xᵀy, plus (XᵀX)⁻¹ itself for the covariance.
    let xmat = Expr::Matrix(rows.clone());
    let ycol = col(y.clone());
    let xt = matrix::transpose(&xmat);
    let xtx = matrix::mat_mul(&xt, &xmat)?;
    let xty = matrix::mat_mul(&xt, &ycol)?;
    let beta = match matrix::solve(&xtx, &xty)? {
        Expr::Struct(_) => {
            return Err(
                "stats.regress: the regressors are linearly dependent (rank-deficient)".into(),
            )
        }
        b => b,
    };
    let xtx_inv = matrix::inverse(&xtx)?;
    let inv_diag = diagonal(&xtx_inv)?;

    let fitted = matrix::mat_mul(&xmat, &beta)?;
    let fitted_v = entries("stats.regress", &fitted)?;
    let beta_v = entries("stats.regress", &beta)?;
    let resid: Vec<Expr> = y
        .iter()
        .zip(&fitted_v)
        .map(|(yi, fi)| add(vec![yi.clone(), mul(vec![int(-1), fi.clone()])]))
        .collect();

    let rss = sum_products(&resid, &resid);
    if is_known_zero(&rss) {
        return Err(
            "stats.regress: residuals are exactly zero (a perfect fit leaves no residual variance \
             to do inference with)"
                .into(),
        );
    }
    let sigma2 = mul(vec![inv_int(df), rss.clone()]);

    let cy = centered(&y);
    let ss_tot = sum_products(&cy, &cy);
    if is_known_zero(&ss_tot) {
        return Err("stats.regress is undefined for constant observations (zero variance)".into());
    }

    // se_j = √(σ̂²·(XᵀX)⁻¹_jj); t_j = β_j/se_j; two-sided p via the t CDF.
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

    // Leverage hᵢ (hat-matrix diagonal), internally studentized residuals, and
    // Cook's distance — the last is exact (no radical).
    let hat = matrix::mat_mul(&matrix::mat_mul(&xmat, &xtx_inv)?, &xt)?;
    let lev = diagonal(&hat)?;
    let mut studentized = Vec::with_capacity(n);
    let mut cooks = Vec::with_capacity(n);
    for (ei, hi) in resid.iter().zip(&lev) {
        let one_minus_h = add(vec![int(1), mul(vec![int(-1), hi.clone()])]);
        studentized.push(mul(vec![
            ei.clone(),
            pow(mul(vec![sigma2.clone(), one_minus_h.clone()]), neg_half()),
        ]));
        cooks.push(mul(vec![
            pow(ei.clone(), int(2)),
            inv_int(k),
            pow(sigma2.clone(), int(-1)),
            hi.clone(),
            pow(one_minus_h, int(-2)),
        ]));
    }

    // Gaussian log-likelihood and the information criteria (symbolic: each
    // carries an `ln`). loglik = -(n/2)·(ln 2π + ln(RSS/n) + 1).
    let loglik = mul(vec![
        rat_to_expr(BigRational::new(BigInt::from(-(n as i64)), BigInt::from(2))),
        add(vec![
            func("ln", vec![mul(vec![int(2), Expr::Const(Constant::Pi)])]),
            func("ln", vec![mul(vec![inv_int(n), rss.clone()])]),
            int(1),
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
        ("fitted".into(), fitted),
        ("residuals".into(), col(resid)),
        ("leverage".into(), col(lev)),
        ("studentized".into(), col(studentized)),
        ("cooks".into(), col(cooks)),
        ("cov".into(), matrix::scalar_mul(&sigma2, &xtx_inv)),
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

/// Interpret the regressor argument as `n` rows of predictors: an n×k design
/// matrix as-is, or a length-n vector (row or column) as a single predictor.
fn design_rows(x: &Expr, n: usize) -> Result<Vec<Vec<Expr>>, String> {
    let Expr::Matrix(rows) = x else {
        return Err("stats.regress expects a matrix or vector of regressors".into());
    };
    if rows.len() == 1 && rows[0].len() == n {
        return Ok(rows[0].iter().map(|e| vec![e.clone()]).collect());
    }
    if rows.len() == n {
        return Ok(rows.clone());
    }
    Err(format!(
        "stats.regress: {} regressor rows but {} observations",
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
