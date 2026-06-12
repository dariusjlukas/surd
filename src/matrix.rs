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

/// 1-based indexing. One index reads a vector element — or a whole row of a
/// general matrix; two indices read an element as (row, column).
pub fn index(m: &Expr, idxs: &[usize]) -> Result<Expr, String> {
    let Expr::Matrix(rows) = m else {
        return Err(format!("cannot index into '{}' (not a matrix)", m));
    };
    let (nr, nc) = (rows.len(), rows[0].len());
    let check = |i: usize, n: usize, what: &str| {
        if (1..=n).contains(&i) {
            Ok(())
        } else {
            Err(format!("index {} is out of range ({} has {})", i, what, n))
        }
    };
    match idxs {
        [i] if nr == 1 => {
            check(*i, nc, "the vector")?;
            Ok(rows[0][i - 1].clone())
        }
        [i] if nc == 1 => {
            check(*i, nr, "the vector")?;
            Ok(rows[i - 1][0].clone())
        }
        [i] => {
            check(*i, nr, "the matrix")?;
            Ok(Expr::Matrix(vec![rows[i - 1].clone()]))
        }
        [i, j] => {
            check(*i, nr, "the matrix's rows")?;
            check(*j, nc, "the matrix's columns")?;
            Ok(rows[i - 1][j - 1].clone())
        }
        _ => Err("indexing takes 1 index (vector element / matrix row) or 2 (row, column)".into()),
    }
}

/// The entries of a 1×n or n×1 matrix, in order. `None` for anything else.
pub fn vector_of(e: &Expr) -> Option<Vec<Expr>> {
    let Expr::Matrix(rows) = e else { return None };
    if rows.len() == 1 {
        Some(rows[0].clone())
    } else if rows.iter().all(|r| r.len() == 1) {
        Some(rows.iter().map(|r| r[0].clone()).collect())
    } else {
        None
    }
}

/// Apply a fallible scalar function entrywise, preserving shape.
pub fn try_map(m: &Expr, mut f: impl FnMut(&Expr) -> Result<Expr, String>) -> Result<Expr, String> {
    let rows = rows_of(m);
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut new_row = Vec::with_capacity(row.len());
        for cell in row {
            new_row.push(f(cell)?);
        }
        out.push(new_row);
    }
    Ok(Expr::Matrix(out))
}

