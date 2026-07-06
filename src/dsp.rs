//! The `dsp` built-in namespace: exact digital signal processing.
//!
//! Everything follows the engine's exactness contract. DFT twiddle factors
//! are built through the smart constructors, so for transform sizes whose
//! angles have surd forms (1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24 — see
//! the table in [`crate::expr`]) the result folds all the way to exact
//! surds: the DFT of a rational vector over ℚ(i, √2, √3, √5, …), with no
//! rounding anywhere. Other sizes stay exact but symbolic — entries hold
//! `cos`/`sin` of rational multiples of π — and `N(...)` evaluates them to
//! any precision on demand.

use crate::expr::*;
use crate::remez;
use crate::signal;
use num_bigint::BigInt;
use num_traits::{One, ToPrimitive};
use std::rc::Rc;

/// Functions in the namespace, in the order the docs list them.
pub const FUNCTIONS: &[&str] = &[
    "conv",
    "circconv",
    "dft",
    "dftmatrix",
    "idft",
    "freqz",
    "firlow",
    "remez",
    "hann",
    "hamming",
    "blackman",
    "window",
    "quantize",
    "stft",
    "butter",
    "tf",
    "poles",
    "zeros",
    "stable",
    "filter",
    "impz",
    "fft",
    "ifft",
    "pad",
    "peak",
    "rms",
];

/// Cap on pairwise symbolic products per call (a DFT is n², a convolution
/// m·n) — same philosophy as the evaluator's loop guard: arbitrary cost is
/// fine, an effective hang is not.
const MAX_PAIRWISE_OPS: usize = 4_000_000;

pub fn call(name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    match name {
        "dft" => transform("dsp.dft", args, false),
        "idft" => transform("dsp.idft", args, true),
        "dftmatrix" => {
            arity("dsp.dftmatrix", &args, 1)?;
            dft_matrix(&args[0])
        }
        "conv" => {
            // Packed signals get the certified bulk path; exact vectors keep
            // the symbolic one. No implicit crossing between the two.
            if args.iter().any(|a| matches!(a, Expr::Signal(_))) {
                arity("dsp.conv", &args, 2)?;
                return match (&args[0], &args[1]) {
                    (Expr::Signal(a), Expr::Signal(b)) => {
                        Ok(Expr::Signal(Rc::new(signal::conv(a, b)?)))
                    }
                    _ => Err("dsp.conv on bulk data needs both sides packed — wrap the \
                              exact one in signal(...)"
                        .into()),
                };
            }
            convolution("dsp.conv", args, false)
        }
        "circconv" => convolution("dsp.circconv", args, true),
        "fft" => bulk_transform("dsp.fft", args, false),
        "ifft" => bulk_transform("dsp.ifft", args, true),
        "pad" => {
            arity("dsp.pad", &args, 2)?;
            let Expr::Signal(s) = &args[0] else {
                return Err(
                    "dsp.pad expects a signal (exact vectors concatenate with vcat)".into(),
                );
            };
            let n = as_size("dsp.pad", &args[1])?;
            Ok(Expr::Signal(Rc::new(signal::pad(s, n)?)))
        }
        "peak" => {
            arity("dsp.peak", &args, 1)?;
            let Expr::Signal(s) = &args[0] else {
                return Err("dsp.peak expects a signal".into());
            };
            signal::peak(s)
        }
        "rms" => {
            arity("dsp.rms", &args, 1)?;
            let Expr::Signal(s) = &args[0] else {
                return Err("dsp.rms expects a signal".into());
            };
            signal::rms(s)
        }
        "freqz" => freqz(args),
        "firlow" => firlow(args),
        "remez" => remez_design(args),
        "window" => {
            arity("dsp.window", &args, 2)?;
            let Expr::Symbol(name) = &args[0] else {
                return Err(
                    "dsp.window expects a window name first: dsp.window(hann, n) — \
                     hann, hamming, or blackman"
                        .into(),
                );
            };
            let n = as_size("dsp.window", &args[1])?;
            Ok(Expr::Signal(Rc::new(signal::window(name, n)?)))
        }
        "hann" => window("dsp.hann", args, (1, 2), (1, 2), (0, 1)),
        "hamming" => window("dsp.hamming", args, (27, 50), (23, 50), (0, 1)),
        "blackman" => window("dsp.blackman", args, (21, 50), (1, 2), (2, 25)),
        "quantize" => quantize(args),
        "stft" => stft(args),
        "tf" => crate::iir::tf(args),
        "poles" => crate::iir::poles_or_zeros("dsp.poles", args, true),
        "zeros" => crate::iir::poles_or_zeros("dsp.zeros", args, false),
        "butter" => crate::iir::butter(args),
        "stable" => crate::iir::stable(args),
        "filter" => crate::iir::filter(args),
        "impz" => crate::iir::impz(args),
        _ => Err(format!(
            "unknown function 'dsp.{}' (available: dsp.{})",
            name,
            FUNCTIONS.join(", dsp.")
        )),
    }
}

