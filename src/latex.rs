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
            (
                PREC_ATOM,
                format!(r"\begin{{bmatrix}} {} \end{{bmatrix}}", body),
            )
        }
        Expr::Complex(re, im) => render_complex(re, im),
        Expr::Bool(b) => (
            PREC_ATOM,
            format!(r"\mathrm{{{}}}", if *b { "true" } else { "false" }),
        ),
        Expr::Function { params, .. } => (
            PREC_ATOM,
            format!(r"\mathrm{{function}}({})", params.join(", ")),
        ),
        Expr::Equation(l, r) => (
            PREC_EQ,
            format!("{} = {}", render(l, PREC_ADD), render(r, PREC_ADD)),
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
                out.push_str(if prev_numeric && numeric { r" \cdot " } else { r"\, " });
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
    (
        PREC_MUL,
        if negative {
            format!("-{}", body)
        } else {
            body
        },
    )
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

/// Multi-character names render upright; Greek-letter names get their symbol.
fn latex_symbol(s: &str) -> String {
    const GREEK: &[&str] = &[
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu", "nu", "xi", "rho", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega",
    ];
    if GREEK.contains(&s) {
        format!(r"\{}", s)
    } else if s.chars().count() == 1 {
        s.to_string()
    } else {
        format!(r"\mathrm{{{}}}", s)
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
    fn floats_with_exponents() {
        assert_eq!(lx("N(1 + 10^(-50)*I, 30)"), r"1 + 1\times 10^{-50}\, i");
    }
}
