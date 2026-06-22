//! The tree-walking evaluator: lowers an [`crate::ast::Node`] into a canonical
//! [`Expr`] within a scope, dispatches builtins, and runs control flow.

use crate::ast::{Node, Op};
use crate::dsp;
use crate::expr::*;
use crate::interval;
use crate::lexer::lex;
use crate::matrix;
use crate::nlfit;
use crate::parser::parse;
use crate::signal;
use crate::stats;
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
        self.frames
            .last_mut()
            .unwrap()
            .insert(name.to_string(), value);
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
                            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                            format!(
                                "struct has no field '{}' (fields: {})",
                                field,
                                names.join(", ")
                            )
                        }),
                    // An unbound namespace name evaluates to its symbol; point
                    // at the call syntax instead of a baffling struct error.
                    Expr::Symbol(s) if is_namespace(&s) => Err(format!(
                        "'{}.{}' names a function in the built-in '{}' namespace — \
                         call it with arguments: {}.{}(...)",
                        s, field, s, s, field
                    )),
                    other => Err(format!(
                        "cannot read field '.{}' of a non-struct value '{}'",
                        field, other
                    )),
                }
            }
            Node::FieldCall(base, name, args) => self.eval_field_call(base, name, args),
            Node::Index(base, idxs) => {
                let value = self.eval_node(base)?;
                let mut indices = Vec::with_capacity(idxs.len());
                for ix in idxs {
                    indices.push(as_index(&self.eval_node(ix)?)?);
                }
                // Indexing a signal reads the midpoint of that sample; the
                // certified half-width is bound(s, i).
                if let Expr::Signal(s) = &value {
                    let [i] = indices.as_slice() else {
                        return Err("a signal takes a single index".into());
                    };
                    if !(1..=s.len()).contains(i) {
                        return Err(format!(
                            "index {} is out of range (the signal has {})",
                            i,
                            s.len()
                        ));
                    }
                    return Ok(signal::midpoint(s, i - 1));
                }
                matrix::index(&value, &indices)
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
                if name == "struct" && !matches!(self.get_var(name), Some(Expr::Function { .. })) {
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
            // A formula keeps its operands symbolic — they name data columns,
            // so a workspace binding of `x` must not collapse `y ~ x`.
            Node::Formula(l, r) => {
                let mut names = Vec::new();
                collect_node_idents(l, &mut names);
                collect_node_idents(r, &mut names);
                let le = self.eval_shadowed(&names, l)?;
                let re = self.eval_shadowed(&names, r)?;
                Ok(Expr::Formula(Box::new(le), Box::new(re)))
            }
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
        if matches!(&x, Expr::Signal(_)) || matches!(&y, Expr::Signal(_)) {
            return signal_arith(op, &x, &y);
        }
        if matches!(op, Op::ElemMul | Op::ElemDiv | Op::ElemPow) {
            return elementwise_binop(op, &x, &y);
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
            return self.call_function(name, params, body, args);
        }
        // `precision` and `map` need &mut self (one mutates state, the other
        // calls back into user functions), so they live here rather than
        // among the (read-only) builtins.
        if name == "precision" {
            return self.set_precision(args);
        }
        if name == "map" {
            return self.call_map(args);
        }
        self.call_builtin(name, args)
    }

    /// `map(f, m)` — apply a function entrywise, preserving shape. `f` is a
    /// function value or a function's name (user-defined or built-in), so
    /// both `map(sin, v)` and `map(myfunc, v)` work.
    fn call_map(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        arity("map", &args, 2)?;
        let Expr::Matrix(rows) = &args[1] else {
            return Err("map expects a vector or matrix as its second argument".into());
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut new_row = Vec::with_capacity(row.len());
            for cell in row {
                let v = match &args[0] {
                    Expr::Function { params, body } => self.call_function(
                        "the mapped function",
                        params.clone(),
                        body.clone(),
                        vec![cell.clone()],
                    )?,
                    Expr::Symbol(s) => self.call(&s.clone(), vec![cell.clone()])?,
                    other => {
                        return Err(format!(
                            "map expects a function as its first argument, got '{}'",
                            other
                        ))
                    }
                };
                new_row.push(v);
            }
            out.push(new_row);
        }
        Ok(Expr::Matrix(out))
    }

    /// Invoke a function value: bind the arguments in a fresh frame, run the
    /// body, pop the frame. `name` is only for error messages.
    fn call_function(
        &mut self,
        name: &str,
        params: Vec<String>,
        body: Rc<Node>,
        args: Vec<Expr>,
    ) -> Result<Expr, String> {
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
        let frame: HashMap<String, Expr> = params.into_iter().zip(args).collect();
        self.frames.push(frame);
        let result = self.eval_node(&body);
        self.frames.pop();
        result
    }

    /// `base.name(args)`: a built-in namespace function (`dsp.dft(v)`), or a
    /// user module — a struct whose `name` field holds a function. A user
    /// binding of the namespace's name shadows it, the same rule as any
    /// other builtin.
    fn eval_field_call(&mut self, base: &Node, name: &str, args: &[Node]) -> Result<Expr, String> {
        if let Node::Ident(ns) = base {
            if self.get_var(ns).is_none() && is_namespace(ns) {
                // nlfit takes a symbolic model and parameter *names*, so its
                // arguments must be seen before the workspace collapses them —
                // the same treatment diff/plot give their variable argument.
                if ns == "stats" && name == "nlfit" {
                    return self.eval_nlfit(args);
                }
                let evaluated = args
                    .iter()
                    .map(|a| self.eval_node(a))
                    .collect::<Result<Vec<_>, _>>()?;
                return call_namespace(ns, name, evaluated);
            }
        }
        let value = self.eval_node(base)?;
        let Expr::Struct(fields) = value else {
            return Err(format!(
                "cannot call '.{}(...)' on a non-struct value '{}'",
                name, value
            ));
        };
        let Some((_, field)) = fields.iter().find(|(n, _)| n == name) else {
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            return Err(format!(
                "struct has no field '{}' (fields: {})",
                name,
                names.join(", ")
            ));
        };
        let Expr::Function { params, body } = field.clone() else {
            return Err(format!(
                "field '{}' holds '{}', which is not a function",
                name, field
            ));
        };
        let evaluated = args
            .iter()
            .map(|a| self.eval_node(a))
            .collect::<Result<Vec<_>, _>>()?;
        self.call_function(name, params, body, evaluated)
    }

    /// `stats.nlfit(model, [params], x, y[, init])`: nonlinear least squares.
    /// The `model` and the parameter-name list are read symbolically (the
    /// parameters are shadowed so a workspace binding doesn't collapse them);
    /// whatever free symbol is left over is the independent variable, matched
    /// to the `x` data.
    fn eval_nlfit(&mut self, args: &[Node]) -> Result<Expr, String> {
        if !(4..=5).contains(&args.len()) {
            return Err(format!(
                "stats.nlfit expects 4 or 5 arguments (model, [params], x, y[, init]), got {}",
                args.len()
            ));
        }
        let params = read_param_names(&args[1])?;
        if params.is_empty() {
            return Err("stats.nlfit: the parameter list is empty".into());
        }
        // The model with parameters held symbolic; constants resolve normally.
        let model = self.eval_shadowed(&params, &args[0])?;
        let free = nlfit::free_symbols(&model);
        for p in &params {
            if !free.contains(p) {
                return Err(format!(
                    "stats.nlfit: parameter '{}' does not appear in the model",
                    p
                ));
            }
        }
        let indep: Vec<&String> = free.iter().filter(|s| !params.contains(s)).collect();
        let xvar = match indep.as_slice() {
            [v] => (*v).clone(),
            [] => {
                return Err(
                    "stats.nlfit: the model has no independent variable (is it bound in \
                            the workspace?)"
                        .into(),
                )
            }
            _ => {
                return Err(format!(
                    "stats.nlfit: the model has more than one independent variable {:?}",
                    indep
                ))
            }
        };
        let x = numeric_vector(&self.eval_node(&args[2])?)?;
        let y = numeric_vector(&self.eval_node(&args[3])?)?;
        let init = match args.get(4) {
            Some(node) => numeric_vector(&self.eval_node(node)?)?,
            None => vec![1.0; params.len()],
        };
        let result = nlfit::fit(&model, &params, &xvar, &x, &y, &init)?;
        attach_predict(result, &model, &params, &xvar)
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
    /// `plot(s1, ..., sk)` over signals draws their samples directly.
    fn call_plot(&mut self, args: &[Node]) -> Result<Expr, String> {
        if !args.is_empty() && args.len() < 4 {
            let evaluated = args
                .iter()
                .map(|a| self.eval_node(a))
                .collect::<Result<Vec<_>, _>>()?;
            if evaluated.iter().all(|e| matches!(e, Expr::Signal(_))) {
                return Ok(Expr::Func("plotsignal".to_string(), evaluated));
            }
            // All data series and no window: draw the points over a window
            // derived from the data (the wasm side pads the x-extent).
            if evaluated.iter().all(is_scatter) {
                return Ok(Expr::Func("plotscatter".to_string(), evaluated));
            }
        }
        if args.len() < 4 {
            return Err(format!(
                "plot expects plot(f1, ..., fk, x, a, b) — or plot(s) for a signal — got {} argument(s)",
                args.len()
            ));
        }
        let var_idx = args.len() - 3;
        let var = self.var_name(&args[var_idx])?;
        let mut out = Vec::with_capacity(args.len());
        for f in &args[..var_idx] {
            let curve = self.eval_shadowed(std::slice::from_ref(&var), f)?;
            if matches!(curve, Expr::Signal(_)) {
                return Err("signals plot without a window — plot(s), not plot(s, x, a, b)".into());
            }
            // A bare function value — a fit's `predict`, or any user function —
            // plots as its body: apply it to the plot variable to get the curve.
            let curve = match curve {
                Expr::Function { params, body } => {
                    if params.len() != 1 {
                        return Err(format!(
                            "plot: a function curve must take one argument, but this takes {}",
                            params.len()
                        ));
                    }
                    self.call_function(
                        "the plotted function",
                        params,
                        body,
                        vec![Expr::Symbol(var.clone())],
                    )?
                }
                other => other,
            };
            out.push(curve);
        }
        out.push(Expr::Symbol(var));
        out.push(self.eval_node(&args[var_idx + 1])?);
        out.push(self.eval_node(&args[var_idx + 2])?);
        Ok(Expr::Func("plot".to_string(), out))
    }

    /// `plot3d(f, x, a, b, y, c, d)` — a surface z = f(x, y) over [a, b]×[c, d].
    /// `scatter3d(x, y, z)` data may be overlaid before the window args (one
    /// surface at most), and `plot3d(scatter3d(...))` draws points alone over a
    /// window derived from the data. Stays symbolic, like `plot`.
    fn call_plot3d(&mut self, args: &[Node]) -> Result<Expr, String> {
        // Bare 3D scatter: all data and no window — box it from the data.
        if !args.is_empty() && args.len() < 7 {
            let evaluated = args
                .iter()
                .map(|a| self.eval_node(a))
                .collect::<Result<Vec<_>, _>>()?;
            if evaluated.iter().all(is_scatter3d) {
                return Ok(Expr::Func("plot3dscatter".to_string(), evaluated));
            }
        }
        if args.len() < 7 {
            return Err(format!(
                "plot3d expects plot3d(f, x, a, b, y, c, d) — or plot3d(scatter3d(x, y, z)) \
                 for data — got {} argument(s)",
                args.len()
            ));
        }
        // The trailing six args are always x, a, b, y, c, d; everything before
        // them is a drawable (the surface, and/or scatter3d overlays).
        let base = args.len() - 6;
        let xvar = self.var_name(&args[base])?;
        let yvar = self.var_name(&args[base + 3])?;
        if xvar == yvar {
            return Err("plot3d: the two plot variables must differ".into());
        }
        let mut out = Vec::with_capacity(args.len());
        let mut surfaces = 0;
        for d in &args[..base] {
            let drawable = self.eval_shadowed(&[xvar.clone(), yvar.clone()], d)?;
            if !is_scatter3d(&drawable) {
                surfaces += 1;
                if surfaces > 1 {
                    return Err("plot3d draws a single surface; pass one f(x, y) \
                                (plus any scatter3d data)"
                        .into());
                }
            }
            out.push(drawable);
        }
        out.push(Expr::Symbol(xvar));
        out.push(self.eval_node(&args[base + 1])?);
        out.push(self.eval_node(&args[base + 2])?);
        out.push(Expr::Symbol(yvar));
        out.push(self.eval_node(&args[base + 4])?);
        out.push(self.eval_node(&args[base + 5])?);
        Ok(Expr::Func("plot3d".to_string(), out))
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
            // Scalar functions apply entrywise to a matrix argument.
            "sqrt" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(pow(e.clone(), half())));
                }
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::unary("sqrt", s)?)));
                }
                Ok(pow(args[0].clone(), half()))
            }
            "sin" | "cos" | "tan" | "exp" | "ln" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(func(name, vec![e.clone()])));
                }
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::unary(name, s)?)));
                }
                Ok(func(name, args))
            }
            // Special functions: symbolic objects that fold at exact arguments
            // (gamma of an integer, erf(0)) and otherwise evaluate under N(...).
            "erf" | "erfc" | "gamma" | "lgamma" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(func(name, vec![e.clone()])));
                }
                Ok(func(name, args))
            }
            "beta" => {
                arity(name, &args, 2)?;
                Ok(func("beta", args))
            }
            "scatter" => {
                arity(name, &args, 2)?;
                let x = vector_entries("scatter", &args[0])?;
                let y = vector_entries("scatter", &args[1])?;
                if x.len() != y.len() {
                    return Err(format!(
                        "scatter expects two vectors of the same length, got {} and {}",
                        x.len(),
                        y.len()
                    ));
                }
                if x.is_empty() {
                    return Err("scatter expects non-empty vectors".into());
                }
                // A scatter is a static data series: keep the two columns as
                // row vectors and carry them as a tagged value the plot path
                // (`call_plot`, wasm `plot_data`) recognizes and draws as
                // markers. It is not a function, so it is never sampled.
                Ok(func(
                    "scatter",
                    vec![Expr::Matrix(vec![x]), Expr::Matrix(vec![y])],
                ))
            }
            "scatter3d" => {
                arity(name, &args, 3)?;
                let x = vector_entries("scatter3d", &args[0])?;
                let y = vector_entries("scatter3d", &args[1])?;
                let z = vector_entries("scatter3d", &args[2])?;
                if !(x.len() == y.len() && y.len() == z.len()) {
                    return Err(format!(
                        "scatter3d expects three vectors of the same length, got {}, {}, {}",
                        x.len(),
                        y.len(),
                        z.len()
                    ));
                }
                if x.is_empty() {
                    return Err("scatter3d expects non-empty vectors".into());
                }
                // A 3D scatter is static data carried as a tagged value the
                // plot3d path recognizes and draws as markers — never sampled.
                Ok(func(
                    "scatter3d",
                    vec![
                        Expr::Matrix(vec![x]),
                        Expr::Matrix(vec![y]),
                        Expr::Matrix(vec![z]),
                    ],
                ))
            }
            "conj" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(conjugate(e)));
                }
                Ok(conjugate(&args[0]))
            }
            "re" | "real" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(real_part(e)));
                }
                Ok(real_part(&args[0]))
            }
            "im" | "imag" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(imag_part(e)));
                }
                Ok(imag_part(&args[0]))
            }
            "abs" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(absolute_value(e)));
                }
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::unary("abs", s)?)));
                }
                Ok(absolute_value(&args[0]))
            }
            // -- the exact ↔ certified-bulk boundary ---------------------------
            "signal" => {
                let digits = match args.len() {
                    1 => None,
                    2 => Some(as_usize(&args[1])?),
                    _ => {
                        return Err(format!(
                            "signal expects 1 or 2 arguments, got {}",
                            args.len()
                        ))
                    }
                };
                let entries = vector_arg(name, &args[0])?;
                Ok(Expr::Signal(std::rc::Rc::new(signal::pack(
                    &entries, digits,
                )?)))
            }
            "mid" => {
                arity(name, &args, 1)?;
                let Expr::Signal(s) = &args[0] else {
                    return Err("mid expects a signal".into());
                };
                Ok(signal::mid_matrix(s))
            }
            "bound" => {
                let Some(Expr::Signal(s)) = args.first() else {
                    return Err("bound expects a signal".into());
                };
                match args.len() {
                    1 => Ok(signal::half_width(s, None)),
                    2 => {
                        let i = as_usize(&args[1])?;
                        if !(1..=s.len()).contains(&i) {
                            return Err(format!(
                                "index {} is out of range (the signal has {})",
                                i,
                                s.len()
                            ));
                        }
                        Ok(signal::half_width(s, Some(i - 1)))
                    }
                    _ => Err(format!(
                        "bound expects 1 or 2 arguments, got {}",
                        args.len()
                    )),
                }
            }
            // -- data primitives ---------------------------------------------
            "len" => {
                arity(name, &args, 1)?;
                match (matrix::vector_of(&args[0]), &args[0]) {
                    (Some(v), _) => Ok(int(v.len() as i64)),
                    (None, Expr::Matrix(rows)) => Ok(int(rows.len() as i64)),
                    (None, Expr::Signal(s)) => Ok(int(s.len() as i64)),
                    _ => Err("len expects a vector, matrix, or signal".into()),
                }
            }
            "size" => {
                arity(name, &args, 1)?;
                expect_matrix(name, &args[0])?;
                let Expr::Matrix(rows) = &args[0] else {
                    unreachable!()
                };
                structure(vec![
                    ("rows".to_string(), int(rows.len() as i64)),
                    ("cols".to_string(), int(rows[0].len() as i64)),
                ])
            }
            "dot" => {
                arity(name, &args, 2)?;
                let (a, b) = (vector_arg(name, &args[0])?, vector_arg(name, &args[1])?);
                if a.len() != b.len() {
                    return Err(format!(
                        "dot expects two vectors of the same length, got {} and {}",
                        a.len(),
                        b.len()
                    ));
                }
                // Plain bilinear Σ aᵢ·bᵢ — no conjugation (use conj()).
                Ok(add(a
                    .into_iter()
                    .zip(b)
                    .map(|(x, y)| mul(vec![x, y]))
                    .collect()))
            }
            "vcat" | "hcat" => concat(name, &args),
            "slice" => {
                arity(name, &args, 3)?;
                let start = as_index(&args[1])?;
                let n = as_usize(&args[2])?;
                match &args[0] {
                    Expr::Signal(s) => Ok(Expr::Signal(std::rc::Rc::new(signal::slice(
                        s,
                        start - 1,
                        n,
                    )?))),
                    Expr::Matrix(rows) if rows.len() == 1 => {
                        check_slice(start, n, rows[0].len())?;
                        Ok(Expr::Matrix(vec![
                            rows[0][start - 1..start - 1 + n].to_vec()
                        ]))
                    }
                    Expr::Matrix(rows) if rows.iter().all(|r| r.len() == 1) => {
                        check_slice(start, n, rows.len())?;
                        Ok(Expr::Matrix(rows[start - 1..start - 1 + n].to_vec()))
                    }
                    _ => Err("slice expects a vector or signal".into()),
                }
            }
            "linspace" => {
                arity(name, &args, 3)?;
                let n = as_usize(&args[2])?;
                if n < 2 {
                    return Err("linspace expects at least 2 points".into());
                }
                // a + k·(b−a)/(n−1): exact when the endpoints are.
                let step = mul(vec![
                    add(vec![args[1].clone(), mul(vec![int(-1), args[0].clone()])]),
                    pow(int(n as i64 - 1), int(-1)),
                ]);
                let row = (0..n)
                    .map(|k| {
                        add(vec![
                            args[0].clone(),
                            mul(vec![int(k as i64), step.clone()]),
                        ])
                    })
                    .collect();
                Ok(Expr::Matrix(vec![row]))
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
                _ => Err(format!(
                    "charpoly expects 1 or 2 arguments, got {}",
                    args.len()
                )),
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

/// The built-in namespaces. Each groups a domain toolkit behind one name so
/// the global builtin set stays small; `ns.func(...)` dispatches here.
fn is_namespace(name: &str) -> bool {
    matches!(name, "dsp" | "stats" | "data")
}

fn call_namespace(ns: &str, name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    match ns {
        "dsp" => dsp::call(name, args),
        "stats" => stats::call(name, args),
        "data" => crate::data::call(name, args),
        _ => unreachable!("call_namespace on a non-namespace"),
    }
}

/// Collect the identifiers appearing in a syntax node (deduped, first-seen
/// order) — used to shadow a formula's column names before evaluating it.
fn collect_node_idents(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Ident(s) => {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        Node::BinOp(_, a, b) | Node::Equation(a, b) | Node::Formula(a, b) => {
            collect_node_idents(a, out);
            collect_node_idents(b, out);
        }
        Node::Neg(a) | Node::Not(a) => collect_node_idents(a, out),
        Node::Call(_, args) => args.iter().for_each(|a| collect_node_idents(a, out)),
        Node::Field(b, _) => collect_node_idents(b, out),
        Node::FieldCall(b, _, args) => {
            collect_node_idents(b, out);
            args.iter().for_each(|a| collect_node_idents(a, out));
        }
        Node::Index(b, idx) => {
            collect_node_idents(b, out);
            idx.iter().for_each(|i| collect_node_idents(i, out));
        }
        Node::Matrix(rows) => rows
            .iter()
            .flatten()
            .for_each(|c| collect_node_idents(c, out)),
        _ => {}
    }
}

/// Read a bracketed list of plain identifiers, e.g. the `[a, b]` parameter list
/// of `stats.nlfit`. Taken from the syntax so the names stay names.
fn read_param_names(node: &Node) -> Result<Vec<String>, String> {
    let Node::Matrix(rows) = node else {
        return Err(
            "stats.nlfit: the second argument must be a list of parameter names, e.g. [a, b]"
                .into(),
        );
    };
    let mut names = Vec::new();
    for cell in rows.iter().flatten() {
        match cell {
            Node::Ident(s) => names.push(s.clone()),
            _ => {
                return Err(
                    "stats.nlfit: parameter names must be plain identifiers, e.g. [a, b]".into(),
                )
            }
        }
    }
    Ok(names)
}

/// Collapse an evaluated vector (1×n or n×1 matrix) of numbers to `f64`s.
fn numeric_vector(e: &Expr) -> Result<Vec<f64>, String> {
    let entries: Vec<Expr> = match e {
        Expr::Matrix(rows) if rows.len() == 1 => rows[0].clone(),
        Expr::Matrix(rows) if rows.iter().all(|r| r.len() == 1) => {
            rows.iter().map(|r| r[0].clone()).collect()
        }
        _ => return Err("stats.nlfit expects vectors for x, y, and initial values".into()),
    };
    entries
        .iter()
        .map(|v| crate::f64eval::eval_f64(v, &[]))
        .collect()
}

/// Elementwise `.*` `./` `.^`: entrywise when a matrix is involved (shapes
/// must match when both sides are matrices, a scalar broadcasts), and the
/// plain scalar operation otherwise — so `2 .* 3` is just `6`.
fn elementwise_binop(op: Op, x: &Expr, y: &Expr) -> Result<Expr, String> {
    let scalar = |p: &Expr, q: &Expr| -> Result<Expr, String> {
        match op {
            Op::ElemMul => Ok(mul(vec![p.clone(), q.clone()])),
            Op::ElemDiv => {
                if comparable_value(q).is_some_and(|r| r.is_zero()) {
                    Err("division by zero".to_string())
                } else {
                    Ok(mul(vec![p.clone(), pow(q.clone(), int(-1))]))
                }
            }
            Op::ElemPow => Ok(pow(p.clone(), q.clone())),
            _ => unreachable!("non-elementwise op in elementwise_binop"),
        }
    };
    match (matrix::is_matrix(x), matrix::is_matrix(y)) {
        (true, true) => matrix::try_zip(x, y, scalar),
        (true, false) => matrix::try_map(x, |p| scalar(p, y)),
        (false, true) => matrix::try_map(y, |q| scalar(x, q)),
        (false, false) => scalar(x, y),
    }
}

/// Arithmetic with a signal on either side. Signal⊕signal is elementwise
/// for `+ −` (and the dotted operators); plain `*`/`/` between two signals
/// refuse, pointing at `.*`/`./` — signals have no matrix product to be
/// ambiguous with, but consistency with matrices keeps one mental model.
/// Scalars broadcast; exact matrices must be packed explicitly.
fn signal_arith(op: Op, x: &Expr, y: &Expr) -> Result<Expr, String> {
    use crate::signal as sig;
    use std::rc::Rc;
    let wrap = |s: sig::SignalData| Ok(Expr::Signal(Rc::new(s)));
    if matrix::is_matrix(x) || matrix::is_matrix(y) {
        return Err(
            "cannot mix an exact matrix with a signal — pack it first: signal([...])".into(),
        );
    }
    let opstr = match op {
        Op::Add => "+",
        Op::Sub => "-",
        Op::Mul | Op::ElemMul => "*",
        Op::Div | Op::ElemDiv => "/",
        Op::Pow | Op::ElemPow => "^",
        _ => unreachable!("non-arithmetic op on a signal"),
    };
    match (x, y) {
        (Expr::Signal(a), Expr::Signal(b)) => match (op, opstr) {
            (Op::Mul, _) => Err("use .* for elementwise signal multiplication".into()),
            (Op::Div, _) => Err("use ./ for elementwise signal division".into()),
            (_, "^") => Err("signals only take integer scalar exponents (s .^ 2)".into()),
            _ => wrap(sig::binop(opstr, a, b)?),
        },
        (Expr::Signal(s), scalar) => {
            if opstr == "^" {
                let n = numeric_value(scalar)
                    .filter(|r| r.is_integer())
                    .and_then(|r| r.to_integer().to_i64())
                    .ok_or("signals only take integer scalar exponents (s .^ 2)")?;
                return wrap(sig::powi(s, n)?);
            }
            wrap(sig::scalar_binop(opstr, s, scalar, false)?)
        }
        (scalar, Expr::Signal(s)) => {
            if opstr == "^" {
                return Err("cannot raise a scalar to a signal power".into());
            }
            wrap(sig::scalar_binop(opstr, s, scalar, true)?)
        }
        _ => unreachable!("signal_arith without a signal"),
    }
}

fn check_slice(start: usize, n: usize, len: usize) -> Result<(), String> {
    if n == 0 {
        return Err("slice needs at least 1 element".into());
    }
    if start.checked_add(n).is_none_or(|e| e - 1 > len) {
        return Err(format!(
            "slice of {} from position {} runs past the end (length {})",
            n, start, len
        ));
    }
    Ok(())
}

fn as_index(e: &Expr) -> Result<usize, String> {
    match as_usize(e) {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!(
            "indices are 1-based positive integers, got '{}'",
            e
        )),
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

/// Ordering comparison. Numbers compare by value. Constant symbolic
/// expressions (`sqrt(2)+sqrt(3) > pi`) are decided by certified interval
/// refinement: the answer is only given once enclosures provably separate,
/// so it is never wrong — values that can't be separated (they may be
/// equal) refuse, as do free symbols, whose order is genuinely undecidable.
fn compare(op: Op, x: &Expr, y: &Expr) -> Result<Expr, String> {
    use std::cmp::Ordering;

    let decide = |ord: Ordering| {
        Ok(Expr::Bool(match op {
            Op::Less => ord == Ordering::Less,
            Op::Greater => ord == Ordering::Greater,
            Op::LessEq => ord != Ordering::Greater,
            Op::GreaterEq => ord != Ordering::Less,
            _ => unreachable!(),
        }))
    };

    if let (Some(p), Some(q)) = (comparable_value(x), comparable_value(y)) {
        return decide(p.cmp(&q));
    }
    // Values arithmetic can't touch can't be ordered either — and must not
    // reach the difference construction below.
    let unorderable = |e: &Expr| {
        is_opaque_value(e)
            || matrix::is_matrix(e)
            || matches!(e, Expr::Equation(..) | Expr::Signal(_))
    };
    if unorderable(x) || unorderable(y) {
        return Err(format!("cannot order '{}' and '{}'", x, y));
    }
    // Sign of the difference: exact canonicalization may settle it outright
    // (x − x is 0, (x+1) − x is 1 — sound for every real x), and certified
    // interval refinement handles the constant remainder.
    let d = add(vec![x.clone(), mul(vec![int(-1), y.clone()])]);
    if let Some(r) = comparable_value(&d) {
        return decide(r.cmp(&BigRational::zero()));
    }
    let inseparable = || {
        Err(format!(
            "cannot order '{}' and '{}': they agree to at least {} significant digits — \
             the values may be equal",
            x,
            y,
            interval::max_digits()
        ))
    };
    match interval::certified_sign(&d) {
        interval::Sign::Negative => decide(Ordering::Less),
        interval::Sign::Zero => decide(Ordering::Equal),
        interval::Sign::Positive => decide(Ordering::Greater),
        // One-sided knowledge answers the operators it can and refuses the
        // rest: x − y ≥ 0 settles `>=` and `<`, but not `>` or `<=`.
        interval::Sign::NonNegative => match op {
            Op::GreaterEq => Ok(Expr::Bool(true)),
            Op::Less => Ok(Expr::Bool(false)),
            _ => inseparable(),
        },
        interval::Sign::NonPositive => match op {
            Op::LessEq => Ok(Expr::Bool(true)),
            Op::Greater => Ok(Expr::Bool(false)),
            _ => inseparable(),
        },
        interval::Sign::Inseparable => inseparable(),
        interval::Sign::Unsupported => Err(format!(
            "cannot order '{}' and '{}'; both must be constant real values \
             (a free symbol has no fixed value — try subs(...) or N(...))",
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
        other => Err(format!("expected a true/false condition, got '{}'", other)),
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

fn vector_arg(name: &str, e: &Expr) -> Result<Vec<Expr>, String> {
    matrix::vector_of(e).ok_or_else(|| format!("{} expects a vector (a 1×n or n×1 matrix)", name))
}

/// `vcat`/`hcat`: stack matrices vertically/horizontally. Scalars join as
/// 1×1 matrices, so `vcat(v, 5)` appends an element.
fn concat(name: &str, args: &[Expr]) -> Result<Expr, String> {
    if args.is_empty() {
        return Err(format!("{} expects at least 1 argument", name));
    }
    let mut blocks: Vec<Vec<Vec<Expr>>> = Vec::with_capacity(args.len());
    for a in args {
        match a {
            Expr::Matrix(rows) => blocks.push(rows.clone()),
            e if is_opaque_value(e) => return Err(format!("{} cannot include '{}'", name, e)),
            scalar => blocks.push(vec![vec![scalar.clone()]]),
        }
    }
    if name == "vcat" {
        let cols = blocks[0][0].len();
        if blocks.iter().any(|b| b[0].len() != cols) {
            return Err("vcat needs the same number of columns in every piece".into());
        }
        Ok(Expr::Matrix(blocks.into_iter().flatten().collect()))
    } else {
        let rows = blocks[0].len();
        if blocks.iter().any(|b| b.len() != rows) {
            return Err("hcat needs the same number of rows in every piece".into());
        }
        let mut out: Vec<Vec<Expr>> = vec![Vec::new(); rows];
        for block in blocks {
            for (i, row) in block.into_iter().enumerate() {
                out[i].extend(row);
            }
        }
        Ok(Expr::Matrix(out))
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

/// Entries of a vector argument (a 1×n or n×1 matrix), in order.
fn vector_entries(name: &str, e: &Expr) -> Result<Vec<Expr>, String> {
    let Expr::Matrix(rows) = e else {
        return Err(format!("{} expects vectors (1×n or n×1 matrices)", name));
    };
    if rows.len() == 1 {
        Ok(rows[0].clone())
    } else if rows.iter().all(|r| r.len() == 1) {
        Ok(rows.iter().map(|r| r[0].clone()).collect())
    } else {
        Err(format!("{} expects vectors (1×n or n×1 matrices)", name))
    }
}

/// A `scatter(x, y)` data value, carried symbolically into a plot.
fn is_scatter(e: &Expr) -> bool {
    matches!(e, Expr::Func(name, _) if name.as_str() == "scatter")
}

/// A `scatter3d(x, y, z)` data value, carried symbolically into a plot3d.
fn is_scatter3d(e: &Expr) -> bool {
    matches!(e, Expr::Func(name, _) if name.as_str() == "scatter3d")
}

/// Wrap a closed expression in `var` as a one-argument function value — how a
/// fit hands back its curve as a real `predict` function (`m.predict(x)`
/// evaluates it; `plot(m.predict, x, a, b)` draws it). Built by round-tripping
/// the expression's re-parseable text into a function body, so the result is
/// an ordinary `Expr::Function`, indistinguishable from one written `f(x) := …`.
pub(crate) fn function_from_expr(var: &str, body: &Expr) -> Result<Expr, String> {
    let node = parse(lex(&format!("{}", body))?)?;
    Ok(Expr::Function {
        params: vec![var.to_string()],
        body: Rc::new(node),
    })
}

/// Add a `predict` field — the fitted curve as a function of the predictor — to
/// an `nlfit` result, by substituting the fitted coefficients into the model.
fn attach_predict(
    result: Expr,
    model: &Expr,
    params: &[String],
    xvar: &str,
) -> Result<Expr, String> {
    let Expr::Struct(mut fields) = result else {
        return Ok(result);
    };
    let Some(coefs) = fields.iter().find(|(n, _)| n == "coefficients") else {
        return Ok(Expr::Struct(fields));
    };
    let coefs = vector_entries("nlfit", &coefs.1)?;
    if coefs.len() != params.len() {
        return Ok(Expr::Struct(fields));
    }
    let mut body = model.clone();
    for (p, c) in params.iter().zip(coefs.iter()) {
        body = substitute(&body, p, c);
    }
    fields.push(("predict".to_string(), function_from_expr(xvar, &body)?));
    structure(fields)
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
