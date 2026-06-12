//! The `stats` built-in namespace: exact statistics.
//!
//! Every estimator runs in exact arithmetic: the mean of rationals is a
//! rational, a variance is a rational, and a standard deviation is an exact
//! surd — `stats.std([1; 2; 3; 4])` is `sqrt(5/3)`, with `N(...)` taking it
//! to floats only on request. `var`, `std`, `cov`, and `cor` are the
//! *sample* estimators (n−1 denominator). Symbolic entries flow through
//! everything that doesn't need ordering; `median` requires numeric data.

use crate::expr::*;
use num_bigint::BigInt;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &["mean", "median", "var", "std", "cov", "cor", "linfit"];

pub fn call(name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    match name {
        "mean" => {
            let xs = one_vector("stats.mean", &args)?;
            Ok(mean_of(&xs))
        }
        "median" => median(&one_vector("stats.median", &args)?),
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
    Ok(mul(vec![
        cov,
        pow(va, neg_half()),
        pow(vb, neg_half()),
    ]))
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
    let mut keyed: Vec<(BigRational, &Expr)> = Vec::with_capacity(xs.len());
    for x in xs {
        let key = match x {
            Expr::Float(bf, _) => float_to_rational(bf),
            other => numeric_value(other),
        }
        .ok_or_else(|| {
            format!(
                "stats.median needs numeric entries; '{}' can't be ordered (try N(...))",
                x
            )
        })?;
        keyed.push((key, x));
    }
    keyed.sort_by(|p, q| p.0.cmp(&q.0));
    let n = keyed.len();
    Ok(if n % 2 == 1 {
        keyed[n / 2].1.clone()
    } else {
        mul(vec![
            inv_int(2),
            add(vec![keyed[n / 2 - 1].1.clone(), keyed[n / 2].1.clone()]),
        ])
    })
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
        return Err(format!("{} expects 1 argument(s), got {}", name, args.len()));
    }
    entries(name, &args[0])
}

fn two_vectors(name: &str, args: &[Expr]) -> Result<(Vec<Expr>, Vec<Expr>), String> {
    if args.len() != 2 {
        return Err(format!("{} expects 2 argument(s), got {}", name, args.len()));
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
