use text_diagram::{
    AsciiRenderer, CellFill, CellSpan, CellSpec, ConcreteGraph, DiagramBlock, DiagramRenderer,
    DiagramSpec, GraphSpec, RenderWidth, RowSpec, RowsSpec, UnicodeRenderer,
};

#[test]
fn mixed_diagram_separates_flow_graph_from_capacity_rows() -> Result<(), Box<dyn std::error::Error>>
{
    let mut graph = ConcreteGraph::new();
    let ingest = graph.add_node("ingest")?;
    let compact = graph.add_node("compact")?;
    graph.add_labeled_edge(ingest, compact, "schedule")?;

    let rows = RowsSpec::new(vec![
        RowSpec::new(vec![CellSpec::new("worker capacity", span(4)?)]),
        RowSpec::new(vec![
            CellSpec::filled(span(1)?, CellFill::Solid),
            CellSpec::filled(span(2)?, CellFill::Medium),
            CellSpec::filled(span(1)?, CellFill::Light),
        ]),
    ])?;

    let spec = DiagramSpec::new(vec![
        DiagramBlock::Graph(GraphSpec::from_graph(&graph)?),
        DiagramBlock::Rows(rows),
    ]);
    let rendered = UnicodeRenderer::with_width(RenderWidth::new(41)?).render(&spec);

    assert!(rendered.contains("│ ingest │ ─ schedule ─▶ │ compact │"));
    assert!(rendered.contains("\n\n┌"));
    assert!(rendered.contains("worker capacity"));
    assert!(rendered.contains("│█████████│▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│▒▒▒▒▒▒▒▒▒│"));
    Ok(())
}

#[test]
fn mixed_diagram_has_ascii_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let rows = RowsSpec::new(vec![RowSpec::new(vec![CellSpec::new(
        "fallback",
        span(2)?,
    )])])?;
    let spec = DiagramSpec::from_rows(rows);
    let rendered = AsciiRenderer::with_width(RenderWidth::new(19)?).render(&spec);

    assert_eq!(
        rendered,
        "+-----------------+\n|    fallback     |\n+-----------------+"
    );
    Ok(())
}

fn span(units: u16) -> Result<CellSpan, Box<dyn std::error::Error>> {
    Ok(CellSpan::new(units)?)
}
