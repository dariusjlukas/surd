//! Browser bindings for the `surd` engine.
//!
//! One `Session` wraps one interpreter. `eval` returns a JSON-encoded
//! [`EvalResult`] so the JS side gets structure (kind, text, LaTeX, plot
//! samples, error) rather than a bare string. The worker that hosts a session
//! is the cancellation boundary: killing the worker and replaying the
//! transcript is the supported way to abort a runaway evaluation.

use serde::Serialize;
use std::collections::BTreeSet;
use surd::ast::{ForIter, IndexArg, Node, Step};
use surd::expr::Expr;
use surd::{f64eval, latex};
use wasm_bindgen::prelude::*;

/// Base number of samples per plotted curve. Enough for a smooth
/// 1000-px-wide canvas; cheap to recompute on zoom by re-evaluating the plot
/// line.
const PLOT_SAMPLES: usize = 601;
/// Per-curve sample cap. Like surfaces, curves sample adaptively
/// (601 → 1201 → 2401 → 4801 while the convergence test fails); the cap is
/// ~8 samples per pixel on a typical canvas — oscillations finer than that
/// can't be drawn anyway, so past it the curve is flagged `undersampled`
/// instead.
const PLOT_SAMPLES_MAX: usize = 4801;
/// Surface base grid resolution per axis (81×81 = 6 561 samples — cheap, and
/// a finer mesh than a ~600-px canvas can show for a smooth surface).
const SURFACE_GRID: usize = 81;
/// Surface grid cap. Sampling is adaptive (81 → 161 → 321 → 641, doubling
/// cell density while the grid fails its convergence test — see
/// `f64eval::sample2d_adaptive`); the cap bounds a worst-case surface at
/// ~550k evaluations. Windows the cap can't certify come back flagged
/// `undersampled` rather than silently aliased.
const SURFACE_GRID_MAX: usize = 641;
/// Cap on curves per plot — beyond this the legend is unreadable and the
/// caller almost certainly passed a matrix by mistake.
const MAX_SERIES: usize = 12;

#[derive(Serialize)]
struct EvalResult {
    ok: bool,
    /// "scalar" | "matrix" | "boolean" | "equation" | "function" | "plot"
    /// | "plot3d" | "splom"
    kind: &'static str,
    /// Plain-text rendering (the REPL form; re-parseable).
    text: String,
    /// LaTeX rendering for KaTeX.
    latex: String,
    /// True when the input ended in `;` (MATLAB/Julia output suppression): the
    /// value was still computed and the workspace still updated, but the cell
    /// should render compactly instead of echoing a possibly-huge matrix.
    #[serde(skip_serializing_if = "is_false")]
    suppressed: bool,
    /// A one-line shape hint (e.g. `"5×3 matrix"`, `"8-vector"`) for the
    /// compact rendering of a suppressed result. Absent unless `suppressed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot: Option<PlotData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot3d: Option<Plot3dData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    splom: Option<SplomData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spectrogram: Option<SpectrogramData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PlotData {
    var: String,
    a: f64,
    b: f64,
    /// Registry id for signal plots (zoom refinement asks the session to
    /// re-decimate); absent for function plots, which resample by text.
    #[serde(skip_serializing_if = "Option::is_none")]
    sig: Option<u32>,
    /// One entry per curve, drawn over the shared [a, b] window.
    series: Vec<Series>,
    /// Optional figure title / axis labels from `plot(..., title = "...")`.
    /// Mathtext: plain text with `$...$` segments rendered as LaTeX.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    xlabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ylabel: Option<String>,
}

/// Trailing `key = "text"` label equations of a tagged plot value, peeled
/// back off (the mirror of eval's `attach_plot_labels`). Returns the
/// positional prefix and the labels.
#[derive(Default)]
struct PlotLabels {
    title: Option<String>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    zlabel: Option<String>,
}

fn split_plot_labels(args: &[Expr]) -> (&[Expr], PlotLabels) {
    let mut labels = PlotLabels::default();
    let mut end = args.len();
    while end > 0 {
        let Expr::Equation(l, r) = &args[end - 1] else {
            break;
        };
        let (Expr::Symbol(key), Expr::Str(text)) = (l.as_ref(), r.as_ref()) else {
            break;
        };
        let slot = match key.as_str() {
            "title" => &mut labels.title,
            "xlabel" => &mut labels.xlabel,
            "ylabel" => &mut labels.ylabel,
            "zlabel" => &mut labels.zlabel,
            _ => break,
        };
        *slot = Some(text.clone());
        end -= 1;
    }
    (&args[..end], labels)
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
    /// True when even the finest sampling resolution failed its convergence
    /// test on this window — the curve may alias and the UI must say so.
    undersampled: bool,
    /// True for static data series (signals, scatter): all points are already
    /// here, and `text` cannot be resampled — the frontend re-windows
    /// client-side.
    fixed: bool,
    /// True for scatter series: the frontend draws discrete markers instead of
    /// a connected line. Omitted (defaults false) for curves and signals.
    #[serde(skip_serializing_if = "is_false")]
    scatter: bool,
    /// Sampled (x, y) pairs; y is null at poles / domain gaps.
    points: Vec<(f64, Option<f64>)>,
}

fn is_false(b: &bool) -> bool {
    !*b
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
    /// True when even the finest sampling grid failed its convergence test
    /// on this window — the surface may alias and the UI must say so.
    undersampled: bool,
    /// Row-major heights (y outer, x inner); null at poles / domain gaps.
    /// Empty (with `nx` = 0) for a points-only plot.
    heights: Vec<Option<f64>>,
    /// 3D scatter markers `(x, y, z)` in data coordinates; omitted when none.
    /// Static data — the frontend boxes and re-windows them without resampling.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scatter: Vec<(f64, f64, f64)>,
    /// Optional figure title / axis labels from `plot3d(..., title = "...")`.
    /// Mathtext: plain text with `$...$` segments rendered as LaTeX.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    xlabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ylabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    zlabel: Option<String>,
}

fn error_result(msg: String) -> EvalResult {
    EvalResult {
        ok: false,
        kind: "error",
        text: String::new(),
        latex: String::new(),
        suppressed: false,
        summary: None,
        plot: None,
        plot3d: None,
        splom: None,
        spectrogram: None,
        error: Some(msg),
    }
}

/// A one-line shape hint for a suppressed result, so the compact cell still
/// says *what* it hid — the dimensions of a matrix/vector being exactly the
/// thing a user suppressing a "large matrix or vector" wants reassured about.
fn shape_summary(e: &Expr) -> String {
    match e {
        Expr::Matrix(rows) => {
            let r = rows.len();
            let c = rows.first().map_or(0, |row| row.len());
            match (r, c) {
                (n, 1) => format!("{n}-vector"),
                (1, n) => format!("{n}-vector"),
                (r, c) => format!("{r}×{c} matrix"),
            }
        }
        other => kind_of(other).to_string(),
    }
}

/// Standard base64 (no line breaks). The transport for binary-export bytes
/// over the string-based worker protocol; the desktop save command and the
/// web download path both decode it back to raw bytes.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn kind_of(e: &Expr) -> &'static str {
    match e {
        Expr::Matrix(_) => "matrix",
        Expr::Bool(_) => "boolean",
        Expr::Equation(..) => "equation",
        Expr::Formula(..) => "formula",
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
    if name != "plot" {
        return None;
    }
    let (args, labels) = split_plot_labels(args);
    if args.len() < 4 {
        return None;
    }
    let var_idx = args.len() - 3;
    let Expr::Symbol(var) = &args[var_idx] else {
        return Some(Err("plot: the variable argument must be a name".into()));
    };
    Some(plot_data_inner(args, var_idx, var, labels))
}

