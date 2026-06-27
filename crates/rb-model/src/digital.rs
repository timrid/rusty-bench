//! Digital (logic) sample storage and its transition index.
//!
//! Logic samples are bit-packed: each base sample is one [`LogicWord`], where
//! bit `c` holds the state of digital channel `c`. Instead of a min/max pyramid,
//! the multi-resolution structure for logic is a per-channel **transition
//! index**: the sorted list of sample indices where a channel's level changes.
//! Drawing a channel at any zoom is then a binary search into its transitions.

use core::ops::Range;

use crate::channel::DigitalChannel;
use crate::timebase::Timebase;

/// A bit-packed set of digital channel states for one sample (up to 64 channels).
pub type LogicWord = u64;

/// Maximum number of digital channels in a single logic group.
pub const MAX_DIGITAL_CHANNELS: u8 = 64;

/// Append-only bit-packed logic samples for a channel group.
#[derive(Clone, Debug)]
pub struct DigitalStore {
    channel_count: u8,
    words: Vec<LogicWord>,
}

impl DigitalStore {
    /// Creates an empty store for `channel_count` channels.
    ///
    /// # Panics
    /// Panics if `channel_count` is `0` or greater than [`MAX_DIGITAL_CHANNELS`].
    #[must_use]
    pub fn new(channel_count: u8) -> Self {
        assert!(
            channel_count > 0 && channel_count <= MAX_DIGITAL_CHANNELS,
            "channel_count must be in 1..=64"
        );
        Self {
            channel_count,
            words: Vec::new(),
        }
    }

    /// Number of channels packed into each word.
    #[must_use]
    pub fn channel_count(&self) -> u8 {
        self.channel_count
    }

    /// Number of samples stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether the store holds no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Appends a single logic word.
    pub fn push(&mut self, word: LogicWord) {
        self.words.push(word);
    }

    /// Appends a slice of logic words.
    pub fn extend_from_slice(&mut self, words: &[LogicWord]) {
        self.words.extend_from_slice(words);
    }

    /// All logic words.
    #[must_use]
    pub fn words(&self) -> &[LogicWord] {
        &self.words
    }

    /// A consistent read slice for `range`, clamped to the available samples.
    #[must_use]
    pub fn read(&self, range: Range<usize>) -> &[LogicWord] {
        let start = range.start.min(self.words.len());
        let end = range.end.min(self.words.len());
        if start >= end {
            return &[];
        }
        &self.words[start..end]
    }

    /// State of `bit` at sample `index`.
    ///
    /// # Panics
    /// Panics if `index` is out of bounds.
    #[must_use]
    pub fn state(&self, bit: u8, index: usize) -> bool {
        (self.words[index] >> bit) & 1 != 0
    }
}

/// Aggregation radix for the digital mip-map pyramid.  Larger than the
/// analog radix because digital chunks are tiny (2 bools each).
const DIGITAL_RADIX: usize = 64;

/// One aggregated chunk in the digital multi-resolution pyramid.
#[derive(Clone, Copy, Debug, Default)]
struct Chunk {
    /// At least one transition occurred within this chunk's sample range.
    has_edge: bool,
    /// The number of transitions in this chunk is odd.
    parity: bool,
}

/// Result of [`DigitalMipMap::query_dense`] – per-bucket activity and
/// toggle parity for efficient dense-mode waveform drawing.
#[derive(Clone, Debug)]
pub struct DenseQuery {
    /// `has_edge[i]` is true when bucket `i` contains ≥ 1 transition.
    pub has_edge: Vec<bool>,
    /// `parity[i]` is true when bucket `i` contains an odd number of
    /// transitions (used to track level toggles across buckets).
    pub parity: Vec<bool>,
}

/// Per-channel transition index *and* multi-resolution chunk pyramid.
///
/// The transition list gives O(log n) single-sample queries; the pyramid
/// gives O(k) dense queries (k ≈ pixel columns), independent of edge count.
#[derive(Clone, Debug)]
pub struct DigitalMipMap {
    channel_count: u8,
    base_len: usize,
    initial: Vec<bool>,
    last: Vec<bool>,
    transitions: Vec<Vec<u64>>,
    /// `levels[channel][level][chunk]`.  Level 0 chunks span
    /// [`DIGITAL_RADIX`] samples; each higher level aggregates
    /// `DIGITAL_RADIX` chunks of the level below.
    levels: Vec<Vec<Vec<Chunk>>>,
}