/// The root of unity e^(∓2πi·k/n) = cos(2πk/n) ∓ i·sin(2πk/n), built through
/// the smart constructors so it folds to an exact surd whenever the angle has
/// one; otherwise it stays exact and symbolic.
fn root_of_unity(k: usize, n: usize, inverse: bool) -> Expr {
    let r = BigRational::new(BigInt::from(2 * (k % n)), BigInt::from(n));
    let angle = mul(vec![rat_to_expr(r), Expr::Const(Constant::Pi)]);
    let re = func("cos", vec![angle.clone()]);
    let im = func("sin", vec![angle]);
    let sign = if inverse { 1 } else { -1 };
    complex(re, mul(vec![int(sign), im]))
}

/// `dsp.dft(v)` / `dsp.idft(v)`: X[k] = Σⱼ v[j]·e^(−2πi·kj/n), and the
/// inverse with the +i kernel and a 1/n factor. Direct O(n²) summation —
/// exact arithmetic is the point here, not asymptotics.
///
/// Each entry is `expand`ed: the canonical form deliberately doesn't
/// distribute products over sums, but a transform's terms (input × surd
/// twiddle) only cancel across the sum once distributed — without this,
/// `idft(dft(v))` returns v in a structurally different (unrecognizable)
/// form at sizes like 8 instead of folding back to the input.
fn transform(name: &str, args: Vec<Expr>, inverse: bool) -> Result<Expr, String> {
    arity(name, &args, 1)?;
    let (x, shape) = as_vector(name, &args[0])?;
    let n = x.len();
    check_ops(name, n.saturating_mul(n))?;
    let scale = BigRational::new(BigInt::one(), BigInt::from(n));
    // k·j only matters mod n, so there are just n distinct twiddles; build
    // each once (surd-table lookups and angle canonicalization are the
    // expensive part) and clone thereafter — output-identical to rebuilding.
    let tw: Vec<Expr> = (0..n).map(|r| root_of_unity(r, n, inverse)).collect();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let terms = x
            .iter()
            .enumerate()
            .map(|(j, xj)| mul(vec![xj.clone(), tw[(k * j) % n].clone()]))
            .collect();
        let mut value = expand(&add(terms));
        if inverse {
            value = expand(&mul(vec![rat_to_expr(scale.clone()), value]));
        }
        out.push(value);
    }
    Ok(from_vector(out, shape))
}

/// `dsp.dftmatrix(n)`: the n×n Fourier matrix F[j][k] = e^(−2πi·jk/n)
/// (unnormalized, so `dsp.dft(v)` equals `dsp.dftmatrix(n) * v`).
fn dft_matrix(arg: &Expr) -> Result<Expr, String> {
    let n = as_size("dsp.dftmatrix", arg)?;
    check_ops("dsp.dftmatrix", n.saturating_mul(n))?;
    // Same residue trick as `transform`: n distinct twiddles, not n².
    let tw: Vec<Expr> = (0..n).map(|r| root_of_unity(r, n, false)).collect();
    let rows = (0..n)
        .map(|j| (0..n).map(|k| tw[(j * k) % n].clone()).collect())
        .collect();
    Ok(Expr::Matrix(rows))
}

