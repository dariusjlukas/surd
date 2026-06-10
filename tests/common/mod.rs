//! Shared test helpers. Every evaluation runs on a large-stack thread (via
//! `exact::run_with_stack`) so deeply nested generated inputs hit the
//! evaluator's depth guards instead of overflowing a bare 2 MB test thread.
#![allow(dead_code)]

use exact::Interpreter;

/// Evaluate one line on a fresh interpreter; errors come back as
/// `"error: ..."` strings so tests can assert on them uniformly.
pub fn ev(src: &str) -> String {
    let src = src.to_string();
    exact::run_with_stack(move || {
        let mut interp = Interpreter::new();
        match interp.eval_line(&src) {
            Ok(value) => format!("{}", value),
            Err(err) => format!("error: {}", err),
        }
    })
}

/// Evaluate several lines on one interpreter; return the last result.
pub fn ev_all(lines: &[&str]) -> String {
    let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    exact::run_with_stack(move || {
        let mut interp = Interpreter::new();
        let mut out = String::new();
        for line in &lines {
            out = match interp.eval_line(line) {
                Ok(value) => format!("{}", value),
                Err(err) => format!("error: {}", err),
            };
        }
        out
    })
}

/// Collapse whitespace/newlines so multi-line matrix output can be compared.
pub fn norm(src: &str) -> String {
    ev(src).split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn is_err(out: &str) -> bool {
    out.starts_with("error:")
}