fn plot_data_inner(
    args: &[Expr],
    var_idx: usize,
    var: &str,
    labels: PlotLabels,
) -> Result<PlotData, String> {
    let a = bound_f64(&args[var_idx + 1], "plot", "lower")?;
    let b = bound_f64(&args[var_idx + 2], "plot", "upper")?;
    if !(a.is_finite() && b.is_finite() && a < b) {
        return Err("plot: bounds must be finite with a < b".into());
    }
    let mut series: Vec<Series> = Vec::new();
    for target in &args[..var_idx] {
        match target {
            // A scatter overlay: static (x, y) markers, drawn over the same
            // window as the curves but never sampled.
            Expr::Func(name, _) if name == "scatter" => {
                series.push(scatter_series(target)?.0);
            }
            // A matrix flattens into one curve per entry.
            Expr::Matrix(rows) => {
                for expr in rows.iter().flatten() {
                    series.push(sample_series(expr, var, a, b)?);
                }
            }
            other => series.push(sample_series(other, var, a, b)?),
        }
        if series.len() > MAX_SERIES {
            return Err(format!(
                "plot: too many curves ({}, max {})",
                series.len(),
                MAX_SERIES
            ));
        }
    }
    Ok(PlotData {
        var: var.to_string(),
        a,
        b,
        sig: None,
        series,
        title: labels.title,
        xlabel: labels.xlabel,
        ylabel: labels.ylabel,
    })
}

/// Sample one function expression into a curve series over `[a, b]`.
fn sample_series(expr: &Expr, var: &str, a: f64, b: f64) -> Result<Series, String> {
    let curve = f64eval::sample_adaptive(expr, var, a, b, PLOT_SAMPLES, PLOT_SAMPLES_MAX);
    if curve.points.iter().all(|(_, y)| y.is_none()) {
        return Err(format!(
            "plot: '{}' never evaluates to a real number on this interval",
            expr
        ));
    }
    Ok(Series {
        latex: latex::to_latex(expr),
        text: format!("{}", expr),
        undersampled: curve.undersampled,
        fixed: false,
        scatter: false,
        points: curve.points,
    })
}

/// Build a static marker series from a `scatter(xvec, yvec)` value. Also
/// returns the (min, max) of its x values, for deriving a bare-scatter window.
fn scatter_series(tag: &Expr) -> Result<(Series, f64, f64), String> {
    let Expr::Func(_, sargs) = tag else {
        return Err("plot: malformed scatter value".into());
    };
    if sargs.len() != 2 {
        return Err("plot: malformed scatter value".into());
    }
    let xs = mat_entries(&sargs[0])?;
    let ys = mat_entries(&sargs[1])?;
    let mut points = Vec::with_capacity(xs.len());
    let mut xlo = f64::INFINITY;
    let mut xhi = f64::NEG_INFINITY;
    for (xe, ye) in xs.iter().zip(ys.iter()) {
        let x = f64eval::eval_f64(xe, &[])
            .map_err(|e| format!("scatter: x value is not a number ({})", e))?;
        if !x.is_finite() {
            return Err("scatter: x values must be finite".into());
        }
        // A non-finite y is a gap (no marker), mirroring poles in curves.
        let y = f64eval::eval_f64(ye, &[]).ok().filter(|v| v.is_finite());
        xlo = xlo.min(x);
        xhi = xhi.max(x);
        points.push((x, y));
    }
    Ok((
        Series {
            latex: r"\{(x_i,\,y_i)\}".to_string(),
            text: "scatter".to_string(),
            undersampled: false,
            fixed: true,
            scatter: true,
            points,
        },
        xlo,
        xhi,
    ))
}

/// Entries of a vector value (a 1×n or n×1 matrix), in order.
fn mat_entries(e: &Expr) -> Result<Vec<Expr>, String> {
    let Expr::Matrix(rows) = e else {
        return Err("scatter expects vectors (1×n or n×1 matrices)".into());
    };
    if rows.len() == 1 {
        Ok(rows[0].clone())
    } else if rows.iter().all(|r| r.len() == 1) {
        Ok(rows.iter().map(|r| r[0].clone()).collect())
    } else {
        Err("scatter expects vectors (1×n or n×1 matrices)".into())
    }
}

/// A drawing window for a bare scatter: the x-extent with 5% padding, with a
/// unit of breathing room when every point shares one x.
fn pad_window(lo: f64, hi: f64) -> (f64, f64) {
    if lo < hi {
        let m = (hi - lo) * 0.05;
        (lo - m, hi + m)
    } else {
        let p = if lo.abs() > 0.0 { lo.abs() * 0.5 } else { 1.0 };
        (lo - p, lo + p)
    }
}

/// A `plot(d1, ..., dk)` over scatter data only: the points are the data,
/// drawn as markers over a window derived from their x-extent. `None` if `e`
/// isn't one.
fn plot_scatter_data(e: &Expr) -> Option<Result<PlotData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "plotscatter" || args.is_empty() {
        return None;
    }
    let (args, labels) = split_plot_labels(args);
    Some((|| {
        if args.len() > MAX_SERIES {
            return Err(format!(
                "plot: too many series ({}, max {})",
                args.len(),
                MAX_SERIES
            ));
        }
        let mut series = Vec::with_capacity(args.len());
        let mut xlo = f64::INFINITY;
        let mut xhi = f64::NEG_INFINITY;
        for arg in args {
            let (s, lo, hi) = scatter_series(arg)?;
            xlo = xlo.min(lo);
            xhi = xhi.max(hi);
            series.push(s);
        }
        if !(xlo.is_finite() && xhi.is_finite()) {
            return Err("scatter: no finite points to plot".into());
        }
        let (a, b) = pad_window(xlo, xhi);
        Ok(PlotData {
            var: "x".to_string(),
            a,
            b,
            sig: None,
            series,
            title: labels.title,
            xlabel: labels.xlabel,
            ylabel: labels.ylabel,
        })
    })())
}

/// Cap on samples drawn per variable in a scatterplot matrix. Denser data
/// decimates by an even stride — the panels keep their shape, while the payload
/// and the point count stay bounded.
const SPLOM_POINTS_MAX: usize = 3000;
/// Cap on variables: panels form a k×k grid, so this bounds it. Mirrors
/// `MAX_SPLOM_VARS` on the engine side (`build_splom`).
const SPLOM_VARS_MAX: usize = 10;

#[derive(Serialize)]
struct SplomData {
    /// Variable labels — one per row and column of the panel grid.
    labels: Vec<String>,
    /// k columns of decimated samples; `null` is a non-numeric / non-finite gap.
    columns: Vec<Vec<Option<f64>>>,
    /// (min, max) per variable — the shared scale down its column and across
    /// its row, so every panel in a row/column reads on the same axis.
    ranges: Vec<(f64, f64)>,
    /// Row-major k×k Pearson r, for the upper-triangle annotations; `null`
    /// where a variable is constant (correlation undefined).
    cor: Vec<Option<f64>>,
    /// Samples drawn per variable after decimation, and the original count —
    /// the UI notes when it's showing a thinned view.
    shown: usize,
    total: usize,
}

/// A `pairs(...)` value, prepared for drawing as a scatterplot matrix. The
/// `splom` tag carries the n×k data matrix (columns are variables) followed by
/// one symbol per column label. `None` if `e` isn't one.
fn splom_data(e: &Expr) -> Option<Result<SplomData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "splom" || args.len() < 3 {
        return None;
    }
    Some(splom_data_inner(args))
}

