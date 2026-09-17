//! Owned display facts and bounded admission, independent of terminal types.

use std::collections::BTreeSet;
use std::fmt;

/// Identity of one caller-defined visual category within a dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CategoryId(pub u16);

/// Familiar semantic role resolved through the active terminal theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceVisualRole {
    /// Ordinary activity without an outcome or urgency claim.
    #[default]
    Neutral,
    /// Work that is queued, scheduled, or currently in progress.
    Scheduled,
    /// A successful outcome.
    Success,
    /// A partial, cancelled, or otherwise cautionary outcome.
    Warning,
    /// A failed or invalid outcome.
    Failure,
}

/// Caller-owned category label paired with a conventional visual role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceCategory {
    /// Dataset-wide stable category identity.
    pub id: CategoryId,
    /// Short legend label.
    pub label: String,
    /// Semantic palette role; labels and glyphs retain meaning without color.
    pub role: TraceVisualRole,
}

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

/// Renderer-neutral structured content for an item's detail inspector.
///
/// The trace model deliberately keeps layout out of these values: renderers may
/// present the same attributes as a table, a side pane, or a browser panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceDetailBlock {
    /// A compact labeled value.
    Attribute { name: String, value: String },
    /// A labeled prose value which should not be parsed as JSON.
    Text { name: String, value: String },
    /// A labeled structured payload, rendered with JSON-aware styling when the
    /// renderer supports it.
    Json { name: String, value: String },
    /// A titled group of related blocks.
    Section {
        title: String,
        blocks: Vec<TraceDetailBlock>,
    },
    /// A compact row/column representation for related facts.
    Table {
        title: Option<String>,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

impl TraceDetailBlock {
    /// Return text useful to search and accessibility adapters.
    pub(crate) fn search_text(&self, output: &mut String) {
        match self {
            Self::Attribute { name, value }
            | Self::Text { name, value }
            | Self::Json { name, value } => {
                output.push_str(name);
                output.push(' ');
                output.push_str(value);
                output.push(' ');
            }
            Self::Section { title, blocks } => {
                output.push_str(title);
                output.push(' ');
                for block in blocks {
                    block.search_text(output);
                }
            }
            Self::Table {
                title,
                columns,
                rows,
            } => {
                if let Some(title) = title {
                    output.push_str(title);
                    output.push(' ');
                }
                for column in columns {
                    output.push_str(column);
                    output.push(' ');
                }
                for row in rows {
                    for cell in row {
                        output.push_str(cell);
                        output.push(' ');
                    }
                }
            }
        }
    }
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
}

/// One selectable display observation, with caller-formatted detail fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceItem {
    /// Dataset-wide stable item identity.
    pub id: ItemId,
    /// Category used consistently by the time mark, legend, and details.
    pub category: CategoryId,
    /// Short row label.
    pub label: String,
    /// Explicit time observations without inferred endpoints.
    pub timing: TraceTiming,
    /// Ordered key/value pairs; controls render as spaces, and overflow is counted.
    pub details: Vec<(String, String)>,
    /// Structured sections, tables, and payloads for richer inspectors.
    pub detail_blocks: Vec<TraceDetailBlock>,
    /// Nested endpoint or lifecycle events owned by this item.
    pub children: Vec<TraceItem>,
}

/// A named concurrent track. Items need not arrive in time order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceTrack {
    /// Dataset-wide stable track identity.
    pub id: TrackId,
    /// Optional flat display group.
    pub group: Option<GroupId>,
    /// Short track label.
    pub label: String,
    /// Ordered track-level attributes shown by the detail inspector.
    pub details: Vec<(String, String)>,
    /// Structured sections and tables for track-level inspectors.
    pub detail_blocks: Vec<TraceDetailBlock>,
    /// Selectable observations supplied in deterministic source order.
    pub items: Vec<TraceItem>,
}

/// A flat presentation group, with no claim about execution hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceGroup {
    /// Dataset-wide stable group identity.
    pub id: GroupId,
    /// Short group label.
    pub label: String,
}

