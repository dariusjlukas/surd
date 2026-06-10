//! Browser bindings for the `exact` engine.
//!
//! One `Session` wraps one interpreter. `eval` returns a JSON-encoded
//! [`EvalResult`] so the JS side gets structure (kind, text, LaTeX, plot
//! samples, error) rather than a bare string. The worker that hosts a session
//! is the cancellation boundary: killing the worker and replaying the
//! transcript is the supported way to abort a runaway evaluation.

use exact::expr::Expr;
use exact::{f64eval, latex};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Number of samples per plotted curve. Enough for a smooth 1000-px-wide
/// canvas; cheap to recompute on zoom by re-evaluating the plot line.
const PLOT_SAMPLES: usize = 600;

#[derive(Serialize)]
struct EvalResult {
    ok: bool,
    /// "scalar" | "matrix" | "boolean" | "equation" | "function" | "plot"
    kind: &'static str,
    /// Plain-text rendering (the REPL form; re-parseable).
    text: String,
    /// LaTeX rendering for KaTeX.
    latex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot: Option<PlotData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PlotData {
    /// LaTeX of the plotted expression, for the plot legend.
    latex: String,
    var: String,
    a: f64,
    b: f64,
    /// Sampled (x, y) pairs; y is null at poles / domain gaps.
    points: Vec<(f64, Option<f64>)>,
}

fn error_result(msg: String) -> EvalResult {
    EvalResult {
        ok: false,
        kind: "error",
        text: String::new(),
        latex: String::new(),
        plot: None,
        error: Some(msg),
    }
}

fn kind_of(e: &Expr) -> &'static str {
    match e {
        Expr::Matrix(_) => "matrix",
        Expr::Bool(_) => "boolean",
        Expr::Equation(..) => "equation",
        Expr::Function { .. } => "function",
        _ => "scalar",
    }
}

/// A `plot(f, x, a, b)` value, sampled for drawing. `None` if `e` isn't one.
fn plot_data(e: &Expr) -> Option<Result<PlotData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "plot" || args.len() != 4 {
        return None;
    }
    let Expr::Symbol(var) = &args[1] else {
        return Some(Err("plot: second argument must be a variable".into()));
    };
    let bound = |arg: &Expr, which: &str| {
        f64eval::eval_f64(arg, None)
            .map_err(|e| format!("plot: {} bound is not a number ({})", which, e))
    };
    let a = match bound(&args[2], "lower") {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    let b = match bound(&args[3], "upper") {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    if !(a.is_finite() && b.is_finite() && a < b) {
        return Some(Err("plot: bounds must be finite with a < b".into()));
    }
    let points = f64eval::sample(&args[0], var, a, b, PLOT_SAMPLES);
    if points.iter().all(|(_, y)| y.is_none()) {
        return Some(Err(
            "plot: the expression never evaluates to a real number on this interval".into(),
        ));
    }
    Some(Ok(PlotData {
        latex: latex::to_latex(&args[0]),
        var: var.clone(),
        a,
        b,
        points,
    }))
}

#[wasm_bindgen]
pub struct Session {
    interp: exact::Interpreter,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Session {
        Session {
            interp: exact::Interpreter::new(),
        }
    }

    /// Evaluate one complete statement block; returns JSON ([`EvalResult`]).
    pub fn eval(&mut self, src: &str) -> String {
        let result = match self.interp.eval_line(src) {
            Err(e) => error_result(e),
            Ok(value) => match plot_data(&value) {
                Some(Err(e)) => error_result(e),
                Some(Ok(plot)) => EvalResult {
                    ok: true,
                    kind: "plot",
                    text: format!("{}", value),
                    latex: latex::to_latex(&value),
                    plot: Some(plot),
                    error: None,
                },
                None => EvalResult {
                    ok: true,
                    kind: kind_of(&value),
                    text: format!("{}", value),
                    latex: latex::to_latex(&value),
                    plot: None,
                    error: None,
                },
            },
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }
}

/// Should the REPL keep reading lines before evaluating (unclosed brackets or
/// `if`/`while`/`function` blocks)?
#[wasm_bindgen]
pub fn is_incomplete(src: &str) -> bool {
    exact::lexer::is_incomplete(src)
}

/// Only whitespace/comments — nothing to evaluate.
#[wasm_bindgen]
pub fn is_blank(src: &str) -> bool {
    exact::lexer::is_blank(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_returns_structured_json() {
        let mut s = Session::new();
        let v: serde_json::Value = serde_json::from_str(&s.eval("1/3 + 1/6")).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "scalar");
        assert_eq!(v["text"], "1/2");
        assert_eq!(v["latex"], r"\frac{1}{2}");

        let v: serde_json::Value = serde_json::from_str(&s.eval("1/0")).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "division by zero");
    }

    #[test]
    fn workspace_persists_across_eval_calls() {
        let mut s = Session::new();
        s.eval("x := 3");
        let v: serde_json::Value = serde_json::from_str(&s.eval("x^2 + 1")).unwrap();
        assert_eq!(v["text"], "10");
    }

    #[test]
    fn plot_results_carry_samples() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(sin(x), x, -pi, pi)")).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "plot");
        let pts = v["plot"]["points"].as_array().unwrap();
        assert_eq!(pts.len(), 600);
        // sin over a symmetric interval: first sample is sin(-π) ≈ 0.
        let y0 = pts[0][1].as_f64().unwrap();
        assert!(y0.abs() < 1e-12);
        // a pole-free curve has no gaps
        assert!(pts.iter().all(|p| !p[1].is_null()));

        // 1/x on [-1, 1] has a gap at the pole
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(1/x, x, -1, 1)")).unwrap();
        assert_eq!(v["kind"], "plot");
    }
}