fn splom_data_inner(args: &[Expr]) -> Result<SplomData, String> {
    let Expr::Matrix(rows) = &args[0] else {
        return Err("pairs: malformed value".into());
    };
    let labels: Vec<String> = args[1..]
        .iter()
        .map(|s| match s {
            Expr::Symbol(s) => s.clone(),
            other => format!("{}", other),
        })
        .collect();
    let k = labels.len();
    if k < 2 {
        return Err("pairs needs at least 2 variables".into());
    }
    if k > SPLOM_VARS_MAX {
        return Err(format!(
            "pairs: too many variables ({}, max {})",
            k, SPLOM_VARS_MAX
        ));
    }
    if rows.is_empty() || rows[0].len() != k {
        return Err("pairs: data shape does not match its labels".into());
    }
    let total = rows.len();
    // Even-stride decimation: keep the panel's shape while bounding the payload.
    let stride = total.div_ceil(SPLOM_POINTS_MAX).max(1);
    let mut columns: Vec<Vec<Option<f64>>> = vec![Vec::new(); k];
    for row in rows.iter().step_by(stride) {
        for (j, x) in row.iter().enumerate() {
            let v = f64eval::eval_f64(x, &[]).ok().filter(|v| v.is_finite());
            columns[j].push(v);
        }
    }
    let shown = columns.first().map_or(0, Vec::len);
    // Per-variable extent over finite samples, padded so markers don't sit on
    // a panel edge. A variable with no numeric data can't be drawn.
    let mut ranges = Vec::with_capacity(k);
    for (j, col) in columns.iter().enumerate() {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for v in col.iter().flatten() {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if !lo.is_finite() {
            return Err(format!(
                "pairs: variable '{}' has no numeric data",
                labels[j]
            ));
        }
        ranges.push(pad_window(lo, hi));
    }
    // Pearson r per ordered pair (a numeric echo of the engine's exact cormat;
    // here it only labels a panel, so f64 is fine).
    let mut cor = Vec::with_capacity(k * k);
    for ci in &columns {
        for cj in &columns {
            cor.push(pearson(ci, cj));
        }
    }
    Ok(SplomData {
        labels,
        columns,
        ranges,
        cor,
        shown,
        total,
    })
}

/// Pearson correlation of two equal-length sample columns over the indices
/// where both are finite. `None` when fewer than two complete pairs remain, or
/// either column is constant.
fn pearson(a: &[Option<f64>], b: &[Option<f64>]) -> Option<f64> {
    let pairs: Vec<(f64, f64)> = a.iter().zip(b).filter_map(|(x, y)| (*x).zip(*y)).collect();
    let n = pairs.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mx = pairs.iter().map(|(x, _)| x).sum::<f64>() / nf;
    let my = pairs.iter().map(|(_, y)| y).sum::<f64>() / nf;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in &pairs {
        let (dx, dy) = (x - mx, y - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    let denom = (sxx * syy).sqrt();
    (denom != 0.0).then(|| (sxy / denom).clamp(-1.0, 1.0))
}

/// A `spectrogram(s, nfft, hop)` value, prepared for drawing: dB magnitudes
/// of hop-strided, Hann-windowed FFT frames, max-pooled down to a display
/// grid. Computed in f64 from sample midpoints — the plot path is the
/// engine's one deliberately uncertified boundary, and a spectrogram is a
/// picture. Exact per-frame spectra: dsp.stft / dsp.fft on a slice.
#[derive(Serialize, Clone)]
struct SpectrogramData {
    /// dB·10 as integers (0.1 dB resolution keeps the payload compact),
    /// row-major `[frame][bin]`, after max-pooling.
    db10: Vec<i16>,
    /// Grid shape after pooling.
    frames: usize,
    bins: usize,
    /// Sample positions covered (frame centers of the first/last frame).
    t_lo: f64,
    t_hi: f64,
    /// Frequency extent in units of π rad/sample: [0, 1] for real signals,
    /// [-1, 1] (fftshifted) for complex ones.
    f_lo: f64,
    f_hi: f64,
    /// Color range (dB), robust: the 1st percentile to the maximum.
    db_min: f64,
    db_max: f64,
    /// Original frame count, and whether pooling dropped resolution.
    total_frames: usize,
    pooled: bool,
}

/// Display-grid caps: enough for any on-screen panel, small enough that the
/// serialized payload stays light.
const SPEC_MAX_FRAMES: usize = 512;
const SPEC_MAX_BINS: usize = 256;
/// Silence floor, dB.
const SPEC_FLOOR_DB: f64 = -140.0;

fn spectrogram_data(e: &Expr) -> Option<Result<SpectrogramData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "spectrogram" || args.len() != 3 {
        return None;
    }
    Some(spectrogram_data_inner(args))
}

fn spectrogram_data_inner(args: &[Expr]) -> Result<SpectrogramData, String> {
    let Expr::Signal(sig) = &args[0] else {
        return Err("spectrogram: expected a signal".into());
    };
    let nfft = as_usize(&args[1]).ok_or("spectrogram: bad nfft")?;
    let hop = as_usize(&args[2]).ok_or("spectrogram: bad hop")?;
    let (re, im) = surd::signal::midpoints_f64(sig);
    let complex_input = im.is_some();
    let n = re.len();
    if n < nfft {
        return Err("spectrogram: signal shorter than nfft".into());
    }
    let total_frames = (n - nfft) / hop + 1;
    // Periodic Hann.
    let window: Vec<f64> = (0..nfft)
        .map(|k| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * k as f64 / nfft as f64).cos())
        .collect();
    let out_bins_full = if complex_input { nfft } else { nfft / 2 + 1 };
    // dB per frame, at full resolution first (pooled on the fly over frames).
    let frame_pool = total_frames.div_ceil(SPEC_MAX_FRAMES).max(1);
    let bin_pool = out_bins_full.div_ceil(SPEC_MAX_BINS).max(1);
    let frames_out = total_frames.div_ceil(frame_pool);
    let bins_out = out_bins_full.div_ceil(bin_pool);
    let mut db10 = vec![i16::MIN; frames_out * bins_out];
    let mut db_max = f64::NEG_INFINITY;
    let mut buf_re = vec![0.0f64; nfft];
    let mut buf_im = vec![0.0f64; nfft];
    let mut all_db: Vec<f64> = Vec::with_capacity(frames_out * bins_out);
    for f in 0..total_frames {
        let start = f * hop;
        for k in 0..nfft {
            buf_re[k] = re[start + k] * window[k];
            buf_im[k] = im.as_ref().map_or(0.0, |v| v[start + k] * window[k]);
        }
        fft_in_place(&mut buf_re, &mut buf_im);
        let fo = f / frame_pool;
        for b in 0..out_bins_full {
            // Real input: bins 0..=nfft/2 in order. Complex: fftshift so
            // the axis runs −π..π.
            let src = if complex_input {
                (b + nfft / 2) % nfft
            } else {
                b
            };
            let p = buf_re[src] * buf_re[src] + buf_im[src] * buf_im[src];
            let db = if p > 0.0 {
                (10.0 * p.log10()).max(SPEC_FLOOR_DB)
            } else {
                SPEC_FLOOR_DB
            };
            let cell = fo * bins_out + b / bin_pool;
            let v = (db * 10.0).round() as i16;
            if v > db10[cell] {
                db10[cell] = v; // max-pool: peaks survive decimation
            }
            if db > db_max {
                db_max = db;
            }
        }
    }
    for v in &db10 {
        all_db.push(f64::from(*v) / 10.0);
    }
    // Robust lower edge for the color scale: the 1st percentile, so one
    // silent cell doesn't stretch the ramp to the floor.
    let mut sorted = all_db.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite dB"));
    let db_min = sorted[(sorted.len() - 1) / 100].max(db_max - 120.0);
    let (f_lo, f_hi) = if complex_input {
        (-1.0, 1.0)
    } else {
        (0.0, 1.0)
    };
    Ok(SpectrogramData {
        db10,
        frames: frames_out,
        bins: bins_out,
        t_lo: (nfft as f64) / 2.0,
        t_hi: ((total_frames - 1) * hop) as f64 + (nfft as f64) / 2.0,
        f_lo,
        f_hi,
        db_min,
        db_max,
        total_frames,
        pooled: frame_pool > 1 || bin_pool > 1,
    })
}

fn as_usize(e: &Expr) -> Option<usize> {
    let r = surd::expr::numeric_value(e)?;
    if !r.is_integer() {
        return None;
    }
    usize::try_from(r.to_integer()).ok()
}

/// Plain iterative radix-2 FFT over f64 — plot-path only (uncertified by
/// design; the certified transform lives in surd::signal).
fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two());
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (ur, ui) = (re[i + k], im[i + k]);
                let (vr, vi) = (
                    re[i + k + len / 2] * cr - im[i + k + len / 2] * ci,
                    re[i + k + len / 2] * ci + im[i + k + len / 2] * cr,
                );
                re[i + k] = ur + vr;
                im[i + k] = ui + vi;
                re[i + k + len / 2] = ur - vr;
                im[i + k + len / 2] = ui - vi;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// A `plot(s1, ..., sk)` over signals: the samples are the data — no
