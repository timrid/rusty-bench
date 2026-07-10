use crate::LogicWord;

/// Digital sample data carried in a chunk.
///
/// Drivers choose the representation that maps most directly to their USB
/// data format, avoiding unnecessary decode steps. The store then ingests
/// the data in its native form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DigitalChunkData {
    /// Pre-decoded [`LogicWord`]s (u64 per sample).
    /// Used by wide (16-bit) devices, demo devices, and post-processed data.
    Words(Vec<LogicWord>),
    /// Raw 8-bit USB bytes (1 byte per sample, 1:1 mapping to U8 stores).
    /// Used by 8-bit fx2lafw and similar narrow logic analyzers.
    Raw8(Vec<u8>),
}

impl DigitalChunkData {
    /// Number of samples in this chunk.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Words(v) => v.len(),
            Self::Raw8(v) => v.len(),
        }
    }

    /// Whether this chunk carries no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrow as pre-decoded words, if this is a [`Words`](Self::Words) variant.
    #[must_use]
    pub fn as_words(&self) -> Option<&[LogicWord]> {
        match self {
            Self::Words(v) => Some(v),
            Self::Raw8(_) => None,
        }
    }

    /// Borrow as raw 8-bit bytes, if this is a [`Raw8`](Self::Raw8) variant.
    #[must_use]
    pub fn as_raw8(&self) -> Option<&[u8]> {
        match self {
            Self::Words(_) => None,
            Self::Raw8(v) => Some(v),
        }
    }
}

/// A contiguous block of freshly-acquired samples for one device, carrying every
/// channel's samples for the same `[0, sample_count)` window.
///
/// A chunk is the unit of bulk data flow from a driver's acquisition source into
/// the per-device stores. It deliberately holds *raw* samples only: analog values
/// as raw integers (one inner vector per analog channel, in channel order) and
/// digital data in a driver-optimised format via [`DigitalChunkData`].
/// Conversion to physical units and mip-map aggregation happen in the stores.
///
/// All present series describe the same time window, so every analog channel and,
/// if present, the digital series share a single [`sample_count`](Self::sample_count).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleChunk {
    analog: Option<AnalogChunkData>,
    digital: Option<DigitalChunkData>,
}

// ── AnalogChunkData ───────────────────────────────────────────────────────────

/// Analog sample data carried in a chunk, one vector per channel.
///
/// Drivers choose the representation that matches their ADC width,
/// avoiding unnecessary i32 expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnalogChunkData {
    /// Pre-decoded i32 samples, one `Vec<i32>` per channel (17–32 bit ADC).
    I32(Vec<Vec<i32>>),
    /// Raw i16 samples, one `Vec<i16>` per channel (9–16 bit ADC).
    I16(Vec<Vec<i16>>),
    /// Raw i8 samples, one `Vec<i8>` per channel (1–8 bit ADC).
    I8(Vec<Vec<i8>>),
}

impl AnalogChunkData {
    fn channel_count(&self) -> usize {
        match self {
            Self::I32(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::I8(v) => v.len(),
        }
    }

    fn sample_count(&self) -> usize {
        self.first_len().unwrap_or(0)
    }

    /// Returns true if all channels have the same length.
    fn is_consistent(&self) -> bool {
        let expected = self.first_len();
        match self {
            Self::I32(v) => expected.is_some_and(|n| v.iter().all(|c| c.len() == n)),
            Self::I16(v) => expected.is_some_and(|n| v.iter().all(|c| c.len() == n)),
            Self::I8(v) => expected.is_some_and(|n| v.iter().all(|c| c.len() == n)),
        }
    }

    fn first_len(&self) -> Option<usize> {
        match self {
            Self::I32(v) => v.first().map(|c| c.len()),
            Self::I16(v) => v.first().map(|c| c.len()),
            Self::I8(v) => v.first().map(|c| c.len()),
        }
    }
}

impl SampleChunk {
    /// Creates an empty chunk carrying no samples.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets pre-decoded i32 analog data (17–32 bit ADC, backward compat).
    #[must_use]
    pub fn with_analog_i32(mut self, analog: Vec<Vec<i32>>) -> Self {
        self.analog = Some(AnalogChunkData::I32(analog));
        self
    }

    /// Sets raw i16 analog data (9–16 bit ADC).
    #[must_use]
    pub fn with_analog_i16(mut self, analog: Vec<Vec<i16>>) -> Self {
        self.analog = Some(AnalogChunkData::I16(analog));
        self
    }

    /// Sets raw i8 analog data (1–8 bit ADC).
    #[must_use]
    pub fn with_analog_i8(mut self, analog: Vec<Vec<i8>>) -> Self {
        self.analog = Some(AnalogChunkData::I8(analog));
        self
    }

    /// The analog data, if present.
    #[must_use]
    pub fn analog(&self) -> Option<&AnalogChunkData> {
        self.analog.as_ref()
    }

    /// The analog channel count, or 0 if no analog data.
    #[must_use]
    pub fn analog_channel_count(&self) -> usize {
        self.analog.as_ref().map_or(0, |a| a.channel_count())
    }

    /// The raw i32 samples of analog channel `index`, if present and I32 variant.
    pub fn analog_channel(&self, index: usize) -> Option<&[i32]> {
        match &self.analog {
            Some(AnalogChunkData::I32(v)) => v.get(index).map(Vec::as_slice),
            _ => None,
        }
    }

