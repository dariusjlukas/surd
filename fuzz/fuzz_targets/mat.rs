//! No-panic fuzz target for the MAT-file importer: arbitrary bytes must
//! always come back as `Ok` or a clean `Err` — never a panic, arithmetic
//! overflow (declared sizes and dimension products are attacker-controlled),
//! or runaway allocation (the inflate and sample caps must hold).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = surd::dataio::import_mat(data);
});
