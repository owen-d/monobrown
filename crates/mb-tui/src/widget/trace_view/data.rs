//! Owned display facts and bounded admission, independent of terminal types.

use std::collections::BTreeSet;
use std::fmt;

/// Identity of a flat display group within one dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub u32);

/// Identity of a track within one dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackId(pub u32);

/// Identity of an item, unique across all tracks in one dataset.
///
/// IDs break equal-time ties. Callers should assign them in their desired
/// deterministic source order; this widget knows nothing about source sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u64);

/// Unit shared by every time coordinate in a dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceTimeUnit {
    /// One billionth of a second.
    Nanoseconds,
    /// One millionth of a second.
    Microseconds,
    /// One thousandth of a second.
    Milliseconds,
    /// Whole seconds.
    Seconds,
}

impl fmt::Display for TraceTimeUnit {
    /// Format the unit used by axis and detail labels.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nanoseconds => "ns",
            Self::Microseconds => "us",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
        })
    }
}

/// Recorded time geometry; absence is distinct from a zero-length interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceTiming {
    /// A single observation, with no implied duration.
    Instant(i64),
    /// An explicitly supplied pair. Reversed pairs display warning endpoints
    /// without a connecting bar; equal pairs display a zero-length marker.
    Interval { start: i64, end: i64 },
    /// Only the ending observation is known.
    MissingStart { end: i64 },
    /// Only the starting observation is known.
    MissingEnd { start: i64 },
    /// No time coordinate is known; displayed outside the time axis.
    Untimed,
}

impl TraceTiming {
    /// Return recorded coordinates without creating absent endpoints.
    pub(crate) fn coordinates(self) -> [Option<i64>; 2] {
        match self {
            Self::Instant(at) => [Some(at), None],
            Self::Interval { start, end } => [Some(start), Some(end)],
            Self::MissingStart { end } => [Some(end), None],
            Self::MissingEnd { start } => [Some(start), None],
            Self::Untimed => [None, None],
        }
    }

    /// Identify only positive intervals eligible for duration bars and packing.
    pub(crate) fn interval(self) -> Option<(i64, i64)> {
        match self {
            Self::Interval { start, end } if start < end => Some((start, end)),
            _ => None,
        }
    }

    /// Describe the supplied observations, preserving endpoint direction.
    pub(crate) fn description(self, unit: TraceTimeUnit) -> String {
        match self {
            Self::Instant(at) => format!("instant {at} {unit}"),
            Self::Interval { start, end } => {
                let note = if start > end { " (regressed)" } else { "" };
                format!("{start}..{end} {unit}{note}")
            }
            Self::MissingStart { end } => format!("missing start; end {end} {unit}"),
            Self::MissingEnd { start } => format!("start {start} {unit}; missing end"),
            Self::Untimed => "untimed".into(),
        }
    }
}

/// One selectable display observation, with caller-formatted detail fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceItem {
    pub id: ItemId,
    pub label: String,
    pub timing: TraceTiming,
    /// Ordered key/value pairs; controls render as spaces, and overflow is counted.
    pub details: Vec<(String, String)>,
}

/// A named concurrent track. Items need not arrive in time order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceTrack {
    pub id: TrackId,
    pub group: Option<GroupId>,
    pub label: String,
    pub items: Vec<TraceItem>,
}

/// A flat presentation group, with no claim about execution hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceGroup {
    pub id: GroupId,
    pub label: String,
}

/// Complete owned input to a [`super::TraceView`].
///
/// Groups appear in supplied order, followed by ungrouped tracks. Tracks retain
/// supplied order within each group. Admission accepts at most 4096 groups,
/// 4096 tracks, 65536 items, 32 detail pairs per item, 4096 UTF-8 bytes per field,
/// and 8 MiB of total text. Oversized input is rejected without partial display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceData {
    pub time_unit: TraceTimeUnit,
    pub groups: Vec<TraceGroup>,
    pub tracks: Vec<TraceTrack>,
}

/// Invalid display input or viewport; the original facts are never repaired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceError {
    /// A documented admission bound was exceeded.
    LimitExceeded(&'static str),
    /// Two groups have the same identity.
    DuplicateGroup(GroupId),
    /// Two tracks have the same identity.
    DuplicateTrack(TrackId),
    /// Two items have the same identity, even across tracks.
    DuplicateItem(ItemId),
    /// A track references a group absent from the dataset.
    UnknownGroup(GroupId),
    /// A viewport must have a strictly increasing pair of coordinates.
    InvalidTimeWindow,
}

impl fmt::Display for TraceError {
    /// Report the rejected boundary in caller-facing diagnostics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded(bound) => write!(f, "trace limit exceeded: {bound}"),
            Self::DuplicateGroup(id) => write!(f, "duplicate trace group {}", id.0),
            Self::DuplicateTrack(id) => write!(f, "duplicate trace track {}", id.0),
            Self::DuplicateItem(id) => write!(f, "duplicate trace item {}", id.0),
            Self::UnknownGroup(id) => write!(f, "unknown trace group {}", id.0),
            Self::InvalidTimeWindow => f.write_str("trace window requires start < end"),
        }
    }
}

impl std::error::Error for TraceError {}

impl TraceData {
    /// Check size before allocating identity indexes or derived layout.
    pub(crate) fn validate(&self) -> Result<(), TraceError> {
        self.validate_size()?;
        let mut groups = BTreeSet::new();
        for group in &self.groups {
            if !groups.insert(group.id) {
                return Err(TraceError::DuplicateGroup(group.id));
            }
        }
        let mut tracks = BTreeSet::new();
        let mut items = BTreeSet::new();
        for track in &self.tracks {
            if !tracks.insert(track.id) {
                return Err(TraceError::DuplicateTrack(track.id));
            }
            if let Some(group) = track.group
                && !groups.contains(&group)
            {
                return Err(TraceError::UnknownGroup(group));
            }
            for item in &track.items {
                if !items.insert(item.id) {
                    return Err(TraceError::DuplicateItem(item.id));
                }
            }
        }
        Ok(())
    }

    /// Bound all nested collections and text before expensive processing.
    fn validate_size(&self) -> Result<(), TraceError> {
        check_limit(self.groups.len(), 4096, "groups (4096)")?;
        check_limit(self.tracks.len(), 4096, "tracks (4096)")?;
        let mut items = 0;
        let mut text_bytes = 0;
        for group in &self.groups {
            check_text(&group.label, &mut text_bytes)?;
        }
        for track in &self.tracks {
            items += track.items.len();
            check_limit(items, 65536, "items (65536)")?;
            check_text(&track.label, &mut text_bytes)?;
            for item in &track.items {
                check_limit(item.details.len(), 32, "detail pairs per item (32)")?;
                check_text(&item.label, &mut text_bytes)?;
                for (key, value) in &item.details {
                    check_text(key, &mut text_bytes)?;
                    check_text(value, &mut text_bytes)?;
                }
            }
        }
        Ok(())
    }
}

/// Reject excess before the caller allocates proportional derived state.
fn check_limit(value: usize, limit: usize, name: &'static str) -> Result<(), TraceError> {
    if value > limit {
        Err(TraceError::LimitExceeded(name))
    } else {
        Ok(())
    }
}

/// Account UTF-8 bytes without copying or normalizing caller text.
fn check_text(text: &str, total: &mut usize) -> Result<(), TraceError> {
    check_limit(text.len(), 4096, "bytes per field (4096)")?;
    *total += text.len();
    check_limit(*total, 8 * 1024 * 1024, "total text bytes (8 MiB)")
}