/// `dsp.conv(a, b)`: linear convolution, length m+n−1. `dsp.circconv(a, b)`:
/// circular convolution of two equal-length vectors. Orientation follows the
/// first argument.
fn convolution(name: &str, args: Vec<Expr>, circular: bool) -> Result<Expr, String> {
    arity(name, &args, 2)?;
    let (a, shape) = as_vector(name, &args[0])?;
    let (b, _) = as_vector(name, &args[1])?;
    let (m, n) = (a.len(), b.len());
    check_ops(name, m.saturating_mul(n))?;
    if circular {
        if m != n {
            return Err(format!(
                "{} expects two vectors of the same length, got {} and {}",
                name, m, n
            ));
        }
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let terms = (0..n)
                .map(|j| mul(vec![a[j].clone(), b[(i + n - j) % n].clone()]))
                .collect();
            out.push(expand(&add(terms)));
        }
        return Ok(from_vector(out, shape));
    }
    let mut term_lists: Vec<Vec<Expr>> = vec![Vec::new(); m + n - 1];
    for (j, aj) in a.iter().enumerate() {
        for (k, bk) in b.iter().enumerate() {
            term_lists[j + k].push(mul(vec![aj.clone(), bk.clone()]));
        }
    }
    Ok(from_vector(
        term_lists.into_iter().map(|ts| expand(&add(ts))).collect(),
        shape,
    ))
}

/// `dsp.freqz(h, w)`: the frequency response H(ω) = Σₖ h[k]·e^(−iωk) of FIR
/// taps `h`, evaluated at each ω in the vector `w` (radians/sample). Exact
/// whenever k·ω lands on the trig surd table — a grid like
/// `linspace(0, pi, 9)` does — and exact-symbolic elsewhere, with `N(...)`
/// finishing the job.
fn freqz(args: Vec<Expr>) -> Result<Expr, String> {
    // IIR forms: freqz(f, w) with a filter struct, freqz(b, a, w) rational.
    if args.len() == 2 && matches!(&args[0], Expr::Struct(_)) {
        let sections = crate::iir::sos_sections("dsp.freqz", &args[0])?;
        let (w, shape) = as_vector("dsp.freqz", &args[1])?;
        check_ops("dsp.freqz", w.len().saturating_mul(6 * sections.len()))?;
        return Ok(from_vector(
            crate::iir::freqz_rational(&sections, &w)?,
            shape,
        ));
    }
    if args.len() == 3 {
        let (b, _) = as_vector("dsp.freqz", &args[0])?;
        let (a, _) = as_vector("dsp.freqz", &args[1])?;
        let (w, shape) = as_vector("dsp.freqz", &args[2])?;
        check_ops("dsp.freqz", w.len().saturating_mul(b.len() + a.len()))?;
        return Ok(from_vector(
            crate::iir::freqz_rational(&[(b, a)], &w)?,
            shape,
        ));
    }
    arity("dsp.freqz", &args, 2)?;
    let (h, _) = as_vector("dsp.freqz", &args[0])?;
    let (w, shape) = as_vector("dsp.freqz", &args[1])?;
    check_ops("dsp.freqz", h.len().saturating_mul(w.len()))?;
    let mut out = Vec::with_capacity(w.len());
    for wi in &w {
        let terms = h
            .iter()
            .enumerate()
            .map(|(k, hk)| {
                let arg = mul(vec![int(k as i64), wi.clone()]);
                let kernel = complex(
                    func("cos", vec![arg.clone()]),
                    mul(vec![int(-1), func("sin", vec![arg])]),
                );
                mul(vec![hk.clone(), kernel])
            })
            .collect();
        out.push(expand(&add(terms)));
    }
    Ok(from_vector(out, shape))
}

/// `dsp.firlow(n, wc)`: n-tap windowed-sinc lowpass prototype with cutoff
/// `wc` (radians/sample, 0 < wc < π): h[k] = sin(wc·(k−M))/(π·(k−M)) with
/// M = (n−1)/2, and wc/π at the center. Rectangular window — taper it
/// elementwise: `dsp.firlow(n, wc) .* dsp.hann(n)`.
fn firlow(args: Vec<Expr>) -> Result<Expr, String> {
    arity("dsp.firlow", &args, 2)?;
    let n = as_size("dsp.firlow", &args[0])?;
    check_ops("dsp.firlow", n)?;
    let wc = args[1].clone();
    let rat =
        |num: i64, den: i64| rat_to_expr(BigRational::new(BigInt::from(num), BigInt::from(den)));
    let mut row = Vec::with_capacity(n);
    for k in 0..n {
        // d = k − (n−1)/2, kept exact (a half-integer for even n).
        let d = (
            rat(2 * k as i64 - (n as i64 - 1), 2),
            2 * k as i64 != n as i64 - 1,
        );
        row.push(if d.1 {
            // sin(d·wc) / (d·π)
            mul(vec![
                func("sin", vec![mul(vec![d.0.clone(), wc.clone()])]),
                pow(mul(vec![d.0, Expr::Const(Constant::Pi)]), int(-1)),
            ])
        } else {
            // The center tap: lim sin(d·wc)/(d·π) = wc/π.
            mul(vec![wc.clone(), pow(Expr::Const(Constant::Pi), int(-1))])
        });
    }
    Ok(Expr::Matrix(vec![row]))
}

