//! MAT-file import against files a *real* writer produced (scipy.io.savemat,
//! which emits the same level-5 layout as MATLAB's `save -v6`/`-v7`). The
//! unit tests in src/dataio.rs build their own fixtures, so they can't catch
//! the parser and the fixtures sharing one misreading of the spec — these
//! bytes can. Fixtures live in tests/fixtures/ (see the generator script in
//! this file's history: scalars, vectors, 2-D, complex, narrow-int, int64
//! beyond 2^53, char, logical, struct, empty, NaN).

use surd::dataio::import_mat;
use surd::expr::Expr;
use surd::signal::SignalData;
use surd::Interpreter;

fn val(src: &str) -> Expr {
    Interpreter::new().eval_line(src).expect(src)
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
    assert_eq!(lo, hi, "exact decodes must be point intervals");
    lo
}

fn check_common(v: &Expr) {
    // A vector of doubles: a point-interval signal, bit-for-bit.
    assert_eq!(point_signal(field(v, "sig")), &[1.0, 2.5, -3.0, 0.1]);
    // Scalars and 2-D matrices: exact values (0.5 is exactly 1/2).
    assert_eq!(field(v, "x"), &val("1/2"));
    assert_eq!(field(v, "m"), &val("[1, 1/2; 3, 4]"));
    // Complex vector: a complex signal with exact parts.
    let Expr::Signal(s) = field(v, "z") else {
        panic!("z should be a signal")
    };
    let SignalData::Complex { re, im } = &**s else {
        panic!("z should be complex")
    };
    let (SignalData::F64 { lo: r, .. }, SignalData::F64 { lo: i, .. }) = (&**re, &**im) else {
        panic!("f64 parts")
    };
    assert_eq!(
        (r.as_slice(), i.as_slice()),
        (&[1.0, -0.5][..], &[2.0, 0.0][..])
    );
    // Narrow integer class decodes exactly.
    assert_eq!(point_signal(field(v, "counts")), &[1.0, 2.0, 300.0]);
    // int64 beyond 2^53: exact integers, never rounded through f64.
    let Expr::Matrix(rows) = field(v, "big") else {
        panic!("big should be an exact matrix")
    };
    assert_eq!(rows[0][0], val(&format!("{}", (1u64 << 60) + 1)));
    // Char row → string; logical scalar → boolean; empty array → NA.
    assert_eq!(field(v, "label"), &Expr::Str("hello world".into()));
    assert_eq!(field(v, "flag"), &Expr::Bool(true));
    assert_eq!(field(v, "empty"), &Expr::Symbol("NA".into()));
    // 1×1 struct recurses.
    let s = field(v, "s");
    assert_eq!(field(s, "a"), &val("1/2"));
    assert_eq!(point_signal(field(s, "bb")), &[1.0, 2.0]);
}

#[test]
fn scipy_v6_uncompressed_imports() {
    check_common(&import_mat(include_bytes!("fixtures/real_v6.mat")).unwrap());
}

#[test]
fn scipy_v7_compressed_imports() {
    check_common(&import_mat(include_bytes!("fixtures/real_v7.mat")).unwrap());
}

#[test]
fn scipy_nan_vector_imports_as_na_cells() {
    let v = import_mat(include_bytes!("fixtures/nanvec.mat")).unwrap();
    assert_eq!(field(&v, "v"), &val("[1, NA, 3]"));
}
