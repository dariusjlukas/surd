//! No-panic fuzz target: feed arbitrary text to the full pipeline (lex → parse
//! → eval) and assert only that it never panics, overflows, or hangs — it must
//! always return `Ok` or `Err`.
//!
//! Evaluation runs through `run_with_stack` so this exercises the real
//! production configuration (large stack + depth guards), rather than letting
//! the fuzzer "find" stack overflows that are just the bare libFuzzer thread.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let text = text.to_owned();
        surd::run_with_stack(move || {
            let mut interp = surd::Interpreter::new();
            let _ = interp.eval_line(&text);
        });
    }
});
