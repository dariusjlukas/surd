//! Tokenizer. Hand-written, character-at-a-time.
//!
//! Newlines are significant (they separate statements) but only at bracket
//! depth 0 — inside `(...)` or `[...]` a newline is just line continuation.
//! `#` starts a comment to end of line. Block keywords (`if`, `then`, `while`,
//! `function`, `end`, ...) and the logical words `and`/`or`/`not` are lexed as
//! ordinary identifiers and recognized by the parser.

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A numeric literal, kept as text (e.g. "12", "1.5").
    Num(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    /// `.` — struct field access. Only when not starting a number: `.5` stays
    /// a numeric literal, `s.a` is field access.
    Dot,
    /// `.*` — elementwise multiplication.
    DotStar,
    /// `./` — elementwise division.
    DotSlash,
    /// `.^` — elementwise power.
    DotCaret,
    /// `;` — statement separator, or matrix row separator inside `[...]`.
    Semicolon,
    /// A significant newline (statement separator).
    Newline,
    /// `:=` — assignment.
    Assign,
    /// `=` — builds an equation, not a truth test.
    Eq,
    /// `==` — decidable equality test.
    EqEq,
    /// `!=`
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Eof,
}

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    // Bracket nesting, so newlines inside `(...)`/`[...]` are not significant.
    let mut depth: i32 = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '\n' || c == '\r' {
            if depth <= 0 && !matches!(tokens.last(), None | Some(Token::Newline)) {
                tokens.push(Token::Newline);
            }
            i += 1;
            continue;
        }
        if c == '#' {
            // comment to end of line (the newline itself is handled next loop)
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            '+' => push(&mut tokens, Token::Plus, &mut i),
            '-' => push(&mut tokens, Token::Minus, &mut i),
            '*' => push(&mut tokens, Token::Star, &mut i),
            '/' => push(&mut tokens, Token::Slash, &mut i),
            '^' => push(&mut tokens, Token::Caret, &mut i),
            '(' => {
                depth += 1;
                push(&mut tokens, Token::LParen, &mut i);
            }
            ')' => {
                depth -= 1;
                push(&mut tokens, Token::RParen, &mut i);
            }
            '[' => {
                depth += 1;
                push(&mut tokens, Token::LBracket, &mut i);
            }
            ']' => {
                depth -= 1;
                push(&mut tokens, Token::RBracket, &mut i);
            }
            ',' => push(&mut tokens, Token::Comma, &mut i),
            // `.` followed by an operator is elementwise (`.*`, `./`, `.^`);
            // not followed by a digit it's field access; `.5` falls through
            // to the numeric-literal arm below. (`2.*x` lexes as the number
            // `2.` times `x` — harmless, since scalars multiply elementwise
            // and plainly the same way.)
            '.' if matches!(chars.get(i + 1), Some('*' | '/' | '^')) => {
                tokens.push(match chars[i + 1] {
                    '*' => Token::DotStar,
                    '/' => Token::DotSlash,
                    _ => Token::DotCaret,
                });
                i += 2;
            }
            '.' if !matches!(chars.get(i + 1), Some(c) if c.is_ascii_digit()) => {
                push(&mut tokens, Token::Dot, &mut i)
            }
            ';' => push(&mut tokens, Token::Semicolon, &mut i),
            '=' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::EqEq);
                    i += 2;
                } else {
                    push(&mut tokens, Token::Eq, &mut i);
                }
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Ne);
                    i += 2;
                } else {
                    return Err("unexpected '!' (use 'not' for logical negation)".into());
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Le);
                    i += 2;
                } else {
                    push(&mut tokens, Token::Lt, &mut i);
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    push(&mut tokens, Token::Gt, &mut i);
                }
            }
            ':' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Assign);
                    i += 2;
                } else {
                    return Err("unexpected ':' (did you mean ':=' for assignment?)".into());
                }
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                let mut seen_dot = false;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || (chars[i] == '.' && !seen_dot))
                {
                    if chars[i] == '.' {
                        seen_dot = true;
                    }
                    i += 1;
                }
                tokens.push(Token::Num(chars[start..i].iter().collect()));
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token::Ident(chars[start..i].iter().collect()));
            }
            _ => return Err(format!("unexpected character '{}'", c)),
        }
    }

    // A trailing significant newline is just noise to the parser; drop it.
    if tokens.last() == Some(&Token::Newline) {
        tokens.pop();
    }
    let mut tokens = insert_implicit_mul(tokens)?;
    tokens.push(Token::Eof);
    Ok(tokens)
}

/// Reserved words with grammatical meaning. Implicit multiplication never
/// fires before one, so `while (x < 3) do … end` keeps parsing.
const RESERVED: &[&str] = &[
    "if", "then", "else", "end", "while", "do", "function", "and", "or", "not", "true", "false",
];

