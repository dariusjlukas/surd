//! Behavioral tests. These double as executable documentation of what the
//! engine currently guarantees.

use exact::Interpreter;

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
fn eigenvalue_limits_are_honest() {
    // Irreducible cubic (companion of x³ − 2): cube root of 2 has no rational
    // or quadratic-factor form, so we say so rather than approximating.
    assert!(ev("eigenvalues([0,0,2; 1,0,0; 0,1,0])").starts_with("error:"));
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
    assert!(ev("pi < 4").starts_with("error:")); // ordering a symbolic constant
    assert!(ev("2 + true").starts_with("error:")); // arithmetic on a boolean
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