impl DigitalMipMap {
    /// Creates an empty index + pyramid for `channel_count` channels.
    ///
    /// # Panics
    /// Panics if `channel_count` is `0` or greater than [`MAX_DIGITAL_CHANNELS`].
    #[must_use]
    pub fn new(channel_count: u8) -> Self {
        assert!(
            channel_count > 0 && channel_count <= MAX_DIGITAL_CHANNELS,
            "channel_count must be in 1..=64"
        );
        let n = channel_count as usize;
        Self {
            channel_count,
            base_len: 0,
            initial: vec![false; n],
            last: vec![false; n],
            transitions: vec![Vec::new(); n],
            levels: vec![Vec::new(); n],
        }
    }

    /// Builds the index and pyramid over `words` in one shot.
    #[must_use]
    pub fn build(words: &[LogicWord], channel_count: u8) -> Self {
        let mut m = Self::new(channel_count);
        m.extend(words);
        m
    }

    /// Number of channels.
    #[must_use]
    pub fn channel_count(&self) -> u8 {
        self.channel_count
    }

    /// Number of base samples reflected in the index.
    #[must_use]
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    /// Brings the index and pyramid up to date with `words`.
    ///
    /// `words` is the full, append-only logic word slice.  Only newly
    /// appended samples are scanned; repeated calls are cheap and match
    /// a single [`DigitalMipMap::build`].
    pub fn extend(&mut self, words: &[LogicWord]) {
        debug_assert!(
            words.len() >= self.base_len,
            "DigitalMipMap base is append-only"
        );
        let radix = DIGITAL_RADIX;
        let prev_base_len = self.base_len;

        // ── Update transitions ─────────────────────────────────────────
        for (idx, &word) in words.iter().enumerate().skip(self.base_len) {
            for ch in 0..self.channel_count as usize {
                let v = (word >> ch) & 1 != 0;
                if idx == 0 {
                    self.initial[ch] = v;
                    self.last[ch] = v;
                } else if v != self.last[ch] {
                    self.transitions[ch].push(idx as u64);
                    self.last[ch] = v;
                }
            }
        }
        self.base_len = words.len();

        // ── Rebuild pyramid levels ─────────────────────────────────────
        // Same incremental strategy as the analog mip-map: only chunks
        // affected by newly appended base samples are recomputed.
        for ch in 0..self.channel_count as usize {
            if self.levels[ch].is_empty() {
                self.levels[ch].push(Vec::new());
            }
        }

        let mut from = prev_base_len / radix;
        for ch in 0..self.channel_count as usize {
            rebuild_level0(&self.transitions[ch], &mut self.levels[ch][0], from, radix, self.base_len);
        }

        let mut lvl = 1;
        loop {
            let below_len = self.levels[0][lvl - 1].len();
            if below_len <= 1 {
                for ch in 0..self.channel_count as usize {
                    self.levels[ch].truncate(lvl);
                }
                break;
            }
            from /= radix;

            for ch in 0..self.channel_count as usize {
                if self.levels[ch].len() == lvl {
                    self.levels[ch].push(Vec::new());
                }
                // Safe split: levels[ch][..lvl] and levels[ch][lvl..]
                let (lower_slice, upper_slice) = self.levels[ch].split_at_mut(lvl);
                rebuild_level(&lower_slice[lvl - 1], &mut upper_slice[0], from, radix);
            }
            lvl += 1;
        }
    }

