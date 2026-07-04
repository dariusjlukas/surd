//! The `data` built-in namespace: data-preparation helpers that sit in front of
//! the `stats` models. Column transforms (`standardize`, `center`, `rescale`)
//! stay exact — a z-score is `(x − μ)/σ` with `μ` rational and `σ` a surd, so
//! the result is an exact surd, not a rounded float. `dummy` one-hot-encodes a
//! categorical column (distinct values become indicator columns), and `groupby`
//! aggregates one column by the levels of another. Categories are just
//! symbol- (or number-) valued vector entries; no separate categorical type is
//! needed. `dropna` removes rows carrying the missing marker `NA` (which the
//! transforms here, like the `stats` models, refuse to compute through), and
//! `split` partitions rows into a seeded, reproducible train/test pair.

use crate::expr::*;
use crate::rng;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &[
    "standardize",
    "center",
    "rescale",
    "dummy",
    "groupby",
    "dropna",
    "split",
];

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
        "dropna" => {
            if args.len() != 1 {
                return Err(format!(
                    "data.dropna expects 1 argument(s), got {}",
                    args.len()
                ));
            }
            dropna(&args[0])
        }
        "split" => split(&args),
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

// -- missing data and splitting -------------------------------------------------

/// Remove missing values: `NA` entries from a vector, rows containing an `NA`
/// from a matrix, and — for a table (a struct of equal-length column vectors)
/// — every row where *any* column is `NA`, keeping the columns aligned
/// (listwise deletion).
fn dropna(v: &Expr) -> Result<Expr, String> {
    const ALL_GONE: &str = "data.dropna: every row has a missing value";
    match v {
        Expr::Struct(fields) => {
            let cols = table_columns("data.dropna", fields)?;
            let n = cols[0].1.len();
            let keep: Vec<usize> = (0..n)
                .filter(|&i| cols.iter().all(|(_, c)| !is_missing(&c[i])))
                .collect();
            if keep.is_empty() {
                return Err(ALL_GONE.into());
            }
            structure(
                cols.into_iter()
                    .map(|(name, c)| (name, col(keep.iter().map(|&i| c[i].clone()).collect())))
                    .collect(),
            )
        }
        // A 1×n row vector: filter entries, keeping the row shape.
        Expr::Matrix(rows) if rows.len() == 1 => {
            let kept: Vec<Expr> = rows[0].iter().filter(|e| !is_missing(e)).cloned().collect();
            if kept.is_empty() {
                return Err(ALL_GONE.into());
            }
            Ok(Expr::Matrix(vec![kept]))
        }
        Expr::Matrix(rows) => {
            let kept: Vec<Vec<Expr>> = rows
                .iter()
                .filter(|r| !r.iter().any(is_missing))
                .cloned()
                .collect();
            if kept.is_empty() {
                return Err(ALL_GONE.into());
            }
            Ok(Expr::Matrix(kept))
        }
        _ => Err(
            "data.dropna expects a vector, a matrix, or a struct of column vectors (a table)"
                .into(),
        ),
    }
}

/// `data.split(x, frac[, seed])` → `struct(train, test)`: a reproducible
/// random split of the rows of a table/matrix (or the entries of a vector).
/// `frac` is the train fraction, exact in (0, 1); the seed (default 0) drives
/// the deterministic shuffle, so the same call always produces the same
/// split. Membership is random but each side keeps the original row order.
fn split(args: &[Expr]) -> Result<Expr, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "data.split expects (data, fraction[, seed]), got {} argument(s)",
            args.len()
        ));
    }
    let frac = numeric_value(&args[1])
        .filter(|r| {
            *r > BigRational::from_integer(0.into()) && *r < BigRational::from_integer(1.into())
        })
        .ok_or("data.split: the train fraction must be an exact number strictly between 0 and 1")?;
    let seed = seed_arg("data.split", args.get(2))?;

    let n = row_count("data.split", &args[0])?;
    if n < 2 {
        return Err("data.split needs at least 2 rows".into());
    }
    // Train size ⌊frac·n + 1/2⌉ (round half up), rejected if a side comes out
    // empty — a split that isn't a split is a mistake worth hearing about.
    let half = BigRational::new(BigInt::from(1), BigInt::from(2));
    let ntrain = (frac * BigRational::from_integer(BigInt::from(n)) + half)
        .floor()
        .to_integer()
        .to_usize()
        .expect("train size fits usize: it is at most n");
    if ntrain == 0 || ntrain == n {
        return Err(format!(
            "data.split: that fraction of {} rows leaves the {} side empty",
            n,
            if ntrain == 0 { "train" } else { "test" }
        ));
    }

    let perm = rng::permutation(n, seed);
    let mut train = perm[..ntrain].to_vec();
    let mut test = perm[ntrain..].to_vec();
    train.sort_unstable();
    test.sort_unstable();
    structure(vec![
        ("train".into(), take_rows("data.split", &args[0], &train)?),
        ("test".into(), take_rows("data.split", &args[0], &test)?),
    ])
}

