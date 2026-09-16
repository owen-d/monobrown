//! Exact admitted time range retained alongside proportional presentation.

use super::data::TraceData;

/// A nonempty inclusive range over supplied trace coordinates.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::trace_view::{
        CategoryId, ItemId, TraceCategory, TraceItem, TraceTimeUnit, TraceTiming, TraceTrack,
        TraceVisualRole, TrackId,
    };

    fn data(times: &[i64]) -> TraceData {
        TraceData {
            time_unit: TraceTimeUnit::Nanoseconds,
            categories: vec![TraceCategory {
                id: CategoryId(0),
                label: "activity".into(),
                role: TraceVisualRole::Neutral,
            }],
            groups: vec![],
            tracks: vec![TraceTrack {
                id: TrackId(0),
                group: None,
                label: "worker".into(),
                details: vec![],
                items: times
                    .iter()
                    .enumerate()
                    .map(|(id, &at)| TraceItem {
                        id: ItemId(id as u64),
                        category: CategoryId(0),
                        label: "event".into(),
                        timing: TraceTiming::Instant(at),
                        details: vec![],
                        children: vec![],
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn fit_handles_empty_equal_and_extreme_coordinates() {
        assert_eq!(TimeWindow::fit(&data(&[])), TimeWindow { start: 0, end: 1 });
        assert_eq!(
            TimeWindow::fit(&data(&[7])),
            TimeWindow { start: 7, end: 8 }
        );
        assert_eq!(
            TimeWindow::fit(&data(&[i64::MIN, i64::MAX])),
            TimeWindow {
                start: i64::MIN,
                end: i64::MAX,
            }
        );
    }
}
