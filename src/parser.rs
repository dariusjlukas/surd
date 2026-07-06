//! Recursive-descent parser.
//!
//! A program is a block: statements separated by newlines or `;`. Precedence,
//! loosest to tightest:
//!   equation `=`  →  `or`  →  `and`  →  `not`  →  comparison
//!   →  `+ -`  →  `* /`  →  unary `-`  →  `^` (right-assoc)  →  atom
//!
//! `if`/`while` are atoms (so they can be used as values), and blocks delimited
//! by `then`/`do`/`else`/`end`. Block keywords and `and`/`or`/`not` are plain
//! identifiers recognized here.

use crate::ast::{IndexArg, Node, Op, Step};
use crate::lexer::Token;

/// The parsed middle field of a range, before we know whether it is a stride
/// (three-field `lo:step:hi`) or an upper bound (two-field `lo:hi`). Only a
/// stride may take the `(take, skip)` pair shape.
enum StepField {
    Expr(Box<Node>),
    Pair(Box<Node>, Box<Node>),
}

impl StepField {
    fn into_step(self) -> Step {
        match self {
            StepField::Expr(e) => Step::By(e),
            StepField::Pair(t, s) => Step::TakeSkip(t, s),
        }
    }
}

/// Bound on nesting depth, so pathologically nested input errors gracefully
/// instead of overflowing the stack during recursive-descent parsing.
const MAX_DEPTH: usize = 512;

/// Bound on total tokens. This caps the size (and so the depth) of the AST,
/// which matters because dropping a very deeply nested tree itself recurses and
/// could overflow the stack — even for a flat `1+1+...+1` chain the parser
/// builds iteratively.
const MAX_TOKENS: usize = 15_000;

