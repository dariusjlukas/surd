//! The tree-walking evaluator: lowers an [`crate::ast::Node`] into a canonical
//! [`Expr`] within a scope, dispatches builtins, and runs control flow.

use crate::ast::{Node, Op};
use crate::expr::*;
use crate::lexer::lex;
use crate::matrix;
use crate::parser::parse;
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::collections::HashMap;
use std::rc::Rc;

/// Safety backstops, since "compute time be damned" still shouldn't mean
/// "hang the REPL forever" or overflow the stack on adversarial input. These
/// assume evaluation runs with a generous stack (see `surd::run_with_stack`).
const MAX_FRAMES: usize = 1_500; // function-call recursion depth
const MAX_EVAL_DEPTH: usize = 8_000; // expression-evaluation recursion depth
const MAX_ITERS: u64 = 10_000_000;

/// Holds the workspace and, during a function call, a stack of local frames.
/// `frames[0]` is always the global workspace.
pub struct Interpreter {
    frames: Vec<HashMap<String, Expr>>,
    /// Default significant digits for `N(x)` (overridable per call and via
    /// `precision(d)`).
    default_digits: usize,
    /// Current expression-evaluation recursion depth.
    eval_depth: usize,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            frames: vec![HashMap::new()],
            default_digits: 30,
            eval_depth: 0,
        }
    }

    /// Parse and evaluate a complete program (one or more statements). The
    /// value is that of the final statement.
    pub fn eval_line(&mut self, src: &str) -> Result<Expr, String> {
        let program = parse(lex(src)?)?;
        self.eval_node(&program)
    }

    /// The global workspace, for `:vars`.
    pub fn workspace(&self) -> impl Iterator<Item = (&String, &Expr)> {
        self.frames[0].iter()
    }

    /// Bind a value directly in the global workspace, bypassing the parser.
    /// Used by data imports, where the value never had source text.
    pub fn set_global(&mut self, name: &str, value: Expr) {
        self.frames[0].insert(name.to_string(), value);
    }

    /// Look up a global binding by name (for data export).
    pub fn get_global(&self, name: &str) -> Option<&Expr> {
        self.frames[0].get(name)
    }

    // -- scope ---------------------------------------------------------------

    /// Look up a bound value: the current frame first, then the global frame.
    fn get_var(&self, name: &str) -> Option<Expr> {
        if self.frames.len() > 1 {
            if let Some(v) = self.frames.last().unwrap().get(name) {
                return Some(v.clone());
            }
        }
        self.frames[0].get(name).cloned()
    }

    /// Bind a value in the current frame (local inside a function, else global).
    fn set_var(&mut self, name: &str, value: Expr) {
        self.frames.last_mut().unwrap().insert(name.to_string(), value);
    }

    // -- evaluation ----------------------------------------------------------

    /// Depth-guarded entry point: bounds expression-evaluation recursion so
    /// deeply nested or very long expressions error instead of overflowing.
    fn eval_node(&mut self, node: &Node) -> Result<Expr, String> {
        self.eval_depth += 1;
        if self.eval_depth > MAX_EVAL_DEPTH {
            self.eval_depth -= 1;
            return Err("expression is nested too deeply".to_string());
        }
        let result = self.eval_node_inner(node);
        self.eval_depth -= 1;
        result
    }

    fn eval_node_inner(&mut self, node: &Node) -> Result<Expr, String> {
        match node {
            Node::Num(s) => Ok(parse_number(s)),
            Node::Ident(name) => Ok(self.lookup(name)),
            Node::Neg(n) => Ok(mul(vec![int(-1), self.eval_node(n)?])),
            Node::Not(n) => {
                let b = as_bool(&self.eval_node(n)?)?;
                Ok(Expr::Bool(!b))
            }
            Node::BinOp(op, a, b) => self.eval_binop(*op, a, b),
            Node::Matrix(node_rows) => {
                let mut rows = Vec::with_capacity(node_rows.len());
                for node_row in node_rows {
                    let mut row = Vec::with_capacity(node_row.len());
                    for cell in node_row {
                        row.push(self.eval_node(cell)?);
                    }
                    rows.push(row);
                }
                matrix::matrix(rows)
            }
            Node::Field(base, field) => {
                let value = self.eval_node(base)?;
                match value {
                    Expr::Struct(fields) => fields
                        .iter()
                        .find(|(n, _)| n == field)
                        .map(|(_, v)| v.clone())
                        .ok_or_else(|| {
                            let names: Vec<&str> =
                                fields.iter().map(|(n, _)| n.as_str()).collect();
                            format!(
                                "struct has no field '{}' (fields: {})",
                                field,
                                names.join(", ")
                            )
                        }),
                    other => Err(format!(
                        "cannot read field '.{}' of a non-struct value '{}'",
                        field, other
                    )),
                }
            }
            Node::Call(name, args) => {
                // diff/subs treat their variable argument as a *name*, not a
                // value — they must see the expression before the workspace
                // collapses it (x := 3 must not turn diff(x^2, x) into
                // diff(9, 3)). Skipped if the user defined their own function
                // with that name.
                if matches!(name.as_str(), "diff" | "D" | "subs" | "plot" | "plot3d")
                    && !matches!(self.get_var(name), Some(Expr::Function { .. }))
                {
                    return self.call_calculus(name, args);
                }
                // struct(a = 1, ...) reads its field names from the syntax —
                // the `a` must not collapse to a workspace binding first.
                if name == "struct"
                    && !matches!(self.get_var(name), Some(Expr::Function { .. }))
                {
                    return self.call_struct(args);
                }
                let evaluated = args
                    .iter()
                    .map(|a| self.eval_node(a))
                    .collect::<Result<Vec<_>, _>>()?;
                self.call(name, evaluated)
            }
            Node::Assign(name, n) => {
                check_assignable(name)?;
                let value = self.eval_node(n)?;
                self.set_var(name, value.clone());
                Ok(value)
            }
            Node::Equation(l, r) => Ok(Expr::Equation(
                Box::new(self.eval_node(l)?),
                Box::new(self.eval_node(r)?),
            )),
            Node::If(cond, then_b, else_b) => {
                if as_bool(&self.eval_node(cond)?)? {
                    self.eval_node(then_b)
                } else if let Some(e) = else_b {
                    self.eval_node(e)
                } else {
                    Ok(int(0))
                }
            }
            Node::While(cond, body) => {
                let mut last = int(0);
                let mut iters: u64 = 0;
                while as_bool(&self.eval_node(cond)?)? {
                    last = self.eval_node(body)?;
                    iters += 1;
                    if iters >= MAX_ITERS {
                        return Err(format!(
                            "while loop exceeded {} iterations (possible infinite loop)",
                            MAX_ITERS
                        ));
                    }
                }
                Ok(last)
            }
            Node::FuncDef(name, params, body) => {
                check_assignable(name)?;
                let f = Expr::Function {
                    params: params.clone(),
                    body: Rc::new((**body).clone()),
                };
                self.set_var(name, f.clone());
                Ok(f)
            }
            Node::Block(stmts) => {
                let mut last = int(0);
                for s in stmts {
                    last = self.eval_node(s)?;
                }
                Ok(last)
            }
        }
    }

    fn eval_binop(&mut self, op: Op, a: &Node, b: &Node) -> Result<Expr, String> {
        // Logical operators short-circuit, so evaluate lazily.
        if matches!(op, Op::And | Op::Or) {
            let left = as_bool(&self.eval_node(a)?)?;
            return match op {
                Op::And if !left => Ok(Expr::Bool(false)),
                Op::Or if left => Ok(Expr::Bool(true)),
                _ => Ok(Expr::Bool(as_bool(&self.eval_node(b)?)?)),
            };
        }

        let x = self.eval_node(a)?;
        let y = self.eval_node(b)?;
        match op {
            // Two numbers compare by value — decidable, and it keeps `==`
            // consistent with `<`/`<=` (so N(2) == 2 is true). Everything else
            // is decidable *structural* equality after canonicalization; that
            // is NOT a claim about mathematical equality of reals (undecidable).
            Op::Equal => Ok(Expr::Bool(value_eq(&x, &y))),
            Op::NotEqual => Ok(Expr::Bool(!value_eq(&x, &y))),
            Op::Less | Op::Greater | Op::LessEq | Op::GreaterEq => compare(op, &x, &y),
            _ => self.eval_arith(op, x, y),
        }
    }

    fn eval_arith(&mut self, op: Op, x: Expr, y: Expr) -> Result<Expr, String> {
        if is_opaque_value(&x) || is_opaque_value(&y) {
            return Err("cannot do arithmetic on a boolean, function, or struct value".into());
        }
        if matrix::is_matrix(&x) || matrix::is_matrix(&y) {
            return matrix_binop(op, x, y);
        }
        match op {
            Op::Add => Ok(add(vec![x, y])),
            Op::Sub => Ok(add(vec![x, mul(vec![int(-1), y])])),
            Op::Mul => Ok(mul(vec![x, y])),
            Op::Div => {
                if comparable_value(&y).is_some_and(|r| r.is_zero()) {
                    Err("division by zero".to_string())
                } else {
                    Ok(mul(vec![x, pow(y, int(-1))]))
                }
            }
            Op::Pow => Ok(pow(x, y)),
            _ => unreachable!("non-arithmetic op reached eval_arith"),
        }
    }

    fn lookup(&self, name: &str) -> Expr {
        // true/false are literals, never bindable (rejected at assignment).
        match name {
            "true" => return Expr::Bool(true),
            "false" => return Expr::Bool(false),
            _ => {}
        }
        // User bindings shadow the built-in constants, so `e`, `pi`, and `I`
        // all stay usable as ordinary variables (like `i` for loop counters).
        self.get_var(name).unwrap_or_else(|| match name {
            "pi" | "π" => Expr::Const(Constant::Pi),
            "e" => Expr::Const(Constant::E),
            "I" => imaginary_unit(),
            _ => Expr::Symbol(name.to_string()),
        })
    }

    /// Call `name`: a user-defined function if one is bound, otherwise a builtin.
    fn call(&mut self, name: &str, args: Vec<Expr>) -> Result<Expr, String> {
        if let Some(Expr::Function { params, body }) = self.get_var(name) {
            if params.len() != args.len() {
                return Err(format!(
                    "{} expects {} argument(s), got {}",
                    name,
                    params.len(),
                    args.len()
                ));
            }
            if self.frames.len() >= MAX_FRAMES {
                return Err("maximum recursion depth exceeded".into());
            }
            let frame: HashMap<String, Expr> =
                params.into_iter().zip(args).collect();
            self.frames.push(frame);
            let result = self.eval_node(&body);
            self.frames.pop();
            return result;
        }
        // `precision` needs &mut self to change the default, so it lives here
        // rather than among the (read-only) builtins.
        if name == "precision" {
            return self.set_precision(args);
        }
        self.call_builtin(name, args)
    }

    /// `diff(expr, x)` / `D(expr, x)` / `subs(expr, x, val)` / `plot(expr, x,
    /// a, b)`: the variable is taken by name and kept symbolic while `expr`
    /// evaluates, so a workspace binding of `x` doesn't collapse the
    /// expression first. For `diff`, the binding is substituted back into the
    /// derivative afterwards — the same treatment any other expression
    /// mentioning `x` gets: `x := 3; diff(x^2, x)` is 2·x at x = 3, i.e. 6.
    /// `plot` stays a symbolic value; the frontend samples and draws it.
    fn call_calculus(&mut self, name: &str, args: &[Node]) -> Result<Expr, String> {
        if name == "plot" {
            return self.call_plot(args);
        }
        if name == "plot3d" {
            return self.call_plot3d(args);
        }
        let expected = if name == "subs" { 3 } else { 2 };
        if args.len() != expected {
            return Err(format!(
                "{} expects {} argument(s), got {}",
                name,
                expected,
                args.len()
            ));
        }
        let var = self.var_name(&args[1])?;
        let target = self.eval_shadowed(&[var.clone()], &args[0])?;

        if name == "subs" {
            let val = self.eval_node(&args[2])?;
            return Ok(substitute(&target, &var, &val));
        }
        let deriv = differentiate(&target, &var);
        match self.lookup(&var) {
            Expr::Symbol(s) if s == var => Ok(deriv),
            bound => Ok(substitute(&deriv, &var, &bound)),
        }
    }

    /// `plot(f1, ..., fk, x, a, b)` — one or more curves over a shared
    /// window. Stays a symbolic value; the frontend samples and draws it.
    fn call_plot(&mut self, args: &[Node]) -> Result<Expr, String> {
        if args.len() < 4 {
            return Err(format!(
                "plot expects plot(f1, ..., fk, x, a, b), got {} argument(s)",
                args.len()
            ));
        }
        let var_idx = args.len() - 3;
        let var = self.var_name(&args[var_idx])?;
        let mut out = Vec::with_capacity(args.len());
        for f in &args[..var_idx] {
            out.push(self.eval_shadowed(std::slice::from_ref(&var), f)?);
        }
        out.push(Expr::Symbol(var));
        out.push(self.eval_node(&args[var_idx + 1])?);
        out.push(self.eval_node(&args[var_idx + 2])?);
        Ok(Expr::Func("plot".to_string(), out))
    }

    /// `plot3d(f, x, a, b, y, c, d)` — a surface z = f(x, y) over
    /// [a, b]×[c, d]. Stays symbolic, like `plot`.
    fn call_plot3d(&mut self, args: &[Node]) -> Result<Expr, String> {
        if args.len() != 7 {
            return Err(format!(
                "plot3d expects plot3d(f, x, a, b, y, c, d), got {} argument(s)",
                args.len()
            ));
        }
        let xvar = self.var_name(&args[1])?;
        let yvar = self.var_name(&args[4])?;
        if xvar == yvar {
            return Err("plot3d: the two plot variables must differ".into());
        }
        let target = self.eval_shadowed(&[xvar.clone(), yvar.clone()], &args[0])?;
        Ok(Expr::Func(
            "plot3d".to_string(),
            vec![
                target,
                Expr::Symbol(xvar),
                self.eval_node(&args[2])?,
                self.eval_node(&args[3])?,
                Expr::Symbol(yvar),
                self.eval_node(&args[5])?,
                self.eval_node(&args[6])?,
            ],
        ))
    }

    /// `struct(name = value, ...)` — each argument must literally be
    /// `name = value`; the names become fields, the values evaluate normally.
    fn call_struct(&mut self, args: &[Node]) -> Result<Expr, String> {
        let mut fields = Vec::with_capacity(args.len());
        for arg in args {
            let Node::Equation(lhs, rhs) = arg else {
                return Err("struct expects 'name = value' arguments, e.g. struct(a = 1)".into());
            };
            let Node::Ident(name) = lhs.as_ref() else {
                return Err("struct field names must be plain identifiers".into());
            };
            fields.push((name.clone(), self.eval_node(rhs)?));
        }
        structure(fields)
    }

    /// A *name* argument (the variable of diff/subs/plot): a bare identifier,
    /// or anything that evaluates to a symbol.
    fn var_name(&mut self, node: &Node) -> Result<String, String> {
        match node {
            Node::Ident(s) => Ok(s.clone()),
            other => as_symbol(&self.eval_node(other)?),
        }
    }

    /// Evaluate `node` with each of `vars` shadowed by its own symbol, so the
    /// expression is seen before the workspace collapses those names
    /// (`x := 3` must not turn `plot(x^2, x, 0, 1)` into `plot(9, 3, 0, 1)`).
    /// Callers must not pass duplicate names (restore order would be wrong).
    fn eval_shadowed(&mut self, vars: &[String], node: &Node) -> Result<Expr, String> {
        let frame = self.frames.last_mut().unwrap();
        let saved: Vec<Option<Expr>> = vars
            .iter()
            .map(|v| frame.insert(v.clone(), Expr::Symbol(v.clone())))
            .collect();
        let result = self.eval_node(node);
        let frame = self.frames.last_mut().unwrap();
        for (v, s) in vars.iter().zip(saved) {
            match s {
                Some(val) => {
                    frame.insert(v.clone(), val);
                }
                None => {
                    frame.remove(v);
                }
            }
        }
        result
    }

    /// `precision()` queries the default digit count; `precision(d)` sets it.
    fn set_precision(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        match args.len() {
            0 => Ok(int(self.default_digits as i64)),
            1 => {
                let d = as_usize(&args[0])?.clamp(1, 100_000);
                self.default_digits = d;
                Ok(int(d as i64))
            }
            _ => Err("precision expects 0 or 1 argument".into()),
        }
    }

    fn call_builtin(&self, name: &str, args: Vec<Expr>) -> Result<Expr, String> {
        match name {
            "sqrt" => {
                arity(name, &args, 1)?;
                Ok(pow(args[0].clone(), half()))
            }
            "sin" | "cos" | "tan" | "exp" | "ln" => {
                arity(name, &args, 1)?;
                Ok(func(name, args))
            }
            "conj" => {
                arity(name, &args, 1)?;
                Ok(conjugate(&args[0]))
            }
            "re" | "real" => {
                arity(name, &args, 1)?;
                Ok(real_part(&args[0]))
            }
            "im" | "imag" => {
                arity(name, &args, 1)?;
                Ok(imag_part(&args[0]))
            }
            "abs" => {
                arity(name, &args, 1)?;
                Ok(absolute_value(&args[0]))
            }
            // ("diff"/"D"/"subs" never reach here — they're intercepted before
            // argument evaluation so the variable argument stays symbolic.)
            "expand" => {
                arity(name, &args, 1)?;
                Ok(expand(&args[0]))
            }
            "det" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::det(&args[0])
            }
            "inv" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::inverse(&args[0])
            }
            "transpose" | "T" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                Ok(matrix::transpose(&args[0]))
            }
            "rref" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                Ok(matrix::rref(&args[0]))
            }
            "rank" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                Ok(matrix::rank(&args[0]))
            }
            "solve" => {
                arity(name, &args, 2)?;
                matrix::solve(&args[0], &args[1])
            }
            "charpoly" => match args.len() {
                1 => {
                    expect_matrix(name, &args[0])?;
                    matrix::char_poly(&args[0], "lambda")
                }
                2 => {
                    expect_matrix(name, &args[0])?;
                    let var = as_symbol(&args[1])?;
                    matrix::char_poly(&args[0], &var)
                }
                _ => Err(format!("charpoly expects 1 or 2 arguments, got {}", args.len())),
            },
            "eigenvalues" | "eig" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::eigenvalues(&args[0])
            }
            "eigenvectors" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::eigenvectors(&args[0])
            }
            "nullspace" | "kernel" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::nullspace(&args[0])
            }
            "lu" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::lu(&args[0])
            }
            "qr" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                matrix::qr(&args[0])
            }
            "eye" | "identity" => {
                arity(name, &args, 1)?;
                Ok(matrix::identity(as_usize(&args[0])?))
            }
            "N" => {
                // N(x) uses a default precision; N(x, digits) sets it.
                let digits = match args.len() {
                    1 => self.default_digits,
                    2 => as_usize(&args[1])?,
                    _ => return Err(format!("N expects 1 or 2 arguments, got {}", args.len())),
                };
                numeric_eval(&args[0], digits)
            }
            // Unknown name: keep it as a symbolic, unevaluated application.
            _ => Ok(func(name, args)),
        }
    }
}