    /// Level of channel `ch` at sample `index`.  O(log n) via binary search.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn value_at(&self, ch: usize, index: u64) -> bool {
        let t = &self.transitions[ch];
        let edges_before_or_at = t.partition_point(|&x| x <= index);
        self.initial[ch] ^ (edges_before_or_at % 2 == 1)
    }

    /// The transition sample indices of channel `ch` lying in `range`.
    ///
    /// Edges at `range.start` are **excluded** – [`value_at`] already
    /// captures their effect.  For sparse (zoomed-in) drawing.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn edges_in(&self, ch: usize, range: Range<u64>) -> &[u64] {
        let t = &self.transitions[ch];
        let lo = t.partition_point(|&x| x <= range.start);
        let hi = t.partition_point(|&x| x < range.end);
        &t[lo..hi]
    }

    /// Number of transitions in `range`.  O(log n), does not materialise
    /// the edge slice.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn edge_count_in(&self, ch: usize, range: Range<u64>) -> usize {
        let t = &self.transitions[ch];
        let lo = t.partition_point(|&x| x <= range.start);
        let hi = t.partition_point(|&x| x < range.end);
        hi - lo
    }

    /// Total number of transitions recorded for channel `ch`.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn transition_count(&self, ch: usize) -> usize {
        self.transitions[ch].len()
    }

    /// Dense query: per-bucket activity and toggle parity for drawing at
    /// a fixed pixel budget.  Picks the appropriate pyramid level so that
    /// `has_edge` lookups are O(1) per bucket; `parity` is resolved via
    /// binary search on the transition list.  Total O(k · log n).
    ///
    /// Caller supplies `value_at(ch, range.start)` separately for the
    /// starting level.
    ///
    /// # Panics
    /// Panics if `ch` is out of range or `num_buckets == 0`.
    #[must_use]
    pub fn query_dense(
        &self,
        ch: usize,
        range: Range<u64>,
        num_buckets: usize,
    ) -> DenseQuery {
        assert!(num_buckets > 0, "num_buckets must be > 0");
        let span = range.end.saturating_sub(range.start) as f64;
        let bucket_size = (span / num_buckets as f64).max(1.0);

        // Pick coarsest level whose chunk size ≤ bucket size.
        let radix = DIGITAL_RADIX;
        let mut lvl = 0;
        let mut chunk_size = radix;
        while lvl + 1 < self.levels[ch].len() {
            let next_size = chunk_size * radix;
            if (next_size as f64) <= bucket_size {
                lvl += 1;
                chunk_size = next_size;
            } else {
                break;
            }
        }

        let level = &self.levels[ch][lvl];
        let t = &self.transitions[ch];

        let mut has_edge = Vec::with_capacity(num_buckets);
        let mut parity = Vec::with_capacity(num_buckets);

        let cs = chunk_size as u64;
        for i in 0..num_buckets {
            let b_start = range.start + (i as f64 * bucket_size) as u64;
            let b_end = range.start + ((i + 1) as f64 * bucket_size) as u64;

            // has_edge: OR of covering chunks  O(1–3)
            let c_start = ((b_start / cs) as usize).min(level.len());
            let c_end = ((b_end.saturating_sub(1) / cs + 1) as usize).min(level.len());
            let mut he = false;
            for ci in c_start..c_end {
                he |= level[ci].has_edge;
            }

            // parity: binary search on transitions  O(log n)
            let lo = t.partition_point(|&x| x <= b_start);
            let hi = t.partition_point(|&x| x < b_end);
            let p = (hi - lo) % 2 == 1;

            has_edge.push(he);
            parity.push(p);
        }

        DenseQuery { has_edge, parity }
    }
}

// ── Level-rebuild helpers ────────────────────────────────────────────────────

fn rebuild_level0(
    transitions: &[u64],
    out: &mut Vec<Chunk>,
    from_bucket: usize,
    radix: usize,
    base_len: usize,
) {
    out.truncate(from_bucket);
    let mut b = from_bucket;
    // Start of the transition slice for the current chunk.
    let mut ti = transitions.partition_point(|&x| (x as usize) < b * radix);
    loop {
        let start = b * radix;
        if start >= base_len {
            break;
        }
        let end = (start + radix).min(base_len);
        let mut has_edge = false;
        let mut count: u64 = 0;
        while ti < transitions.len() && (transitions[ti] as usize) < end {
            if (transitions[ti] as usize) > start {
                has_edge = true;
                count += 1;
            }
            ti += 1;
        }
        out.push(Chunk { has_edge, parity: count % 2 == 1 });
        b += 1;
    }
}

fn rebuild_level(
    children: &[Chunk],
    out: &mut Vec<Chunk>,
    from_bucket: usize,
    radix: usize,
) {
    out.truncate(from_bucket);
    let n = children.len();
    let mut b = from_bucket;
    loop {
        let start = b * radix;
        if start >= n {
            break;
        }
        let end = (start + radix).min(n);
        let mut has_edge = children[start].has_edge;
        let mut parity = children[start].parity;
        for c in &children[start + 1..end] {
            has_edge |= c.has_edge;
            parity ^= c.parity;
        }
        out.push(Chunk { has_edge, parity });
        b += 1;
    }
}

/// A digital channel group bundled with its samples, timebase and transition index.
#[derive(Clone, Debug)]
pub struct DigitalTrace {
    channels: Vec<DigitalChannel>,
    timebase: Timebase,
    store: DigitalStore,
    mip: DigitalMipMap,
}

impl DigitalTrace {
    /// Creates a trace for the given channels.
    ///
    /// # Panics
    /// Panics if `channels` is empty or holds more than
    /// [`MAX_DIGITAL_CHANNELS`] channels.
    #[must_use]
    pub fn new(channels: Vec<DigitalChannel>, timebase: Timebase) -> Self {
        let count = u8::try_from(channels.len()).expect("at most 64 channels");
        Self {
            store: DigitalStore::new(count),
            mip: DigitalMipMap::new(count),
            channels,
            timebase,
        }
    }

