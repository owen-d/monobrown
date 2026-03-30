use tui_lib::devkit::vim_editor::vim_editor_interactive_catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = vim_editor_interactive_catalog();
    tui_lib::devkit::playground::run(&catalog)
}
