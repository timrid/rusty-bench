//! Sample-to-time mapping.
//!
//! A [`Timebase`] describes how a base-level sample index maps to a point in
//! time. It deliberately carries an optional wall-clock anchor so that captures
//! from independent devices can later be correlated on a shared display axis
//! (soft cross-device sync), without coupling their acquisition.

/// Maps base-level sample indices to seconds (and optionally wall-clock time).
///
/// The time of sample `i` is `start_offset_s + i / sample_rate_hz`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timebase {
    sample_rate_hz: f64,
    start_offset_s: f64,
    host_unix_nanos: Option<i128>,
}

impl Timebase {
    /// Creates a timebase with the given sample rate (Hz) and the time of
    /// sample `0` expressed as an offset in seconds from the capture origin.
    ///
    /// # Panics
    /// Panics if `sample_rate_hz` is not finite and strictly positive, or if
    /// `start_offset_s` is not finite.
    #[must_use]
    pub fn new(sample_rate_hz: f64, start_offset_s: f64) -> Self {
        assert!(
            sample_rate_hz.is_finite() && sample_rate_hz > 0.0,
            "sample_rate_hz must be finite and > 0"
        );
        assert!(start_offset_s.is_finite(), "start_offset_s must be finite");
        Self {
            sample_rate_hz,
            start_offset_s,
            host_unix_nanos: None,
        }
    }

    /// Returns a copy anchored to a wall-clock instant (Unix nanoseconds) for
    /// sample `0`. Used for optional soft cross-device time correlation.
    #[must_use]
    pub fn with_host_anchor(mut self, host_unix_nanos: i128) -> Self {
        self.host_unix_nanos = Some(host_unix_nanos);
        self
    }

    /// Samples per second at the base level.
    #[must_use]
    pub fn sample_rate_hz(&self) -> f64 {
        self.sample_rate_hz
    }

    /// Time of sample `0`, in seconds, relative to the capture origin.
    #[must_use]
    pub fn start_offset_s(&self) -> f64 {
        self.start_offset_s
    }

    /// Optional wall-clock anchor (Unix nanoseconds) for sample `0`.
    #[must_use]
    pub fn host_unix_nanos(&self) -> Option<i128> {
        self.host_unix_nanos
    }

    /// Duration between two adjacent samples, in seconds.
    #[must_use]
    pub fn sample_period_s(&self) -> f64 {
        1.0 / self.sample_rate_hz
    }

    /// Time, in seconds, of the sample at `index`.
    #[must_use]
    pub fn time_at(&self, index: u64) -> f64 {
        self.start_offset_s + index as f64 * self.sample_period_s()
    }

    /// Wall-clock time, in Unix nanoseconds, of the sample at `index`, if this
    /// timebase carries a host anchor.
    #[must_use]
    pub fn host_time_at(&self, index: u64) -> Option<i128> {
        self.host_unix_nanos.map(|anchor| {
            let delta_ns = (index as f64 * self.sample_period_s() * 1e9).round() as i128;
            anchor + delta_ns
        })
    }

    /// Duration, in seconds, spanned by `sample_count` samples.
    #[must_use]
    pub fn duration_s(&self, sample_count: u64) -> f64 {
        sample_count as f64 * self.sample_period_s()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_at_index_is_linear() {
        let tb = Timebase::new(1_000.0, 0.0);
        assert!((tb.sample_period_s() - 0.001).abs() < 1e-12);
        assert!((tb.time_at(0) - 0.0).abs() < 1e-12);
        assert!((tb.time_at(1000) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn start_offset_shifts_the_axis() {
        let tb = Timebase::new(2_000.0, 5.0);
        assert!((tb.time_at(0) - 5.0).abs() < 1e-12);
        assert!((tb.time_at(4000) - 7.0).abs() < 1e-12);
    }

    #[test]
    fn duration_counts_sample_periods() {
        let tb = Timebase::new(500.0, 0.0);
        assert!((tb.duration_s(500) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn host_anchor_round_trips() {
        let tb = Timebase::new(1_000.0, 0.0).with_host_anchor(1_000_000_000);
        assert_eq!(tb.host_unix_nanos(), Some(1_000_000_000));
        // 1000 samples at 1 kHz == 1 s == 1e9 ns later.
        assert_eq!(tb.host_time_at(1000), Some(2_000_000_000));
    }

    #[test]
    fn no_anchor_means_no_host_time() {
        let tb = Timebase::new(1_000.0, 0.0);
        assert_eq!(tb.host_time_at(10), None);
    }

    #[test]
    #[should_panic(expected = "sample_rate_hz")]
    fn zero_rate_panics() {
        let _ = Timebase::new(0.0, 0.0);
    }
}
