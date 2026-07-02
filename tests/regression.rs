//! Regression corpus: one test per bug actually hit during development, so none
//! can silently come back. Each comment records what broke.

mod common;
use common::*;

#[test]
fn division_by_zero_is_an_error_not_a_panic() {
    // Was: `1/0` panicked inside num-rational (recip of 0).
    assert_eq!(ev("1/0"), "error: division by zero");
    assert_eq!(ev("1/(x - x)"), "error: division by zero");
}

#[test]
fn expand_terminates() {
    // Was: expand((x+1)^2) looped forever, because mul re-folded (x+1)*(x+1)
    // back into (x+1)^2, which expand then re-expanded.
    assert_eq!(ev("expand((x+1)^2)"), "1 + x^2 + 2*x");
    assert_eq!(ev("expand((x+1)^3)"), "1 + x^3 + 3*x + 3*x^2");
}

#[test]
fn composite_terms_cancel() {
    // Was: (x+1) - (x+1) did NOT simplify to 0, because -(x+1) stayed an opaque
    // Mul([-1, Add]) while +(x+1) flattened. Fixed by distributing a numeric
    // coefficient over a sum. (Found by property testing.)
    assert_eq!(ev("(x + 1) - (x + 1)"), "0");
    assert_eq!(ev("2*(x + y) - 2*x - 2*y"), "0");
    assert_eq!(ev("(x + 1)*3 - 3*x - 3"), "0");
}

#[test]
fn negative_rational_coefficients_display_with_minus() {
    // Was: a term like Mul([-1/2, sqrt(5)]) printed as "+ -1/2*sqrt(5)" because
    // negative_part only recognized a leading Int(-1), not a negative rational.
    assert_eq!(ev("1/2 - 1/2*sqrt(5)"), "1/2 - 1/2*sqrt(5)");
}

#[test]
fn implicit_multiplication() {
    // Was: `2x` was a parse error. Now adjacency means multiplication in the
    // unambiguous cases (number/`)` followed by `(`/identifier).
    assert_eq!(ev("2pi + pi"), "3*π");
    assert_eq!(ev_all(&["x := 3", "2x + 1"]), "7");
    assert_eq!(ev("2(3 + 4)"), "14");
    assert_eq!(ev("2sin(0)"), "0");
    assert_eq!(ev("(x + 1)(x - 1) - (x + 1)*(x - 1)"), "0");
    assert_eq!(ev("(x + 1)y - y*(x + 1)"), "0");
    // 1/2x reads as (1/2)*x, like 1/2*x.
    assert_eq!(ev("1/2x - x/2"), "0");
    // Exponents bind first: x^2y is (x^2)*y, not x^(2y).
    assert_eq!(ev("x^2y - y*x^2"), "0");
    // Still errors — these adjacencies stay meaningless or dangerous:
    assert!(is_err(&ev("1.5.5"))); // Num·Num: a typo, not multiplication
    assert!(is_err(&ev("x y"))); // Ident·Ident would break `x then …` grammar
    assert!(is_err(&ev("3e5"))); // scientific notation must not become 3*e5
}

#[test]
fn pi_input_alias_and_round_trip() {
    // Display uses "π"; the parser must accept it back (and "pi").
    assert_eq!(ev("pi"), "π");
    assert_eq!(ev("π"), "π");
    assert_eq!(ev("π"), ev("pi"));
}

#[test]
fn sqrt_of_negative_is_imaginary() {
    assert_eq!(ev("sqrt(-1)"), "I");
    assert_eq!(ev("sqrt(-4)"), "2*I");
}

#[test]
fn lowercase_i_is_a_free_variable() {
    // The imaginary unit is capital I, so loop counters named `i` still work.
    assert_eq!(ev_all(&["i := 5", "i + 1"]), "6");
}

#[test]
fn huge_exponent_does_not_hang() {
    // Was: 2^(10^15) tried to build an astronomically large bignum.
    assert_eq!(ev("2^999999999999999"), "2^999999999999999");
}

#[test]
fn oversized_power_result_stays_symbolic() {
    // Was: 200000000000003^114974 built a ~1.6-million-digit number (seconds to
    // evaluate) — the exponent is small, but big-base^small-exp is still huge.
    // The cap bounds the *result* size. Found by the roundtrip fuzzer's
    // slow-unit detector.
    assert!(ev("200000000000003^114974").starts_with("200000000000003^"));
    assert_eq!(ev("2^10"), "1024"); // small results still compute
    assert_eq!(ev("10^1000").len(), 1001); // a 1001-digit number is fine
}

