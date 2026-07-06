//! LaTeX rendering of [`Expr`], for the web UI (KaTeX).
//!
//! Mirrors the precedence logic of the plain-text `Display` impl in `expr.rs`
//! but renders mathematical structure properly: rationals and negative-power
//! factors become `\frac`, `x^(1/2)` becomes `\sqrt`, π becomes `\pi`.
//! Like `Display`, this is purely cosmetic — it never changes the expression.

use crate::expr::{format_bigfloat, is_one_half, negative_part, Constant, Expr};
use num_bigint::BigInt;
use num_traits::Signed;

// Same precedence levels as the plain-text printer.
const PREC_EQ: u8 = 1;
const PREC_ADD: u8 = 2;
const PREC_MUL: u8 = 3;
const PREC_POW: u8 = 4;
const PREC_ATOM: u8 = 10;

/// Render an expression as LaTeX (no surrounding `$`).
pub fn to_latex(e: &Expr) -> String {
    render(e, 0)
}

fn render(e: &Expr, parent: u8) -> String {
    let (prec, s) = render_inner(e);
    if prec < parent {
        format!(r"\left({}\right)", s)
    } else {
        s
    }
}

fn render_inner(e: &Expr) -> (u8, String) {
    match e {
        Expr::Int(i) => (PREC_ATOM, i.to_string()),
        Expr::Rat(r) => {
            let s = format!(r"\frac{{{}}}{{{}}}", r.numer().abs(), r.denom());
            if r.is_negative() {
                // The sign rides outside the fraction; precedence like a sum
                // so -1/2 in a product gets parenthesized.
                (PREC_ADD, format!("-{}", s))
            } else {
                (PREC_ATOM, s)
            }
        }
        Expr::Float(bf, digits) => (PREC_ATOM, latex_float(&format_bigfloat(bf, *digits))),
        Expr::Const(Constant::Pi) => (PREC_ATOM, r"\pi".to_string()),
        Expr::Const(Constant::E) => (PREC_ATOM, "e".to_string()),
        Expr::Symbol(s) => (PREC_ATOM, latex_symbol(s)),
        Expr::Func(name, args) => {
            let inner = args
                .iter()
                .map(|a| render(a, 0))
                .collect::<Vec<_>>()
                .join(",\\, ");
            let head = match name.as_str() {
                "sin" | "cos" | "tan" | "exp" | "ln" => format!(r"\{}", name),
                "abs" => return (PREC_ATOM, format!(r"\left|{}\right|", inner)),
                _ => format!(r"\mathrm{{{}}}", name),
            };
            (PREC_ATOM, format!(r"{}\left({}\right)", head, inner))
        }
        Expr::Pow(b, ex) => {
            if is_one_half(ex) {
                return (PREC_ATOM, format!(r"\sqrt{{{}}}", render(b, 0)));
            }
            // Exponents never need parens in LaTeX — braces group them.
            let base = render(b, PREC_POW + 1);
            (PREC_POW, format!("{}^{{{}}}", base, render(ex, 0)))
        }
        Expr::Mul(fs) => render_product(fs),
        Expr::Add(ts) => {
            let mut out = String::new();
            for (i, t) in ts.iter().enumerate() {
                if i == 0 {
                    out.push_str(&render(t, PREC_ADD));
                } else if let Some(pos) = negative_part(t) {
                    out.push_str(" - ");
                    out.push_str(&render(&pos, PREC_ADD + 1));
                } else {
                    out.push_str(" + ");
                    out.push_str(&render(t, PREC_ADD + 1));
                }
            }
            (PREC_ADD, out)
        }
        Expr::Matrix(rows) => {
            let body = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| render(c, 0))
                        .collect::<Vec<_>>()
                        .join(" & ")
                })
                .collect::<Vec<_>>()
                .join(r" \\ ");
            // At the default array row spacing, stacked fractions in adjacent
            // rows sit only ~0.015em apart and appear to overlap. When any cell
            // is a fraction, stretch the rows so they breathe. The change is
            // wrapped in a group so it stays local to this matrix.
            let matrix = if body.contains(r"\frac") {
                format!(
                    r"{{\def\arraystretch{{1.4}}\begin{{bmatrix}} {} \end{{bmatrix}}}}",
                    body
                )
            } else {
                format!(r"\begin{{bmatrix}} {} \end{{bmatrix}}", body)
            };
            (PREC_ATOM, matrix)
        }
        Expr::Complex(re, im) => render_complex(re, im),
        Expr::Bool(b) => (
            PREC_ATOM,
            format!(r"\mathrm{{{}}}", if *b { "true" } else { "false" }),
        ),
        Expr::Str(s) => (PREC_ATOM, latex_text(s)),
        Expr::Function { params, .. } => (
            PREC_ATOM,
            format!(r"\mathrm{{function}}({})", params.join(", ")),
        ),
        Expr::Signal(s) => (
            PREC_ATOM,
            format!(r"\mathrm{{signal}}({}\ \mathrm{{samples}})", s.len()),
        ),
        Expr::Equation(l, r) => (
            PREC_EQ,
            format!("{} = {}", render(l, PREC_ADD), render(r, PREC_ADD)),
        ),
        Expr::Formula(l, r) => (
            PREC_EQ,
            format!("{} \\sim {}", render(l, PREC_ADD), render(r, PREC_ADD)),
        ),
        Expr::Struct(fields) => (
            PREC_ATOM,
            format!(
                r"\left\{{ {} \right\}}",
                fields
                    .iter()
                    .map(|(n, v)| format!(r"\mathrm{{{}}} = {}", n, render(v, PREC_ADD)))
                    .collect::<Vec<_>>()
                    .join(",\\; ")
            ),
        ),
    }
}

