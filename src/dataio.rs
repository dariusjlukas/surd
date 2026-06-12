//! Raw-data import/export: the `surd-data` JSON file format, plus
//! best-effort importers for generic JSON and CSV (sensor logs etc.).
//!
//! Exactness contract:
//!   * Exact values (integers, rationals, symbolic expressions, matrices,
//!     structs, equations, user functions) round-trip losslessly.
//!   * `Float`s are stored as their *exact* decimal value (a binary float is
//!     exactly m·2^k, which always terminates in decimal) plus the displayed
//!     digit count; re-import re-rounds that exact decimal at the same
//!     working precision `N(...)` would use — faithful to every displayed
//!     digit, like-for-like with how the float was produced.
//!   * Numbers in generic JSON / CSV are read from their *literal text*
//!     (serde_json's `arbitrary_precision` keeps it), so `0.1` imports as the
//!     exact rational 1/10, never an f64.
//!
//! On-disk shape:
//! ```json
//! { "format": "surd-data", "version": 1,
//!   "variables": [ { "name": "x", "value": ... } ] }
//! ```
//! Values are JSON numbers (exact decimals), booleans, nested arrays
//! (matrices), or `{"t": ...}`-tagged objects for everything else. Import
//! always wraps a file's variables in one struct, so imported names can never
//! collide with existing workspace bindings.

use crate::ast::Node;
use crate::expr::{
    add, complex, float_to_rational, func, mul, numeric_eval, pow, rat_to_expr, structure,
    BigRational, Constant, Expr,
};
use crate::matrix;
use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use serde_json::{json, Map, Number, Value};
use std::rc::Rc;

const FORMAT: &str = "surd-data";
/// Files written before the rename to Surd; still accepted on import.
const LEGACY_FORMAT: &str = "exact-data";
const VERSION: u64 = 1;

/// Bound on decimal exponents / digit counts while parsing, so a crafted
/// `1e999999999` can't allocate a gigantic bignum.
const MAX_DECIMAL_DIGITS: usize = 100_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialize named workspace values into a `surd-data` file.
pub fn export_variables(vars: &[(&str, &Expr)]) -> Result<String, String> {
    let mut entries = Vec::with_capacity(vars.len());
    for (name, value) in vars {
        entries.push(json!({ "name": name, "value": encode(value)? }));
    }
    let file = json!({ "format": FORMAT, "version": VERSION, "variables": entries });
    serde_json::to_string(&file).map_err(|e| format!("could not serialize: {}", e))
}

/// Parse a data file (surd-data JSON, generic JSON, or CSV — sniffed from
/// the content) into a single value, ready to bind to one workspace name.
/// Files with named members (surd-data variables, JSON object keys, CSV
/// columns) come back as a struct of those members; anonymous data (a bare
/// JSON array / scalar, a headerless CSV) comes back as the value itself.
pub fn import(text: &str) -> Result<Expr, String> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        let v: Value = serde_json::from_str(t).map_err(|e| format!("invalid JSON: {}", e))?;
        if let Value::Object(map) = &v {
            let fmt = map.get("format").and_then(Value::as_str);
            if fmt == Some(FORMAT) || fmt == Some(LEGACY_FORMAT) {
                return import_native(map);
            }
        }
        decode(&v, Mode::Generic)
    } else {
        import_csv(text)
    }
}

/// A short human description of a value, for import summaries
/// ("struct with 3 fields: t (600×1 matrix), …").
pub fn describe(e: &Expr) -> String {
    match e {
        Expr::Matrix(rows) => format!("{}×{} matrix", rows.len(), rows[0].len()),
        Expr::Function { params, .. } => format!("function({})", params.join(", ")),
        Expr::Struct(fields) => {
            const SHOWN: usize = 6;
            let mut parts: Vec<String> = fields
                .iter()
                .take(SHOWN)
                .map(|(n, v)| format!("{} ({})", n, describe_short(v)))
                .collect();
            if fields.len() > SHOWN {
                parts.push(format!("… {} more", fields.len() - SHOWN));
            }
            format!(
                "struct with {} field{}: {}",
                fields.len(),
                if fields.len() == 1 { "" } else { "s" },
                parts.join(", ")
            )
        }
        other => describe_short(other),
    }
}

fn describe_short(e: &Expr) -> String {
    match e {
        Expr::Matrix(rows) => format!("{}×{} matrix", rows.len(), rows[0].len()),
        Expr::Struct(fields) => format!("struct, {} fields", fields.len()),
        Expr::Function { params, .. } => format!("function({})", params.join(", ")),
        other => {
            let s = format!("{}", other);
            if s.chars().count() > 40 {
                let cut: String = s.chars().take(40).collect();
                format!("{}…", cut)
            } else {
                s
            }
        }
    }
}

/// Is `s` usable as a workspace variable name (parses as a bare identifier)?
pub fn is_valid_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !crate::lexer::is_reserved(s)
        && s != "struct"
}

// ---------------------------------------------------------------------------
// Encoding (Expr -> JSON)
// ---------------------------------------------------------------------------

