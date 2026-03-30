//! A data-driven development kit for snapshot-testing TUI components.
//!
//! Provides [`Surface`] for off-screen rendering, [`Scenario`] and
//! [`ScenarioCatalog`] for organizing test states, and snapshot assertion
//! integration via [`insta`].
//!
//! Gated behind the `devkit` feature flag.

pub mod bar_selector;
pub mod color;
pub mod command_palette;
pub mod dispatch_demo;
pub mod flame_graph;
pub mod flashcard_demo;
pub mod frame_tape;
pub mod gauge_demo;
pub mod hotkey_coach_demo;
pub mod playground;
pub mod progress_demo;
pub mod queue_demo;
pub mod rearview_mirror_demo;
mod scenario;
pub mod simple_widgets;
pub mod slider_demo;
pub mod sparkline_demo;
pub mod stepper_demo;
mod surface;
pub mod tab_bar_demo;
mod text;
pub mod unified;
pub mod vim_editor;

pub use playground::{Action, PlaygroundController, PlaygroundMode};
pub use scenario::{Scenario, ScenarioCatalog};
pub use surface::Surface;
pub use text::{buffer_to_ansi, buffer_to_styled_text, buffer_to_text};
