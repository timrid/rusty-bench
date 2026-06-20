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

/// Per-channel transition index over a [`DigitalStore`], maintained incrementally.
///
/// For each channel it stores the value at sample `0` and the sorted sample
/// indices at which the channel's level differs from the previous sample. This
/// lets the GUI find all visible edges in a range with a binary search and
/// reconstruct the level between them.
#[derive(Clone, Debug)]
pub struct DigitalMipMap {
    channel_count: u8,
    base_len: usize,
    initial: Vec<bool>,
    last: Vec<bool>,
    transitions: Vec<Vec<u64>>,
}

impl DigitalMipMap {
    /// Creates an empty transition index for `channel_count` channels.
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
        }
    }

    /// Builds a transition index over `words` in one shot.
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

    /// Brings the index up to date with `words`.
    ///
    /// `words` is the full, append-only logic word slice (e.g.
    /// [`DigitalStore::words`]). Only the newly appended samples are scanned, so
    /// repeated calls always match a single [`DigitalMipMap::build`].
    pub fn extend(&mut self, words: &[LogicWord]) {
        debug_assert!(
            words.len() >= self.base_len,
            "DigitalMipMap base is append-only"
        );
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
    }

    /// Level of channel `ch` at sample `index`.
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
    /// Each returned index is the sample at which the level becomes the new
    /// value (the edge sits between `index - 1` and `index`). Use
    /// [`DigitalMipMap::value_at`] at `range.start` to get the level the run
    /// begins with.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn edges_in(&self, ch: usize, range: Range<u64>) -> &[u64] {
        let t = &self.transitions[ch];
        let lo = t.partition_point(|&x| x < range.start);
        let hi = t.partition_point(|&x| x < range.end);
        &t[lo..hi]
    }

    /// Total number of transitions recorded for channel `ch`.
    ///
    /// # Panics
    /// Panics if `ch` is out of range.
    #[must_use]
    pub fn transition_count(&self, ch: usize) -> usize {
        self.transitions[ch].len()
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
        assert_eq!(m.edges_in(0, 2..5), &[2, 3, 4]);
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
