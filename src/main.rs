mod app;
mod diagram;
mod fonts;
mod mdview;
mod model;
mod preview;
mod find;
mod git;
mod pricing;
mod cheatsheet;
mod linemove;
mod settings;
mod spell;
mod store;
mod textdir;
mod theme;

/// The wordesign logo, 256x256 raw RGBA.
pub const ICON_RGBA: &[u8] = include_bytes!("../assets/icon_256.rgba");
pub const ICON_SIZE: u32 = 256;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("wordesign {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let icon = eframe::egui::IconData { rgba: ICON_RGBA.to_vec(), width: ICON_SIZE, height: ICON_SIZE };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 860.0])
            .with_maximized(true)
            .with_min_inner_size([900.0, 520.0])
            .with_title("wordesign")
            .with_app_id("wordesign")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "wordesign",
        options,
        Box::new(|cc| Ok(Box::new(app::PromptEditorApp::new(cc)))),
    )
}
