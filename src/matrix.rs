//! Exact linear algebra over the field of expressions.
//!
//! Entries are general [`Expr`], so everything works symbolically; the
//! flagship "exact ℚ" case is just when every entry is a number, where the
//! arithmetic folds to exact rationals with no rounding.
//!
//! All operations build their results through the canonical smart constructors
//! ([`add`], [`mul`], [`pow`]) so results stay in canonical form for free.

use crate::expr::*;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, ToPrimitive, Zero};

pub fn is_matrix(e: &Expr) -> bool {
    matches!(e, Expr::Matrix(_))
}

/// Validate and build a matrix from rows. Rejects empty or ragged input.
pub fn matrix(rows: Vec<Vec<Expr>>) -> Result<Expr, String> {
    if rows.is_empty() || rows[0].is_empty() {
        return Err("a matrix needs at least one entry".into());
    }
    let cols = rows[0].len();
    if rows.iter().any(|r| r.len() != cols) {
        return Err("every row of a matrix must have the same number of entries".into());
    }
    Ok(Expr::Matrix(rows))
}

fn rows_of(e: &Expr) -> &Vec<Vec<Expr>> {
    match e {
        Expr::Matrix(r) => r,
        _ => unreachable!("rows_of called on a non-matrix"),
    }
}

/// (rows, columns)
fn dims(e: &Expr) -> (usize, usize) {
    let r = rows_of(e);
    (r.len(), r[0].len())
}

/// Whether we can see that an entry is zero. Numeric zeros only — an arbitrary
/// symbolic expression's zero-ness is undecidable (Richardson), so symbolic
/// elimination here is best-effort.
fn is_known_zero(e: &Expr) -> bool {
    numeric_value(e).is_some_and(|r| r.is_zero())
}

// ---------------------------------------------------------------------------
// Elementwise / product arithmetic
// ---------------------------------------------------------------------------

pub fn mat_add(a: &Expr, b: &Expr, subtract: bool) -> Result<Expr, String> {
    if dims(a) != dims(b) {
        let (ar, ac) = dims(a);
        let (br, bc) = dims(b);
        return Err(format!(
            "cannot {} a {}×{} and a {}×{} matrix",
            if subtract { "subtract" } else { "add" },
            ar,
            ac,
            br,
            bc
        ));
    }
    let rows = rows_of(a)
        .iter()
        .zip(rows_of(b))
        .map(|(x, y)| {
            x.iter()
                .zip(y)
                .map(|(p, q)| {
                    if subtract {
                        add(vec![p.clone(), mul(vec![int(-1), q.clone()])])
                    } else {
                        add(vec![p.clone(), q.clone()])
                    }
                })
                .collect()
        })
        .collect();
    Ok(Expr::Matrix(rows))
}

pub fn scalar_mul(s: &Expr, m: &Expr) -> Expr {
    Expr::Matrix(map_entries(rows_of(m), |e| mul(vec![s.clone(), e.clone()])))
}

pub fn mat_mul(a: &Expr, b: &Expr) -> Result<Expr, String> {
    let (m, k) = dims(a);
    let (k2, n) = dims(b);
    if k != k2 {
        return Err(format!(
            "cannot multiply a {}×{} by a {}×{} matrix (inner dimensions {} and {} differ)",
            m, k, k2, n, k, k2
        ));
    }
    let (ar, br) = (rows_of(a), rows_of(b));
    let mut rows = Vec::with_capacity(m);
    for i in 0..m {
        let mut row = Vec::with_capacity(n);
        for j in 0..n {
            let terms = (0..k)
                .map(|t| mul(vec![ar[i][t].clone(), br[t][j].clone()]))
                .collect();
            row.push(add(terms));
        }
        rows.push(row);
    }
    Ok(Expr::Matrix(rows))
}

pub fn transpose(m: &Expr) -> Expr {
    let r = rows_of(m);
    let (rn, cn) = (r.len(), r[0].len());
    let mut out = Vec::with_capacity(cn);
    for j in 0..cn {
        out.push((0..rn).map(|i| r[i][j].clone()).collect());
    }
    Expr::Matrix(out)
}

pub fn identity(n: usize) -> Expr {
    let rows = (0..n)
        .map(|i| (0..n).map(|j| int(if i == j { 1 } else { 0 })).collect())
        .collect();
    Expr::Matrix(rows)
}

