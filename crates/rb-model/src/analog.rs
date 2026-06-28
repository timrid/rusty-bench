//! Analog sample storage and its min/max mip-map.
//!
//! Base samples are kept as raw integers in an [`AnalogStore`]. An
//! [`AnalogMipMap`] maintains a min/max pyramid over the base level so the GUI
//! can draw any zoom level at roughly constant cost:
//!
//! - Per-sample drawing (zoomed in): read raw samples directly via
//!   [`AnalogTrace::store`] → [`AnalogStore::read`].
//! - Envelope rendering (zoomed out): [`AnalogMipMap::query_envelope`]
//!   returns ~`pixel_count * 8` MipMap-level buckets that the JS side
//!   aggregates onto exact pixel columns.

use core::ops::Range;

use crate::channel::AnalogChannel;
use crate::timebase::Timebase;

/// Default number of children aggregated per pyramid step.
///
/// A larger radix means fewer levels (less memory, coarser zoom granularity).
pub const DEFAULT_RADIX: usize = 4;

/// Append-only raw analog samples for a single channel.
///
/// Samples are stored as `i32`, which losslessly holds common ADC widths
/// (`i16`/`u16`/`i24`/...). Physical values are obtained via the channel's
/// [`AnalogFormat`]; the store itself is unit-agnostic.
#[derive(Clone, Debug, Default)]
pub struct AnalogStore {
    raw: Vec<i32>,
}

impl AnalogStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of samples stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Whether the store holds no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Appends a single raw sample.
    pub fn push(&mut self, raw: i32) {
        self.raw.push(raw);
    }

    /// Appends a slice of raw `i32` samples.
    pub fn extend_from_slice(&mut self, raw: &[i32]) {
        self.raw.extend_from_slice(raw);
    }

    /// Appends samples from a narrower signed ADC width.
    pub fn extend_i16(&mut self, raw: &[i16]) {
        self.raw.extend(raw.iter().map(|&v| i32::from(v)));
    }

    /// Appends samples from a narrower unsigned ADC width.
    pub fn extend_u16(&mut self, raw: &[u16]) {
        self.raw.extend(raw.iter().map(|&v| i32::from(v)));
    }

    /// All raw samples.
    #[must_use]
    pub fn raw(&self) -> &[i32] {
        &self.raw
    }

    /// A consistent read slice for `range`, clamped to the available samples.
    ///
    /// Returns the raw samples that lie within `[range.start, range.end)`
    /// intersected with `[0, len())`. An out-of-order range yields an empty
    /// slice.
    #[must_use]
    pub fn read(&self, range: Range<usize>) -> &[i32] {
        let start = range.start.min(self.raw.len());
        let end = range.end.min(self.raw.len());
        if start >= end {
            return &[];
        }
        &self.raw[start..end]
    }
}

/// The minimum, maximum, and arithmetic mean over a contiguous run of samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinMax {
    /// Smallest raw value in the run.
    pub min: i32,
    /// Largest raw value in the run.
    pub max: i32,
    /// Sum of all raw values (for computing the average).
    sum: i64,
    /// Number of base samples in this run.
    count: u32,
}

impl MinMax {
    /// A degenerate run holding a single value.
    #[must_use]
    fn point(v: i32) -> Self {
        Self { min: v, max: v, sum: v as i64, count: 1 }
    }

    /// Extends this range to include `v`.
    fn include(&mut self, v: i32) {
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.sum += v as i64;
        self.count += 1;
    }

    /// Merges another range into this one.
    fn merge(&mut self, other: MinMax) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.sum += other.sum;
        self.count += other.count;
    }

    /// Arithmetic mean of the raw values in this run.
    fn avg(&self) -> f64 {
        if self.count == 0 { 0.0 }
        else { self.sum as f64 / self.count as f64 }
    }
}

