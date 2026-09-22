#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod notch;
mod screenshot;
mod tray;
mod usage;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let settings = config::Settings::load();
    let notch_sizes = notch::edge::NotchSize::default();
    let initial_pos = notch::edge::window_position(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)),
        settings.edge,
        settings.offset_along_edge,
        notch_sizes.collapsed,
    );

    let viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_inner_size(notch_sizes.collapsed)
        .with_position(initial_pos);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "win-notch",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
