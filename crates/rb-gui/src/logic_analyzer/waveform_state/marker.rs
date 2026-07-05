//! Time markers and marker pairs for the waveform display.

// ── Marker types ──────────────────────────────────────────────────────────────

/// Unique identifier for a Time Marker.
pub type MarkerId = u32;

/// A user-placed time marker at a specific sample position.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeMarker {
    pub id: MarkerId,
    /// Sample position in the Sample Store.
    pub sample_pos: u64,
    /// Optional user label.
    pub label: Option<String>,
}

/// Unique identifier for a Marker Pair.
pub type PairId = u32;

/// Two linked Time Markers that display Δt and frequency.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerPair {
    pub id: PairId,
    pub marker_a: MarkerId,
    pub marker_b: MarkerId,
    pub label: Option<String>,
}

// ── MarkerSet ─────────────────────────────────────────────────────────────────

/// Owns the lists of time markers and marker pairs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkerSet {
    pub markers: Vec<TimeMarker>,
    pub marker_pairs: Vec<MarkerPair>,
    next_marker_id: MarkerId,
    next_pair_id: PairId,
}

impl MarkerSet {
    /// Add a new TimeMarker at the given sample position.
    pub fn add_marker(&mut self, sample_pos: u64) -> MarkerId {
        let id = self.next_marker_id;
        self.next_marker_id += 1;
        self.markers.push(TimeMarker {
            id,
            sample_pos,
            label: None,
        });
        id
    }

    /// Move an existing marker to a new sample position.
    pub fn move_marker(&mut self, id: MarkerId, new_pos: u64) {
        if let Some(m) = self.markers.iter_mut().find(|m| m.id == id) {
            m.sample_pos = new_pos;
        }
    }

    /// Remove a marker (and any pairs that reference it).
    pub fn remove_marker(&mut self, id: MarkerId) {
        self.markers.retain(|m| m.id != id);
        self.marker_pairs
            .retain(|p| p.marker_a != id && p.marker_b != id);
    }

    /// Create a Marker Pair from two existing markers.
    pub fn add_marker_pair(
        &mut self,
        marker_a: MarkerId,
        marker_b: MarkerId,
    ) -> Option<PairId> {
        if !self.markers.iter().any(|m| m.id == marker_a)
            || !self.markers.iter().any(|m| m.id == marker_b)
            || marker_a == marker_b
        {
            return None;
        }
        let id = self.next_pair_id;
        self.next_pair_id += 1;
        self.marker_pairs.push(MarkerPair {
            id,
            marker_a,
            marker_b,
            label: None,
        });
        Some(id)
    }

    /// Remove a Marker Pair (does not remove the underlying markers).
    pub fn remove_marker_pair(&mut self, id: PairId) {
        self.marker_pairs.retain(|p| p.id != id);
    }

    /// Find the sample position of a marker by ID.
    pub fn marker_pos(&self, id: MarkerId) -> Option<u64> {
        self.markers.iter().find(|m| m.id == id).map(|m| m.sample_pos)
    }
}
