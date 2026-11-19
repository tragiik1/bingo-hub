#![windows_subsystem = "windows"]

mod app;
mod bingosave;
mod classes;
mod core;
mod crash;
mod graces;
mod randocheck;
mod sys;
mod ui;
mod update;
mod worker;

use eframe::egui;

fn main() {
    // After a self-update the old copy may still be closing: wait for it instead of bailing.
    let updated = std::env::args().any(|a| a == "--updated");
    let mut first = sys::single_instance(r"Local\EldenBingoHub");
    for _ in 0..60 {
        if first || !updated {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        first = sys::single_instance(r"Local\EldenBingoHub");
    }
    if !first {
        sys::focus_titled("Bingo Hub");
        return;
    }
    update::cleanup();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Bingo Hub")
        .with_inner_size([1100.0, 740.0])
        .with_min_inner_size([940.0, 640.0])
        .with_decorations(false)
        .with_resizable(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, centered: true, vsync: true, ..Default::default() };
    let _ = eframe::run_native("Bingo Hub", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))));
}
