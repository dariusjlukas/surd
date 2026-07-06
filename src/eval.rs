//! The tree-walking evaluator: lowers an [`crate::ast::Node`] into a canonical
//! [`Expr`] within a scope, dispatches builtins, and runs control flow.

use crate::ast::{ForIter, IndexArg, Node, Op, Step};
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
    /// value is that of the final statement. Top-level statements run in
    /// statement position (their values feed only the REPL echo), so a
    /// conditional statement — `if converged then r := x end` — is allowed
    /// even though a false `if` without `else` has no value.
    pub fn eval_line(&mut self, src: &str) -> Result<Expr, String> {
        let program = parse(lex(src)?)?;
        self.eval_discarded(&program)
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

    /// Evaluate an optional range bound to a 1-based index, leaving an omitted
    /// bound (`None`) for the selection to fill against the axis length.
    fn eval_opt_index(&mut self, n: &Option<Box<Node>>) -> Result<Option<usize>, String> {
        match n {
            Some(n) => Ok(Some(as_index(&self.eval_node(n)?)?)),
            None => Ok(None),
        }
    }

    /// Lower a range's stride to a `(take, skip)` pair: keep `take` consecutive
    /// positions, then skip `skip`, repeating. No stride means contiguous
    /// (`1, 0`); a scalar stride `k` keeps every k-th position (`1, k - 1`).
    fn eval_step(&mut self, step: &Option<Step>) -> Result<(usize, usize), String> {
        match step {
            None => Ok((1, 0)),
            Some(Step::By(k)) => {
                let k = as_positive(&self.eval_node(k)?, "a stride")?;
                Ok((1, k - 1))
            }
            Some(Step::TakeSkip(t, s)) => {
                let take = as_positive(&self.eval_node(t)?, "a take count")?;
                let skip = as_count(&self.eval_node(s)?, "a skip count")?;
                Ok((take, skip))
            }
        }
    }

    fn eval_node_inner(&mut self, node: &Node) -> Result<Expr, String> {
        match node {
            Node::Num(s) => Ok(parse_number(s)),
            Node::Str(s) => Ok(Expr::Str(s.clone())),
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
                let mut sels = Vec::with_capacity(idxs.len());
                for arg in idxs {
                    sels.push(match arg {
                        IndexArg::Scalar(n) => matrix::Sel::One(as_index(&self.eval_node(n)?)?),
                        IndexArg::Range { lo, hi, step } => {
                            let (take, skip) = self.eval_step(step)?;
                            matrix::Sel::Range {
                                lo: self.eval_opt_index(lo)?,
                                hi: self.eval_opt_index(hi)?,
                                take,
                                skip,
                            }
                        }
                    });
                }
                if let Expr::Signal(s) = &value {
                    return index_signal(s, &sels);
                }
                matrix::select(&value, &sels)
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
                // pairs(M, [a, b]) names/selects columns by *symbol*, like a
                // model formula — a workspace binding of `weight` must not
                // collapse a column named `weight` to a number first.
                if name == "pairs" && !matches!(self.get_var(name), Some(Expr::Function { .. })) {
                    return self.call_pairs(args);
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
            // This arm evaluates an `if` whose value is *used* (an assignment's
            // right side, a function argument, a function's result, ...). With
            // no `else` and a false condition there is no value to produce —
            // refuse rather than invent one (a silent `0` here shipped as the
            // same bug family as defaulting an undecidable `==` to false). In
            // statement position [`Self::eval_discarded`] allows the same
            // program, because nothing consumes the missing value there.
            Node::If(cond, then_b, else_b) => {
                if as_bool(&self.eval_node(cond)?)? {
                    self.eval_node(then_b)
                } else if let Some(e) = else_b {
                    self.eval_node(e)
                } else {
                    Err(
                        "this 'if' has no 'else', so it has no value when the condition \
                         is false — add an else branch"
                            .to_string(),
                    )
                }
            }
            Node::While(cond, body) => {
                let mut last = int(0);
                let mut iters: u64 = 0;
                while as_bool(&self.eval_node(cond)?)? {
                    // The body is statement position: its statements run for
                    // their bindings, and the loop's value (the last body
                    // evaluation) is almost never consumed.
                    last = self.eval_discarded(body)?;
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
            Node::For { var, iter, body } => self.eval_for(var, iter, body),
            Node::FuncDef(name, params, body) => {
                check_assignable(name)?;
                let f = Expr::Function {
                    params: params.clone(),
                    body: Rc::new((**body).clone()),
                    env: self.capture_env(params, body),
                };
                self.set_var(name, f.clone());
                Ok(f)
            }
            Node::Lambda(params, body) => Ok(Expr::Function {
                params: params.clone(),
                body: Rc::new((**body).clone()),
                env: self.capture_env(params, body),
            }),
            Node::Block(stmts) => {
                if let Some((final_stmt, rest)) = stmts.split_last() {
                    for s in rest {
                        self.eval_discarded(s)?;
                    }
                    // The final statement's value is the block's value.
                    self.eval_node(final_stmt)
                } else {
                    Ok(int(0))
                }
            }
        }
    }

    /// Evaluate a node in *statement position* — its value is discarded (or
    /// only echoed by the REPL), so an `if` without `else` is allowed to take
    /// its false branch. Discard-ness propagates structurally: through every
    /// statement of a block, and into the branches of an `if`. Everything
    /// else is evaluated normally.
    fn eval_discarded(&mut self, node: &Node) -> Result<Expr, String> {
        self.eval_depth += 1;
        if self.eval_depth > MAX_EVAL_DEPTH {
            self.eval_depth -= 1;
            return Err("expression is nested too deeply".to_string());
        }
        let result = match node {
            Node::If(cond, then_b, else_b) => {
                match self.eval_node(cond).and_then(|c| as_bool(&c)) {
                    Err(e) => Err(e),
                    Ok(true) => self.eval_discarded(then_b),
                    Ok(false) => match else_b {
                        Some(e) => self.eval_discarded(e),
                        None => Ok(int(0)),
                    },
                }
            }
            Node::Block(stmts) => {
                let mut last = int(0);
                for s in stmts {
                    last = self.eval_discarded(s)?;
                }
                Ok(last)
            }
            other => self.eval_node_inner(other),
        };
        self.eval_depth -= 1;
        result
    }

    /// Run a `for` loop. A range `lo:hi` / `lo:step:hi` iterates the exact
    /// values lo, lo+step, … up to and including `hi` when it lands on it
    /// (endpoints and step must be exact numbers — the comparisons that stop
    /// the loop must be decidable). A matrix iterates its elements (vector)
    /// or its rows (m×n). The loop variable stays bound after the loop, like
    /// a `while` counter; the loop's value is the last body evaluation.
    fn eval_for(&mut self, var: &str, iter: &ForIter, body: &Node) -> Result<Expr, String> {
        check_assignable(var)?;
        let mut last = int(0);
        match iter {
            ForIter::Range { lo, step, hi } => {
                let lo = exact_bound(&self.eval_node(lo)?)?;
                let hi = exact_bound(&self.eval_node(hi)?)?;
                let step = match step {
                    Some(s) => exact_bound(&self.eval_node(s)?)?,
                    None => BigRational::from_integer(BigInt::from(1)),
                };
                if step.is_zero() {
                    return Err("for: the range step must be nonzero".into());
                }
                // Bound the trip count before running (exact arithmetic, so
                // this is cheap and cannot be fooled by rounding).
                let span = (&hi - &lo) / &step;
                if span >= BigRational::from_integer(BigInt::from(MAX_ITERS)) {
                    return Err(format!(
                        "for loop would run more than {} iterations",
                        MAX_ITERS
                    ));
                }
                let ascending = step > BigRational::zero();
                let mut i = lo;
                loop {
                    if (ascending && i > hi) || (!ascending && i < hi) {
                        break;
                    }
                    self.set_var(var, rat_to_expr(i.clone()));
                    last = self.eval_discarded(body)?;
                    i += &step;
                }
            }
            ForIter::Expr(node) => {
                let value = self.eval_node(node)?;
                let Expr::Matrix(rows) = &value else {
                    return Err(format!(
                        "for: cannot iterate over '{}' — loop over a range \
                         (for i in 1:n do ... end) or a matrix",
                        value
                    ));
                };
                let items: Vec<Expr> = if rows.len() == 1 {
                    rows[0].clone()
                } else if rows.iter().all(|r| r.len() == 1) {
                    rows.iter().map(|r| r[0].clone()).collect()
                } else {
                    // An m×n matrix iterates its rows, matching `m[i]`.
                    rows.iter().map(|r| Expr::Matrix(vec![r.clone()])).collect()
                };
                for item in items {
                    self.set_var(var, item);
                    last = self.eval_discarded(body)?;
                }
            }
        }
        Ok(last)
    }

    /// The environment a function value captures at creation: the *local*
    /// variables (current frame) that its body mentions, snapshotted by
    /// value. At top level the current frame is the global workspace and
    /// nothing is captured — free names there stay late-bound (which is what
    /// makes `fact(n) := ... fact(n-1) ...` work before `fact` exists).
    fn capture_env(&self, params: &[String], body: &Node) -> Vec<(String, Expr)> {
        if self.frames.len() <= 1 {
            return Vec::new();
        }
        let mut names = Vec::new();
        collect_capture_names(body, &mut names);
        let frame = self.frames.last().unwrap();
        let mut env: Vec<(String, Expr)> = names
            .into_iter()
            .filter(|n| !params.contains(n))
            .filter_map(|n| frame.get(&n).map(|v| (n, v.clone())))
            .collect();
        // Sorted so equal captures compare equal regardless of mention order.
        env.sort_by(|a, b| a.0.cmp(&b.0));
        env
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
            // consistent with `<`/`<=` (so N(2) == 2 is true). Constant
            // symbolic values go through the certified equality machinery
            // (which refuses when it cannot decide); expressions with free
            // symbols compare *structurally* after canonicalization — that
            // is NOT a claim about mathematical equality of reals (undecidable).
            Op::Equal => Ok(Expr::Bool(value_eq(&x, &y)?)),
            Op::NotEqual => Ok(Expr::Bool(!value_eq(&x, &y)?)),
            Op::Less | Op::Greater | Op::LessEq | Op::GreaterEq => compare(op, &x, &y),
            _ => self.eval_arith(op, x, y),
        }
    }

    fn eval_arith(&mut self, op: Op, x: Expr, y: Expr) -> Result<Expr, String> {
        if is_opaque_value(&x) || is_opaque_value(&y) {
            return Err(
                "cannot do arithmetic on a boolean, string, function, or struct value".into(),
            );
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
        if let Some(f @ Expr::Function { .. }) = self.get_var(name) {
            return self.call_value(name, &f, args, Some(name));
        }
        // `precision` and the higher-order builtins need &mut self (one
        // mutates state, the others call back into user functions), so they
        // live here rather than among the (read-only) builtins.
        match name {
            "precision" => self.set_precision(args),
            "map" => self.call_map(args),
            "fill" => self.call_fill(args),
            "filter" => self.call_filter(args),
            "fold" => self.call_fold(args),
            _ => self.call_builtin(name, args),
        }
    }

    /// Apply a function value (or a builtin named by a bare symbol, so
    /// `map(sin, v)` works) to arguments. `who` is only for error messages;
    /// `self_name` optionally rebinds the function under its own name inside
    /// the call, so a *local* function can recurse (a global one already can,
    /// through the workspace).
    fn call_value(
        &mut self,
        who: &str,
        f: &Expr,
        args: Vec<Expr>,
        self_name: Option<&str>,
    ) -> Result<Expr, String> {
        match f {
            Expr::Function { params, body, env } => self.call_function(
                who,
                params.clone(),
                body.clone(),
                env.clone(),
                args,
                self_name,
            ),
            Expr::Symbol(s) => self.call(&s.clone(), args),
            other => Err(format!("{} expects a function, got '{}'", who, other)),
        }
    }

    /// `map(f, m)` / `map(f, m1, ..., mk)` — apply a function entrywise,
    /// preserving shape; with several matrices (all the same shape) the
    /// function receives one entry from each, so `map(f, a, b)` is the
    /// zip-with-`f` of `a` and `b`. `f` is a function value or a function's
    /// name (user-defined or built-in), so both `map(sin, v)` and
    /// `map(myfunc, v)` work.
    fn call_map(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        if args.len() < 2 {
            return Err(format!(
                "map expects map(f, m) or map(f, m1, ..., mk), got {} argument(s)",
                args.len()
            ));
        }
        check_callable("map", &args[0])?;
        let mut shapes = Vec::with_capacity(args.len() - 1);
        for m in &args[1..] {
            let Expr::Matrix(rows) = m else {
                return Err("map expects vectors or matrices after the function".into());
            };
            shapes.push((rows.len(), rows[0].len()));
        }
        if shapes.iter().any(|s| *s != shapes[0]) {
            return Err("map: all matrices must have the same shape".into());
        }
        let (nrows, ncols) = shapes[0];
        let mut out = Vec::with_capacity(nrows);
        for i in 0..nrows {
            let mut new_row = Vec::with_capacity(ncols);
            for j in 0..ncols {
                let cells: Vec<Expr> = args[1..]
                    .iter()
                    .map(|m| match m {
                        Expr::Matrix(rows) => rows[i][j].clone(),
                        _ => unreachable!("checked above"),
                    })
                    .collect();
                let v = self.call_value("the mapped function", &args[0], cells, None)?;
                if !is_scalar(&v) {
                    return Err(format!(
                        "map: the mapped function must return a scalar for each entry, got '{}'",
                        v
                    ));
                }
                new_row.push(v);
            }
            out.push(new_row);
        }
        Ok(Expr::Matrix(out))
    }

    /// `filter(pred, v)` — the elements of a vector for which `pred` returns
    /// `true`, preserving orientation (row in, row out). The predicate must
    /// return an actual boolean for every element — anything else refuses.
    /// Keeping *no* elements is an error too: there is no empty matrix.
    fn call_filter(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        arity("filter", &args, 2)?;
        check_callable("filter", &args[0])?;
        let entries = vector_entries("filter", &args[1])?;
        let is_row = matches!(&args[1], Expr::Matrix(rows) if rows.len() == 1);
        let mut kept = Vec::new();
        for cell in entries {
            let verdict =
                self.call_value("the filter predicate", &args[0], vec![cell.clone()], None)?;
            let Expr::Bool(keep) = verdict else {
                return Err(format!(
                    "filter: the predicate must return true or false for every element, \
                     got '{}'",
                    verdict
                ));
            };
            if keep {
                kept.push(cell);
            }
        }
        if kept.is_empty() {
            return Err("filter kept no elements, and there is no empty matrix — \
                        check the predicate"
                .into());
        }
        Ok(if is_row {
            Expr::Matrix(vec![kept])
        } else {
            Expr::Matrix(kept.into_iter().map(|e| vec![e]).collect())
        })
    }

    /// `fold(f, init, v)` — left fold: acc := f(acc, x) over the elements of a
    /// vector (or the entries of a matrix, row-major), starting from `init`.
    /// The accumulator may be any value, so folds can build matrices or
    /// structs, not just scalars.
    fn call_fold(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        arity("fold", &args, 3)?;
        check_callable("fold", &args[0])?;
        let Expr::Matrix(rows) = &args[2] else {
            return Err("fold expects a vector or matrix as its third argument".into());
        };
        let rows = rows.clone();
        let mut acc = args[1].clone();
        for row in rows {
            for cell in row {
                acc = self.call_value("the fold function", &args[0], vec![acc, cell], None)?;
            }
        }
        Ok(acc)
    }

    /// `fill(value, n)` / `fill(value, rows, cols)` — a matrix whose every entry
    /// is `value`, or, when `value` is a function, the value `f(row, col)` at
    /// each 1-based coordinate (matching `m[row, col]` indexing). The function
    /// form calls back into user code, so — like `map` — this lives here rather
    /// than among the read-only builtins.
    fn call_fill(&mut self, args: Vec<Expr>) -> Result<Expr, String> {
        let (rows, cols) = match args.len() {
            2 => {
                let n = as_usize(&args[1])?;
                (n, n)
            }
            3 => (as_usize(&args[1])?, as_usize(&args[2])?),
            _ => {
                return Err(format!(
                "fill expects 2 or 3 arguments: fill(value, n) or fill(value, rows, cols), got {}",
                args.len()
            ))
            }
        };
        if rows == 0 || cols == 0 {
            return Err("fill needs positive dimensions (there is no empty matrix)".into());
        }
        // A function is applied at each coordinate; anything else is repeated.
        if matches!(&args[0], Expr::Function { .. }) {
            let f = args[0].clone();
            let mut out = Vec::with_capacity(rows);
            for i in 1..=rows {
                let mut row = Vec::with_capacity(cols);
                for j in 1..=cols {
                    let v = self.call_value(
                        "the fill function",
                        &f,
                        vec![int(i as i64), int(j as i64)],
                        None,
                    )?;
                    if !is_scalar(&v) {
                        return Err(format!(
                            "fill: the fill function must return a scalar for each entry, got '{}'",
                            v
                        ));
                    }
                    row.push(v);
                }
                out.push(row);
            }
            return Ok(Expr::Matrix(out));
        }
        let value = &args[0];
        if !is_scalar(value) {
            return Err(format!(
                "fill needs a scalar value or a function of (row, col), not '{}'",
                value
            ));
        }
        matrix::fill(value, rows, cols)
    }

    /// Invoke a function value: bind the captured environment, then the
    /// arguments (parameters win a name collision), in a fresh frame; run the
    /// body; pop the frame. `name` is only for error messages. `self_name`
    /// additionally binds the function to its own name inside the frame, so
    /// local functions and lambdas can recurse.
    fn call_function(
        &mut self,
        name: &str,
        params: Vec<String>,
        body: Rc<Node>,
        env: Vec<(String, Expr)>,
        args: Vec<Expr>,
        self_name: Option<&str>,
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
        let mut frame: HashMap<String, Expr> = env.iter().cloned().collect();
        if let Some(self_name) = self_name {
            frame.insert(
                self_name.to_string(),
                Expr::Function {
                    params: params.clone(),
                    body: body.clone(),
                    env,
                },
            );
        }
        frame.extend(params.into_iter().zip(args));
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
        let Expr::Function { params, body, env } = field.clone() else {
            return Err(format!(
                "field '{}' holds '{}', which is not a function",
                name, field
            ));
        };
        let evaluated = args
            .iter()
            .map(|a| self.eval_node(a))
            .collect::<Result<Vec<_>, _>>()?;
        // No self-binding by field name: the body may use that name for
        // something else entirely (a struct field `sin` must not shadow the
        // builtin `sin` inside the function it holds).
        self.call_function(name, params, body, env, evaluated, None)
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
            // `substitute` splices the value into scalar positions directly,
            // so a container value would corrupt the canonical form.
            if !is_scalar(&val) {
                return Err(format!(
                    "subs replaces a variable with a scalar expression, not '{}'",
                    val
                ));
            }
            return Ok(substitute(&target, &var, &val));
        }
        let deriv = differentiate(&target, &var);
        match self.lookup(&var) {
            Expr::Symbol(s) if s == var => Ok(deriv),
            bound if !is_scalar(&bound) => Err(format!(
                "{} is bound to '{}', but a derivative can only be evaluated at a scalar",
                var, bound
            )),
            bound => Ok(substitute(&deriv, &var, &bound)),
        }
    }

    /// Split off the trailing `key = "text"` label arguments of a plot call
    /// (`title = "..."`, `xlabel = "..."`, ...). Returns the positional prefix
    /// and the evaluated labels. Labels must be string literals and must come
    /// after every positional argument; `allowed` scopes which keys this plot
    /// kind supports, so an unsupported one refuses instead of being drawn
    /// wrong or dropped silently.
    fn split_plot_labels<'a>(
        &mut self,
        args: &'a [Node],
        allowed: &[&str],
        who: &str,
    ) -> Result<(&'a [Node], Vec<(String, Expr)>), String> {
        fn label_key(n: &Node) -> Option<&str> {
            const KEYS: &[&str] = &["title", "xlabel", "ylabel", "zlabel"];
            if let Node::Equation(lhs, _) = n {
                if let Node::Ident(k) = lhs.as_ref() {
                    if KEYS.contains(&k.as_str()) {
                        return Some(k);
                    }
                }
            }
            None
        }
        let mut split = args.len();
        while split > 0 && label_key(&args[split - 1]).is_some() {
            split -= 1;
        }
        if let Some(k) = args[..split].iter().find_map(label_key) {
            return Err(format!(
                "{}: label arguments ({} = \"...\") must come after the plotted \
                 expressions and window",
                who, k
            ));
        }
        let mut labels: Vec<(String, Expr)> = Vec::with_capacity(args.len() - split);
        for n in &args[split..] {
            let Node::Equation(lhs, rhs) = n else {
                unreachable!("only label equations are split off");
            };
            let Node::Ident(key) = lhs.as_ref() else {
                unreachable!("label_key checked the lhs is an identifier");
            };
            if !allowed.contains(&key.as_str()) {
                return Err(format!(
                    "{} does not support the '{}' label (supported: {})",
                    who,
                    key,
                    allowed.join(", ")
                ));
            }
            if labels.iter().any(|(k, _)| k == key) {
                return Err(format!("{}: '{}' was given twice", who, key));
            }
            let value = self.eval_node(rhs)?;
            if !matches!(value, Expr::Str(_)) {
                return Err(format!(
                    "{}: {} must be a string, e.g. {} = \"label text\" \
                     (use $...$ inside it for LaTeX math) — got '{}'",
                    who, key, key, value
                ));
            }
            labels.push((key.clone(), value));
        }
        Ok((&args[..split], labels))
    }

    /// `plot(f1, ..., fk, x, a, b)` — one or more curves over a shared
    /// window. Stays a symbolic value; the frontend samples and draws it.
    /// `plot(s1, ..., sk)` over signals draws their samples directly.
    /// Trailing `title`/`xlabel`/`ylabel` string arguments annotate any form.
    fn call_plot(&mut self, args: &[Node]) -> Result<Expr, String> {
        let (args, labels) =
            self.split_plot_labels(args, &["title", "xlabel", "ylabel"], "plot")?;
        if !args.is_empty() && args.len() < 4 {
            let evaluated = args
                .iter()
                .map(|a| self.eval_node(a))
                .collect::<Result<Vec<_>, _>>()?;
            if evaluated.iter().all(|e| matches!(e, Expr::Signal(_))) {
                return Ok(Expr::Func(
                    "plotsignal".to_string(),
                    attach_plot_labels(evaluated, labels),
                ));
            }
            // All data series and no window: draw the points over a window
            // derived from the data (the wasm side pads the x-extent).
            if evaluated.iter().all(is_scatter) {
                return Ok(Expr::Func(
                    "plotscatter".to_string(),
                    attach_plot_labels(evaluated, labels),
                ));
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
                Expr::Function { params, body, env } => {
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
                        env,
                        vec![Expr::Symbol(var.clone())],
                        None,
                    )?
                }
                other => other,
            };
            out.push(curve);
        }
        out.push(Expr::Symbol(var));
        out.push(self.eval_node(&args[var_idx + 1])?);
        out.push(self.eval_node(&args[var_idx + 2])?);
        Ok(Expr::Func(
            "plot".to_string(),
            attach_plot_labels(out, labels),
        ))
    }

    /// `plot3d(f, x, a, b, y, c, d)` — a surface z = f(x, y) over [a, b]×[c, d].
    /// `scatter3d(x, y, z)` data may be overlaid before the window args (one
    /// surface at most), and `plot3d(scatter3d(...))` draws points alone over a
    /// window derived from the data. Stays symbolic, like `plot`.
    fn call_plot3d(&mut self, args: &[Node]) -> Result<Expr, String> {
        let (args, labels) =
            self.split_plot_labels(args, &["title", "xlabel", "ylabel", "zlabel"], "plot3d")?;
        // Bare 3D scatter: all data and no window — box it from the data.
        if !args.is_empty() && args.len() < 7 {
            let evaluated = args
                .iter()
                .map(|a| self.eval_node(a))
                .collect::<Result<Vec<_>, _>>()?;
            if evaluated.iter().all(is_scatter3d) {
                return Ok(Expr::Func(
                    "plot3dscatter".to_string(),
                    attach_plot_labels(evaluated, labels),
                ));
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
        Ok(Expr::Func(
            "plot3d".to_string(),
            attach_plot_labels(out, labels),
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

    /// `pairs(M)` / `pairs(M, [names])` / `pairs(struct)` /
    /// `pairs(struct, [fields])` — a scatterplot matrix. The data evaluates
    /// normally; the optional name list is read symbolically (see
    /// [`Self::pairs_names`]) so it can label a matrix's columns or select a
    /// subset of a struct's fields. Stays a tagged value the plot path (wasm
    /// `splom_data`) turns into a grid of panels; never sampled.
    fn call_pairs(&mut self, args: &[Node]) -> Result<Expr, String> {
        if args.is_empty() || args.len() > 2 {
            return Err(
                "pairs expects pairs(M), pairs(M, [name1, ...]), pairs(struct), or \
                 pairs(struct, [field1, ...])"
                    .into(),
            );
        }
        let data = self.eval_node(&args[0])?;
        let names = match args.get(1) {
            Some(node) => Some(self.pairs_names(node)?),
            None => None,
        };
        build_splom(&data, names)
    }

    /// The names in a `pairs` label/selection argument. A `[a, b]` literal
    /// keeps its bare identifiers symbolic (a binding of `weight` doesn't
    /// disturb a column named `weight`, matching model formulas); any other
    /// entry, or a value passed by variable, is evaluated and rendered.
    fn pairs_names(&mut self, node: &Node) -> Result<Vec<String>, String> {
        let Node::Matrix(rows) = node else {
            return splom_names(&self.eval_node(node)?);
        };
        rows.iter()
            .flatten()
            .map(|entry| match entry {
                Node::Ident(s) => Ok(s.clone()),
                other => match self.eval_node(other)? {
                    Expr::Symbol(s) => Ok(s),
                    v => Ok(format!("{}", v)),
                },
            })
            .collect()
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
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::conj(s))));
                }
                Ok(conjugate(&args[0]))
            }
            "re" | "real" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(real_part(e)));
                }
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::re_part(s))));
                }
                Ok(real_part(&args[0]))
            }
            "im" | "imag" => {
                arity(name, &args, 1)?;
                if matrix::is_matrix(&args[0]) {
                    return matrix::try_map(&args[0], |e| Ok(imag_part(e)));
                }
                if let Expr::Signal(s) = &args[0] {
                    return Ok(Expr::Signal(std::rc::Rc::new(signal::im_part(s))));
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
                    (None, Expr::Str(s)) => Ok(int(s.chars().count() as i64)),
                    _ => Err("len expects a vector, matrix, signal, or string".into()),
                }
            }
            // `str(a, b, ...)` — render every argument to its canonical
            // printed form and concatenate. One primitive covers conversion
            // and concatenation; precision comes from composing with
            // `N(x, digits)`: str("pi is ", N(pi, 5)) is "pi is 3.1416".
            // String arguments splice in bare (no quotes); everything else
            // prints exactly as the REPL would echo it.
            "str" => {
                let mut out = String::new();
                for a in &args {
                    match a {
                        Expr::Str(s) => out.push_str(s),
                        other => out.push_str(&format!("{}", other)),
                    }
                }
                Ok(Expr::Str(out))
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
                if let Some(bad) = args[..2].iter().find(|e| !is_scalar(e)) {
                    return Err(format!("linspace expects scalar endpoints, not '{}'", bad));
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
            // The STFT heatmap of a signal: a tagged drawable the frontend
            // renders (magnitudes on the plot path — the established
            // uncertified display boundary, like every other plot). Exact
            // per-frame analysis composes from slice/window/fft; exact STFT
            // of small vectors is dsp.stft.
            "spectrogram" => {
                if args.is_empty() || args.len() > 3 {
                    return Err(
                        "spectrogram expects spectrogram(s[, nfft[, hop]]) on a signal".into(),
                    );
                }
                let Expr::Signal(sig) = &args[0] else {
                    return Err(
                        "spectrogram expects a signal (pack with signal(...) or import data);                          exact per-frame spectra come from dsp.stft"
                            .into(),
                    );
                };
                let len = sig.len();
                let nfft = match args.get(1) {
                    Some(e) => numeric_value(e)
                        .filter(|r| r.is_integer())
                        .and_then(|r| r.to_integer().to_usize())
                        .filter(|&n| n.is_power_of_two() && (16..=16384).contains(&n))
                        .ok_or_else(|| {
                            "spectrogram's nfft must be a power of two between 16 and 16384"
                                .to_string()
                        })?,
                    // Default: a power of two giving a balanced picture,
                    // capped so short signals still get several frames.
                    None => {
                        let mut n = 1024usize;
                        while n > 16 && n * 4 > len {
                            n /= 2;
                        }
                        n
                    }
                };
                let hop = match args.get(2) {
                    Some(e) => numeric_value(e)
                        .filter(|r| r.is_integer())
                        .and_then(|r| r.to_integer().to_usize())
                        .filter(|&h| h >= 1)
                        .ok_or_else(|| {
                            "spectrogram's hop must be a positive integer".to_string()
                        })?,
                    None => (nfft / 4).max(1),
                };
                if len < nfft {
                    return Err(format!(
                        "spectrogram: the signal has {} samples but nfft is {} —                          pass a smaller nfft",
                        len, nfft
                    ));
                }
                Ok(Expr::Func(
                    "spectrogram".into(),
                    vec![
                        args[0].clone(),
                        Expr::Int(BigInt::from(nfft)),
                        Expr::Int(BigInt::from(hop)),
                    ],
                ))
            }
            // The k-th real root (ascending, 1-based) of a univariate
            // polynomial with rational coefficients — an exact real
            // algebraic number. The value stays symbolic (`root(p, k)`);
            // comparisons decide through the algebraic engine and N()
            // refines the isolating interval. Rational roots the isolator
            // pins exactly collapse to plain numbers.
            "root" => {
                arity(name, &args, 2)?;
                let (coeffs, _) =
                    crate::algebraic::root_call_coeffs(&args[0]).ok_or_else(|| {
                        "root expects a polynomial in one free symbol with rational \
                         coefficients, e.g. root(x^3 - 2, 1)"
                            .to_string()
                    })?;
                if coeffs.len() < 2 {
                    return Err("root expects a polynomial of degree at least 1".into());
                }
                let k = numeric_value(&args[1])
                    .filter(|k| k.is_integer() && *k >= BigRational::from_integer(1.into()))
                    .and_then(|k| k.to_integer().to_usize())
                    .ok_or_else(|| {
                        "root's second argument is the root index: a positive integer".to_string()
                    })?;
                let count =
                    crate::algebraic::RealAlg::real_root_count(&coeffs).ok_or_else(|| {
                        "this polynomial is beyond the algebraic engine's degree/size caps"
                            .to_string()
                    })?;
                if k > count {
                    return Err(match count {
                        0 => "the polynomial has no real roots".to_string(),
                        1 => format!(
                            "the polynomial has only 1 real root, index {k} is out of range"
                        ),
                        n => format!(
                            "the polynomial has only {n} real roots, index {k} is out of range"
                        ),
                    });
                }
                match crate::algebraic::RealAlg::nth_root_of(&coeffs, k) {
                    Some(crate::algebraic::RealAlg::Rational(r)) => Ok(rat_to_expr(r)),
                    _ => Ok(Expr::Func(
                        "root".into(),
                        vec![args[0].clone(), args[1].clone()],
                    )),
                }
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
            // ("fill" never reaches here — a function argument needs &mut self to
            // call back into, so it's intercepted in `call` like `map`.)
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
            for arg in idx {
                match arg {
                    IndexArg::Scalar(n) => collect_node_idents(n, out),
                    IndexArg::Range { lo, hi, step } => {
                        for n in [lo, hi].into_iter().flatten() {
                            collect_node_idents(n, out);
                        }
                        match step {
                            None => {}
                            Some(Step::By(k)) => collect_node_idents(k, out),
                            Some(Step::TakeSkip(t, s)) => {
                                collect_node_idents(t, out);
                                collect_node_idents(s, out);
                            }
                        }
                    }
                }
            }
        }
        Node::Matrix(rows) => rows
            .iter()
            .flatten()
            .for_each(|c| collect_node_idents(c, out)),
        _ => {}
    }
}

