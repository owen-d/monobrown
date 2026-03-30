use ratatui::style::Color;

/// Unique identifier for a span within a flame graph tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpanId(pub u32);

/// Describes one cost dimension (e.g. "cpu", "io") with its display color.
#[derive(Clone, Debug)]
pub struct CostType {
    pub name: &'static str,
    pub color: Color,
}

/// Per-span cost breakdown across all cost dimensions.
///
/// Each element in `amounts` corresponds to the `CostType` at the same index.
#[derive(Clone, Debug)]
pub struct CostBreakdown {
    pub amounts: Vec<f64>,
}

impl CostBreakdown {
    pub fn total(&self) -> f64 {
        self.amounts.iter().sum()
    }
}

/// A node in the span tree. Children are sorted by total cost descending.
#[derive(Clone, Debug)]
pub struct SpanNode {
    pub id: SpanId,
    pub label: String,
    pub costs: CostBreakdown,
    pub children: Vec<SpanNode>,
}

/// Builder for constructing span trees with auto-assigned IDs.
#[derive(Default)]
pub struct SpanNodeBuilder {
    next_id: u32,
}

impl SpanNodeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a span node. Children are sorted by total cost descending.
    pub fn span(
        &mut self,
        label: &str,
        amounts: Vec<f64>,
        mut children: Vec<SpanNode>,
    ) -> SpanNode {
        let id = SpanId(self.next_id);
        self.next_id += 1;
        children.sort_by(|a, b| {
            b.costs
                .total()
                .partial_cmp(&a.costs.total())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        SpanNode {
            id,
            label: label.to_string(),
            costs: CostBreakdown { amounts },
            children,
        }
    }

    /// Create a leaf span (no children).
    pub fn leaf(&mut self, label: &str, amounts: Vec<f64>) -> SpanNode {
        self.span(label, amounts, vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_breakdown_total() {
        let cb = CostBreakdown {
            amounts: vec![1.0, 2.0, 3.0],
        };
        assert!((cb.total() - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_breakdown_empty_is_zero() {
        let cb = CostBreakdown { amounts: vec![] };
        assert!((cb.total()).abs() < f64::EPSILON);
    }

    #[test]
    fn builder_assigns_sequential_ids() {
        let mut b = SpanNodeBuilder::new();
        let a = b.leaf("a", vec![1.0]);
        let bb = b.leaf("b", vec![2.0]);
        assert_eq!(a.id, SpanId(0));
        assert_eq!(bb.id, SpanId(1));
    }

    #[test]
    fn builder_sorts_children_by_total_descending() {
        let mut b = SpanNodeBuilder::new();
        let small = b.leaf("small", vec![1.0]);
        let big = b.leaf("big", vec![10.0]);
        let mid = b.leaf("mid", vec![5.0]);
        let parent = b.span("parent", vec![16.0], vec![small, big, mid]);

        assert_eq!(parent.children.len(), 3);
        assert_eq!(parent.children[0].label, "big");
        assert_eq!(parent.children[1].label, "mid");
        assert_eq!(parent.children[2].label, "small");
    }

    #[test]
    fn builder_leaf_has_no_children() {
        let mut b = SpanNodeBuilder::new();
        let leaf = b.leaf("leaf", vec![1.0, 2.0]);
        assert!(leaf.children.is_empty());
        assert!((leaf.costs.total() - 3.0).abs() < f64::EPSILON);
    }
}
