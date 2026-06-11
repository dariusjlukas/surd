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
/// Surface grid resolution per axis (81×81 = 6 561 samples — cheap, and a
/// finer mesh than a ~600-px canvas can show).
const SURFACE_GRID: usize = 81;
/// Cap on curves per plot — beyond this the legend is unreadable and the
/// caller almost certainly passed a matrix by mistake.
const MAX_SERIES: usize = 12;

#[derive(Serialize)]
struct EvalResult {
    ok: bool,
    /// "scalar" | "matrix" | "boolean" | "equation" | "function" | "plot"
    /// | "plot3d"
    kind: &'static str,
    /// Plain-text rendering (the REPL form; re-parseable).
    text: String,
    /// LaTeX rendering for KaTeX.
    latex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot: Option<PlotData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot3d: Option<Plot3dData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PlotData {
    var: String,
    a: f64,
    b: f64,
    /// One entry per curve, drawn over the shared [a, b] window.
    series: Vec<Series>,
}

#[derive(Serialize)]
struct Series {
    /// LaTeX of the plotted expression, for the plot legend.
    latex: String,
    /// Re-parseable plain text of the plotted expression. Workspace bindings
    /// are already substituted (only the plot variable is free), so the
    /// frontend can resample any window with `resample` — no session state
    /// needed.
    text: String,
    /// Sampled (x, y) pairs; y is null at poles / domain gaps.
    points: Vec<(f64, Option<f64>)>,
}

#[derive(Serialize)]
struct Plot3dData {
    /// LaTeX / re-parseable text of the surface expression (see [`Series`]).
    latex: String,
    text: String,
    xvar: String,
    a: f64,
    b: f64,
    yvar: String,
    c: f64,
    d: f64,
    nx: usize,
    ny: usize,
    /// Row-major heights (y outer, x inner); null at poles / domain gaps.
    heights: Vec<Option<f64>>,
}

fn error_result(msg: String) -> EvalResult {
    EvalResult {
        ok: false,
        kind: "error",
        text: String::new(),
        latex: String::new(),
        plot: None,
        plot3d: None,
        error: Some(msg),
    }
}

fn kind_of(e: &Expr) -> &'static str {
    match e {
        Expr::Matrix(_) => "matrix",
        Expr::Bool(_) => "boolean",
        Expr::Equation(..) => "equation",
        Expr::Function { .. } => "function",
        Expr::Struct(_) => "struct",
        _ => "scalar",
    }
}

fn bound_f64(arg: &Expr, who: &str, which: &str) -> Result<f64, String> {
    f64eval::eval_f64(arg, &[])
        .map_err(|e| format!("{}: {} bound is not a number ({})", who, which, e))
}

/// A `plot(f1, ..., fk, x, a, b)` value, sampled for drawing. `None` if `e`
/// isn't one. Matrix arguments flatten into one curve per entry, so
/// `plot([sin(x); cos(x)], x, a, b)` works too.
fn plot_data(e: &Expr) -> Option<Result<PlotData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "plot" || args.len() < 4 {
        return None;
    }
    let var_idx = args.len() - 3;
    let Expr::Symbol(var) = &args[var_idx] else {
        return Some(Err("plot: the variable argument must be a name".into()));
    };
    Some(plot_data_inner(args, var_idx, var))
}

fn plot_data_inner(args: &[Expr], var_idx: usize, var: &str) -> Result<PlotData, String> {
    let a = bound_f64(&args[var_idx + 1], "plot", "lower")?;
    let b = bound_f64(&args[var_idx + 2], "plot", "upper")?;
    if !(a.is_finite() && b.is_finite() && a < b) {
        return Err("plot: bounds must be finite with a < b".into());
    }
    let mut exprs: Vec<&Expr> = Vec::new();
    for target in &args[..var_idx] {
        match target {
            Expr::Matrix(rows) => exprs.extend(rows.iter().flatten()),
            other => exprs.push(other),
        }
    }
    if exprs.len() > MAX_SERIES {
        return Err(format!(
            "plot: too many curves ({}, max {})",
            exprs.len(),
            MAX_SERIES
        ));
    }
    let mut series = Vec::with_capacity(exprs.len());
    for expr in exprs {
        let points = f64eval::sample(expr, var, a, b, PLOT_SAMPLES);
        if points.iter().all(|(_, y)| y.is_none()) {
            return Err(format!(
                "plot: '{}' never evaluates to a real number on this interval",
                expr
            ));
        }
        series.push(Series {
            latex: latex::to_latex(expr),
            text: format!("{}", expr),
            points,
        });
    }
    Ok(PlotData {
        var: var.to_string(),
        a,
        b,
        series,
    })
}