fn encode(e: &Expr) -> Result<Value, String> {
    Ok(match e {
        // serde_json round-trips f64 exactly (shortest-representation), so
        // the certified bounds survive export/import losslessly.
        Expr::Signal(s) => match s.as_ref() {
            crate::signal::SignalData::F64 { lo, hi } => {
                json!({ "t": "signal", "lo": lo, "hi": hi })
            }
            crate::signal::SignalData::Big { .. } => {
                return Err(
                    "export of arbitrary-precision signals is not supported yet".to_string()
                )
            }
        },
        Expr::Int(i) => number_from_text(&i.to_string()),
        Expr::Rat(r) => match rat_to_decimal(r) {
            // Decimal-friendly denominators (2^a·5^b) write as plain numbers.
            Some(dec) => number_from_text(&dec),
            None => json!({ "t": "rat", "v": format!("{}/{}", r.numer(), r.denom()) }),
        },
        Expr::Float(bf, digits) => {
            let r = float_to_rational(bf)
                .ok_or_else(|| "cannot export a non-finite float".to_string())?;
            let dec = rat_to_decimal(&r)
                .expect("a binary float is always a terminating decimal");
            json!({ "t": "float", "v": dec, "digits": digits })
        }
        Expr::Const(Constant::Pi) => json!({ "t": "const", "v": "pi" }),
        Expr::Const(Constant::E) => json!({ "t": "const", "v": "e" }),
        Expr::Symbol(s) => json!({ "t": "sym", "v": s }),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Add(ts) => json!({ "t": "add", "args": encode_all(ts)? }),
        Expr::Mul(fs) => json!({ "t": "mul", "args": encode_all(fs)? }),
        Expr::Pow(b, x) => json!({ "t": "pow", "base": encode(b)?, "exp": encode(x)? }),
        Expr::Func(name, args) => json!({ "t": "func", "name": name, "args": encode_all(args)? }),
        Expr::Matrix(rows) => Value::Array(
            rows.iter()
                .map(|row| Ok(Value::Array(encode_all(row)?)))
                .collect::<Result<_, String>>()?,
        ),
        Expr::Complex(re, im) => json!({ "t": "complex", "re": encode(re)?, "im": encode(im)? }),
        Expr::Equation(l, r) => json!({ "t": "eq", "lhs": encode(l)?, "rhs": encode(r)? }),
        Expr::Function { params, body } => {
            let body = serde_json::to_value(body.as_ref())
                .map_err(|e| format!("could not serialize function body: {}", e))?;
            json!({ "t": "function", "params": params, "body": body })
        }
        Expr::Struct(fields) => {
            let mut map = Map::new();
            for (n, v) in fields {
                map.insert(n.clone(), encode(v)?);
            }
            json!({ "t": "struct", "fields": map })
        }
    })
}

fn encode_all(es: &[Expr]) -> Result<Vec<Value>, String> {
    es.iter().map(encode).collect()
}

/// A JSON number from already-validated decimal text. With
/// `arbitrary_precision` this preserves every digit.
fn number_from_text(text: &str) -> Value {
    let n: Number = serde_json::from_str(text).expect("decimal text is a valid JSON number");
    Value::Number(n)
}

// ---------------------------------------------------------------------------
// Decoding (JSON -> Expr)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Our own format: objects must be `{"t": ...}`-tagged, bare strings are
    /// an error.
    Tagged,
    /// Foreign JSON: objects become structs, strings become symbols.
    Generic,
}

fn import_native(map: &Map<String, Value>) -> Result<Expr, String> {
    match map.get("version").and_then(Value::as_u64) {
        Some(v) if v <= VERSION => {}
        Some(v) => return Err(format!("unsupported surd-data version {}", v)),
        None => return Err("surd-data file has no version".into()),
    }
    let vars = map
        .get("variables")
        .and_then(Value::as_array)
        .ok_or("surd-data file has no 'variables' array")?;
    let mut fields = Vec::with_capacity(vars.len());
    for entry in vars {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or("a variable entry has no 'name'")?;
        let value = entry
            .get("value")
            .ok_or_else(|| format!("variable '{}' has no 'value'", name))?;
        let value = decode(value, Mode::Tagged)
            .map_err(|e| format!("variable '{}': {}", name, e))?;
        fields.push((name.to_string(), value));
    }
    structure(fields)
}

fn decode(v: &Value, mode: Mode) -> Result<Expr, String> {
    match v {
        Value::Number(n) => decimal_to_rat(&n.to_string()).map(rat_to_expr),
        Value::Bool(b) => Ok(Expr::Bool(*b)),
        Value::Null => Err("null values are not supported".into()),
        Value::String(s) => match mode {
            Mode::Generic => Ok(Expr::Symbol(s.clone())),
            Mode::Tagged => Err(format!("unexpected bare string '{}'", s)),
        },
        Value::Array(items) => decode_array(items, mode),
        Value::Object(map) => match mode {
            Mode::Tagged => decode_tagged(map),
            Mode::Generic => {
                let mut fields = Vec::with_capacity(map.len());
                let mut taken: Vec<String> = Vec::new();
                for (key, value) in map {
                    let name = unique_ident(sanitize_ident(key, "field"), &mut taken);
                    let value =
                        decode(value, mode).map_err(|e| format!("field '{}': {}", key, e))?;
                    fields.push((name, value));
                }
                structure(fields)
            }
        },
    }
}

