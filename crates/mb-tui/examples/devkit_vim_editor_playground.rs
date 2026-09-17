use mb_tui::devkit::vim_editor::vim_editor_interactive_catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    mb_tui::devkit::playground::run(vim_editor_interactive_catalog)
}
