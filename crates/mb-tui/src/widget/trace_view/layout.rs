//! Immutable time lanes and integer viewport geometry.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::data::{TraceData, TraceTrack};

/// One stable track row; untimed observations each have their own lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lane {
    pub items: Vec<usize>,
    pub timed: bool,
}

/// Pack positive half-open intervals in O(n log n), then append observations.
///
/// Sorting uses recorded endpoints and identity, never input order or pixels.
/// Two heaps release finished lanes and reuse the lowest available index.
pub(crate) fn track_lanes(track: &TraceTrack) -> Vec<Lane> {
    let mut intervals = Vec::new();
    let mut observations = Vec::new();
    let mut untimed = Vec::new();
    for (index, item) in track.items.iter().enumerate() {
        if let Some((start, end)) = item.timing.interval() {
            intervals.push((start, end, item.id, index));
        } else if item.timing.coordinates()[0].is_some() {
            observations.push(index);
        } else {
            untimed.push(index);
        }
    }
    intervals.sort_unstable();
    let mut active = BinaryHeap::new();
    let mut free = BinaryHeap::new();
    let mut lanes: Vec<Lane> = Vec::new();
    for (start, end, _, item) in intervals {
        while let Some(&Reverse((finished, lane))) = active.peek() {
            if finished > start {
                break;
            }
            active.pop();
            free.push(Reverse(lane));
        }
        let lane = free.pop().map_or_else(
            || {
                lanes.push(Lane {
                    items: Vec::new(),
                    timed: true,
                });
                lanes.len() - 1
            },
            |Reverse(lane)| lane,
        );
        lanes[lane].items.push(item);
        active.push(Reverse((end, lane)));
    }
    observations.sort_unstable_by_key(|&i| {
        let item = &track.items[i];
        (
            item.timing.coordinates().into_iter().flatten().min(),
            item.id,
        )
    });
    if !observations.is_empty() {
        lanes.push(Lane {
            items: observations,
            timed: true,
        });
    }
    untimed.sort_unstable_by_key(|&i| track.items[i].id);
    for item in untimed {
        lanes.push(Lane {
            items: vec![item],
            timed: false,
        });
    }
    if lanes.is_empty() {
        lanes.push(Lane {
            items: Vec::new(),
            timed: false,
        });
    }
    lanes
}

/// A nonempty inclusive viewing range, independent of source interval validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeWindow {
    pub start: i64,
    pub end: i64,
}

impl TimeWindow {
    /// Fit recorded coordinates; empty and all-equal inputs still get a unit span.
    pub fn fit(data: &TraceData) -> Self {
        let mut times = data
            .tracks
            .iter()
            .flat_map(|track| &track.items)
            .flat_map(|item| item.timing.coordinates())
            .flatten();
        let Some(first) = times.next() else {
            return Self { start: 0, end: 1 };
        };
        let (mut start, mut end) = (first, first);
        for time in times {
            start = start.min(time);
            end = end.max(time);
        }
        if start == end {
            if end == i64::MAX {
                start -= 1;
            } else {
                end += 1;
            }
        }
        Self { start, end }
    }

    /// Compute differences only after widening, including the full signed range.
    pub fn span(self) -> i128 {
        i128::from(self.end) - i128::from(self.start)
    }

    /// Translate or scale a window while retaining its span at coordinate limits.
    pub fn shifted(start: i128, span: i128) -> Self {
        let span = span.clamp(1, i128::from(u64::MAX));
        let start = start.clamp(i128::from(i64::MIN), i128::from(i64::MAX) - span);
        Self {
            start: start as i64,
            end: (start + span) as i64,
        }
    }

    /// Map an observed point to a cell without rounding absolute timestamps.
    pub fn column(self, time: i64, width: u16) -> Option<u16> {
        if width == 0 || time < self.start || time > self.end {
            return None;
        }
        let offset = i128::from(time) - i128::from(self.start);
        Some((offset * i128::from(width - 1) / self.span()) as u16)
    }
}

/// Responsive chrome reserves at least one content row whenever height permits.
#[derive(Clone, Copy)]
pub(crate) struct ViewLayout {
    pub header: u16,
    pub body: u16,
    pub details: u16,
    pub footer: u16,
    pub labels: u16,
}

