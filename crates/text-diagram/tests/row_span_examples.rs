use text_diagram::{
    AsciiRenderer, CellAlign, CellFill, CellSpan, CellSpec, DiagramRenderer, DiagramSpec,
    RenderWidth, RowSpec, RowsSpec, UnicodeRenderer,
};

#[test]
fn two_column_status_row_renders_ascii_and_unicode() -> Result<(), Box<dyn std::error::Error>> {
    let rows = RowsSpec::new(vec![RowSpec::new(vec![
        CellSpec::new("left", span(1)?),
        CellSpec::new("right", span(1)?),
    ])])?;
    let spec = DiagramSpec::from_rows(rows);
    let width = RenderWidth::new(19)?;

    let ascii = AsciiRenderer::with_width(width).render(&spec);
    let unicode = UnicodeRenderer::with_width(width).render(&spec);

    assert_eq!(
        ascii,
        "+--------+--------+\n|  left  | right  |\n+--------+--------+"
    );
    assert_eq!(
        unicode,
        "┌────────┬────────┐\n│  left  │ right  │\n└────────┴────────┘"
    );
    Ok(())
}

#[test]
fn interval_annotation_uses_plain_row_and_dash_fill() -> Result<(), Box<dyn std::error::Error>> {
    let rows = RowsSpec::new(vec![RowSpec::plain(vec![
        CellSpec::new("range", span(2)?).with_fill(CellFill::Dash),
    ])])?;
    let spec = DiagramSpec::from_rows(rows);
    let width = RenderWidth::new(19)?;

    let ascii = AsciiRenderer::with_width(width).render(&spec);
    let unicode = UnicodeRenderer::with_width(width).render(&spec);

    assert_eq!(ascii, " ------range------ ");
    assert_eq!(unicode, " ──────range────── ");
    Ok(())
}

#[test]
fn compaction_like_rows_cover_spans_fills_and_labels() -> Result<(), Box<dyn std::error::Error>> {
    let rows = RowsSpec::new(vec![
        RowSpec::new(vec![CellSpec::new("input: SR(9)", span(6)?)]),
        RowSpec::new(vec![
            CellSpec::new("SST(91)", span(2)?),
            CellSpec::new("SST(92)", span(2)?),
            CellSpec::new("SST(93)", span(2)?),
        ]),
        RowSpec::new(vec![
            CellSpec::filled(span(2)?, CellFill::Solid),
            CellSpec::filled(span(2)?, CellFill::Medium),
            CellSpec::filled(span(2)?, CellFill::Light),
        ]),
        RowSpec::new(vec![
            CellSpec::new("subcompaction A", span(3)?),
            CellSpec::new("subcompaction B", span(3)?),
        ]),
        RowSpec::plain(vec![
            CellSpec::new("(-inf,k)", span(3)?).with_fill(CellFill::Dash),
            CellSpec::new("(k,+inf)", span(3)?).with_fill(CellFill::Dash),
        ]),
        RowSpec::new(vec![CellSpec::new("output: SR(10)", span(6)?)]),
    ])?;
    let spec = DiagramSpec::from_rows(rows);
    let rendered = UnicodeRenderer::with_width(RenderWidth::new(61)?).render(&spec);

    assert_eq!(rendered.lines().count(), 16);
    assert!(rendered.contains("input: SR(9)"));
    assert!(rendered.contains("SST(91)"));
    assert!(rendered.contains("subcompaction A"));
    assert!(rendered.contains("─(-inf,k)"));
    assert!(rendered.contains("█"));
    assert!(rendered.contains("▓"));
    assert!(rendered.contains("▒"));
    assert!(rendered.contains("output: SR(10)"));
    Ok(())
}

#[test]
fn alignment_options_are_visible_in_fixed_cells() -> Result<(), Box<dyn std::error::Error>> {
    let rows = RowsSpec::new(vec![RowSpec::new(vec![
        CellSpec::new("L", span(1)?).with_align(CellAlign::Left),
        CellSpec::new("C", span(1)?).with_align(CellAlign::Center),
        CellSpec::new("R", span(1)?).with_align(CellAlign::Right),
    ])])?;
    let spec = DiagramSpec::from_rows(rows);
    let rendered = AsciiRenderer::with_width(RenderWidth::new(28)?).render(&spec);

    assert_eq!(
        rendered,
        "+--------+--------+--------+\n|L       |   C    |       R|\n+--------+--------+--------+"
    );
    Ok(())
}

fn span(units: u16) -> Result<CellSpan, Box<dyn std::error::Error>> {
    Ok(CellSpan::new(units)?)
}