#[test]
fn complex_numeric_parts_display_cleanly() {
    // Was: N of ±i printed "0 + 1*I" / "0 + -1*I" because the parts are floats,
    // not Int(0)/Int(1). Fixed with a string-based complex formatter.
    assert_eq!(norm("N(eigenvalues([1,-2; 1,-1]), 12)"), "[ I ] [ -I ]");
}

#[test]
fn eulers_identity_snaps_to_minus_one() {
    // Was/ensures: the negligible imaginary residue of exp(iπ) is snapped away.
    assert_eq!(ev("N(exp(I*pi))"), "-1");
}

#[test]
fn equations_are_not_booleans() {
    // `=` builds an equation (data); only `==` tests equality.
    assert_eq!(ev("1 = 1"), "1 = 1");
    assert_eq!(ev("1 == 1"), "true");
}

#[test]
fn rational_base_of_a_power_is_parenthesized() {
    // Was: (11/5)^x printed as "11/5^x", which re-parses as 11/(5^x) — a
    // different expression. Found by the roundtrip fuzz target.
    assert_eq!(ev("(11/5)^x"), "(11/5)^x");
    assert_eq!(ev("(2/3)^x"), "(2/3)^x");
    // The full fuzzer reproducer now round-trips.
    assert_eq!(ev(&ev("6*xx - 4*2.2^x")), ev("6*xx - 4*2.2^x"));
}

#[test]
fn exact_inverse_has_no_roundoff() {
    // The thesis in one line: a float tool gives 11.9999…; this is exact.
    assert_eq!(norm("inv([1/2, 1/3; 1/4, 1/5])"), "[ 12 -20 ] [ -15 30 ]");
}

#[test]
fn rational_exponents_respect_the_power_cap() {
    // Was: exact_rational_root bypassed the result-size cap — 8^(2000003/3)
    // built a 600k-digit number, and a numerator beyond usize (10^20/3)
    // panicked in rat_pow and crashed the process.
    assert_eq!(ev("8^(2000003/3)"), "8^(2000003/3)");
    assert!(ev("8^(100000000000000000000/3)").starts_with("8^"));
    assert_eq!(ev("8^(2/3)"), "4"); // small exact roots still fold
}

#[test]
fn floats_compare_by_exact_value() {
    // Was: `pi < 4` said "try N(...)" but `N(pi) < 4` failed the same way —
    // comparison only accepted Int/Rat. Floats now compare via the exact
    // rational they are (m·2^k), so the decision is lossless, never rounded.
    assert_eq!(ev("N(pi) < 4"), "true");
    assert_eq!(ev("N(pi) > 22/7"), "false"); // π < 22/7, decided exactly
    assert_eq!(ev("N(2) == 2"), "true"); // numbers test equal by value
    assert_eq!(ev("N(1/3) == 1/3"), "false"); // the float is NOT exactly 1/3
}

#[test]
fn constants_can_be_shadowed_by_assignment() {
    // Was: `e := 5` succeeded but `e` still evaluated to Euler's constant —
    // lookup checked constants before the workspace. Bindings now shadow
    // pi/e (like I); true/false are literals and reject assignment.
    assert_eq!(ev_all(&["e := 5", "e + 1"]), "6");
    assert_eq!(ev_all(&["pi := 3", "pi"]), "3");
    assert_eq!(ev("e"), "e"); // unbound, still the constant
    assert!(is_err(&ev("true := 1")));
}

#[test]
fn tiny_exact_imaginary_parts_survive_n() {
    // Was: snap_negligible zeroed any component below 10^-digits relative to
    // the other — deleting genuinely nonzero exact input like 1 + 10^-50·I.
    // Snapping now applies only to transcendental results, where a residue
    // can stand in for a mathematical zero.
    assert_eq!(ev("N(1 + 10^(-50)*I, 30)"), "1 + 1e-50*I");
    assert_eq!(ev("N(exp(I*pi))"), "-1"); // residue snapping still works
}

#[test]
fn diff_and_subs_see_through_workspace_bindings() {
    // Was: x := 3 turned diff(x^2, x) into diff(9, 3) — an error. The
    // variable argument is now taken by name and kept symbolic while the
    // expression evaluates; for diff the binding is substituted back after.
    assert_eq!(ev_all(&["x := 3", "diff(x^2, x)"]), "6"); // 2x at x = 3
    assert_eq!(ev_all(&["x := 3", "subs(x^2 + x, x, 10)"]), "110");
    assert_eq!(ev("diff(sin(y), y)"), "cos(y)"); // unbound is unchanged
}

