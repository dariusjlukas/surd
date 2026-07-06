//! Behavioral tests. These double as executable documentation of what the
//! engine currently guarantees.

use surd::Interpreter;

/// Evaluate a single line on a fresh interpreter, returning its rendered form.
fn ev(src: &str) -> String {
    let mut interp = Interpreter::new();
    interp
        .eval_line(src)
        .map(|e| format!("{}", e))
        .unwrap_or_else(|e| format!("error: {}", e))
}

/// Collapse all runs of whitespace/newlines to single spaces, so multi-line
/// matrix output can be compared without pinning exact column padding.
fn norm(src: &str) -> String {
    let mut interp = Interpreter::new();
    let rendered = interp
        .eval_line(src)
        .map(|e| format!("{}", e))
        .unwrap_or_else(|e| format!("error: {}", e));
    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Evaluate several lines on one interpreter; return the last result.
fn ev_all(lines: &[&str]) -> String {
    let mut interp = Interpreter::new();
    let mut out = String::new();
    for line in lines {
        out = interp
            .eval_line(line)
            .map(|e| format!("{}", e))
            .unwrap_or_else(|e| format!("error: {}", e));
    }
    out
}

#[test]
fn exact_rational_arithmetic() {
    assert_eq!(ev("1/3 + 1/6"), "1/2");
    assert_eq!(ev("2/4"), "1/2");
    assert_eq!(ev("1/2 + 1/2"), "1");
    assert_eq!(ev("6/4"), "3/2");
    assert_eq!(ev("2 + 3"), "5");
    assert_eq!(ev("2^10"), "1024");
}

#[test]
fn decimals_are_exact_rationals() {
    assert_eq!(ev("1.5"), "3/2");
    assert_eq!(ev("0.25"), "1/4");
    assert_eq!(ev("1.5 + 1.5"), "3");
}

#[test]
fn radicals_stay_exact() {
    assert_eq!(ev("sqrt(4)"), "2");
    assert_eq!(ev("sqrt(2)^2"), "2");
    assert_eq!(ev("sqrt(2)*sqrt(2)"), "2");
    assert_eq!(ev("8^(1/3)"), "2");
    // sqrt(2) has no exact rational value, so it stays symbolic.
    assert_eq!(ev("sqrt(2)"), "sqrt(2)");
    assert_eq!(ev("2*sqrt(2)"), "2*sqrt(2)");
}

#[test]
fn like_terms_combine() {
    assert_eq!(ev("x + x"), "2*x");
    assert_eq!(ev("x - x"), "0");
    assert_eq!(ev("2*x + 3*x"), "5*x");
    assert_eq!(ev("(2*x)^2"), "4*x^2");
}

#[test]
fn symbolic_constants_do_not_collapse() {
    assert_eq!(ev("pi"), "π");
    assert_eq!(ev("pi - pi"), "0");
    assert_eq!(ev("2*pi + pi"), "3*π");
}

#[test]
fn differentiation() {
    assert_eq!(ev("diff(x^2, x)"), "2*x");
    assert_eq!(ev("diff(x^3, x)"), "3*x^2");
    assert_eq!(ev("diff(sin(x), x)"), "cos(x)");
    assert_eq!(ev("diff(5, x)"), "0");
}

#[test]
fn expansion() {
    assert_eq!(ev("expand((x+1)^2)"), "1 + x^2 + 2*x");
}

#[test]
fn assignment_and_lookup() {
    assert_eq!(ev_all(&["x := 3", "x^2 + 1"]), "10");
    assert_eq!(ev_all(&["a := 1/2", "a + a"]), "1");
}

#[test]
fn substitution() {
    assert_eq!(ev("subs(x^2 + 1, x, 3)"), "10");
}

#[test]
fn equations_are_data_not_booleans() {
    // Canonical ordering puts the number first, so this prints `y = 1 + x`.
    assert_eq!(ev("y = x + 1"), "y = 1 + x");
    assert_eq!(ev("1 = 1"), "1 = 1"); // NOT simplified to `true`
}

#[test]
fn errors_do_not_panic() {
    // Division by zero is a clean error, never a crash.
    assert_eq!(ev("1/0"), "error: division by zero");
    assert_eq!(ev("1/(x - x)"), "error: division by zero");
    // Adjacent identifiers are not implicit multiplication (see regression.rs).
    assert!(ev("x y").starts_with("error:"));
}

#[test]
fn matrix_arithmetic() {
    assert_eq!(norm("[1,2;3,4] + [5,6;7,8]"), "[ 6 8 ] [ 10 12 ]");
    assert_eq!(norm("[1,2;3,4] - [1,1;1,1]"), "[ 0 1 ] [ 2 3 ]");
    assert_eq!(norm("[1,2;3,4] * [5,6;7,8]"), "[ 19 22 ] [ 43 50 ]");
    assert_eq!(norm("2 * [1,2;3,4]"), "[ 2 4 ] [ 6 8 ]");
    assert_eq!(norm("[1,2;3,4] / 2"), "[ 1/2 1 ] [ 3/2 2 ]");
    // A scalar broadcasts over + and − just as it does over * and /.
    assert_eq!(norm("[1,2;3,4] + 10"), "[ 11 12 ] [ 13 14 ]");
    assert_eq!(norm("10 + [1,2;3,4]"), "[ 11 12 ] [ 13 14 ]");
    assert_eq!(norm("[1,2;3,4] - 1"), "[ 0 1 ] [ 2 3 ]");
    // Subtraction order matters: `s − M` negates each entry.
    assert_eq!(norm("10 - [1,2;3,4]"), "[ 9 8 ] [ 7 6 ]");
    assert_eq!(norm("[1,2;3,4]^2"), "[ 7 10 ] [ 15 22 ]");
    assert_eq!(norm("transpose([1,2,3;4,5,6])"), "[ 1 4 ] [ 2 5 ] [ 3 6 ]");
    assert_eq!(norm("eye(3)"), "[ 1 0 0 ] [ 0 1 0 ] [ 0 0 1 ]");
}

#[test]
fn fill_constant_matrices() {
    // One size argument fills a square matrix, like eye(n).
    assert_eq!(norm("fill(0, 2)"), "[ 0 0 ] [ 0 0 ]");
    // Two sizes give rows×cols.
    assert_eq!(norm("fill(7, 2, 3)"), "[ 7 7 7 ] [ 7 7 7 ]");
    // A 1×n fill is the convenient constant row vector.
    assert_eq!(norm("fill(1, 1, 4)"), "[ 1 1 1 1 ]");
    // The value can be any scalar expression — symbols included.
    assert_eq!(norm("fill(x, 2)"), "[ x x ] [ x x ]");
    // It composes: a constant matrix is a normal matrix.
    assert_eq!(norm("fill(2, 2) + eye(2)"), "[ 3 2 ] [ 2 3 ]");
    // Zero dimensions, non-scalar values, and a missing size are all errors.
    assert!(ev("fill(5, 0)").starts_with("error:"));
    assert!(ev("fill([1,2], 2)").starts_with("error:"));
    assert!(ev("fill(3)").starts_with("error:"));
}

#[test]
fn fill_with_coordinate_function() {
    // Collapse whitespace so multi-line matrix output compares cleanly.
    let nw = |lines: &[&str]| {
        ev_all(lines)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    // A function argument is applied at each 1-based (row, col), like m[i, j].
    assert_eq!(
        nw(&["g(i, j) := (i - 1)*3 + j", "fill(g, 3)"]),
        "[ 1 2 3 ] [ 4 5 6 ] [ 7 8 9 ]"
    );
    // Non-square shapes get the same (row, col) treatment.
    assert_eq!(
        nw(&["s(i, j) := i + j", "fill(s, 2, 3)"]),
        "[ 2 3 4 ] [ 3 4 5 ]"
    );
    // The body is ordinary surd, so a conditional rebuilds the identity.
    assert_eq!(
        nw(&["d(i, j) := if i == j then 1 else 0 end", "fill(d, 3)"]),
        "[ 1 0 0 ] [ 0 1 0 ] [ 0 0 1 ]"
    );
    // A function of the wrong arity reports it, rather than filling nonsense.
    assert!(ev_all(&["p(x) := x^2", "fill(p, 3)"]).starts_with("error:"));
}

#[test]
fn exact_determinants() {
    assert_eq!(ev("det([1,2;3,4])"), "-2");
    assert_eq!(ev("det([2,0,0;0,3,0;0,0,4])"), "24");
    assert_eq!(ev("det([1,2,3;4,5,6;7,8,10])"), "-3");
    // Symbolic (cofactor) determinant. Canonical ordering yields this form.
    assert_eq!(ev("det([a,b;c,d])"), "-b*c + a*d");
}

#[test]
fn exact_inverse_and_solve() {
    // Inverse stays exact: 1/det · adjugate.
    assert_eq!(norm("inv([1,2;3,4])"), "[ -2 1 ] [ 3/2 -1/2 ]");
    // A·A⁻¹ = I, with no roundoff.
    assert_eq!(norm("[1,2;3,4] * inv([1,2;3,4])"), "[ 1 0 ] [ 0 1 ]");
    // x + y = 3, x - y = 1  ⇒  x = 2, y = 1.
    assert_eq!(norm("solve([1,1;1,-1], [3;1])"), "[ 2 ] [ 1 ]");
    assert_eq!(ev("rank([1,2;2,4])"), "1");
}

#[test]
fn characteristic_polynomial() {
    assert_eq!(ev("charpoly([2,1;1,2])"), "3 + lambda^2 - 4*lambda");
    assert_eq!(ev("charpoly([2,0;0,3], x)"), "6 + x^2 - 5*x");
}

#[test]
fn exact_eigenvalues() {
    assert_eq!(norm("eigenvalues([2,1;1,2])"), "[ 3 ] [ 1 ]");
    assert_eq!(norm("eigenvalues([1,1;1,1])"), "[ 2 ] [ 0 ]");
    // Irrational eigenvalues stay exact via sqrt: the golden ratio and conjugate.
    // (A numeric coefficient distributes over the sum, hence the expanded form.)
    assert_eq!(
        norm("eigenvalues([1,1;1,0])"),
        "[ 1/2 + 1/2*sqrt(5) ] [ 1/2 - 1/2*sqrt(5) ]"
    );
    // 3×3: one rational root peeled off, then a quadratic factor.
    assert_eq!(
        norm("eigenvalues([2,0,0; 0,3,0; 0,0,5])"),
        "[ 2 ] [ 3 ] [ 5 ]"
    );
}

#[test]
fn exact_eigenvectors() {
    // Columns pair with eigenvalues(A) in order: λ = 3 → (1,1), λ = 1 → (−1,1).
    assert_eq!(norm("eigenvectors([2,1;1,2])"), "[ 1 -1 ] [ 1 1 ]");
    // Irrational eigenvalues: elimination runs exactly in ℚ(√5), so the
    // golden-ratio eigenvector comes out symbolically, not as floats.
    assert_eq!(
        norm("eigenvectors([1,1;1,0])"),
        "[ 1/2 + 1/2*sqrt(5) 1/2 - 1/2*sqrt(5) ] [ 1 1 ]"
    );
    // A·V stays exact: each column is the eigenvalue times the eigenvector.
    assert_eq!(
        norm("[1,1;1,0] * eigenvectors([1,1;1,0])"),
        "[ 3/2 + 1/2*sqrt(5) 3/2 - 1/2*sqrt(5) ] [ 1/2 + 1/2*sqrt(5) 1/2 - 1/2*sqrt(5) ]"
    );
    // A repeated eigenvalue with full geometric multiplicity gets a whole basis.
    assert_eq!(norm("eigenvectors([1,0;0,1])"), "[ 1 0 ] [ 0 1 ]");
}

#[test]
fn complex_eigenvectors_diagonalize_exactly() {
    // Rotation by 90°: eigenvectors over ℚ(i).
    assert_eq!(norm("eigenvectors([0,-1;1,0])"), "[ I -I ] [ 1 1 ]");
    // Complex arithmetic folds eagerly, so V⁻¹·B·V is exactly diag(1+i, 1−i).
    assert_eq!(
        norm("inv(eigenvectors([1,-1;1,1])) * [1,-1;1,1] * eigenvectors([1,-1;1,1])"),
        "[ 1 + I 0 ] [ 0 1 - I ]"
    );
}

#[test]
fn defective_matrices_are_reported_not_padded() {
    // The Jordan block [1,1;0,1] has one eigenvector for a double eigenvalue.
    let msg = ev("eigenvectors([1,1;0,1])");
    assert!(msg.starts_with("error:"), "got: {msg}");
    assert!(msg.contains("defective"), "got: {msg}");
}

#[test]
fn nullspace_basis() {
    // Rank-1: kernel spanned by (−2, 1).
    assert_eq!(norm("nullspace([1,2;2,4])"), "[ -2 ] [ 1 ]");
    // Wide matrix: one free column.
    assert_eq!(norm("nullspace([1,2,3;4,5,6])"), "[ 1 ] [ -2 ] [ 1 ]");
    // Full column rank: the trivial kernel is said in words, not guessed at.
    let msg = ev("nullspace([1,0;0,1])");
    assert!(
        msg.starts_with("error:") && msg.contains("trivial"),
        "got: {msg}"
    );
    // `kernel` is an alias.
    assert_eq!(norm("kernel([1,2;2,4])"), "[ -2 ] [ 1 ]");
}

#[test]
fn underdetermined_solve_returns_general_solution() {
    // x + y = 3 (twice): particular (3,0) plus the homogeneous span of (−1,1).
    assert_eq!(
        norm("solve([1,1;2,2], [3;6])"),
        "struct(nullspace = [ -1 ] [ 1 ], particular = [ 3 ] [ 0 ])"
    );
    // The pieces are reachable as struct fields.
    assert_eq!(norm("solve([1,1;2,2], [3;6]).particular"), "[ 3 ] [ 0 ]");
}

#[test]
fn complex_arithmetic() {
    assert_eq!(ev("I^2"), "-1");
    assert_eq!(ev("(1 + I)*(1 - I)"), "2");
    assert_eq!(ev("(2 + 3*I) + (1 - I)"), "3 + 2*I");
    assert_eq!(ev("(1 + I)^2"), "2*I");
    assert_eq!(ev("2*I - 3*I"), "-I"); // imaginary like-terms combine
    assert_eq!(ev("1/I"), "-I");
    assert_eq!(ev("(2 + 3*I)/(1 + I)"), "5/2 + 1/2*I");
    assert_eq!(ev("conj(3 + 4*I)"), "3 - 4*I");
    assert_eq!(ev("abs(3 + 4*I)"), "5");
    assert_eq!(ev("re(3 + 4*I)"), "3");
    assert_eq!(ev("im(3 + 4*I)"), "4");
}

#[test]
fn sqrt_of_negatives_is_imaginary() {
    assert_eq!(ev("sqrt(-4)"), "2*I");
    assert_eq!(ev("sqrt(-3)"), "sqrt(3)*I");
    assert_eq!(ev("sqrt(-1)"), "I");
}

#[test]
fn complex_transcendentals() {
    // Euler's identity, exactly (the negligible imaginary residue is snapped).
    assert_eq!(ev("N(exp(I*pi))"), "-1");
    assert_eq!(ev("N(exp(I*pi/2), 20)"), "I"); // e^(iπ/2) = i
                                               // Primitive cube root of unity: −1/2 + (√3/2)i.
    assert!(ev("N(exp(2*pi*I/3), 25)").starts_with("-0.5 + 0.866025403784438"));
    // ln(i) = iπ/2.
    assert!(ev("N(ln(I), 20)").starts_with("1.5707963267948966"));
    // sin(i) = i·sinh(1).
    assert!(ev("N(sin(I), 20)").starts_with("1.1752011936438014"));
    // e^(1+i) = e·(cos 1 + i·sin 1).
    assert!(ev("N(exp(1 + I), 20)").starts_with("1.46869393991588"));
}

#[test]
fn complex_eigenvalues() {
    // Rotation by 90°: eigenvalues ±i, returned (not refused).
    assert_eq!(norm("eigenvalues([0,-1;1,0])"), "[ I ] [ -I ]");
}

#[test]
fn lowercase_i_stays_a_variable() {
    // Only capital I is the imaginary unit, so `i` is free for loop counters.
    assert_eq!(ev_all(&["i := 5", "i + 1"]), "6");
}

#[test]
fn cubic_and_biquadratic_eigenvalues_in_radicals() {
    // Companion matrix of x³ − 2: the real cube root plus its complex pair,
    // via Cardano. (This used to be the "honest error" example.)
    assert_eq!(
        norm("eigenvalues([0,0,2; 1,0,0; 0,1,0])"),
        "[ 2^(1/3) ] [ -1/2*2^(1/3) + 1/2*2^(1/3)*sqrt(3)*I ] \
         [ -1/2*2^(1/3) - 1/2*2^(1/3)*sqrt(3)*I ]"
    );
    // Cardano with a depression shift (char poly x³ + x² − 1), checked
    // against the known real root 0.75487766…
    assert!(norm("N(eigenvalues([0,0,1; 1,0,0; 0,1,-1]), 20)")
        .starts_with("[ 0.75487766624669276005 ]"));
    // Biquadratic quartic x⁴ − 2x² − 1: nested radicals ±√(1+√2), ±i·√(√2−1).
    assert!(
        norm("N(eigenvalues([0,0,0,1; 1,0,0,0; 0,1,0,2; 0,0,1,0]), 20)")
            .starts_with("[ 1.5537739740300373073 ] [ -1.5537739740300373073 ]")
    );
    // Eigenvectors don't pretend to follow into cubic fields.
    let msg = ev("eigenvectors([0,0,2; 1,0,0; 0,1,0])");
    assert!(
        msg.starts_with("error:") && msg.contains("radical"),
        "got: {msg}"
    );
}

#[test]
fn eigenvalue_limits_are_honest() {
    // Three real irrational roots (casus irreducibilis): provably not
    // expressible in real radicals, so we say so rather than approximating.
    assert!(ev("eigenvalues([0,0,-1; 1,0,3; 0,1,0])").contains("casus irreducibilis"));
    // A quartic with odd-power terms needs the full Ferrari reduction.
    assert!(ev("eigenvalues([0,0,0,1; 1,0,0,1; 0,1,0,0; 0,0,1,0])").starts_with("error:"));
    // Degree ≥ 5 has no radical formula at all (Abel–Ruffini).
    assert!(
        ev("eigenvalues([0,0,0,0,1; 1,0,0,0,1; 0,1,0,0,0; 0,0,1,0,0; 0,0,0,1,0])")
            .starts_with("error:")
    );
}

#[test]
fn lu_decomposition() {
    // A zero pivot forces a row swap, recorded in P: P·A = L·U.
    assert_eq!(
        norm("lu([0,2;3,4])"),
        "struct(L = [ 1 0 ] [ 0 1 ], P = [ 0 1 ] [ 1 0 ], U = [ 3 4 ] [ 0 2 ])"
    );
    // The factorization reassembles to A exactly.
    assert_eq!(
        norm("d := lu([2,1,1; 4,3,3; 8,7,9]); d.P * [2,1,1; 4,3,3; 8,7,9] - d.L * d.U"),
        "[ 0 0 0 ] [ 0 0 0 ] [ 0 0 0 ]"
    );
    // Singular matrices factor too — U keeps the zero row.
    assert_eq!(norm("lu([1,2;2,4]).U"), "[ 1 2 ] [ 0 0 ]");
}

#[test]
fn qr_decomposition() {
    // The classic integer example: all-rational Q and R.
    assert_eq!(
        norm("qr([3,0;4,5])"),
        "struct(Q = [ 3/5 -4/5 ] [ 4/5 3/5 ], R = [ 5 4 ] [ 0 3 ])"
    );
    // Surd norms stay exact: QᵀQ folds to the identity and Q·R back to A —
    // no float QR can do either.
    assert_eq!(norm("f := qr([1,1;1,0]); T(f.Q) * f.Q"), "[ 1 0 ] [ 0 1 ]");
    assert_eq!(norm("f := qr([1,1;1,0]); f.Q * f.R"), "[ 1 1 ] [ 1 0 ]");
    // Dependent columns are refused, not silently degenerate.
    let msg = ev("qr([1,2;2,4])");
    assert!(
        msg.starts_with("error:") && msg.contains("independent"),
        "got: {msg}"
    );
}

#[test]
fn precision_context() {
    assert_eq!(
        ev_all(&["precision(50)", "N(pi)"]),
        "3.1415926535897932384626433832795028841971693993751"
    );
    // An explicit digit count still overrides the default.
    assert_eq!(
        ev_all(&["precision(5)", "N(1/3, 20)"]),
        "0.33333333333333333333"
    );
    assert_eq!(ev("precision()"), "30"); // factory default
}

#[test]
fn matrix_errors_are_clean() {
    assert_eq!(
        ev("inv([1,2;2,4])"),
        "error: matrix is singular (no inverse)"
    );
    assert!(ev("[1,2;3,4] + [1,2,3]").starts_with("error:"));
    assert!(ev("[1,2;3,4] * [1,2,3]").starts_with("error:"));
    assert!(ev("solve([1,1;2,2], [1;3])").starts_with("error:")); // inconsistent
}

#[test]
fn booleans_and_comparisons() {
    assert_eq!(ev("2 < 3"), "true");
    assert_eq!(ev("3 < 2"), "false");
    assert_eq!(ev("1/2 == 2/4"), "true"); // exact equality
    assert_eq!(ev("2 < 3 and 5 == 5"), "true");
    assert_eq!(ev("2 > 3 or 1 == 1"), "true");
    assert_eq!(ev("not (1 == 1)"), "false");
}

#[test]
fn if_is_an_expression() {
    assert_eq!(ev("if 2 < 3 then 10 else 20 end"), "10");
    assert_eq!(ev("if 2 > 3 then 10 else 20 end"), "20");
    assert_eq!(ev("y := if 1 == 1 then 7 end"), "7");
}

#[test]
fn recursion_with_exact_bignums() {
    // 20! is exact arbitrary-precision, no overflow.
    assert_eq!(
        ev_all(&[
            "fact(n) := if n == 0 then 1 else n*fact(n-1) end",
            "fact(20)",
        ]),
        "2432902008176640000"
    );
}

#[test]
fn while_loop_is_exact() {
    // Newton's method for sqrt(2), five steps, in exact rationals.
    let prog = ev_all(&[
        "x := 1",
        "i := 0",
        "while i < 5 do x := (x + 2/x)/2; i := i + 1 end",
        "x",
    ]);
    assert_eq!(prog, "886731088897/627013566048");
}

#[test]
fn function_scope_is_local() {
    // Parameter `x` inside f must not leak into the global `x`.
    assert_eq!(ev_all(&["x := 99", "f(x) := x + 1", "f(2)", "x"]), "99");
}

#[test]
fn control_flow_requires_decidable_booleans() {
    // The core design rule: undecidable/symbolic conditions error, not guess.
    assert!(ev("if x then 1 else 2 end").starts_with("error:"));
    assert!(ev("x < 4").starts_with("error:")); // a free symbol has no order
    assert!(ev("2 + true").starts_with("error:")); // arithmetic on a boolean
                                                   // (`pi < 4` is fine — constants are decided by certified intervals.)
}

#[test]
fn float_boundary_is_explicit() {
    // Exact by default; N(...) is the only way out.
    assert_eq!(ev("1/3"), "1/3");
    assert!(ev("N(1/3)").starts_with("0.333"));
}

#[test]
fn arbitrary_precision() {
    // 50 significant digits of well-known constants.
    assert_eq!(
        ev("N(pi, 50)"),
        "3.1415926535897932384626433832795028841971693993751"
    );
    assert_eq!(
        ev("N(sqrt(2), 50)"),
        "1.4142135623730950488016887242096980785696718753769"
    );
    assert_eq!(
        ev("N(exp(1), 40)"),
        "2.718281828459045235360287471352662497757"
    );
    // Exact rationals at arbitrary length.
    assert_eq!(ev("N(1/3, 20)"), "0.33333333333333333333");
    assert_eq!(ev("N(1/4, 8)"), "0.25");
    // The numeric engine reaches through symbolic structure: sin(π/6) = 1/2.
    assert_eq!(ev("N(sin(pi/6), 30)"), "0.5");
    // 1000 digits of pi: just check the length and a famous landmark — the
    // Feynman point (six 9s) begins at decimal place 762.
    let pi_1000 = ev("N(pi, 1000)");
    assert!(pi_1000.starts_with("3.14159265358979"));
    assert!(pi_1000.contains("999999"));
}

#[test]
fn float_contagion() {
    // A float operand makes the numeric part of arithmetic float — the
    // inexactness is contagious, never silently laundered back to exact.
    assert_eq!(ev("N(pi, 10) + 1"), "4.141592654");
    assert_eq!(ev("2 * N(1/2)"), "1");
    assert_eq!(ev("N(2)^3"), "8");
    assert_eq!(ev("1/N(4)"), "0.25");
    assert_eq!(ev("N(1) - N(1)"), "0");
    // Symbolic operands still keep the expression symbolic.
    assert_eq!(ev("N(2) + x"), "2 + x");
    assert_eq!(ev("N(pi, 10) + pi"), "3.141592654 + π");
    // Division by a float zero is caught like the exact case.
    assert_eq!(ev("1/N(0)"), "error: division by zero");
    // Non-real results stay symbolic rather than going NaN.
    assert_eq!(ev("N(-2)^(1/2)"), "sqrt(-2)");
}

#[test]
fn plot_is_a_symbolic_value() {
    // The engine doesn't draw; plot(...) is data for the frontend, with the
    // plotted variable quoted (kept symbolic) like diff's.
    assert_eq!(ev("plot(sin(x), x, -pi, pi)"), "plot(sin(x), x, -π, π)");
    assert_eq!(
        ev_all(&["x := 3", "plot(x^2, x, 0, 1)"]),
        "plot(x^2, x, 0, 1)"
    );
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[test]
fn structs_construct_and_access_fields() {
    assert_eq!(ev("struct(a = 1, b = 2).a"), "1");
    assert_eq!(ev("struct(a = 1, b = 2).b"), "2");
    // Fields hold anything a variable can: matrices, functions, equations.
    assert_eq!(
        ev_all(&["s := struct(m = [1, 2; 3, 4], k = 1/3)", "det(s.m) + s.k"]),
        "-5/3"
    );
    // Chained access through nested structs.
    assert_eq!(ev("struct(inner = struct(x = 7)).inner.x"), "7");
    // Field access binds tighter than ^.
    assert_eq!(ev("struct(a = 3).a^2"), "9");
}

#[test]
fn struct_field_names_come_from_syntax_not_bindings() {
    // `a` is bound, but struct(a = ...) still names the field "a".
    assert_eq!(ev_all(&["a := 99", "struct(a = 1).a"]), "1");
    // ...while the value side does evaluate.
    assert_eq!(ev_all(&["a := 99", "struct(b = a).b"]), "99");
}

#[test]
fn structs_are_canonical_and_compare_by_value() {
    // Field order doesn't matter: sorted at construction.
    assert_eq!(ev("struct(b = 2, a = 1) == struct(a = 1, b = 2)"), "true");
    assert_eq!(ev("struct(a = 1) == struct(a = 2)"), "false");
    // Display is re-parseable (sorted, parenthesized as needed).
    assert_eq!(ev("struct(b = 2, a = 1)"), "struct(a = 1, b = 2)");
}

#[test]
fn struct_errors_are_graceful() {
    assert_eq!(
        ev("struct(a = 1).c"),
        "error: struct has no field 'c' (fields: a)"
    );
    assert!(ev("struct(1, 2)").starts_with("error: struct expects"));
    assert!(ev("struct()").starts_with("error: a struct needs"));
    assert!(ev("struct(a = 1, a = 2)").starts_with("error: duplicate struct field"));
    assert!(ev("(1 + 2).a").starts_with("error: cannot read field"));
    assert!(ev("struct(a = 1) + 1").starts_with("error: cannot do arithmetic"));
    // `.5` is still a numeric literal, not field access.
    assert_eq!(ev("[1, .5]"), "[ 1  1/2 ]");
}

// ---------------------------------------------------------------------------
// Exact trig values and Euler's formula
// ---------------------------------------------------------------------------

#[test]
fn trig_folds_at_rational_multiples_of_pi() {
    assert_eq!(ev("sin(pi/6)"), "1/2");
    assert_eq!(ev("cos(pi/3)"), "1/2");
    assert_eq!(ev("cos(pi/4)"), "1/2*sqrt(2)");
    assert_eq!(ev("tan(pi/3)"), "sqrt(3)");
    assert_eq!(ev("tan(0)"), "0");
    assert_eq!(ev("sin(pi)"), "0");
    assert_eq!(ev("cos(pi)"), "-1");
    assert_eq!(ev("sin(2*pi)"), "0");
    // The 15° grid and the 22.5° grid (nested radicals).
    assert_eq!(ev("cos(pi/12)"), "1/4*sqrt(2) + 1/4*sqrt(6)");
    assert_eq!(ev("sin(pi/8)"), "1/2*sqrt(2 - sqrt(2))");
    // Quadrant symmetry: signs and reflections.
    assert_eq!(ev("sin(-pi/6)"), "-1/2");
    assert_eq!(ev("sin(7*pi/6)"), "-1/2");
    assert_eq!(ev("cos(3*pi/4)"), "-1/2*sqrt(2)");
    assert_eq!(ev("sin(13*pi/6)"), "1/2"); // periodicity past 2π
                                           // The folded surd squares back to the exact value.
    assert_eq!(ev("sin(pi/4)^2"), "1/2");
}

#[test]
fn trig_stays_symbolic_outside_the_table() {
    assert_eq!(ev("tan(pi/2)"), "tan(1/2*π)"); // a pole: no value invented
    assert_eq!(ev("cos(pi/7)"), "cos(1/7*π)"); // no surd form exists (deg-3 minimal poly)
    assert_eq!(ev("sin(x)"), "sin(x)");
    assert_eq!(ev("sin(1)"), "sin(1)"); // 1 radian is not a multiple of π
}

#[test]
fn symbolic_trig_angles_normalize_into_the_first_quadrant() {
    // Outside the surd table the *angle* still canonicalizes into [0, π/2]
    // (periodicity, antipode, reflection — exact identities), so every angle
    // with the same trig value gets one canonical form and differences cancel
    // structurally instead of needing the certified comparison engine.
    assert_eq!(ev("cos(4/7*pi)"), "-cos(3/7*π)"); // reflection across π/2
    assert_eq!(ev("cos(10/7*pi)"), "-cos(3/7*π)"); // 10/7π = 2π − 4/7π
    assert_eq!(ev("cos(-3/7*pi)"), "cos(3/7*π)"); // evenness
    assert_eq!(ev("sin(10/7*pi)"), "-sin(3/7*π)"); // third-quadrant sine
    assert_eq!(ev("sin(-2/7*pi)"), "-sin(2/7*π)"); // oddness
    assert_eq!(ev("tan(8/7*pi)"), "tan(1/7*π)"); // tan has period π
    assert_eq!(ev("tan(6/7*pi)"), "-tan(1/7*π)"); // reflection negates tan
    assert_eq!(ev("cos(3/7*pi)"), "cos(3/7*π)"); // already reduced: fixed point
                                                 // A huge angle collapses exactly — no π digits needed to reduce it.
    assert_eq!(ev("cos(100000000000000000001/7*pi)"), "cos(3/7*π)");
    // Equal values now cancel by construction.
    assert_eq!(ev("cos(10/7*pi) - cos(4/7*pi)"), "0");
    assert_eq!(ev("sin(9/7*pi) + sin(2/7*pi)"), "0");
}

#[test]
fn certified_equality_refuses_instead_of_asserting_a_false_disequality() {
    // The size-7 DFT round-trip is exactly the input, but the residue mixes
    // sin·sin products past the algebraic degree caps: the honest answers are
    // `true` or a refusal — `false` would be a wrong certified claim. (This
    // was a shipped bug: value_eq defaulted to false on the fall-through.)
    let out = ev("dsp.idft(dsp.dft([1; 2; 3; 4; 5; 6; 7]))[5] == 5");
    assert!(
        out.starts_with("error:") && out.contains("may be equal"),
        "expected an honest refusal, got: {out}"
    );
    // Sizes on the surd table fold all the way and decide entry-by-entry.
    assert_eq!(ev("dsp.idft(dsp.dft([1; 2; 3])) == [1; 2; 3]"), "true");
    // Where the difference stays inside the algebraic caps, equality decides
    // exactly — the heptagon identity has no surd form but IS algebraic.
    assert_eq!(ev("cos(1/7*pi) - cos(2/7*pi) + cos(3/7*pi) == 1/2"), "true");
    // Certified inequality still separates (the classic near-integer)...
    assert_eq!(ev("exp(pi*sqrt(163)) == 262537412640768744"), "false");
    assert_eq!(ev("exp(pi*sqrt(163)) != 262537412640768744"), "true");
    // ...and free symbols keep structural semantics, not refusals.
    assert_eq!(ev("sin(x) == cos(x)"), "false");
    assert_eq!(ev("x != y"), "true");
}

#[test]
fn pentagon_trig_folds_to_golden_ratio_surds() {
    assert_eq!(ev("cos(pi/5)"), "1/4 + 1/4*sqrt(5)"); // φ/2
    assert_eq!(ev("sin(pi/10)"), "-1/4 + 1/4*sqrt(5)");
    assert_eq!(ev("sin(pi/5)"), "1/4*sqrt(10 - 2*sqrt(5))");
    assert_eq!(ev("cos(2*pi/5)"), "-1/4 + 1/4*sqrt(5)");
    // The classic identity: cos(π/5) − cos(2π/5) = 1/2, exactly.
    assert_eq!(ev("cos(pi/5) - cos(2*pi/5)"), "1/2");
    // sin²+cos² = 1 through the nested radicals.
    assert_eq!(ev("expand(sin(pi/5)^2 + cos(pi/5)^2)"), "1");
}

#[test]
fn square_factor_extraction_and_radical_combining() {
    assert_eq!(ev("sqrt(8)"), "2*sqrt(2)");
    assert_eq!(ev("sqrt(12)"), "2*sqrt(3)");
    assert_eq!(ev("sqrt(8/9)"), "2/3*sqrt(2)");
    assert_eq!(ev("sqrt(720)"), "12*sqrt(5)");
    assert_eq!(ev("8^(-1/2)"), "1/2*2^(-1/2)");
    // Provably nonnegative radicands combine; unknown signs stay apart.
    assert_eq!(ev("sqrt(2)*sqrt(3)"), "sqrt(6)");
    assert_eq!(ev("sqrt(2)*sqrt(6)"), "2*sqrt(3)");
    assert_eq!(ev("sqrt(x)*sqrt(y)"), "sqrt(x)*sqrt(y)");
    // Conjugate quadratic-surd radicands: (10−2√5)(10+2√5) = 80 = 16·5.
    assert_eq!(ev("sqrt(10-2*sqrt(5))*sqrt(10+2*sqrt(5))"), "4*sqrt(5)");
    // sqrt(x^2) still stays put — the |x| branch-cut trap is still refused.
    assert_eq!(ev("sqrt(x^2)"), "sqrt(x^2)");
}

#[test]
fn exp_of_complex_unfolds_by_euler() {
    assert_eq!(ev("exp(I*pi)"), "-1");
    assert_eq!(ev("exp(2*pi*I)"), "1");
    assert_eq!(ev("exp(I*pi/2)"), "I");
    assert_eq!(ev("exp(I*x)"), "cos(x) + sin(x)*I");
    assert_eq!(ev("exp(1 + I*pi)"), "-exp(1)");
}

// ---------------------------------------------------------------------------
// Namespaces and user modules
// ---------------------------------------------------------------------------

#[test]
fn structs_of_functions_are_modules() {
    assert_eq!(
        ev_all(&[
            "twice(x) := 2*x",
            "inc(x) := x + 1",
            "mylib := struct(twice = twice, inc = inc)",
            "mylib.inc(mylib.twice(3))",
        ]),
        "7"
    );
    // Arity and error reporting go through the same machinery as plain calls.
    assert!(ev_all(&["f(x) := x", "m := struct(f = f)", "m.f(1, 2)"])
        .starts_with("error: f expects 1 argument(s)"));
    assert_eq!(
        ev_all(&["m := struct(a = 1)", "m.missing(1)"]),
        "error: struct has no field 'missing' (fields: a)"
    );
    assert_eq!(
        ev_all(&["m := struct(a = 5)", "m.a(3)"]),
        "error: field 'a' holds '5', which is not a function"
    );
    assert!(ev("(1 + 2).f(3)").starts_with("error: cannot call"));
}

#[test]
fn builtin_namespace_dispatch_and_shadowing() {
    assert!(ev("dsp.fhqwhgads([1; 2])")
        .starts_with("error: unknown function 'dsp.fhqwhgads' (available:"));
    // Reading a namespace function without calling it points at the syntax.
    assert!(ev("dsp.dft").starts_with("error: 'dsp.dft' names a function"));
    // A user binding shadows the namespace, like any other builtin.
    assert_eq!(
        ev_all(&["inc(x) := x + 1", "dsp := struct(dft = inc)", "dsp.dft(5)"]),
        "6"
    );
    // An unbound namespace name is still an ordinary symbol on its own.
    assert_eq!(ev("dsp"), "dsp");
}

// ---------------------------------------------------------------------------
// The dsp namespace
// ---------------------------------------------------------------------------

#[test]
fn dft_of_known_vectors() {
    assert_eq!(norm("dsp.dft([1; 1; 1; 1])"), "[ 4 ] [ 0 ] [ 0 ] [ 0 ]");
    assert_eq!(
        norm("dsp.dft([1; 2; 3; 4])"),
        "[ 10 ] [ -2 + 2*I ] [ -2 ] [ -2 - 2*I ]"
    );
    // An impulse transforms to all ones.
    assert_eq!(norm("dsp.dft([1; 0; 0; 0])"), "[ 1 ] [ 1 ] [ 1 ] [ 1 ]");
    // Size 8 needs the 45° grid: exact √2 surds, not floats.
    // X_k = 1 + e^(−iπk/4).
    assert_eq!(
        norm("dsp.dft([1; 1; 0; 0; 0; 0; 0; 0])"),
        "[ 2 ] [ 1 + 1/2*sqrt(2) - 1/2*sqrt(2)*I ] [ 1 - I ] \
         [ 1 - 1/2*sqrt(2) - 1/2*sqrt(2)*I ] [ 0 ] \
         [ 1 - 1/2*sqrt(2) + 1/2*sqrt(2)*I ] [ 1 + I ] \
         [ 1 + 1/2*sqrt(2) + 1/2*sqrt(2)*I ]"
    );
}

#[test]
fn idft_inverts_dft_exactly() {
    // Size 3 exercises the √3/2 twiddles; the round trip folds back to ℚ.
    assert_eq!(
        norm("dsp.idft(dsp.dft([1/3; -2; 5/7]))"),
        "[ 1/3 ] [ -2 ] [ 5/7 ]"
    );
    // Complex entries round-trip too.
    assert_eq!(norm("dsp.idft(dsp.dft([I; 1 + I]))"), "[ I ] [ 1 + I ]");
}

#[test]
fn dftmatrix_matches_dft() {
    assert_eq!(
        norm("dsp.dftmatrix(4)"),
        "[ 1 1 1 1 ] [ 1 -I -1 I ] [ 1 -1 1 -1 ] [ 1 I -1 -I ]"
    );
    assert_eq!(
        norm("dsp.dftmatrix(4) * [1; 2; 3; 4] - dsp.dft([1; 2; 3; 4])"),
        "[ 0 ] [ 0 ] [ 0 ] [ 0 ]"
    );
    assert!(ev("dsp.dftmatrix(0)").starts_with("error: dsp.dftmatrix expects a positive"));
}

#[test]
fn convolution_known_results() {
    // (1 + 2z)(1 + 3z) = 1 + 5z + 6z² — convolution is polynomial product.
    assert_eq!(norm("dsp.conv([1, 2], [1, 3])"), "[ 1 5 6 ]");
    // Orientation follows the first argument.
    assert_eq!(
        norm("dsp.conv([1; 2; 1], [1; 1])"),
        "[ 1 ] [ 3 ] [ 3 ] [ 1 ]"
    );
    // Circular shift: convolving with a rotated impulse rotates the input.
    assert_eq!(norm("dsp.circconv([1, 2, 3], [0, 1, 0])"), "[ 3 1 2 ]");
    assert!(ev("dsp.circconv([1, 2], [1, 2, 3])")
        .starts_with("error: dsp.circconv expects two vectors of the same length"));
}

#[test]
fn dsp_argument_errors_are_graceful() {
    assert!(ev("dsp.dft(3)").starts_with("error: dsp.dft expects a vector"));
    assert!(ev("dsp.dft([1, 2; 3, 4])").starts_with("error: dsp.dft expects a vector"));
    assert!(ev("dsp.dft([1; 2], [3; 4])").starts_with("error: dsp.dft expects 1 argument(s)"));
    assert!(ev("dsp.conv([1, 2])").starts_with("error: dsp.conv expects 2 argument(s)"));
}

#[test]
fn dft_of_symbolic_and_unfoldable_sizes_stays_exact() {
    // Symbolic entries pass straight through the exact arithmetic.
    assert_eq!(norm("dsp.dft([a; b])"), "[ a + b ] [ a - b ]");
    // A size-7 transform has no surd form: entries stay as cos/sin of
    // rational multiples of π — exact, and N(...) can evaluate them.
    assert!(ev("dsp.dft([1; 0; 0; 0; 0; 0; 1])").contains("cos"));
}

#[test]
fn pentagonal_dft_folds_and_round_trips() {
    // Size 5 twiddles are golden-ratio surds; the spectrum is exact.
    assert_eq!(
        norm("dsp.dft([1; 0; 0; 0; 1])"),
        "[ 2 ] [ 3/4 + 1/4*sqrt(5) + 1/4*sqrt(10 + 2*sqrt(5))*I ] \
         [ 3/4 - 1/4*sqrt(5) + 1/4*sqrt(10 - 2*sqrt(5))*I ] \
         [ 3/4 - 1/4*sqrt(5) - 1/4*sqrt(10 - 2*sqrt(5))*I ] \
         [ 3/4 + 1/4*sqrt(5) - 1/4*sqrt(10 + 2*sqrt(5))*I ]"
    );
    // …and the round trip lands back on the input identically.
    assert_eq!(
        norm("dsp.idft(dsp.dft([1; 2; 3; 4; 5]))"),
        "[ 1 ] [ 2 ] [ 3 ] [ 4 ] [ 5 ]"
    );
    assert_eq!(
        norm("dsp.idft(dsp.dft([1/3; -2; 0; 1; 22/7; 0; 0; 1; 0; 5]))"),
        "[ 1/3 ] [ -2 ] [ 0 ] [ 1 ] [ 22/7 ] [ 0 ] [ 0 ] [ 1 ] [ 0 ] [ 5 ]"
    );
}

// ---------------------------------------------------------------------------
// The stats namespace
// ---------------------------------------------------------------------------

#[test]
fn stats_estimators_are_exact() {
    assert_eq!(ev("stats.sum([1; 2; 3; 4])"), "10");
    assert_eq!(ev("stats.sum([1/2; 1/3; 1/4])"), "13/12"); // exact rational sum
    assert_eq!(ev("stats.mean([1; 2; 3; 4])"), "5/2");
    assert_eq!(ev("stats.var([1; 2; 3; 4])"), "5/3"); // sample variance, n−1
    assert_eq!(ev("stats.std([1; 2; 3; 4])"), "sqrt(5/3)"); // an exact surd
    assert_eq!(ev("stats.median([3; 1; 2])"), "2");
    assert_eq!(ev("stats.median([1; 2; 3; 4])"), "5/2"); // mean of middle two
    assert_eq!(ev("stats.median([1/2; 1/3; 1/4])"), "1/3"); // exact ordering
    assert_eq!(ev("stats.min([1/2; 1/3; 1/4])"), "1/4"); // exact ordering
    assert_eq!(ev("stats.max([1/3; 0.34; 3/8])"), "3/8"); // rational beats the float
    assert_eq!(ev("stats.cov([1; 2; 3], [2; 4; 6])"), "2");
    // Symbolic entries flow through estimators that don't need ordering.
    assert_eq!(ev("stats.mean([a; b])"), "1/2*a + 1/2*b");
    assert_eq!(ev("stats.sum([a; b; 2])"), "2 + a + b");
}

#[test]
fn correlation_of_linear_data_is_exactly_one() {
    // No float tool gets ±1 exactly; the surds cancel by radical merging.
    assert_eq!(ev("stats.cor([1; 2; 3], [2; 4; 6])"), "1");
    assert_eq!(ev("stats.cor([1; 2; 3], [5; 3; 1])"), "-1");
}

#[test]
fn correlation_and_covariance_matrices_are_exact() {
    // Columns are variables, rows observations. col2 = 2·col1, so every
    // correlation is exactly 1 and the diagonal is exactly 1.
    assert_eq!(norm("stats.cormat([1,2; 2,4; 3,6])"), "[ 1 1 ] [ 1 1 ]");
    // Hand-checked: var(col1)=1, var(col2)=4, cov=2 — exact, no rounding.
    assert_eq!(norm("stats.covmat([1,2; 2,4; 3,6])"), "[ 1 2 ] [ 2 4 ]");
    // Unit diagonal and exact symmetry hold for arbitrary columns too.
    assert_eq!(ev("stats.cormat([1,4; 2,2; 3,7])[1,1]"), "1");
    assert_eq!(
        ev("m := stats.cormat([1,4; 2,2; 3,7]); m[1,2] - m[2,1]"),
        "0"
    );
    // Graceful errors.
    assert!(norm("stats.cormat([1,2,3])").starts_with("error: stats.cormat expects at least 2"));
    assert!(norm("stats.covmat(5)").starts_with("error: stats.covmat expects a data matrix"));
}

#[test]
fn pairs_builds_a_scatterplot_matrix_value() {
    // A tagged `splom` value: the data matrix, then one symbol per column.
    let v = norm("pairs([1,2; 3,4; 5,6])");
    assert!(v.starts_with("splom("), "got {v}");
    // Default labels x1..xk, or explicit ones from a second vector argument.
    assert!(v.ends_with("x1, x2)"), "got {v}");
    assert!(norm("pairs([1,2; 3,4; 5,6], [mpg, hp])").ends_with("mpg, hp)"));
    // A struct's numeric fields become labelled columns (alphabetical order).
    assert!(norm("pairs(struct(b = [2; 4; 6], a = [1; 2; 3]))").ends_with("a, b)"));
    // A second list selects a subset of the struct's fields, in that order.
    assert!(
        norm("pairs(struct(a = [1;2;3], b = [2;4;6], c = [1;0;1]), [c, a])").ends_with("c, a)")
    );
    // Column names stay symbolic — a workspace binding of `b` must not collapse
    // the selected/labelled column to a number (the model-formula guarantee).
    assert_eq!(
        norm("b := 9; pairs(struct(a = [1;2;3], b = [2;4;6]), [a, b])"),
        norm("pairs(struct(a = [1;2;3], b = [2;4;6]), [a, b])")
    );
    // Errors: too few variables / observations / mismatched struct columns,
    // and a selection naming a missing or non-numeric field.
    assert!(ev("pairs([1; 2; 3])").starts_with("error: pairs needs at least 2 variables"));
    assert!(ev("pairs([1, 2])").starts_with("error: pairs needs at least 2 observations"));
    assert!(ev("pairs(struct(a = [1; 2], b = [1; 2; 3]))").starts_with("error: pairs(struct)"));
    assert!(ev("pairs(struct(a = [1;2], b = [3;4]), [a, zzz])")
        .starts_with("error: pairs: struct has no field 'zzz'"));
    assert!(ev("pairs(struct(a = [1;2], g = [x; y]), [a, g])")
        .starts_with("error: pairs: field 'g' is not a numeric column"));
}

#[test]
fn linfit_is_exact_least_squares() {
    assert_eq!(
        ev("stats.linfit([1; 2; 3; 4], [3; 5; 7; 9])"),
        "struct(intercept = 1, predict = <function(x)>, slope = 2)"
    );
    // Hand-checked OLS: x̄=1, ȳ=7/3, Sxx=2, Sxy=3.
    assert_eq!(
        ev("stats.linfit([0; 1; 2], [1; 2; 4])"),
        "struct(intercept = 5/6, predict = <function(x)>, slope = 3/2)"
    );
    // `predict` is the fitted line as a real function: it evaluates exactly…
    assert_eq!(
        ev_all(&[
            "m := stats.linfit([1; 2; 3; 4], [3; 5; 7; 9])",
            "m.predict(10)",
        ]),
        "21"
    );
    // …and a symbolic argument gives back the line, so it plots like any curve.
    assert_eq!(
        ev_all(&["m := stats.linfit([0; 1; 2], [1; 2; 4])", "m.predict(x)"]),
        "5/6 + 3/2*x"
    );
}

#[test]
fn stats_errors_are_graceful() {
    assert!(ev("stats.median([x; 1])").starts_with("error: stats.median needs numeric"));
    assert!(ev("stats.min([x; 1])").starts_with("error: stats.min needs numeric"));
    assert!(ev("stats.var([1])").starts_with("error: stats.var expects at least 2"));
    assert!(ev("stats.cor([1; 1; 1], [1; 2; 3])")
        .starts_with("error: stats.cor is undefined for zero-variance"));
    assert!(ev("stats.linfit([2; 2], [1; 5])")
        .starts_with("error: stats.linfit needs at least two distinct x"));
    assert!(ev("stats.cov([1; 2], [1; 2; 3])")
        .starts_with("error: stats.cov expects two vectors of the same length"));
    assert!(ev("stats.mean(3)").starts_with("error: stats.mean expects a vector"));
    assert!(ev("stats.histogram([1])").starts_with("error: unknown function 'stats.histogram'"));
}

// ---------------------------------------------------------------------------
// Certified interval comparisons
// ---------------------------------------------------------------------------

#[test]
fn certified_constant_comparisons() {
    assert_eq!(ev("pi < 4"), "true"); // used to error; now certified
    assert_eq!(ev("pi > 3"), "true");
    assert_eq!(ev("sqrt(2) + sqrt(3) > pi"), "true"); // 3.1462… vs 3.1415…
    assert_eq!(ev("355/113 > pi"), "true"); // agree to 6 decimals, still separated
    assert_eq!(ev("exp(pi) > pi^e"), "true"); // the classic
    assert_eq!(ev("sin(1) < cos(1)"), "false");
    assert_eq!(ev("tan(1) > 1"), "true");
    assert_eq!(ev("2^(1/3) < 5^(1/4)"), "true");
    assert_eq!(ev("pi <= pi"), "true");
    // Comparisons feed control flow directly now.
    assert_eq!(ev("if sqrt(2) < pi then 1 else 2 end"), "1");
}

#[test]
fn symbol_comparisons_decide_only_what_holds_for_all_reals() {
    // The difference canonicalizes to an exact rational: sound for every x.
    assert_eq!(ev("x <= x"), "true");
    assert_eq!(ev("x + 1 > x"), "true");
    assert_eq!(ev("x < x"), "false");
    // Anything genuinely value-dependent refuses.
    assert!(ev("x < 1").starts_with("error: cannot order"));
}

#[test]
fn equal_constants_refuse_rather_than_guess() {
    // (√2+√3)² = 5+2√6 exactly, but not structurally: enclosures can never
    // separate — the ALGEBRAIC engine settles it (equal ⇒ `<` is false,
    // `==`/`<=` true), where this once refused with "may be equal".
    assert_eq!(ev("(sqrt(2)+sqrt(3))^2 < 5 + 2*sqrt(6)"), "false");
    assert_eq!(ev("(sqrt(2)+sqrt(3))^2 <= 5 + 2*sqrt(6)"), "true");
    assert_eq!(ev("(sqrt(2)+sqrt(3))^2 == 5 + 2*sqrt(6)"), "true");
    // Transcendental ties are beyond algebra and still refuse honestly.
    let msg = ev("exp(1) < e");
    assert!(msg.contains("may be equal"), "got: {msg}");
    // Non-real and opaque values still refuse outright.
    assert!(ev("sqrt(2) < I").starts_with("error: cannot order"));
    assert!(ev("[1] < [2]").starts_with("error: cannot order"));
    assert!(ev("true < 2").starts_with("error: cannot order"));
}

// ---------------------------------------------------------------------------
// Indexing, elementwise operations, and data primitives
// ---------------------------------------------------------------------------

#[test]
fn one_based_indexing() {
    assert_eq!(ev_all(&["v := [3; 1; 4]", "v[2]"]), "1");
    assert_eq!(ev("[3, 1, 4][3]"), "4"); // row vectors index the same way
    assert_eq!(ev("[1,2;3,4][2,1]"), "3");
    assert_eq!(norm("[1,2;3,4][2]"), "[ 3 4 ]"); // one index on a matrix: the row
    assert_eq!(ev_all(&["d := struct(s = [5; 6])", "d.s[2]"]), "6"); // chains
    assert_eq!(
        ev("[1; 2][3]"),
        "error: index 3 is out of range (the vector has 2)"
    );
    assert!(ev("[1; 2][0]").starts_with("error: indices are 1-based"));
    assert!(ev("(1 + 2)[1]").starts_with("error: cannot index"));
}

#[test]
fn elementwise_operators() {
    assert_eq!(norm("[1, 2, 3] .* [4, 5, 6]"), "[ 4 10 18 ]");
    assert_eq!(norm("[1, 2, 3] ./ [2, 4, 8]"), "[ 1/2 1/2 3/8 ]");
    assert_eq!(norm("[1, 2, 3] .^ 2"), "[ 1 4 9 ]");
    assert_eq!(norm("2 .* [1, 2]"), "[ 2 4 ]"); // scalars broadcast
    assert_eq!(ev("2 .* 3"), "6"); // …and degrade to plain arithmetic
    assert!(ev("[1, 2] ./ [1, 0]").starts_with("error: division by zero"));
    assert!(ev("[1, 2] .* [1; 2]").starts_with("error: elementwise operation needs"));
}

#[test]
fn scalar_functions_map_over_matrices() {
    assert_eq!(norm("sin([0; pi/6])"), "[ 0 ] [ 1/2 ]");
    assert_eq!(norm("sqrt([4, 8])"), "[ 2 2*sqrt(2) ]");
    assert_eq!(norm("abs([-1, 2; -3, 4])"), "[ 1 2 ] [ 3 4 ]");
}

#[test]
fn data_primitives() {
    assert_eq!(ev("len([3; 1; 4; 1; 5])"), "5");
    assert_eq!(ev("len([1, 2; 3, 4])"), "2"); // rows, for non-vectors
    assert_eq!(ev("size([1, 2; 3, 4])"), "struct(cols = 2, rows = 2)");
    assert_eq!(ev("dot([1, 2, 3], [4, 5, 6])"), "32");
    assert_eq!(norm("vcat([1; 2], 9)"), "[ 1 ] [ 2 ] [ 9 ]");
    assert_eq!(norm("hcat([1; 2], [3; 4])"), "[ 1 3 ] [ 2 4 ]");
    assert_eq!(norm("linspace(0, 1, 5)"), "[ 0 1/4 1/2 3/4 1 ]"); // exact steps
    assert_eq!(norm("map(sin, [0, pi/2])"), "[ 0 1 ]");
    assert_eq!(ev_all(&["f(x) := x^2 + 1", "map(f, [1, 2])"]), "[ 2  5 ]");
    assert!(ev("map(3, [1])").starts_with("error: map expects a function"));
    assert!(ev("vcat([1, 2], [1; 2])").starts_with("error: vcat needs"));
}

// ---------------------------------------------------------------------------
// FIR design: freqz, windows, firlow, quantize
// ---------------------------------------------------------------------------

#[test]
fn freqz_of_known_filters() {
    // The 2-tap boxcar: H(0) = 2, H(π/2) = 1 − i, H(π) = 0 — all exact.
    assert_eq!(norm("dsp.freqz([1, 1], [0, pi/2, pi])"), "[ 2 1 - I 0 ]");
    // A pure delay has unit magnitude everywhere.
    assert_eq!(
        norm("map(abs, dsp.freqz([0, 1], [0, pi/3, pi/2]))"),
        "[ 1 1 1 ]"
    );
}

#[test]
fn windows_are_exact() {
    assert_eq!(norm("dsp.hann(4)"), "[ 0 3/4 3/4 0 ]");
    assert_eq!(norm("dsp.hamming(3)"), "[ 2/25 1 2/25 ]");
    // Exactly zero at the ends — float tools report ~−1.4e-17 here.
    assert_eq!(norm("dsp.blackman(3)"), "[ 0 1 0 ]");
    assert_eq!(ev("dsp.hann(1)"), "[ 1 ]");
}

#[test]
fn windowed_sinc_design_is_exact() {
    // 5 taps at wc = π/2, Hann-windowed: ends vanish, center is wc/π = 1/2.
    assert_eq!(
        norm("dsp.firlow(5, pi/2) .* dsp.hann(5)"),
        "[ 0 1/2*π^(-1) 1/2 1/2*π^(-1) 0 ]"
    );
    // The cutoff response is −1/2 exactly: magnitude 1/2 carrying the
    // linear-phase factor e^(−iωM) = e^(−iπ) = −1 of the M = 2 delay.
    assert_eq!(
        norm("dsp.freqz(dsp.firlow(5, pi/2) .* dsp.hann(5), [pi/2])"),
        "[ -1/2 ]"
    );
    assert_eq!(
        norm("map(abs, dsp.freqz(dsp.firlow(5, pi/2) .* dsp.hann(5), [pi/2]))"),
        "[ 1/2 ]"
    );
}

#[test]
fn quantize_snaps_to_the_fixed_point_grid() {
    assert_eq!(
        norm("dsp.quantize([1/3; -1/3; 1/32], 4)"),
        "[ 5/16 ] [ -5/16 ] [ 1/16 ]" // ties (1/32·16 = 1/2) round away from zero
    );
    // Floats quantize via their exact binary value.
    assert_eq!(ev("dsp.quantize([N(1/3)], 8)"), "[ 85/256 ]");
    // The quantization error is an exact object you can measure.
    assert_eq!(
        ev_all(&[
            "h := [1/3, 1/3]",
            "e := h - dsp.quantize(h, 4)",
            "dsp.freqz(e, [0])",
        ]),
        "[ 1/24 ]" // 2·(1/3 − 5/16) = 1/24, exactly
    );
    assert!(ev("dsp.quantize([x], 4)").starts_with("error: dsp.quantize needs numeric"));
}

// ---------------------------------------------------------------------------
// stats expansion: quantile, rmse, r2, polyfit/polyval, lsq
// ---------------------------------------------------------------------------

#[test]
fn quantiles_interpolate_exactly() {
    assert_eq!(ev("stats.quantile([1; 2; 3; 4], 1/2)"), "5/2"); // == median
    assert_eq!(ev("stats.quantile([0; 10], 1/4)"), "5/2"); // exact interpolation
    assert_eq!(ev("stats.quantile([3; 1; 2], 0)"), "1"); // min
    assert_eq!(ev("stats.quantile([3; 1; 2], 1)"), "3"); // max
    assert!(ev("stats.quantile([1; 2], 2)").starts_with("error: stats.quantile expects"));
    assert!(ev("stats.quantile([x; 1], 1/2)").starts_with("error: stats.quantile needs numeric"));
}

#[test]
fn fit_metrics_are_exact() {
    assert_eq!(ev("stats.rmse([1, 2, 3], [1, 2, 3])"), "0");
    assert_eq!(ev("stats.rmse([1, 2], [2, 4])"), "sqrt(5/2)"); // an exact surd
    assert_eq!(ev("stats.r2([1, 2, 3], [1, 2, 3])"), "1"); // perfect fit: exactly 1
    assert_eq!(ev("stats.r2([1, 2, 3, 4], [1, 2, 3, 5])"), "4/5");
    assert!(ev("stats.r2([2, 2], [1, 3])").starts_with("error: stats.r2 is undefined"));
}

#[test]
fn polyfit_and_polyval() {
    // y = x² on four points: recovered exactly, no residual.
    assert_eq!(
        norm("stats.polyfit([0, 1, 2, 3], [0, 1, 4, 9], 2)"),
        "[ 0 ] [ 0 ] [ 1 ]"
    );
    // polyval renders a symbolic argument as the polynomial itself.
    assert_eq!(ev("stats.polyval([1; 2; 3], t)"), "1 + 2*t + 3*t^2");
    assert_eq!(norm("stats.polyval([0; 0; 1], [0, 1, 5])"), "[ 0 1 25 ]");
    // Degree-1 polyfit agrees with linfit.
    assert_eq!(
        ev_all(&[
            "f := stats.linfit([0; 1; 2], [1; 2; 4])",
            "c := stats.polyfit([0; 1; 2], [1; 2; 4], 1)",
            "c[1] == f.intercept and c[2] == f.slope",
        ]),
        "true"
    );
    assert!(ev("stats.polyfit([1, 1, 2], [1, 2, 3], 2)")
        .starts_with("error: stats.polyfit needs at least 3 distinct"));
}

#[test]
fn least_squares_is_exact() {
    assert_eq!(
        norm("stats.lsq([1, 0; 0, 1; 1, 1], [1; 1; 2])"),
        "[ 1 ] [ 1 ]"
    );
    assert!(ev("stats.lsq([1, 2; 2, 4], [1; 2])")
        .starts_with("error: stats.lsq: the regressors are linearly dependent"));
    assert!(ev("stats.lsq([1, 0; 0, 1], [1; 2; 3])").starts_with("error: stats.lsq expects one"));
}

// ---------------------------------------------------------------------------
// Signals: packed, certified bulk data
// ---------------------------------------------------------------------------

#[test]
fn signals_pack_and_read_back() {
    assert_eq!(ev_all(&["s := signal([1; 2; 3])", "len(s)"]), "3");
    // Dyadic rationals pack exactly: a point interval, certified error 0.
    assert!(ev("signal([1/2; 3; -5/8])").contains("exact"));
    // 1/3 is not representable: the display owns up to the enclosure.
    assert!(ev("signal([1/3])").contains("max error ±"));
    // Indexing reads the midpoint; bound() is the certified deviation.
    assert_eq!(ev_all(&["s := signal([1; 2])", "s[2]"]), "2");
    assert_eq!(ev_all(&["s := signal([1; 2])", "bound(s)"]), "0");
    // Symbolic entries refuse — the boundary is explicit.
    assert!(ev("signal([x; 1])").starts_with("error: signal needs numeric"));
}

#[test]
fn signal_arithmetic_and_boundary_rules() {
    // peak is a certified *upper bound* — at or barely above the true 8.
    assert_eq!(
        ev_all(&[
            "s := signal([3; 4])",
            "p := dsp.peak(2 .* s)",
            "p >= 8 and p < 8.000001",
        ]),
        "true"
    );
    // Plain * between signals refuses, pointing at .* (same rule as matrices).
    assert!(ev_all(&["s := signal([1])", "s * s"]).starts_with("error: use .*"));
    // Exact matrices never mix in silently.
    assert!(ev_all(&["s := signal([1; 2])", "s + [1; 2]"])
        .starts_with("error: cannot mix an exact matrix"));
    // Substrates never mix silently either.
    assert!(
        ev_all(&["a := signal([1])", "b := signal([1], 30)", "a + b"])
            .starts_with("error: cannot mix f64 and arbitrary-precision")
    );
    // Signals cannot be ordered (which sample would it mean?).
    assert!(ev_all(&["s := signal([1])", "s < 2"]).starts_with("error: cannot order"));
}

#[test]
fn signal_division_by_zero_sample_refuses() {
    assert!(ev_all(&["a := signal([1; 1])", "b := signal([2; 0])", "a ./ b"])
        .starts_with("error: division by an interval containing zero (a sample's divisor may be 0) (sample 2)"));
}

#[test]
fn signal_fft_roundtrip_within_certified_bounds() {
    // The certified peak of the round-trip error is provably tiny — this is
    // a *decidable* comparison (Float vs rational).
    // fft/ifft now return a single complex signal; re(...) pulls the real part.
    assert_eq!(
        ev_all(&[
            "s := signal([1; 2; 3; 4; 5; 6; 7; 8])",
            "r := re(dsp.ifft(dsp.fft(s)))",
            "dsp.peak(r - s) < 1/10^12",
        ]),
        "true"
    );
    // Non-power-of-two lengths refuse with a pointer at dsp.pad.
    assert!(ev_all(&["s := signal([1; 2; 3])", "dsp.fft(s)"])
        .starts_with("error: fft length must be a power of two"));
    assert_eq!(
        ev_all(&["s := signal([1; 2; 3])", "len(re(dsp.fft(dsp.pad(s, 4))))"]),
        "4"
    );
}

#[test]
fn complex_signals_pack_split_and_compute() {
    // Complex entries pack into a complex signal; integer parts stay exact.
    assert_eq!(
        ev("signal([1 + 2*I; 3 - 4*I])"),
        "<signal: 2 samples, complex f64, exact>"
    );
    // re/im pull out real signals; indexing reads a (real) midpoint.
    assert_eq!(
        ev_all(&["z := signal([1 + 2*I; 3 - 4*I])", "re(z)[2]"]),
        "3"
    );
    assert_eq!(
        ev_all(&["z := signal([1 + 2*I; 3 - 4*I])", "im(z)[1]"]),
        "2"
    );
    // |3 + 4i| = 5, within the certified envelope.
    assert_eq!(
        ev_all(&[
            "z := signal([3 + 4*I])",
            "dsp.peak(abs(z) - signal([5])) < 1/10^9"
        ]),
        "true"
    );
    // i·i = −1: complex .* really is the complex product.
    assert_eq!(
        ev_all(&[
            "z := signal([1*I])",
            "dsp.peak((z .* z) - signal([-1])) < 1/10^9"
        ]),
        "true"
    );
    // conj flips the imaginary part.
    assert_eq!(ev_all(&["z := signal([1 + 2*I])", "im(conj(z))[1]"]), "-2");
}

#[test]
fn complex_fft_roundtrips() {
    // ifft∘fft is the identity on a complex signal too (within bounds).
    assert_eq!(
        ev_all(&[
            "z := signal([1 + 1*I; 2 - 1*I; 0 + 3*I; -1 - 2*I])",
            "w := dsp.ifft(dsp.fft(z))",
            "dsp.peak(w - z) < 1/10^12",
        ]),
        "true"
    );
}

#[test]
fn signal_conv_matches_exact_conv() {
    // Bulk convolution agrees with the exact one to certified precision.
    assert_eq!(
        ev_all(&[
            "a := [1, 2, 1]",
            "b := [1, 3]",
            "d := dsp.conv(signal(a), signal(b)) - signal(dsp.conv(a, b))",
            "dsp.peak(d) < 1/10^14",
        ]),
        "true"
    );
}

#[test]
fn signal_reductions_are_certified() {
    assert_eq!(ev("dsp.peak(signal([3; -4; 2]))"), "4");
    // rms([3; 4]) = √(25/2): the certified upper bound brackets it tightly.
    assert_eq!(
        ev_all(&[
            "r := dsp.rms(signal([3; 4]))",
            "r >= sqrt(25/2) and r < 3.5356",
        ]),
        "true"
    );
}

#[test]
fn high_precision_signals_tighten_bounds() {
    // The same data at 50 digits has a far smaller certified error than f64.
    assert_eq!(
        ev_all(&[
            "lofi := signal([1/3; 2/7])",
            "hifi := signal([1/3; 2/7], 50)",
            "bound(hifi) < bound(lofi) ./ 10^50",
        ]),
        "true"
    );
}

// ---------------------------------------------------------------------------
// Slicing and signal plotting
// ---------------------------------------------------------------------------

#[test]
fn slice_vectors_and_signals() {
    assert_eq!(norm("slice([10, 20, 30, 40], 2, 2)"), "[ 20 30 ]");
    assert_eq!(norm("slice([10; 20; 30], 1, 2)"), "[ 10 ] [ 20 ]");
    assert_eq!(
        ev_all(&["s := signal([3; 1; 4; 1; 5])", "len(slice(s, 2, 3))"]),
        "3"
    );
    assert_eq!(
        ev_all(&["s := signal([3; 1; 4; 1; 5])", "slice(s, 2, 3)[1]"]),
        "1"
    );
    assert!(ev("slice([1, 2], 2, 5)").starts_with("error: slice of 5 from position 2"));
    assert!(
        ev_all(&["s := signal([1])", "slice(s, 1, 2)"]).starts_with("error: slice of 2 samples")
    );
    assert!(ev("slice(3, 1, 1)").starts_with("error: slice expects"));
}

#[test]
fn range_slicing_with_colon() {
    let m = "[1,2,3;4,5,6;7,8,9]";
    // A range keeps its axis; a scalar collapses it.
    assert_eq!(norm(&format!("{m}[1:2, 2:3]")), "[ 2 3 ] [ 5 6 ]"); // submatrix
    assert_eq!(norm(&format!("{m}[2, :]")), "[ 4 5 6 ]"); // a whole row
    assert_eq!(norm(&format!("{m}[:, 2]")), "[ 2 ] [ 5 ] [ 8 ]"); // a whole column
    assert_eq!(norm(&format!("{m}[1:2, 3]")), "[ 3 ] [ 6 ]"); // scalar column collapses
    assert_eq!(norm(&format!("{m}[3, 1:2]")), "[ 7 8 ]"); // scalar row collapses
    assert_eq!(ev(&format!("{m}[2, 3]")), "6"); // both scalar → the element

    // Vector sub-ranges, open ends, and `:` binding looser than arithmetic.
    assert_eq!(norm("[10,20,30,40][2:3]"), "[ 20 30 ]");
    assert_eq!(norm("[10;20;30;40][2:3]"), "[ 20 ] [ 30 ]");
    assert_eq!(norm("[10,20,30,40][:3]"), "[ 10 20 30 ]"); // open start
    assert_eq!(norm("[10,20,30,40][2:]"), "[ 20 30 40 ]"); // open end
    assert_eq!(norm("[10,20,30,40][:]"), "[ 10 20 30 40 ]"); // whole axis
    assert_eq!(ev("[10,20,30][2:2]"), "[ 20 ]"); // a one-long range stays a vector
    assert_eq!(ev("[10,20,30][2]"), "20"); // …but a scalar collapses to the element
    assert_eq!(norm("[10,20,30,40,50][(1+1):(1+3)]"), "[ 20 30 40 ]"); // expression bounds

    // Bounds are checked; reversed and over-long ranges name the axis.
    assert_eq!(
        ev("[1,2,3][2:5]"),
        "error: range 2:5 is out of range (the vector has 3)"
    );
    assert_eq!(
        ev("[1,2,3][3:2]"),
        "error: range 3:2 is out of range (the vector has 3)"
    );
    assert!(ev("[1,2;3,4][1, 1, 1]").starts_with("error: indexing takes 1 index"));
}

#[test]
fn range_slicing_signals() {
    let s = "s := signal([3; 1; 4; 1; 5])";
    assert_eq!(ev_all(&[s, "len(s[2:4])"]), "3"); // a sub-signal, not a vector
    assert_eq!(ev_all(&[s, "s[2:4][1]"]), "1"); // re-indexes into the slice
    assert_eq!(ev_all(&[s, "len(s[:])"]), "5"); // whole signal
    assert_eq!(ev_all(&[s, "s[3]"]), "4"); // a scalar still reads the midpoint
    assert!(ev_all(&["s := signal([1; 2])", "s[1:5]"])
        .starts_with("error: range 1:5 is out of range (the signal has 2)"));
}

#[test]
fn strided_slicing() {
    // `lo:step:hi` keeps every step-th position (MATLAB/Julia order).
    assert_eq!(norm("[10,20,30,40,50][1:2:5]"), "[ 10 30 50 ]");
    assert_eq!(norm("[10,20,30,40,50][2:2:5]"), "[ 20 40 ]"); // stops within bounds
    assert_eq!(norm("[10,20,30,40,50][1:2:]"), "[ 10 30 50 ]"); // open stop
    assert_eq!(norm("[10,20,30,40,50][:2:]"), "[ 10 30 50 ]"); // open start and stop
    assert_eq!(norm("[10,20,30,40,50][1:3:]"), "[ 10 40 ]");
    assert_eq!(norm("[10,20,30,40,50][1:1:5]"), "[ 10 20 30 40 50 ]"); // step 1 == range
    assert_eq!(norm("[10,20,30,40,50][1:9:5]"), "[ 10 ]"); // step past the end → just lo

    // A column vector keeps its orientation.
    assert_eq!(norm("[10;20;30;40;50][1:2:5]"), "[ 10 ] [ 30 ] [ 50 ]");

    // Both matrix axes can stride independently.
    let m = "[1,2,3;4,5,6;7,8,9]";
    assert_eq!(norm(&format!("{m}[1:2:3, :]")), "[ 1 2 3 ] [ 7 8 9 ]"); // rows 1,3
    assert_eq!(norm(&format!("{m}[:, 1:2:3]")), "[ 1 3 ] [ 4 6 ] [ 7 9 ]"); // cols 1,3
    assert_eq!(norm(&format!("{m}[1:2:3, 1:2:3]")), "[ 1 3 ] [ 7 9 ]"); // both

    // The stride may be any expression; a parenthesized one stays a scalar
    // stride (it is the comma that makes a (take, skip) pair).
    assert_eq!(norm("[10,20,30,40,50][1:(1+1):5]"), "[ 10 30 50 ]");
    assert_eq!(norm("[10,20,30,40,50][1:(1+1):(2+3)]"), "[ 10 30 50 ]");

    // A stride of 0 is rejected.
    assert_eq!(
        ev("[1,2,3,4,5][1:0:5]"),
        "error: a stride must be a positive integer, got '0'"
    );
}

#[test]
fn take_skip_slicing() {
    // `lo:(take, skip):hi` keeps `take` consecutive, then skips `skip`.
    let v = "[10,20,30,40,50,60,70,80,90,100]";
    assert_eq!(
        norm(&format!("{v}[1:(4,1):]")),
        "[ 10 20 30 40 60 70 80 90 ]"
    );
    assert_eq!(norm(&format!("{v}[2:(2,3):]")), "[ 20 30 70 80 ]");
    assert_eq!(norm(&format!("{v}[1:(4,1):6]")), "[ 10 20 30 40 60 ]"); // bounded stop

    // A scalar stride k is the pair (1, k - 1).
    assert_eq!(norm(&format!("{v}[1:3:]")), norm(&format!("{v}[1:(1,2):]")));

    // skip 0 keeps everything (a contiguous take); take must be positive.
    assert_eq!(norm("[1,2,3][1:(2,0):]"), "[ 1 2 3 ]");
    assert_eq!(
        ev("[1,2,3,4,5][1:(0,1):]"),
        "error: a take count must be a positive integer, got '0'"
    );

    // A pair is only meaningful as the stride, not as an upper bound.
    assert_eq!(
        ev("[10,20,30][1:(4,1)]"),
        "error: a (take, skip) pair is only valid as the stride, as in lo:(take, skip):hi"
    );
}

#[test]
fn strided_slicing_signals() {
    let s = "s := signal([10; 20; 30; 40; 50])";
    assert_eq!(ev_all(&[s, "len(s[1:2:5])"]), "3"); // still a sub-signal
    assert_eq!(ev_all(&[s, "s[1:2:5][2]"]), "30"); // re-index into the stride
    let big = "s := signal([1; 2; 3; 4; 5; 6; 7; 8; 9; 10])";
    assert_eq!(ev_all(&[big, "len(s[1:(4,1):])"]), "8"); // take 4, skip 1
    assert_eq!(ev_all(&[big, "s[1:(4,1):][5]"]), "6"); // gathered sample
    assert!(ev_all(&["s := signal([1; 2; 3])", "s[1:0:3]"])
        .starts_with("error: a stride must be a positive integer"));
}

#[test]
fn plotting_signals() {
    // plot over signals produces the static signal-plot value.
    assert!(ev_all(&["s := signal([1; 2; 3])", "plot(s)"]).starts_with("plotsignal("));
    assert!(ev_all(&["s := signal([1; 2])", "plot(s, 2 .* s)"]).starts_with("plotsignal("));
    // Mixing a signal into a function plot still refuses.
    assert!(ev_all(&["s := signal([1])", "plot(s, x, 0, 1)"]).starts_with("error:"));
    // The non-signal short form keeps its error.
    assert!(ev("plot(sin(x))").starts_with("error: plot expects"));
}

// ---------------------------------------------------------------------------
// Exact Parks–McClellan (dsp.remez) and certified windows
// ---------------------------------------------------------------------------

#[test]
fn remez_degenerate_allpass_is_exact() {
    // Approximating 1 over the whole band: the impulse, with ripple *exactly*
    // zero — no tolerance saying "close enough", the answer is just right.
    assert_eq!(norm("dsp.remez(7, [0, pi], [1]).taps"), "[ 0 0 0 1 0 0 0 ]");
    assert_eq!(ev("dsp.remez(7, [0, pi], [1]).ripple"), "0");
}

#[test]
fn remez_lowpass_meets_its_spec_exactly() {
    // The deterministic exact optimum: same grid, same answer, every time.
    assert_eq!(
        ev_all(&[
            "f := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0])",
            "N(f.ripple, 6)",
        ]),
        "0.119476"
    );
    // Spec compliance at band-edge grid points is *decidable*: |H − D| ≤ δ
    // as an exact comparison of rationals — not a float eyeball.
    assert_eq!(
        ev_all(&[
            "f := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0])",
            "a := abs(dsp.freqz(f.taps, [0])[1] - 1) <= f.ripple",
            "b := abs(dsp.freqz(f.taps, [pi])[1]) <= f.ripple",
            "c := f.taps[1] == f.taps[15] and f.taps[3] == f.taps[13]",
            "a and b and c",
        ]),
        "true"
    );
}

#[test]
fn remez_weights_trade_ripple_between_bands() {
    // A 10× stopband weight forces the stopband error under δ/10 — again an
    // exact, decidable claim.
    assert_eq!(
        ev_all(&[
            "g := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0], [1, 10])",
            "10 * abs(dsp.freqz(g.taps, [pi])[1]) <= g.ripple",
        ]),
        "true"
    );
}

#[test]
fn remez_validates_its_spec() {
    // An even tap count is no longer an error — it selects Type II.
    assert!(ev("dsp.remez(8, [0, 1/2*pi], [1])").starts_with("struct"));
    assert!(ev("dsp.remez(7, [0, 1, 2], [1])").starts_with("error: dsp.remez band edges"));
    assert!(ev("dsp.remez(7, [0, 1], [1, 0])").starts_with("error: dsp.remez expects one desired"));
    assert!(ev("dsp.remez(7, [1, 1/2], [1])")
        .starts_with("error: dsp.remez band edges must be strictly increasing"));
    assert!(ev("dsp.remez(7, [0, 1], [1], [0])").contains("must be a positive number"));
    assert!(ev("dsp.remez(201, [0, pi], [1])").starts_with("error: dsp.remez supports up to"));
}

#[test]
fn certified_windows_enclose_the_exact_values() {
    // dsp.window is the bulk (certified-interval) sibling of the exact
    // dsp.hann: at n = 4 the exact values are [0, 3/4, 3/4, 0], and each
    // must lie within mid ± bound — checked with decidable comparisons.
    assert_eq!(
        ev_all(&[
            "w := dsp.window(hann, 4)",
            "abs(w[2] - 3/4) <= bound(w, 2) and abs(w[1] - 0) <= bound(w, 1)",
        ]),
        "true"
    );
    // Tapering bulk data is now one honest elementwise step.
    assert_eq!(
        ev_all(&[
            "s := signal([1; 1; 1; 1; 1; 1; 1; 1])",
            "len(s .* dsp.window(hamming, 8))",
        ]),
        "8"
    );
    assert!(ev("dsp.window(kaiser, 8)").starts_with("error: unknown window 'kaiser'"));
    assert!(ev("dsp.window(8, 8)").starts_with("error: dsp.window expects a window name"));
}

// ---------------------------------------------------------------------------
// Special functions and statistical distributions
// ---------------------------------------------------------------------------

#[test]
fn special_functions_fold_at_exact_arguments() {
    // Gamma closes in elementary form at integers and half-integers — an
    // exact value, not an opaque application.
    assert_eq!(ev("gamma(5)"), "24");
    assert_eq!(ev("gamma(7)"), "720");
    assert_eq!(ev("gamma(1/2)"), "sqrt(π)");
    assert_eq!(ev("gamma(3/2)"), "1/2*sqrt(π)");
    assert_eq!(ev("gamma(5/2)"), "3/4*sqrt(π)");
    assert_eq!(ev("erf(0)"), "0");
    assert_eq!(ev("erfc(0)"), "1");
    // Everywhere else they stay symbolic until N(...) — the visible crossing.
    assert!(ev("N(erf(1))").starts_with("0.84270079294971"));
    assert!(ev("N(gamma(1/2))").starts_with("1.77245385090551"));
    assert!(ev("N(beta(2, 3))").starts_with("0.0833333333333")); // 1/12
}

#[test]
fn distributions_evaluate_to_known_values() {
    assert!(ev("N(stats.normcdf(1.96))").starts_with("0.97500210485177"));
    assert!(ev("N(stats.norminv(0.975))").starts_with("1.95996398454005"));
    assert!(ev("N(stats.normcdf(0))").starts_with("0.5")); // exactly one half
    assert!(ev("N(stats.tcdf(2, 5))").starts_with("0.94903026058507"));
    assert!(ev("N(stats.chisqinv(0.95, 1))").starts_with("3.84145882069412"));
    assert!(ev("N(stats.fcdf(1, 10, 10))").starts_with("0.5")); // F(1; d, d) = 1/2
                                                                // The inverse genuinely inverts the forward CDF.
    assert!(ev("N(stats.tcdf(stats.tinv(0.975, 10), 10))").starts_with("0.975"));
    // Arity is checked up front.
    assert!(ev("stats.tcdf(2)").starts_with("error: stats.tcdf expects 2"));
}

#[test]
fn regression_reports_exact_inference() {
    let m = "m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])";
    // Point estimates and fit statistics are exact rationals.
    assert_eq!(ev_all(&[m, "m.coefficients[1]"]), "11/5"); // intercept
    assert_eq!(ev_all(&[m, "m.coefficients[2]"]), "3/5"); // slope
    assert_eq!(ev_all(&[m, "m.r2"]), "3/5");
    assert_eq!(ev_all(&[m, "m.rss"]), "12/5");
    assert_eq!(ev_all(&[m, "m.sigma2"]), "4/5");
    assert_eq!(ev_all(&[m, "m.df"]), "3");
    assert_eq!(ev_all(&[m, "m.fstat"]), "9/2");
    // Standard errors are exact surds; Cook's distance is exactly rational.
    assert_eq!(ev_all(&[m, "m.se[2]"]), "1/5*sqrt(2)");
    assert_eq!(ev_all(&[m, "m.cooks[1]"]), "3/2");
    // The hat-matrix diagonal sums to the parameter count (trace H = k = 2).
    assert_eq!(
        ev_all(&[m, "h := m.leverage", "h[1] + h[2] + h[3] + h[4] + h[5]",]),
        "2"
    );
    // For a simple regression the overall-F p-value equals the slope's
    // two-sided t p-value — a strong internal consistency check.
    let fp = ev_all(&[m, "N(m.fpvalue)"]);
    assert_eq!(fp, ev_all(&[m, "N(m.pvalue[2])"]));
    assert!(fp.starts_with("0.12402706265755"));
}

#[test]
fn regress_uses_an_existing_intercept_column() {
    // An explicit ones column is detected and used rather than duplicated
    // (which would make XᵀX singular); same fit as the auto-intercept form.
    assert_eq!(
        ev("stats.regress([1, 1; 1, 2; 1, 3; 1, 4; 1, 5], [2; 4; 5; 4; 5]).coefficients[2]"),
        "3/5"
    );
    // Too few observations for the parameters is a clean error.
    assert!(
        ev("stats.regress([1; 2], [3; 4])").starts_with("error: stats.regress needs at least 3")
    );
}

const FIT: &str = "m := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])";

#[test]
fn regress_confidence_intervals() {
    // 95% CIs: β ± tinv(39/40, df)·se, kept symbolic; N gives the bounds
    // (verified by hand: slope 0.6 ± 3.1824·0.2828).
    assert_eq!(ev_all(&[FIT, "m.intercept"]), "true");
    assert!(ev_all(&[FIT, "N(m.confint[2, 1])"]).starts_with("-0.30013174529"));
    assert!(ev_all(&[FIT, "N(m.confint[2, 2])"]).starts_with("1.50013174529"));
}

#[test]
fn predict_with_intervals() {
    // Point predictions reattach the intercept automatically.
    assert_eq!(ev_all(&[FIT, "stats.predict(m, [6; 7]).fit[1]"]), "29/5");
    assert_eq!(ev_all(&[FIT, "stats.predict(m, [6; 7]).fit[2]"]), "32/5");
    // The prediction interval is wider than the confidence interval — it
    // carries the extra σ̂² for a fresh observation.
    assert!(ev_all(&[FIT, "N(stats.predict(m, [6; 7]).ci[1, 1])"]).starts_with("2.8146007"));
    assert!(ev_all(&[FIT, "N(stats.predict(m, [6; 7]).pi[1, 1])"]).starts_with("1.6750781"));
}

#[test]
fn regression_assumption_tests() {
    // All three statistics are exact rationals from the residuals.
    assert_eq!(ev_all(&[FIT, "stats.dwtest(m).statistic"]), "121/60"); // Durbin–Watson ≈ 2
    assert_eq!(ev_all(&[FIT, "stats.jbtest(m).statistic"]), "3283/5760"); // Jarque–Bera
    assert_eq!(ev_all(&[FIT, "stats.bptest(m).statistic"]), "25/18"); // Breusch–Pagan
    assert!(ev_all(&[FIT, "N(stats.jbtest(m).pvalue)"]).starts_with("0.7520273"));
}

#[test]
fn robust_se_and_nested_f_test() {
    // HC1 robust slope se equals the textbook sandwich √(43/750) exactly.
    assert_eq!(
        ev_all(&[FIT, "stats.robustse(m, [1; 2; 3; 4; 5]).se[2]"]),
        "1/5*sqrt(43/30)"
    );
    // Nested model F-test: y ~ x vs y ~ x + x².
    let red = "red := stats.regress([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])";
    let full = "full := stats.regress([1, 1; 2, 4; 3, 9; 4, 16; 5, 25], [2; 4; 5; 4; 5])";
    assert_eq!(
        ev_all(&[red, full, "stats.anova(red, full).fstat"]),
        "20/11"
    );
    assert_eq!(ev_all(&[red, full, "stats.anova(red, full).df1"]), "1");
    assert_eq!(ev_all(&[red, full, "stats.anova(red, full).df2"]), "2");
}

#[test]
fn nlfit_recovers_parameters_with_exact_jacobian() {
    // y = a·exp(b·x), a = 2, b = 1/2 — the fit recovers both.
    let f = "f := stats.nlfit(a*exp(b*x), [a, b], [0; 1; 2; 3; 4], \
             [2; 3.29744; 5.43656; 8.96338; 14.7781], [1, 1])";
    assert_eq!(ev_all(&[f, "f.converged"]), "true");
    assert!(ev_all(&[f, "f.coefficients[1]"]).starts_with("2.0000"));
    assert!(ev_all(&[f, "f.coefficients[2]"]).starts_with("0.4999"));
    // The Jacobian columns are the exact analytic ∂f/∂θ, not finite differences.
    assert_eq!(ev_all(&[f, "f.jacobian[1]"]), "exp(b*x)");
    assert_eq!(ev_all(&[f, "f.jacobian[2]"]), "a*x*exp(b*x)");
}

#[test]
fn nlfit_linear_model_matches_ols() {
    // A linear model fit by nonlinear least squares reproduces OLS (2.2, 0.6).
    let f = "stats.nlfit(a + b*x, [a, b], [1; 2; 3; 4; 5], [2; 4; 5; 4; 5], [0, 0])";
    assert!(ev(&format!("{}.coefficients[1]", f)).starts_with("2.19999999"));
    assert!(ev(&format!("{}.coefficients[2]", f)).starts_with("0.60000000"));
}

#[test]
fn nlfit_predict_is_the_fitted_curve() {
    // The fitted a·exp(b·x) (a≈2, b≈1/2) comes back as a `predict` function:
    // at x = 0 it is a·exp(0) = a ≈ 2…
    let f = "m := stats.nlfit(a*exp(b*x), [a, b], [0; 1; 2; 3; 4], \
             [2; 3.29744; 5.43656; 8.96338; 14.7781], [1, 1])";
    assert_eq!(
        ev_all(&[f, "m.predict(0) > 19/10 and m.predict(0) < 21/10"]),
        "true"
    );
    // …and a symbolic argument reconstructs the fitted model expression.
    assert!(ev_all(&[f, "m.predict(x)"]).contains("exp("));
}

#[test]
fn nlfit_argument_errors() {
    // A parameter that never appears in the model is caught.
    assert!(ev("stats.nlfit(a*x, [a, b], [1; 2; 3], [1; 2; 3], [1, 1])")
        .starts_with("error: stats.nlfit: parameter 'b' does not appear"));
    // A bound independent variable leaves nothing to fit against.
    assert!(
        ev_all(&["x := 3", "stats.nlfit(a*x, [a], [1; 2; 3], [1; 2; 3], [1])"])
            .starts_with("error: stats.nlfit: the model has no independent variable")
    );
}

#[test]
fn weighted_least_squares() {
    // Unit weights reproduce OLS exactly.
    assert_eq!(
        ev("stats.wls([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], [1; 1; 1; 1; 1]).coefficients[2]"),
        "3/5"
    );
    // Non-trivial weights, hand-checked: (XᵀWX)⁻¹XᵀWy = [8/11, 5/11].
    assert_eq!(
        ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; 2]).coefficients[1]"),
        "8/11"
    );
    assert_eq!(
        ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; 2]).coefficients[2]"),
        "5/11"
    );
    assert!(ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; 0])").starts_with("error: stats.wls"));
}

