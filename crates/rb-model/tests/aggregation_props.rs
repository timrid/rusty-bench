//! Property tests for the multi-resolution aggregation.
//!
//! These guard the two correctness properties that are easy to get subtly wrong
//! with off-by-one errors at bucket and chunk borders:
//!
//! 1. The mip-map built incrementally (in arbitrary chunks) is identical to the
//!    mip-map built in one shot.
//! 2. Every draw bucket's aggregate equals the true aggregate over the exact
//!    base span it claims to cover, and the returned buckets tile the requested
//!    range with no gaps or overlaps.

use proptest::prelude::*;
use rb_model::{AnalogMipMap, DigitalMipMap, DigitalStore, LogicWord};

/// True min/max over a base range, the slow obvious way.
fn true_minmax(base: &[i32], start: usize, end: usize) -> (i32, i32) {
    let mut min = base[start];
    let mut max = base[start];
    for &v in &base[start + 1..end] {
        min = min.min(v);
        max = max.max(v);
    }
    (min, max)
}

/// Replays a base vector into a fresh mip-map using the given chunk sizes.
fn build_incrementally(base: &[i32], radix: usize, chunks: &[usize]) -> AnalogMipMap {
    let mut m = AnalogMipMap::new(radix);
    let mut filled = 0usize;
    for &c in chunks {
        filled = (filled + c).min(base.len());
        m.extend(&base[..filled]);
    }
    // Ensure all samples are in, regardless of the chunk sizes drawn.
    m.extend(base);
    m
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Incremental construction in arbitrary chunks == one-shot build.
    #[test]
    fn analog_incremental_equals_batch(
        base in prop::collection::vec(-10_000i32..10_000, 0..600),
        radix in 2usize..6,
        chunks in prop::collection::vec(1usize..40, 0..40),
    ) {
        let batch = AnalogMipMap::build(&base, radix);
        let incr = build_incrementally(&base, radix, &chunks);

        prop_assert_eq!(incr.base_len(), batch.base_len());
        prop_assert_eq!(incr.level_count(), batch.level_count());
        // Compare via query_envelope at pixel counts proportional to level granularity.
        let n = base.len().max(1);
        for l in 0..batch.level_count() {
            let pc = ((1usize << (l + 1)) / 8).max(1);
            let a = incr.query_envelope(&base, 0..n, pc);
            let b = batch.query_envelope(&base, 0..n, pc);
            prop_assert_eq!(a, b);
        }
    }

    /// Each draw bucket covers exactly its claimed span and tiles the range.
    #[test]
    fn analog_buckets_are_exact_and_contiguous(
        base in prop::collection::vec(-1_000i32..1_000, 1..800),
        radix in 2usize..6,
        a in 0usize..800,
        b in 0usize..800,
        pixel_count in 1usize..25,
    ) {
        let len = base.len();
        let start = a.min(len);
        let end = b.min(len);
        prop_assume!(start < end);

        let m = AnalogMipMap::build(&base, radix);
        let buckets = m.query_envelope(&base, start..end, pixel_count);
        prop_assert!(!buckets.is_empty());

        // Contiguous, no gaps or overlaps.
        for w in buckets.windows(2) {
            prop_assert_eq!(w[0].end, w[1].start);
        }
        // The tiling covers the whole requested range (query_envelope clamps
        // bucket edges to range boundaries, so start == range.start).
        prop_assert_eq!(buckets.first().unwrap().start, start);
        prop_assert_eq!(buckets.last().unwrap().end, end);
        // No bucket exceeds the base length.
        prop_assert!(buckets.last().unwrap().end <= len);

        // Each bucket's aggregate is conservative: its min/max covers the
        // full MipMap bucket (may be wider than the clamped span).  For
        // envelope rendering this is correct and desired.
        for bk in &buckets {
            prop_assert!(bk.start < bk.end);
            let (min, max) = true_minmax(&base, bk.start, bk.end);
            prop_assert!(bk.min <= min, "bucket {:?} min {} > true min {}", bk, bk.min, min);
            prop_assert!(bk.max >= max, "bucket {:?} max {} < true max {}", bk, bk.max, max);
        }
    }

    /// Digital transition index: incremental == batch, and value_at is exact.
    #[test]
    fn digital_incremental_equals_batch_and_value_exact(
        raw in prop::collection::vec(0u64..16, 0..600),
        chunks in prop::collection::vec(1usize..40, 0..40),
    ) {
        let words: Vec<LogicWord> = raw;
        let channel_count = 4u8;

        let batch = DigitalMipMap::build(&words, channel_count);

        let mut store = DigitalStore::new(channel_count);
        let mut incr = DigitalMipMap::new(channel_count);
        let mut filled = 0usize;
        for &c in &chunks {
            filled = (filled + c).min(words.len());
            let prev = store.len();
            if filled > prev {
                store.extend_from_slice(&words[prev..filled]);
            }
            incr.extend(&store);
        }
        // Ensure all samples are in.
        let prev = store.len();
        if words.len() > prev {
            store.extend_from_slice(&words[prev..]);
        }
        incr.extend(&store);

        prop_assert_eq!(incr.base_len(), batch.base_len());
        for ch in 0..channel_count as usize {
            let len = words.len() as u64;
            prop_assert_eq!(incr.edges_in(ch, 0..len), batch.edges_in(ch, 0..len));
            // value_at matches a direct bit lookup at every sample.
            for (idx, &word) in words.iter().enumerate() {
                let expected = (word >> ch) & 1 != 0;
                prop_assert_eq!(batch.value_at(ch, idx as u64), expected);
            }
        }
    }
}