pub(crate) fn is_reserved(s: &str) -> bool {
    RESERVED.contains(&s)
}

/// Implicit multiplication: `2x`, `2(x+1)`, `2sin(x)`, `(x+1)(x-1)`, `(x+1)y`
/// mean multiplication, so insert the `*` the user left out. Deliberately
/// narrow: the left side must be a number or `)`, the right side `(` or a
/// non-reserved identifier. In particular `ident(` stays a function call,
/// `1.5.5` stays an error (no Num·Num), and adjacent identifiers (`x y`) stay
/// an error — `x then`/`x end` carry grammar.
fn insert_implicit_mul(tokens: Vec<Token>) -> Result<Vec<Token>, String> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for t in tokens {
        if let Some(prev) = out.last() {
            let left_ok = matches!(prev, Token::Num(_) | Token::RParen);
            if left_ok {
                match &t {
                    Token::LParen => out.push(Token::Star),
                    Token::Ident(s) if !is_reserved(s) => {
                        // Guard the scientific-notation trap: `3e5` lexes as
                        // Num(3) · Ident("e5") and would silently become
                        // 3*e5 (a free symbol). Refuse loudly instead.
                        if matches!(prev, Token::Num(_))
                            && s.starts_with(['e', 'E'])
                            && s[1..].chars().all(|c| c.is_ascii_digit())
                            && !s[1..].is_empty()
                        {
                            return Err(format!(
                                "scientific notation '{}…' isn't supported; write a power of 10 \
                                 explicitly (e.g. 3*10^5)",
                                s
                            ));
                        }
                        out.push(Token::Star);
                    }
                    _ => {}
                }
            }
        }
        out.push(t);
    }
    Ok(out)
}

fn push(tokens: &mut Vec<Token>, t: Token, i: &mut usize) {
    tokens.push(t);
    *i += 1;
}

/// True if `src` holds no statements at all (only whitespace and comments), so
/// the REPL has nothing to evaluate or print.
pub fn is_blank(src: &str) -> bool {
    matches!(lex(src).as_deref(), Ok([Token::Eof]))
}

/// True if `src` has unclosed brackets or block keywords — i.e. the REPL should
/// keep reading more lines before trying to evaluate.
pub fn is_incomplete(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return false, // let the parser surface the error
    };
    let mut depth: i32 = 0;
    for t in &tokens {
        match t {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth -= 1,
            Token::Ident(s) if s == "if" || s == "while" || s == "function" => depth += 1,
            Token::Ident(s) if s == "end" => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

#[cfg(test)]
mod tests {
    use super::Token::*;
    use super::*;

    fn toks(s: &str) -> Vec<Token> {
        lex(s).unwrap()
    }

    #[test]
    fn distinguishes_assign_eq_and_eqeq() {
        assert_eq!(
            toks("a := b = c == d"),
            vec![
                Ident("a".into()),
                Assign,
                Ident("b".into()),
                Eq,
                Ident("c".into()),
                EqEq,
                Ident("d".into()),
                Eof
            ]
        );
    }

    #[test]
    fn comparison_operators() {
        assert_eq!(
            toks("< <= > >= !="),
            vec![Lt, Le, Gt, Ge, Ne, Eof]
        );
    }

    #[test]
    fn decimals_are_one_token() {
        assert_eq!(toks("1.5"), vec![Num("1.5".into()), Eof]);
        // a second dot ends the number
        assert_eq!(toks("1.5.5"), vec![Num("1.5".into()), Num(".5".into()), Eof]);
    }

    #[test]
    fn comments_run_to_end_of_line() {
        assert_eq!(toks("1 # ignored\n2"), vec![Num("1".into()), Newline, Num("2".into()), Eof]);
    }

    #[test]
    fn newlines_are_significant_only_at_depth_zero() {
        assert_eq!(toks("1\n2"), vec![Num("1".into()), Newline, Num("2".into()), Eof]);
        // inside parens a newline is line continuation, not a separator
        assert_eq!(
            toks("(1\n+ 2)"),
            vec![LParen, Num("1".into()), Plus, Num("2".into()), RParen, Eof]
        );
    }

    #[test]
    fn errors_instead_of_panicking() {
        assert!(lex("@").is_err());
        assert!(lex("a : b").is_err()); // lone ':' is not ':='
        assert!(lex("!").is_err()); // lone '!'
    }

    #[test]
    fn blank_and_incomplete() {
        assert!(is_blank("   # just a comment"));
        assert!(!is_blank("1"));
        assert!(is_incomplete("if x then"));
        assert!(is_incomplete("(1 + "));
        assert!(!is_incomplete("1 + 1"));
        assert!(!is_incomplete("if x then 1 end"));
    }
}