#[test]
fn ridge_regression() {
    // λ = 0 is ordinary least squares.
    assert_eq!(
        ev("stats.ridge([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 0).coefficients[2]"),
        "3/5"
    );
    // λ = 1, hand-checked penalized normal equations → [26/11, 6/11], the
    // intercept unpenalized; effective df drops below k = 2.
    let r = "r := stats.ridge([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 1)";
    assert_eq!(ev_all(&[r, "r.coefficients[1]"]), "26/11");
    assert_eq!(ev_all(&[r, "r.coefficients[2]"]), "6/11");
    assert_eq!(ev_all(&[r, "r.edf"]), "21/11");
}

#[test]
fn logistic_regression() {
    // Matches an independent IRLS implementation to ~14 digits.
    let m = "m := stats.logit([1; 2; 3; 4; 5; 6; 7; 8], [0; 0; 0; 1; 0; 1; 1; 1])";
    assert_eq!(ev_all(&[m, "m.converged"]), "true");
    assert!(ev_all(&[m, "m.coefficients[1]"]).starts_with("-5.7703203522912"));
    assert!(ev_all(&[m, "m.coefficients[2]"]).starts_with("1.2822934116202"));
    assert!(ev_all(&[m, "m.se[2]"]).starts_with("0.8604127050525"));
    assert!(ev_all(&[m, "m.deviance"]).starts_with("5.0060993969358"));
    // The response must be binary.
    assert!(ev("stats.logit([1; 2; 3], [0; 1; 2])")
        .starts_with("error: stats.logit: the response must be binary"));
}