    /// Number of samples in this chunk: the length of any present series. Returns
    /// `0` for an empty chunk.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        if let Some(ref a) = self.analog {
            a.sample_count()
        } else if let Some(ref d) = self.digital {
            d.len()
        } else {
            0
        }
    }

    /// Whether the chunk carries no samples at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sample_count() == 0
    }

    /// Whether every present series has the same length (`sample_count`).
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        let analog_ok = self.analog.as_ref().is_none_or(|a| a.is_consistent());
        let digital_ok = self.digital.as_ref().is_none_or(|d| d.len() == self.sample_count());
        analog_ok && digital_ok
    }

    // ── Digital convenience methods ───────────────────────────────────────

    /// Sets pre-decoded logic words (wide devices, demo).
    #[must_use]
    pub fn with_logic_words(mut self, words: Vec<LogicWord>) -> Self {
        self.digital = Some(DigitalChunkData::Words(words));
        self
    }

    /// Sets raw 8-bit USB sample bytes (8-bit devices).
    #[must_use]
    pub fn with_logic_raw8(mut self, bytes: Vec<u8>) -> Self {
        self.digital = Some(DigitalChunkData::Raw8(bytes));
        self
    }

    /// The digital data, if present.
    #[must_use]
    pub fn digital(&self) -> Option<&DigitalChunkData> {
        self.digital.as_ref()
    }

    // ── Deprecated migration helpers ──────────────────────────────────────

    /// Deprecated: use [`with_analog_i32`](Self::with_analog_i32).
    #[deprecated(since = "0.2.0", note = "use `with_analog_i32` instead")]
    #[must_use]
    #[allow(dead_code)]
    pub fn with_analog(self, analog: Vec<Vec<i32>>) -> Self {
        self.with_analog_i32(analog)
    }

    /// Deprecated: use [`with_logic_words`](Self::with_logic_words).
    #[deprecated(since = "0.2.0", note = "use `with_logic_words` instead")]
    #[must_use]
    #[allow(dead_code)]
    pub fn with_logic(self, logic: Vec<LogicWord>) -> Self {
        self.with_logic_words(logic)
    }

    /// Deprecated: use [`digital().and_then(|d| d.as_words())`](Self::digital).
    #[deprecated(since = "0.2.0", note = "get digital data via `digital()` instead")]
    #[must_use]
    #[allow(dead_code)]
    pub fn logic(&self) -> &[LogicWord] {
        match &self.digital {
            Some(DigitalChunkData::Words(v)) => v,
            _ => &[],
        }
    }

    /// Deprecated: use [`digital().and_then(|d| d.as_raw8())`](Self::digital).
    #[deprecated(since = "0.2.0", note = "get raw bytes via `digital()` instead")]
    #[must_use]
    #[allow(dead_code)]
    pub fn logic_raw(&self) -> Option<&[u8]> {
        self.digital.as_ref().and_then(|d| d.as_raw8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chunk_has_no_samples() {
        let chunk = SampleChunk::new();
        assert_eq!(chunk.sample_count(), 0);
        assert!(chunk.is_empty());
        assert!(chunk.is_consistent());
        assert_eq!(chunk.analog_channel_count(), 0);
    }

    #[test]
    fn sample_count_follows_analog_then_digital() {
        let analog_only = SampleChunk::new().with_analog(vec![vec![1, 2, 3]]);
        assert_eq!(analog_only.sample_count(), 3);
        assert_eq!(analog_only.analog_channel(0), Some([1, 2, 3].as_slice()));
        assert_eq!(analog_only.analog_channel(1), None);

        let logic_only = SampleChunk::new().with_logic_words(vec![0b01, 0b10]);
        assert_eq!(logic_only.sample_count(), 2);
        assert!(!logic_only.is_empty());

        let raw_only = SampleChunk::new().with_logic_raw8(vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(raw_only.sample_count(), 3);
        assert_eq!(raw_only.digital().unwrap().as_raw8(), Some([0xAA, 0xBB, 0xCC].as_slice()));
    }

    #[test]
    fn consistency_detects_mismatched_series() {
        let good = SampleChunk::new()
            .with_analog(vec![vec![1, 2], vec![3, 4]])
            .with_logic_words(vec![1, 0]);
        assert!(good.is_consistent());
        assert_eq!(good.analog_channel_count(), 2);

        let bad = SampleChunk::new()
            .with_analog(vec![vec![1, 2], vec![3]])
            .with_logic_words(vec![1, 0]);
        assert!(!bad.is_consistent());

        let bad_logic = SampleChunk::new()
            .with_analog(vec![vec![1, 2]])
            .with_logic_words(vec![1, 0, 1]);
        assert!(!bad_logic.is_consistent());
    }

    #[test]
    fn digital_chunk_data_dispatch() {
        let words = DigitalChunkData::Words(vec![0x01, 0x02]);
        assert_eq!(words.len(), 2);
        let wlen = words.as_words().unwrap().len();
        assert_eq!(wlen, 2);

        let raw = DigitalChunkData::Raw8(vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(raw.len(), 3);
        let rlen = raw.as_raw8().unwrap().len();
        assert_eq!(rlen, 3);
    }
}