    /// Appends logic words and updates the transition index.
    pub fn push_words(&mut self, words: &[LogicWord]) {
        self.store.extend_from_slice(words);
        self.mip.extend(self.store.words());
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
    pub fn channels(&self) -> &[DigitalChannel] {
        &self.channels
    }

    /// The timebase.
    #[must_use]
    pub fn timebase(&self) -> &Timebase {
        &self.timebase
    }

    /// The underlying sample store.
    #[must_use]
    pub fn store(&self) -> &DigitalStore {
        &self.store
    }

    /// The transition index.
    #[must_use]
    pub fn transitions(&self) -> &DigitalMipMap {
        &self.mip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelId;

    /// Reference value of a channel at an index, computed directly from words.
    fn true_value(words: &[LogicWord], ch: usize, index: usize) -> bool {
        (words[index] >> ch) & 1 != 0
    }

    #[test]
    fn store_state_and_read() {
        let mut s = DigitalStore::new(3);
        s.extend_from_slice(&[0b001, 0b011, 0b010, 0b000]);
        assert_eq!(s.len(), 4);
        assert!(s.state(0, 0));
        assert!(s.state(1, 1));
        assert!(!s.state(0, 2));
        assert_eq!(s.read(1..3), &[0b011, 0b010]);
        assert_eq!(s.read(10..20), &[] as &[LogicWord]);
    }

    #[test]
    fn transitions_recorded_per_channel() {
        // Channel 0 toggles every sample; channel 1 is steady high after sample 2.
        let words = [0b00, 0b01, 0b11, 0b10, 0b11];
        let m = DigitalMipMap::build(&words, 2);
        // ch0 pattern: 0,1,1,0,1 -> edges at 1, 3, 4
        assert_eq!(m.edges_in(0, 0..5), &[1, 3, 4]);
        // ch1 pattern: 0,0,1,1,1 -> edge at 2
        assert_eq!(m.edges_in(1, 0..5), &[2]);
    }

    #[test]
    fn value_at_matches_direct_lookup() {
        let words: Vec<LogicWord> = (0..200u64).map(|i| (i * 2654435761) & 0b111).collect();
        let m = DigitalMipMap::build(&words, 3);
        for ch in 0..3 {
            for idx in 0..words.len() {
                assert_eq!(
                    m.value_at(ch, idx as u64),
                    true_value(&words, ch, idx),
                    "ch {ch} idx {idx}"
                );
            }
        }
    }

    #[test]
    fn edges_in_subrange_is_a_window() {
        let words = [0b0, 0b1, 0b0, 0b1, 0b0, 0b1, 0b0];
        let m = DigitalMipMap::build(&words, 1);
        // All edges: 1,2,3,4,5,6
        // Edge at 2 is excluded because value_at(2) already reflects it.
        assert_eq!(m.edges_in(0, 2..5), &[3, 4]);
        assert_eq!(m.edges_in(0, 0..1), &[] as &[u64]);
    }

    #[test]
    fn incremental_extend_matches_batch_build() {
        let words: Vec<LogicWord> = (0..500u64).map(|i| (i ^ (i >> 1)) & 0xF).collect();
        let batch = DigitalMipMap::build(&words, 4);

        let mut incr = DigitalMipMap::new(4);
        for chunk_end in [1usize, 3, 4, 9, 64, 199, 200, 499, 500] {
            incr.extend(&words[..chunk_end]);
        }
        assert_eq!(incr.base_len(), batch.base_len());
        for ch in 0..4 {
            assert_eq!(
                incr.edges_in(ch, 0..500),
                batch.edges_in(ch, 0..500),
                "channel {ch}"
            );
        }
    }

    #[test]
    fn trace_push_and_inspect() {
        let channels = vec![
            DigitalChannel::new(ChannelId(0), "D0", 0),
            DigitalChannel::new(ChannelId(1), "D1", 1),
        ];
        let mut trace = DigitalTrace::new(channels, Timebase::new(1_000_000.0, 0.0));
        trace.push_words(&[0b00, 0b01]);
        trace.push_words(&[0b11, 0b10]);
        assert_eq!(trace.len(), 4);
        assert_eq!(trace.transitions().edges_in(0, 0..4), &[1, 3]);
        assert!(trace.transitions().value_at(1, 2));
    }
}