#[test]
fn lasso_regression() {
    // λ = 0 recovers ordinary least squares: slope = OLS's 3/5, intercept 11/5.
    // Coefficients are floats (coordinate descent), so compare within tolerance.
    let z = "z := stats.lasso([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 0)";
    assert_eq!(ev_all(&[z, "z.converged"]), "true");
    assert_eq!(
        ev_all(&[z, "abs(z.coefficients[1] - 11/5) < 1/10^6"]),
        "true"
    );
    assert_eq!(
        ev_all(&[z, "abs(z.coefficients[2] - 3/5) < 1/10^6"]),
        "true"
    );
    // λ = 1/5: hand-checked soft-thresholded solution [5/2, 1/2], both active.
    let l = "l := stats.lasso([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 1/5)";
    assert_eq!(
        ev_all(&[l, "abs(l.coefficients[1] - 5/2) < 1/10^6"]),
        "true"
    );
    assert_eq!(
        ev_all(&[l, "abs(l.coefficients[2] - 1/2) < 1/10^6"]),
        "true"
    );
    assert_eq!(ev_all(&[l, "l.df"]), "2");
    // A large penalty drives the slope to *exactly* zero (the L1 corner),
    // leaving the unpenalized intercept at mean(y) = 4; df drops to 1.
    let h = "h := stats.lasso([1; 2; 3; 4; 5], [2; 4; 5; 4; 5], 2)";
    assert_eq!(ev_all(&[h, "h.coefficients[1]"]), "4");
    assert_eq!(ev_all(&[h, "h.coefficients[2]"]), "0");
    assert_eq!(ev_all(&[h, "h.df"]), "1");
    // More predictors than observations is fine for lasso (unlike OLS).
    assert_eq!(
        ev("stats.lasso([1, 0, 2; 0, 1, 1; 1, 1, 0], [1; 2; 3], 1/10).df"),
        "2"
    );
    // The penalty must be a nonnegative number.
    assert!(ev("stats.lasso([1; 2; 3], [1; 2; 3], -1)")
        .starts_with("error: stats.lasso: the penalty lambda must be nonnegative"));
}

