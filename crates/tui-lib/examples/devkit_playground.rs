/// Unified devkit playground combining all widget catalogs into one tabbed
/// session. Tab/Shift-Tab switches between entries; each entry keeps its
/// own explorer/live state.
use std::time::Duration;

use mb_tui::devkit::bar_selector::bar_selector_catalog;
use mb_tui::devkit::command_palette::command_palette_interactive_catalog;
use mb_tui::devkit::dispatch_demo;
use mb_tui::devkit::flame_graph::flame_graph_interactive_catalog;
use mb_tui::devkit::flashcard_demo;
use mb_tui::devkit::gauge_demo;
use mb_tui::devkit::hotkey_coach_demo as hotkey_demo;
use mb_tui::devkit::progress_demo;
use mb_tui::devkit::queue_demo;
use mb_tui::devkit::rearview_mirror_demo as rearview_demo;
use mb_tui::devkit::simple_widgets::{
    ShimmerDemo, render_shimmer, render_spinner, shimmer_catalog,
};
use mb_tui::devkit::slider_demo;
use mb_tui::devkit::sparkline_demo;
use mb_tui::devkit::stepper_demo;
use mb_tui::devkit::tab_bar_demo as tab_demo;
use mb_tui::devkit::unified::{animated_entry, animated_interactive_entry, entry, run_unified};
use mb_tui::devkit::vim_editor::vim_editor_interactive_catalog;

// ===========================================================================
// Main
// ===========================================================================

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let shimmer_catalog = shimmer_catalog();
    let Some(shimmer_index) = shimmer_catalog.scenario_index_by_name("truecolor-mid") else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "shimmer catalog missing 'truecolor-mid' scenario",
        ));
    };
    let shimmer_state = shimmer_catalog.initial_state(shimmer_index).clone();

    let entries = vec![
        // --- Existing catalog entries ---
        entry("Palette", command_palette_interactive_catalog()),
        entry("Flame Graph", flame_graph_interactive_catalog()),
        entry("Vim Editor", vim_editor_interactive_catalog()),
        entry("Bar Selector", bar_selector_catalog()),
        // --- Animated entries ---
        animated_entry(
            "Spinner",
            Duration::ZERO,
            render_spinner,
            |elapsed, dt| *elapsed += dt,
            Duration::from_millis(100),
        ),
        animated_entry(
            "Shimmer",
            shimmer_state,
            render_shimmer,
            |state: &mut ShimmerDemo, dt| state.elapsed += dt,
            Duration::from_millis(100),
        ),
        // --- New widget demos ---
        animated_interactive_entry(
            "Sparkline",
            sparkline_demo::initial_state(),
            sparkline_demo::render,
            sparkline_demo::tick,
            sparkline_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Progress",
            progress_demo::initial_state(),
            progress_demo::render,
            progress_demo::tick,
            progress_demo::apply,
            Duration::from_millis(50),
        ),
        animated_interactive_entry(
            "Gauge",
            gauge_demo::initial_state(),
            gauge_demo::render,
            gauge_demo::tick,
            gauge_demo::apply,
            Duration::from_millis(50),
        ),
        animated_interactive_entry(
            "Tab Bar",
            tab_demo::initial_state(),
            tab_demo::render,
            tab_demo::tick,
            tab_demo::apply,
            Duration::from_millis(16),
        ),
        animated_interactive_entry(
            "Stepper",
            stepper_demo::initial_state(),
            stepper_demo::render,
            stepper_demo::tick,
            stepper_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Slider",
            slider_demo::initial_state(),
            slider_demo::render,
            slider_demo::tick,
            slider_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Queue",
            queue_demo::initial_state(),
            queue_demo::render,
            queue_demo::tick,
            queue_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Flashcard",
            flashcard_demo::initial_state(),
            flashcard_demo::render,
            flashcard_demo::tick,
            flashcard_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Hotkeys",
            hotkey_demo::initial_state(),
            hotkey_demo::render,
            hotkey_demo::tick,
            hotkey_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Rearview",
            rearview_demo::initial_state(),
            rearview_demo::render,
            rearview_demo::tick,
            rearview_demo::apply,
            Duration::from_millis(100),
        ),
        animated_interactive_entry(
            "Dispatch",
            dispatch_demo::initial_state(),
            dispatch_demo::render,
            dispatch_demo::tick,
            dispatch_demo::apply,
            Duration::from_millis(100),
        ),
    ];

    run_unified(entries)
}
