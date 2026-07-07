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
    // Strip a UTF-8 BOM (Excel's "CSV UTF-8" writes one) from the text that
    // is actually PARSED, not just from a sniffing copy: a BOM'd headerless
    // CSV used to have its first data row silently consumed as a header.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let t = text.trim_start();
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
/// ("struct with 3 fields: t (600×1 matrix), …"). Missing cells (`NA`) are
/// counted up front, so an import of real-world data says what it dragged in.
pub fn describe(e: &Expr) -> String {
    let described = match e {
        Expr::Matrix(_) => describe_short(e),
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
    };
    match count_missing(e) {
        0 => described,
        1 => format!("{} — 1 missing value (NA)", described),
        m => format!("{} — {} missing values (NA)", described, m),
    }
}

/// If the matrix holds categorical entries (symbols other than `NA`), the
/// number of distinct non-missing levels; `None` for a purely numeric one.
fn categorical_levels(rows: &[Vec<Expr>]) -> Option<usize> {
    let mut levels: Vec<&Expr> = Vec::new();
    let mut saw_symbol = false;
    for cell in rows.iter().flatten() {
        if crate::expr::is_missing(cell) {
            continue;
        }
        saw_symbol |= matches!(cell, Expr::Symbol(_));
        if !levels.contains(&cell) {
            levels.push(cell);
        }
    }
    saw_symbol.then_some(levels.len())
}

/// How many `NA` markers a value holds (recursing through matrices and
/// structs — the shapes imports produce).
fn count_missing(e: &Expr) -> usize {
    match e {
        Expr::Matrix(rows) => rows
            .iter()
            .flatten()
            .filter(|c| crate::expr::is_missing(c))
            .count(),
        Expr::Struct(fields) => fields.iter().map(|(_, v)| count_missing(v)).sum(),
        other if crate::expr::is_missing(other) => 1,
        _ => 0,
    }
}