/// A flat array is a column vector; an array of arrays is a matrix, row-major.
fn decode_array(items: &[Value], mode: Mode) -> Result<Expr, String> {
    if items.is_empty() {
        return Err("empty arrays are not supported".into());
    }
    let nested = items.iter().filter(|v| v.is_array()).count();
    if nested == items.len() {
        let mut rows = Vec::with_capacity(items.len());
        for (i, row) in items.iter().enumerate() {
            let cells = row.as_array().expect("counted as array above");
            let mut out = Vec::with_capacity(cells.len());
            for cell in cells {
                if cell.is_array() {
                    return Err("arrays nest at most two levels (matrix rows)".into());
                }
                out.push(decode(cell, mode).map_err(|e| format!("row {}: {}", i + 1, e))?);
            }
            rows.push(out);
        }
        matrix::matrix(rows)
    } else if nested == 0 {
        let mut rows = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            rows.push(vec![
                decode(item, mode).map_err(|e| format!("entry {}: {}", i + 1, e))?
            ]);
        }
        matrix::matrix(rows)
    } else {
        Err("an array must hold either all scalars or all rows, not a mix".into())
    }
}

fn decode_tagged(map: &Map<String, Value>) -> Result<Expr, String> {
    let tag = map
        .get("t")
        .and_then(Value::as_str)
        .ok_or("object in surd-data file has no 't' tag")?;
    let field = |k: &str| -> Result<&Value, String> {
        map.get(k).ok_or_else(|| format!("'{}' value has no '{}'", tag, k))
    };
    let dec = |k: &str| -> Result<Expr, String> { decode(field(k)?, Mode::Tagged) };
    let dec_args = |k: &str| -> Result<Vec<Expr>, String> {
        field(k)?
            .as_array()
            .ok_or_else(|| format!("'{}' of '{}' must be an array", k, tag))?
            .iter()
            .map(|v| decode(v, Mode::Tagged))
            .collect()
    };
    let text = |k: &str| -> Result<&str, String> {
        field(k)?
            .as_str()
            .ok_or_else(|| format!("'{}' of '{}' must be a string", k, tag))
    };

    match tag {
        "rat" => {
            let (n, d) = text("v")?
                .split_once('/')
                .ok_or("'rat' value must look like \"num/den\"")?;
            let n: BigInt = n.trim().parse().map_err(|_| "bad rational numerator")?;
            let d: BigInt = d.trim().parse().map_err(|_| "bad rational denominator")?;
            if d.is_zero() {
                return Err("rational with zero denominator".into());
            }
            Ok(rat_to_expr(BigRational::new(n, d)))
        }
        "float" => {
            let digits = map
                .get("digits")
                .and_then(Value::as_u64)
                .unwrap_or(30)
                .clamp(1, 100_000) as usize;
            let r = decimal_to_rat(text("v")?)?;
            numeric_eval(&rat_to_expr(r), digits)
        }
        "const" => match text("v")? {
            "pi" => Ok(Expr::Const(Constant::Pi)),
            "e" => Ok(Expr::Const(Constant::E)),
            other => Err(format!("unknown constant '{}'", other)),
        },
        "sym" => Ok(Expr::Symbol(text("v")?.to_string())),
        // Smart constructors re-canonicalize, so a hand-edited file can't
        // smuggle in values that violate the engine's invariants.
        "add" => Ok(add(dec_args("args")?)),
        "mul" => Ok(mul(dec_args("args")?)),
        "pow" => Ok(pow(dec("base")?, dec("exp")?)),
        "func" => Ok(func(text("name")?, dec_args("args")?)),
        "complex" => Ok(complex(dec("re")?, dec("im")?)),
        "eq" => Ok(Expr::Equation(Box::new(dec("lhs")?), Box::new(dec("rhs")?))),
        "signal" => {
            let lo: Vec<f64> = serde_json::from_value(field("lo")?.clone())
                .map_err(|_| "'signal' lo must be an array of numbers".to_string())?;
            let hi: Vec<f64> = serde_json::from_value(field("hi")?.clone())
                .map_err(|_| "'signal' hi must be an array of numbers".to_string())?;
            if lo.len() != hi.len() {
                return Err("'signal' lo and hi must have the same length".into());
            }
            if lo.iter().zip(&hi).any(|(l, h)| !l.is_finite() || !h.is_finite() || l > h) {
                return Err("'signal' bounds must be finite with lo <= hi".into());
            }
            Ok(Expr::Signal(Rc::new(crate::signal::SignalData::F64 {
                lo,
                hi,
            })))
        }
        "function" => {
            let params: Vec<String> = serde_json::from_value(field("params")?.clone())
                .map_err(|_| "'function' params must be an array of strings".to_string())?;
            let body: Node = serde_json::from_value(field("body")?.clone())
                .map_err(|e| format!("bad function body: {}", e))?;
            Ok(Expr::Function { params, body: Rc::new(body) })
        }
        "struct" => {
            let fields = field("fields")?
                .as_object()
                .ok_or("'fields' of 'struct' must be an object")?;
            let mut out = Vec::with_capacity(fields.len());
            for (n, v) in fields {
                out.push((n.clone(), decode(v, Mode::Tagged)?));
            }
            structure(out)
        }
        other => Err(format!("unknown value tag '{}'", other)),
    }
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

/// CSV with a header row becomes a struct of column vectors; an all-numeric
/// file becomes a plain matrix. Cells parse from their literal text into
/// exact rationals (scientific notation included).
fn import_csv(text: &str) -> Result<Expr, String> {
    let records = parse_csv(text);
    if records.is_empty() {
        return Err("the file is empty".into());
    }
    let width = records[0].len();
    for (i, r) in records.iter().enumerate() {
        if r.len() != width {
            return Err(format!(
                "row {} has {} cell(s), but row 1 has {}",
                i + 1,
                r.len(),
                width
            ));
        }
    }

    let has_header = records[0].iter().any(|c| decimal_to_rat(c).is_err());
    if !has_header {
        let rows = records
            .iter()
            .map(|r| r.iter().map(|c| decimal_to_rat(c).map(rat_to_expr)).collect())
            .collect::<Result<Vec<Vec<Expr>>, String>>()?;
        return matrix::matrix(rows);
    }

    if records.len() < 2 {
        return Err("the file has a header row but no data rows".into());
    }
    let mut taken: Vec<String> = Vec::new();
    let names: Vec<String> = records[0]
        .iter()
        .enumerate()
        .map(|(j, h)| unique_ident(sanitize_ident(h, &format!("column_{}", j + 1)), &mut taken))
        .collect();

    let mut fields = Vec::with_capacity(width);
    for (j, name) in names.iter().enumerate() {
        let mut col = Vec::with_capacity(records.len() - 1);
        for (i, record) in records.iter().enumerate().skip(1) {
            let cell = &record[j];
            let value = decimal_to_rat(cell).map_err(|_| {
                format!(
                    "row {}, column '{}': '{}' is not a number",
                    i + 1,
                    records[0][j],
                    cell
                )
            })?;
            col.push(vec![rat_to_expr(value)]);
        }
        fields.push((name.clone(), matrix::matrix(col)?));
    }
    structure(fields)
}

/// Minimal RFC-4180-ish reader: quoted cells (with `""` escapes) may contain
/// the delimiter and newlines; unquoted cells are trimmed. The delimiter is
/// sniffed from the first line (comma, semicolon, or tab). Blank records are
/// dropped.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first_line = text.lines().next().unwrap_or("");
    let delim = [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| first_line.matches(*d).count())
        .unwrap_or(',');

    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false; // the *cell* was opened with a quote
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    let push_cell = |record: &mut Vec<String>, cell: &mut String, quoted: bool| {
        let done = std::mem::take(cell);
        record.push(if quoted { done } else { done.trim().to_string() });
    };

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if cell.trim().is_empty() => {
                in_quotes = true;
                quoted = true;
                cell.clear();
            }
            '\r' => {} // swallowed; '\n' ends the record
            '\n' => {
                push_cell(&mut record, &mut cell, quoted);
                quoted = false;
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                }
                record.clear();
            }
            c if c == delim => {
                push_cell(&mut record, &mut cell, quoted);
                quoted = false;
            }
            c => cell.push(c),
        }
    }
    push_cell(&mut record, &mut cell, quoted);
    if !(record.len() == 1 && record[0].is_empty()) {
        records.push(record);
    }
    records
}