/// A `plot3d(f, x, a, b, y, c, d)` value, sampled on a grid. `None` if `e`
/// isn't one.
fn plot3d_data(e: &Expr) -> Option<Result<Plot3dData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "plot3d" || args.len() != 7 {
        return None;
    }
    let (Expr::Symbol(xvar), Expr::Symbol(yvar)) = (&args[1], &args[4]) else {
        return Some(Err("plot3d: the variable arguments must be names".into()));
    };
    let inner = || -> Result<Plot3dData, String> {
        let a = bound_f64(&args[2], "plot3d", "lower x")?;
        let b = bound_f64(&args[3], "plot3d", "upper x")?;
        let c = bound_f64(&args[5], "plot3d", "lower y")?;
        let d = bound_f64(&args[6], "plot3d", "upper y")?;
        if !(a.is_finite() && b.is_finite() && a < b && c.is_finite() && d.is_finite() && c < d)
        {
            return Err("plot3d: bounds must be finite with a < b and c < d".into());
        }
        let heights =
            f64eval::sample2d(&args[0], xvar, yvar, a, b, c, d, SURFACE_GRID, SURFACE_GRID);
        if heights.iter().all(|h| h.is_none()) {
            return Err(
                "plot3d: the expression never evaluates to a real number on this domain".into(),
            );
        }
        Ok(Plot3dData {
            latex: latex::to_latex(&args[0]),
            text: format!("{}", args[0]),
            xvar: xvar.clone(),
            a,
            b,
            yvar: yvar.clone(),
            c,
            d,
            nx: SURFACE_GRID,
            ny: SURFACE_GRID,
            heights,
        })
    };
    Some(inner())
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

    /// The global workspace as JSON: `[{name, text, latex, kind}]`, sorted by
    /// name. Drives the variables panel in the UI.
    pub fn workspace(&self) -> String {
        #[derive(Serialize)]
        struct Entry {
            name: String,
            text: String,
            latex: String,
            kind: &'static str,
        }
        let mut entries: Vec<Entry> = self
            .interp
            .workspace()
            .map(|(name, value)| Entry {
                name: name.clone(),
                text: format!("{}", value),
                latex: latex::to_latex(value),
                kind: kind_of(value),
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        serde_json::to_string(&entries).expect("workspace entries are serializable")
    }

    /// Import a raw data file (exact-data JSON, generic JSON, or CSV —
    /// sniffed) and bind the result to `name` in the global workspace.
    /// Returns an [`EvalResult`]-shaped JSON whose `text` is a short import
    /// summary (the value itself can be enormous), kind `"data"`.
    pub fn import_data(&mut self, payload: &str, name: &str) -> String {
        let result = if !exact::dataio::is_valid_var_name(name) {
            error_result(format!("'{}' is not a valid variable name", name))
        } else {
            match exact::dataio::import(payload) {
                Err(e) => error_result(e),
                Ok(value) => {
                    let summary = format!("{}: {}", name, exact::dataio::describe(&value));
                    self.interp.set_global(name, value);
                    EvalResult {
                        ok: true,
                        kind: "data",
                        text: summary,
                        latex: String::new(),
                        plot: None,
                        plot3d: None,
                        error: None,
                    }
                }
            }
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }

    /// Export the named workspace variables as one `exact-data` JSON file.
    /// Returns `{ok, data?, error?}`; `data` is the file's text.
    pub fn export_data(&self, names_json: &str) -> String {
        let inner = || -> Result<String, String> {
            let names: Vec<String> = serde_json::from_str(names_json)
                .map_err(|_| "expected a JSON array of variable names".to_string())?;
            if names.is_empty() {
                return Err("nothing selected to export".into());
            }
            let mut vars: Vec<(&str, &Expr)> = Vec::with_capacity(names.len());
            for n in &names {
                let value = self
                    .interp
                    .get_global(n)
                    .ok_or_else(|| format!("no workspace variable named '{}'", n))?;
                vars.push((n.as_str(), value));
            }
            exact::dataio::export_variables(&vars)
        };
        match inner() {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }).to_string(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }

    /// Evaluate one complete statement block; returns JSON ([`EvalResult`]).
    pub fn eval(&mut self, src: &str) -> String {
        let result = match self.interp.eval_line(src) {
            Err(e) => error_result(e),
            Ok(value) => {
                let ok = |kind, plot, plot3d| EvalResult {
                    ok: true,
                    kind,
                    text: format!("{}", value),
                    latex: latex::to_latex(&value),
                    plot,
                    plot3d,
                    error: None,
                };
                match (plot_data(&value), plot3d_data(&value)) {
                    (Some(Err(e)), _) | (_, Some(Err(e))) => error_result(e),
                    (Some(Ok(plot)), _) => ok("plot", Some(plot), None),
                    (_, Some(Ok(surface))) => ok("plot3d", None, Some(surface)),
                    (None, None) => ok(kind_of(&value), None, None),
                }
            }
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }
}

/// Resample a previously returned plot expression over a new window (zoom /
/// pan). Stateless: `expr_text` is `PlotData::text`, which is closed except
/// for the plot variable, so a scratch interpreter suffices.
#[wasm_bindgen]
pub fn resample(expr_text: &str, var: &str, a: f64, b: f64, n: usize) -> String {
    if !(a.is_finite() && b.is_finite() && a < b) {
        return serde_json::json!({"ok": false, "error": "bounds must be finite with a < b"})
            .to_string();
    }
    let mut interp = exact::Interpreter::new();
    match interp.eval_line(expr_text) {
        Err(e) => serde_json::json!({"ok": false, "error": e}).to_string(),
        Ok(expr) => {
            let points = f64eval::sample(&expr, var, a, b, n);
            serde_json::json!({"ok": true, "points": points}).to_string()
        }
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
    fn workspace_lists_bindings() {
        let mut s = Session::new();
        s.eval("x := 3");
        s.eval("f(n) := n + 1");
        let v: serde_json::Value = serde_json::from_str(&s.workspace()).unwrap();
        let entries = v.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], "f");
        assert_eq!(entries[0]["kind"], "function");
        assert_eq!(entries[1]["name"], "x");
        assert_eq!(entries[1]["text"], "3");
    }

    #[test]
    fn import_binds_a_struct_and_export_round_trips() {
        let mut s = Session::new();
        // CSV with header → struct of column vectors under the given name.
        let v: serde_json::Value =
            serde_json::from_str(&s.import_data("t, temp\n0, 1.5\n1, 2.5\n", "sensor")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["kind"], "data");
        assert!(v["text"].as_str().unwrap().contains("struct with 2 fields"));

        // The struct is live in the workspace; fields are exact.
        let v: serde_json::Value = serde_json::from_str(&s.eval("sensor.temp")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert!(v["text"].as_str().unwrap().contains("3/2"));

        // Export a group of variables and re-import: lands inside one struct,
        // so nothing collides with existing bindings.
        s.eval("x := 1/3");
        let r: serde_json::Value =
            serde_json::from_str(&s.export_data(r#"["x", "sensor"]"#)).unwrap();
        assert_eq!(r["ok"], true, "{}", r["error"]);
        let mut s2 = Session::new();
        s2.eval("x := 999"); // would collide if import didn't wrap
        let v: serde_json::Value =
            serde_json::from_str(&s2.import_data(r["data"].as_str().unwrap(), "saved")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        let v: serde_json::Value = serde_json::from_str(&s2.eval("saved.x + x")).unwrap();
        assert_eq!(v["text"], "2998/3");
        let v: serde_json::Value =
            serde_json::from_str(&s2.eval("saved.sensor.t")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);

        // Errors surface, and bad names are rejected.
        let v: serde_json::Value = serde_json::from_str(&s.import_data("{", "d")).unwrap();
        assert_eq!(v["ok"], false);
        let v: serde_json::Value = serde_json::from_str(&s.import_data("1", "not a name")).unwrap();
        assert_eq!(v["ok"], false);
        let r: serde_json::Value = serde_json::from_str(&s.export_data(r#"["nope"]"#)).unwrap();
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn plot_results_carry_samples() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(sin(x), x, -pi, pi)")).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["kind"], "plot");
        let series = v["plot"]["series"].as_array().unwrap();
        assert_eq!(series.len(), 1);
        let pts = series[0]["points"].as_array().unwrap();
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

    #[test]
    fn multi_curve_plots() {
        let mut s = Session::new();
        // variadic form
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(sin(x), cos(x), x, 0, 1)")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        let series = v["plot"]["series"].as_array().unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0]["text"], "sin(x)");
        assert_eq!(series[1]["text"], "cos(x)");

        // matrix form flattens into one curve per entry
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot([x; x^2; x^3], x, 0, 1)")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["plot"]["series"].as_array().unwrap().len(), 3);

        // workspace bindings substitute into every curve
        s.eval("a := 2");
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(a*x, a*x^2, x, 0, 1)")).unwrap();
        assert_eq!(v["plot"]["series"][0]["text"], "2*x");
    }

    #[test]
    fn plot3d_results_carry_a_grid() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x^2 + y^2, x, -1, 1, y, -1, 1)")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["kind"], "plot3d");
        let p = &v["plot3d"];
        assert_eq!(p["xvar"], "x");
        assert_eq!(p["yvar"], "y");
        let nx = p["nx"].as_u64().unwrap() as usize;
        let ny = p["ny"].as_u64().unwrap() as usize;
        let heights = p["heights"].as_array().unwrap();
        assert_eq!(heights.len(), nx * ny);
        // corner (x=-1, y=-1) → 2; center → 0
        assert!((heights[0].as_f64().unwrap() - 2.0).abs() < 1e-12);
        let center = (ny / 2) * nx + nx / 2;
        assert!(heights[center].as_f64().unwrap().abs() < 1e-12);

        // same variable twice is an error, not a hang
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x^2, x, -1, 1, x, -1, 1)")).unwrap();
        assert_eq!(v["ok"], false);

        // x := 3 must not collapse the surface expression
        s.eval("x := 3");
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x*y, x, 0, 1, y, 0, 1)")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["plot3d"]["text"], "x*y");
    }
}
