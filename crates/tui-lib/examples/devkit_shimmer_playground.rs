use std::time::Duration;

use tui_lib::devkit::simple_widgets::{render_shimmer, shimmer_catalog};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = shimmer_catalog();
    let index = catalog
        .scenario_index_by_name("truecolor-mid")
        .ok_or("shimmer catalog missing 'truecolor-mid' scenario")?;
    let state = catalog.initial_state(index).clone();
    tui_lib::devkit::playground::run_animated(
        state,
        "shimmer",
        render_shimmer,
        |state, dt| state.elapsed += dt,
        Duration::from_millis(100),
    )
}