#[test]
fn data_namespace_transforms() {
    // Centering and standardizing stay exact (the z-scores are surds).
    assert_eq!(ev("data.center([1; 2; 3; 4; 5])[1]"), "-2");
    assert_eq!(ev("data.standardize([1; 2; 3; 4; 5])[3]"), "0");
    assert!(ev("N(data.standardize([1; 2; 3; 4; 5])[1])").starts_with("-1.264911064067"));
    // Min–max rescaling to [0, 1], exact.
    assert_eq!(ev("data.rescale([10; 20; 30; 40; 50])[2]"), "1/4");
    assert!(ev("data.standardize([7; 7; 7])").starts_with("error: data.standardize"));
}

#[test]
fn data_namespace_dummy_and_groupby() {
    // One-hot encoding: distinct values become indicator columns.
    assert_eq!(ev("data.dummy([a; b; a; c]).levels[2]"), "b");
    assert_eq!(ev("data.dummy([a; b; a]).indicators[1, 1]"), "1"); // row a, column a
    assert_eq!(ev("data.dummy([a; b; a]).indicators[1, 2]"), "0"); // row a, column b
                                                                   // Aggregation by group, exact.
    let g = "g := data.groupby([a; b; a; b; a], [1; 2; 3; 4; 5])";
    assert_eq!(ev_all(&[g, "g.count[1]"]), "3"); // three a's
    assert_eq!(ev_all(&[g, "g.sum[1]"]), "9"); // 1 + 3 + 5
    assert_eq!(ev_all(&[g, "g.mean[2]"]), "3"); // (2 + 4)/2
}