/// One aggregated draw bucket: the `[start, end)` base-sample span it covers,
/// the min/max raw value, and the arithmetic mean over that span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    /// First base-sample index (inclusive).
    pub start: usize,
    /// Last base-sample index (exclusive).
    pub end: usize,
    /// Minimum raw value over the span.
    pub min: i32,
    /// Maximum raw value over the span.
    pub max: i32,
    /// Arithmetic mean over the span.
    pub avg: f64,
}

/// A min/max pyramid over an [`AnalogStore`], maintained incrementally.
///
/// `levels[0]` aggregates `radix` base samples per bucket; each higher level
/// aggregates `radix` buckets of the level below. Every level always covers the
/// full base length (its final bucket may be partial), so any level can answer a
/// query over the whole capture.
#[derive(Clone, Debug)]
pub struct AnalogMipMap {
    radix: usize,
    base_len: usize,
    levels: Vec<Vec<MinMax>>,
}

impl AnalogMipMap {
    /// Creates an empty pyramid with the given aggregation `radix`.
    ///
    /// # Panics
    /// Panics if `radix < 2`.
    #[must_use]
    pub fn new(radix: usize) -> Self {
        assert!(radix >= 2, "mip-map radix must be >= 2");
        Self {
            radix,
            base_len: 0,
            levels: Vec::new(),
        }
    }

    /// Builds a pyramid over `base` in one shot.
    #[must_use]
    pub fn build(base: &[i32], radix: usize) -> Self {
        let mut m = Self::new(radix);
        m.extend(base);
        m
    }

    /// Aggregation radix.
    #[must_use]
    pub fn radix(&self) -> usize {
        self.radix
    }

    /// Number of base samples currently reflected in the pyramid.
    #[must_use]
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    /// Number of pyramid levels (`0` while empty).
    #[must_use]
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Brings the pyramid up to date with `base`.
    ///
    /// `base` is the full, append-only base sample slice (e.g.
    /// [`AnalogStore::raw`]). Only the buckets affected by newly appended
    /// samples are recomputed, so repeated calls are cheap and always yield the
    /// same result as a single [`AnalogMipMap::build`] over the final data.
    pub fn extend(&mut self, base: &[i32]) {
        debug_assert!(
            base.len() >= self.base_len,
            "AnalogMipMap base is append-only"
        );
        let radix = self.radix;
        let prev_base_len = self.base_len;
        self.base_len = base.len();

        if self.levels.is_empty() {
            self.levels.push(Vec::new());
        }
        // The first base bucket that can change is the one holding the previously
        // partial tail (and all newly appended samples). The first *changed*
        // bucket index then cascades up the pyramid by dividing by the radix:
        // every recomputed child may alter its parent.
        let mut from = prev_base_len / radix;
        rebuild_level0(base, &mut self.levels[0], from, radix);

        let mut l = 1;
        loop {
            let below_len = self.levels[l - 1].len();
            if below_len <= 1 {
                // The level below already spans the whole capture in one bucket.
                self.levels.truncate(l);
                break;
            }
            if self.levels.len() == l {
                self.levels.push(Vec::new());
            }
            from /= radix;
            let (lower, upper) = self.levels.split_at_mut(l);
            rebuild_level(&lower[l - 1], &mut upper[0], from, radix);
            l += 1;
        }
    }