/// The cosine-sum windows, length n (symmetric):
/// w[k] = a0 − a1·cos(2πk/(n−1)) + a2·cos(4πk/(n−1)).
fn window(
    name: &str,
    args: Vec<Expr>,
    a0: (i64, i64),
    a1: (i64, i64),
    a2: (i64, i64),
) -> Result<Expr, String> {
    arity(name, &args, 1)?;
    let n = as_size(name, &args[0])?;
    check_ops(name, n)?;
    let rat = |(p, q): (i64, i64)| rat_to_expr(BigRational::new(BigInt::from(p), BigInt::from(q)));
    if n == 1 {
        return Ok(Expr::Matrix(vec![vec![int(1)]]));
    }
    let angle = |k: usize, mult: i64| {
        mul(vec![
            rat_to_expr(BigRational::new(
                BigInt::from(mult * k as i64),
                BigInt::from(n as i64 - 1),
            )),
            Expr::Const(Constant::Pi),
        ])
    };
    // The window is symmetric: w[k] = w[n−1−k] because cos(2π−θ) = cos(θ)
    // exactly, and trig angle normalization canonicalizes both sides to the
    // *same* expression — so mirroring is output-identical to recomputing,
    // at half the symbolic-construction cost.
    let half = n.div_ceil(2);
    let mut row: Vec<Expr> = (0..half)
        .map(|k| {
            add(vec![
                rat(a0),
                mul(vec![int(-1), rat(a1), func("cos", vec![angle(k, 2)])]),
                mul(vec![rat(a2), func("cos", vec![angle(k, 4)])]),
            ])
        })
        .collect();
    for k in half..n {
        row.push(row[n - 1 - k].clone());
    }
    Ok(Expr::Matrix(vec![row]))
}

/// `dsp.quantize(v, bits)`: snap every entry to the fixed-point grid with
/// `bits` fractional bits — round(x·2^bits)/2^bits, ties away from zero.
/// The result is exact rationals, so `h - dsp.quantize(h, 15)` is the exact
/// quantization error (feed it to `dsp.freqz`). Range/overflow handling is
/// the implementer's concern: this never clamps.
fn quantize(args: Vec<Expr>) -> Result<Expr, String> {
    arity("dsp.quantize", &args, 2)?;
    let Expr::Matrix(rows) = &args[0] else {
        return Err("dsp.quantize expects a vector or matrix".into());
    };
    let bits = as_size("dsp.quantize", &args[1])?;
    if bits > 256 {
        return Err("dsp.quantize expects at most 256 fractional bits".into());
    }
    let scale = BigInt::from(1) << bits;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut new_row = Vec::with_capacity(row.len());
        for cell in row {
            let r = match cell {
                Expr::Float(bf, _) => float_to_rational(bf),
                other => numeric_value(other),
            }
            .ok_or_else(|| {
                format!(
                    "dsp.quantize needs numeric entries, got '{}' (wrap the design in N(...))",
                    cell
                )
            })?;
            let scaled = r * BigRational::from_integer(scale.clone());
            new_row.push(rat_to_expr(BigRational::new(
                round_half_away(&scaled),
                scale.clone(),
            )));
        }
        out.push(new_row);
    }
    Ok(Expr::Matrix(out))
}

/// Round to the nearest integer, ties away from zero (the common fixed-point
/// convention): round(5/2) = 3, round(−5/2) = −3.
fn round_half_away(r: &BigRational) -> BigInt {
    use num_traits::Signed;
    let two = BigInt::from(2);
    // q > 0 after normalization; floor((2|p| + q) / 2q) rounds |r| half-up.
    let mag = (r.numer().abs() * &two + r.denom()) / (r.denom() * &two);
    if r.numer().is_negative() {
        -mag
    } else {
        mag
    }
}

