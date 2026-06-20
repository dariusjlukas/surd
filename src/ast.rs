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
    /// (matrix element).
    Index(Box<Node>, Vec<Node>),
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
