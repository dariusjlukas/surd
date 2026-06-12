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
    assert_eq!(norm("[1,2;3,4]^2"), "[ 7 10 ] [ 15 22 ]");
    assert_eq!(norm("transpose([1,2,3;4,5,6])"), "[ 1 4 ] [ 2 5 ] [ 3 6 ]");
    assert_eq!(norm("eye(3)"), "[ 1 0 0 ] [ 0 1 0 ] [ 0 0 1 ]");
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
    assert_eq!(norm("eigenvalues([2,0,0; 0,3,0; 0,0,5])"), "[ 2 ] [ 3 ] [ 5 ]");
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
        norm(
            "inv(eigenvectors([1,-1;1,1])) * [1,-1;1,1] * eigenvectors([1,-1;1,1])"
        ),
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
    assert!(msg.starts_with("error:") && msg.contains("trivial"), "got: {msg}");
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
    assert_eq!(
        norm("solve([1,1;2,2], [3;6]).particular"),
        "[ 3 ] [ 0 ]"
    );
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
    assert!(norm("N(eigenvalues([0,0,0,1; 1,0,0,0; 0,1,0,2; 0,0,1,0]), 20)")
        .starts_with("[ 1.5537739740300373073 ] [ -1.5537739740300373073 ]"));
    // Eigenvectors don't pretend to follow into cubic fields.
    let msg = ev("eigenvectors([0,0,2; 1,0,0; 0,1,0])");
    assert!(msg.starts_with("error:") && msg.contains("radical"), "got: {msg}");
}

#[test]
fn eigenvalue_limits_are_honest() {
    // Three real irrational roots (casus irreducibilis): provably not
    // expressible in real radicals, so we say so rather than approximating.
    assert!(ev("eigenvalues([0,0,-1; 1,0,3; 0,1,0])").contains("casus irreducibilis"));
    // A quartic with odd-power terms needs the full Ferrari reduction.
    assert!(ev("eigenvalues([0,0,0,1; 1,0,0,1; 0,1,0,0; 0,0,1,0])").starts_with("error:"));
    // Degree ≥ 5 has no radical formula at all (Abel–Ruffini).
    assert!(ev("eigenvalues([0,0,0,0,1; 1,0,0,0,1; 0,1,0,0,0; 0,0,1,0,0; 0,0,0,1,0])")
        .starts_with("error:"));
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
    assert!(msg.starts_with("error:") && msg.contains("independent"), "got: {msg}");
}

#[test]
fn precision_context() {
    assert_eq!(
        ev_all(&["precision(50)", "N(pi)"]),
        "3.1415926535897932384626433832795028841971693993751"
    );
    // An explicit digit count still overrides the default.
    assert_eq!(ev_all(&["precision(5)", "N(1/3, 20)"]), "0.33333333333333333333");
    assert_eq!(ev("precision()"), "30"); // factory default
}

#[test]
fn matrix_errors_are_clean() {
    assert_eq!(ev("inv([1,2;2,4])"), "error: matrix is singular (no inverse)");
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
    assert_eq!(
        ev_all(&["x := 99", "f(x) := x + 1", "f(2)", "x"]),
        "99"
    );
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
    assert_eq!(ev_all(&["x := 3", "plot(x^2, x, 0, 1)"]), "plot(x^2, x, 0, 1)");
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
    assert!(ev("dsp.fft([1; 2])").starts_with("error: unknown function 'dsp.fft' (available:"));
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
    assert_eq!(norm("dsp.idft(dsp.dft([1/3; -2; 5/7]))"), "[ 1/3 ] [ -2 ] [ 5/7 ]");
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
    assert_eq!(norm("dsp.conv([1; 2; 1], [1; 1])"), "[ 1 ] [ 3 ] [ 3 ] [ 1 ]");
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
    assert_eq!(ev("stats.mean([1; 2; 3; 4])"), "5/2");
    assert_eq!(ev("stats.var([1; 2; 3; 4])"), "5/3"); // sample variance, n−1
    assert_eq!(ev("stats.std([1; 2; 3; 4])"), "sqrt(5/3)"); // an exact surd
    assert_eq!(ev("stats.median([3; 1; 2])"), "2");
    assert_eq!(ev("stats.median([1; 2; 3; 4])"), "5/2"); // mean of middle two
    assert_eq!(ev("stats.median([1/2; 1/3; 1/4])"), "1/3"); // exact ordering
    assert_eq!(ev("stats.cov([1; 2; 3], [2; 4; 6])"), "2");
    // Symbolic entries flow through estimators that don't need ordering.
    assert_eq!(ev("stats.mean([a; b])"), "1/2*a + 1/2*b");
}

#[test]
fn correlation_of_linear_data_is_exactly_one() {
    // No float tool gets ±1 exactly; the surds cancel by radical merging.
    assert_eq!(ev("stats.cor([1; 2; 3], [2; 4; 6])"), "1");
    assert_eq!(ev("stats.cor([1; 2; 3], [5; 3; 1])"), "-1");
}

#[test]
fn linfit_is_exact_least_squares() {
    assert_eq!(
        ev("stats.linfit([1; 2; 3; 4], [3; 5; 7; 9])"),
        "struct(intercept = 1, slope = 2)"
    );
    // Hand-checked OLS: x̄=1, ȳ=7/3, Sxx=2, Sxy=3.
    assert_eq!(
        ev("stats.linfit([0; 1; 2], [1; 2; 4])"),
        "struct(intercept = 5/6, slope = 3/2)"
    );
}

#[test]
fn stats_errors_are_graceful() {
    assert!(ev("stats.median([x; 1])").starts_with("error: stats.median needs numeric"));
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
    // separate, so the comparison refuses instead of inventing an answer.
    let msg = ev("(sqrt(2)+sqrt(3))^2 < 5 + 2*sqrt(6)");
    assert!(msg.starts_with("error:") && msg.contains("may be equal"), "got: {msg}");
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
    assert_eq!(
        ev_all(&["f(x) := x^2 + 1", "map(f, [1, 2])"]),
        "[ 2  5 ]"
    );
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