    /// Returns MipMap-level buckets covering `range`, with roughly
    /// `pixel_count * 8` buckets so that JS can aggregate them
    /// pixel-precisely into a min/max envelope.
    ///
    /// The returned `Bucket`s retain their native MipMap `start`/`end`
    /// (not snapped to pixel boundaries).  The caller aggregates them
    /// onto pixel columns in JS for correct 1-px envelope rendering.
    ///
    /// # Panics
    /// Panics if `pixel_count == 0`.
    #[must_use]
    pub fn query_envelope(
        &self,
        base: &[i32],
        range: Range<usize>,
        pixel_count: usize,
    ) -> Vec<Bucket> {
        assert!(pixel_count > 0, "pixel_count must be > 0");
        let len = base.len();
        let start = range.start.min(len);
        let end = range.end.min(len);
        if start >= end {
            return Vec::new();
        }
        let span = end - start;

        // Per-sample fallback: zoomed in far enough to draw individual samples.
        if span <= pixel_count {
            return (start..end)
                .map(|i| Bucket {
                    start: i, end: i + 1,
                    min: base[i], max: base[i], avg: base[i] as f64,
                })
                .collect();
        }

        // Pick the finest level with ≤ pixel_count * 8 buckets.
        let oversample = pixel_count * 8;
        let mut chosen: Option<(&[MinMax], usize)> = None;
        for (l, _level) in self.levels.iter().enumerate() {
            let bucket_span = self.radix.pow(l as u32 + 1);
            let count = end.div_ceil(bucket_span) - start / bucket_span;
            if count <= oversample {
                chosen = Some((&self.levels[l], bucket_span));
                break;
            }
        }
        let (level, bucket_span) = chosen.unwrap_or_else(|| {
            let l = self.levels.len() - 1;
            (&self.levels[l], self.radix.pow(l as u32 + 1))
        });

        let first = start / bucket_span;
        let last = (end - 1) / bucket_span;
        (first..=last)
            .filter_map(|b| {
                level.get(b).map(|mm| Bucket {
                    start: (b * bucket_span).max(start),
                    end: ((b + 1) * bucket_span).min(end).min(len),
                    min: mm.min,
                    max: mm.max,
                    avg: mm.avg(),
                })
            })
            .collect()
    }
}

fn rebuild_level0(base: &[i32], out: &mut Vec<MinMax>, from_bucket: usize, radix: usize) {
    out.truncate(from_bucket);
    let n = base.len();
    let mut b = from_bucket;
    loop {
        let start = b * radix;
        if start >= n {
            break;
        }
        let end = (start + radix).min(n);
        let mut mm = MinMax::point(base[start]);
        for &v in &base[start + 1..end] {
            mm.include(v);
        }
        out.push(mm);
        b += 1;
    }
}

fn rebuild_level(children: &[MinMax], out: &mut Vec<MinMax>, from_bucket: usize, radix: usize) {
    out.truncate(from_bucket);
    let n = children.len();
    let mut b = from_bucket;
    loop {
        let start = b * radix;
        if start >= n {
            break;
        }
        let end = (start + radix).min(n);
        let mut mm = children[start];
        for &c in &children[start + 1..end] {
            mm.merge(c);
        }
        out.push(mm);
        b += 1;
    }
}

/// An analog channel bundled with its samples, timebase and mip-map.
///
/// This is the ergonomic display-facing surface: push raw samples, then ask for
/// draw [`Bucket`]s over a range. The mip-map is kept current automatically.
#[derive(Clone, Debug)]
pub struct AnalogTrace {
    channel: AnalogChannel,
    timebase: Timebase,
    store: AnalogStore,
    mip: AnalogMipMap,
}

impl AnalogTrace {
    /// Creates a trace using the default mip-map radix.
    #[must_use]
    pub fn new(channel: AnalogChannel, timebase: Timebase) -> Self {
        Self::with_radix(channel, timebase, DEFAULT_RADIX)
    }

    /// Creates a trace with an explicit mip-map radix.
    #[must_use]
    pub fn with_radix(channel: AnalogChannel, timebase: Timebase, radix: usize) -> Self {
        Self {
            channel,
            timebase,
            store: AnalogStore::new(),
            mip: AnalogMipMap::new(radix),
        }
    }

    /// Appends raw samples and updates the mip-map.
    pub fn push_raw(&mut self, raw: &[i32]) {
        self.store.extend_from_slice(raw);
        self.mip.extend(self.store.raw());
    }

    /// Number of samples in the trace.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Whether the trace holds no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// The channel metadata.
    #[must_use]
    pub fn channel(&self) -> &AnalogChannel {
        &self.channel
    }