/// A product renders as a fraction when any factor carries a negative exact
/// exponent: 2/3·x·y⁻¹ → \frac{2x}{3y}. Within numerator/denominator, factors
/// are juxtaposed with thin spaces; an explicit \cdot separates two numbers.
fn render_product(fs: &[Expr]) -> (u8, String) {
    let mut negative = false;
    let mut num: Vec<Expr> = Vec::new();
    let mut den: Vec<Expr> = Vec::new();

    for f in fs {
        match f {
            Expr::Int(i) if *i == BigInt::from(-1) => negative = true,
            Expr::Rat(r) => {
                // A rational coefficient splits across the fraction bar.
                let n = r.numer().abs();
                if r.is_negative() {
                    negative = !negative;
                }
                if n != BigInt::from(1) {
                    num.push(Expr::Int(n));
                }
                den.push(Expr::Int(r.denom().clone()));
            }
            Expr::Int(i) if i.is_negative() => {
                negative = !negative;
                num.push(Expr::Int(-i.clone()));
            }
            Expr::Pow(b, ex) => {
                // Negative exact exponent → denominator with the sign flipped.
                if let Some(flipped) = negated_exponent(ex) {
                    den.push(crate::expr::pow((**b).clone(), flipped));
                } else {
                    num.push(f.clone());
                }
            }
            other => num.push(other.clone()),
        }
    }

    let join = |factors: &[Expr]| -> String {
        if factors.is_empty() {
            return "1".to_string();
        }
        let mut out = String::new();
        let mut prev_numeric = false;
        for (i, f) in factors.iter().enumerate() {
            let numeric = matches!(f, Expr::Int(_) | Expr::Rat(_) | Expr::Float(..));
            if i > 0 {
                // Two adjacent numbers need an explicit dot; otherwise a thin
                // space reads as multiplication.
                out.push_str(if prev_numeric && numeric {
                    r" \cdot "
                } else {
                    r"\, "
                });
            }
            out.push_str(&render(f, PREC_MUL));
            prev_numeric = numeric;
        }
        out
    };

    let body = if den.is_empty() {
        join(&num)
    } else {
        format!(r"\frac{{{}}}{{{}}}", join(&num), join(&den))
    };
    (PREC_MUL, if negative { format!("-{}", body) } else { body })
}