// ---------------------------------------------------------------------------
// Identifier + decimal helpers
// ---------------------------------------------------------------------------

/// Bend an arbitrary header/key into a valid variable name.
fn sanitize_ident(s: &str, fallback: &str) -> String {
    let mut out: String = s
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if out.is_empty() {
        out = fallback.to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if !is_valid_var_name(&out) {
        out.push('_');
    }
    out
}

/// Suffix `_2`, `_3`, … until the name is new, then record it.
fn unique_ident(base: String, taken: &mut Vec<String>) -> String {
    let mut name = base.clone();
    let mut i = 2;
    while taken.contains(&name) {
        name = format!("{}_{}", base, i);
        i += 1;
    }
    taken.push(name.clone());
    name
}

/// Exact rational from decimal text: optional sign, digits, optional
/// fraction, optional e-exponent. `0.1` → 1/10, `1.5e-3` → 3/2000.
pub(crate) fn decimal_to_rat(s: &str) -> Result<BigRational, String> {
    let s = s.trim();
    let bad = || format!("'{}' is not a number", s);
    if s.is_empty() {
        return Err(bad());
    }
    let (mantissa, exp) = match s.split_once(['e', 'E']) {
        Some((m, e)) => {
            let exp: i64 = e.parse().map_err(|_| bad())?;
            (m, exp)
        }
        None => (s, 0),
    };
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(bad());
    }
    if !int_part.chars().chain(frac_part.chars()).all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let scale = exp - frac_part.len() as i64;
    if int_part.len() + frac_part.len() > MAX_DECIMAL_DIGITS
        || scale.unsigned_abs() > MAX_DECIMAL_DIGITS as u64
    {
        return Err(format!("number '{}' is too large to represent", s));
    }
    let mut numer: BigInt = format!("{}{}", int_part, frac_part).parse().map_err(|_| bad())?;
    if negative {
        numer = -numer;
    }
    let ten = BigInt::from(10);
    Ok(if scale >= 0 {
        BigRational::from_integer(numer * num_traits::pow::pow(ten, scale as usize))
    } else {
        BigRational::new(numer, num_traits::pow::pow(ten, (-scale) as usize))
    })
}