    /// The timebase.
    #[must_use]
    pub fn timebase(&self) -> &Timebase {
        &self.timebase
    }

    /// The underlying sample store.
    #[must_use]
    pub fn store(&self) -> &AnalogStore {
        &self.store
    }

    /// MipMap-level buckets for envelope rendering, roughly `pixel_count * 8`
    /// buckets so JS can aggregate them pixel-precisely.
    #[must_use]
    pub fn envelope_buckets(
        &self,
        range: Range<usize>,
        pixel_count: usize,
    ) -> Vec<Bucket> {
        self.mip.query_envelope(self.store.raw(), range, pixel_count)
    }

    /// Converts a raw bucket value to a physical value via the channel format.
    #[must_use]
    pub fn to_physical(&self, raw: i32) -> f64 {
        self.channel.format.to_physical(raw)
    }

    /// Converts a (possibly fractional) raw average to physical.
    #[must_use]
    pub fn to_physical_f64(&self, raw: f64) -> f64 {
        self.channel.format.to_physical_f64(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::AnalogFormat;

    /// Reference min/max over a base range, computed the slow, obvious way.
    fn true_minmax(base: &[i32], range: Range<usize>) -> MinMax {
        let mut it = base[range].iter().copied();
        let first = it.next().expect("non-empty range");
        let mut mm = MinMax::point(first);
        for v in it {
            mm.include(v);
        }
        mm
    }

    #[test]
    fn store_read_clamps_range() {
        let mut s = AnalogStore::new();
        s.extend_from_slice(&[0, 1, 2, 3, 4]);
        assert_eq!(s.read(1..3), &[1, 2]);
        assert_eq!(s.read(3..100), &[3, 4]);
        assert_eq!(s.read(10..20), &[] as &[i32]);
        let inverted = core::ops::Range { start: 4, end: 2 };
        assert_eq!(s.read(inverted), &[] as &[i32]);
    }

    #[test]
    fn store_ingests_narrow_widths() {
        let mut s = AnalogStore::new();
        s.extend_i16(&[-1, 2, -3]);
        s.extend_u16(&[4, 5]);
        assert_eq!(s.raw(), &[-1, 2, -3, 4, 5]);
    }

    #[test]
    fn mipmap_level0_groups_by_radix() {
        let base = [3, 1, 4, 1, 5, 9, 2, 6];
        let m = AnalogMipMap::build(&base, 2);
        // 8 samples / radix 2 = 4 base buckets, then 2, then 1.
        assert_eq!(m.level_count(), 3);
        assert_eq!(m.base_len(), 8);
    }

    // ── query_envelope tests ──────────────────────────────────────────────

    #[test]
    fn envelope_buckets_cover_range_with_correct_minmax() {
        let base: Vec<i32> = (0..1000).map(|i| (i * 7 % 53) - 20).collect();
        let m = AnalogMipMap::build(&base, 4);
        let range = 100..900;
        let buckets = m.query_envelope(&base, range.clone(), 32);
        assert!(!buckets.is_empty());

        for w in buckets.windows(2) {
            assert_eq!(w[0].end, w[1].start, "no gaps/overlaps between buckets");
        }
        assert_eq!(buckets.first().unwrap().start, range.start);
        assert_eq!(buckets.last().unwrap().end, range.end);

        for b in &buckets {
            let truth = true_minmax(&base, b.start..b.end);
            assert!(b.min <= truth.min, "bucket {b:?} min {} > truth min {}", b.min, truth.min);
            assert!(b.max >= truth.max, "bucket {b:?} max {} < truth max {}", b.max, truth.max);
        }
    }

    #[test]
    fn envelope_per_sample_fallback() {
        let base: Vec<i32> = (0..100).collect();
        let m = AnalogMipMap::build(&base, 4);
        let buckets = m.query_envelope(&base, 10..15, 64);
        assert_eq!(buckets.len(), 5);
        for (i, b) in buckets.iter().enumerate() {
            let idx = 10 + i;
            assert_eq!(b.start, idx);
            assert_eq!(b.end, idx + 1);
            assert_eq!(b.min, base[idx]);
            assert_eq!(b.max, base[idx]);
        }
    }

    #[test]
    fn envelope_empty_or_inverted_range() {
        let base: Vec<i32> = (0..50).collect();
        let m = AnalogMipMap::build(&base, 4);
        assert!(m.query_envelope(&base, 30..30, 8).is_empty());
        let inverted = core::ops::Range { start: 40, end: 10 };
        assert!(m.query_envelope(&base, inverted, 8).is_empty());
        assert!(m.query_envelope(&base, 100..200, 8).is_empty());
    }

    #[test]
    fn incremental_extend_matches_batch_build() {
        let base: Vec<i32> = (0..777).map(|i| ((i * 13) % 31) - 15).collect();
        let batch = AnalogMipMap::build(&base, 3);

        let mut incr = AnalogMipMap::new(3);
        // Feed in irregular chunks.
        for chunk_end in [1usize, 2, 5, 6, 17, 64, 65, 300, 776, 777] {
            incr.extend(&base[..chunk_end]);
        }
        assert_eq!(incr.level_count(), batch.level_count());
        assert_eq!(incr.base_len(), batch.base_len());
        for (a, b) in incr.levels.iter().zip(batch.levels.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn trace_push_and_envelope_query() {
        use crate::channel::ChannelId;
        let ch = AnalogChannel::new(ChannelId(1), "CH1", AnalogFormat::new(0.001, 0.0));
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000.0, 0.0));
        let data: Vec<i32> = (0..256).collect();
        trace.push_raw(&data[..128]);
        trace.push_raw(&data[128..]);
        assert_eq!(trace.len(), 256);
        let buckets = trace.envelope_buckets(0..256, 16);
        assert!(!buckets.is_empty());
        assert!((trace.to_physical(1000) - 1.0).abs() < 1e-12);
    }

    /// Simulates the scenario reported by the user: 400 k samples viewed at
    /// 12 000 max buckets (Full‑HD width).  Ensures every bucket’s min/max
    /// captures the full amplitude of the underlying sine wave.
    #[test]
    fn dense_query_preserves_amplitude() {
        let n = 400_000usize;
        let amplitude = 20_000i32;
        // Sine wave: full-scale swing every ~1 000 samples (1 kHz @ 1 MSa/s).
        let base: Vec<i32> = (0..n)
            .map(|i| {
                let phase = i as f64 * 1_000.0 / 1_000_000.0 * 2.0 * std::f64::consts::PI;
                (phase.sin() * amplitude as f64) as i32
            })
            .collect();

        let m = AnalogMipMap::build(&base, 4);
        let buckets = m.query_envelope(&base, 0..n, 1200);
        assert!(!buckets.is_empty());
        assert!(buckets.len() <= 9600 + 10);

        let overall_min = base.iter().copied().min().unwrap();
        let overall_max = base.iter().copied().max().unwrap();
        let tolerance = (amplitude as f64 * 0.005) as i32;

        for b in &buckets {
            let truth = true_minmax(&base, b.start..b.end);
            assert!(b.min <= truth.min, "min at [{}, {})", b.start, b.end);
            assert!(b.max >= truth.max, "max at [{}, {})", b.start, b.end);
        }

        // The union of all bucket extents must cover the global amplitude.
        let union_min = buckets.iter().map(|b| b.min).min().unwrap();
        let union_max = buckets.iter().map(|b| b.max).max().unwrap();
        assert!(
            (union_min - overall_min).abs() <= tolerance,
            "union min {union_min} vs global min {overall_min}"
        );
        assert!(
            (union_max - overall_max).abs() <= tolerance,
            "union max {union_max} vs global max {overall_max}"
        );
    }
}
