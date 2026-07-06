//! The parse tree. Distinct from [`crate::expr::Expr`]: this is the literal
//! syntactic structure the parser produces; the evaluator lowers it into
//! canonical `Expr` values.
//!
//! `PartialEq` is derived so that function bodies (stored in an `Expr::Function`
//! as `Rc<Node>`) can participate in `Expr` equality.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Num(String),
    /// A string literal (already unescaped). Inert data — used for plot
    /// titles and axis labels, not computed with.
    Str(String),
    Ident(String),
    BinOp(Op, Box<Node>, Box<Node>),
    Neg(Box<Node>),
    /// Logical negation: `not x`.
    Not(Box<Node>),
    Call(String, Vec<Node>),
    /// Struct field access: `base.name`.
    Field(Box<Node>, String),
    /// A namespaced call: `base.name(args)`. `base` evaluates to a struct
    /// whose `name` field is a function (a user module), or names a built-in
    /// namespace like `dsp`.
    FieldCall(Box<Node>, String, Vec<Node>),
    /// Indexing, 1-based: `v[i]` (vector element or matrix row), `m[i, j]`
    /// (matrix element). Each argument is a scalar or a range (`a:b`, `a:`,
    /// `:b`, `:`, with an optional stride `a:s:b`) — a scalar collapses its
    /// axis, a range keeps it.
    Index(Box<Node>, Vec<IndexArg>),
    /// A matrix literal, rows of cells: `[1, 2; 3, 4]`.
    Matrix(Vec<Vec<Node>>),
    /// `name := rhs`
    Assign(String, Box<Node>),
    /// `lhs = rhs` — an equation object, never a boolean.
    Equation(Box<Node>, Box<Node>),
    /// `response ~ terms` — a model formula (column names kept symbolic).
    Formula(Box<Node>, Box<Node>),
    /// `if cond then <block> [else <block>] end` — an expression whose value is
    /// the taken branch.
    If(Box<Node>, Box<Node>, Option<Box<Node>>),
    /// `while cond do <block> end`
    While(Box<Node>, Box<Node>),
    /// A function definition: name, parameters, body block.
    FuncDef(String, Vec<String>, Box<Node>),
    /// A sequence of statements; its value is the last one's.
    Block(Vec<Node>),
}

/// One argument of an [`Node::Index`]. `Scalar` selects a single position and
/// collapses that axis; `Range` keeps the axis, with either bound left open
/// (`None`) to mean "to the start/end of the axis", and an optional stride
/// (`None` = every element).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum IndexArg {
    Scalar(Node),
    Range {
        lo: Option<Box<Node>>,
        hi: Option<Box<Node>>,
        step: Option<Step>,
    },
}

/// The stride between kept positions in a range `lo:step:hi` (MATLAB/Julia
/// order — the step sits in the middle). A scalar `By(k)` keeps every k-th
/// position; a `TakeSkip(t, s)` keeps `t` consecutive then skips `s`, repeating.
/// `By(k)` is the special case `TakeSkip(1, k - 1)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Step {
    By(Box<Node>),
    TakeSkip(Box<Node>, Box<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    // Elementwise (`.*`, `./`, `.^`): entrywise on matrices, and the plain
    // operation on scalars.
    ElemMul,
    ElemDiv,
    ElemPow,
    // Comparisons and logic produce boolean values.
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    And,
    Or,
}