/// resampling, no window arguments. Long signals decimate to a min/max
/// envelope (extremes survive; the `undersampled` flag says so).
fn plot_signal_data(
    e: &Expr,
    sig_id: u32,
) -> Option<Result<(PlotData, Vec<surd::signal::Signal>), String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if name != "plotsignal" || args.is_empty() {
        return None;
    }
    let (args, labels) = split_plot_labels(args);
    if args.len() > MAX_SERIES {
        return Some(Err(format!(
            "plot: too many signals ({}, max {})",
            args.len(),
            MAX_SERIES
        )));
    }
    let mut series = Vec::new();
    let mut signals: Vec<surd::signal::Signal> = Vec::new();
    let mut maxlen = 0usize;
    for (i, arg) in args.iter().enumerate() {
        let Expr::Signal(s) = arg else {
            return Some(Err(
                "plot: signals and functions cannot mix in one plot".into()
            ));
        };
        maxlen = maxlen.max(s.len());
        // A complex signal plots as two series: real and imaginary parts.
        let parts: Vec<(String, surd::signal::Signal)> =
            if matches!(s.as_ref(), surd::signal::SignalData::Complex { .. }) {
                vec![
                    (
                        format!(r"\Re\,\mathrm{{signal}}_{{{}}}", i + 1),
                        std::rc::Rc::new(surd::signal::re_part(s)),
                    ),
                    (
                        format!(r"\Im\,\mathrm{{signal}}_{{{}}}", i + 1),
                        std::rc::Rc::new(surd::signal::im_part(s)),
                    ),
                ]
            } else {
                vec![(format!(r"\mathrm{{signal}}_{{{}}}", i + 1), s.clone())]
            };
        for (latex, comp) in parts {
            series.push(Series {
                latex,
                text: format!("{}", arg),
                undersampled: surd::signal::plot_decimated(&comp, PLOT_SAMPLES_MAX),
                fixed: true,
                scatter: false,
                points: surd::signal::plot_points(&comp, PLOT_SAMPLES_MAX),
            });
            signals.push(comp);
        }
    }
    if series.len() > MAX_SERIES {
        return Some(Err(format!(
            "plot: too many signal series ({}, max {})",
            series.len(),
            MAX_SERIES
        )));
    }
    Some(Ok((
        PlotData {
            var: "n".to_string(),
            a: 1.0,
            b: maxlen as f64,
            sig: Some(sig_id),
            series,
            title: labels.title,
            xlabel: labels.xlabel,
            ylabel: labels.ylabel,
        },
        signals,
    )))
}

/// A `plot3d(f, x, a, b, y, c, d)` value, sampled on a grid. `None` if `e`
/// isn't one.
fn plot3d_data(e: &Expr) -> Option<Result<Plot3dData, String>> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    let (args, labels) = split_plot_labels(args);
    match name.as_str() {
        "plot3dscatter" => Some(plot3d_scatter_inner(args, labels)),
        "plot3d" if args.len() >= 7 => {
            // The trailing six args are x, a, b, y, c, d; the rest are drawables.
            let base = args.len() - 6;
            let (Expr::Symbol(xvar), Expr::Symbol(yvar)) = (&args[base], &args[base + 3]) else {
                return Some(Err("plot3d: the variable arguments must be names".into()));
            };
            Some(plot3d_surface_inner(args, base, xvar, yvar, labels))
        }
        _ => None,
    }
}

/// `plot3d(d1, ..., dk, x, a, b, y, c, d)` — a surface and/or scatter3d data
/// over an explicit window. At most one drawable is a surface; the rest are
/// scatter3d markers.
fn plot3d_surface_inner(
    args: &[Expr],
    base: usize,
    xvar: &str,
    yvar: &str,
    labels: PlotLabels,
) -> Result<Plot3dData, String> {
    let a = bound_f64(&args[base + 1], "plot3d", "lower x")?;
    let b = bound_f64(&args[base + 2], "plot3d", "upper x")?;
    let c = bound_f64(&args[base + 4], "plot3d", "lower y")?;
    let d = bound_f64(&args[base + 5], "plot3d", "upper y")?;
    if !(a.is_finite() && b.is_finite() && a < b && c.is_finite() && d.is_finite() && c < d) {
        return Err("plot3d: bounds must be finite with a < b and c < d".into());
    }
    let mut surface_expr: Option<&Expr> = None;
    let mut scatter: Vec<(f64, f64, f64)> = Vec::new();
    for drawable in &args[..base] {
        if let Expr::Func(n, _) = drawable {
            if n == "scatter3d" {
                scatter.extend(scatter3d_points(drawable)?.0);
                continue;
            }
        }
        if surface_expr.is_some() {
            return Err("plot3d draws a single surface; pass one f(x, y)".into());
        }
        surface_expr = Some(drawable);
    }
    let (latex, text, nx, ny, undersampled, heights) = match surface_expr {
        Some(expr) => {
            let s = f64eval::sample2d_adaptive(
                expr,
                xvar,
                yvar,
                a,
                b,
                c,
                d,
                SURFACE_GRID,
                SURFACE_GRID_MAX,
            );
            if s.heights.iter().all(|h| h.is_none()) {
                return Err(
                    "plot3d: the expression never evaluates to a real number on this domain".into(),
                );
            }
            (
                latex::to_latex(expr),
                format!("{}", expr),
                s.n,
                s.n,
                s.undersampled,
                s.heights,
            )
        }
        // Scatter only, but framed by an explicit window.
        None => (
            r"\{(x_i,\,y_i,\,z_i)\}".to_string(),
            "scatter3d".to_string(),
            0,
            0,
            false,
            Vec::new(),
        ),
    };
    Ok(Plot3dData {
        latex,
        text,
        xvar: xvar.to_string(),
        a,
        b,
        yvar: yvar.to_string(),
        c,
        d,
        nx,
        ny,
        undersampled,
        heights,
        scatter,
        title: labels.title,
        xlabel: labels.xlabel,
        ylabel: labels.ylabel,
        zlabel: labels.zlabel,
    })
}

/// `plot3d(s1, ..., sk)` over scatter3d data only: the points are the data,
/// boxed by their x/y-extent (z is ranged by the frontend). No surface.
fn plot3d_scatter_inner(args: &[Expr], labels: PlotLabels) -> Result<Plot3dData, String> {
    if args.is_empty() {
        return Err("plot3d: nothing to draw".into());
    }
    let mut scatter = Vec::new();
    let (mut xlo, mut xhi, mut ylo, mut yhi) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for arg in args {
        let (pts, (axlo, axhi, aylo, ayhi)) = scatter3d_points(arg)?;
        xlo = xlo.min(axlo);
        xhi = xhi.max(axhi);
        ylo = ylo.min(aylo);
        yhi = yhi.max(ayhi);
        scatter.extend(pts);
    }
    if !(xlo.is_finite() && ylo.is_finite()) {
        return Err("scatter3d: no finite points to plot".into());
    }
    let (a, b) = pad_window(xlo, xhi);
    let (c, d) = pad_window(ylo, yhi);
    Ok(Plot3dData {
        latex: r"\{(x_i,\,y_i,\,z_i)\}".to_string(),
        text: "scatter3d".to_string(),
        xvar: "x".to_string(),
        a,
        b,
        yvar: "y".to_string(),
        c,
        d,
        nx: 0,
        ny: 0,
        undersampled: false,
        heights: Vec::new(),
        scatter,
        title: labels.title,
        xlabel: labels.xlabel,
        ylabel: labels.ylabel,
        zlabel: labels.zlabel,
    })
}