/// Zip two same-shape matrices entrywise through a fallible scalar function.
pub fn try_zip(
    a: &Expr,
    b: &Expr,
    mut f: impl FnMut(&Expr, &Expr) -> Result<Expr, String>,
) -> Result<Expr, String> {
    if dims(a) != dims(b) {
        let ((ar, ac), (br, bc)) = (dims(a), dims(b));
        return Err(format!(
            "elementwise operation needs matching shapes, got {}×{} and {}×{}",
            ar, ac, br, bc
        ));
    }
    let mut out = Vec::new();
    for (ra, rb) in rows_of(a).iter().zip(rows_of(b)) {
        let mut row = Vec::with_capacity(ra.len());
        for (x, y) in ra.iter().zip(rb) {
            row.push(f(x, y)?);
        }
        out.push(row);
    }
    Ok(Expr::Matrix(out))
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

/// A basis for the null space (kernel) of `m`, returned as the columns of a
/// matrix. A trivial kernel is an error — there is no empty matrix to return,
/// and "only the zero vector" deserves to be said in words anyway.
pub fn nullspace(m: &Expr) -> Result<Expr, String> {
    let cols = rows_of(m)[0].len();
    let (reduced, _, pivots) = gauss_jordan(rows_of(m).clone());
    let basis = kernel_basis(&reduced, &pivots, cols);
    if basis.is_empty() {
        return Err(format!(
            "the null space is trivial: the matrix has full column rank ({}), so A·x = 0 only for x = 0",
            cols
        ));
    }
    Ok(columns_to_matrix(basis))
}

/// Read a kernel basis off a reduced row echelon form: for each non-pivot
/// (free) column f, the vector with 1 in slot f and −R[k][f] in the slot of
/// the k-th pivot column solves R·x = 0. Considers only the first `cols`
/// columns, so it works on augmented matrices too.
fn kernel_basis(reduced: &[Vec<Expr>], pivots: &[usize], cols: usize) -> Vec<Vec<Expr>> {
    (0..cols)
        .filter(|c| !pivots.contains(c))
        .map(|f| {
            let mut v = vec![int(0); cols];
            v[f] = int(1);
            for (k, &p) in pivots.iter().enumerate() {
                v[p] = mul(vec![int(-1), reduced[k][f].clone()]);
            }
            v
        })
        .collect()
}

/// Assemble column vectors into a matrix (the columns become, well, columns).
fn columns_to_matrix(cols: Vec<Vec<Expr>>) -> Expr {
    let n = cols[0].len();
    Expr::Matrix(
        (0..n)
            .map(|i| cols.iter().map(|c| c[i].clone()).collect())
            .collect(),
    )
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

/// Solve `A x = b` for a column vector `b`. Reports inconsistent systems; an
/// underdetermined system returns the *general* solution as a struct — a
/// particular solution plus a null-space basis (every solution is
/// `particular + any combination of the nullspace columns`).
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

    // Pivots are in row order, so row k solves variable var_pivots[k], and its
    // value sits in the augmented column. Free variables are pinned to 0,
    // making this the unique solution when there are no free variables and a
    // particular solution otherwise.
    let mut sol = vec![int(0); vars];
    for (k, &col) in var_pivots.iter().enumerate() {
        sol[col] = reduced[k][vars].clone();
    }
    let particular = Expr::Matrix(sol.into_iter().map(|e| vec![e]).collect());
    if var_pivots.len() == vars {
        return Ok(particular);
    }

    // Underdetermined: the general solution is particular + span(nullspace).
    let basis = columns_to_matrix(kernel_basis(&reduced, &var_pivots, vars));
    structure(vec![
        ("particular".to_string(), particular),
        ("nullspace".to_string(), basis),
    ])
}

// ---------------------------------------------------------------------------
// Decompositions: LU (Doolittle with row pivoting) and QR (Gram-Schmidt)
// ---------------------------------------------------------------------------

/// LU decomposition with row pivoting: `struct(L, U, P)` with P·A = L·U,
/// L unit lower triangular, U upper triangular. Exact; singular matrices
/// work too (an all-zero pivot column is simply skipped past).
pub fn lu(a: &Expr) -> Result<Expr, String> {
    let (n, c) = dims(a);
    if n != c {
        return Err("lu needs a square matrix".into());
    }
    let mut u = rows_of(a).clone();
    let mut l = rows_of(&identity(n)).clone();
    let mut perm: Vec<usize> = (0..n).collect();

    for k in 0..n {
        // First usably-nonzero pivot at or below the diagonal.
        let Some(sel) = (k..n).find(|&r| !is_known_zero(&u[r][k])) else {
            continue; // column already eliminated — U[k][k] stays 0
        };
        if sel != k {
            u.swap(k, sel);
            perm.swap(k, sel);
            // Already-computed multipliers travel with their rows.
            for j in 0..k {
                let tmp = l[k][j].clone();
                l[k][j] = l[sel][j].clone();
                l[sel][j] = tmp;
            }
        }
        let inv_pivot = pow(u[k][k].clone(), int(-1));
        for i in k + 1..n {
            if is_known_zero(&u[i][k]) {
                continue;
            }
            let factor = mul(vec![u[i][k].clone(), inv_pivot.clone()]);
            l[i][k] = factor.clone();
            // Zero by construction — set it outright so a symbolic entry
            // can't leave behind an unsimplified residue.
            u[i][k] = int(0);
            for j in k + 1..n {
                let scaled = mul(vec![factor.clone(), u[k][j].clone()]);
                u[i][j] = add(vec![u[i][j].clone(), mul(vec![int(-1), scaled])]);
            }
        }
    }

    // P has a 1 at (i, perm[i]): row i of P·A is original row perm[i].
    let p = Expr::Matrix(
        (0..n)
            .map(|i| (0..n).map(|j| int((j == perm[i]) as i64)).collect())
            .collect(),
    );
    structure(vec![
        ("L".to_string(), Expr::Matrix(l)),
        ("U".to_string(), Expr::Matrix(u)),
        ("P".to_string(), p),
    ])
}

/// QR decomposition by exact Gram-Schmidt: `struct(Q, R)` with A = Q·R, the
/// columns of Q orthonormal and R upper triangular. Column norms are square
/// roots, so Q and R hold exact surds — Qᵀ·Q folds to the identity exactly.
/// Projections happen on the *unnormalized* orthogonal columns, which stay in
/// the base field, so radicals only ever enter at the final normalization.
pub fn qr(a: &Expr) -> Result<Expr, String> {
    let (m, n) = dims(a);
    if m < n {
        return Err(format!(
            "qr needs at least as many rows as columns, got a {}×{} matrix",
            m, n
        ));
    }
    let rows = rows_of(a);
    if rows
        .iter()
        .flatten()
        .any(|e| matches!(e, Expr::Complex(..)))
    {
        return Err("qr of a complex matrix isn't implemented (needs the conjugate inner product)".into());
    }
    let column = |j: usize| -> Vec<Expr> { (0..m).map(|i| rows[i][j].clone()).collect() };
    let dot = |x: &[Expr], y: &[Expr]| -> Expr {
        add(x.iter()
            .zip(y)
            .map(|(p, q)| mul(vec![p.clone(), q.clone()]))
            .collect())
    };
    let neg_half = Expr::Rat(BigRational::new(BigInt::from(-1), BigInt::from(2)));

    let mut q_cols: Vec<Vec<Expr>> = Vec::with_capacity(n); // orthonormal
    let mut u_cols: Vec<Vec<Expr>> = Vec::with_capacity(n); // orthogonal, unnormalized
    let mut u_norms2: Vec<Expr> = Vec::with_capacity(n);
    let mut r = vec![vec![int(0); n]; n];

    for j in 0..n {
        let aj = column(j);
        let mut v = aj.clone();
        for i in 0..j {
            // v -= (uᵢ·aⱼ / uᵢ·uᵢ)·uᵢ
            let coeff = mul(vec![
                dot(&u_cols[i], &aj),
                pow(u_norms2[i].clone(), int(-1)),
            ]);
            for k in 0..m {
                let scaled = mul(vec![coeff.clone(), u_cols[i][k].clone()]);
                v[k] = add(vec![v[k].clone(), mul(vec![int(-1), scaled])]);
            }
        }
        let norm2 = dot(&v, &v);
        if is_known_zero(&norm2) {
            return Err(format!(
                "qr needs linearly independent columns, but column {} is a combination of the ones before it",
                j + 1
            ));
        }
        let inv_norm = pow(norm2.clone(), neg_half.clone());
        q_cols.push(v.iter().map(|e| mul(vec![inv_norm.clone(), e.clone()])).collect());
        r[j][j] = pow(norm2.clone(), half()); // |vⱼ|
        for i in 0..j {
            r[i][j] = dot(&q_cols[i], &aj); // qᵢ·aⱼ
        }
        u_cols.push(v);
        u_norms2.push(norm2);
    }
    structure(vec![
        ("Q".to_string(), columns_to_matrix(q_cols)),
        ("R".to_string(), Expr::Matrix(r)),
    ])
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
    Ok(Expr::Matrix(
        roots.iter().map(|r| vec![root_to_expr(r)]).collect(),
    ))
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

/// A root of a rational polynomial that we can express exactly. Structured
/// (rather than an `Expr`) so eigenvector elimination can compute with it in
/// the field ℚ(√d) — see `root_to_quad` below.
#[derive(Clone, PartialEq)]
enum ExactRoot {
    Rational(BigRational),
    /// (−b ± √disc) / (2a). The discriminant is never a perfect square (those
    /// roots fold to `Rational`); a negative one means a complex pair via
    /// √(−disc)·i.
    Quad {
        a: BigRational,
        b: BigRational,
        disc: BigRational,
        plus: bool,
    },
    /// An exact root that needs radicals beyond a single square root — cube
    /// roots (Cardano) or nested square roots (biquadratic quartics). Kept as
    /// the rendered expression; eigen*values* report these, but eigen*vector*
    /// elimination (which needs field arithmetic with a decidable zero test)
    /// doesn't reach into these fields yet.
    Radical(Expr),
}

/// Render a root as a canonical expression.
fn root_to_expr(r: &ExactRoot) -> Expr {
    match r {
        ExactRoot::Rational(q) => rat_to_expr(q.clone()),
        ExactRoot::Quad { a, b, disc, plus } => {
            let two = BigRational::from_integer(BigInt::from(2));
            let two_a_inv = pow(rat_to_expr(two * a.clone()), int(-1));
            let neg_b = rat_to_expr(-b.clone());
            let sign = int(if *plus { 1 } else { -1 });
            if *disc < BigRational::zero() {
                // (−b ± i·√(−disc)) / (2a) — one of a complex-conjugate pair.
                let real_term = mul(vec![neg_b, two_a_inv.clone()]);
                let imag_term = mul(vec![sign, pow(rat_to_expr(-disc.clone()), half()), two_a_inv]);
                complex(real_term, imag_term)
            } else {
                let signed_sqrt = mul(vec![sign, pow(rat_to_expr(disc.clone()), half())]);
                mul(vec![add(vec![neg_b, signed_sqrt]), two_a_inv])
            }
        }
        ExactRoot::Radical(e) => e.clone(),
    }
}

/// Find every root we can express exactly, peeling off rational roots and
/// finishing any final linear/quadratic factor.
fn roots_of_poly(mut c: Vec<BigRational>) -> Result<Vec<ExactRoot>, String> {
    trim_leading_zeros(&mut c);
    let mut roots = Vec::new();
    loop {
        trim_leading_zeros(&mut c);
        match c.len() - 1 {
            0 => break,
            1 => {
                roots.push(ExactRoot::Rational(-c[0].clone() / c[1].clone()));
                break;
            }
            2 => {
                roots.append(&mut quad_roots(&c[2], &c[1], &c[0]));
                break;
            }
            deg => match find_rational_root(&c) {
                Some(r) => {
                    roots.push(ExactRoot::Rational(r.clone()));
                    c = synthetic_divide(&c, &r);
                }
                None if deg == 3 => {
                    roots.append(&mut cubic_roots(&c)?);
                    break;
                }
                None if deg == 4 && c[1].is_zero() && c[3].is_zero() => {
                    roots.append(&mut biquadratic_roots(&c)?);
                    break;
                }
                None => {
                    return Err(format!(
                        "could not find all eigenvalues exactly: after {} rational root(s), a \
                         degree-{} factor remains with no rational roots ({})",
                        roots.len(),
                        deg,
                        if deg == 4 {
                            "only biquadratic quartics — no odd-power terms — are implemented; \
                             the general Ferrari reduction isn't"
                        } else {
                            "degree ≥ 5 has no radical formula at all — Abel–Ruffini"
                        }
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

/// Roots of a·x² + b·x + c, exact: rational when the discriminant is a perfect
/// square, otherwise a quadratic-surd pair (complex-conjugate for a negative
/// discriminant).
fn quad_roots(a: &BigRational, b: &BigRational, c: &BigRational) -> Vec<ExactRoot> {
    let four = BigRational::from_integer(BigInt::from(4));
    let two = BigRational::from_integer(BigInt::from(2));
    let disc = b.clone() * b.clone() - four * a.clone() * c.clone();
    if let Some(s) = exact_sqrt(&disc) {
        let two_a = two * a.clone();
        return vec![
            ExactRoot::Rational((-b.clone() + s.clone()) / two_a.clone()),
            ExactRoot::Rational((-b.clone() - s) / two_a),
        ];
    }
    vec![
        ExactRoot::Quad {
            a: a.clone(),
            b: b.clone(),
            disc: disc.clone(),
            plus: true,
        },
        ExactRoot::Quad {
            a: a.clone(),
            b: b.clone(),
            disc,
            plus: false,
        },
    ]
}

/// √r when r is the square of a rational (so 0, 4, 9/4, …), else `None`.
fn exact_sqrt(r: &BigRational) -> Option<BigRational> {
    if *r < BigRational::zero() {
        return None;
    }
    let (ns, ds) = (r.numer().sqrt(), r.denom().sqrt());
    (&ns * &ns == *r.numer() && &ds * &ds == *r.denom()).then(|| BigRational::new(ns, ds))
}

/// Exact sign (−1, 0, 1) of p + q·√d, for d > 0 and not a perfect square —
/// one of the comparisons an exact representation makes decidable.
fn quad_surd_sign(p: &BigRational, q: &BigRational, d: &BigRational) -> i32 {
    let zero = BigRational::zero();
    if *p >= zero && *q >= zero {
        return if p.is_zero() && q.is_zero() { 0 } else { 1 };
    }
    if *p <= zero && *q <= zero {
        return -1;
    }
    // Mixed signs: the part with the larger square wins.
    let pp = p.clone() * p.clone();
    let qq = q.clone() * q.clone() * d.clone();
    if pp == qq {
        return 0; // impossible for non-square d, but never guess
    }
    if (pp > qq) == (*p > zero) {
        1
    } else {
        -1
    }
}

fn third() -> Expr {
    Expr::Rat(BigRational::new(BigInt::from(1), BigInt::from(3)))
}

/// Real cube root of a rational, sign-normalized so `pow` never sees a
/// negative base with a fractional exponent (that would be the complex
/// principal branch — not the real root Cardano needs).
fn real_cbrt_rat(w: &BigRational) -> Expr {
    if *w < BigRational::zero() {
        mul(vec![int(-1), pow(rat_to_expr(-w.clone()), third())])
    } else {
        pow(rat_to_expr(w.clone()), third())
    }
}

/// Real cube root of w = a + s·√Δ (s = ±1, Δ > 0 not a perfect square),
/// sign-normalized like `real_cbrt_rat` using the exact sign of the surd.
fn real_cbrt_surd(a: &BigRational, s: i64, delta: &BigRational) -> Expr {
    let q = BigRational::from_integer(BigInt::from(s));
    let sgn = quad_surd_sign(a, &q, delta);
    let (aa, qq) = if sgn < 0 { (-a.clone(), -q) } else { (a.clone(), q) };
    let inner = add(vec![
        rat_to_expr(aa),
        mul(vec![rat_to_expr(qq), pow(rat_to_expr(delta.clone()), half())]),
    ]);
    let root = pow(inner, third());
    if sgn < 0 {
        mul(vec![int(-1), root])
    } else {
        root
    }
}

/// Cardano's formula, for a cubic with no rational roots (hence irreducible
/// over ℚ, hence distinct roots). With Δ = (q/2)² + (p/3)³ of the depressed
/// cubic t³ + pt + q: Δ > 0 gives one real root in real radicals plus a
/// complex pair; Δ < 0 is the *casus irreducibilis* — three real roots that
/// provably have no expression in real radicals — reported, not approximated.
fn cubic_roots(c: &[BigRational]) -> Result<Vec<ExactRoot>, String> {
    let n = |k: i64| BigRational::from_integer(BigInt::from(k));
    // Monic x³ + Bx² + Cx + D, depressed with x = t − B/3.
    let bb = c[2].clone() / c[3].clone();
    let cc = c[1].clone() / c[3].clone();
    let dd = c[0].clone() / c[3].clone();
    let shift = -bb.clone() / n(3);
    let p = cc.clone() - bb.clone() * bb.clone() / n(3);
    let q =
        n(2) * bb.clone() * bb.clone() * bb.clone() / n(27) - bb.clone() * cc / n(3) + dd;
    let delta =
        q.clone() * q.clone() / n(4) + p.clone() * p.clone() * p.clone() / n(27);

    if delta < BigRational::zero() {
        return Err(
            "could not express the eigenvalues exactly: three real roots remain that have no \
             expression in real radicals (casus irreducibilis); the trigonometric closed form \
             isn't implemented"
                .into(),
        );
    }
    // Δ = 0 means repeated roots, which over ℚ are rational. They normally
    // get peeled by the rational-root search, but its divisor scan is capped,
    // so finish them here when they slip through.
    if delta.is_zero() {
        if p.is_zero() {
            return Ok(vec![ExactRoot::Rational(shift); 3]);
        }
        let t1 = n(3) * q.clone() / p.clone();
        let t2 = -n(3) * q / (n(2) * p);
        return Ok(vec![
            ExactRoot::Rational(t1 + shift.clone()),
            ExactRoot::Rational(t2.clone() + shift.clone()),
            ExactRoot::Rational(t2 + shift),
        ]);
    }

    // Δ > 0: the real root is cbrt(u) + cbrt(v) with u, v = −q/2 ± √Δ; the
    // complex pair is −(cbrt u + cbrt v)/2 ± i·(√3/2)(cbrt u − cbrt v).
    // (Real cube roots, so cbrt(u)·cbrt(v) = −p/3 holds as required.)
    let neg_half_q = -q / n(2);
    let (cb_u, cb_v) = match exact_sqrt(&delta) {
        Some(s) => (
            real_cbrt_rat(&(neg_half_q.clone() + s.clone())),
            real_cbrt_rat(&(neg_half_q - s)),
        ),
        None => (
            real_cbrt_surd(&neg_half_q, 1, &delta),
            real_cbrt_surd(&neg_half_q, -1, &delta),
        ),
    };
    let shift_e = rat_to_expr(shift);
    let real_root = add(vec![cb_u.clone(), cb_v.clone(), shift_e.clone()]);
    let re = add(vec![
        mul(vec![
            Expr::Rat(BigRational::new(BigInt::from(-1), BigInt::from(2))),
            add(vec![cb_u.clone(), cb_v.clone()]),
        ]),
        shift_e,
    ]);
    // u > v, so cbrt(u) − cbrt(v) > 0: this is the +i member of the pair.
    let im = mul(vec![
        half(),
        pow(int(3), half()),
        add(vec![cb_u, mul(vec![int(-1), cb_v])]),
    ]);
    Ok(vec![
        ExactRoot::Radical(real_root),
        ExactRoot::Radical(complex(re.clone(), im.clone())),
        ExactRoot::Radical(complex(re, mul(vec![int(-1), im]))),
    ])
}

/// Roots of a biquadratic c4·x⁴ + c2·x² + c0 (no odd powers, no rational
/// roots) via the quadratic in y = x², then x = ±√y per branch.
fn biquadratic_roots(c: &[BigRational]) -> Result<Vec<ExactRoot>, String> {
    let one = BigRational::one();
    let zero = BigRational::zero();
    let mut out = Vec::with_capacity(4);
    for y in quad_roots(&c[4], &c[2], &c[0]) {
        match &y {
            ExactRoot::Rational(r) => {
                // x² = r: the quadratic machinery handles ±√r, including the
                // imaginary case for negative r.
                out.append(&mut quad_roots(&one, &zero, &(-r.clone())));
            }
            ExactRoot::Quad { disc, .. } if *disc < zero => {
                return Err(
                    "could not express the eigenvalues exactly: they are square roots of \
                     complex numbers, and nested complex radicals aren't implemented"
                        .into(),
                );
            }
            ExactRoot::Quad { .. } => {
                // y is a real quadratic surd with an exactly decidable sign:
                // x = ±√y when positive, ±i·√(−y) when negative.
                let (yq, d) = root_to_quad(&y);
                if quad_surd_sign(&yq.re.a, &yq.re.b, &d) > 0 {
                    let s = pow(quad_rat_to_expr(&yq.re, &d), half());
                    out.push(ExactRoot::Radical(s.clone()));
                    out.push(ExactRoot::Radical(mul(vec![int(-1), s])));
                } else {
                    let neg = QuadRat {
                        a: -yq.re.a.clone(),
                        b: -yq.re.b.clone(),
                    };
                    let s = pow(quad_rat_to_expr(&neg, &d), half());
                    out.push(ExactRoot::Radical(complex(int(0), s.clone())));
                    out.push(ExactRoot::Radical(complex(int(0), mul(vec![int(-1), s]))));
                }
            }
            ExactRoot::Radical(_) => unreachable!("quad_roots never returns Radical"),
        }
    }
    Ok(out)
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

// ---------------------------------------------------------------------------
// Eigenvectors — exact elimination in ℚ(√d) and its complex extension
// ---------------------------------------------------------------------------
//
// Every eigenvalue we produce is rational or a quadratic surd (that's all
// `roots_of_poly` can express), so the entries of A − λI live in the field
// ℚ(√d)(i). The general symbolic Gauss-Jordan can't decide whether an
// expression like 1/(1 − φ) + φ is zero — `is_known_zero` only sees folded
// rationals — so eigenvector elimination runs on an explicit representation
// of that field, where arithmetic stays closed and the zero test is exact.

/// An element a + b·√d of ℚ(√d). The d is fixed per computation and carried
/// externally; when d = 0 the invariant b = 0 holds throughout, and d is
/// never a perfect square (such roots fold to rationals at construction).
#[derive(Clone, PartialEq)]
struct QuadRat {
    a: BigRational,
    b: BigRational,
}

impl QuadRat {
    fn from_rat(a: BigRational) -> Self {
        QuadRat {
            a,
            b: BigRational::zero(),
        }
    }
    fn is_zero(&self) -> bool {
        self.a.is_zero() && self.b.is_zero()
    }
    fn add(&self, o: &Self) -> Self {
        QuadRat {
            a: self.a.clone() + o.a.clone(),
            b: self.b.clone() + o.b.clone(),
        }
    }
    fn neg(&self) -> Self {
        QuadRat {
            a: -self.a.clone(),
            b: -self.b.clone(),
        }
    }
    fn mul(&self, o: &Self, d: &BigRational) -> Self {
        QuadRat {
            a: self.a.clone() * o.a.clone() + self.b.clone() * o.b.clone() * d.clone(),
            b: self.a.clone() * o.b.clone() + self.b.clone() * o.a.clone(),
        }
    }
    /// 1/(a + b√d) = (a − b√d)/(a² − b²·d). The denominator is nonzero for a
    /// nonzero element because √d is irrational.
    fn inv(&self, d: &BigRational) -> Self {
        let denom =
            self.a.clone() * self.a.clone() - self.b.clone() * self.b.clone() * d.clone();
        QuadRat {
            a: self.a.clone() / denom.clone(),
            b: -self.b.clone() / denom,
        }
    }
}

/// An element re + im·i of ℚ(√d)(i).
#[derive(Clone, PartialEq)]
struct QuadComplex {
    re: QuadRat,
    im: QuadRat,
}

impl QuadComplex {
    fn from_rat(r: BigRational) -> Self {
        QuadComplex {
            re: QuadRat::from_rat(r),
            im: QuadRat::from_rat(BigRational::zero()),
        }
    }
    fn is_zero(&self) -> bool {
        self.re.is_zero() && self.im.is_zero()
    }
    fn sub(&self, o: &Self) -> Self {
        QuadComplex {
            re: self.re.add(&o.re.neg()),
            im: self.im.add(&o.im.neg()),
        }
    }
    fn neg(&self) -> Self {
        QuadComplex {
            re: self.re.neg(),
            im: self.im.neg(),
        }
    }
    fn mul(&self, o: &Self, d: &BigRational) -> Self {
        QuadComplex {
            re: self.re.mul(&o.re, d).add(&self.im.mul(&o.im, d).neg()),
            im: self.re.mul(&o.im, d).add(&self.im.mul(&o.re, d)),
        }
    }
    /// conj(z) / |z|². The norm re² + im² lives in ℚ(√d) ⊂ ℝ, so it vanishes
    /// only when z itself is zero.
    fn inv(&self, d: &BigRational) -> Self {
        let norm_inv = self.re.mul(&self.re, d).add(&self.im.mul(&self.im, d)).inv(d);
        QuadComplex {
            re: self.re.mul(&norm_inv, d),
            im: self.im.neg().mul(&norm_inv, d),
        }
    }
}

/// Express a root as an element of ℚ(√d)(i), returning the d to compute in.
/// Callers gate out `Radical` roots first — those live in degree-3+ fields
/// this representation can't hold.
fn root_to_quad(r: &ExactRoot) -> (QuadComplex, BigRational) {
    match r {
        ExactRoot::Radical(_) => {
            unreachable!("Radical roots are gated out before field conversion")
        }
        ExactRoot::Rational(q) => (QuadComplex::from_rat(q.clone()), BigRational::zero()),
        ExactRoot::Quad { a, b, disc, plus } => {
            let two_a = BigRational::from_integer(BigInt::from(2)) * a.clone();
            let rat_part = -b.clone() / two_a.clone();
            let mut coeff = BigRational::one() / two_a; // the ±1/(2a) on the √
            if !*plus {
                coeff = -coeff;
            }
            if *disc > BigRational::zero() {
                // Real surd: λ = −b/(2a) + (±1/(2a))·√disc.
                let re = QuadRat { a: rat_part, b: coeff };
                (
                    QuadComplex {
                        re,
                        im: QuadRat::from_rat(BigRational::zero()),
                    },
                    disc.clone(),
                )
            } else {
                // Complex pair: λ = −b/(2a) ± (1/(2a))·√(−disc)·i. When
                // √(−disc) is rational the imaginary part folds and no
                // irrationality remains (d = 0).
                let neg_disc = -disc.clone();
                let (im, d) = match exact_sqrt(&neg_disc) {
                    Some(s) => (QuadRat::from_rat(coeff * s), BigRational::zero()),
                    None => (
                        QuadRat {
                            a: BigRational::zero(),
                            b: coeff,
                        },
                        neg_disc,
                    ),
                };
                (
                    QuadComplex {
                        re: QuadRat::from_rat(rat_part),
                        im,
                    },
                    d,
                )
            }
        }
    }
}

fn quad_rat_to_expr(q: &QuadRat, d: &BigRational) -> Expr {
    if q.b.is_zero() {
        rat_to_expr(q.a.clone())
    } else {
        add(vec![
            rat_to_expr(q.a.clone()),
            mul(vec![
                rat_to_expr(q.b.clone()),
                pow(rat_to_expr(d.clone()), half()),
            ]),
        ])
    }
}

fn quad_complex_to_expr(z: &QuadComplex, d: &BigRational) -> Expr {
    let re = quad_rat_to_expr(&z.re, d);
    if z.im.is_zero() {
        re
    } else {
        complex(re, quad_rat_to_expr(&z.im, d))
    }
}

/// Kernel basis of a matrix over ℚ(√d)(i): Gauss-Jordan with the field's
/// exact zero test, then the free-column construction (cf. `kernel_basis`).
fn quad_kernel_basis(mut m: Vec<Vec<QuadComplex>>, d: &BigRational) -> Vec<Vec<QuadComplex>> {
    let (rows, cols) = (m.len(), m[0].len());
    let mut pivots = Vec::new();
    let mut pivot_row = 0;
    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }
        let Some(sel) = (pivot_row..rows).find(|&r| !m[r][col].is_zero()) else {
            continue;
        };
        m.swap(pivot_row, sel);
        let inv_pivot = m[pivot_row][col].inv(d);
        for j in 0..cols {
            m[pivot_row][j] = m[pivot_row][j].mul(&inv_pivot, d);
        }
        for r in 0..rows {
            if r == pivot_row || m[r][col].is_zero() {
                continue;
            }
            let factor = m[r][col].clone();
            for j in 0..cols {
                m[r][j] = m[r][j].sub(&factor.mul(&m[pivot_row][j], d));
            }
        }
        pivots.push(col);
        pivot_row += 1;
    }
    (0..cols)
        .filter(|c| !pivots.contains(c))
        .map(|f| {
            let mut v = vec![QuadComplex::from_rat(BigRational::zero()); cols];
            v[f] = QuadComplex::from_rat(BigRational::one());
            for (k, &p) in pivots.iter().enumerate() {
                v[p] = m[k][f].neg();
            }
            v
        })
        .collect()
}

/// Exact eigenvectors of a numeric matrix, returned as the columns of a
/// matrix V whose j-th column pairs with the j-th entry of `eigenvalues` —
/// so A·V = V·diag(eigenvalues). Such a V exists only when the matrix is
/// diagonalizable; a defective matrix is reported, never padded with zeros.
pub fn eigenvectors(a: &Expr) -> Result<Expr, String> {
    let (n, c) = dims(a);
    if n != c {
        return Err("eigenvectors need a square matrix".into());
    }
    let rows = rows_of(a);
    if !is_numeric_matrix(rows) {
        return Err("eigenvectors are only supported for matrices with numeric entries".into());
    }
    let num: Vec<Vec<BigRational>> = rows
        .iter()
        .map(|r| r.iter().map(|e| numeric_value(e).unwrap()).collect())
        .collect();
    let cp = char_poly(a, "lambda")?;
    let coeffs = poly_coeffs(&cp, "lambda")
        .ok_or("eigenvectors are only supported for matrices with numeric entries")?;
    let roots = roots_of_poly(coeffs)?;
    if roots.iter().any(|r| matches!(r, ExactRoot::Radical(_))) {
        return Err(
            "eigenvectors are implemented for rational and quadratic-surd eigenvalues; these \
             eigenvalues need cubic or nested radicals (eigenvalues(A) still reports them \
             exactly)"
                .into(),
        );
    }

    // One kernel computation per *distinct* eigenvalue; its basis vectors are
    // then dealt out to that eigenvalue's occurrences in order.
    let mut distinct: Vec<(ExactRoot, usize)> = Vec::new();
    for r in &roots {
        match distinct.iter_mut().find(|(root, _)| root == r) {
            Some((_, count)) => *count += 1,
            None => distinct.push((r.clone(), 1)),
        }
    }

    let mut bases: Vec<Vec<Vec<Expr>>> = Vec::with_capacity(distinct.len());
    for (root, mult) in &distinct {
        let (lambda, d) = root_to_quad(root);
        let shifted: Vec<Vec<QuadComplex>> = num
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .map(|(j, e)| {
                        let entry = QuadComplex::from_rat(e.clone());
                        if i == j {
                            entry.sub(&lambda)
                        } else {
                            entry
                        }
                    })
                    .collect()
            })
            .collect();
        let basis = quad_kernel_basis(shifted, &d);
        if basis.len() < *mult {
            return Err(format!(
                "matrix is defective: eigenvalue {} has algebraic multiplicity {} but only {} \
                 independent eigenvector(s), so no eigenbasis exists",
                root_to_expr(root),
                mult,
                basis.len()
            ));
        }
        bases.push(
            basis
                .iter()
                .map(|v| v.iter().map(|z| quad_complex_to_expr(z, &d)).collect())
                .collect(),
        );
    }

    // Columns in eigenvalue order: the k-th occurrence of a root takes the
    // k-th vector of its basis.
    let mut used = vec![0usize; distinct.len()];
    let mut columns = Vec::with_capacity(n);
    for r in &roots {
        let idx = distinct.iter().position(|(root, _)| root == r).unwrap();
        columns.push(bases[idx][used[idx]].clone());
        used[idx] += 1;
    }
    Ok(columns_to_matrix(columns))
}
