use crate::LogicWord;

/// A contiguous block of freshly-acquired samples for one device, carrying every
/// channel's samples for the same `[0, sample_count)` window.
///
/// A chunk is the unit of bulk data flow from a driver's acquisition source into
/// the per-device stores. It deliberately holds *raw* samples only: analog values
/// as raw integers (one inner vector per analog channel, in channel order) and
/// digital values as bit-packed [`LogicWord`]s. Conversion to physical units and
/// mip-map aggregation happen in the stores, not here.
///
/// All present series describe the same time window, so every analog channel and,
/// if present, the logic series share a single [`sample_count`](Self::sample_count).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleChunk {
    analog: Vec<Vec<i32>>,
    logic: Vec<LogicWord>,
}

impl SampleChunk {
    /// Creates an empty chunk carrying no samples.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the analog series, one raw-sample vector per analog channel in
    /// channel order.
    #[must_use]
    pub fn with_analog(mut self, analog: Vec<Vec<i32>>) -> Self {
        self.analog = analog;
        self
    }

    /// Sets the bit-packed logic series.
    #[must_use]
    pub fn with_logic(mut self, logic: Vec<LogicWord>) -> Self {
        self.logic = logic;
        self
    }

    /// The raw analog series, one vector per analog channel in channel order.
    #[must_use]
    pub fn analog(&self) -> &[Vec<i32>] {
        &self.analog
    }

    /// The raw samples of analog channel `index`, if present.
    pub fn analog_channel(&self, index: usize) -> Option<&[i32]> {
        self.analog.get(index).map(Vec::as_slice)
    }

    /// The bit-packed logic series.
    #[must_use]
    pub fn logic(&self) -> &[LogicWord] {
        &self.logic
    }

    /// Number of analog channels carried.
    #[must_use]
    pub fn analog_channel_count(&self) -> usize {
        self.analog.len()
    }

    /// Number of samples in this chunk: the length of any present series. Returns
    /// `0` for an empty chunk.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        if let Some(first) = self.analog.first() {
            first.len()
        } else {
            self.logic.len()
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
        let n = self.sample_count();
        self.analog.iter().all(|c| c.len() == n) && (self.logic.is_empty() || self.logic.len() == n)
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
    fn sample_count_follows_analog_then_logic() {
        let analog_only = SampleChunk::new().with_analog(vec![vec![1, 2, 3]]);
        assert_eq!(analog_only.sample_count(), 3);
        assert_eq!(analog_only.analog_channel(0), Some([1, 2, 3].as_slice()));
        assert_eq!(analog_only.analog_channel(1), None);

        let logic_only = SampleChunk::new().with_logic(vec![0b01, 0b10]);
        assert_eq!(logic_only.sample_count(), 2);
        assert!(!logic_only.is_empty());
    }

    #[test]
    fn consistency_detects_mismatched_series() {
        let good = SampleChunk::new()
            .with_analog(vec![vec![1, 2], vec![3, 4]])
            .with_logic(vec![1, 0]);
        assert!(good.is_consistent());
        assert_eq!(good.analog_channel_count(), 2);

        let bad = SampleChunk::new()
            .with_analog(vec![vec![1, 2], vec![3]])
            .with_logic(vec![1, 0]);
        assert!(!bad.is_consistent());

        let bad_logic = SampleChunk::new()
            .with_analog(vec![vec![1, 2]])
            .with_logic(vec![1, 0, 1]);
        assert!(!bad_logic.is_consistent());
    }
}