/// Extract a `scatter3d(x, y, z)` value into `(points, (xlo, xhi, ylo, yhi))`;
/// points with any non-finite coordinate are dropped.
fn scatter3d_points(tag: &Expr) -> Result<(Vec<(f64, f64, f64)>, (f64, f64, f64, f64)), String> {
    let Expr::Func(_, a) = tag else {
        return Err("plot3d: malformed scatter3d value".into());
    };
    if a.len() != 3 {
        return Err("plot3d: malformed scatter3d value".into());
    }
    let xs = mat_entries(&a[0])?;
    let ys = mat_entries(&a[1])?;
    let zs = mat_entries(&a[2])?;
    let mut pts = Vec::with_capacity(xs.len());
    let (mut xlo, mut xhi, mut ylo, mut yhi) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for ((xe, ye), ze) in xs.iter().zip(ys.iter()).zip(zs.iter()) {
        let x =
            f64eval::eval_f64(xe, &[]).map_err(|e| format!("scatter3d: not a number ({})", e))?;
        let y =
            f64eval::eval_f64(ye, &[]).map_err(|e| format!("scatter3d: not a number ({})", e))?;
        let z =
            f64eval::eval_f64(ze, &[]).map_err(|e| format!("scatter3d: not a number ({})", e))?;
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            continue;
        }
        xlo = xlo.min(x);
        xhi = xhi.max(x);
        ylo = ylo.min(y);
        yhi = yhi.max(y);
        pts.push((x, y, z));
    }
    if pts.is_empty() {
        return Err("scatter3d: no finite points".into());
    }
    Ok((pts, (xlo, xhi, ylo, yhi)))
}

#[wasm_bindgen]
pub struct Session {
    interp: surd::Interpreter,
    /// Signal-plot registry: zoom refinement re-decimates from the original
    /// data, which only lives here. Ids are assigned in evaluation order, so
    /// a deterministic transcript replay reproduces them. Bounded: old plots
    /// evict and their zoom degrades gracefully to the shipped envelope.
    signal_plots: std::collections::VecDeque<(u32, Vec<surd::signal::Signal>)>,
    next_plot_id: u32,
}

/// Registry size cap (each entry pins its signals' Rc'd data in memory).
const MAX_SIGNAL_PLOTS: usize = 64;

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
            signal_plots: std::collections::VecDeque::new(),
            next_plot_id: 0,
            interp: surd::Interpreter::new(),
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
            /// Raw-binary export shape: "real" (f32/f64), "complex" (cf32/cf64),
            /// or absent when the value can't export to raw binary.
            #[serde(skip_serializing_if = "Option::is_none")]
            raw: Option<&'static str>,
        }
        let mut entries: Vec<Entry> = self
            .interp
            .workspace()
            .map(|(name, value)| Entry {
                name: name.clone(),
                text: format!("{}", value),
                latex: latex::to_latex(value),
                kind: kind_of(value),
                raw: surd::dataio::raw_export_kind(value),
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        serde_json::to_string(&entries).expect("workspace entries are serializable")
    }

    /// Import a raw data file (surd-data JSON, generic JSON, or CSV —
    /// sniffed) and bind the result to `name` in the global workspace.
    /// Returns an [`EvalResult`]-shaped JSON whose `text` is a short import
    /// summary (the value itself can be enormous), kind `"data"`.
    pub fn import_data(&mut self, payload: &str, name: &str) -> String {
        let result = if !surd::dataio::is_valid_var_name(name) {
            error_result(format!("'{}' is not a valid variable name", name))
        } else {
            match surd::dataio::import(payload) {
                Err(e) => error_result(e),
                Ok(value) => {
                    let descr = format!("{}: {}", name, surd::dataio::describe(&value));
                    self.interp.set_global(name, value);
                    EvalResult {
                        ok: true,
                        kind: "data",
                        text: descr,
                        latex: String::new(),
                        suppressed: false,
                        summary: None,
                        plot: None,
                        plot3d: None,
                        splom: None,
                        spectrogram: None,
                        error: None,
                    }
                }
            }
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }

    /// Import a WAV file as `struct(rate, ch1[, ch2…])` of packed signals,
    /// bound to `name`. Same result shape as [`Session::import_data`].
    pub fn import_wav_data(&mut self, bytes: &[u8], name: &str) -> String {
        self.bind_bulk(name, surd::dataio::import_wav(bytes))
    }

    /// Import a headerless little-endian binary array (`format` is one of
    /// "f64", "f32", "i16") as a packed signal bound to `name`.
    pub fn import_raw_data(&mut self, bytes: &[u8], format: &str, name: &str) -> String {
        self.bind_bulk(name, surd::dataio::import_raw(bytes, format))
    }

    /// Import interleaved I/Q samples (`format` is "cf32" or "cf64") as a
    /// packed complex signal bound to `name`.
    pub fn import_raw_iq_data(&mut self, bytes: &[u8], format: &str, name: &str) -> String {
        self.bind_bulk(name, surd::dataio::import_raw_iq(bytes, format))
    }

    /// Import CSV straight into packed signals (one per column) — the bulk
    /// path for files too large for exact rationals.
    pub fn import_csv_packed_data(&mut self, payload: &str, name: &str) -> String {
        self.bind_bulk(name, surd::dataio::import_csv_packed(payload))
    }

    fn bind_bulk(&mut self, name: &str, value: Result<Expr, String>) -> String {
        let result = if !surd::dataio::is_valid_var_name(name) {
            error_result(format!("'{}' is not a valid variable name", name))
        } else {
            match value {
                Err(e) => error_result(e),
                Ok(value) => {
                    let descr = format!("{}: {}", name, surd::dataio::describe(&value));
                    self.interp.set_global(name, value);
                    EvalResult {
                        ok: true,
                        kind: "data",
                        text: descr,
                        latex: String::new(),
                        suppressed: false,
                        summary: None,
                        plot: None,
                        plot3d: None,
                        splom: None,
                        spectrogram: None,
                        error: None,
                    }
                }
            }
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }

    /// Re-decimate one series of a registered signal plot over the index
    /// window [a, b] — the zoom-refinement path. The full-resolution data
    /// lives in the session's registry; a narrower window means more detail
    /// per pixel. Same response shape as the stateless `resample`:
    /// `{ok, points, undersampled}`, or `{ok: false}` when the plot has
    /// been evicted or the session restarted (the frontend keeps the
    /// shipped envelope in that case).
    pub fn resample_signal(&self, sig: u32, series: usize, a: f64, b: f64) -> String {
        let Some((_, signals)) = self.signal_plots.iter().find(|(id, _)| *id == sig) else {
            return serde_json::json!({"ok": false, "error": "plot no longer registered"})
                .to_string();
        };
        let Some(s) = signals.get(series) else {
            return serde_json::json!({"ok": false, "error": "no such series"}).to_string();
        };
        if !(a.is_finite() && b.is_finite() && a < b) {
            return serde_json::json!({"ok": false, "error": "bounds must be finite with a < b"})
                .to_string();
        }
        // x values are 1-based sample indices; clamp the window to the data.
        let from = (a.floor().max(1.0) as usize).saturating_sub(1);
        let to = (b.ceil().max(1.0) as usize).min(s.len());
        let points = surd::signal::plot_points_range(s, from, to, PLOT_SAMPLES_MAX);
        serde_json::json!({
            "ok": true,
            "points": points,
            "undersampled": surd::signal::range_decimated(from, to, PLOT_SAMPLES_MAX),
        })
        .to_string()
    }

    /// Export the named workspace variables as one `surd-data` JSON file.
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
            surd::dataio::export_variables(&vars)
        };
        match inner() {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }).to_string(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }

    /// Export one workspace variable as raw little-endian binary. `format` is
    /// "f32"/"f64" (real) or "cf32"/"cf64" (interleaved I/Q). Returns
    /// `{ok, data?, error?}` where `data` is the base64 of the bytes (the save
    /// command decodes it back to a file).
    pub fn export_raw(&self, name: &str, format: &str) -> String {
        let inner = || -> Result<String, String> {
            let value = self
                .interp
                .get_global(name)
                .ok_or_else(|| format!("no workspace variable named '{}'", name))?;
            let bytes = surd::dataio::export_raw(value, format)?;
            Ok(base64_encode(&bytes))
        };
        match inner() {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }).to_string(),
            Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
        }
    }

    /// Evaluate one complete statement block; returns JSON ([`EvalResult`]).
    pub fn eval(&mut self, src: &str) -> String {
        // A trailing `;` suppresses the echo (MATLAB/Julia style). The value is
        // still computed below and the workspace still updated — only the
        // rendering is collapsed, so replay and the workspace panel are
        // unaffected.
        let suppressed = surd::lexer::suppresses_output(src);
        let result = match self.interp.eval_line(src) {
            Err(e) => error_result(e),
            Ok(value) => {
                let summary = suppressed.then(|| shape_summary(&value));
                let ok = |kind, plot, plot3d, splom, spectrogram| EvalResult {
                    ok: true,
                    kind,
                    text: format!("{}", value),
                    latex: latex::to_latex(&value),
                    suppressed,
                    summary: summary.clone(),
                    plot,
                    plot3d,
                    splom,
                    spectrogram,
                    error: None,
                };
                // Tagged drawables are unambiguous — handle them before the
                // curve/surface paths.
                if let Some(r) = spectrogram_data(&value) {
                    match r {
                        Ok(sg) => ok("spectrogram", None, None, None, Some(sg)),
                        Err(e) => error_result(e),
                    }
                } else if let Some(r) = splom_data(&value) {
                    match r {
                        Ok(s) => ok("splom", None, None, Some(s), None),
                        Err(e) => error_result(e),
                    }
                } else {
                    let curve = plot_data(&value)
                        .map(|r| r.map(|p| (p, Vec::new())))
                        .or_else(|| plot_scatter_data(&value).map(|r| r.map(|p| (p, Vec::new()))))
                        .or_else(|| plot_signal_data(&value, self.next_plot_id));
                    match (curve, plot3d_data(&value)) {
                        (Some(Err(e)), _) | (_, Some(Err(e))) => error_result(e),
                        (Some(Ok((plot, signals))), _) => {
                            if !signals.is_empty() {
                                self.signal_plots.push_back((self.next_plot_id, signals));
                                self.next_plot_id += 1;
                                if self.signal_plots.len() > MAX_SIGNAL_PLOTS {
                                    self.signal_plots.pop_front();
                                }
                            }
                            ok("plot", Some(plot), None, None, None)
                        }
                        (_, Some(Ok(surface))) => ok("plot3d", None, Some(surface), None, None),
                        (None, None) => ok(kind_of(&value), None, None, None, None),
                    }
                }
            }
        };
        serde_json::to_string(&result).expect("EvalResult is always serializable")
    }
}