#[test]
fn formula_interface() {
    // The formula form reproduces the matrix form exactly.
    let d = "d := struct(y = [2; 4; 5; 4; 5], x = [1; 2; 3; 4; 5])";
    assert_eq!(
        ev_all(&[d, "stats.regress(y ~ x, d).coefficients[2]"]),
        "3/5"
    );
    // A categorical predictor auto-expands to drop-first dummies; the
    // coefficients are exactly the group-mean contrasts.
    let dc = "d := struct(y = [1; 2; 3; 4; 5; 6], g = [a; a; b; b; c; c])";
    assert_eq!(
        ev_all(&[dc, "stats.regress(y ~ g, d).coefficients[1]"]),
        "3/2"
    ); // mean(a)
    assert_eq!(
        ev_all(&[dc, "stats.regress(y ~ g, d).coefficients[2]"]),
        "2"
    ); // mean(b) − mean(a)
    assert_eq!(
        ev_all(&[dc, "stats.regress(y ~ g, d).coefficients[3]"]),
        "4"
    ); // mean(c) − mean(a)
       // The formula's column names stay symbolic even when bound in the workspace.
    assert_eq!(ev_all(&["x := 99", "y ~ x + z"]), "y ~ x + z");
    // A missing column is a clean error.
    assert!(ev_all(&[
        "d := struct(y = [1; 2; 3], a = [4; 5; 6])",
        "stats.regress(y ~ b, d)"
    ])
    .starts_with("error: stats.regress: the data has no column 'b'"));
}