/// Integer matrix power. Negative powers invert first.
pub fn mat_pow(m: &Expr, n: i64) -> Result<Expr, String> {
    let (r, c) = dims(m);
    if r != c {
        return Err("only a square matrix can be raised to a power".into());
    }
    if n.unsigned_abs() > 100_000 {
        return Err("matrix exponent is too large".into());
    }
    if n == 0 {
        return Ok(identity(r));
    }
    let base = if n < 0 { inverse(m)? } else { m.clone() };
    let mut acc = identity(r);
    for _ in 0..n.unsigned_abs() {
        acc = mat_mul(&acc, &base)?;
    }
    Ok(acc)
}

// ---------------------------------------------------------------------------
// Gauss-Jordan elimination — the workhorse for inverse / solve / rank / rref
// ---------------------------------------------------------------------------

/// Reduce to reduced row echelon form over the field of expressions. Exact for
/// rational entries. Returns (reduced rows, rank, pivot columns in row order).
fn gauss_jordan(mut m: Vec<Vec<Expr>>) -> (Vec<Vec<Expr>>, usize, Vec<usize>) {
    let rows = m.len();
    let cols = m[0].len();
    let mut pivot_row = 0;
    let mut pivots = Vec::new();

    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }
        // Find a usable pivot at or below pivot_row.
        let sel = (pivot_row..rows).find(|&r| !is_known_zero(&m[r][col]));
        let sel = match sel {
            Some(r) => r,
            None => continue,
        };
        m.swap(pivot_row, sel);

        // Scale the pivot row so the pivot becomes exactly 1.
        let inv_pivot = pow(m[pivot_row][col].clone(), int(-1));
        for j in 0..cols {
            m[pivot_row][j] = mul(vec![inv_pivot.clone(), m[pivot_row][j].clone()]);
        }

        // Eliminate this column from every other row.
        for r in 0..rows {
            if r == pivot_row {
                continue;
            }
            let factor = m[r][col].clone();
            if is_known_zero(&factor) {
                continue;
            }
            for j in 0..cols {
                let scaled = mul(vec![factor.clone(), m[pivot_row][j].clone()]);
                m[r][j] = add(vec![m[r][j].clone(), mul(vec![int(-1), scaled])]);
            }
        }

        pivots.push(col);
        pivot_row += 1;
    }

    let rank = pivots.len();
    (m, rank, pivots)
}

pub fn rref(m: &Expr) -> Expr {
    let (reduced, _, _) = gauss_jordan(rows_of(m).clone());
    Expr::Matrix(reduced)
}

pub fn rank(m: &Expr) -> Expr {
    let (_, rk, _) = gauss_jordan(rows_of(m).clone());
    int(rk as i64)
}

pub fn inverse(m: &Expr) -> Result<Expr, String> {
    let (n, c) = dims(m);
    if n != c {
        return Err("only a square matrix can be inverted".into());
    }
    // Build the augmented matrix [A | I] and reduce.
    let id = rows_of(&identity(n)).clone();
    let aug: Vec<Vec<Expr>> = rows_of(m)
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut full = row.clone();
            full.extend(id[i].clone());
            full
        })
        .collect();
    let (reduced, _, pivots) = gauss_jordan(aug);

    // Invertible iff the left n×n block reduced to the identity, i.e. there is
    // a pivot in each of the first n columns. (Pivots can appear in the
    // appended identity columns even when A is singular, so count carefully.)
    let left_pivots = pivots.iter().filter(|&&col| col < n).count();
    if left_pivots < n {
        return Err("matrix is singular (no inverse)".into());
    }
    let inv = reduced.into_iter().map(|row| row[n..].to_vec()).collect();
    Ok(Expr::Matrix(inv))
}

/// Solve `A x = b` for a column vector `b`. Reports inconsistent and
/// underdetermined systems rather than guessing.
pub fn solve(a: &Expr, b: &Expr) -> Result<Expr, String> {
    if !is_matrix(a) || !is_matrix(b) {
        return Err("solve expects two matrices: solve(A, b)".into());
    }
    let (n, vars) = dims(a);
    let (bn, bc) = dims(b);
    if bc != 1 {
        return Err("the right-hand side of solve must be a column vector".into());
    }
    if bn != n {
        return Err(format!(
            "dimension mismatch: A has {} rows but b has {}",
            n, bn
        ));
    }

    let aug: Vec<Vec<Expr>> = rows_of(a)
        .iter()
        .zip(rows_of(b))
        .map(|(row, brow)| {
            let mut full = row.clone();
            full.push(brow[0].clone());
            full
        })
        .collect();
    let (reduced, _, pivots) = gauss_jordan(aug);

    // A pivot in the final (augmented) column means 0 = nonzero.
    if pivots.contains(&vars) {
        return Err("system is inconsistent (no solution)".into());
    }
    let var_pivots: Vec<usize> = pivots.iter().cloned().filter(|&c| c < vars).collect();
    if var_pivots.len() < vars {
        return Err("system is underdetermined (infinitely many solutions)".into());
    }

    // Unique solution: pivots are in row order, so row k solves variable
    // var_pivots[k], and the value sits in the augmented column.
    let mut sol = vec![int(0); vars];
    for (k, &col) in var_pivots.iter().enumerate() {
        sol[col] = reduced[k][vars].clone();
    }
    Ok(Expr::Matrix(sol.into_iter().map(|e| vec![e]).collect()))
}