/// How many rows `x` has for splitting purposes: table rows (column length),
/// matrix rows, or the entries of a 1×n row vector.
fn row_count(name: &str, x: &Expr) -> Result<usize, String> {
    match x {
        Expr::Struct(fields) => Ok(table_columns(name, fields)?[0].1.len()),
        Expr::Matrix(rows) if rows.len() == 1 => Ok(rows[0].len()),
        Expr::Matrix(rows) => Ok(rows.len()),
        _ => Err(format!(
            "{} expects a vector, a matrix, or a struct of column vectors (a table)",
            name
        )),
    }
}

/// The subset of `x`'s rows at `idx` (ascending), in `x`'s own shape.
fn take_rows(name: &str, x: &Expr, idx: &[usize]) -> Result<Expr, String> {
    match x {
        Expr::Struct(fields) => structure(
            table_columns(name, fields)?
                .into_iter()
                .map(|(fname, c)| (fname, col(idx.iter().map(|&i| c[i].clone()).collect())))
                .collect(),
        ),
        Expr::Matrix(rows) if rows.len() == 1 => Ok(Expr::Matrix(vec![idx
            .iter()
            .map(|&i| rows[0][i].clone())
            .collect()])),
        Expr::Matrix(rows) => Ok(Expr::Matrix(idx.iter().map(|&i| rows[i].clone()).collect())),
        _ => unreachable!("row_count already vetted the shape"),
    }
}

/// The named columns of a table struct, each flattened to its entries, all
/// required to be vectors of one shared length.
fn table_columns(
    name: &str,
    fields: &[(String, Expr)],
) -> Result<Vec<(String, Vec<Expr>)>, String> {
    let mut out: Vec<(String, Vec<Expr>)> = Vec::with_capacity(fields.len());
    for (fname, v) in fields {
        let c = entries_raw(v).ok_or_else(|| {
            format!(
                "{}: struct field '{}' is not a column vector — a table is a struct of columns",
                name, fname
            )
        })?;
        out.push((fname.clone(), c));
    }
    let n = out[0].1.len();
    if let Some((bad, c)) = out.iter().find(|(_, c)| c.len() != n) {
        return Err(format!(
            "{}: columns must have equal lengths, but '{}' has {} rows and '{}' has {}",
            name,
            out[0].0,
            n,
            bad,
            c.len()
        ));
    }
    Ok(out)
}

/// The optional seed argument: a nonnegative integer, default 0. (Also used
/// by `stats.cv`, the other seeded shuffle.)
pub(crate) fn seed_arg(name: &str, e: Option<&Expr>) -> Result<u64, String> {
    match e {
        None => Ok(0),
        Some(e) => numeric_value(e)
            .filter(|r| r.is_integer())
            .and_then(|r| r.to_integer().to_u64())
            .ok_or_else(|| {
                format!(
                    "{}: the seed must be a nonnegative integer below 2^64, got '{}'",
                    name, e
                )
            }),
    }
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

/// A vector's entries with no vetting — for `dropna`/`split`, which must be
/// able to look at data that still carries `NA`.
fn entries_raw(e: &Expr) -> Option<Vec<Expr>> {
    let Expr::Matrix(rows) = e else { return None };
    if rows.len() == 1 {
        Some(rows[0].clone())
    } else if rows.iter().all(|r| r.len() == 1) {
        Some(rows.iter().map(|r| r[0].clone()).collect())
    } else {
        None
    }
}

fn entries(name: &str, e: &Expr) -> Result<Vec<Expr>, String> {
    let Expr::Matrix(rows) = e else {
        return Err(format!("{} expects a vector (a 1×n or n×1 matrix)", name));
    };
    let v = entries_raw(e).ok_or_else(|| {
        format!(
            "{} expects a vector (a 1×n or n×1 matrix), got a {}×{} matrix",
            name,
            rows.len(),
            rows[0].len()
        )
    })?;
    no_missing(name, &v)?;
    Ok(v)
}

/// Refuse data that still carries the missing marker. `NA` is an ordinary
/// symbol to the algebra — computing a mean "through" one would produce
/// well-formed nonsense — so everything statistical stops here instead.
fn no_missing(name: &str, xs: &[Expr]) -> Result<(), String> {
    let n = xs.iter().filter(|e| is_missing(e)).count();
    if n > 0 {
        return Err(format!(
            "{}: the data has {} missing value{} (NA) — drop the affected rows first \
             with data.dropna(...)",
            name,
            n,
            if n == 1 { "" } else { "s" }
        ));
    }
    Ok(())
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