#[test]
fn formula_transforms_and_interactions() {
    // A term may be any expression in column names: powers, transforms,
    // and products (interactions), evaluated by exact substitution.
    let d = "d := struct(y = [3; 7; 13; 21; 32], x = [1; 2; 3; 4; 5])";
    assert_eq!(
        ev_all(&[d, "stats.regress(y ~ x + x^2, d).k"]),
        "3" // intercept + x + x²
    );
    // An interaction is just a product term.
    let di =
        "d := struct(y = [5; 8; 11; 15; 14; 19], a = [1; 2; 3; 1; 2; 3], b = [1; 1; 1; 2; 2; 2])";
    assert_eq!(ev_all(&[di, "stats.regress(y ~ a + b + a*b, d).k"]), "4");
    // The response can be transformed too: ln(y) ~ x on y = 2^x holds the
    // slope as an exact combination of logs — N(exp(slope)) collapses to 2.
    assert_eq!(
        ev_all(&[
            "d := struct(y = [2; 4; 8; 16; 32; 64], x = [1; 2; 3; 4; 5; 6])",
            "N(exp(stats.regress(ln(y) ~ x, d).coefficients[2]))"
        ]),
        "2"
    );
    // Transforms need numeric columns; categorical ones point at data.dummy.
    assert!(ev_all(&[
        "t := struct(y = [1; 2; 3], g = [us; eu; us])",
        "stats.regress(y ~ ln(g), t)"
    ])
    .contains("categorical"));
    // A constant term is rejected (the intercept is automatic).
    assert!(ev_all(&[d, "stats.regress(y ~ x + 1, d)"]).contains("the intercept is automatic"));
}