/// Complete owned input to a [`super::TraceView`].
///
/// Groups appear in supplied order, followed by ungrouped tracks. Tracks retain
/// supplied order within each group. Admission accepts at most 4096 groups,
/// 4096 tracks, 65536 items, 32 detail pairs per item, 4096 UTF-8 bytes per field,
/// 32 detail pairs or blocks per item/track, 8 section nesting levels, and
/// 8 MiB of total text. Oversized input is rejected without partial display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceData {
    /// Unit shared by every item coordinate.
    pub time_unit: TraceTimeUnit,
    /// Visual vocabulary in legend order.
    pub categories: Vec<TraceCategory>,
    /// Flat display groups in caller-selected order.
    pub groups: Vec<TraceGroup>,
    /// Tracks in caller-selected order within each group.
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
    /// Two categories have the same identity.
    DuplicateCategory(CategoryId),
    /// An item references a category absent from the dataset.
    UnknownCategory(CategoryId),
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
            Self::DuplicateCategory(id) => write!(f, "duplicate trace category {}", id.0),
            Self::UnknownCategory(id) => write!(f, "unknown trace category {}", id.0),
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
        let mut categories = BTreeSet::new();
        for category in &self.categories {
            if !categories.insert(category.id) {
                return Err(TraceError::DuplicateCategory(category.id));
            }
        }
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
                validate_item(item, &categories, &mut items)?;
            }
        }
        Ok(())
    }

    /// Bound all nested collections and text before expensive processing.
    fn validate_size(&self) -> Result<(), TraceError> {
        check_limit(self.groups.len(), 4096, "groups (4096)")?;
        check_limit(self.categories.len(), 256, "categories (256)")?;
        check_limit(self.tracks.len(), 4096, "tracks (4096)")?;
        let mut items = 0;
        let mut text_bytes = 0;
        for category in &self.categories {
            check_text(&category.label, &mut text_bytes)?;
        }
        for group in &self.groups {
            check_text(&group.label, &mut text_bytes)?;
        }
        for track in &self.tracks {
            check_limit(track.details.len(), 32, "detail pairs per track (32)")?;
            check_limit(
                track.detail_blocks.len(),
                32,
                "detail blocks per track (32)",
            )?;
            check_text(&track.label, &mut text_bytes)?;
            for (key, value) in &track.details {
                check_text(key, &mut text_bytes)?;
                check_text(value, &mut text_bytes)?;
            }
            validate_detail_blocks(&track.detail_blocks, &mut text_bytes, 0)?;
            for item in &track.items {
                validate_item_size(item, &mut items, &mut text_bytes)?;
            }
        }
        Ok(())
    }
}

fn validate_item(
    item: &TraceItem,
    categories: &BTreeSet<CategoryId>,
    items: &mut BTreeSet<ItemId>,
) -> Result<(), TraceError> {
    if !categories.contains(&item.category) {
        return Err(TraceError::UnknownCategory(item.category));
    }
    if !items.insert(item.id) {
        return Err(TraceError::DuplicateItem(item.id));
    }
    for child in &item.children {
        validate_item(child, categories, items)?;
    }
    Ok(())
}

fn validate_item_size(
    item: &TraceItem,
    count: &mut usize,
    text_bytes: &mut usize,
) -> Result<(), TraceError> {
    *count += 1;
    check_limit(*count, 65536, "items (65536)")?;
    check_limit(item.details.len(), 32, "detail pairs per item (32)")?;
    check_limit(item.detail_blocks.len(), 32, "detail blocks per item (32)")?;
    check_text(&item.label, text_bytes)?;
    for (key, value) in &item.details {
        check_text(key, text_bytes)?;
        check_text(value, text_bytes)?;
    }
    validate_detail_blocks(&item.detail_blocks, text_bytes, 0)?;
    for child in &item.children {
        validate_item_size(child, count, text_bytes)?;
    }
    Ok(())
}

fn validate_detail_blocks(
    blocks: &[TraceDetailBlock],
    text_bytes: &mut usize,
    depth: usize,
) -> Result<(), TraceError> {
    check_limit(depth, 8, "detail section depth (8)")?;
    check_limit(blocks.len(), 32, "detail blocks per section (32)")?;
    for block in blocks {
        match block {
            TraceDetailBlock::Attribute { name, value }
            | TraceDetailBlock::Text { name, value }
            | TraceDetailBlock::Json { name, value } => {
                check_text(name, text_bytes)?;
                check_text(value, text_bytes)?;
            }
            TraceDetailBlock::Section { title, blocks } => {
                check_text(title, text_bytes)?;
                validate_detail_blocks(blocks, text_bytes, depth + 1)?;
            }
            TraceDetailBlock::Table {
                title,
                columns,
                rows,
            } => {
                if let Some(title) = title {
                    check_text(title, text_bytes)?;
                }
                check_limit(columns.len(), 32, "detail table columns (32)")?;
                check_limit(rows.len(), 256, "detail table rows (256)")?;
                for column in columns {
                    check_text(column, text_bytes)?;
                }
                for row in rows {
                    check_limit(row.len(), 32, "detail table cells per row (32)")?;
                    for cell in row {
                        check_text(cell, text_bytes)?;
                    }
                }
            }
        }
    }
    Ok(())
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
