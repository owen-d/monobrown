use text_diagram::{
    AsciiRenderer, CellDepth, CellFill, CellSpan, CellSpec, CornerStyle, DepthStyle,
    DiagramRenderer, DiagramSpec, RenderConfig, RenderWidth, RowSpec, RowsSpec, UnicodeRenderer,
};

#[test]
fn three_tier_app_uses_stacked_cells_for_replicas() -> Result<(), Box<dyn std::error::Error>> {
    let spec = DiagramSpec::from_rows(three_tier_rows()?);
    let width = RenderWidth::new(67)?;

    let unicode = UnicodeRenderer::with_width(width).render(&spec);
    for line in unicode.lines() {
        assert!(line.chars().count() <= usize::from(width.columns()));
    }
    assert_eq!(
        unicode,
        concat!(
            "┌───────────────────────────────────────────────────────────┐\n",
            "│                      client traffic                       │\n",
            "└───────────────────────────────────────────────────────────┘\n",
            " ───────────────────────────HTTPS─────────────────────────── \n",
            "┌───────────────────┬┐┐───────────────────┬┐───────────────────┐\n",
            "│    web pods x3    │││    api pods x2    ││     postgres      │\n",
            "└───────────────────┴┘┘───────────────────┴┘───────────────────┘\n",
            " ──────service────── ────────SQL──────── ──────volume─────── \n",
            "┌───────────────────┬┐┐───────────────────┬┐───────────────────┐\n",
            "│███████████████████│││▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓││▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│\n",
            "└───────────────────┴┘┘───────────────────┴┘───────────────────┘",
        )
    );

    let ascii = AsciiRenderer::with_width(width).render(&spec);
    assert_eq!(
        ascii,
        concat!(
            "+-----------------------------------------------------------+\n",
            "|                      client traffic                       |\n",
            "+-----------------------------------------------------------+\n",
            " ---------------------------HTTPS--------------------------- \n",
            "+-------------------+++-------------------++-------------------+\n",
            "|    web pods x3    |||    api pods x2    ||     postgres      |\n",
            "+-------------------+++-------------------++-------------------+\n",
            " ------service------ --------SQL-------- ------volume------- \n",
            "+-------------------+++-------------------++-------------------+\n",
            "|###################|||===================||-------------------|\n",
            "+-------------------+++-------------------++-------------------+",
        )
    );
    Ok(())
}

#[test]
fn three_tier_app_can_project_depth_downward() -> Result<(), Box<dyn std::error::Error>> {
    let spec = DiagramSpec::from_rows(three_tier_rows()?);
    let width = RenderWidth::new(67)?;
    let unicode = UnicodeRenderer::with_depth_style(width, DepthStyle::Projected3d).render(&spec);

    for line in unicode.lines() {
        assert!(line.chars().count() <= usize::from(width.columns()));
    }
    assert_eq!(
        unicode,
        concat!(
            "┌───────────────────────────────────────────────────────────┐\n",
            "│                      client traffic                       │\n",
            "└───────────────────────────────────────────────────────────┘\n",
            " ───────────────────────────HTTPS─────────────────────────── \n",
            "┌───────────────────┬┐┐───────────────────┬┐───────────────────┐\n",
            "│    web pods x3    │││    api pods x2    ││     postgres      │\n",
            "└───────────────────┘││───────────────────┘│───────────────────┘\n",
            " └───────────────────┘│└───────────────────┘\n",
            "  └───────────────────┘\n",
            " ──────service────── ────────SQL──────── ──────volume─────── \n",
            "┌───────────────────┬┐┐───────────────────┬┐───────────────────┐\n",
            "│███████████████████│││▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓││▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│\n",
            "└───────────────────┘││───────────────────┘│───────────────────┘\n",
            " └───────────────────┘│└───────────────────┘\n",
            "  └───────────────────┘",
        )
    );
    Ok(())
}

#[test]
fn three_tier_app_can_use_rounded_projected_depth() -> Result<(), Box<dyn std::error::Error>> {
    let spec = DiagramSpec::from_rows(three_tier_rows()?);
    let width = RenderWidth::new(67)?;
    let unicode = UnicodeRenderer::new(
        RenderConfig::new(width)
            .with_depth_style(DepthStyle::Projected3d)
            .with_corner_style(CornerStyle::Rounded),
    )
    .render(&spec);

    for line in unicode.lines() {
        assert!(line.chars().count() <= usize::from(width.columns()));
    }
    assert_eq!(
        unicode,
        concat!(
            "╭───────────────────────────────────────────────────────────╮\n",
            "│                      client traffic                       │\n",
            "╰───────────────────────────────────────────────────────────╯\n",
            " ───────────────────────────HTTPS─────────────────────────── \n",
            "╭───────────────────┬╮╮───────────────────┬╮───────────────────╮\n",
            "│    web pods x3    │││    api pods x2    ││     postgres      │\n",
            "╰───────────────────╯││───────────────────╯│───────────────────╯\n",
            " ╰───────────────────╯│╰───────────────────╯\n",
            "  ╰───────────────────╯\n",
            " ──────service────── ────────SQL──────── ──────volume─────── \n",
            "╭───────────────────┬╮╮───────────────────┬╮───────────────────╮\n",
            "│███████████████████│││▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓││▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│\n",
            "╰───────────────────╯││───────────────────╯│───────────────────╯\n",
            " ╰───────────────────╯│╰───────────────────╯\n",
            "  ╰───────────────────╯",
        )
    );
    Ok(())
}

fn three_tier_rows() -> Result<RowsSpec, Box<dyn std::error::Error>> {
    Ok(RowsSpec::new(vec![
        RowSpec::new(vec![CellSpec::new("client traffic", span(6)?)]),
        RowSpec::plain(vec![
            CellSpec::new("HTTPS", span(6)?).with_fill(CellFill::Dash),
        ]),
        RowSpec::new(vec![
            CellSpec::new("web pods x3", span(2)?).with_depth(CellDepth::new(3)?),
            CellSpec::new("api pods x2", span(2)?).with_depth(CellDepth::new(2)?),
            CellSpec::new("postgres", span(2)?),
        ]),
        RowSpec::plain(vec![
            CellSpec::new("service", span(2)?).with_fill(CellFill::Dash),
            CellSpec::new("SQL", span(2)?).with_fill(CellFill::Dash),
            CellSpec::new("volume", span(2)?).with_fill(CellFill::Dash),
        ]),
        RowSpec::new(vec![
            CellSpec::filled(span(2)?, CellFill::Solid).with_depth(CellDepth::new(3)?),
            CellSpec::filled(span(2)?, CellFill::Medium).with_depth(CellDepth::new(2)?),
            CellSpec::filled(span(2)?, CellFill::Light),
        ]),
    ])?)
}

fn span(units: u16) -> Result<CellSpan, Box<dyn std::error::Error>> {
    Ok(CellSpan::new(units)?)
}