/// Linear-algebra dispatch for binary operators when a matrix is involved.
fn matrix_binop(op: Op, x: Expr, y: Expr) -> Result<Expr, String> {
    let (xm, ym) = (matrix::is_matrix(&x), matrix::is_matrix(&y));
    match op {
        Op::Add | Op::Sub => {
            if xm && ym {
                matrix::mat_add(&x, &y, matches!(op, Op::Sub))
            } else {
                Err("matrix addition needs two matrices (a matrix and a scalar don't add)".into())
            }
        }
        Op::Mul => {
            if xm && ym {
                matrix::mat_mul(&x, &y)
            } else if xm {
                Ok(matrix::scalar_mul(&y, &x))
            } else {
                Ok(matrix::scalar_mul(&x, &y))
            }
        }
        Op::Div => {
            if ym {
                let inv = matrix::inverse(&y)?;
                if xm {
                    matrix::mat_mul(&x, &inv)
                } else {
                    Ok(matrix::scalar_mul(&x, &inv))
                }
            } else {
                if comparable_value(&y).is_some_and(|r| r.is_zero()) {
                    return Err("division by zero".into());
                }
                Ok(matrix::scalar_mul(&pow(y, int(-1)), &x))
            }
        }
        Op::Pow => {
            if !xm {
                return Err("cannot raise a scalar to a matrix power".into());
            }
            match matrix::integer_exponent(&y) {
                Some(n) => matrix::mat_pow(&x, n),
                None => Err("a matrix can only be raised to an integer power".into()),
            }
        }
        _ => Err("that operator isn't defined on matrices".into()),
    }
}