/// `dsp.fft(s)` / `dsp.ifft(f)`: the certified bulk transform. A real or
/// complex signal goes in and a single complex signal comes out (use `re`/`im`
/// to split it, `abs` for the magnitude spectrum). For back-compat the inverse
/// also accepts a legacy `struct(re = signal, im = signal)`. Exact spectra come
/// from `dsp.dft`.
fn bulk_transform(name: &str, args: Vec<Expr>, inverse: bool) -> Result<Expr, String> {
    arity(name, &args, 1)?;
    let input = match &args[0] {
        Expr::Signal(s) => (**s).clone(),
        // Legacy struct(re, im) input — fold it back into a complex signal.
        Expr::Struct(fields) => {
            let get = |n: &str| fields.iter().find(|(k, _)| k == n).map(|(_, v)| v);
            match (get("re"), get("im")) {
                (Some(Expr::Signal(r)), Some(Expr::Signal(i))) => {
                    signal::complex((**r).clone(), (**i).clone())?
                }
                _ => {
                    return Err(format!(
                        "{} expects a signal or struct(re = signal, im = signal)",
                        name
                    ))
                }
            }
        }
        _ => {
            return Err(format!(
                "{} expects a signal (pack with signal(...)); exact spectra come from dsp.dft",
                name
            ))
        }
    };
    Ok(Expr::Signal(Rc::new(signal::fft_signal(&input, inverse)?)))
}

/// `dsp.remez(n, edges, desired[, weights][, antisymmetric])`: exact
/// Parks–McClellan, all four linear-phase types. Band edges in
/// radians/sample, ascending pairs within [0, π]; one desired value (and
/// optional weight) per band. The type follows the length parity and the
/// optional trailing `antisymmetric` flag: odd+symmetric = I,
/// even+symmetric = II, odd+antisymmetric = III (Hilbert transformers),
/// even+antisymmetric = IV. Returns struct(taps, ripple, iterations,
/// fir_type): exact rational taps and the exact minimax ripple on the
/// design grid.
fn remez_design(mut args: Vec<Expr>) -> Result<Expr, String> {
    // Optional trailing symmetry flag.
    let mut antisymmetric = false;
    if let Some(Expr::Symbol(sym)) = args.last() {
        match sym.as_str() {
            "antisymmetric" | "hilbert" => {
                antisymmetric = true;
                args.pop();
            }
            "symmetric" => {
                args.pop();
            }
            _ => {}
        }
    }
    if !(3..=4).contains(&args.len()) {
        return Err(format!(
            "dsp.remez expects remez(n, edges, desired[, weights][, antisymmetric]),              got {} argument(s)",
            args.len()
        ));
    }
    let n = as_size("dsp.remez", &args[0])?;
    let (edges, _) = as_vector("dsp.remez", &args[1])?;
    let (desired, _) = as_vector("dsp.remez", &args[2])?;
    let weights = match args.get(3) {
        Some(wv) => as_vector("dsp.remez", wv)?.0,
        None => vec![int(1); desired.len()],
    };
    let ty = remez::Ty::classify(n, antisymmetric);
    let d = remez::design_typed(n, antisymmetric, &edges, &desired, &weights)?;
    let taps = Expr::Matrix(vec![d.taps.into_iter().map(rat_to_expr).collect()]);
    structure(vec![
        ("taps".to_string(), taps),
        ("ripple".to_string(), rat_to_expr(d.ripple)),
        ("iterations".to_string(), int(d.iterations as i64)),
        ("fir_type".to_string(), int(ty.number())),
    ])
}

// -- argument plumbing -------------------------------------------------------

/// A vector argument is a 1×n or n×1 matrix; results keep its orientation.
pub(crate) enum Shape {
    Row,
    Col,
}

