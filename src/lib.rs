//! `exact` — an exact-by-default computer algebra system, prototype core.
//!
//! Design pillars (decided in the design conversation):
//!   * Exact by default. `1/3` is the rational 1/3, `1.5` is 3/2, `sqrt(2)` and
//!     `pi` are symbolic objects. Floats only appear when you ask, via `N(...)`.
//!   * CAS-by-default that can also compute numerically. Every value is a
//!     symbolic expression that may collapse to a number.
//!   * `:=` assigns, `=` builds an equation (it does NOT test truth — equality
//!     of reals is undecidable; see Richardson's theorem).
//!   * Bounded auto-simplification on construction (canonical forms); deep
//!     simplification is reserved for explicit ops.
//!
//! Architecture: source -> [`lexer`] -> tokens -> [`parser`] -> [`ast`] ->
//! [`eval`] walks the AST in an environment and produces canonical [`expr::Expr`]
//! values. All canonicalization lives in the smart constructors in [`expr`].

pub mod algebraic;
pub mod ast;
pub mod data;
pub mod dataio;
pub mod discriminant;
pub mod dsp;
pub mod eval;
pub mod expr;
pub mod f64eval;
pub mod iir;
pub mod interval;
pub mod latex;
pub mod lexer;
pub mod matrix;
pub mod nlfit;
pub mod parser;
pub mod remez;
pub mod rng;
pub mod signal;
pub mod special;
pub mod stats;

pub use eval::Interpreter;

/// The engine version, taken from this crate's `Cargo.toml` at compile time.
/// This is the single source of truth the CLI banner and the wasm `version()`
/// binding report; the desktop/web app and the other manifests are kept in
/// step with it by `scripts/bump-version.sh` (and checked by
/// `scripts/check-version.sh`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run `f` on a thread with a large stack, giving deeply recursive evaluation
/// room before the depth guards trip (debug builds use ~4 KB stack frames per
/// `eval_node`, and test threads only get 2 MB by default). Native embedders —
/// the REPL and the test suite — wrap evaluation in this. The WASM target
/// should instead configure its stack size at link time (`--stack-size`).
///
/// Panics in `f` are propagated to the caller; a stack overflow cannot happen
/// within the depth guards given this stack size.
pub fn run_with_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    const STACK_BYTES: usize = 256 * 1024 * 1024;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, f)
            .expect("failed to spawn evaluation thread")
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}