/// The exact rational value of `e` for comparison purposes: exact numbers
/// directly, floats via their exact binary value (lossless — a float *is* a
/// rational m·2^k). `None` for anything symbolic.
fn comparable_value(e: &Expr) -> Option<BigRational> {
    match e {
        Expr::Float(bf, _) => float_to_rational(bf),
        _ => numeric_value(e),
    }
}

/// Equality test: by value when both sides are numbers (decidable, and floats
/// participate), structural otherwise.
fn value_eq(x: &Expr, y: &Expr) -> bool {
    match (comparable_value(x), comparable_value(y)) {
        (Some(p), Some(q)) => p == q,
        _ => x == y,
    }
}

/// Ordering comparison. Requires both sides to be numbers (exact or float) —
/// deciding the order of an arbitrary symbolic value is undecidable, so we
/// refuse rather than guess (wrap in `N(...)` to compare numerically).
fn compare(op: Op, x: &Expr, y: &Expr) -> Result<Expr, String> {
    match (comparable_value(x), comparable_value(y)) {
        (Some(p), Some(q)) => {
            let result = match op {
                Op::Less => p < q,
                Op::Greater => p > q,
                Op::LessEq => p <= q,
                Op::GreaterEq => p >= q,
                _ => unreachable!(),
            };
            Ok(Expr::Bool(result))
        }
        _ => Err(format!(
            "cannot order '{}' and '{}'; both must be numbers (try N(...))",
            x, y
        )),
    }
}

