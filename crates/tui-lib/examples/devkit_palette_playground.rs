use tui_lib::devkit::command_palette::command_palette_interactive_catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = command_palette_interactive_catalog();
    tui_lib::devkit::playground::run(&catalog)
}
