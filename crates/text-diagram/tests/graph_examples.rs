use text_diagram::{
    AsciiRenderer, ConcreteGraph, DiagramRenderer, DiagramSpec, GraphDiagram, GraphEdge, GraphSpec,
    RenderWidth, UnicodeRenderer,
};

#[test]
fn concrete_graph_pipeline_renders_portable_ascii() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = ConcreteGraph::new();
    let input = graph.add_node("input")?;
    let parse = graph.add_node("parse")?;
    let output = graph.add_node("output")?;
    graph.add_labeled_edge(input, parse, "read")?;
    graph.add_labeled_edge(parse, output, "emit")?;

    let spec = DiagramSpec::from_graph(&graph)?;
    let rendered = AsciiRenderer::default().render(&spec);

    assert_eq!(
        rendered,
        "[input] --read--> [parse]\n[parse] --emit--> [output]"
    );
    Ok(())
}

#[test]
fn custom_graph_adapter_keeps_caller_graph_ownership() -> Result<(), Box<dyn std::error::Error>> {
    struct QueryPlan;

    impl GraphDiagram for QueryPlan {
        type NodeId = &'static str;

        fn nodes(&self) -> Vec<Self::NodeId> {
            vec!["scan", "filter", "aggregate"]
        }

        fn node_label(&self, node: &Self::NodeId) -> String {
            match *node {
                "scan" => "Scan parquet".to_owned(),
                "filter" => "Filter tenant".to_owned(),
                "aggregate" => "Group by day".to_owned(),
                _ => node.to_string(),
            }
        }

        fn edges(&self) -> Vec<GraphEdge<Self::NodeId>> {
            vec![
                GraphEdge::labeled("scan", "filter", "predicate"),
                GraphEdge::labeled("filter", "aggregate", "rows"),
            ]
        }
    }

    let graph = GraphSpec::from_graph(&QueryPlan)?;
    let spec = DiagramSpec::from_graph_spec(graph);
    let rendered = AsciiRenderer::default().render(&spec);

    assert_eq!(
        rendered,
        "[Filter tenant] --rows--> [Group by day]\n\
         [Scan parquet] --predicate--> [Filter tenant]"
    );
    Ok(())
}

#[test]
fn unicode_graph_renderer_produces_boxed_edge() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = ConcreteGraph::new();
    let cache = graph.add_node("cache")?;
    let database = graph.add_node("db")?;
    graph.add_labeled_edge(cache, database, "hit")?;

    let spec = DiagramSpec::from_graph(&graph)?;
    let rendered = UnicodeRenderer::with_width(RenderWidth::new(80)?).render(&spec);

    assert_eq!(
        rendered,
        "┌───────┐          ┌────┐\n\
         │ cache │ ─ hit ─▶ │ db │\n\
         └───────┘          └────┘"
    );
    Ok(())
}