pub fn parse(tokens: Vec<Token>) -> Result<Node, String> {
    if tokens.len() > MAX_TOKENS {
        return Err("input is too large".to_string());
    }
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let program = p.parse_block(&[], &[])?;
    p.expect(Token::Eof)?;
    Ok(program)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        // Saturate at the final token (always Eof), so an over-advance can
        // never index out of bounds.
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Token) -> bool {
        if self.peek() == t {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Token) -> Result<(), String> {
        if self.eat(&t) {
            Ok(())
        } else {
            Err(format!("expected {:?}, found {:?}", t, self.peek()))
        }
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Token::Ident(s) if s == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> Result<(), String> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(format!("expected '{}', found {:?}", kw, self.peek()))
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            other => Err(format!("expected a name, found {:?}", other)),
        }
    }

    fn is_separator(&self) -> bool {
        matches!(self.peek(), Token::Newline | Token::Semicolon)
    }

    fn skip_separators(&mut self) {
        while self.is_separator() {
            self.pos += 1;
        }
    }

    // -- blocks --------------------------------------------------------------

    /// Parse a sequence of statements, stopping (without consuming) at EOF, any
    /// keyword in `stop_kws`, or any token in `stop_tokens`.
    fn parse_block(&mut self, stop_kws: &[&str], stop_tokens: &[Token]) -> Result<Node, String> {
        let mut stmts = Vec::new();
        self.skip_separators();
        while !self.at_block_end(stop_kws, stop_tokens) {
            stmts.push(self.parse_statement()?);
            if self.at_block_end(stop_kws, stop_tokens) {
                break;
            }
            if !self.is_separator() {
                return Err(format!(
                    "expected a newline, ';', or end of block, found {:?}",
                    self.peek()
                ));
            }
            self.skip_separators();
        }
        Ok(Node::Block(stmts))
    }

    fn at_block_end(&self, stop_kws: &[&str], stop_tokens: &[Token]) -> bool {
        if self.peek() == &Token::Eof {
            return true;
        }
        if let Token::Ident(s) = self.peek() {
            if stop_kws.contains(&s.as_str()) {
                return true;
            }
        }
        stop_tokens.contains(self.peek())
    }

    // -- statements ----------------------------------------------------------

    fn parse_statement(&mut self) -> Result<Node, String> {
        if self.is_kw("function") {
            return self.parse_function_def();
        }
        if let Token::Ident(name) = self.peek().clone() {
            // plain assignment:  name := expr
            if self.at(1) == Some(&Token::Assign) {
                self.pos += 2;
                let rhs = self.parse_expr()?;
                return Ok(Node::Assign(name, Box::new(rhs)));
            }
            // function shorthand:  name(params) := expr
            if self.at(1) == Some(&Token::LParen) {
                if let Some(def) = self.try_function_shorthand(&name)? {
                    return Ok(def);
                }
            }
        }
        self.parse_expr()
    }

    fn parse_function_def(&mut self) -> Result<Node, String> {
        self.expect_kw("function")?;
        let name = self.expect_ident()?;
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;
        let body = self.parse_block(&["end"], &[])?;
        self.expect_kw("end")?;
        Ok(Node::FuncDef(name, params, Box::new(body)))
    }

    /// If `name(...)` is immediately followed by `:=`, it's a single-expression
    /// function definition; otherwise it's a call and we leave it for `parse_expr`.
    fn try_function_shorthand(&mut self, name: &str) -> Result<Option<Node>, String> {
        // Find the `)` matching the `(` at self.pos+1.
        let mut i = self.pos + 2;
        let mut depth = 1;
        while i < self.tokens.len() && depth > 0 {
            match &self.tokens[i] {
                Token::LParen => depth += 1,
                Token::RParen => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            i += 1;
        }
        if self.tokens.get(i + 1) != Some(&Token::Assign) {
            return Ok(None);
        }
        self.pos += 2; // consume name and '('
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;
        self.expect(Token::Assign)?;
        let body = self.parse_expr()?;
        Ok(Some(Node::FuncDef(
            name.to_string(),
            params,
            Box::new(body),
        )))
    }

    fn parse_params(&mut self) -> Result<Vec<String>, String> {
        let mut params = Vec::new();
        if self.peek() == &Token::RParen {
            return Ok(params);
        }
        loop {
            params.push(self.expect_ident()?);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        Ok(params)
    }

    // -- expressions ---------------------------------------------------------

    /// Top of the expression grammar: an equation, or just a value. Every level
    /// of nesting (parens, calls, blocks, control flow) routes through here, so
    /// this is where we bound recursion depth.
    fn parse_expr(&mut self) -> Result<Node, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("expression is nested too deeply".to_string());
        }
        let result = self.parse_expr_inner();
        self.depth -= 1;
        result
    }

    fn parse_expr_inner(&mut self) -> Result<Node, String> {
        let left = self.parse_formula()?;
        if self.eat(&Token::Eq) {
            let right = self.parse_formula()?;
            Ok(Node::Equation(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    /// `response ~ terms` — a model formula, just inside `=` in precedence and
    /// non-associative (one tilde), so `y ~ a + b` is `y ~ (a + b)`.
    fn parse_formula(&mut self) -> Result<Node, String> {
        let left = self.parse_or()?;
        if self.eat(&Token::Tilde) {
            let right = self.parse_or()?;
            Ok(Node::Formula(Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    fn parse_or(&mut self) -> Result<Node, String> {
        let mut left = self.parse_and()?;
        while self.eat_kw("or") {
            let right = self.parse_and()?;
            left = Node::BinOp(Op::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Node, String> {
        let mut left = self.parse_not()?;
        while self.eat_kw("and") {
            let right = self.parse_not()?;
            left = Node::BinOp(Op::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Node, String> {
        if self.eat_kw("not") {
            Ok(Node::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> Result<Node, String> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            Token::EqEq => Op::Equal,
            Token::Ne => Op::NotEqual,
            Token::Lt => Op::Less,
            Token::Gt => Op::Greater,
            Token::Le => Op::LessEq,
            Token::Ge => Op::GreaterEq,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.parse_add()?;
        Ok(Node::BinOp(op, Box::new(left), Box::new(right)))
    }

    fn parse_add(&mut self) -> Result<Node, String> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Token::Plus => Op::Add,
                Token::Minus => Op::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Node::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Node, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => Op::Mul,
                Token::Slash => Op::Div,
                Token::DotStar => Op::ElemMul,
                Token::DotSlash => Op::ElemDiv,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Node::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Node, String> {
        if self.eat(&Token::Minus) {
            Ok(Node::Neg(Box::new(self.parse_unary()?)))
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Result<Node, String> {
        let base = self.parse_postfix()?;
        let op = match self.peek() {
            Token::Caret => Op::Pow,
            Token::DotCaret => Op::ElemPow,
            _ => return Ok(base),
        };
        self.pos += 1;
        let exp = self.parse_unary()?; // right-assoc; exponent may be negative
        Ok(Node::BinOp(op, Box::new(base), Box::new(exp)))
    }

    /// Postfix operators bind tighter than `^`: `s.a^2` is `(s.a)^2`,
    /// `v[1]^2` is `(v[1])^2`. Chains freely: `data.samples[3]`. A field
    /// followed by `(` is a namespaced call: `dsp.dft(v)`, `mylib.f(x)`.
    fn parse_postfix(&mut self) -> Result<Node, String> {
        let mut node = self.parse_atom()?;
        loop {
            if self.eat(&Token::Dot) {
                let name = self.expect_ident()?;
                node = if self.eat(&Token::LParen) {
                    Node::FieldCall(Box::new(node), name, self.parse_args()?)
                } else {
                    Node::Field(Box::new(node), name)
                };
            } else if self.eat(&Token::LBracket) {
                let mut idxs = vec![self.parse_index_arg()?];
                while self.eat(&Token::Comma) {
                    idxs.push(self.parse_index_arg()?);
                }
                self.expect(Token::RBracket)?;
                node = Node::Index(Box::new(node), idxs);
            } else {
                return Ok(node);
            }
        }
    }

    /// A call's argument list; the opening `(` was consumed.
    fn parse_args(&mut self) -> Result<Vec<Node>, String> {
        let mut args = Vec::new();
        if self.peek() != &Token::RParen {
            loop {
                args.push(self.parse_expr()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        Ok(args)
    }

    /// One comma-separated index argument: a scalar `e`, or a range with up to
    /// three colon-separated fields — `lo:hi`, `lo:step:hi`, with any bound
    /// omitted (`lo:`, `:hi`, `:`, `lo:step:`, …). In the three-field form the
    /// middle field is the stride (MATLAB/Julia order): a scalar `k` keeps every
    /// k-th position, or a `(take, skip)` pair keeps `take` then skips `skip`.
    /// The `:` is bracket-local — it never participates in ordinary expression
    /// precedence.
    fn parse_index_arg(&mut self) -> Result<IndexArg, String> {
        // First field (`lo`), possibly empty when the arg leads with `:`.
        let lo = if self.peek() == &Token::Colon {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        if !self.eat(&Token::Colon) {
            // No colon at all → a scalar index (`lo` is necessarily present).
            return Ok(IndexArg::Scalar(*lo.unwrap()));
        }
        // We have a range. The second field is `hi` (two-field form) or the
        // stride (three-field form) — we only learn which after looking for a
        // third colon, so allow the `(take, skip)` pair shape here.
        let second = if self.at_index_end() || self.peek() == &Token::Colon {
            None
        } else {
            Some(self.parse_step_field()?)
        };
        if self.eat(&Token::Colon) {
            // Three fields: the second was the stride, the third is `hi`.
            let hi = if self.at_index_end() {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            };
            return Ok(IndexArg::Range {
                lo,
                hi,
                step: second.map(StepField::into_step),
            });
        }
        // Two fields: the second is `hi`, which may not be a `(take, skip)` pair.
        let hi = match second {
            None => None,
            Some(StepField::Expr(e)) => Some(e),
            Some(StepField::Pair(..)) => {
                return Err("a (take, skip) pair is only valid as the stride, \
                            as in lo:(take, skip):hi"
                    .into())
            }
        };
        Ok(IndexArg::Range { lo, hi, step: None })
    }

    /// The middle field of a range. A leading `(` introduces either a
    /// `(take, skip)` pair (a comma follows the first expression) or an ordinary
    /// parenthesized expression used as a scalar stride; anything else is a bare
    /// scalar-stride expression.
    fn parse_step_field(&mut self) -> Result<StepField, String> {
        if self.eat(&Token::LParen) {
            let first = self.parse_expr()?;
            if self.eat(&Token::Comma) {
                let second = self.parse_expr()?;
                self.expect(Token::RParen)?;
                return Ok(StepField::Pair(Box::new(first), Box::new(second)));
            }
            self.expect(Token::RParen)?;
            return Ok(StepField::Expr(Box::new(first)));
        }
        Ok(StepField::Expr(Box::new(self.parse_expr()?)))
    }

    /// True at the close of an index argument — a comma or the closing `]`.
    fn at_index_end(&self) -> bool {
        matches!(self.peek(), Token::Comma | Token::RBracket)
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        // Keyword-led primaries.
        if self.is_kw("if") {
            return self.parse_if();
        }
        if self.is_kw("while") {
            return self.parse_while();
        }

        match self.advance() {
            Token::Num(s) => Ok(Node::Num(s)),
            Token::Str(s) => Ok(Node::Str(s)),
            Token::Ident(name) => {
                if self.eat(&Token::LParen) {
                    Ok(Node::Call(name, self.parse_args()?))
                } else {
                    Ok(Node::Ident(name))
                }
            }
            // A parenthesized group is a one-or-more-statement block.
            Token::LParen => {
                let inner = self.parse_block(&[], &[Token::RParen])?;
                self.expect(Token::RParen)?;
                Ok(unwrap_block(inner))
            }
            Token::LBracket => self.parse_matrix(),
            other => Err(format!("unexpected token {:?}", other)),
        }
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        self.expect_kw("if")?;
        let cond = self.parse_expr()?;
        self.expect_kw("then")?;
        let then_block = self.parse_block(&["else", "end"], &[])?;
        let else_block = if self.eat_kw("else") {
            Some(Box::new(self.parse_block(&["end"], &[])?))
        } else {
            None
        };
        self.expect_kw("end")?;
        Ok(Node::If(Box::new(cond), Box::new(then_block), else_block))
    }

    fn parse_while(&mut self) -> Result<Node, String> {
        self.expect_kw("while")?;
        let cond = self.parse_expr()?;
        self.expect_kw("do")?;
        let body = self.parse_block(&["end"], &[])?;
        self.expect_kw("end")?;
        Ok(Node::While(Box::new(cond), Box::new(body)))
    }

    /// Matrix literal: `[ row (';' row)* ]`. The opening `[` was consumed.
    fn parse_matrix(&mut self) -> Result<Node, String> {
        if self.eat(&Token::RBracket) {
            return Err("empty matrix '[]' is not allowed".into());
        }
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut row: Vec<Node> = Vec::new();
        loop {
            row.push(self.parse_add()?);
            match self.peek() {
                Token::Comma => {
                    self.pos += 1;
                }
                Token::Semicolon => {
                    self.pos += 1;
                    rows.push(std::mem::take(&mut row));
                }
                Token::RBracket => {
                    self.pos += 1;
                    rows.push(row);
                    break;
                }
                other => {
                    return Err(format!(
                        "expected ',', ';' or ']' in matrix literal, found {:?}",
                        other
                    ))
                }
            }
        }
        Ok(Node::Matrix(rows))
    }
}

/// A single-statement block in `(...)` is just that expression.
fn unwrap_block(node: Node) -> Node {
    match node {
        Node::Block(mut stmts) if stmts.len() == 1 => stmts.pop().unwrap(),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    /// Parse a single-statement program and return that statement.
    fn stmt(s: &str) -> Node {
        match parse(lex(s).unwrap()).unwrap() {
            Node::Block(mut v) if v.len() == 1 => v.pop().unwrap(),
            other => panic!("expected one statement, got {:?}", other),
        }
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        // 1 + 2 * 3  ==  1 + (2 * 3)
        match stmt("1 + 2 * 3") {
            Node::BinOp(Op::Add, _, rhs) => assert!(matches!(*rhs, Node::BinOp(Op::Mul, _, _))),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn power_is_right_associative() {
        // 2 ^ 3 ^ 2  ==  2 ^ (3 ^ 2)
        match stmt("2 ^ 3 ^ 2") {
            Node::BinOp(Op::Pow, _, rhs) => assert!(matches!(*rhs, Node::BinOp(Op::Pow, _, _))),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn unary_minus_below_power() {
        // -2 ^ 2  ==  -(2 ^ 2)
        assert!(
            matches!(stmt("-2 ^ 2"), Node::Neg(inner) if matches!(*inner, Node::BinOp(Op::Pow, _, _)))
        );
    }

    #[test]
    fn comparison_below_arithmetic() {
        // 1 + 1 == 2  parses as  (1 + 1) == 2
        match stmt("1 + 1 == 2") {
            Node::BinOp(Op::Equal, lhs, _) => assert!(matches!(*lhs, Node::BinOp(Op::Add, _, _))),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn function_shorthand_vs_call() {
        assert!(matches!(stmt("f(x) := x"), Node::FuncDef(..)));
        assert!(matches!(stmt("f(x)"), Node::Call(..)));
    }

    #[test]
    fn errors_do_not_panic() {
        assert!(parse(lex("1 +").unwrap()).is_err());
        assert!(parse(lex("(1").unwrap()).is_err());
        assert!(parse(lex(")").unwrap()).is_err());
        assert!(parse(lex("[1, 2").unwrap()).is_err());
    }

    #[test]
    fn deeply_nested_is_rejected_not_overflowed() {
        // Production always parses on a big stack (the REPL uses run_with_stack);
        // replicate that here, since the guard fires at depth 512 (~6 MB of
        // parser frames), above a bare test thread's 2 MB.
        crate::run_with_stack(|| {
            let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
            assert!(parse(lex(&deep).unwrap()).is_err());
        });
    }

    #[test]
    fn oversized_input_is_rejected() {
        let huge: Vec<Token> = std::iter::repeat_n(Token::Num("1".into()), MAX_TOKENS + 10)
            .chain(std::iter::once(Token::Eof))
            .collect();
        assert!(parse(huge).is_err());
    }
}