/// If `ex` is a negative exact number, return its negation (else None).
fn negated_exponent(ex: &Expr) -> Option<Expr> {
    match ex {
        Expr::Int(i) if i.is_negative() => Some(Expr::Int(-i.clone())),
        Expr::Rat(r) if r.is_negative() => Some(Expr::Rat(-r.clone())),
        _ => None,
    }
}

fn render_complex(re: &Expr, im: &Expr) -> (u8, String) {
    let re_s = render(re, PREC_ADD);
    let coeff = render(im, PREC_MUL);
    let (neg, mag) = match coeff.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, coeff),
    };
    let imag = if mag == "1" {
        "i".to_string()
    } else {
        format!(r"{}\, i", mag)
    };
    if re_s == "0" {
        return (PREC_MUL, if neg { format!("-{}", imag) } else { imag });
    }
    (
        PREC_ADD,
        if neg {
            format!("{} - {}", re_s, imag)
        } else {
            format!("{} + {}", re_s, imag)
        },
    )
}

/// `1.5e-50` → `1.5\times 10^{-50}`; plain decimals pass through.
fn latex_float(s: &str) -> String {
    match s.split_once('e') {
        Some((mant, exp)) => format!(r"{}\times 10^{{{}}}", mant, exp),
        None => s.to_string(),
    }
}

/// A string literal → `\text{...}` with every LaTeX-special character
/// escaped, so the value echoes literally (including any `$...$` math
/// markers a plot label carries — those are interpreted by the *label*
/// renderer, not by the value display).
fn latex_text(s: &str) -> String {
    let mut body = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => body.push_str(r"\textbackslash "),
            '{' | '}' | '$' | '%' | '&' | '#' | '_' => {
                body.push('\\');
                body.push(c);
            }
            '^' => body.push_str(r"\^{}"),
            '~' => body.push_str(r"\textasciitilde{}"),
            _ => body.push(c),
        }
    }
    format!(r"\text{{{}}}", body)
}

/// A bare symbol → LaTeX. Underscores split into nested subscripts and each
/// segment renders as an atom: `beta_0` → `\beta_{0}`, `x_i_j` → `x_{i_{j}}`,
/// `v_max` → `v_{\mathrm{max}}`. A degenerate name (a leading, trailing, or
/// doubled underscore leaves an empty segment) falls back to one upright token
/// so KaTeX never chokes on a bare `_`.
///
/// Mirrored by `nameToLatex` in the web UI (app/src/engine/nameLatex.ts), which
/// renders variable *names* the same way; keep the two in sync.
fn latex_symbol(s: &str) -> String {
    let parts: Vec<&str> = s.split('_').collect();
    if parts.len() == 1 || parts.iter().any(|p| p.is_empty()) {
        return latex_atom(s);
    }
    // Build the subscript chain from the inside out: a_b_c → a_{b_{c}}.
    let mut acc = latex_atom(parts[parts.len() - 1]);
    for p in parts[..parts.len() - 1].iter().rev() {
        acc = format!("{}_{{{}}}", latex_atom(p), acc);
    }
    acc
}

