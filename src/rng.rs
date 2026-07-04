//! A tiny deterministic PRNG for the data-splitting helpers.
//!
//! surd's engine is deterministic by contract — the web app replays a
//! transcript to rebuild the workspace, so the same input must always produce
//! the same value. Randomization therefore always runs from an explicit (or
//! documented default) seed: `data.split` and `stats.cv` shuffle with the seed
//! they were given, and re-evaluating reproduces the identical split.
//!
//! The generator is SplitMix64 (Steele, Lea & Flood 2014): 64 bits of state,
//! full period, passes BigCrush, and simple enough to be obviously portable
//! across native and wasm builds. Bounded draws use rejection sampling, so a
//! shuffle is exactly uniform over permutations — no modulo bias.

/// One SplitMix64 step: advance the state and hash it into an output word.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// An exactly uniform draw from `[0, n)` by rejection: discard the sliver of
/// the 64-bit range that doesn't divide evenly into `n` buckets.
fn bounded(state: &mut u64, n: u64) -> u64 {
    debug_assert!(n > 0);
    let threshold = n.wrapping_neg() % n; // 2^64 mod n
    loop {
        let r = next(state);
        if r >= threshold {
            return r % n;
        }
    }
}

/// The seeded Fisher–Yates permutation of `0..n` — exactly uniform over all
/// n! orderings, and identical for identical `(n, seed)` forever.
pub fn permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut state = seed;
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = bounded(&mut state, i as u64 + 1) as usize;
        v.swap(i, j);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutation_is_deterministic_and_complete() {
        let a = permutation(10, 42);
        let b = permutation(10, 42);
        assert_eq!(a, b);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>());
        assert_ne!(permutation(10, 43), a, "different seeds should differ");
    }

    #[test]
    fn permutation_edge_sizes() {
        assert_eq!(permutation(0, 7), Vec::<usize>::new());
        assert_eq!(permutation(1, 7), vec![0]);
    }

    #[test]
    fn known_values_pin_portability() {
        // SplitMix64 from seed 0 — reference values from the published
        // algorithm. If these move, saved notebooks would re-split datasets
        // differently on replay.
        let mut s = 0u64;
        assert_eq!(next(&mut s), 0xE220_A839_7B1D_CDAF);
        assert_eq!(next(&mut s), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(permutation(5, 0), vec![2, 3, 1, 4, 0]);
    }
}