/// The exact decimal text of `r`, if its denominator is 2^a·5^b (i.e. the
/// decimal terminates within [`MAX_DECIMAL_DIGITS`]).
pub(crate) fn rat_to_decimal(r: &BigRational) -> Option<String> {
    let mut den = r.denom().clone(); // normalized: always positive
    let (mut twos, mut fives) = (0usize, 0usize);
    let (two, five) = (BigInt::from(2), BigInt::from(5));
    while (&den % &two).is_zero() {
        den /= &two;
        twos += 1;
    }
    while (&den % &five).is_zero() {
        den /= &five;
        fives += 1;
    }
    if !den.is_one() {
        return None;
    }
    let digits = twos.max(fives);
    if digits > MAX_DECIMAL_DIGITS {
        return None;
    }
    if digits == 0 {
        return Some(r.numer().to_string());
    }
    // numer/den · 10^digits is an integer: pad the missing 2s and 5s.
    let scaled = r.numer().abs()
        * num_traits::pow::pow(two, digits - twos)
        * num_traits::pow::pow(five, digits - fives);
    let mut s = scaled.to_string();
    if s.len() <= digits {
        s = format!("{}{}", "0".repeat(digits - s.len() + 1), s);
    }
    s.insert(s.len() - digits, '.');
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    let s = if s.is_empty() { "0".to_string() } else { s };
    Some(if r.is_negative() { format!("-{}", s) } else { s })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    fn val(src: &str) -> Expr {
        Interpreter::new().eval_line(src).expect(src)
    }

    /// Export one variable, re-import, and unwrap it from the import struct.
    fn round_trip(e: &Expr) -> Expr {
        let file = export_variables(&[("x", e)]).expect("export");
        match import(&file).expect("import") {
            Expr::Struct(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "x");
                fields[0].1.clone()
            }
            other => panic!("import of surd-data should be a struct, got {}", other),
        }
    }

    #[test]
    fn exact_values_round_trip_losslessly() {
        for src in [
            "123",
            "-7",
            "10^40",          // beyond u64
            "1/3",            // non-decimal rational -> tagged
            "-3/2",           // decimal-friendly rational -> plain number
            "true",
            "pi + e",         // constants inside a sum
            "sqrt(2)",        // 2^(1/2)
            "1 + 2*x + x^2",  // symbolic polynomial
            "sin(y) * ln(y)", // function applications
            "[1, 2; 3, 4]",
            "[1; 2; 3]",
            "2 + 3*I",
            "x^2 = 4",       // equation
            "struct(a = 1, b = struct(c = [1; 2]))",
            "plot(x^2, x, 0, 1)",
        ] {
            let v = val(src);
            assert_eq!(round_trip(&v), v, "round-trip changed {}", src);
        }
    }

    #[test]
    fn functions_round_trip_and_stay_callable() {
        let mut interp = Interpreter::new();
        interp.eval_line("f(n) := n^2 + 1").unwrap();
        let f = interp.get_global("f").unwrap().clone();
        let back = round_trip(&f);
        assert_eq!(back, f);
        let mut interp2 = Interpreter::new();
        interp2.set_global("g", back);
        assert_eq!(format!("{}", interp2.eval_line("g(3)").unwrap()), "10");
    }

    #[test]
    fn floats_round_trip_to_every_displayed_digit() {
        for src in ["N(pi, 40)", "N(1/3)", "N(2)^(1/3)", "N(-1.5e-10, 12)"] {
            let v = val(src);
            let back = round_trip(&v);
            assert_eq!(format!("{}", back), format!("{}", v), "float text changed for {}", src);
        }
    }

    #[test]
    fn multiple_variables_export_into_one_struct() {
        let a = val("42");
        let b = val("[1; 2]");
        let file = export_variables(&[("a", &a), ("b", &b)]).unwrap();
        let imported = import(&file).unwrap();
        assert_eq!(
            imported,
            structure(vec![("a".into(), a), ("b".into(), b)]).unwrap()
        );
    }

    #[test]
    fn generic_json_imports_exactly() {
        // Decimals come from their literal text: 0.1 is exactly 1/10.
        let v = import(r#"{"gain": 0.1, "n": 3, "ok": true, "label": "probe"}"#).unwrap();
        assert_eq!(
            format!("{}", v),
            "struct(gain = 1/10, label = probe, n = 3, ok = true)"
        );
        // Flat arrays are column vectors; nested arrays are matrices.
        assert_eq!(format!("{}", import("[1, 2.5, 3e2]").unwrap()), "[   1 ]\n[ 5/2 ]\n[ 300 ]");
        assert_eq!(
            import("[[1, 2], [3, 4]]").unwrap(),
            val("[1, 2; 3, 4]")
        );
        // Awkward keys are bent into identifiers.
        let v = import(r#"{"sensor 1": 5, "2nd": 6}"#).unwrap();
        assert_eq!(format!("{}", v), "struct(_2nd = 6, sensor_1 = 5)");
        // Nulls and empty arrays refuse loudly.
        assert!(import(r#"{"a": null}"#).is_err());
        assert!(import("[]").is_err());
        assert!(import(r#"{"a": [1, [2]]}"#).is_err());
    }

    #[test]
    fn csv_with_header_becomes_struct_of_columns() {
        let v = import("t, value\n0, 1.5\n1, 2.5e-1\n2, -3\n").unwrap();
        let Expr::Struct(fields) = &v else { panic!("expected struct") };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "t");
        assert_eq!(fields[0].1, val("[0; 1; 2]"));
        assert_eq!(fields[1].1, val("[3/2; 1/4; -3]"));
    }

    #[test]
    fn csv_without_header_becomes_a_matrix() {
        assert_eq!(import("1, 2\n3, 4\n").unwrap(), val("[1, 2; 3, 4]"));
        // Semicolon and tab delimiters are sniffed.
        assert_eq!(import("1;2\n3;4").unwrap(), val("[1, 2; 3, 4]"));
        assert_eq!(import("1\t2\n3\t4").unwrap(), val("[1, 2; 3, 4]"));
    }

    #[test]
    fn csv_handles_quotes_and_errors() {
        // Quoted headers (with embedded delimiter) still become field names.
        let v = import("\"time, s\", \"temp\"\n1, 20\n").unwrap();
        assert_eq!(format!("{}", v), "struct(temp = [ 20 ], time__s = [ 1 ])");
        // A non-numeric data cell errors with its location.
        let err = import("t, v\n1, oops\n").unwrap_err();
        assert!(err.contains("row 2") && err.contains("'v'") && err.contains("oops"), "{}", err);
        // Ragged rows error with the row number.
        assert!(import("a, b\n1\n").unwrap_err().contains("row 2"));
        assert!(import("").is_err());
    }

    #[test]
    fn decimal_text_helpers() {
        assert_eq!(decimal_to_rat("0.1").unwrap(), BigRational::new(1.into(), 10.into()));
        assert_eq!(decimal_to_rat("-1.5e-3").unwrap(), BigRational::new((-3).into(), 2000.into()));
        assert_eq!(decimal_to_rat("+2e3").unwrap(), BigRational::from_integer(2000.into()));
        assert!(decimal_to_rat("nope").is_err());
        assert!(decimal_to_rat("1e999999999").is_err());

        let r = |n: i64, d: i64| BigRational::new(n.into(), d.into());
        assert_eq!(rat_to_decimal(&r(1, 10)).unwrap(), "0.1");
        assert_eq!(rat_to_decimal(&r(-3, 2)).unwrap(), "-1.5");
        assert_eq!(rat_to_decimal(&r(7, 1)).unwrap(), "7");
        assert_eq!(rat_to_decimal(&r(1, 8)).unwrap(), "0.125");
        assert_eq!(rat_to_decimal(&r(1, 3)), None);
        // Sanity: text -> rational -> text is stable.
        assert_eq!(rat_to_decimal(&decimal_to_rat("123.456").unwrap()).unwrap(), "123.456");
    }

    #[test]
    fn hostile_files_error_rather_than_panic() {
        for text in [
            "{",
            r#"{"format": "surd-data"}"#,
            r#"{"format": "surd-data", "version": 99, "variables": []}"#,
            r#"{"format": "surd-data", "version": 1, "variables": [{"name": "x"}]}"#,
            r#"{"format": "surd-data", "version": 1, "variables": [{"name": "x", "value": {"t": "wat"}}]}"#,
            r#"{"format": "surd-data", "version": 1, "variables": [{"name": "x", "value": {"t": "rat", "v": "1/0"}}]}"#,
            r#"{"format": "surd-data", "version": 1, "variables": []}"#,
            // The legacy marker must hit the same strict path, not fall
            // through to the generic-JSON importer (which would succeed).
            r#"{"format": "exact-data"}"#,
        ] {
            assert!(import(text).is_err(), "should error: {}", text);
        }
    }

    #[test]
    fn legacy_exact_data_files_still_import() {
        let legacy = r#"{"format": "exact-data", "version": 1,
            "variables": [{"name": "x", "value": {"t": "rat", "v": "1/3"}}]}"#;
        match import(legacy).expect("legacy import") {
            Expr::Struct(fields) => assert_eq!(fields[0].0, "x"),
            other => panic!("expected struct, got {}", other),
        }
    }
}

// ---------------------------------------------------------------------------
// Bulk imports: WAV audio, raw binary arrays, packed CSV — all land as
// signals (certified point intervals; integer PCM and IEEE floats convert
// to f64 exactly, so the initial error bound is exactly zero).
// ---------------------------------------------------------------------------

/// Decoded-sample cap across all channels (memory guard: each sample holds
/// two f64 bounds). 2^24 ≈ 3 minutes of stereo 44.1 kHz audio.
const MAX_BULK_SAMPLES: usize = 1 << 24;

/// Parse a WAV file (PCM 16/24/32-bit int or IEEE float 32/64, any channel
/// count) into `struct(rate, ch1[, ch2…])` of signals. Integer samples are
/// normalized to [−1, 1) by the type's full scale — exactly, since dividing
/// by a power of two is lossless in binary floating point.
pub fn import_wav(bytes: &[u8]) -> Result<Expr, String> {
    let u16le = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let u32le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file (missing RIFF/WAVE header)".into());
    }
    let mut fmt: Option<(u16, usize, u32, u16)> = None; // (format, channels, rate, bits)
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32le(&bytes[at + 4..at + 8]) as usize;
        let body_end = (at + 8).saturating_add(size).min(bytes.len());
        let body = &bytes[at + 8..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                fmt = Some((
                    u16le(&body[0..2]),
                    u16le(&body[2..4]) as usize,
                    u32le(&body[4..8]),
                    u16le(&body[14..16]),
                ));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at = body_end + (size & 1); // chunks are word-aligned
    }
    let (format, channels, rate, bits) = fmt.ok_or("WAV file has no fmt chunk")?;
    let data = data.ok_or("WAV file has no data chunk")?;
    if channels == 0 {
        return Err("WAV file declares zero channels".into());
    }
    let bytes_per = (bits as usize) / 8;
    if bytes_per == 0 || data.len() / bytes_per / channels == 0 {
        return Err("WAV file has no samples".into());
    }
    let frames = data.len() / (bytes_per * channels);
    if frames * channels > MAX_BULK_SAMPLES {
        return Err(format!(
            "WAV file too large ({} samples; the cap is {})",
            frames * channels,
            MAX_BULK_SAMPLES
        ));
    }
    let decode = |frame: usize, ch: usize| -> Result<f64, String> {
        let o = (frame * channels + ch) * bytes_per;
        let s = &data[o..o + bytes_per];
        Ok(match (format, bits) {
            (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f64 / 32768.0,
            (1, 24) => {
                let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8; // sign-extend
                v as f64 / 8388608.0
            }
            (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2147483648.0,
            (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
            (3, 64) => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
            _ => {
                return Err(format!(
                    "unsupported WAV encoding (format {}, {} bits) — PCM 16/24/32 and \
                     IEEE float 32/64 are supported",
                    format, bits
                ))
            }
        })
    };
    let mut fields = vec![("rate".to_string(), Expr::Int(BigInt::from(rate)))];
    for ch in 0..channels {
        let mut lo = Vec::with_capacity(frames);
        for f in 0..frames {
            let v = decode(f, ch)?;
            if !v.is_finite() {
                return Err(format!("non-finite sample at frame {}", f + 1));
            }
            lo.push(v);
        }
        let hi = lo.clone(); // every decode above is exact: point intervals
        fields.push((
            format!("ch{}", ch + 1),
            Expr::Signal(Rc::new(crate::signal::SignalData::F64 { lo, hi })),
        ));
    }
    structure(fields)
}

/// Parse a headerless little-endian array of `f64`, `f32`, or `i16` into a
/// signal. No normalization — raw captures keep their raw values (use
/// arithmetic to scale; it's certified anyway).
pub fn import_raw(bytes: &[u8], format: &str) -> Result<Expr, String> {
    let width = match format {
        "f64" => 8,
        "f32" => 4,
        "i16" => 2,
        other => {
            return Err(format!(
                "unknown raw format '{}' (supported: f64, f32, i16; little-endian)",
                other
            ))
        }
    };
    if bytes.is_empty() || !bytes.len().is_multiple_of(width) {
        return Err(format!(
            "raw data length {} is not a multiple of {} bytes",
            bytes.len(),
            width
        ));
    }
    let n = bytes.len() / width;
    if n > MAX_BULK_SAMPLES {
        return Err(format!("raw data too large ({} samples; cap {})", n, MAX_BULK_SAMPLES));
    }
    let mut lo = Vec::with_capacity(n);
    for i in 0..n {
        let s = &bytes[i * width..(i + 1) * width];
        let v = match format {
            "f64" => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
            "f32" => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
            _ => i16::from_le_bytes([s[0], s[1]]) as f64,
        };
        if !v.is_finite() {
            return Err(format!("non-finite value at sample {}", i + 1));
        }
        lo.push(v);
    }
    let hi = lo.clone(); // exact conversions: point intervals
    Ok(Expr::Signal(Rc::new(crate::signal::SignalData::F64 {
        lo,
        hi,
    })))
}

/// Parse CSV straight into packed signals (one per column) — the bulk path
/// for files too large for exact rationals. Integers within ±2^53 pack as
/// exact points; other decimals as certified ±1-ulp enclosures around the
/// correctly-rounded parse (Rust's float parsing is correctly rounded).
pub fn import_csv_packed(text: &str) -> Result<Expr, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty()).peekable();
    let first = *lines.peek().ok_or("the CSV file is empty")?;
    let cells = |l: &str| l.split(',').map(|c| c.trim().to_string()).collect::<Vec<_>>();
    let head = cells(first);
    let has_header = head.iter().any(|c| c.parse::<f64>().is_err());
    let names: Vec<String> = if has_header {
        lines.next();
        head.iter()
            .enumerate()
            .map(|(i, c)| {
                if is_valid_var_name(c) {
                    c.clone()
                } else {
                    format!("col{}", i + 1)
                }
            })
            .collect()
    } else {
        (1..=head.len()).map(|i| format!("col{}", i)).collect()
    };
    let mut lo: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
    let mut hi: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
    let mut total = 0usize;
    for (row, line) in lines.enumerate() {
        let row_cells = cells(line);
        if row_cells.len() != names.len() {
            return Err(format!(
                "row {} has {} cells, expected {}",
                row + 2,
                row_cells.len(),
                names.len()
            ));
        }
        for (c, cell) in row_cells.iter().enumerate() {
            let exact_int = cell
                .parse::<i64>()
                .ok()
                .filter(|v| v.unsigned_abs() <= (1 << 53));
            let (l, h) = match exact_int {
                Some(v) => (v as f64, v as f64),
                None => {
                    let v: f64 = cell.parse().map_err(|_| {
                        format!("row {}, column {}: '{}' is not a number", row + 2, c + 1, cell)
                    })?;
                    if !v.is_finite() {
                        return Err(format!("row {}: non-finite value", row + 2));
                    }
                    (v.next_down(), v.next_up())
                }
            };
            lo[c].push(l);
            hi[c].push(h);
        }
        total += names.len();
        if total > MAX_BULK_SAMPLES {
            return Err(format!("CSV too large (cap {} values)", MAX_BULK_SAMPLES));
        }
    }
    if lo[0].is_empty() {
        return Err("the CSV file has no data rows".into());
    }
    structure(
        names
            .into_iter()
            .zip(lo.into_iter().zip(hi))
            .map(|(name, (lo, hi))| {
                (
                    name,
                    Expr::Signal(Rc::new(crate::signal::SignalData::F64 { lo, hi })),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod bulk_tests {
    use super::*;

    /// A minimal 16-bit PCM WAV: mono, 4 samples.
    fn tiny_wav() -> Vec<u8> {
        let samples: [i16; 4] = [0, 16384, -16384, 32767];
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend(((36 + data.len()) as u32).to_le_bytes());
        w.extend(b"WAVE");
        w.extend(b"fmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(1u16.to_le_bytes()); // mono
        w.extend(8000u32.to_le_bytes()); // rate
        w.extend(16000u32.to_le_bytes()); // byte rate
        w.extend(2u16.to_le_bytes()); // block align
        w.extend(16u16.to_le_bytes()); // bits
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(&data);
        w
    }

    #[test]
    fn wav_imports_exactly() {
        let v = import_wav(&tiny_wav()).unwrap();
        let Expr::Struct(fields) = &v else { panic!() };
        assert_eq!(fields[1].0, "rate");
        assert_eq!(fields[1].1, Expr::Int(BigInt::from(8000)));
        let Expr::Signal(s) = &fields[0].1 else { panic!() };
        let crate::signal::SignalData::F64 { lo, hi } = s.as_ref() else {
            panic!()
        };
        // Integer PCM normalizes exactly: point intervals, error zero.
        assert_eq!(lo, hi);
        assert_eq!(lo[1], 0.5);
        assert_eq!(lo[2], -0.5);
    }

    #[test]
    fn raw_and_csv_imports() {
        let bytes: Vec<u8> = [1.5f32, -2.25, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let Expr::Signal(s) = import_raw(&bytes, "f32").unwrap() else {
            panic!()
        };
        assert_eq!(s.len(), 3);

        let v = import_csv_packed("t, y\n0, 1.5\n1, 0.1\n").unwrap();
        let Expr::Struct(fields) = &v else { panic!() };
        let Expr::Signal(y) = &fields[1].1 else { panic!() };
        let crate::signal::SignalData::F64 { lo, hi } = y.as_ref() else {
            panic!()
        };
        // 1.5 is dyadic → ±1 ulp around the parse still encloses it; 0.1 is
        // not — either way lo ≤ value ≤ hi must hold.
        assert!(lo[1] < 0.1 && 0.1 < hi[1]);
        assert!(lo[0] <= 1.5 && 1.5 <= hi[0]);

        assert!(import_wav(b"not a wav").is_err());
        assert!(import_raw(&[1, 2, 3], "f32").is_err());
    }
}