// ---------------------------------------------------------------------------
// Determinant: Bareiss for numeric matrices, cofactor for symbolic ones
// ---------------------------------------------------------------------------

pub fn det(m: &Expr) -> Result<Expr, String> {
    let (n, c) = dims(m);
    if n != c {
        return Err("only a square matrix has a determinant".into());
    }
    let rows = rows_of(m);
    if is_numeric_matrix(rows) {
        Ok(bareiss_det(rows))
    } else {
        Ok(cofactor_det(rows))
    }
}

fn is_numeric_matrix(rows: &[Vec<Expr>]) -> bool {
    rows.iter().flatten().all(|e| numeric_value(e).is_some())
}

/// Fraction-free (Bareiss) determinant. Every division is exact, so for integer
/// entries the intermediate values stay integers — no coefficient blow-up.
fn bareiss_det(rows: &[Vec<Expr>]) -> Expr {
    let n = rows.len();
    let mut m = rows.to_vec();
    let mut sign = 1i64;
    let mut prev = int(1);

    for k in 0..n {
        if is_known_zero(&m[k][k]) {
            // Swap in a nonzero pivot from below, flipping the sign.
            match (k + 1..n).find(|&r| !is_known_zero(&m[r][k])) {
                Some(r) => {
                    m.swap(k, r);
                    sign = -sign;
                }
                None => return int(0), // an all-zero column ⇒ determinant 0
            }
        }
        for i in k + 1..n {
            for j in k + 1..n {
                // m[i][j] = (m[i][j]·m[k][k] − m[i][k]·m[k][j]) / prev
                let cross = add(vec![
                    mul(vec![m[i][j].clone(), m[k][k].clone()]),
                    mul(vec![int(-1), m[i][k].clone(), m[k][j].clone()]),
                ]);
                m[i][j] = mul(vec![cross, pow(prev.clone(), int(-1))]);
            }
        }
        prev = m[k][k].clone();
    }

    let d = m[n - 1][n - 1].clone();
    if sign < 0 {
        mul(vec![int(-1), d])
    } else {
        d
    }
}

/// Laplace cofactor expansion. Division-free, so it's exact and well-behaved on
/// symbolic entries (at O(n!) cost — fine for the small matrices this targets).
fn cofactor_det(rows: &[Vec<Expr>]) -> Expr {
    let n = rows.len();
    match n {
        1 => rows[0][0].clone(),
        2 => add(vec![
            mul(vec![rows[0][0].clone(), rows[1][1].clone()]),
            mul(vec![int(-1), rows[0][1].clone(), rows[1][0].clone()]),
        ]),
        _ => {
            let terms = (0..n)
                .map(|j| {
                    let minor = cofactor_det(&minor_matrix(rows, 0, j));
                    let entry = rows[0][j].clone();
                    if j % 2 == 0 {
                        mul(vec![entry, minor])
                    } else {
                        mul(vec![int(-1), entry, minor])
                    }
                })
                .collect();
            add(terms)
        }
    }
}

fn minor_matrix(rows: &[Vec<Expr>], skip_r: usize, skip_c: usize) -> Vec<Vec<Expr>> {
    rows.iter()
        .enumerate()
        .filter(|(i, _)| *i != skip_r)
        .map(|(_, row)| {
            row.iter()
                .enumerate()
                .filter(|(j, _)| *j != skip_c)
                .map(|(_, e)| e.clone())
                .collect()
        })
        .collect()
}

