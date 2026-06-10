//! Fast approximate evaluation: `Expr` → `f64`. The *pixel* path.
//!
//! Plotting samples an expression at hundreds of points, where arbitrary
//! precision would be wasted — pixels are already approximate. Anything
//! correctness-critical still goes through the exact engine and `N(...)`;
//! nothing here feeds back into symbolic results.

use crate::expr::{float_to_rational, Constant, Expr};
use num_traits::ToPrimitive;

/// Evaluate `e` to an `f64`, with at most one free variable bound to a value
/// (`var`). Errors on anything symbolic, complex, or non-scalar; IEEE
/// non-finite results (poles, domain edges) are returned as-is and dealt with
/// by the caller.
pub fn eval_f64(e: &Expr, var: Option<(&str, f64)>) -> Result<f64, String> {
    match e {
        Expr::Int(i) => i
            .to_f64()
            .ok_or_else(|| "integer does not fit in f64".to_string()),
        Expr::Rat(r) => r
            .to_f64()
            .ok_or_else(|| "rational does not fit in f64".to_string()),
        Expr::Float(bf, _) => float_to_rational(bf)
            .and_then(|r| r.to_f64())
            .ok_or_else(|| "float is not finite".to_string()),
        Expr::Const(Constant::Pi) => Ok(std::f64::consts::PI),
        Expr::Const(Constant::E) => Ok(std::f64::consts::E),
        Expr::Symbol(s) => match var {
            Some((name, x)) if name == s => Ok(x),
            _ => Err(format!("cannot evaluate free symbol '{}'", s)),
        },
        Expr::Add(ts) => {
            let mut acc = 0.0;
            for t in ts {
                acc += eval_f64(t, var)?;
            }
            Ok(acc)
        }
        Expr::Mul(fs) => {
            let mut acc = 1.0;
            for f in fs {
                acc *= eval_f64(f, var)?;
            }
            Ok(acc)
        }
        Expr::Pow(b, ex) => {
            let base = eval_f64(b, var)?;
            let exp = eval_f64(ex, var)?;
            // Integer exponents use powi so negative bases work ((-2)^3 = -8;
            // powf would give NaN).
            if exp.fract() == 0.0 && exp.abs() <= i32::MAX as f64 {
                Ok(base.powi(exp as i32))
            } else {
                Ok(base.powf(exp))
            }
        }
        Expr::Func(name, args) if args.len() == 1 => {
            let x = eval_f64(&args[0], var)?;
            match name.as_str() {
                "sin" => Ok(x.sin()),
                "cos" => Ok(x.cos()),
                "tan" => Ok(x.tan()),
                "exp" => Ok(x.exp()),
                "ln" => Ok(x.ln()),
                "abs" => Ok(x.abs()),
                _ => Err(format!("cannot evaluate '{}' numerically", name)),
            }
        }
        Expr::Func(name, _) => Err(format!("cannot evaluate '{}' numerically", name)),
        Expr::Complex(..) => Err("cannot plot a complex value on a real axis".to_string()),
        Expr::Matrix(..) => Err("cannot evaluate a matrix to a single number".to_string()),
        Expr::Bool(_) => Err("cannot evaluate a boolean to a number".to_string()),
        Expr::Function { .. } => Err("cannot evaluate a function value to a number".to_string()),
        Expr::Equation(..) => Err("cannot evaluate an equation to a number".to_string()),
    }
}

/// Sample `e` at `n` evenly spaced values of `var` across [a, b], for plotting.
/// Points where evaluation fails or is non-finite come back as `None` — the
/// renderer draws a gap there (poles, log of negatives, …) rather than a lie.
pub fn sample(e: &Expr, var: &str, a: f64, b: f64, n: usize) -> Vec<(f64, Option<f64>)> {
    let n = n.clamp(2, 100_000);
    let step = (b - a) / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let x = a + step * i as f64;
            let y = match eval_f64(e, Some((var, x))) {
                Ok(y) if y.is_finite() => Some(y),
                _ => None,
            };
            (x, y)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    fn expr_of(src: &str) -> Expr {
        Interpreter::new().eval_line(src).unwrap()
    }

    #[test]
    fn agrees_with_the_exact_engine() {
        // Same oracle idea as the property suite, pointed the other way: the
        // f64 path must match exact-then-N to f64 precision.
        for src in ["1/3 + sin(1)", "exp(2) - pi", "2^10 + 1/7", "cos(pi)"] {
            let e = expr_of(src);
            let fast = eval_f64(&e, None).unwrap();
            let exact = expr_of(&format!("N({}, 25)", src));
            let exact_str = format!("{}", exact);
            let slow: f64 = exact_str.parse().unwrap();
            assert!(
                (fast - slow).abs() <= 1e-12 * slow.abs().max(1.0),
                "{}: fast {} vs exact {}",
                src,
                fast,
                slow
            );
        }
    }

    #[test]
    fn variable_binding_and_gaps() {
        let e = expr_of("x^2 + 1");
        assert_eq!(eval_f64(&e, Some(("x", 3.0))).unwrap(), 10.0);
        assert!(eval_f64(&e, None).is_err()); // free symbol

        // 1/x has a pole at 0: the sample there is a gap, not infinity.
        let inv = expr_of("x^(-1)");
        let pts = sample(&inv, "x", -1.0, 1.0, 3);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].1, Some(-1.0));
        assert_eq!(pts[1].1, None); // x = 0
        assert_eq!(pts[2].1, Some(1.0));
    }

    #[test]
    fn negative_base_integer_power() {
        let e = expr_of("y^3");
        assert_eq!(eval_f64(&e, Some(("y", -2.0))).unwrap(), -8.0);
    }
}