#[test]
fn negated_product_of_sums_display_is_a_fixed_point() {
    // Was: `(z-z) - (x-1)*(1+x)` printed as `-(-1 + x)*(1 + x)`, which
    // re-parses with the minus bound to the first factor alone; `mul` then
    // distributes it into the sum, yielding the *different* canonical form
    // `(1 + x)*(1 - x)` (found by the display round-trip property). A
    // sign/coefficient before a leading parenthesized sum now groups the
    // factors: `-((-1 + x)*(1 + x))`.
    assert_eq!(ev("(z - z) - (x + (-1))*(1 + x)"), "-((-1 + x)*(1 + x))");
    assert_eq!(ev("-((-1 + x)*(1 + x))"), "-((-1 + x)*(1 + x))");
    // Same trap with a plain coefficient (built via a binding, since direct
    // source distributes immediately under left-associative parsing).
    assert_eq!(ev_all(&["b := (1+x)*(1+y)", "2*b"]), "2*((1 + x)*(1 + y))");
    assert_eq!(ev("2*((1 + x)*(1 + y))"), "2*((1 + x)*(1 + y))");
    // Unaffected shapes keep their plain rendering.
    assert_eq!(ev("-x*(1 + y)"), "-x*(1 + y)");
    assert_eq!(ev_all(&["c := x*(1+y)", "2*c"]), "2*x*(1 + y)");
}

#[test]
fn low_precision_n_of_products_and_sums_works() {
    // Was: `N(2*pi, 8)` (any Mul/Add at ≤ 9 digits) errored with "numeric
    // result is undefined". The working precision for ≤ 9 digits came to
    // under 64 bits, and astro-float's `from_i64` returns NaN(InvalidArgument)
    // below one word — poisoning the Mul/Add accumulators, which start from
    // `from_i64(1)` / `from_i64(0)`. Precision is now floored at 64 bits.
    // (Found driving N(...) over symbolic dsp.dft entries at low digits.)
    assert_eq!(ev("N(2*pi, 8)"), "6.2831853");
    assert_eq!(ev("N(1 + pi, 8)"), "4.1415927");
    assert_eq!(ev("N(2*sqrt(2), 1)"), "3");
    assert_eq!(ev("N(cos(2/5*pi), 8)"), "0.30901699");
}

#[test]
fn container_values_cannot_reach_scalar_positions() {
    // Was: a container value could slip into a scalar position and come out of
    // canonicalization as well-sorted nonsense — `dot([[1,2], 2], [3,4])`
    // evaluated to `8 + 3*[ 1  2 ]`, `[[1,2], 3]` built a nested matrix, and
    // `linspace(1, [1,2], 3)` put matrices inside entries. Every path that
    // feeds user values into scalar positions (matrix entries, `subs`
    // replacements, `linspace` endpoints, `map`/`fill` results) now checks
    // `is_scalar` first. (Found by code review.)
    assert!(ev("[[1,2], 3]").starts_with("error: matrices don't nest"));
    assert!(ev("dot([[1,2], 2], [3, 4])").starts_with("error: matrices don't nest"));
    assert!(ev("linspace(1, [1, 2], 3)").starts_with("error: linspace expects scalar endpoints"));
    assert!(ev("subs(x + 1, x, [1, 2])").starts_with("error: subs replaces a variable"));
    assert!(ev("[true, false]").starts_with("error: a matrix entry must be a scalar"));
    assert!(ev_all(&["g(i, j) := [i, j]", "fill(g, 2)"])
        .starts_with("error: fill: the fill function must return a scalar"));
    assert!(ev_all(&["g(x) := [x, x]", "map(g, [1, 2])"])
        .starts_with("error: map: the mapped function must return a scalar"));
    assert!(ev_all(&["x := [1; 2]", "D(x^2, x)"]).starts_with("error: x is bound to"));
    // Symbolic entries are still the point of a CAS matrix, and a *matrix*
    // substituted for the whole target of subs was never the contract.
    assert_eq!(norm("[a + 1, 2]"), "[ 1 + a 2 ]");
    assert_eq!(ev("subs(x^2 + x, x, y + 1)"), "1 + y + (1 + y)^2");
}

#[test]
fn wls_rejects_symbolic_and_nonpositive_weights() {
    // Was: the positivity check only looked at *numeric* weights, so a
    // symbolic weight sailed through into √wᵢ and ln wᵢ and surfaced later as
    // a confusing downstream error. (Found by code review.)
    assert!(ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; w])")
        .starts_with("error: stats.wls: weights must be positive numbers"));
    assert!(ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; 0])")
        .starts_with("error: stats.wls: weights must be positive numbers"));
    assert!(ev("stats.wls([1; 2; 3], [1; 2; 2], [1; 1; -2])")
        .starts_with("error: stats.wls: weights must be positive numbers"));
}