#[test]
fn t_tests_match_the_textbook() {
    // One-sample: t = (x̄ − μ)/(s/√n) is an exact surd; p and CI symbolic.
    let t1 = "t := stats.ttest([1; 2; 3; 4; 5], 2)";
    assert_eq!(ev_all(&[t1, "t.estimate"]), "3");
    assert_eq!(ev_all(&[t1, "t.df"]), "4");
    assert_eq!(ev_all(&[t1, "t.statistic^2"]), "2"); // t = √2
    assert!(ev_all(&[t1, "N(t.pvalue)"]).starts_with("0.2301996"));
    // Two-sample Welch: the Satterthwaite df is an exact rational, and the
    // numbers match R's t.test to every printed digit.
    let t2 = "t := stats.ttest([1; 2; 3; 4], [2; 4; 6; 8; 10])";
    assert_eq!(ev_all(&[t2, "t.df"]), "2523/457");
    assert_eq!(ev_all(&[t2, "t.estimate"]), "-7/2");
    assert!(ev_all(&[t2, "N(t.statistic)"]).starts_with("-2.251436"));
    assert!(ev_all(&[t2, "N(t.pvalue)"]).starts_with("0.069133"));
    // Paired: a one-sample test on the differences.
    let t3 = "t := stats.ttest([2; 4; 7], [1; 3; 5], paired)";
    assert_eq!(ev_all(&[t3, "t.statistic"]), "4");
    assert_eq!(ev_all(&[t3, "t.df"]), "2");
    assert!(ev("stats.ttest([2; 4], [1; 3; 5], paired)")
        .starts_with("error: stats.ttest: paired samples must have equal lengths"));
    assert!(ev("stats.ttest([7; 7; 7], 5)")
        .starts_with("error: stats.ttest: the data has zero variance"));
}

#[test]
fn chisq_and_correlation_tests() {
    // Independence on a contingency table: exact statistic and expected
    // counts (matches R's chisq.test with correct = FALSE).
    let c = "c := stats.chisqtest([10, 20; 30, 40])";
    assert_eq!(ev_all(&[c, "c.statistic"]), "50/63");
    assert_eq!(ev_all(&[c, "c.df"]), "1");
    assert_eq!(ev_all(&[c, "c.expected[1, 1]"]), "12");
    assert!(ev_all(&[c, "N(c.pvalue)"]).starts_with("0.372998"));
    // The two-column form cross-tabulates categorical data.
    let c2 = "c := stats.chisqtest([a; a; b; b; a; b], [x; y; x; y; x; y])";
    assert_eq!(ev_all(&[c2, "c.observed[1, 1]"]), "2"); // (a, x) pairs
    assert_eq!(ev_all(&[c2, "c.rows[2]"]), "b");
    assert!(ev("stats.chisqtest([1, 2; 3, -4])")
        .starts_with("error: stats.chisqtest: counts must be nonnegative"));
    assert!(ev("stats.chisqtest([a; a; a], [x; y; x])")
        .starts_with("error: stats.chisqtest needs at least a 2×2 table"));
    // Correlation test: the estimate is the exact surd r.
    let r = "r := stats.cortest([1; 2; 3; 4; 5], [2; 4; 5; 4; 5])";
    assert_eq!(ev_all(&[r, "r.estimate^2"]), "3/5"); // r² = 3/5 exactly
    assert_eq!(ev_all(&[r, "r.df"]), "3");
    assert!(ev_all(&[r, "N(r.statistic)"]).starts_with("2.121320"));
    assert!(ev("stats.cortest([1; 2; 3], [2; 4; 6])")
        .starts_with("error: stats.cortest: the data is perfectly correlated"));
}

#[test]
fn missing_values_are_refused_until_dropped() {
    // NA is an ordinary symbol to the algebra, but every statistical
    // consumer refuses it, pointing at data.dropna.
    assert!(ev("stats.mean([1; NA; 3])").starts_with(
        "error: stats.mean: the data has 1 missing value (NA) — drop the affected rows"
    ));
    assert!(ev("stats.regress([1; 2; NA; 4], [1; 2; 3; 4])")
        .starts_with("error: stats.regress: the data has 1 missing value"));
    assert!(ev("data.dummy([red; NA; red])").starts_with("error: data.dummy: the data has 1"));
    assert!(
        ev("stats.covmat([1, 2; NA, 4; 5, 6])").starts_with("error: stats.covmat: the data has 1")
    );
    // A formula against a table with NA is stopped the same way.
    assert!(ev_all(&[
        "d := struct(y = [1; 2; NA], x = [4; 5; 6])",
        "stats.regress(y ~ x, d)"
    ])
    .starts_with("error: stats.regress: the data has 1 missing value"));
}

#[test]
fn dropna_removes_missing_rows() {
    // A vector loses its NA entries; the mean of what's left is exact.
    assert_eq!(ev("stats.mean(data.dropna([1; NA; 3; NA; 5]))"), "3");
    // A matrix drops rows containing any NA.
    assert_eq!(norm("data.dropna([1, 2; NA, 4; 5, 6])"), "[ 1 2 ] [ 5 6 ]");
    // A table drops rows listwise, keeping the columns aligned.
    assert_eq!(
        ev_all(&[
            "t := struct(x = [1; 2; 3; 4], y = [10; NA; 30; 40])",
            "data.dropna(t).x[2]"
        ]),
        "3"
    );
    // Dropping everything is an error, not an empty value.
    assert!(ev("data.dropna([NA; NA])")
        .starts_with("error: data.dropna: every row has a missing value"));
}

#[test]
fn split_is_seeded_and_reproducible() {
    let t = "t := struct(x = [1; 2; 3; 4; 5; 6; 7; 8; 9; 10], y = [1; 2; 3; 4; 5; 6; 7; 8; 9; 10])";
    // 4/5 of 10 rows: 8 train, 2 test, columns still aligned.
    assert_eq!(ev_all(&[t, "len(data.split(t, 4/5).train.x)"]), "8");
    assert_eq!(ev_all(&[t, "len(data.split(t, 4/5).test.y)"]), "2");
    assert_eq!(
        ev_all(&[t, "s := data.split(t, 4/5)", "s.train.x == s.train.y"]),
        "true"
    );
    // Same seed, same split — the engine stays deterministic.
    assert_eq!(
        ev_all(&[t, "data.split(t, 4/5).test.x == data.split(t, 4/5).test.x"]),
        "true"
    );
    // Vectors and matrices split too (a matrix by rows).
    assert_eq!(ev("len(data.split([1; 2; 3; 4; 5], 3/5).train)"), "3");
    assert_eq!(
        ev("size(data.split([1, 2; 3, 4; 5, 6; 7, 8], 1/2).test).rows"),
        "2"
    );
    // Degenerate fractions are refused.
    assert!(ev("data.split([1; 2; 3], 1/100)")
        .starts_with("error: data.split: that fraction of 3 rows leaves the train side empty"));
    assert!(ev("data.split([1; 2; 3], 2)").starts_with("error: data.split: the train fraction"));
}

