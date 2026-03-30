use mb_tui::devkit::command_palette::command_palette_interactive_catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = command_palette_interactive_catalog();
    mb_tui::devkit::playground::run(&catalog)
}