/// Convert a scalar expression to a non-negative `i64` exponent for `mat_pow`.
pub fn integer_exponent(e: &Expr) -> Option<i64> {
    let r = numeric_value(e)?;
    if r.is_integer() {
        r.to_integer().to_i64()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Characteristic polynomial and eigenvalues
// ---------------------------------------------------------------------------

/// The characteristic polynomial det(A − λI), expanded, as a polynomial in `var`.
/// Works for symbolic matrices too — it's just `det` of a matrix whose entries
/// happen to contain the symbol `var`.
pub fn char_poly(a: &Expr, var: &str) -> Result<Expr, String> {
    let (n, c) = dims(a);
    if n != c {
        return Err("the characteristic polynomial needs a square matrix".into());
    }
    let rows = rows_of(a);
    let lambda = Expr::Symbol(var.to_string());
    let mut m = Vec::with_capacity(n);
    for (i, row) in rows.iter().enumerate() {
        let mut new_row = Vec::with_capacity(n);
        for (j, entry) in row.iter().enumerate() {
            new_row.push(if i == j {
                add(vec![entry.clone(), mul(vec![int(-1), lambda.clone()])])
            } else {
                entry.clone()
            });
        }
        m.push(new_row);
    }
    // Entries contain λ, so this is the symbolic (cofactor) path; expand to a
    // flat polynomial.
    Ok(expand(&cofactor_det(&m)))
}

/// Exact eigenvalues of a numeric matrix, returned as a column vector.
///
/// Roots of the characteristic polynomial are found exactly when it factors
/// over ℚ into linear and quadratic pieces (rational-root search + the
/// quadratic formula). Irreducible factors of degree ≥ 3 (Abel–Ruffini for
/// ≥ 5) and complex roots are reported, not approximated.
pub fn eigenvalues(a: &Expr) -> Result<Expr, String> {
    let (n, c) = dims(a);
    if n != c {
        return Err("eigenvalues need a square matrix".into());
    }
    let cp = char_poly(a, "lambda")?;
    let coeffs = poly_coeffs(&cp, "lambda")
        .ok_or("eigenvalues are only supported for matrices with numeric entries")?;
    let roots = roots_of_poly(coeffs)?;
    Ok(Expr::Matrix(roots.into_iter().map(|r| vec![r]).collect()))
}

/// Coefficients [c0, c1, …, cn] of a univariate rational polynomial, or `None`
/// if `e` isn't a polynomial in `var` with rational coefficients.
fn poly_coeffs(e: &Expr, var: &str) -> Option<Vec<BigRational>> {
    let terms: Vec<&Expr> = match e {
        Expr::Add(ts) => ts.iter().collect(),
        other => vec![other],
    };
    let mut coeffs: Vec<BigRational> = Vec::new();
    for t in terms {
        let (power, coeff) = monomial(t, var)?;
        if coeffs.len() <= power {
            coeffs.resize(power + 1, BigRational::zero());
        }
        coeffs[power] += coeff;
    }
    if coeffs.is_empty() {
        coeffs.push(BigRational::zero());
    }
    Some(coeffs)
}

/// Decompose one term into (power of `var`, rational coefficient).
fn monomial(t: &Expr, var: &str) -> Option<(usize, BigRational)> {
    if !contains_symbol(t, var) {
        return Some((0, numeric_value(t)?));
    }
    match t {
        Expr::Symbol(s) if s == var => Some((1, BigRational::one())),
        Expr::Pow(..) => var_power(t, var).map(|k| (k, BigRational::one())),
        Expr::Mul(fs) => {
            let mut coeff = BigRational::one();
            let mut power = 0usize;
            let mut seen = false;
            for f in fs {
                if let Some(r) = numeric_value(f) {
                    coeff *= r;
                } else if let Some(k) = var_power(f, var) {
                    if seen {
                        return None; // var appears twice → not canonical / not a monomial
                    }
                    seen = true;
                    power = k;
                } else {
                    return None; // some other symbol → not a polynomial in `var` alone
                }
            }
            Some((power, coeff))
        }
        _ => None,
    }
}

/// If `f` is `var` or `var^k` (k a non-negative integer), return k.
fn var_power(f: &Expr, var: &str) -> Option<usize> {
    match f {
        Expr::Symbol(s) if s == var => Some(1),
        Expr::Pow(b, ex) => {
            if let Expr::Symbol(s) = &**b {
                if s == var {
                    let k = numeric_value(ex)?;
                    if k.is_integer() && k >= BigRational::zero() {
                        return k.to_integer().to_usize();
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Find every root we can express exactly, peeling off rational roots and
/// finishing any final linear/quadratic factor.
fn roots_of_poly(mut c: Vec<BigRational>) -> Result<Vec<Expr>, String> {
    trim_leading_zeros(&mut c);
    let mut roots = Vec::new();
    loop {
        trim_leading_zeros(&mut c);
        match c.len() - 1 {
            0 => break,
            1 => {
                roots.push(rat_to_expr(-c[0].clone() / c[1].clone()));
                break;
            }
            2 => {
                roots.append(&mut quad_roots(&c[2], &c[1], &c[0])?);
                break;
            }
            deg => match find_rational_root(&c) {
                Some(r) => {
                    roots.push(rat_to_expr(r.clone()));
                    c = synthetic_divide(&c, &r);
                }
                None => {
                    return Err(format!(
                        "could not find all eigenvalues exactly: after {} rational root(s), a \
                         degree-{} factor remains with no rational roots (cubics/quartics and \
                         the radical-free degree ≥ 5 case aren't implemented)",
                        roots.len(),
                        deg
                    ));
                }
            },
        }
    }
    Ok(roots)
}

fn trim_leading_zeros(c: &mut Vec<BigRational>) {
    while c.len() > 1 && c.last().unwrap().is_zero() {
        c.pop();
    }
}

/// Roots of a·x² + b·x + c, exact: rational, real-irrational (via `sqrt`), or a
/// complex-conjugate pair when the discriminant is negative.
fn quad_roots(a: &BigRational, b: &BigRational, c: &BigRational) -> Result<Vec<Expr>, String> {
    let four = BigRational::from_integer(BigInt::from(4));
    let two = BigRational::from_integer(BigInt::from(2));
    let disc = b.clone() * b.clone() - four * a.clone() * c.clone();
    let two_a_inv = pow(rat_to_expr(two * a.clone()), int(-1));
    let neg_b = rat_to_expr(-b.clone());

    if disc < BigRational::zero() {
        // (−b ± i·sqrt(−disc)) / (2a) — a complex-conjugate pair.
        let real_term = mul(vec![neg_b, two_a_inv.clone()]);
        let imag_term = mul(vec![pow(rat_to_expr(-disc), half()), two_a_inv]);
        return Ok(vec![
            complex(real_term.clone(), imag_term.clone()),
            complex(real_term, mul(vec![int(-1), imag_term])),
        ]);
    }

    // Real roots, kept in the compact (−b ± sqrt(disc))/(2a) factored form.
    let sqrt_disc = pow(rat_to_expr(disc), half());
    Ok(vec![
        mul(vec![add(vec![neg_b.clone(), sqrt_disc.clone()]), two_a_inv.clone()]),
        mul(vec![add(vec![neg_b, mul(vec![int(-1), sqrt_disc])]), two_a_inv]),
    ])
}

fn eval_poly(c: &[BigRational], x: &BigRational) -> BigRational {
    let mut acc = BigRational::zero();
    for coeff in c.iter().rev() {
        acc = acc * x.clone() + coeff.clone();
    }
    acc
}

/// Scale a rational polynomial to integer coefficients (multiply through by the
/// lcm of the denominators).
fn clear_denominators(c: &[BigRational]) -> Vec<BigInt> {
    let mut lcm = BigInt::from(1);
    for r in c {
        lcm = lcm.lcm(r.denom());
    }
    c.iter().map(|r| r.numer() * &lcm / r.denom()).collect()
}

/// Rational-root theorem: test p/q with p | a0 and q | an.
fn find_rational_root(c: &[BigRational]) -> Option<BigRational> {
    let ic = clear_denominators(c);
    if ic[0].is_zero() {
        return Some(BigRational::zero()); // x = 0 is a root
    }
    let a0 = ic[0].to_i64()?;
    let an = ic.last().unwrap().to_i64()?;
    if a0.abs() > 10_000_000 || an.abs() > 10_000_000 {
        return None; // avoid an enormous divisor search
    }
    for p in divisors(a0) {
        for q in divisors(an) {
            for &sign in &[1i64, -1] {
                let cand = BigRational::new(BigInt::from(sign * p), BigInt::from(q));
                if eval_poly(c, &cand).is_zero() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

fn divisors(n: i64) -> Vec<i64> {
    let n = n.abs();
    let mut out = Vec::new();
    let mut d = 1i64;
    while d * d <= n {
        if n % d == 0 {
            out.push(d);
            if d != n / d {
                out.push(n / d);
            }
        }
        d += 1;
    }
    out
}

/// Divide `c` (ascending coeffs) by (x − r); returns the quotient (ascending).
fn synthetic_divide(c: &[BigRational], r: &BigRational) -> Vec<BigRational> {
    let desc: Vec<BigRational> = c.iter().rev().cloned().collect();
    let mut b = Vec::with_capacity(desc.len());
    b.push(desc[0].clone());
    for k in 1..desc.len() {
        b.push(desc[k].clone() + r.clone() * b[k - 1].clone());
    }
    b.pop(); // drop the remainder
    b.reverse();
    b
}