#[test]
fn cross_validation_scores_out_of_sample() {
    // A perfectly linear relationship predicts its held-out folds exactly:
    // the CV error is *exactly* zero, not approximately.
    assert_eq!(
        ev("stats.cv([1; 2; 3; 4; 5; 6], [3; 5; 7; 9; 11; 13], 3).rmse"),
        "0"
    );
    // Exact data → exact pooled MSE (a rational) and per-fold MSEs.
    let cv = "c := stats.cv([1; 2; 3; 4; 5; 6; 7; 8], [2; 5; 6; 9; 10; 13; 14; 17], 4)";
    assert_eq!(ev_all(&[cv, "len(c.foldmse)"]), "4");
    assert_eq!(ev_all(&[cv, "c.k"]), "4");
    assert_eq!(ev_all(&[cv, "c.mse == c.rmse^2"]), "true");
    // The formula form works against a table.
    assert_eq!(
        ev_all(&[
            "d := struct(y = [3; 5; 7; 9; 11; 13], x = [1; 2; 3; 4; 5; 6])",
            "stats.cv(y ~ x, d, 3).mse"
        ]),
        "0"
    );
    // A lambda sweep scores every candidate on the same folds and names the
    // winner; for noiseless linear data, no penalty beats any penalty.
    assert_eq!(
        ev_all(&[
            "r := stats.cv([1; 2; 3; 4; 5; 6], [3; 5; 7; 9; 11; 13], 3, struct(model = ridge, lambda = [0; 1; 10]))",
            "r.best"
        ]),
        "0"
    );
    // Option validation.
    assert!(
        ev("stats.cv([1; 2; 3; 4], [1; 2; 3; 4], 2, struct(model = ridge))")
            .starts_with("error: stats.cv: model = ridge needs a lambda")
    );
    assert!(
        ev("stats.cv([1; 2; 3; 4], [1; 2; 3; 4], 2, struct(lambda = 1))")
            .starts_with("error: stats.cv: lambda only applies")
    );
    assert!(ev("stats.cv([1; 2; 3], [1; 2; 3], 4)")
        .starts_with("error: stats.cv: 4 folds need at least 4 observations"));
    assert!(
        ev("stats.cv([1; 2; 3; 4], [1; 2; 3; 4], 2, struct(model = ridge, lambda = -1))")
            .starts_with("error: stats.cv: the penalty lambda must be nonnegative")
    );
}

// ---------------------------------------------------------------------------
// Real algebraic numbers: root(p, k), exact equality, and ordering
// ---------------------------------------------------------------------------

#[test]
fn root_constructs_and_evaluates_real_algebraic_numbers() {
    // Symbolic value, exact numeric refinement on demand.
    assert_eq!(ev("root(x^3 - 2, 1)"), "root(-2 + x^3, 1)");
    assert_eq!(
        ev("N(root(x^3 - 2, 1), 30)"),
        "1.25992104989487316476721060728"
    );
    // A quintic with no radical form at all — the whole point.
    assert_eq!(ev("N(root(x^5 - x - 1, 1), 20)"), "1.1673039782614186843");
    // Rational roots collapse to plain numbers when pinned.
    assert_eq!(ev("root(2*x - 4, 1)"), "2");
    // Validation is loud.
    assert!(ev("root(x^2 + 1, 1)").contains("no real roots"));
    assert!(ev("root(x^2 - 2, 3)").contains("out of range"));
    assert!(ev("root(7, 1)").starts_with("error:"));
    assert!(ev("root(x*y, 1)").starts_with("error:"));
}

#[test]
fn algebraic_equality_decides_what_intervals_cannot() {
    // Roots participate in exact arithmetic comparisons.
    assert_eq!(ev("root(x^3 - 2, 1)^3 == 2"), "true");
    assert_eq!(ev("root(x^2 - 2, 2) == sqrt(2)"), "true");
    assert_eq!(ev("root(x^2 - 2, 1) + root(x^2 - 2, 2) == 0"), "true");
    // Nested-radical identity: √(3+2√2) = 1+√2.
    assert_eq!(ev("sqrt(3 + 2*sqrt(2)) == 1 + sqrt(2)"), "true");
    // Ordering across representations, exact even for near-ties.
    assert_eq!(ev("root(x^5 - x - 1, 1) > 1"), "true");
    assert_eq!(ev("root(x^2 - 2, 2) < root(x^2 - 3, 2)"), "true");
}

#[test]
fn trig_of_rational_pi_is_algebraic() {
    // cos(π/7) has no surd form (cubic minimal polynomial) — but it is
    // exactly the largest root of 8x³ − 4x² − 4x + 1, and satisfies it.
    assert_eq!(
        ev("8*cos(pi/7)^3 - 4*cos(pi/7)^2 - 4*cos(pi/7) + 1 == 0"),
        "true"
    );
    assert_eq!(ev("cos(pi/7) == root(8*x^3 - 4*x^2 - 4*x + 1, 3)"), "true");
    // tan²(π/5) = 5 − 2√5.
    assert_eq!(ev("tan(pi/5)^2 == 5 - 2*sqrt(5)"), "true");
    // Not every constant is algebraic: transcendental ties still refuse.
    assert!(ev("exp(1) <= e").contains("may be equal"));
}

// ---------------------------------------------------------------------------
// Certified IIR: dsp.butter, dsp.stable, dsp.filter, dsp.impz
// ---------------------------------------------------------------------------

#[test]
fn butterworth_designs_are_exact_and_provably_stable() {
    // The classic n = 2, ωc = π/2 design: K = tan(π/4) = 1 folds everything
    // to ℚ(√2) — a2 = (2−√2)/(2+√2), a1 = 0.
    let sos = ev("dsp.butter(2, pi/2).sos");
    assert!(sos.contains("sqrt(2)"), "got: {sos}");
    // Exact unity DC gain (lowpass) and Nyquist gain (highpass), proven.
    assert_eq!(ev("dsp.freqz(dsp.butter(2, pi/2), [0]) == [1]"), "true");
    assert_eq!(ev("dsp.freqz(dsp.butter(3, pi/2), [0]) == [1]"), "true");
    assert_eq!(
        ev("dsp.freqz(dsp.butter(3, pi/2, highpass), [pi]) == [1]"),
        "true"
    );
    assert_eq!(ev("dsp.freqz(dsp.butter(4, pi/2), [pi]) == [0]"), "true");
    // Half-power at the cutoff: |H(ωc)|² = 1/2, to 20 digits.
    assert_eq!(
        ev("N(abs(dsp.freqz(dsp.butter(2, 2/5*pi), [2/5*pi])[1])^2, 20)"),
        "0.5"
    );
    // Every design is certified stable — including off-grid cutoffs.
    for src in [
        "dsp.stable(dsp.butter(1, 2/5*pi))",
        "dsp.stable(dsp.butter(4, 2/5*pi))",
        "dsp.stable(dsp.butter(5, pi/3, highpass))",
    ] {
        assert_eq!(ev(src), "true", "{src}");
    }
    // Domain and argument validation.
    assert!(ev("dsp.butter(2, 4)").contains("outside (0, π)"));
    assert!(ev("dsp.butter(2, x)").starts_with("error:"));
}

#[test]
fn stability_is_decided_exactly_including_quantized_coefficients() {
    // Unstable: z² − 3z + 1 has roots (3±√5)/2 — product exactly 1.
    assert_eq!(ev("dsp.stable([1, -3, 1])"), "false");
    // A double pole exactly ON the circle is not strictly stable.
    assert_eq!(ev("dsp.stable([1, -2, 1])"), "false");
    // Higher-degree, stable: (z² − z/2)(z² + 1/4) expanded.
    assert_eq!(ev("dsp.stable([1, -1/2, 1/4, -1/8, 0])"), "true");
    // THE headline: the fixed-point-quantized coefficients you would deploy
    // are themselves provably stable — not the ideal design, the real one.
    assert_eq!(
        ev("dsp.stable(dsp.quantize(N(dsp.butter(6, 2/5*pi).sos), 15))"),
        "true"
    );
    // Symbolic-constant coefficients work too (certified sign decisions).
    assert_eq!(ev("dsp.stable([1, 0, 1/sqrt(2)])"), "true");
    assert_eq!(ev("dsp.stable([1, 0, sqrt(2)])"), "false");
}

#[test]
fn exact_recursive_filtering_and_impulse_response() {
    // y[n] = x[n] + y[n−1]/2: geometric impulse response, exact.
    assert_eq!(
        ev("dsp.impz([1], [1, -1/2], 5)"),
        "[ 1  1/2  1/4  1/8  1/16 ]"
    );
    // filter(f, x) runs the SOS cascade; a two-section identity filter
    // (b = a) passes the input through unchanged.
    assert_eq!(
        ev("dsp.filter([1, 1/3, 0, 1, 1/3, 0; 1, -1/4, 0, 1, -1/4, 0], [5, 7, -2])"),
        "[ 5  7  -2 ]"
    );
    // Bulk signals are refused with the reason.
    let msg = ev("dsp.filter([1], [1, -1/2], signal([1; 0; 0]))");
    assert!(msg.contains("diverges"), "got: {msg}");
}

// ---------------------------------------------------------------------------
// Remez Types II–IV
// ---------------------------------------------------------------------------

#[test]
fn remez_type_ii_meets_its_spec_exactly() {
    // Even length ⇒ Type II. Stopband edge 2π/3 has u = cos(π/3) = 1/2 —
    // an exact grid point, so compliance there is a decidable exact claim.
    let pre = "f := dsp.remez(16, [0, 1/2*pi, 2/3*pi, pi], [1, 0])";
    assert_eq!(ev_all(&[pre, "f.fir_type"]), "2");
    assert_eq!(
        ev_all(&[pre, "abs(dsp.freqz(f.taps, [2/3*pi])[1])^2 <= f.ripple^2"]),
        "true"
    );
    assert_eq!(
        ev_all(&[pre, "(abs(dsp.freqz(f.taps, [0])[1]) - 1)^2 <= f.ripple^2"]),
        "true"
    );
    // The structural zero at Nyquist, exactly.
    assert_eq!(ev_all(&[pre, "dsp.freqz(f.taps, [pi]) == [0]"]), "true");
}

#[test]
fn remez_type_iii_hilbert_transformer() {
    // Odd length + antisymmetric ⇒ Type III: the classic Hilbert design.
    let pre = "f := dsp.remez(15, [1/3*pi, 2/3*pi], [1], [1], antisymmetric)";
    assert_eq!(ev_all(&[pre, "f.fir_type"]), "3");
    // Forced zeros at BOTH ends, exactly.
    assert_eq!(ev_all(&[pre, "dsp.freqz(f.taps, [0]) == [0]"]), "true");
    assert_eq!(ev_all(&[pre, "dsp.freqz(f.taps, [pi]) == [0]"]), "true");
    // Mid-band the magnitude sits within the ripple of 1 (π/2 is interior,
    // not necessarily a grid point — allow the ripple plus grid slack).
    assert_eq!(
        ev_all(&[
            pre,
            "(abs(dsp.freqz(f.taps, [1/2*pi])[1]) - 1)^2 <= (2*f.ripple)^2"
        ]),
        "true"
    );
}

#[test]
fn remez_type_iv_meets_its_spec_exactly() {
    // Even + antisymmetric ⇒ Type IV. Band [π/3, π]: v = sin(ω/2) is
    // exactly 1/2 and 1 at the edges — both are exact grid points.
    let pre = "f := dsp.remez(8, [1/3*pi, pi], [1], [1], antisymmetric)";
    assert_eq!(ev_all(&[pre, "f.fir_type"]), "4");
    for probe in ["1/3*pi", "pi"] {
        assert_eq!(
            ev_all(&[
                pre,
                &format!("(abs(dsp.freqz(f.taps, [{probe}])[1]) - 1)^2 <= f.ripple^2")
            ]),
            "true",
            "compliance at {probe}"
        );
    }
    // Forced zero at DC, exactly; no forced zero at π.
    assert_eq!(ev_all(&[pre, "dsp.freqz(f.taps, [0]) == [0]"]), "true");
}

// ---------------------------------------------------------------------------
// STFT and spectrogram
// ---------------------------------------------------------------------------

#[test]
fn exact_stft_of_vectors() {
    // A constant input isolates the window: each frame's spectrum is the
    // DFT of the periodic Hann itself — [n/2, −n/4, 0, ..., 0, −n/4].
    assert_eq!(norm("dsp.stft([1, 1, 1, 1], 4, 4).frames"), "[ 2 -1 0 -1 ]");
    // Frame count and hop: 8 samples, nfft 4, hop 2 → 3 frames.
    assert_eq!(ev("len(dsp.stft([1,1,1,1,1,1,1,1], 4, 2).frames)"), "3");
    assert!(ev("dsp.stft([1, 2], 4, 1)").starts_with("error:"));
}

#[test]
fn spectrogram_validates_and_tags() {
    // The value is a tagged drawable; the frontend does the drawing.
    let out =
        ev("spectrogram(signal([1; 2; 3; 4; 5; 6; 7; 8; 9; 10; 11; 12; 13; 14; 15; 16]), 16, 4)");
    assert!(out.starts_with("spectrogram("), "got: {out}");
    // Validation is loud.
    assert!(ev("spectrogram([1, 2, 3])").contains("expects a signal"));
    assert!(ev("spectrogram(signal([1; 2; 3]), 7)").contains("power of two"));
    assert!(ev("spectrogram(signal([1; 2; 3]), 16)").contains("3 samples"));
}

// ---------------------------------------------------------------------------
// z-domain utilities: dsp.tf, dsp.poles, dsp.zeros
// ---------------------------------------------------------------------------

#[test]
fn tf_expands_sos_exactly_and_composes() {
    let pre = "f := dsp.tf(dsp.butter(2, pi/2))";
    // The expanded transfer function keeps every exact identity.
    assert_eq!(ev_all(&[pre, "dsp.freqz(f.b, f.a, [0]) == [1]"]), "true");
    assert_eq!(ev_all(&[pre, "dsp.stable(f.a)"]), "true");
    // Butterworth n=2 at π/2 has a1 = 0 exactly.
    assert_eq!(ev_all(&[pre, "f.a[2] == 0"]), "true");
}

#[test]
fn poles_and_zeros_are_exact() {
    // Rational biquad: z² − 5z/6 + 1/6 = (z − 1/2)(z − 1/3).
    assert_eq!(norm("dsp.poles([1, -5/6, 1/6])"), "[ 1/3 ] [ 1/2 ]");
    // Butterworth zeros: a double −1, proven exactly through the radicals.
    assert_eq!(ev("dsp.zeros(dsp.butter(2, pi/2)) == [-1; -1]"), "true");
    // Complex pole pair: |p|² = a2 exactly (product of conjugate roots).
    assert_eq!(
        ev("abs(dsp.poles(dsp.butter(2, pi/2))[1])^2 == (2-sqrt(2))/(2+sqrt(2))"),
        "true"
    );
    // ...and certified inside the unit circle.
    assert_eq!(ev("abs(dsp.poles(dsp.butter(2, pi/2))[1])^2 < 1"), "true");
    // Degree ≥ 3, all-real: exact root(p, k) values.
    assert_eq!(ev("dsp.poles([1, -6, 11, -6])[2] == 2"), "true");
    // Degree ≥ 3 with complex roots refuses toward SOS form.
    assert!(ev("dsp.poles([1, 0, 0, -1/8])").contains("second-order sections"));
    // A first-order section contributes a genuine zero at the origin, not a
    // padding artifact: b = [1, 1/2], a = [1, 1/3, 1/4] → zeros {0, −1/2}.
    assert_eq!(
        norm("dsp.zeros([1, 1/2, 0, 1, 1/3, 1/4])"),
        "[ 0 ] [ -1/2 ]"
    );
}
