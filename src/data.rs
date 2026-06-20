//! The `data` built-in namespace: data-preparation helpers that sit in front of
//! the `stats` models. Column transforms (`standardize`, `center`, `rescale`)
//! stay exact — a z-score is `(x − μ)/σ` with `μ` rational and `σ` a surd, so
//! the result is an exact surd, not a rounded float. `dummy` one-hot-encodes a
//! categorical column (distinct values become indicator columns), and `groupby`
//! aggregates one column by the levels of another. Categories are just
//! symbol- (or number-) valued vector entries; no separate categorical type is
//! needed.

use crate::expr::*;
use num_bigint::BigInt;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &["standardize", "center", "rescale", "dummy", "groupby"];

pub fn call(name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    match name {
        "standardize" => standardize(&one_vector("data.standardize", &args)?),
        "center" => Ok(col(center(&one_vector("data.center", &args)?))),
        "rescale" => rescale(&one_vector("data.rescale", &args)?),
        "dummy" => dummy(&one_vector("data.dummy", &args)?),
        "groupby" => {
            let (keys, values) = two_vectors("data.groupby", &args)?;
            groupby(&keys, &values)
        }
        _ => Err(format!(
            "unknown function 'data.{}' (available: data.{})",
            name,
            FUNCTIONS.join(", data.")
        )),
    }
}

// -- column transforms --------------------------------------------------------

/// `(xᵢ − μ)/σ` with the sample standard deviation σ — an exact surd column.
fn standardize(xs: &[Expr]) -> Result<Expr, String> {
    if xs.len() < 2 {
        return Err("data.standardize expects at least 2 data points".into());
    }
    let var = variance(xs);
    if is_known_zero(&var) {
        return Err("data.standardize is undefined for constant data (zero variance)".into());
    }
    let inv_sd = pow(var, rat(-1, 2));
    Ok(col(centered(xs)
        .into_iter()
        .map(|c| mul(vec![c, inv_sd.clone()]))
        .collect()))
}

/// `xᵢ − μ` — the mean-centered column.
fn center(xs: &[Expr]) -> Vec<Expr> {
    centered(xs)
}

/// `(xᵢ − min)/(max − min)`, rescaled to [0, 1]. Numeric data only (it needs an
/// ordering to find the extremes).
fn rescale(xs: &[Expr]) -> Result<Expr, String> {
    let keys = numeric_keys("data.rescale", xs)?;
    let (lo, hi) = (
        keys.iter().cloned().min().unwrap(),
        keys.iter().cloned().max().unwrap(),
    );
    if lo == hi {
        return Err("data.rescale is undefined for constant data (zero range)".into());
    }
    let span = rat_to_expr(&hi - &lo);
    let min = rat_to_expr(lo);
    Ok(col(xs
        .iter()
        .map(|x| {
            mul(vec![
                add(vec![x.clone(), mul(vec![int(-1), min.clone()])]),
                pow(span.clone(), int(-1)),
            ])
        })
        .collect()))
}

// -- encoding and aggregation -------------------------------------------------

/// One-hot encode a categorical column: each distinct value (in first-seen
/// order) becomes a 0/1 indicator column. Returns `struct(levels, indicators)`.
fn dummy(xs: &[Expr]) -> Result<Expr, String> {
    let levels = distinct(xs);
    let indicators: Vec<Vec<Expr>> = xs
        .iter()
        .map(|x| {
            levels
                .iter()
                .map(|lv| if x == lv { int(1) } else { int(0) })
                .collect()
        })
        .collect();
    structure(vec![
        ("levels".into(), col(levels)),
        ("indicators".into(), Expr::Matrix(indicators)),
    ])
}

/// Aggregate `values` by the levels of `keys`. Returns
/// `struct(levels, count, sum, mean)`, one row per distinct key.
fn groupby(keys: &[Expr], values: &[Expr]) -> Result<Expr, String> {
    if keys.len() != values.len() {
        return Err(format!(
            "data.groupby expects matching lengths, got {} keys and {} values",
            keys.len(),
            values.len()
        ));
    }
    let levels = distinct(keys);
    let mut count = Vec::with_capacity(levels.len());
    let mut sum = Vec::with_capacity(levels.len());
    let mut mean = Vec::with_capacity(levels.len());
    for lv in &levels {
        let group: Vec<Expr> = keys
            .iter()
            .zip(values)
            .filter(|(k, _)| *k == lv)
            .map(|(_, v)| v.clone())
            .collect();
        let n = group.len();
        let total = add(group);
        count.push(int(n as i64));
        sum.push(total.clone());
        mean.push(mul(vec![inv_int(n), total]));
    }
    structure(vec![
        ("levels".into(), col(levels)),
        ("count".into(), col(count)),
        ("sum".into(), col(sum)),
        ("mean".into(), col(mean)),
    ])
}

// -- helpers ------------------------------------------------------------------

/// Distinct entries, in first-appearance order.
fn distinct(xs: &[Expr]) -> Vec<Expr> {
    let mut out: Vec<Expr> = Vec::new();
    for x in xs {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

fn mean_of(xs: &[Expr]) -> Expr {
    mul(vec![inv_int(xs.len()), add(xs.to_vec())])
}

fn centered(xs: &[Expr]) -> Vec<Expr> {
    let m = mean_of(xs);
    xs.iter()
        .map(|x| add(vec![x.clone(), mul(vec![int(-1), m.clone()])]))
        .collect()
}

/// Sample variance (n−1 denominator), each squared term expanded so symbolic
/// data tidies up.
fn variance(xs: &[Expr]) -> Expr {
    let c = centered(xs);
    let ss = add(c
        .iter()
        .map(|ci| expand(&mul(vec![ci.clone(), ci.clone()])))
        .collect());
    mul(vec![inv_int(xs.len() - 1), ss])
}

/// Exact numeric sort keys, erroring on anything unorderable (for min/max).
fn numeric_keys(name: &str, xs: &[Expr]) -> Result<Vec<BigRational>, String> {
    xs.iter()
        .map(|x| {
            match x {
                Expr::Float(bf, _) => float_to_rational(bf),
                other => numeric_value(other),
            }
            .ok_or_else(|| {
                format!(
                    "{} needs numeric entries; '{}' can't be ordered (try N(...))",
                    name, x
                )
            })
        })
        .collect()
}

fn col(v: Vec<Expr>) -> Expr {
    Expr::Matrix(v.into_iter().map(|e| vec![e]).collect())
}

fn inv_int(n: usize) -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(1), BigInt::from(n)))
}

fn rat(n: i64, d: i64) -> Expr {
    rat_to_expr(BigRational::new(BigInt::from(n), BigInt::from(d)))
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
    Ok((entries(name, &args[0])?, entries(name, &args[1])?))
}
