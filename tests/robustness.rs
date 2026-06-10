//! Robustness / fuzzing: the engine must never panic, hang, or overflow the
//! stack on *any* input — only ever return a value or an error. Critical for a
//! tool that will take untrusted input in the browser.

mod common;
use common::*;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 1500, ..ProptestConfig::default() })]

    /// Arbitrary printable text (including non-ASCII) must not panic.
    #[test]
    fn never_panics_on_arbitrary_text(s in "\\PC{0,120}") {
        let _ = ev(&s);
    }

    /// "Math soup" — only characters the lexer/parser care about — stresses the
    /// grammar far harder than arbitrary text does.
    #[test]
    fn never_panics_on_math_soup(s in "[-+*/^()\\[\\]{};,.=<>!: a-zA-Z0-9\n]{0,120}") {
        let _ = ev(&s);
    }

    /// Multi-statement programs of soup.
    #[test]
    fn never_panics_on_program_soup(
        lines in proptest::collection::vec("[-+*/^()=<> a-zI0-9]{0,30}", 0..6)
    ) {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let _ = ev_all(&refs);
    }
}

/// A curated set of nasty inputs. Each may legitimately error; none may crash.
#[test]
fn adversarial_inputs_never_crash() {
    let cases = [
        "",
        "   ",
        "\n\n\n",
        "((((",
        "))))",
        "[[[[",
        "1 2 3",
        "+-*/^",
        ":=",
        "=",
        "==",
        "<=>=",
        "[1,2",
        "1;;;2",
        "sin",
        "sin(",
        "sin()",
        "x^^2",
        "1..2",
        "........",
        "if",
        "if then end",
        "while do",
        "function end",
        "f(",
        ")(",
        "2^2^2^2",
        "1/0",
        "0^-1",
        "[1,2;3]",
        "N()",
        "diff(1)",
        "eigenvalues(1)",
        "det([1,2,3])",
        "sqrt(-1)",
        "I^I",
        "ln(0)",
        "1e999",
        "----x",
        "x = = y",
        "not not not 1",
        "\0",
        "π + 𝕏",
        "🎉 * 2",
        "../../etc",
        "0/0/0/0",
    ];
    for case in cases {
        // The whole point: this call returns (Ok or Err) and never panics.
        let _ = ev(case);
    }
}

/// Resource guards turn pathological inputs into clean errors, not crashes.
#[test]
fn resource_guards_error_gracefully() {
    // Astronomically large exponent stays symbolic (no multi-GB bignum).
    assert!(!is_err(&ev("2^999999999999999")));
    // Deeply nested input is rejected, not overflowed.
    let nested = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(is_err(&ev(&nested)));
    // A very long flat chain is rejected by the input-size cap.
    let long_chain = format!("1{}", "+1".repeat(50_000));
    assert!(is_err(&ev(&long_chain)));
    // Runaway recursion errors instead of overflowing.
    assert!(is_err(&ev_all(&[
        "f(n) := if n == 0 then 0 else f(n-1) end",
        "f(50000)",
    ])));
}