impl ViewLayout {
    /// Use text summaries below 24 columns; details yield to short terminals.
    pub fn new(width: u16, height: u16, show_details: bool) -> Self {
        let chrome = u16::from(height >= 3);
        let details = if show_details && height >= 7 {
            (height / 3).min(4)
        } else {
            0
        };
        Self {
            header: chrome,
            body: height - chrome * 2 - details,
            details,
            footer: chrome,
            labels: if width >= 24 { (width / 3).min(24) } else { 0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::trace_view::{ItemId, TraceItem, TraceTimeUnit, TraceTiming, TrackId};

    /// Construct owned observations without coupling fixtures to terminal state.
    fn track(items: &[(u64, TraceTiming)]) -> TraceTrack {
        TraceTrack {
            id: TrackId(0),
            group: None,
            label: "worker".into(),
            items: items
                .iter()
                .map(|&(id, timing)| TraceItem {
                    id: ItemId(id),
                    label: id.to_string(),
                    timing,
                    details: vec![],
                })
                .collect(),
        }
    }

    /// Inspect identities rather than input offsets when comparing reordered facts.
    fn lane_ids(track: &TraceTrack) -> Vec<Vec<ItemId>> {
        track_lanes(track)
            .iter()
            .map(|lane| lane.items.iter().map(|&i| track.items[i].id).collect())
            .collect()
    }

    /// Real overlap consumes a separate lane, while touching half-open intervals reuse it.
    #[test]
    fn concurrent_lanes_are_minimal_stable_and_independent_of_input_order() {
        let mut track = track(&[
            (1, TraceTiming::Interval { start: 0, end: 10 }),
            (2, TraceTiming::Interval { start: 2, end: 5 }),
            (3, TraceTiming::Interval { start: 5, end: 8 }),
            (4, TraceTiming::Interval { start: 10, end: 20 }),
        ]);
        let expected = vec![vec![ItemId(1), ItemId(4)], vec![ItemId(2), ItemId(3)]];
        assert_eq!(lane_ids(&track), expected);
        track.items.reverse();
        assert_eq!(lane_ids(&track), expected);
    }

    /// Unknown duration never allocates a positive work interval; ties use identity.
    #[test]
    fn incomplete_regressed_and_zero_pairs_are_observations() {
        let track = track(&[
            (8, TraceTiming::Untimed),
            (4, TraceTiming::MissingStart { end: 5 }),
            (2, TraceTiming::Instant(5)),
            (3, TraceTiming::MissingEnd { start: 5 }),
            (1, TraceTiming::Interval { start: 9, end: 2 }),
            (5, TraceTiming::Interval { start: 5, end: 5 }),
            (7, TraceTiming::Untimed),
        ]);
        let lanes = track_lanes(&track);
        assert_eq!(
            lane_ids(&track),
            vec![
                vec![ItemId(1), ItemId(2), ItemId(3), ItemId(4), ItemId(5)],
                vec![ItemId(7)],
                vec![ItemId(8)],
            ]
        );
        assert!(lanes[0].timed);
        assert!(!lanes[1].timed);
        assert!(!lanes[2].timed);
    }

    /// Exhaustive small coordinate pairs prove same-lane disjointness and minimal lane count.
    #[test]
    fn bounded_interval_enumeration_preserves_overlap() {
        let pairs: Vec<_> = (-3..3)
            .flat_map(|start| ((start + 1)..=3).map(move |end| (start, end)))
            .enumerate()
            .map(|(id, (start, end))| (id as u64, TraceTiming::Interval { start, end }))
            .collect();
        let track = track(&pairs);
        let lanes = track_lanes(&track);
        for lane in &lanes {
            for pair in lane.items.windows(2) {
                let (_, end) = track.items[pair[0]].timing.interval().unwrap();
                let (start, _) = track.items[pair[1]].timing.interval().unwrap();
                assert!(end <= start);
            }
        }
        let concurrency = (-3..3)
            .map(|time| {
                track
                    .items
                    .iter()
                    .filter(|item| {
                        let (start, end) = item.timing.interval().unwrap();
                        start <= time && time < end
                    })
                    .count()
            })
            .max()
            .unwrap();
        assert_eq!(lanes.len(), concurrency);
    }

    /// Epoch-sized and full-range coordinates retain precision before cell projection.
    #[test]
    fn integer_projection_handles_extremes_and_sub_float_precision() {
        let window = TimeWindow {
            start: i64::MIN,
            end: i64::MAX,
        };
        assert_eq!(window.span(), i128::from(u64::MAX));
        assert_eq!(window.column(i64::MIN, u16::MAX), Some(0));
        assert_eq!(window.column(i64::MAX, u16::MAX), Some(u16::MAX - 1));
        assert_eq!(window.column(0, 0), None);
        let epoch = 1_800_000_000_000_000_000;
        let precise = TimeWindow {
            start: epoch,
            end: epoch + 2,
        };
        assert_eq!(precise.column(epoch + 1, 3), Some(1));
        assert_eq!(precise.column(epoch - 1, 3), None);
    }

    /// Empty and all-equal traces provide a usable window without numeric overflow.
    #[test]
    fn empty_and_equal_time_ranges_fit() {
        let mut data = TraceData {
            time_unit: TraceTimeUnit::Nanoseconds,
            groups: vec![],
            tracks: vec![],
        };
        assert_eq!(TimeWindow::fit(&data), TimeWindow { start: 0, end: 1 });
        for time in [i64::MIN, 0, i64::MAX] {
            data.tracks = vec![track(&[(0, TraceTiming::Instant(time))])];
            let window = TimeWindow::fit(&data);
            assert_eq!(window.span(), 1);
            assert!(window.column(time, 80).is_some());
        }
    }
}