/// Resample a previously returned plot expression over a new window (zoom /
/// pan). Stateless: `expr_text` is `Series::text`, which is closed except
/// for the plot variable, so a scratch interpreter suffices. The resolution
/// is adaptive (same policy as the original `plot` evaluation), so the
/// response carries the honesty flag: `{ok, points, undersampled}`.
#[wasm_bindgen]
pub fn resample(expr_text: &str, var: &str, a: f64, b: f64) -> String {
    if !(a.is_finite() && b.is_finite() && a < b) {
        return serde_json::json!({"ok": false, "error": "bounds must be finite with a < b"})
            .to_string();
    }
    let mut interp = surd::Interpreter::new();
    match interp.eval_line(expr_text) {
        Err(e) => serde_json::json!({"ok": false, "error": e}).to_string(),
        Ok(expr) => {
            let curve = f64eval::sample_adaptive(&expr, var, a, b, PLOT_SAMPLES, PLOT_SAMPLES_MAX);
            serde_json::json!({
                "ok": true,
                "points": curve.points,
                "undersampled": curve.undersampled,
            })
            .to_string()
        }
    }
}

/// Resample a surface expression over a new [a, b]×[c, d] domain (zoom /
/// pan). Stateless, like [`resample`]: `expr_text` is `Plot3dData::text`,
/// closed except for the two plot variables. The grid is adaptive (same
/// policy as the original `plot3d` evaluation), so the response carries the
/// resolution it settled on: `{ok, heights, n, undersampled}`.
#[wasm_bindgen]
pub fn resample3d(
    expr_text: &str,
    xvar: &str,
    yvar: &str,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
) -> String {
    if !(a.is_finite() && b.is_finite() && a < b && c.is_finite() && d.is_finite() && c < d) {
        return serde_json::json!({"ok": false, "error": "bounds must be finite with a < b and c < d"})
            .to_string();
    }
    let mut interp = surd::Interpreter::new();
    match interp.eval_line(expr_text) {
        Err(e) => serde_json::json!({"ok": false, "error": e}).to_string(),
        Ok(expr) => {
            let s = f64eval::sample2d_adaptive(
                &expr,
                xvar,
                yvar,
                a,
                b,
                c,
                d,
                SURFACE_GRID,
                SURFACE_GRID_MAX,
            );
            serde_json::json!({
                "ok": true,
                "heights": s.heights,
                "n": s.n,
                "undersampled": s.undersampled,
            })
            .to_string()
        }
    }
}

/// Should the REPL keep reading lines before evaluating (unclosed brackets or
/// `if`/`while`/`function` blocks)?
#[wasm_bindgen]
pub fn is_incomplete(src: &str) -> bool {
    surd::lexer::is_incomplete(src)
}

/// Only whitespace/comments — nothing to evaluate.
#[wasm_bindgen]
pub fn is_blank(src: &str) -> bool {
    surd::lexer::is_blank(src)
}

/// The engine version, so the JS side can report which build of the CAS core
/// it loaded (the app UI surfaces its own version separately via Vite). Tracks
/// the `surd` core crate, not this binding crate, so they can never disagree.
#[wasm_bindgen]
pub fn version() -> String {
    surd::VERSION.to_string()
}

#[derive(Serialize)]
struct Symbols {
    /// Workspace names this cell binds (unconditional top-level `:=` / function
    /// defs). Drives "who reads what I changed" and the healing of names a
    /// later, current cell redefines.
    defs: Vec<String>,
    /// Free workspace names this cell reads — identifiers and call/closure
    /// targets that aren't bound within the cell itself.
    uses: Vec<String>,
}

/// The workspace symbols a cell binds and reads, for the notebook's stale-
/// dependency analysis (which downstream cells an edit invalidates). Reuses
/// the real lexer + parser so the analysis can't drift from evaluation.
///
/// Best-effort and deliberately one-sided: source that doesn't parse (a
/// half-typed draft, a syntax-error cell) yields empty sets, and only
/// *unconditional* top-level bindings count as `defs` — a name bound only
/// inside an `if`/`while` is left out so a later use of it still reads as a
/// workspace dependency rather than being silently healed.
#[wasm_bindgen]
pub fn cell_symbols(src: &str) -> String {
    let mut defs = BTreeSet::new();
    let mut uses = BTreeSet::new();
    if let Ok(node) = surd::lexer::lex(src).and_then(surd::parser::parse) {
        let mut bound = BTreeSet::new();
        symbol_walk(&node, true, false, &mut bound, &mut defs, &mut uses);
    }
    serde_json::to_string(&Symbols {
        defs: defs.into_iter().collect(),
        uses: uses.into_iter().collect(),
    })
    .expect("Symbols is always serializable")
}