/// `true`/`false` are literals; everything else (including `pi`, `e`, `I`) may
/// be rebound — user bindings shadow the built-in constants.
fn check_assignable(name: &str) -> Result<(), String> {
    if matches!(name, "true" | "false") {
        Err(format!("cannot assign to '{}'", name))
    } else {
        Ok(())
    }
}

fn as_bool(e: &Expr) -> Result<bool, String> {
    match e {
        Expr::Bool(b) => Ok(*b),
        other => Err(format!(
            "expected a true/false condition, got '{}'",
            other
        )),
    }
}

/// Values arithmetic can never touch: booleans, functions, structs.
fn is_opaque_value(e: &Expr) -> bool {
    matches!(e, Expr::Bool(_) | Expr::Function { .. } | Expr::Struct(_))
}

fn expect_matrix(name: &str, e: &Expr) -> Result<(), String> {
    if matrix::is_matrix(e) {
        Ok(())
    } else {
        Err(format!("{} expects a matrix argument", name))
    }
}

fn as_symbol(e: &Expr) -> Result<String, String> {
    if let Expr::Symbol(s) = e {
        Ok(s.clone())
    } else {
        Err(format!("expected a variable name, got '{}'", e))
    }
}

fn as_usize(e: &Expr) -> Result<usize, String> {
    if let Some(r) = numeric_value(e) {
        if r.is_integer() {
            if let Some(n) = r.to_integer().to_usize() {
                return Ok(n);
            }
        }
    }
    Err("expected a non-negative integer".into())
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

/// Convert a numeric literal to an exact value. Decimals become exact
/// rationals (`1.5` -> 3/2) — floats are opt-in, never the default.
fn parse_number(s: &str) -> Expr {
    if let Some(dot) = s.find('.') {
        let int_part = &s[..dot];
        let frac_part = &s[dot + 1..];
        let digits = format!("{}{}", int_part, frac_part);
        let numer: BigInt = digits.parse().unwrap_or_else(|_| BigInt::from(0));
        let denom = num_traits::pow::pow(BigInt::from(10), frac_part.len());
        rat_to_expr(BigRational::new(numer, denom))
    } else {
        Expr::Int(s.parse().unwrap_or_else(|_| BigInt::from(0)))
    }
}