pub(crate) fn as_vector(name: &str, e: &Expr) -> Result<(Vec<Expr>, Shape), String> {
    let Expr::Matrix(rows) = e else {
        return Err(format!("{} expects a vector (a 1×n or n×1 matrix)", name));
    };
    if rows.len() == 1 {
        Ok((rows[0].clone(), Shape::Row))
    } else if rows.iter().all(|r| r.len() == 1) {
        Ok((rows.iter().map(|r| r[0].clone()).collect(), Shape::Col))
    } else {
        Err(format!(
            "{} expects a vector (a 1×n or n×1 matrix), got a {}×{} matrix",
            name,
            rows.len(),
            rows[0].len()
        ))
    }
}

pub(crate) fn from_vector(entries: Vec<Expr>, shape: Shape) -> Expr {
    match shape {
        Shape::Row => Expr::Matrix(vec![entries]),
        Shape::Col => Expr::Matrix(entries.into_iter().map(|e| vec![e]).collect()),
    }
}

pub(crate) fn as_size(name: &str, e: &Expr) -> Result<usize, String> {
    numeric_value(e)
        .filter(|r| r.is_integer())
        .and_then(|r| r.to_integer().to_usize())
        .filter(|&n| n >= 1)
        .ok_or_else(|| format!("{} expects a positive integer size", name))
}

fn check_ops(name: &str, ops: usize) -> Result<(), String> {
    if ops > MAX_PAIRWISE_OPS {
        Err(format!(
            "{}: input is too large for exact computation ({} pairwise products, cap {})",
            name, ops, MAX_PAIRWISE_OPS
        ))
    } else {
        Ok(())
    }
}

fn arity(name: &str, args: &[Expr], n: usize) -> Result<(), String> {
    if args.len() == n {
        Ok(())
    } else {
        Err(format!(
            "{} expects {} argument(s), got {}",
            name,
            n,
            args.len()
        ))
    }
}

/// `dsp.stft(v, nfft, hop)`: the exact short-time Fourier transform of an
/// exact vector — one row per frame: DFT(w .* frame), periodic Hann window
/// w[k] = 1/2 − 1/2·cos(2πk/nfft), frames starting at 0, hop apart, only
/// full frames. Exact (surds on the twiddle table, symbolic beyond), so a
/// frame's spectrum here is the certified reference for the spectrogram's
/// display path. Bulk data belongs to `spectrogram(...)`.
pub fn stft(args: Vec<Expr>) -> Result<Expr, String> {
    arity("dsp.stft", &args, 3)?;
    let (x, _) = as_vector("dsp.stft", &args[0])?;
    let nfft = as_size("dsp.stft", &args[1])?;
    let hop = as_size("dsp.stft", &args[2])?;
    if x.len() < nfft {
        return Err(format!(
            "dsp.stft: the vector has {} entries but nfft is {}",
            x.len(),
            nfft
        ));
    }
    let frames = (x.len() - nfft) / hop + 1;
    check_ops("dsp.stft", frames.saturating_mul(nfft.saturating_mul(nfft)))?;
    // Periodic Hann, exact: cos of rational multiples of π.
    let window: Vec<Expr> = (0..nfft)
        .map(|k| {
            let angle = mul(vec![
                rat_to_expr(BigRational::new(
                    BigInt::from(2 * k as i64),
                    BigInt::from(nfft as i64),
                )),
                Expr::Const(Constant::Pi),
            ]);
            add(vec![
                rat_to_expr(BigRational::new(BigInt::from(1), BigInt::from(2))),
                mul(vec![
                    rat_to_expr(BigRational::new(BigInt::from(-1), BigInt::from(2))),
                    func("cos", vec![angle]),
                ]),
            ])
        })
        .collect();
    let mut rows = Vec::with_capacity(frames);
    for f in 0..frames {
        let start = f * hop;
        let frame: Vec<Expr> = (0..nfft)
            .map(|k| mul(vec![window[k].clone(), x[start + k].clone()]))
            .collect();
        let mut row = Vec::with_capacity(nfft);
        for bin in 0..nfft {
            let terms = frame
                .iter()
                .enumerate()
                .map(|(j, xj)| mul(vec![xj.clone(), root_of_unity(bin * j, nfft, false)]))
                .collect();
            row.push(expand(&add(terms)));
        }
        rows.push(row);
    }
    structure(vec![
        ("frames".to_string(), Expr::Matrix(rows)),
        ("nfft".to_string(), int(nfft as i64)),
        ("hop".to_string(), int(hop as i64)),
    ])
}