/// Every name a function body could *read* — used to decide what a lambda (or
/// a locally defined function) captures. Unlike [`collect_node_idents`], this
/// traverses all statement forms and includes call targets (`f(x)` reads `f`),
/// because a captured local may well be a function value. Over-collection is
/// harmless: a name that turns out to be bound by a parameter or a local
/// assignment at call time simply shadows its captured snapshot.
fn collect_capture_names(node: &Node, out: &mut Vec<String>) {
    fn push(s: &String, out: &mut Vec<String>) {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    match node {
        Node::Num(_) | Node::Str(_) => {}
        Node::Ident(s) => push(s, out),
        Node::BinOp(_, a, b) | Node::Equation(a, b) | Node::Formula(a, b) => {
            collect_capture_names(a, out);
            collect_capture_names(b, out);
        }
        Node::Neg(a) | Node::Not(a) | Node::Assign(_, a) => collect_capture_names(a, out),
        Node::Call(name, args) => {
            push(name, out);
            args.iter().for_each(|a| collect_capture_names(a, out));
        }
        Node::Field(b, _) => collect_capture_names(b, out),
        Node::FieldCall(b, _, args) => {
            collect_capture_names(b, out);
            args.iter().for_each(|a| collect_capture_names(a, out));
        }
        Node::Index(b, idx) => {
            collect_capture_names(b, out);
            for arg in idx {
                match arg {
                    IndexArg::Scalar(n) => collect_capture_names(n, out),
                    IndexArg::Range { lo, hi, step } => {
                        for n in [lo, hi].into_iter().flatten() {
                            collect_capture_names(n, out);
                        }
                        match step {
                            None => {}
                            Some(Step::By(k)) => collect_capture_names(k, out),
                            Some(Step::TakeSkip(t, s)) => {
                                collect_capture_names(t, out);
                                collect_capture_names(s, out);
                            }
                        }
                    }
                }
            }
        }
        Node::Matrix(rows) => rows
            .iter()
            .flatten()
            .for_each(|c| collect_capture_names(c, out)),
        Node::If(c, t, e) => {
            collect_capture_names(c, out);
            collect_capture_names(t, out);
            if let Some(e) = e {
                collect_capture_names(e, out);
            }
        }
        Node::While(c, b) => {
            collect_capture_names(c, out);
            collect_capture_names(b, out);
        }
        Node::For { iter, body, .. } => {
            match iter {
                ForIter::Range { lo, step, hi } => {
                    collect_capture_names(lo, out);
                    if let Some(s) = step {
                        collect_capture_names(s, out);
                    }
                    collect_capture_names(hi, out);
                }
                ForIter::Expr(e) => collect_capture_names(e, out),
            }
            collect_capture_names(body, out);
        }
        Node::FuncDef(_, _, body) | Node::Lambda(_, body) => collect_capture_names(body, out),
        Node::Block(stmts) => stmts.iter().for_each(|s| collect_capture_names(s, out)),
    }
}

/// A callable first argument of a higher-order builtin: a function value, or
/// a bare name (an unbound symbol) that will dispatch to a builtin.
fn check_callable(who: &str, f: &Expr) -> Result<(), String> {
    match f {
        Expr::Function { .. } | Expr::Symbol(_) => Ok(()),
        other => Err(format!(
            "{} expects a function as its first argument, got '{}'",
            who, other
        )),
    }
}

/// A `for`-range endpoint or step, which must be an exact number: the loop's
/// stopping comparison has to be decidable, and exact rationals keep it so.
fn exact_bound(e: &Expr) -> Result<BigRational, String> {
    numeric_value(e).ok_or_else(|| {
        format!(
            "for: range bounds and step must be exact numbers (integers or rationals), \
             got '{}'",
            e
        )
    })
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

/// Index or slice a signal. A scalar reads the midpoint of that sample (the
/// certified half-width is `bound(s, i)`); a range cuts a sub-signal, staying
/// inside the signal substrate.
fn index_signal(s: &Rc<signal::SignalData>, sels: &[matrix::Sel]) -> Result<Expr, String> {
    let [sel] = sels else {
        return Err("a signal takes a single index".into());
    };
    let len = s.len();
    match *sel {
        matrix::Sel::One(i) => {
            if !(1..=len).contains(&i) {
                return Err(format!(
                    "index {} is out of range (the signal has {})",
                    i, len
                ));
            }
            Ok(signal::midpoint(s, i - 1))
        }
        matrix::Sel::Range { lo, hi, take, skip } => {
            let (lo, hi) = (lo.unwrap_or(1), hi.unwrap_or(len));
            if lo < 1 || hi > len || lo > hi {
                return Err(format!(
                    "range {}:{} is out of range (the signal has {})",
                    lo, hi, len
                ));
            }
            // A contiguous range keeps the fast slice path; a stride or
            // take/skip gathers the selected samples into a new sub-signal.
            let sub = if take == 1 && skip == 0 {
                signal::slice(s, lo - 1, hi - lo + 1)?
            } else {
                let idx0: Vec<usize> = matrix::strided_indices(sel, len, "the signal")?
                    .into_iter()
                    .map(|i| i - 1)
                    .collect();
                signal::gather(s, &idx0)?
            };
            Ok(Expr::Signal(Rc::new(sub)))
        }
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

/// A stride / take count: a positive integer. `what` names it for errors.
fn as_positive(e: &Expr, what: &str) -> Result<usize, String> {
    match as_usize(e) {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("{} must be a positive integer, got '{}'", what, e)),
    }
}

/// A skip count: a non-negative integer. `what` names it for errors.
fn as_count(e: &Expr, what: &str) -> Result<usize, String> {
    as_usize(e).map_err(|_| format!("{} must be a non-negative integer, got '{}'", what, e))
}

/// Linear-algebra dispatch for binary operators when a matrix is involved.
fn matrix_binop(op: Op, x: Expr, y: Expr) -> Result<Expr, String> {
    let (xm, ym) = (matrix::is_matrix(&x), matrix::is_matrix(&y));
    match op {
        Op::Add | Op::Sub => {
            let subtract = matches!(op, Op::Sub);
            if xm && ym {
                matrix::mat_add(&x, &y, subtract)
            } else if xm {
                Ok(matrix::scalar_add(&y, &x, subtract, true))
            } else {
                Ok(matrix::scalar_add(&x, &y, subtract, false))
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
/// participate); by exact algebra when both sides are constant real algebraic
/// expressions (`(sqrt(2)+sqrt(3))^2 == 5+2*sqrt(6)` is `true`, not a
/// structural mismatch).
///
/// For *constant* expressions the fall-through is decided by the same
/// certified machinery ordering uses: interval refinement can prove ≠
/// (disjoint enclosures), exact algebra can prove =, and when neither can,
/// the comparison refuses (`Err`) rather than defaulting to `false` — a
/// certified `==` must never assert a false disequality (`dsp.idft(dsp.dft(v))`
/// entries *are* the input, whether or not the prover can see it).
/// Expressions with free symbols keep structural semantics (`sin(x) == cos(x)`
/// is `false`, not an error: with unknowns the question is shape, not value).
fn value_eq(x: &Expr, y: &Expr) -> Result<bool, String> {
    if let (Some(p), Some(q)) = (comparable_value(x), comparable_value(y)) {
        return Ok(p == q);
    }
    if x == y {
        return Ok(true);
    }
    // Matrices and complex values compare componentwise, so exact algebraic
    // equality reaches inside them (a frequency-response vector equal to [1]
    // entry-by-entry IS equal). Three-valued combine: one certain mismatch
    // decides ≠ even if another entry refuses; a refusal only propagates when
    // nothing settled the answer first.
    match (x, y) {
        (Expr::Matrix(a), Expr::Matrix(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            let mut refusal = None;
            for (ra, rb) in a.iter().zip(b) {
                if ra.len() != rb.len() {
                    return Ok(false);
                }
                for (ea, eb) in ra.iter().zip(rb) {
                    match value_eq(ea, eb) {
                        Ok(false) => return Ok(false),
                        Ok(true) => {}
                        Err(e) => refusal = Some(e),
                    }
                }
            }
            return match refusal {
                None => Ok(true),
                Some(e) => Err(e),
            };
        }
        (Expr::Complex(ar, ai), Expr::Complex(br, bi)) => {
            return combine_eq(value_eq(ar, br), value_eq(ai, bi));
        }
        // Complex against a real scalar: equal iff the real parts are and the
        // imaginary part is exactly zero. A surviving Complex usually has a
        // nonzero-but-unproven imaginary part, so this refuses rather than
        // wrongly answering `false`.
        (Expr::Complex(ar, ai), b) | (b, Expr::Complex(ar, ai)) if is_scalar(b) => {
            return combine_eq(value_eq(ar, b), value_eq(ai, &int(0)));
        }
        _ => {}
    }
    if let (Some(a), Some(b)) = (
        crate::algebraic::from_expr(x),
        crate::algebraic::from_expr(y),
    ) {
        return Ok(a.cmp_alg(&b) == std::cmp::Ordering::Equal);
    }
    // Opaque values (functions, bools, equations, signals, …) can't enter the
    // difference construction below — the smart constructors would build
    // well-sorted nonsense around them. Their equality stays structural.
    if !is_scalar(x) || !is_scalar(y) {
        return Ok(false);
    }
    let d = add(vec![x.clone(), mul(vec![int(-1), y.clone()])]);
    if let Some(r) = comparable_value(&d) {
        return Ok(r.is_zero());
    }
    match interval::certified_sign(&d) {
        interval::Sign::Positive | interval::Sign::Negative => Ok(false),
        interval::Sign::Zero => Ok(true),
        // Not a constant real expression (free symbols, complex values):
        // structural semantics, exactly the pre-certified behavior.
        interval::Sign::Unsupported => Ok(false),
        // Enclosures touched or straddled zero at the ceiling: refinement can
        // never prove equality — algebra can, when the difference is a real
        // algebraic number. π/e ties refuse honestly.
        _ => match crate::algebraic::certified_sign(&d) {
            Some(ord) => Ok(ord == std::cmp::Ordering::Equal),
            None => Err(format!(
                "cannot decide '{}' == '{}': they agree to at least {} significant digits — \
                 the values may be equal",
                x,
                y,
                interval::max_digits()
            )),
        },
    }
}

/// Is this error an honest *refusal* — the engine declining to certify an
/// answer it cannot prove ("the values may be equal") — rather than a user or
/// domain error? Refusals are the product working as designed; the UI renders
/// them as a distinct outcome, not a failure. Every refusal constructed here
/// (`value_eq`, `compare`, and the algebraic-cap fall-throughs that funnel
/// into them) carries this phrase — keep them in step.
pub fn is_refusal(msg: &str) -> bool {
    msg.contains("may be equal")
}

/// Three-valued AND for equality verdicts: any certain ≠ decides, both
/// certain = decide, otherwise the refusal wins.
fn combine_eq(a: Result<bool, String>, b: Result<bool, String>) -> Result<bool, String> {
    match (a, b) {
        (Ok(false), _) | (_, Ok(false)) => Ok(false),
        (Ok(true), Ok(true)) => Ok(true),
        (Err(e), _) | (_, Err(e)) => Err(e),
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
    // reach the difference construction below. Complex values have no order
    // even when equal: without this, `sqrt(-2) <= sqrt(-2)` answered `true`
    // via exact cancellation while `I < 2*I` refused.
    let unorderable = |e: &Expr| {
        is_opaque_value(e)
            || matrix::is_matrix(e)
            || matches!(e, Expr::Equation(..) | Expr::Signal(_) | Expr::Complex(..))
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
            _ => match crate::algebraic::certified_sign(&d) {
                Some(ord) => decide(ord),
                None => inseparable(),
            },
        },
        interval::Sign::NonPositive => match op {
            Op::LessEq => Ok(Expr::Bool(true)),
            Op::Greater => Ok(Expr::Bool(false)),
            _ => match crate::algebraic::certified_sign(&d) {
                Some(ord) => decide(ord),
                None => inseparable(),
            },
        },
        // Refinement could not separate the values — the one case bits can
        // never settle is *equality*, which algebra can: if the difference
        // is a real algebraic number (radicals, roots, trig of rational
        // multiples of π — no π/e themselves), its sign is decidable
        // exactly. π/e ties still refuse honestly.
        interval::Sign::Inseparable => match crate::algebraic::certified_sign(&d) {
            Some(ord) => decide(ord),
            None => inseparable(),
        },
        interval::Sign::Unsupported => match crate::algebraic::certified_sign(&d) {
            // e.g. root(...) expressions the interval evaluator refuses.
            Some(ord) => decide(ord),
            None => Err(format!(
                "cannot order '{}' and '{}'; both must be constant real values \
                 (a free symbol has no fixed value — try subs(...) or N(...))",
                x, y
            )),
        },
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
    matches!(
        e,
        Expr::Bool(_) | Expr::Str(_) | Expr::Function { .. } | Expr::Struct(_)
    )
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
            e if !is_scalar(e) => return Err(format!("{} cannot include '{}'", name, e)),
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

/// Cap on variables in a scatterplot matrix — beyond this the k×k grid of
/// panels is too small to read. Mirrored by `SPLOM_VARS_MAX` in the wasm bridge.
const MAX_SPLOM_VARS: usize = 10;

/// Build the tagged `splom` value for `pairs(...)`: the data as an n×k matrix
/// (columns are variables) followed by one symbol per column label. The wasm
/// `splom_data` turns it into the grid of panels; here we only validate shapes
/// and resolve the labels. `names`, when given, labels a matrix's columns or
/// *selects* a subset of a struct's fields (by name, in the order given).
fn build_splom(data: &Expr, names: Option<Vec<String>>) -> Result<Expr, String> {
    let (matrix_data, labels) = match data {
        Expr::Struct(fields) => splom_from_struct(fields, names.as_deref())?,
        Expr::Matrix(rows) => {
            if rows.len() < 2 {
                return Err("pairs needs at least 2 observations (rows)".into());
            }
            let ncols = rows[0].len();
            let labels = match names {
                Some(n) if n.len() != ncols => {
                    return Err(format!(
                        "pairs: got {} label(s) for {} column(s)",
                        n.len(),
                        ncols
                    ));
                }
                Some(n) => n,
                None => (1..=ncols).map(|i| format!("x{}", i)).collect(),
            };
            (data.clone(), labels)
        }
        _ => {
            return Err(
                "pairs expects a data matrix (columns are variables) or a struct of \
                 equal-length columns"
                    .into(),
            )
        }
    };
    let k = labels.len();
    if k < 2 {
        return Err("pairs needs at least 2 variables (columns) to form a matrix of panels".into());
    }
    if k > MAX_SPLOM_VARS {
        return Err(format!(
            "pairs: too many variables ({}, max {}) — select fewer columns",
            k, MAX_SPLOM_VARS
        ));
    }
    let mut out = Vec::with_capacity(k + 1);
    out.push(matrix_data);
    out.extend(labels.into_iter().map(Expr::Symbol));
    Ok(Expr::Func("splom".to_string(), out))
}

/// Assemble (data matrix, labels) from a struct of equal-length numeric column
/// vectors. With `selection`, take exactly those fields, in that order (each
/// must exist and be numeric). Without it, take every numeric field in the
/// struct's canonical (alphabetical) order, skipping non-numeric ones (e.g. a
/// categorical column) the way data-frame pair plots use only numeric columns.
fn splom_from_struct(
    fields: &[(String, Expr)],
    selection: Option<&[String]>,
) -> Result<(Expr, Vec<String>), String> {
    let mut labels = Vec::new();
    let mut columns: Vec<Vec<Expr>> = Vec::new();
    match selection {
        Some(names) => {
            for name in names {
                let field = fields
                    .iter()
                    .find(|(f, _)| f == name)
                    .ok_or_else(|| format!("pairs: struct has no field '{}'", name))?;
                let col = numeric_column(&field.1)
                    .ok_or_else(|| format!("pairs: field '{}' is not a numeric column", name))?;
                labels.push(name.clone());
                columns.push(col);
            }
        }
        None => {
            for (name, value) in fields {
                if let Some(col) = numeric_column(value) {
                    labels.push(name.clone());
                    columns.push(col);
                }
            }
        }
    }
    let n = columns
        .first()
        .map(Vec::len)
        .ok_or("pairs(struct): no numeric columns to plot")?;
    for (name, col) in labels.iter().zip(&columns) {
        if col.len() != n {
            return Err(format!(
                "pairs(struct): column '{}' has {} value(s) but other columns have {}",
                name,
                col.len(),
                n
            ));
        }
    }
    // Columns → an n×k row-major matrix (columns are variables).
    let rows: Vec<Vec<Expr>> = (0..n)
        .map(|r| columns.iter().map(|c| c[r].clone()).collect())
        .collect();
    Ok((matrix::matrix(rows)?, labels))
}

/// Read column names from a `pairs` argument that has already evaluated to a
/// vector — symbols pass through, anything else renders to text. (The literal
/// `[a, b]` form keeps its names symbolic via `Interp::pairs_names`.)
fn splom_names(e: &Expr) -> Result<Vec<String>, String> {
    Ok(vector_entries("pairs", e)?
        .iter()
        .map(|x| match x {
            Expr::Symbol(s) => s.clone(),
            other => format!("{}", other),
        })
        .collect())
}

/// A struct field read as a vector of all-numeric entries, or `None` if it
/// isn't a vector or holds a non-number (a symbol, an equation, …).
fn numeric_column(e: &Expr) -> Option<Vec<Expr>> {
    let v = matrix::vector_of(e)?;
    v.iter()
        .all(|x| matches!(x, Expr::Int(_) | Expr::Rat(_) | Expr::Float(..)))
        .then_some(v)
}

/// Append evaluated plot labels to a tagged plot value's arguments, each as a
/// `key = "text"` equation. Trailing equations are how the labels survive in
/// the symbolic value and its printed form (`plot(..., title = "...")`
/// re-parses to the same value); the wasm extractor peels them back off.
fn attach_plot_labels(mut out: Vec<Expr>, labels: Vec<(String, Expr)>) -> Vec<Expr> {
    for (key, value) in labels {
        out.push(Expr::Equation(Box::new(Expr::Symbol(key)), Box::new(value)));
    }
    out
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
        env: Vec::new(),
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