/// Walk the parse tree collecting workspace `defs`/`uses`.
///
/// * `top` — at workspace scope (false inside a function body, whose bindings
///   are local and never workspace defs).
/// * `cond` — under an `if`/`while` branch, so a binding here only *may* run;
///   such names are not recorded as `defs` (they must not heal a dependency).
/// * `bound` — names already bound in this straight-line scope, excluded from
///   `uses`. Branch and function bodies get a throwaway clone so their locals
///   don't leak out.
fn symbol_walk(
    node: &Node,
    top: bool,
    cond: bool,
    bound: &mut BTreeSet<String>,
    defs: &mut BTreeSet<String>,
    uses: &mut BTreeSet<String>,
) {
    match node {
        Node::Num(_) | Node::Str(_) => {}
        Node::Ident(name) => {
            if !bound.contains(name) {
                uses.insert(name.clone());
            }
        }
        Node::Call(name, args) => {
            if !bound.contains(name) {
                uses.insert(name.clone());
            }
            for a in args {
                symbol_walk(a, top, cond, bound, defs, uses);
            }
        }
        Node::BinOp(_, a, b) | Node::Equation(a, b) | Node::Formula(a, b) => {
            symbol_walk(a, top, cond, bound, defs, uses);
            symbol_walk(b, top, cond, bound, defs, uses);
        }
        Node::Neg(a) | Node::Not(a) => symbol_walk(a, top, cond, bound, defs, uses),
        Node::Field(base, _) => symbol_walk(base, top, cond, bound, defs, uses),
        Node::FieldCall(base, _, args) => {
            symbol_walk(base, top, cond, bound, defs, uses);
            for a in args {
                symbol_walk(a, top, cond, bound, defs, uses);
            }
        }
        Node::Index(base, idx) => {
            symbol_walk(base, top, cond, bound, defs, uses);
            for arg in idx {
                match arg {
                    IndexArg::Scalar(n) => symbol_walk(n, top, cond, bound, defs, uses),
                    IndexArg::Range { lo, hi, step } => {
                        for n in lo.iter().chain(hi.iter()) {
                            symbol_walk(n, top, cond, bound, defs, uses);
                        }
                        match step {
                            None => {}
                            Some(Step::By(k)) => symbol_walk(k, top, cond, bound, defs, uses),
                            Some(Step::TakeSkip(t, s)) => {
                                symbol_walk(t, top, cond, bound, defs, uses);
                                symbol_walk(s, top, cond, bound, defs, uses);
                            }
                        }
                    }
                }
            }
        }
        Node::Matrix(rows) => {
            for row in rows {
                for c in row {
                    symbol_walk(c, top, cond, bound, defs, uses);
                }
            }
        }
        Node::Assign(name, rhs) => {
            // The RHS sees only bindings established before this statement.
            symbol_walk(rhs, top, cond, bound, defs, uses);
            if top && !cond {
                defs.insert(name.clone());
            }
            bound.insert(name.clone());
        }
        Node::FuncDef(name, params, body) => {
            // Params and body-local assignments are local to the function; a
            // free identifier in the body is still a workspace read (capture).
            let mut inner = bound.clone();
            for p in params {
                inner.insert(p.clone());
            }
            let mut local = BTreeSet::new();
            symbol_walk(body, false, cond, &mut inner, &mut local, uses);
            if top && !cond {
                defs.insert(name.clone());
            }
            bound.insert(name.clone());
        }
        Node::If(c, then, els) => {
            symbol_walk(c, top, cond, bound, defs, uses);
            // Each branch runs conditionally; bindings stay in a throwaway
            // scope so a post-`if` use of them still counts as a read.
            let mut tb = bound.clone();
            symbol_walk(then, top, true, &mut tb, defs, uses);
            if let Some(e) = els {
                let mut eb = bound.clone();
                symbol_walk(e, top, true, &mut eb, defs, uses);
            }
        }
        Node::While(c, body) => {
            symbol_walk(c, top, cond, bound, defs, uses);
            let mut wb = bound.clone();
            symbol_walk(body, top, true, &mut wb, defs, uses);
        }
        Node::For { var, iter, body } => {
            match iter {
                ForIter::Range { lo, step, hi } => {
                    symbol_walk(lo, top, cond, bound, defs, uses);
                    if let Some(s) = step {
                        symbol_walk(s, top, cond, bound, defs, uses);
                    }
                    symbol_walk(hi, top, cond, bound, defs, uses);
                }
                ForIter::Expr(e) => symbol_walk(e, top, cond, bound, defs, uses),
            }
            // The body may run zero times, so — like `while` — its bindings
            // (including the loop variable) stay in a throwaway scope.
            let mut fb = bound.clone();
            fb.insert(var.clone());
            symbol_walk(body, top, true, &mut fb, defs, uses);
        }
        Node::Lambda(params, body) => {
            // Like a function body: params are local, free names are reads.
            let mut inner = bound.clone();
            for p in params {
                inner.insert(p.clone());
            }
            let mut local = BTreeSet::new();
            symbol_walk(body, false, cond, &mut inner, &mut local, uses);
        }
        Node::Block(stmts) => {
            for s in stmts {
                symbol_walk(s, top, cond, bound, defs, uses);
            }
        }
    }
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
        let v: serde_json::Value = serde_json::from_str(&s2.eval("saved.sensor.t")).unwrap();
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
    fn csv_with_categories_and_gaps_models_end_to_end() {
        // The whole loop on a real-world-shaped file: a categorical column
        // and a missing cell, imported, cleaned, and fitted with a formula.
        let mut s = Session::new();
        let csv = "mpg, weight, origin\n18, 35, us\n21, 31, eu\n30, 22, jp\n\
                   25, 26, us\n, 40, us\n28, 24, jp\n17, 36, eu\n";
        let v: serde_json::Value = serde_json::from_str(&s.import_data(csv, "cars")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        let summary = v["text"].as_str().unwrap();
        assert!(
            summary.contains("categorical (3 levels)") && summary.contains("1 missing value"),
            "{}",
            summary
        );

        // The model refuses the gap, dropna clears it, the fit goes through.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("stats.regress(mpg ~ weight + origin, cars)")).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("data.dropna"));
        s.eval("clean := data.dropna(cars)");
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("stats.regress(mpg ~ weight + origin, clean).n")).unwrap();
        assert_eq!(v["text"], "6", "{}", v["error"]);
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
        // smooth curve: the adaptive sampler stays at the base resolution
        assert_eq!(pts.len(), 601);
        assert_eq!(series[0]["undersampled"], false);
        // sin over a symmetric interval: first sample is sin(-π) ≈ 0.
        let y0 = pts[0][1].as_f64().unwrap();
        assert!(y0.abs() < 1e-12);
        // a pole-free curve has no gaps
        assert!(pts.iter().all(|p| !p[1].is_null()));

        // 1/x on [-1, 1] has a gap at the pole
        let v: serde_json::Value = serde_json::from_str(&s.eval("plot(1/x, x, -1, 1)")).unwrap();
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
    fn scatter3d_point_cloud_and_surface_overlay() {
        let mut s = Session::new();
        s.eval("xs := [0, 1, 2, 3]");
        s.eval("ys := [0, 1, 0, 1]");
        s.eval("zs := [1, 2, 3, 4]");

        // Bare 3D scatter: points only, boxed from the data, no surface grid.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(scatter3d(xs, ys, zs))")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["kind"], "plot3d");
        assert_eq!(v["plot3d"]["nx"], 0); // no surface
        assert!(v["plot3d"]["heights"].as_array().unwrap().is_empty());
        let pts = v["plot3d"]["scatter"].as_array().unwrap();
        assert_eq!(pts.len(), 4);
        assert_eq!(pts[3], serde_json::json!([3.0, 1.0, 4.0]));
        // window padded around x ∈ [0, 3]
        assert!(v["plot3d"]["a"].as_f64().unwrap() < 0.0);
        assert!(v["plot3d"]["b"].as_f64().unwrap() > 3.0);

        // Overlay on a surface: both a grid and the markers come back.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x + y, scatter3d(xs, ys, zs), x, 0, 3, y, 0, 1)"))
                .unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        let p = &v["plot3d"];
        assert!(p["nx"].as_u64().unwrap() >= 81); // surface present
        assert_eq!(p["text"], "x + y");
        assert_eq!(p["scatter"].as_array().unwrap().len(), 4);
        assert_eq!(
            p["heights"].as_array().unwrap().len() as u64,
            p["nx"].as_u64().unwrap() * p["ny"].as_u64().unwrap()
        );
    }

    #[test]
    fn scatter3d_rejects_mismatched_vectors() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(scatter3d([1, 2, 3], [1, 2], [1, 2, 3]))"))
                .unwrap();
        assert_eq!(v["ok"], false);
        // Two surfaces is an error — plot3d draws a single mesh.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x, y, x, 0, 1, y, 0, 1)")).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn pairs_builds_a_scatterplot_matrix() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("pairs([1, 2; 2, 4; 3, 6])")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["kind"], "splom");
        let sp = &v["splom"];
        assert_eq!(sp["labels"], serde_json::json!(["x1", "x2"]));
        assert_eq!(sp["shown"], 3);
        assert_eq!(sp["total"], 3);
        let cols = sp["columns"].as_array().unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(cols[1], serde_json::json!([2.0, 4.0, 6.0]));
        assert_eq!(sp["ranges"].as_array().unwrap().len(), 2);
        // Row-major k×k correlations: diagonal exactly 1, the perfectly-linear
        // off-diagonal pair ≈ 1.
        let cor = sp["cor"].as_array().unwrap();
        assert_eq!(cor.len(), 4);
        assert!((cor[0].as_f64().unwrap() - 1.0).abs() < 1e-12);
        assert!((cor[1].as_f64().unwrap() - 1.0).abs() < 1e-12);

        // A variable with no numeric data can't be drawn.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("pairs([a, 1; b, 2; c, 3])")).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn scatter_data_overlays_and_auto_windows() {
        let mut s = Session::new();
        s.eval("xs := [1, 2, 3, 4]");
        s.eval("ys := [2, 4, 6, 8]");

        // Bare scatter draws markers over a window padded around the x-extent.
        let v: serde_json::Value = serde_json::from_str(&s.eval("plot(scatter(xs, ys))")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        assert_eq!(v["kind"], "plot");
        let series = v["plot"]["series"].as_array().unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0]["scatter"], true);
        assert_eq!(series[0]["fixed"], true);
        let pts = series[0]["points"].as_array().unwrap();
        assert_eq!(pts.len(), 4);
        assert_eq!(pts[0][0].as_f64().unwrap(), 1.0);
        assert_eq!(pts[3][1].as_f64().unwrap(), 8.0);
        // 5% padding on a span of 3 → [0.85, 4.15].
        assert!((v["plot"]["a"].as_f64().unwrap() - 0.85).abs() < 1e-9);
        assert!((v["plot"]["b"].as_f64().unwrap() - 4.15).abs() < 1e-9);

        // A scatter overlaid with a fitted curve: two series on shared axes,
        // one marker series and one (sampled) line series.
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(scatter(xs, ys), 2*x, x, 0, 5)")).unwrap();
        assert_eq!(v["ok"], true, "{}", v["error"]);
        let series = v["plot"]["series"].as_array().unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0]["scatter"], true);
        // The curve series carries no scatter flag (omitted) and is sampled.
        assert!(series[1].get("scatter").is_none());
        assert_eq!(series[1]["text"], "2*x");
        assert!(series[1]["points"].as_array().unwrap().len() >= 601);
    }

    #[test]
    fn fit_predict_overlays_on_its_data() {
        let mut s = Session::new();
        s.eval("xs := [0, 1, 2, 3]");
        s.eval("ys := [1, 3, 5, 7]");
        s.eval("m := stats.linfit(xs, ys)");

        // A fit's `predict` is a real function: it plots both applied to the
        // variable, m.predict(x), and bare, m.predict (inlined as its body).
        for curve in ["m.predict(x)", "m.predict"] {
            let v: serde_json::Value =
                serde_json::from_str(&s.eval(&format!("plot(scatter(xs, ys), {curve}, x, 0, 3)")))
                    .unwrap();
            assert_eq!(v["ok"], true, "{}: {}", curve, v["error"]);
            let series = v["plot"]["series"].as_array().unwrap();
            assert_eq!(series.len(), 2, "{curve}");
            assert_eq!(series[0]["scatter"], true, "{curve}");
            // The fitted line y = 1 + 2x passes through (0,1) and (3,7).
            let pts = series[1]["points"].as_array().unwrap();
            assert!((pts[0][1].as_f64().unwrap() - 1.0).abs() < 1e-9, "{curve}");
            let last = pts.last().unwrap();
            assert!((last[1].as_f64().unwrap() - 7.0).abs() < 1e-9, "{curve}");
        }
    }

    #[test]
    fn scatter_rejects_mismatched_vectors() {
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot(scatter([1, 2, 3], [1, 2]))")).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn resample3d_matches_eval_grid() {
        // resample3d over the original domain must reproduce eval's grid —
        // both run the same adaptive sampler, so heights AND resolution agree.
        let mut s = Session::new();
        let v: serde_json::Value =
            serde_json::from_str(&s.eval("plot3d(x*y, x, -1, 1, y, -1, 1)")).unwrap();
        let text = v["plot3d"]["text"].as_str().unwrap();
        let r: serde_json::Value =
            serde_json::from_str(&resample3d(text, "x", "y", -1.0, 1.0, -1.0, 1.0)).unwrap();
        assert_eq!(r["ok"], true, "{}", r["error"]);
        assert_eq!(r["heights"], v["plot3d"]["heights"]);
        assert_eq!(r["n"], v["plot3d"]["nx"]);
        assert_eq!(r["undersampled"], v["plot3d"]["undersampled"]);

        // a zoomed window samples the new domain: corner (2, 2) → 4
        let r: serde_json::Value =
            serde_json::from_str(&resample3d(text, "x", "y", 0.0, 2.0, 0.0, 2.0)).unwrap();
        let n = r["n"].as_u64().unwrap() as usize;
        let h = r["heights"].as_array().unwrap();
        assert_eq!(h.len(), n * n);
        assert!((h[n * n - 1].as_f64().unwrap() - 4.0).abs() < 1e-12);

        // inverted bounds are an error, not a panic
        let r: serde_json::Value =
            serde_json::from_str(&resample3d(text, "x", "y", 1.0, -1.0, 0.0, 1.0)).unwrap();
        assert_eq!(r["ok"], false);
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
        // a smooth quadratic converges at the base grid, uncontested
        assert_eq!(nx, 81);
        assert_eq!(p["undersampled"], false);
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

    fn symbols(src: &str) -> (Vec<String>, Vec<String>) {
        let v: serde_json::Value = serde_json::from_str(&cell_symbols(src)).unwrap();
        let take = |k: &str| {
            v[k].as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        (take("defs"), take("uses"))
    }

    #[test]
    fn symbols_assignment_defs_and_uses() {
        let (defs, uses) = symbols("y := a*x + b");
        assert_eq!(defs, ["y"]);
        assert_eq!(uses, ["a", "b", "x"]);
    }

    #[test]
    fn symbols_function_def_binds_name_captures_globals_not_params() {
        // `f` is defined; body reads workspace `a` but not the param `n`.
        let (defs, uses) = symbols("f(n) := n*a + g(n)");
        assert_eq!(defs, ["f"]);
        assert_eq!(uses, ["a", "g"]); // `n` is a param, excluded
    }

    #[test]
    fn symbols_local_binding_shadows_later_use() {
        // First statement binds t; the call then reads this cell's t, not the
        // workspace's — so t is a def, not a use.
        let (defs, uses) = symbols("t := 5\nf(t)");
        assert_eq!(defs, ["t"]);
        assert_eq!(uses, ["f"]);
    }

    #[test]
    fn symbols_conditional_binding_is_not_a_def() {
        // x is only bound when the branch runs, so it must not count as a def
        // (it can't heal a downstream reader); the post-`if` `x` reads through.
        let (defs, uses) = symbols("if c then x := 1 end\ny := x");
        assert_eq!(defs, ["y"]);
        assert!(uses.contains(&"c".to_string()));
        assert!(uses.contains(&"x".to_string()));
    }

    #[test]
    fn symbols_namespace_base_is_a_use() {
        let (defs, uses) = symbols("sig := dsp.fft(src)");
        assert_eq!(defs, ["sig"]);
        assert_eq!(uses, ["dsp", "src"]);
    }

    #[test]
    fn symbols_unparseable_draft_is_empty() {
        let (defs, uses) = symbols("y := a +");
        assert!(defs.is_empty());
        assert!(uses.is_empty());
    }
}