/// One underscore-free token → its LaTeX atom. Greek-letter names (lower-,
/// upper-case, and the `var*` variants) get their command; a lone character
/// stays as-is; everything else is set upright. Every Greek command is just a
/// backslash before the name, so one list serves all three.
fn latex_atom(s: &str) -> String {
    const GREEK: &[&str] = &[
        // lowercase
        "alpha",
        "beta",
        "gamma",
        "delta",
        "epsilon",
        "zeta",
        "eta",
        "theta",
        "iota",
        "kappa",
        "lambda",
        "mu",
        "nu",
        "xi",
        "pi",
        "rho",
        "sigma",
        "tau",
        "upsilon",
        "phi",
        "chi",
        "psi",
        "omega", // lowercase variants
        "varepsilon",
        "vartheta",
        "varpi",
        "varphi",
        "varrho",
        "varsigma",
        "varkappa",
        // uppercase (only those with a distinct glyph / command)
        "Gamma",
        "Delta",
        "Theta",
        "Lambda",
        "Xi",
        "Pi",
        "Sigma",
        "Upsilon",
        "Phi",
        "Psi",
        "Omega",
    ];
    if GREEK.contains(&s) {
        format!(r"\{}", s)
    } else if s.chars().count() == 1 {
        s.to_string()
    } else {
        // The escape only bites in the degenerate fallback (clean splits never
        // hand an underscore-bearing token here).
        format!(r"\mathrm{{{}}}", s.replace('_', r"\_"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    fn lx(src: &str) -> String {
        to_latex(&Interpreter::new().eval_line(src).unwrap())
    }

    #[test]
    fn atoms_and_fractions() {
        assert_eq!(lx("1/2"), r"\frac{1}{2}");
        assert_eq!(lx("-1/2"), r"-\frac{1}{2}");
        assert_eq!(lx("pi"), r"\pi");
        assert_eq!(lx("sqrt(2)"), r"\sqrt{2}");
        assert_eq!(lx("lambda"), r"\lambda");
    }

    #[test]
    fn greek_names_and_subscripts() {
        // Lower-, upper-case, and variant Greek names all map to their command.
        assert_eq!(lx("beta"), r"\beta");
        assert_eq!(lx("Omega"), r"\Omega");
        assert_eq!(lx("varphi"), r"\varphi");
        // An unknown multi-letter name stays upright; a lone letter is italic.
        assert_eq!(lx("foo"), r"\mathrm{foo}");
        assert_eq!(lx("k"), "k");
        // Underscores become subscripts, the base mapped like any atom.
        assert_eq!(lx("x_1"), r"x_{1}");
        assert_eq!(lx("beta_0"), r"\beta_{0}");
        assert_eq!(lx("v_max"), r"v_{\mathrm{max}}");
        assert_eq!(lx("a_i_j"), r"a_{i_{j}}");
        // Degenerate underscores fall back to one upright, escaped token.
        assert_eq!(lx("_x"), r"\mathrm{\_x}");
    }

    #[test]
    fn products_become_fractions() {
        // x/y is Mul[x, y^-1] internally; LaTeX puts it back over a bar.
        assert_eq!(lx("x/y"), r"\frac{x}{y}");
        assert_eq!(lx("2/3*x"), r"\frac{2\, x}{3}");
    }

    #[test]
    fn sums_subtract_cleanly() {
        // Canonical order puts the numeric term first, like the text printer.
        assert_eq!(lx("x - 2"), "-2 + x");
        assert_eq!(lx("1 - x"), r"1 - x");
    }

    #[test]
    fn functions_and_powers() {
        assert_eq!(lx("sin(x)^2"), r"\sin\left(x\right)^{2}");
        assert_eq!(lx("x^(x+1)"), r"x^{1 + x}");
        assert_eq!(lx("abs(x)"), r"\left|x\right|");
    }

    #[test]
    fn matrices_and_complex() {
        assert_eq!(
            lx("[1,2;3,4]"),
            r"\begin{bmatrix} 1 & 2 \\ 3 & 4 \end{bmatrix}"
        );
        assert_eq!(lx("2 + 3*I"), r"2 + 3\, i");
        assert_eq!(lx("sqrt(-1)"), "i");
    }

    #[test]
    fn matrices_with_fractions_get_row_spacing() {
        // Stacked fractions otherwise nearly touch — the row stretch keeps them
        // legible, and is scoped to the one matrix.
        assert_eq!(
            lx("[1/2;1/3]"),
            r"{\def\arraystretch{1.4}\begin{bmatrix} \frac{1}{2} \\ \frac{1}{3} \end{bmatrix}}"
        );
        // A fraction produced by a product (x/y) also triggers the stretch.
        assert_eq!(
            lx("[x/y,1]"),
            r"{\def\arraystretch{1.4}\begin{bmatrix} \frac{x}{y} & 1 \end{bmatrix}}"
        );
    }

    #[test]
    fn floats_with_exponents() {
        assert_eq!(lx("N(1 + 10^(-50)*I, 30)"), r"1 + 1\times 10^{-50}\, i");
    }
}
