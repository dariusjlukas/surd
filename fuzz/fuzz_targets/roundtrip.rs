//! Differential fuzz target: a printed result must re-evaluate to itself
//! (canonical form is a fixed point). This is what caught `(x+1)-(x+1)` failing
//! to cancel — the display `1 + x - 1 + x` re-evaluated to `2*x`.
//!
//! We skip the displays that intentionally don't round-trip: floats (from `N`,
//! which print as decimals that re-parse to exact rationals), matrices
//! (multi-line), and function values.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // Floats only come from `N`, and their decimal display re-parses to an
    // exact rational — not a round-trip, by design.
    if text.contains('N') {
        return;
    }
    let text = text.to_owned();
    exact::run_with_stack(move || {
        let mut interp = exact::Interpreter::new();
        let first = match interp.eval_line(&text) {
            Ok(value) => format!("{}", value),
            Err(_) => return,
        };
        // Matrices (multi-line) and function values don't re-parse.
        if first.contains('\n') || first.contains("function") {
            return;
        }
        // Re-evaluate on a fresh interpreter so definitions in `text` can't
        // change the meaning of the printed form.
        let mut fresh = exact::Interpreter::new();
        let second = match fresh.eval_line(&first) {
            Ok(value) => format!("{}", value),
            // Conservatively skip rather than flag (the eval target already
            // guarantees no panic); we only care about non-idempotence here.
            Err(_) => return,
        };
        assert_eq!(
            first, second,
            "canonical form is not idempotent for input {:?}",
            text
        );
    });
});