fn describe_short(e: &Expr) -> String {
    match e {
        Expr::Matrix(rows) => {
            let base = format!("{}×{} matrix", rows.len(), rows[0].len());
            // A column holding symbols is a categorical column — say so, and
            // say how many levels, so a typo'd file is visible at import.
            match categorical_levels(rows) {
                Some(1) => format!("{}, categorical (1 level)", base),
                Some(k) => format!("{}, categorical ({} levels)", base, k),
                None => base,
            }
        }
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
        // the certified bounds survive export/import losslessly. Big bounds
        // write as exact decimal strings (a binary float terminates in
        // decimal) — also lossless, just bulkier.
        Expr::Signal(s) => encode_signal(s)?,
        Expr::Int(i) => number_from_text(&i.to_string()),
        Expr::Rat(r) => match rat_to_decimal(r) {
            // Decimal-friendly denominators (2^a·5^b) write as plain numbers.
            Some(dec) => number_from_text(&dec),
            None => json!({ "t": "rat", "v": format!("{}/{}", r.numer(), r.denom()) }),
        },
        Expr::Float(bf, digits) => {
            let r = float_to_rational(bf)
                .ok_or_else(|| "cannot export a non-finite float".to_string())?;
            // A binary float always terminates in decimal, but rat_to_decimal
            // caps the digit count — N(10^(-200000)) would exceed it, and an
            // expect() here took down the whole (wasm) session.
            let dec = rat_to_decimal(&r).ok_or_else(|| {
                "cannot export: this float's exact decimal form is too long \
                 (over 100000 digits)"
                    .to_string()
            })?;
            json!({ "t": "float", "v": dec, "digits": digits })
        }
        Expr::Const(Constant::Pi) => json!({ "t": "const", "v": "pi" }),
        Expr::Const(Constant::E) => json!({ "t": "const", "v": "e" }),
        Expr::Symbol(s) => json!({ "t": "sym", "v": s }),
        Expr::Str(s) => json!({ "t": "str", "v": s }),
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
        Expr::Formula(l, r) => json!({ "t": "formula", "lhs": encode(l)?, "rhs": encode(r)? }),
        Expr::Function { params, body, env } => {
            let body = serde_json::to_value(body.as_ref())
                .map_err(|e| format!("could not serialize function body: {}", e))?;
            let env = env
                .iter()
                .map(|(n, v)| Ok(json!([n, encode(v)?])))
                .collect::<Result<Vec<_>, String>>()?;
            json!({ "t": "function", "params": params, "body": body, "env": env })
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

/// Encode a signal. f64 bounds ride as plain JSON numbers (serde round-trips
/// f64 exactly), Big bounds as exact decimal strings, and a complex signal as
/// its two encoded real parts under `kind: "complex"`.
fn encode_signal(s: &crate::signal::SignalData) -> Result<Value, String> {
    use crate::signal::SignalData;
    if let SignalData::Complex { re, im } = s {
        return Ok(json!({
            "t": "signal",
            "kind": "complex",
            "re": encode_signal(re)?,
            "im": encode_signal(im)?,
        }));
    }
    Ok(match crate::signal::big_decimal_bounds(s) {
        Some(bounds) => {
            let b = bounds?;
            json!({ "t": "signal", "digits": b.digits, "lo": b.lo, "hi": b.hi })
        }
        None => match s {
            SignalData::F64 { lo, hi } => json!({ "t": "signal", "lo": lo, "hi": hi }),
            _ => unreachable!("big/complex handled above"),
        },
    })
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
        let value =
            decode(value, Mode::Tagged).map_err(|e| format!("variable '{}': {}", name, e))?;
        fields.push((name.to_string(), value));
    }
    structure(fields)
}

fn decode(v: &Value, mode: Mode) -> Result<Expr, String> {
    match v {
        Value::Number(n) => decimal_to_rat(&n.to_string()).map(rat_to_expr),
        Value::Bool(b) => Ok(Expr::Bool(*b)),
        // A JSON `null` in a generic data file is a missing value — the same
        // `NA` marker a blank CSV cell imports as. The tagged surd-data format
        // never writes nulls, so there it stays an error.
        Value::Null => match mode {
            Mode::Generic => Ok(missing()),
            Mode::Tagged => Err("null values are not supported".into()),
        },
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
        map.get(k)
            .ok_or_else(|| format!("'{}' value has no '{}'", tag, k))
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
        "str" => Ok(Expr::Str(text("v")?.to_string())),
        // Smart constructors re-canonicalize, so a hand-edited file can't
        // smuggle in values that violate the engine's invariants.
        "add" => Ok(add(dec_args("args")?)),
        "mul" => Ok(mul(dec_args("args")?)),
        "pow" => Ok(pow(dec("base")?, dec("exp")?)),
        "func" => Ok(func(text("name")?, dec_args("args")?)),
        "complex" => Ok(complex(dec("re")?, dec("im")?)),
        "eq" => Ok(Expr::Equation(Box::new(dec("lhs")?), Box::new(dec("rhs")?))),
        "formula" => Ok(Expr::Formula(Box::new(dec("lhs")?), Box::new(dec("rhs")?))),
        "signal" => {
            // Complex: two nested real signals.
            if map.get("kind").and_then(Value::as_str) == Some("complex") {
                let part = |k: &str| -> Result<crate::signal::SignalData, String> {
                    match decode(field(k)?, Mode::Tagged)? {
                        Expr::Signal(s) => Ok((*s).clone()),
                        _ => Err(format!("'signal' {} part must be a signal", k)),
                    }
                };
                return Ok(Expr::Signal(Rc::new(crate::signal::complex(
                    part("re")?,
                    part("im")?,
                )?)));
            }
            // Arbitrary precision (decimal-string bounds + digits)…
            if let Some(d) = map.get("digits") {
                let digits = d
                    .as_u64()
                    .ok_or("'signal' digits must be a positive integer")?
                    as usize;
                let lo: Vec<String> = serde_json::from_value(field("lo")?.clone())
                    .map_err(|_| "'signal' lo must be an array of decimal strings".to_string())?;
                let hi: Vec<String> = serde_json::from_value(field("hi")?.clone())
                    .map_err(|_| "'signal' hi must be an array of decimal strings".to_string())?;
                return Ok(Expr::Signal(Rc::new(
                    crate::signal::big_from_decimal_bounds(&lo, &hi, digits)?,
                )));
            }
            // …or f64 (plain JSON numbers).
            let lo: Vec<f64> = serde_json::from_value(field("lo")?.clone())
                .map_err(|_| "'signal' lo must be an array of numbers".to_string())?;
            let hi: Vec<f64> = serde_json::from_value(field("hi")?.clone())
                .map_err(|_| "'signal' hi must be an array of numbers".to_string())?;
            if lo.len() != hi.len() {
                return Err("'signal' lo and hi must have the same length".into());
            }
            if lo
                .iter()
                .zip(&hi)
                .any(|(l, h)| !l.is_finite() || !h.is_finite() || l > h)
            {
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
            // The captured environment; absent in workspaces saved before
            // closures existed (an empty capture is the compatible reading).
            let mut env = Vec::new();
            if let Some(pairs) = map.get("env") {
                let pairs = pairs
                    .as_array()
                    .ok_or("'env' of 'function' must be an array")?;
                for pair in pairs {
                    let (n, v) = match pair.as_array().map(Vec::as_slice) {
                        Some([n, v]) => (n, v),
                        _ => return Err("'env' entries must be [name, value] pairs".into()),
                    };
                    let n = n
                        .as_str()
                        .ok_or("'env' entry names must be strings")?
                        .to_string();
                    env.push((n, decode(v, Mode::Tagged)?));
                }
            }
            Ok(Expr::Function {
                params,
                body: Rc::new(body),
                env,
            })
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

/// The missing-value marker `NA`, as data cells import it.
fn missing() -> Expr {
    Expr::Symbol("NA".into())
}

/// The cell spellings that mean "no value here": an empty cell and the usual
/// markers (any letter case). These import as the symbol `NA`, which the
/// statistical functions refuse until `data.dropna(...)` handles it.
fn is_missing_cell(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "" | "na" | "n/a" | "nan" | "null" | "?"
    )
}

/// Does this cell *look* like it was meant to be a number (it starts the way
/// numbers do)? Such a cell that then fails to parse — `3.4O`, `1.2.3`,
/// `2024-01-01` — is a loud error, never a category: silently turning a
/// typo'd numeric column into a many-level categorical would be exactly the
/// kind of well-formed nonsense surd exists to refuse.
fn looks_numeric(s: &str) -> bool {
    s.trim()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
}

/// A data cell: the `NA` marker for a missing cell, a number parsed exactly
/// from its literal text, or — for word-like text (`us`, `treated`) — a
/// symbol, i.e. a categorical level, the same value a hand-built
/// `[us; eu; us]` column holds. `None` is the numeric-looking-but-malformed
/// case, which the caller reports as an error with the cell's location.
fn csv_cell(s: &str) -> Option<Expr> {
    if is_missing_cell(s) {
        return Some(missing());
    }
    if let Ok(r) = decimal_to_rat(s) {
        return Some(rat_to_expr(r));
    }
    if looks_numeric(s) {
        None
    } else {
        Some(Expr::Symbol(s.to_string()))
    }
}

/// CSV with a header row becomes a struct of column vectors; an all-numeric
/// headerless file becomes a plain matrix. Cells parse from their literal
/// text into exact rationals (scientific notation included);
/// blank/`NA`/`NaN`/`null` cells become the missing marker `NA`; word-like
/// text cells become symbols — categorical levels, ready for `data.dummy`,
/// `data.groupby`, or a model formula.
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

    // A header is a first row with word-like text in it. A leading
    // `1, NA, 3` row is data; a numeric-looking-but-malformed first cell is
    // also read as data, so it errors below with its location instead of
    // silently becoming a column name.
    let has_header = records[0]
        .iter()
        .any(|c| matches!(csv_cell(c), Some(Expr::Symbol(s)) if s != "NA"));
    if !has_header {
        let rows = records
            .iter()
            .enumerate()
            .map(|(i, r)| {
                r.iter()
                    .enumerate()
                    .map(|(j, c)| {
                        csv_cell(c).ok_or_else(|| {
                            format!("row {}, cell {}: '{}' is not a number", i + 1, j + 1, c)
                        })
                    })
                    .collect()
            })
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
            let value = csv_cell(cell).ok_or_else(|| {
                format!(
                    "row {}, column '{}': '{}' is not a number",
                    i + 1,
                    records[0][j],
                    cell
                )
            })?;
            col.push(vec![value]);
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
        record.push(if quoted {
            done
        } else {
            done.trim().to_string()
        });
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
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
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
///
/// Public because it is the *only* sound route from a fractional decimal
/// string to a number (`BigFloat::parse` mispositions the decimal point of
/// long fractional strings on wasm32) — external callers (the wasm bindings)
/// must come through here too.
pub fn decimal_to_rat(s: &str) -> Result<BigRational, String> {
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
    if !int_part
        .chars()
        .chain(frac_part.chars())
        .all(|c| c.is_ascii_digit())
    {
        return Err(bad());
    }
    // checked: `1.5e-9223372036854775808` would overflow the subtraction.
    let scale = exp
        .checked_sub(frac_part.len() as i64)
        .ok_or_else(|| format!("number '{}' is too large to represent", s))?;
    if int_part.len() + frac_part.len() > MAX_DECIMAL_DIGITS
        || scale.unsigned_abs() > MAX_DECIMAL_DIGITS as u64
    {
        return Err(format!("number '{}' is too large to represent", s));
    }
    let mut numer: BigInt = format!("{}{}", int_part, frac_part)
        .parse()
        .map_err(|_| bad())?;
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
    Some(if r.is_negative() {
        format!("-{}", s)
    } else {
        s
    })
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
            "10^40", // beyond u64
            "1/3",   // non-decimal rational -> tagged
            "-3/2",  // decimal-friendly rational -> plain number
            "true",
            "pi + e",         // constants inside a sum
            "sqrt(2)",        // 2^(1/2)
            "1 + 2*x + x^2",  // symbolic polynomial
            "sin(y) * ln(y)", // function applications
            "[1, 2; 3, 4]",
            "[1; 2; 3]",
            "2 + 3*I",
            "x^2 = 4", // equation
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
            assert_eq!(
                format!("{}", back),
                format!("{}", v),
                "float text changed for {}",
                src
            );
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
        assert_eq!(
            format!("{}", import("[1, 2.5, 3e2]").unwrap()),
            "[   1 ]\n[ 5/2 ]\n[ 300 ]"
        );
        assert_eq!(import("[[1, 2], [3, 4]]").unwrap(), val("[1, 2; 3, 4]"));
        // Awkward keys are bent into identifiers.
        let v = import(r#"{"sensor 1": 5, "2nd": 6}"#).unwrap();
        assert_eq!(format!("{}", v), "struct(_2nd = 6, sensor_1 = 5)");
        // A null is a missing value (the NA marker); empty arrays refuse loudly.
        assert_eq!(
            format!("{}", import(r#"{"a": null}"#).unwrap()),
            "struct(a = NA)"
        );
        assert!(import("[]").is_err());
        assert!(import(r#"{"a": [1, [2]]}"#).is_err());
    }

    #[test]
    fn csv_with_header_becomes_struct_of_columns() {
        let v = import("t, value\n0, 1.5\n1, 2.5e-1\n2, -3\n").unwrap();
        let Expr::Struct(fields) = &v else {
            panic!("expected struct")
        };
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
        // A malformed numeric data cell errors with its location. (A word
        // like `oops` would import as a categorical level; `1.2.3` looks
        // numeric and must not.)
        let err = import("t, v\n1, 1.2.3\n").unwrap_err();
        assert!(
            err.contains("row 2") && err.contains("'v'") && err.contains("1.2.3"),
            "{}",
            err
        );
        // Ragged rows error with the row number.
        assert!(import("a, b\n1\n").unwrap_err().contains("row 2"));
        assert!(import("").is_err());
    }

    #[test]
    fn decimal_text_helpers() {
        assert_eq!(
            decimal_to_rat("0.1").unwrap(),
            BigRational::new(1.into(), 10.into())
        );
        assert_eq!(
            decimal_to_rat("-1.5e-3").unwrap(),
            BigRational::new((-3).into(), 2000.into())
        );
        assert_eq!(
            decimal_to_rat("+2e3").unwrap(),
            BigRational::from_integer(2000.into())
        );
        assert!(decimal_to_rat("nope").is_err());
        assert!(decimal_to_rat("1e999999999").is_err());

        let r = |n: i64, d: i64| BigRational::new(n.into(), d.into());
        assert_eq!(rat_to_decimal(&r(1, 10)).unwrap(), "0.1");
        assert_eq!(rat_to_decimal(&r(-3, 2)).unwrap(), "-1.5");
        assert_eq!(rat_to_decimal(&r(7, 1)).unwrap(), "7");
        assert_eq!(rat_to_decimal(&r(1, 8)).unwrap(), "0.125");
        assert_eq!(rat_to_decimal(&r(1, 3)), None);
        // Sanity: text -> rational -> text is stable.
        assert_eq!(
            rat_to_decimal(&decimal_to_rat("123.456").unwrap()).unwrap(),
            "123.456"
        );
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
    fn missing_csv_cells_import_as_na() {
        // Blank cells and the usual markers, in any case, become `NA`.
        let v = import("t, value\n0, 1.5\n1,\n2, NA\n3, nan\n4, NULL\n5, ?\n").unwrap();
        let Expr::Struct(fields) = &v else {
            panic!("expected struct")
        };
        assert_eq!(fields[1].0, "value");
        assert_eq!(
            format!("{}", fields[1].1)
                .split_whitespace()
                .collect::<Vec<_>>(),
            [
                "[", "3/2", "]", "[", "NA", "]", "[", "NA", "]", "[", "NA", "]", "[", "NA", "]",
                "[", "NA", "]"
            ]
        );
        // The import summary counts what came in.
        assert_eq!(
            describe(&v),
            "struct with 2 fields: t (6×1 matrix), value (6×1 matrix) — 5 missing values (NA)"
        );
        // A first row of data with an NA is data, not a header.
        assert_eq!(import("1, NA\n2, 3\n").unwrap(), val("[1, NA; 2, 3]"));
    }

    #[test]
    fn text_csv_cells_import_as_categories() {
        // Word-like cells become symbols — categorical levels, the same
        // values a hand-built [us; eu; us] column holds.
        let v = import("id, origin\n1, us\n2, eu\n3, us\n").unwrap();
        let Expr::Struct(fields) = &v else {
            panic!("expected struct")
        };
        assert_eq!(fields[1].0, "origin");
        assert_eq!(fields[1].1, val("[us; eu; us]"));
        // The import summary names the categorical columns and their levels.
        assert_eq!(
            describe(&v),
            "struct with 2 fields: id (3×1 matrix), origin (3×1 matrix, categorical (2 levels))"
        );
        // Missing markers mix in without becoming a level of their own.
        let v = import("origin\nus\nNA\neu\nus\n").unwrap();
        assert_eq!(
            describe(&v),
            "struct with 1 field: origin (4×1 matrix, categorical (2 levels)) — 1 missing value (NA)"
        );
        // A numeric-looking cell that doesn't parse is a loud, located error —
        // a typo'd number must never silently become a category.
        let err = import("t, v\n1, 3.4O\n").unwrap_err();
        assert!(
            err.contains("row 2") && err.contains("'v'") && err.contains("3.4O"),
            "{}",
            err
        );
        assert!(import("t, v\n1, 2024-01-01\n").is_err());
        // An all-text headerless file: the first row is read as the header
        // (that ambiguity is fundamental to CSV), so these become columns.
        let v = import("x, y\nred, blue\n").unwrap();
        let Expr::Struct(fields) = &v else {
            panic!("expected struct")
        };
        assert_eq!(fields[0].1, val("[red]"));
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
        return Err(format!(
            "raw data too large ({} samples; cap {})",
            n, MAX_BULK_SAMPLES
        ));
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

/// Parse a headerless little-endian array of *interleaved* I/Q samples
/// (`[I0, Q0, I1, Q1, …]`) into a complex signal. `cf32` is interleaved f32
/// (the GNU Radio `.cfile`/`.cf32` format), `cf64` interleaved f64.
pub fn import_raw_iq(bytes: &[u8], format: &str) -> Result<Expr, String> {
    let width = match format {
        "cf32" => 4,
        "cf64" => 8,
        other => {
            return Err(format!(
                "unknown IQ format '{}' (supported: cf32, cf64; interleaved little-endian)",
                other
            ))
        }
    };
    let frame = width * 2; // one complex sample = I + Q
    if bytes.is_empty() || !bytes.len().is_multiple_of(frame) {
        return Err(format!(
            "IQ data length {} is not a multiple of {} bytes (interleaved I/Q pairs)",
            bytes.len(),
            frame
        ));
    }
    let n = bytes.len() / frame;
    if n > MAX_BULK_SAMPLES {
        return Err(format!(
            "IQ data too large ({} complex samples; cap {})",
            n, MAX_BULK_SAMPLES
        ));
    }
    let read = |b: &[u8]| -> f64 {
        match format {
            "cf64" => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        }
    };
    let mut re = Vec::with_capacity(n);
    let mut im = Vec::with_capacity(n);
    for i in 0..n {
        let base = i * frame;
        let iv = read(&bytes[base..base + width]);
        let qv = read(&bytes[base + width..base + frame]);
        if !iv.is_finite() || !qv.is_finite() {
            return Err(format!("non-finite value at complex sample {}", i + 1));
        }
        re.push(iv);
        im.push(qv);
    }
    let real = |v: Vec<f64>| {
        let hi = v.clone(); // exact conversions: point intervals
        crate::signal::SignalData::F64 { lo: v, hi }
    };
    Ok(Expr::Signal(Rc::new(crate::signal::complex(
        real(re),
        real(im),
    )?)))
}

// ---------------------------------------------------------------------------
// MATLAB MAT-files (the level-5 container: MATLAB v5/v6/v7)
// ---------------------------------------------------------------------------

// Storage type tags (mi*) from the MAT-file level-5 spec. Storage is
// independent of an array's class: MATLAB stores a double array in the
// narrowest integer type that holds it losslessly.
const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_SINGLE: u32 = 7;
const MI_DOUBLE: u32 = 9;
const MI_INT64: u32 = 12;
const MI_UINT64: u32 = 13;
const MI_MATRIX: u32 = 14;
const MI_COMPRESSED: u32 = 15;
const MI_UTF8: u32 = 16;
const MI_UTF16: u32 = 17;
const MI_UTF32: u32 = 18;

/// Cell cap for the exact-value paths (2-D matrices, vectors carrying `NaN`,
/// 64-bit integers beyond 2^53) — every cell is a bignum, so these are far
/// more expensive than packed signals. Bulk 1-D data rides
/// [`MAX_BULK_SAMPLES`] as a signal instead.
const MAX_MAT_EXACT_CELLS: usize = 1 << 16;

/// Inflation cap per compressed variable: the largest payload a variable at
/// the sample cap can need (complex double = 16 bytes/sample) plus headroom
/// for tags/names, so a zlib bomb can't balloon past what a legitimate file
/// could hold.
const MAX_MAT_INFLATED: usize = MAX_BULK_SAMPLES * 16 + (1 << 16);

/// Parse a MATLAB MAT-file (the level-5 binary container: MATLAB v5/v6/v7,
/// including v7's zlib-compressed variables) into a struct of its variables.
///
/// Every supported value imports *exactly* — the payload is binary IEEE
/// floats and integers, so nothing is re-rounded. Vectors become packed
/// point-interval signals; 2-D matrices, scalars, and 64-bit integers beyond
/// 2^53 become exact rationals/integers; `NaN` becomes the `NA` missing
/// marker (routing its array to the exact path — signals can't hold `NA`);
/// char rows become strings; 1×1 structs recurse. Anything the mapping can't
/// represent faithfully — cell arrays, sparse, N-d, objects, `Inf` — is a
/// named refusal, never a guess. v4 and v7.3 (HDF5) files are refused with a
/// pointer at `save -v7`.
pub fn import_mat(bytes: &[u8]) -> Result<Expr, String> {
    // v7.3 is HDF5; MATLAB writes the level-5 text header in front of it,
    // but tools that repack (h5repack etc.) leave the bare HDF5 signature.
    if bytes.starts_with(b"\x89HDF\r\n\x1a\n") {
        return Err("this is a MATLAB v7.3 (HDF5) file — re-save it with `save -v7`".into());
    }
    let le = match bytes.get(126..128) {
        Some(b"IM") => true,
        Some(b"MI") => false,
        // Level-4 files have no text header: they open with a small
        // little-endian type code (M·1000 + O·100 + P·10 + T, all digits
        // small) followed by row/column counts.
        _ if bytes.len() >= 20
            && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) < 5000 =>
        {
            return Err(
                "this looks like a MATLAB v4 MAT-file, which is not supported — re-save it \
                 with `save -v7`"
                    .into(),
            )
        }
        _ => return Err("not a MAT-file (missing the level-5 header)".into()),
    };
    match mat_u16(bytes, 124, le)? {
        0x0100 => {}
        0x0200 => {
            return Err(
                "MATLAB v7.3 MAT-files are HDF5, which is not supported — re-save with \
                 `save -v7`"
                    .into(),
            )
        }
        other => return Err(format!("unsupported MAT-file version 0x{:04x}", other)),
    }
    let mut vars: Vec<(String, Expr)> = Vec::new();
    let mut at = 128usize;
    while at < bytes.len() {
        // Writers disagree on whether a compressed element pads to the
        // 8-byte boundary; tolerate zero padding between elements.
        if at % 8 != 0 && bytes[at] == 0 {
            at += 1;
            continue;
        }
        let (ty, data, next) = mat_element(bytes, at, le)?;
        at = next;
        let inflated;
        let (ty, data) = if ty == MI_COMPRESSED {
            inflated =
                miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, MAX_MAT_INFLATED)
                    .map_err(|_| {
                        "corrupt or oversized compressed variable in MAT-file".to_string()
                    })?;
            let (t, d, _) = mat_element(&inflated, 0, le)?;
            (t, d)
        } else {
            (ty, data)
        };
        if ty != MI_MATRIX {
            return Err(format!(
                "unexpected MAT-file element type {} at the top level",
                ty
            ));
        }
        let (name, value) = mat_matrix(data, le, 0)?;
        let name = if is_valid_var_name(&name) {
            name
        } else {
            format!("var{}", vars.len() + 1)
        };
        vars.push((name, value));
    }
    if vars.is_empty() {
        return Err("the MAT-file holds no variables".into());
    }
    structure(vars)
}

fn mat_bytes<const W: usize>(d: &[u8], at: usize) -> Result<[u8; W], String> {
    let end = at
        .checked_add(W)
        .ok_or_else(|| "truncated MAT-file".to_string())?;
    Ok(d.get(at..end)
        .ok_or_else(|| "truncated MAT-file".to_string())?
        .try_into()
        .expect("slice of length W"))
}

fn mat_u16(d: &[u8], at: usize, le: bool) -> Result<u16, String> {
    let b = mat_bytes::<2>(d, at)?;
    Ok(if le {
        u16::from_le_bytes(b)
    } else {
        u16::from_be_bytes(b)
    })
}

fn mat_u32(d: &[u8], at: usize, le: bool) -> Result<u32, String> {
    let b = mat_bytes::<4>(d, at)?;
    Ok(if le {
        u32::from_le_bytes(b)
    } else {
        u32::from_be_bytes(b)
    })
}

/// One tagged data element at `at`: `(storage type, payload, offset of the
/// next element)`. Handles both the 8-byte tag and the packed "small data
/// element" form (type and byte count share the tag's first word, payload in
/// its second). Payloads pad to the next 8-byte boundary — except compressed
/// ones, whose zlib streams sit back-to-back at their exact length.
fn mat_element(d: &[u8], at: usize, le: bool) -> Result<(u32, &[u8], usize), String> {
    let trunc = || "truncated MAT-file element".to_string();
    let word = mat_u32(d, at, le)?;
    if word >> 16 != 0 {
        let size = (word >> 16) as usize;
        if size > 4 {
            return Err("malformed MAT-file element (small tag claims > 4 bytes)".into());
        }
        let data = d.get(at + 4..at + 4 + size).ok_or_else(trunc)?;
        return Ok((word & 0xffff, data, at + 8));
    }
    let size = mat_u32(d, at + 4, le)? as usize;
    let start = at + 8;
    let end = start.checked_add(size).ok_or_else(trunc)?;
    let data = d.get(start..end).ok_or_else(trunc)?;
    let next = if word == MI_COMPRESSED {
        end
    } else {
        start
            .checked_add(size.checked_next_multiple_of(8).ok_or_else(trunc)?)
            .ok_or_else(trunc)?
    };
    Ok((word, data, next))
}

/// A numeric payload, decoded exactly: IEEE floats (every f32 widens to f64
/// losslessly) or integers (i128 holds every integer class incl. uint64).
enum MatNums {
    Floats(Vec<f64>),
    Ints(Vec<i128>),
}

impl MatNums {
    fn len(&self) -> usize {
        match self {
            MatNums::Floats(v) => v.len(),
            MatNums::Ints(v) => v.len(),
        }
    }
}

fn mat_decode(ty: u32, d: &[u8], le: bool) -> Result<MatNums, String> {
    fn chunks<const W: usize>(d: &[u8]) -> Result<impl Iterator<Item = [u8; W]> + '_, String> {
        if !d.len().is_multiple_of(W) {
            return Err(format!(
                "numeric data length {} is not a multiple of the {}-byte sample width",
                d.len(),
                W
            ));
        }
        Ok(d.chunks_exact(W).map(|c| c.try_into().expect("W bytes")))
    }
    let e16 = move |b: [u8; 2]| {
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let e32 = move |b: [u8; 4]| {
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let e64 = move |b: [u8; 8]| {
        if le {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        }
    };
    Ok(match ty {
        MI_INT8 => MatNums::Ints(d.iter().map(|&b| (b as i8).into()).collect()),
        MI_UINT8 => MatNums::Ints(d.iter().map(|&b| b.into()).collect()),
        MI_INT16 => MatNums::Ints(chunks::<2>(d)?.map(|b| (e16(b) as i16).into()).collect()),
        MI_UINT16 => MatNums::Ints(chunks::<2>(d)?.map(|b| e16(b).into()).collect()),
        MI_INT32 => MatNums::Ints(chunks::<4>(d)?.map(|b| (e32(b) as i32).into()).collect()),
        MI_UINT32 => MatNums::Ints(chunks::<4>(d)?.map(|b| e32(b).into()).collect()),
        MI_INT64 => MatNums::Ints(chunks::<8>(d)?.map(|b| (e64(b) as i64).into()).collect()),
        MI_UINT64 => MatNums::Ints(chunks::<8>(d)?.map(|b| e64(b).into()).collect()),
        MI_SINGLE => MatNums::Floats(
            chunks::<4>(d)?
                .map(|b| f32::from_bits(e32(b)) as f64)
                .collect(),
        ),
        MI_DOUBLE => MatNums::Floats(chunks::<8>(d)?.map(|b| f64::from_bits(e64(b))).collect()),
        other => {
            return Err(format!(
                "unsupported numeric storage type {} in MAT-file",
                other
            ))
        }
    })
}

/// Decode a char array's payload. MATLAB writes UTF-16 code units as
/// miUINT16; other writers use the byte/UTF tags. Malformed text decodes
/// with visible replacement characters — text is presentation, not a
/// certified value, so lossy-but-loud beats a refusal here.
fn mat_decode_chars(ty: u32, d: &[u8], le: bool) -> Result<String, String> {
    Ok(match ty {
        MI_INT8 | MI_UINT8 | MI_UTF8 => String::from_utf8_lossy(d).into_owned(),
        MI_UINT16 | MI_UTF16 => {
            if !d.len().is_multiple_of(2) {
                return Err("char data length is not a multiple of 2 bytes".into());
            }
            let units = d.chunks_exact(2).map(|c| {
                let b = [c[0], c[1]];
                if le {
                    u16::from_le_bytes(b)
                } else {
                    u16::from_be_bytes(b)
                }
            });
            char::decode_utf16(units)
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        MI_UTF32 => {
            if !d.len().is_multiple_of(4) {
                return Err("char data length is not a multiple of 4 bytes".into());
            }
            d.chunks_exact(4)
                .map(|c| {
                    let b = [c[0], c[1], c[2], c[3]];
                    let v = if le {
                        u32::from_le_bytes(b)
                    } else {
                        u32::from_be_bytes(b)
                    };
                    char::from_u32(v).unwrap_or(char::REPLACEMENT_CHARACTER)
                })
                .collect()
        }
        other => {
            return Err(format!(
                "unsupported char storage type {} in MAT-file",
                other
            ))
        }
    })
}

/// The exact value of a finite f64 (a binary float is exactly m·2^k);
/// integral values come back as `Expr::Int`.
fn mat_exact(v: f64) -> Expr {
    rat_to_expr(BigRational::from_float(v).expect("finite by construction"))
}

/// (rows, cols) of a dims list already vetted as effectively 2-D.
fn mat_shape(dims: &[usize]) -> (usize, usize) {
    match dims {
        [] => (0, 0),
        [n] => (1, *n),
        [m, n, ..] => (*m, *n),
    }
}

/// Parse one miMATRIX body: `(array name, value)`. `depth` guards struct
/// recursion against crafted files.
fn mat_matrix(d: &[u8], le: bool, depth: usize) -> Result<(String, Expr), String> {
    if depth > 16 {
        return Err("MAT-file structs nest too deeply".into());
    }
    let (fty, flags, at) = mat_element(d, 0, le)?;
    if fty != MI_UINT32 || flags.len() < 8 {
        return Err("malformed MAT-file variable (bad array-flags element)".into());
    }
    let word = mat_u32(flags, 0, le)?;
    let class = word & 0xff;
    let is_complex = word & 0x0800 != 0;
    let is_logical = word & 0x0200 != 0;
    let (dty, dims_raw, at) = mat_element(d, at, le)?;
    if dty != MI_INT32 || !dims_raw.len().is_multiple_of(4) {
        return Err("malformed MAT-file variable (bad dimensions element)".into());
    }
    let mut dims = Vec::with_capacity(dims_raw.len() / 4);
    for i in 0..dims_raw.len() / 4 {
        let v = mat_u32(dims_raw, i * 4, le)? as i32;
        if v < 0 {
            return Err("malformed MAT-file variable (negative dimension)".into());
        }
        dims.push(v as usize);
    }
    let (nty, name_raw, at) = mat_element(d, at, le)?;
    if !matches!(nty, MI_INT8 | MI_UINT8 | MI_UTF8) {
        return Err("malformed MAT-file variable (bad name element)".into());
    }
    let name = String::from_utf8_lossy(name_raw)
        .trim_end_matches('\0')
        .to_string();
    let label = || {
        if name.is_empty() {
            "<unnamed>".to_string()
        } else {
            format!("'{}'", name)
        }
    };
    let numel = dims
        .iter()
        .try_fold(1usize, |a, &b| a.checked_mul(b))
        .filter(|&n| n <= MAX_BULK_SAMPLES)
        .ok_or_else(|| {
            format!(
                "variable {} is too large (the cap is {} values)",
                label(),
                MAX_BULK_SAMPLES
            )
        })?;
    // Trailing singleton dimensions are still 2-D data.
    let flat = dims.len() <= 2 || dims[2..].iter().all(|&x| x == 1);
    let value = match class {
        // mx classes: 1 cell, 2 struct, 3 object, 4 char, 5 sparse,
        // 6 double, 7 single, 8–15 int8…uint64.
        2 => mat_struct(d, at, le, numel, depth)?,
        4 => mat_char(d, at, le, &dims, numel, flat)?,
        6..=15 => {
            if !flat {
                return Err(format!(
                    "variable {} is an N-dimensional array — reshape to 2-D before saving",
                    label()
                ));
            }
            mat_numeric(d, at, le, &dims, numel, is_complex, is_logical)
                .map_err(|e| format!("variable {}: {}", label(), e))?
        }
        1 => {
            return Err(format!(
                "variable {} is a cell array — not supported",
                label()
            ))
        }
        3 => {
            return Err(format!(
                "variable {} is a MATLAB object — not supported",
                label()
            ))
        }
        5 => {
            return Err(format!(
                "variable {} is a sparse matrix — not supported (convert with `full`)",
                label()
            ))
        }
        other => {
            return Err(format!(
                "variable {} has unsupported MAT-file class {}",
                label(),
                other
            ))
        }
    };
    Ok((name, value))
}

fn mat_numeric(
    d: &[u8],
    at: usize,
    le: bool,
    dims: &[usize],
    numel: usize,
    is_complex: bool,
    is_logical: bool,
) -> Result<Expr, String> {
    if numel == 0 {
        // MATLAB `[]`: no value — the same marker a blank CSV cell imports as.
        return Ok(missing());
    }
    let (rty, rdata, at) = mat_element(d, at, le)?;
    let re = mat_decode(rty, rdata, le)?;
    if re.len() != numel {
        return Err(format!("declares {} values but holds {}", numel, re.len()));
    }
    if is_complex {
        let (ity, idata, _) = mat_element(d, at, le)?;
        let im = mat_decode(ity, idata, le)?;
        if im.len() != numel {
            return Err(format!(
                "declares {} values but holds {} imaginary parts",
                numel,
                im.len()
            ));
        }
        return mat_complex_value(mat_to_f64(re)?, mat_to_f64(im)?, dims);
    }
    match re {
        MatNums::Ints(v) => {
            if is_logical && numel == 1 {
                return Ok(Expr::Bool(v[0] != 0));
            }
            if v.iter().all(|x| x.unsigned_abs() <= 1 << 53) {
                mat_real_value(v.into_iter().map(|x| x as f64).collect(), dims)
            } else {
                // 64-bit integers beyond 2^53: exact, entry by entry.
                if numel == 1 {
                    return Ok(Expr::Int(BigInt::from(v[0])));
                }
                mat_exact_matrix(numel, dims, |i| Expr::Int(BigInt::from(v[i])))
            }
        }
        MatNums::Floats(v) => mat_real_value(v, dims),
    }
}

/// A real numeric array as a surd value: scalars exact, clean vectors as
/// packed point-interval signals, 2-D matrices (and vectors carrying `NaN`,
/// which signals can't hold) as exact cells with `NaN` → `NA`. `Inf` has no
/// exact or missing reading, so it refuses.
fn mat_real_value(v: Vec<f64>, dims: &[usize]) -> Result<Expr, String> {
    if v.iter().any(|x| x.is_infinite()) {
        return Err(
            "contains Inf, which surd has no exact value for — clean the data first".into(),
        );
    }
    if v.len() == 1 {
        return Ok(if v[0].is_nan() {
            missing()
        } else {
            mat_exact(v[0])
        });
    }
    let (rows, cols) = mat_shape(dims);
    let has_nan = v.iter().any(|x| x.is_nan());
    if (rows == 1 || cols == 1) && !has_nan {
        let hi = v.clone(); // exact decodes: point intervals
        return Ok(Expr::Signal(Rc::new(crate::signal::SignalData::F64 {
            lo: v,
            hi,
        })));
    }
    mat_exact_matrix(v.len(), dims, |i| {
        if v[i].is_nan() {
            missing()
        } else {
            mat_exact(v[i])
        }
    })
}

fn mat_complex_value(re: Vec<f64>, im: Vec<f64>, dims: &[usize]) -> Result<Expr, String> {
    if re
        .iter()
        .chain(im.iter())
        .any(|x| x.is_nan() || x.is_infinite())
    {
        return Err("complex data contains NaN/Inf — clean the data first".into());
    }
    if re.len() == 1 {
        return Ok(complex(mat_exact(re[0]), mat_exact(im[0])));
    }
    let (rows, cols) = mat_shape(dims);
    if rows == 1 || cols == 1 {
        let sig = |v: Vec<f64>| {
            let hi = v.clone(); // exact decodes: point intervals
            crate::signal::SignalData::F64 { lo: v, hi }
        };
        return Ok(Expr::Signal(Rc::new(crate::signal::complex(
            sig(re),
            sig(im),
        )?)));
    }
    mat_exact_matrix(re.len(), dims, |i| {
        complex(mat_exact(re[i]), mat_exact(im[i]))
    })
}

/// Build an exact matrix from column-major data (entry (r, c) is element
/// `c·rows + r`), honoring the exact-cell cap. A vector lands as its own
/// 1×n / n×1 shape.
fn mat_exact_matrix(
    numel: usize,
    dims: &[usize],
    cell: impl Fn(usize) -> Expr,
) -> Result<Expr, String> {
    if numel > MAX_MAT_EXACT_CELLS {
        return Err(format!(
            "too large to import exactly ({} cells; the cap is {}) — save clean numeric \
             vectors to import as signals",
            numel, MAX_MAT_EXACT_CELLS
        ));
    }
    let (rows, cols) = mat_shape(dims);
    if rows.checked_mul(cols) != Some(numel) || rows == 0 {
        return Err("malformed MAT-file variable (dimensions don't match data)".into());
    }
    let m = (0..rows)
        .map(|r| (0..cols).map(|c| cell(c * rows + r)).collect())
        .collect();
    Ok(Expr::Matrix(m))
}

/// Every part exactly as f64, or a refusal (complex integer data beyond
/// 2^53 — which no real writer produces — would otherwise round silently).
fn mat_to_f64(n: MatNums) -> Result<Vec<f64>, String> {
    match n {
        MatNums::Floats(v) => Ok(v),
        MatNums::Ints(v) => v
            .into_iter()
            .map(|x| {
                if x.unsigned_abs() <= 1 << 53 {
                    Ok(x as f64)
                } else {
                    Err("complex integer data beyond 2^53 is not supported".to_string())
                }
            })
            .collect(),
    }
}

fn mat_char(
    d: &[u8],
    at: usize,
    le: bool,
    dims: &[usize],
    numel: usize,
    flat: bool,
) -> Result<Expr, String> {
    if numel == 0 {
        return Ok(Expr::Str(String::new()));
    }
    let (rows, _) = mat_shape(dims);
    if !flat || rows > 1 {
        return Err(
            "char matrices are not supported — only single-row char arrays import (as strings)"
                .into(),
        );
    }
    let (ty, data, _) = mat_element(d, at, le)?;
    mat_decode_chars(ty, data, le).map(Expr::Str)
}

fn mat_struct(d: &[u8], at: usize, le: bool, numel: usize, depth: usize) -> Result<Expr, String> {
    if numel != 1 {
        return Err("struct arrays are not supported — only 1×1 structs import".into());
    }
    let (lty, len_raw, at) = mat_element(d, at, le)?;
    if lty != MI_INT32 || len_raw.len() < 4 {
        return Err("malformed MAT-file struct (bad field-name length)".into());
    }
    let flen = mat_u32(len_raw, 0, le)? as usize;
    if flen == 0 || flen > 4096 {
        return Err("malformed MAT-file struct (bad field-name length)".into());
    }
    let (nty, names_raw, mut at) = mat_element(d, at, le)?;
    if !matches!(nty, MI_INT8 | MI_UINT8) || !names_raw.len().is_multiple_of(flen) {
        return Err("malformed MAT-file struct (bad field names)".into());
    }
    let mut fields = Vec::with_capacity(names_raw.len() / flen);
    for (i, chunk) in names_raw.chunks_exact(flen).enumerate() {
        let name = String::from_utf8_lossy(chunk)
            .trim_end_matches('\0')
            .to_string();
        let (ty, body, next) = mat_element(d, at, le)?;
        if ty != MI_MATRIX {
            return Err("malformed MAT-file struct (field is not a matrix element)".into());
        }
        let (_, value) = mat_matrix(body, le, depth + 1)?;
        let name = if is_valid_var_name(&name) {
            name
        } else {
            format!("field{}", i + 1)
        };
        fields.push((name, value));
        at = next;
    }
    if fields.is_empty() {
        // `struct()` with no fields: no value, like `[]`.
        return Ok(missing());
    }
    structure(fields)
}

/// Which raw-binary export a value supports, so the UI only offers formats that
/// will work: `Some("real")` → f32/f64, `Some("complex")` → cf32/cf64, `None`
/// → not raw-exportable. Mirrors what [`export_raw`] accepts, but cheaply (no
/// f64 evaluation): a signal is classified by its variant; a matrix/scalar by
/// whether every entry is a numeric literal (and whether any is complex).
pub fn raw_export_kind(value: &Expr) -> Option<&'static str> {
    /// `Some(is_complex)` if `e` is a plain numeric literal, else `None`.
    fn numeric(e: &Expr) -> Option<bool> {
        match e {
            Expr::Int(_) | Expr::Rat(_) | Expr::Float(..) | Expr::Const(_) => Some(false),
            Expr::Complex(re, im) if numeric(re).is_some() && numeric(im).is_some() => Some(true),
            _ => None,
        }
    }
    let tag = |complex: bool| if complex { "complex" } else { "real" };
    match value {
        Expr::Signal(s) => Some(tag(crate::signal::is_complex(s))),
        Expr::Matrix(rows) => {
            let mut any_complex = false;
            for e in rows.iter().flatten() {
                any_complex |= numeric(e)?;
            }
            Some(tag(any_complex))
        }
        other => numeric(other).map(tag),
    }
}

/// Write a signal or numeric vector/matrix to raw little-endian binary.
/// `format` picks the width and real/complex shape: `f32`/`f64` for real data,
/// `cf32`/`cf64` for interleaved I/Q. This is a deliberate one-way exit from
/// certification — each sample collapses to its midpoint, then rounds to the
/// target width — so it's only valid on data that has a single value per slot.
pub fn export_raw(value: &Expr, format: &str) -> Result<Vec<u8>, String> {
    let (want_complex, narrow) = match format {
        "f32" => (false, true),
        "f64" => (false, false),
        "cf32" => (true, true),
        "cf64" => (true, false),
        other => {
            return Err(format!(
                "unknown export format '{}' (use f32, f64, cf32, cf64)",
                other
            ))
        }
    };
    let (re, im) = gather_streams(value)?;
    match (want_complex, &im) {
        (false, Some(_)) => return Err("this is complex data — export as cf32 or cf64".into()),
        (true, None) => {
            return Err("this is real data — export as f32 or f64 (cf32/cf64 are for I/Q)".into())
        }
        _ => {}
    }
    let mut out = Vec::new();
    let mut push = |v: f64| -> Result<(), String> {
        if narrow {
            let n = v as f32;
            // Casting past f32 range yields ±Inf — bytes that import (here
            // or anywhere else) then rejects. Refuse loudly instead of
            // writing a corrupt file.
            if n.is_infinite() && v.is_finite() {
                return Err(format!(
                    "value {v:e} does not fit in f32 — export as f64 instead"
                ));
            }
            out.extend_from_slice(&n.to_le_bytes());
        } else {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Ok(())
    };
    match im {
        None => {
            for v in &re {
                push(*v)?;
            }
        }
        Some(imv) => {
            for (r, m) in re.iter().zip(&imv) {
                push(*r)?;
                push(*m)?;
            }
        }
    }
    Ok(out)
}

/// The f64 value streams behind an exportable value: `(re, None)` for real
/// data, `(re, Some(im))` for complex. Signals contribute their sample
/// midpoints; a numeric matrix/vector its row-major entries (complex entries
/// split into re/im, real ones get a zero imaginary part only if the whole
/// collection is complex).
fn gather_streams(e: &Expr) -> Result<(Vec<f64>, Option<Vec<f64>>), String> {
    let scalar_f64 = |x: &Expr| crate::f64eval::eval_f64(x, &[]);
    let entry_parts = |x: &Expr| -> Result<(f64, f64), String> {
        match x {
            Expr::Complex(r, i) => Ok((scalar_f64(r)?, scalar_f64(i)?)),
            other => Ok((scalar_f64(other)?, 0.0)),
        }
    };
    match e {
        Expr::Signal(s) => Ok(crate::signal::midpoints_f64(s)),
        Expr::Matrix(rows) => {
            let entries: Vec<&Expr> = rows.iter().flatten().collect();
            if entries.iter().any(|x| matches!(x, Expr::Complex(..))) {
                let mut re = Vec::with_capacity(entries.len());
                let mut im = Vec::with_capacity(entries.len());
                for x in entries {
                    let (r, i) = entry_parts(x)?;
                    re.push(r);
                    im.push(i);
                }
                Ok((re, Some(im)))
            } else {
                let mut re = Vec::with_capacity(entries.len());
                for x in entries {
                    re.push(scalar_f64(x).map_err(|_| {
                        "matrix entries must be numeric to export to raw binary".to_string()
                    })?);
                }
                Ok((re, None))
            }
        }
        // A bare complex / numeric scalar exports as a single sample.
        Expr::Complex(..) => entry_parts(e).map(|(r, i)| (vec![r], Some(vec![i]))),
        _ => match scalar_f64(e) {
            Ok(v) => Ok((vec![v], None)),
            Err(_) => Err("only signals and numeric vectors/matrices export to raw binary".into()),
        },
    }
}

/// Parse CSV straight into packed signals (one per column) — the bulk path
/// for files too large for exact rationals. Integers within ±2^53 pack as
/// exact points; other decimals as certified ±1-ulp enclosures around the
/// correctly-rounded parse (Rust's float parsing is correctly rounded).
pub fn import_csv_packed(text: &str) -> Result<Expr, String> {
    // Same BOM rule as `import`: strip before parsing, or the first cell of
    // a headerless file reads as text and demotes the first row to a header.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().filter(|l| !l.trim().is_empty()).peekable();
    let first = *lines.peek().ok_or("the CSV file is empty")?;
    let cells = |l: &str| {
        l.split(',')
            .map(|c| c.trim().to_string())
            .collect::<Vec<_>>()
    };
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
                        format!(
                            "row {}, column {}: '{}' is not a number",
                            row + 2,
                            c + 1,
                            cell
                        )
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
        let Expr::Signal(s) = &fields[0].1 else {
            panic!()
        };
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
        let Expr::Signal(y) = &fields[1].1 else {
            panic!()
        };
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

    #[test]
    fn iq_import_deinterleaves() {
        // Interleaved [I0, Q0, I1, Q1, I2, Q2] as little-endian f32.
        let iq: [f32; 6] = [1.0, 2.0, 3.0, 4.0, -5.0, -6.0];
        let bytes: Vec<u8> = iq.iter().flat_map(|v| v.to_le_bytes()).collect();
        let Expr::Signal(s) = import_raw_iq(&bytes, "cf32").unwrap() else {
            panic!("complex signal")
        };
        let crate::signal::SignalData::Complex { re, im } = s.as_ref() else {
            panic!("complex variant")
        };
        let (
            crate::signal::SignalData::F64 { lo: rl, .. },
            crate::signal::SignalData::F64 { lo: il, .. },
        ) = (re.as_ref(), im.as_ref())
        else {
            panic!("f64 parts")
        };
        assert_eq!(rl, &[1.0, 3.0, -5.0]); // the I channel
        assert_eq!(il, &[2.0, 4.0, -6.0]); // the Q channel

        // A dangling half-pair (odd number of f32) is rejected.
        assert!(import_raw_iq(&bytes[..bytes.len() - 4], "cf32").is_err());
        assert!(import_raw_iq(&bytes, "bogus").is_err());
    }

    #[test]
    fn raw_binary_export_roundtrips() {
        // Real f32: import → export reproduces the bytes (values are exactly
        // f32-representable, so midpoint == value and f64→f32 is exact).
        let reals: [f32; 4] = [1.5, -2.25, 0.0, 7.0];
        let rbytes: Vec<u8> = reals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let sig = import_raw(&rbytes, "f32").unwrap();
        assert_eq!(export_raw(&sig, "f32").unwrap(), rbytes);

        // Interleaved I/Q cf32: same round-trip.
        let iq: [f32; 6] = [1.0, 2.0, 3.0, 4.0, -5.0, -6.0];
        let ibytes: Vec<u8> = iq.iter().flat_map(|v| v.to_le_bytes()).collect();
        let cz = import_raw_iq(&ibytes, "cf32").unwrap();
        assert_eq!(export_raw(&cz, "cf32").unwrap(), ibytes);
        // cf64 widens each f32 to 8 bytes: twice the length.
        assert_eq!(export_raw(&cz, "cf64").unwrap().len(), ibytes.len() * 2);

        // Shape mismatches are rejected with guidance.
        assert!(export_raw(&sig, "cf32").unwrap_err().contains("real data"));
        assert!(export_raw(&cz, "f32").unwrap_err().contains("complex data"));
        assert!(export_raw(&sig, "f16").is_err());

        // A numeric vector exports its entries (row-major).
        let v = crate::matrix::matrix(vec![
            vec![Expr::Int(BigInt::from(1))],
            vec![Expr::Int(BigInt::from(2))],
        ])
        .unwrap();
        assert_eq!(export_raw(&v, "f64").unwrap().len(), 16);
    }

    #[test]
    fn raw_export_kind_classifies() {
        let reals: [f32; 2] = [1.0, 2.0];
        let rbytes: Vec<u8> = reals.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(
            raw_export_kind(&import_raw(&rbytes, "f32").unwrap()),
            Some("real")
        );
        let iq: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
        let ibytes: Vec<u8> = iq.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(
            raw_export_kind(&import_raw_iq(&ibytes, "cf32").unwrap()),
            Some("complex")
        );
        // A real numeric vector and a bare scalar are real-exportable.
        let v = crate::matrix::matrix(vec![vec![Expr::Int(BigInt::from(1))]]).unwrap();
        assert_eq!(raw_export_kind(&v), Some("real"));
        assert_eq!(raw_export_kind(&Expr::Int(BigInt::from(7))), Some("real"));
        // A complex scalar and a vector holding one are complex.
        assert_eq!(
            raw_export_kind(&complex(
                Expr::Int(BigInt::from(1)),
                Expr::Int(BigInt::from(2))
            )),
            Some("complex")
        );
        // Non-numeric values offer no raw export.
        assert_eq!(raw_export_kind(&Expr::Symbol("x".into())), None);
        assert_eq!(raw_export_kind(&Expr::Bool(true)), None);
    }

    #[test]
    fn complex_signal_export_roundtrips() {
        // [1+2i, 3-4i] as an f64 complex signal, via export → import.
        let entries = vec![
            complex(Expr::Int(BigInt::from(1)), Expr::Int(BigInt::from(2))),
            complex(Expr::Int(BigInt::from(3)), Expr::Int(BigInt::from(-4))),
        ];
        let original = crate::signal::pack(&entries, None).unwrap();
        let sig = Expr::Signal(Rc::new(original.clone()));
        let json = export_variables(&[("z", &sig)]).unwrap();
        let back = import(&json).unwrap();
        let Expr::Struct(fields) = &back else {
            panic!()
        };
        let Expr::Signal(restored) = &fields[0].1 else {
            panic!("field is a signal")
        };
        assert_eq!(&original, restored.as_ref(), "complex signal round-trips");
    }
}

#[cfg(test)]
mod big_signal_tests {
    use super::*;
    use crate::expr::rat_to_expr;
    use crate::signal::{pack, SignalData};

    /// Export → import of an arbitrary-precision signal is the identity:
    /// the decimal bounds are exact, and re-parsing them at the signal's
    /// working precision recovers the same binary floats bit for bit.
    #[test]
    fn big_signal_export_roundtrips_losslessly() {
        let entries: Vec<Expr> = [(1i64, 3i64), (-2, 1), (5, 7)]
            .iter()
            .map(|(n, d)| rat_to_expr(BigRational::new(BigInt::from(*n), BigInt::from(*d))))
            .collect();
        let original = pack(&entries, Some(40)).unwrap();
        let sig = Expr::Signal(Rc::new(original.clone()));
        let json = export_variables(&[("s", &sig)]).unwrap();
        let back = import(&json).unwrap();
        let Expr::Struct(fields) = &back else {
            panic!("import wraps in a struct")
        };
        let Expr::Signal(restored) = &fields[0].1 else {
            panic!("field is a signal")
        };
        let (
            SignalData::Big {
                lo: a,
                hi: b,
                digits: d1,
            },
            SignalData::Big {
                lo: c,
                hi: e,
                digits: d2,
            },
        ) = (&original, restored.as_ref())
        else {
            panic!("both are Big")
        };
        assert_eq!(d1, d2);
        assert_eq!(a, c, "lo bounds must round-trip bit-exactly");
        assert_eq!(b, e, "hi bounds must round-trip bit-exactly");
    }
}

#[cfg(test)]
mod mat_tests {
    use super::*;
    use crate::signal::SignalData;
    use crate::Interpreter;

    fn val(src: &str) -> Expr {
        Interpreter::new().eval_line(src).expect(src)
    }

    // --- fixture builders (little-endian; MATLAB's own writer layout) ---

    fn tag(ty: u32, data: &[u8]) -> Vec<u8> {
        let mut v = ty.to_le_bytes().to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        while !v.len().is_multiple_of(8) {
            v.push(0);
        }
        v
    }

    /// The packed "small data element" form (≤ 4 payload bytes).
    fn small(ty: u32, data: &[u8]) -> Vec<u8> {
        assert!(data.len() <= 4);
        let word = ty | ((data.len() as u32) << 16);
        let mut v = word.to_le_bytes().to_vec();
        v.extend_from_slice(data);
        v.resize(8, 0);
        v
    }

    fn header() -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[..20].copy_from_slice(b"MATLAB 5.0 MAT-file ");
        v[124..126].copy_from_slice(&0x0100u16.to_le_bytes());
        v[126..128].copy_from_slice(b"IM");
        v
    }

    fn flags(class: u32, is_complex: bool, is_logical: bool) -> Vec<u8> {
        let mut w = class & 0xff;
        if is_complex {
            w |= 0x0800;
        }
        if is_logical {
            w |= 0x0200;
        }
        let mut data = w.to_le_bytes().to_vec();
        data.extend_from_slice(&[0; 4]);
        tag(MI_UINT32, &data)
    }

    fn dims(d: &[i32]) -> Vec<u8> {
        let data: Vec<u8> = d.iter().flat_map(|x| x.to_le_bytes()).collect();
        tag(MI_INT32, &data)
    }

    fn matrix_el(
        class: u32,
        is_complex: bool,
        is_logical: bool,
        shape: &[i32],
        name: &str,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut body = flags(class, is_complex, is_logical);
        body.extend(dims(shape));
        body.extend(tag(MI_INT8, name.as_bytes()));
        body.extend_from_slice(payload);
        tag(MI_MATRIX, &body)
    }

    fn doubles(vals: &[f64]) -> Vec<u8> {
        let data: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        tag(MI_DOUBLE, &data)
    }

    fn var_doubles(name: &str, shape: &[i32], vals: &[f64]) -> Vec<u8> {
        matrix_el(6, false, false, shape, name, &doubles(vals))
    }

    fn file(vars: &[Vec<u8>]) -> Vec<u8> {
        let mut v = header();
        for e in vars {
            v.extend_from_slice(e);
        }
        v
    }

    fn field<'a>(e: &'a Expr, name: &str) -> &'a Expr {
        let Expr::Struct(fields) = e else {
            panic!("expected a struct, got {}", e)
        };
        &fields
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no field '{}' in {}", name, e))
            .1
    }

    fn point_signal(e: &Expr) -> &[f64] {
        let Expr::Signal(s) = e else {
            panic!("expected a signal, got {}", e)
        };
        let SignalData::F64 { lo, hi } = &**s else {
            panic!("expected an f64 signal")
        };
        assert_eq!(lo, hi, "exact decodes are point intervals");
        lo
    }

    // --- the happy paths ---

    #[test]
    fn doubles_import_as_signal_and_exact_scalar() {
        let f = file(&[
            var_doubles("sig", &[1, 4], &[1.0, 2.5, -3.0, 4.0]),
            var_doubles("x", &[1, 1], &[0.5]),
        ]);
        let v = import_mat(&f).unwrap();
        assert_eq!(point_signal(field(&v, "sig")), &[1.0, 2.5, -3.0, 4.0]);
        assert_eq!(field(&v, "x"), &val("1/2"));
    }

    #[test]
    fn two_d_matrices_import_exactly_from_column_major() {
        // Column-major [1, 3, 1/2, 4] with dims 2×2 is [1, 1/2; 3, 4].
        let f = file(&[var_doubles("m", &[2, 2], &[1.0, 3.0, 0.5, 4.0])]);
        let v = import_mat(&f).unwrap();
        assert_eq!(field(&v, "m"), &val("[1, 1/2; 3, 4]"));
    }

    #[test]
    fn narrow_integer_storage_of_a_double_class_decodes() {
        // MATLAB stores double arrays in the narrowest lossless integer type.
        let data: Vec<u8> = [1i16, -2, 300]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let el = matrix_el(6, false, false, &[1, 3], "y", &tag(MI_INT16, &data));
        let v = import_mat(&file(&[el])).unwrap();
        assert_eq!(point_signal(field(&v, "y")), &[1.0, -2.0, 300.0]);
    }

    #[test]
    fn small_data_elements_parse() {
        // Short names ride the packed tag form in real MATLAB files.
        let mut body = flags(6, false, false);
        body.extend(dims(&[1, 1]));
        body.extend(small(MI_INT8, b"ab"));
        body.extend(doubles(&[7.0]));
        let v = import_mat(&file(&[tag(MI_MATRIX, &body)])).unwrap();
        assert_eq!(field(&v, "ab"), &val("7"));
    }

    #[test]
    fn complex_vectors_become_complex_signals() {
        let mut payload = doubles(&[1.0, 2.0]); // real parts
        payload.extend(doubles(&[-0.5, 3.0])); // imaginary parts
        let el = matrix_el(6, true, false, &[2, 1], "z", &payload);
        let v = import_mat(&file(&[el])).unwrap();
        let Expr::Signal(s) = field(&v, "z") else {
            panic!("expected a signal")
        };
        let SignalData::Complex { re, im } = &**s else {
            panic!("expected a complex signal")
        };
        let (SignalData::F64 { lo: r, .. }, SignalData::F64 { lo: i, .. }) = (&**re, &**im) else {
            panic!("f64 parts")
        };
        assert_eq!(
            (r.as_slice(), i.as_slice()),
            (&[1.0, 2.0][..], &[-0.5, 3.0][..])
        );
    }

    #[test]
    fn complex_scalars_import_exactly() {
        let mut payload = doubles(&[0.5]);
        payload.extend(doubles(&[-2.0]));
        let el = matrix_el(6, true, false, &[1, 1], "z", &payload);
        let v = import_mat(&file(&[el])).unwrap();
        assert_eq!(field(&v, "z"), &val("1/2 - 2*I"));
    }

    #[test]
    fn utf16_char_rows_import_as_strings() {
        let data: Vec<u8> = "héllo"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let el = matrix_el(4, false, false, &[1, 5], "s", &tag(MI_UINT16, &data));
        let v = import_mat(&file(&[el])).unwrap();
        assert_eq!(field(&v, "s"), &Expr::Str("héllo".into()));
    }

    #[test]
    fn logical_scalars_become_booleans() {
        let el = matrix_el(9, false, true, &[1, 1], "flag", &tag(MI_UINT8, &[1]));
        let v = import_mat(&file(&[el])).unwrap();
        assert_eq!(field(&v, "flag"), &Expr::Bool(true));
    }

    #[test]
    fn structs_recurse() {
        let mut body = flags(2, false, false);
        body.extend(dims(&[1, 1]));
        body.extend(tag(MI_INT8, b"s"));
        body.extend(tag(MI_INT32, &8i32.to_le_bytes()));
        let mut names = vec![0u8; 16];
        names[..1].copy_from_slice(b"a");
        names[8..10].copy_from_slice(b"bb");
        body.extend(tag(MI_INT8, &names));
        body.extend(matrix_el(6, false, false, &[1, 1], "", &doubles(&[0.5])));
        body.extend(matrix_el(
            6,
            false,
            false,
            &[1, 2],
            "",
            &doubles(&[1.0, 2.0]),
        ));
        let v = import_mat(&file(&[tag(MI_MATRIX, &body)])).unwrap();
        let s = field(&v, "s");
        assert_eq!(field(s, "a"), &val("1/2"));
        assert_eq!(point_signal(field(s, "bb")), &[1.0, 2.0]);
    }

    #[test]
    fn compressed_variables_import_including_unpadded_back_to_back() {
        let compress = |el: &[u8]| {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(el, 6);
            let mut v = MI_COMPRESSED.to_le_bytes().to_vec();
            v.extend_from_slice(&(z.len() as u32).to_le_bytes());
            v.extend_from_slice(&z);
            v // deliberately unpadded, like MATLAB's writer
        };
        let mut f = header();
        f.extend(compress(&var_doubles("a", &[1, 3], &[1.0, 2.0, 3.0])));
        f.extend(compress(&var_doubles("b", &[1, 1], &[9.0])));
        let v = import_mat(&f).unwrap();
        assert_eq!(point_signal(field(&v, "a")), &[1.0, 2.0, 3.0]);
        assert_eq!(field(&v, "b"), &val("9"));
    }

    #[test]
    fn big_endian_files_import() {
        let tag_be = |ty: u32, data: &[u8]| {
            let mut v = ty.to_be_bytes().to_vec();
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            while !v.len().is_multiple_of(8) {
                v.push(0);
            }
            v
        };
        let mut flags_data = 6u32.to_be_bytes().to_vec();
        flags_data.extend_from_slice(&[0; 4]);
        let mut body = tag_be(MI_UINT32, &flags_data);
        let dims_data: Vec<u8> = [1i32, 1].iter().flat_map(|x| x.to_be_bytes()).collect();
        body.extend(tag_be(MI_INT32, &dims_data));
        body.extend(tag_be(MI_INT8, b"x"));
        body.extend(tag_be(MI_DOUBLE, &2.5f64.to_be_bytes()));
        let mut f = vec![0u8; 128];
        f[..6].copy_from_slice(b"MATLAB");
        f[124..126].copy_from_slice(&0x0100u16.to_be_bytes());
        f[126..128].copy_from_slice(b"MI");
        f.extend(tag_be(MI_MATRIX, &body));
        let v = import_mat(&f).unwrap();
        assert_eq!(field(&v, "x"), &val("5/2"));
    }

    // --- exactness at the edges ---

    #[test]
    fn int64_beyond_2_pow_53_imports_as_exact_integers() {
        let big = (1i64 << 60) + 1; // not representable in f64
        let data: Vec<u8> = [big, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        let el = matrix_el(14, false, false, &[2, 1], "n", &tag(MI_INT64, &data));
        let v = import_mat(&file(&[el])).unwrap();
        let Expr::Matrix(rows) = field(&v, "n") else {
            panic!("expected an exact matrix")
        };
        assert_eq!(rows[0][0], Expr::Int(BigInt::from(big)));
        assert_eq!(rows[1][0], Expr::Int(BigInt::from(1)));
    }

    #[test]
    fn nan_routes_to_the_exact_path_as_na() {
        let f = file(&[var_doubles("v", &[1, 3], &[1.0, f64::NAN, 3.0])]);
        let v = import_mat(&f).unwrap();
        let Expr::Matrix(rows) = field(&v, "v") else {
            panic!("a NaN-carrying vector can't be a signal")
        };
        assert_eq!(rows[0], vec![val("1"), missing(), val("3")]);
        assert!(describe(&v).contains("1 missing value"));
    }

    #[test]
    fn inf_is_refused() {
        let f = file(&[var_doubles("v", &[1, 2], &[1.0, f64::INFINITY])]);
        let e = import_mat(&f).unwrap_err();
        assert!(e.contains("Inf"), "{}", e);
    }

    #[test]
    fn empty_arrays_import_as_the_missing_marker() {
        let f = file(&[
            matrix_el(6, false, false, &[0, 0], "e", &tag(MI_DOUBLE, &[])),
            var_doubles("x", &[1, 1], &[1.0]),
        ]);
        let v = import_mat(&f).unwrap();
        assert_eq!(field(&v, "e"), &missing());
    }

    #[test]
    fn invalid_variable_names_are_replaced() {
        let f = file(&[var_doubles("1bad", &[1, 1], &[3.0])]);
        let v = import_mat(&f).unwrap();
        assert_eq!(field(&v, "var1"), &val("3"));
    }

    // --- refusals and hostile bytes ---

    #[test]
    fn v73_and_v4_and_garbage_are_refused_with_pointers() {
        let mut v73 = header();
        v73[124..126].copy_from_slice(&0x0200u16.to_le_bytes());
        assert!(import_mat(&v73).unwrap_err().contains("7.3"));
        assert!(import_mat(b"\x89HDF\r\n\x1a\nrest")
            .unwrap_err()
            .contains("7.3"));
        assert!(import_mat(&[0u8; 64]).unwrap_err().contains("v4"));
        assert!(import_mat(b"PK\x03\x04 definitely not a mat file").is_err());
        let mut vbad = header();
        vbad[124..126].copy_from_slice(&0x0300u16.to_le_bytes());
        assert!(import_mat(&vbad).unwrap_err().contains("version"));
    }

    #[test]
    fn unsupported_classes_are_named_refusals() {
        let cell = matrix_el(1, false, false, &[1, 1], "c", &[]);
        assert!(import_mat(&file(&[cell])).unwrap_err().contains("cell"));
        let sparse = matrix_el(5, false, false, &[2, 2], "sp", &[]);
        assert!(import_mat(&file(&[sparse])).unwrap_err().contains("sparse"));
        let nd = var_doubles("t", &[2, 2, 2], &[0.0; 8]);
        assert!(import_mat(&file(&[nd]))
            .unwrap_err()
            .contains("N-dimensional"));
        let charmat = matrix_el(4, false, false, &[2, 2], "cm", &tag(MI_UINT16, &[0; 8]));
        assert!(import_mat(&file(&[charmat])).unwrap_err().contains("char"));
    }

    #[test]
    fn hostile_sizes_and_truncation_fail_loudly() {
        // Declared dimensions that multiply past any real payload.
        let huge = matrix_el(
            6,
            false,
            false,
            &[0x4000_0000, 0x4000_0000],
            "h",
            &doubles(&[]),
        );
        assert!(import_mat(&file(&[huge]))
            .unwrap_err()
            .contains("too large"));
        // An element whose size runs past the end of the file.
        let mut f = header();
        f.extend_from_slice(&MI_MATRIX.to_le_bytes());
        f.extend_from_slice(&1_000_000u32.to_le_bytes());
        f.extend_from_slice(&[0u8; 16]);
        assert!(import_mat(&f).unwrap_err().contains("truncated"));
        // Payload count disagreeing with the declared dimensions.
        let lying = var_doubles("l", &[1, 4], &[1.0, 2.0]);
        assert!(import_mat(&file(&[lying])).is_err());
        // A zlib bomb refuses at the inflation cap instead of allocating.
        let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 1 << 20], 6);
        let mut f = header();
        f.extend_from_slice(&MI_COMPRESSED.to_le_bytes());
        f.extend_from_slice(&(bomb.len() as u32).to_le_bytes());
        f.extend_from_slice(&bomb);
        // (1 MiB inflates fine — it's under the cap — but holds no valid
        // element, so it must still error, not panic.)
        assert!(import_mat(&f).is_err());
        // No variables at all.
        assert!(import_mat(&header()).unwrap_err().contains("no variables"));
    }
}

#[cfg(test)]
mod hostile_import_tests {
    //! The audit's D7 gap: guards against invalid enclosures minted from
    //! external data existed but were never fed hostile bytes. Every case
    //! here must fail LOUDLY — a silent acceptance would let a file forge a
    //! "certified" interval.
    use super::*;

    fn doc(value: &str) -> String {
        format!(
            r#"{{"format":"surd-data","version":1,"variables":[{{"name":"s","value":{}}}]}}"#,
            value
        )
    }

    #[test]
    fn f64_signal_with_inverted_bounds_is_rejected() {
        let e = import(&doc(r#"{"t":"signal","lo":[2.0],"hi":[1.0]}"#));
        assert!(e.is_err(), "lo > hi must not import: {:?}", e.map(|_| ()));
    }

    #[test]
    fn f64_signal_with_out_of_range_numbers_is_rejected() {
        // serde_json maps 1e999 to ±inf on some versions and errors on
        // others; either way it must not become a "certified" enclosure.
        let e = import(&doc(r#"{"t":"signal","lo":[-1e999],"hi":[1e999]}"#));
        assert!(e.is_err(), "non-finite bounds must not import");
    }

    #[test]
    fn big_signal_with_inverted_bounds_is_rejected() {
        let e = import(&doc(
            r#"{"t":"signal","digits":5,"lo":["2.5"],"hi":["1.5"]}"#,
        ));
        assert!(e.is_err(), "Big lo > hi must not import");
    }

    #[test]
    fn big_signal_with_garbage_bounds_is_rejected() {
        for bad in ["nan", "inf", "1e999999999999", "0x10", ""] {
            let e = import(&doc(&format!(
                r#"{{"t":"signal","digits":5,"lo":["{bad}"],"hi":["3"]}}"#
            )));
            assert!(e.is_err(), "bound {bad:?} must not import");
        }
    }

    #[test]
    fn mismatched_bound_lengths_are_rejected() {
        let e = import(&doc(r#"{"t":"signal","lo":[1.0,2.0],"hi":[3.0]}"#));
        assert!(e.is_err(), "length mismatch must not import");
    }

    #[test]
    fn raw_import_rejects_non_finite_samples() {
        for fmt in ["f64", "f32"] {
            for bits in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let bytes: Vec<u8> = if fmt == "f64" {
                    bits.to_le_bytes().to_vec()
                } else {
                    (bits as f32).to_le_bytes().to_vec()
                };
                let e = import_raw(&bytes, fmt);
                assert!(e.is_err(), "{fmt} {bits} must not import");
            }
        }
    }

    #[test]
    fn float_wav_rejects_nan_samples() {
        // Minimal RIFF/WAVE: fmt chunk (IEEE float, mono, 32-bit) + one
        // NaN sample in the data chunk.
        let mut w: Vec<u8> = Vec::new();
        w.extend(b"RIFF");
        w.extend(&(36u32 + 4).to_le_bytes());
        w.extend(b"WAVE");
        w.extend(b"fmt ");
        w.extend(&16u32.to_le_bytes());
        w.extend(&3u16.to_le_bytes()); // IEEE float
        w.extend(&1u16.to_le_bytes()); // mono
        w.extend(&8000u32.to_le_bytes());
        w.extend(&32000u32.to_le_bytes());
        w.extend(&4u16.to_le_bytes());
        w.extend(&32u16.to_le_bytes());
        w.extend(b"data");
        w.extend(&4u32.to_le_bytes());
        w.extend(&f32::NAN.to_le_bytes());
        let e = import_wav(&w);
        assert!(e.is_err(), "NaN float-WAV sample must not import");
    }

    #[test]
    fn truncated_wav_is_rejected_not_panicking() {
        // Every prefix of a valid header must error gracefully.
        let full: Vec<u8> = {
            let mut w: Vec<u8> = Vec::new();
            w.extend(b"RIFF");
            w.extend(&40u32.to_le_bytes());
            w.extend(b"WAVE");
            w.extend(b"fmt ");
            w.extend(&16u32.to_le_bytes());
            w.extend(&1u16.to_le_bytes());
            w.extend(&1u16.to_le_bytes());
            w.extend(&8000u32.to_le_bytes());
            w.extend(&16000u32.to_le_bytes());
            w.extend(&2u16.to_le_bytes());
            w.extend(&16u16.to_le_bytes());
            w.extend(b"data");
            w.extend(&2u32.to_le_bytes());
            w.extend(&0i16.to_le_bytes());
            w
        };
        for cut in 0..full.len() {
            let _ = import_wav(&full[..cut]); // must not panic; Err is fine
        }
        assert!(import_wav(&full).is_ok(), "the uncut file is valid");
    }
}
