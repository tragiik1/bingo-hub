#![windows_subsystem = "windows"]

mod art;
mod app;
mod bingonet;
mod bingosave;
mod boards;
mod classes;
mod core;
mod crash;
mod graces;
mod items;
mod phone;
mod randocheck;
mod sys;
mod ui;
mod update;
mod weapons;
mod worker;

use eframe::egui;

fn main() {
    // After a self-update the old copy may still be closing: wait for it instead of bailing.
    let updated = std::env::args().any(|a| a == "--updated");
    // Debug screenshot runs (BINGOHUB_SHOT) sit beside a running hub instead of focusing it.
    let mutex = if cfg!(debug_assertions) && std::env::var_os("BINGOHUB_SHOT").is_some() { r"Local\EldenBingoHubShot" } else { r"Local\EldenBingoHub" };
    let mut first = sys::single_instance(mutex);
    for _ in 0..60 {
        if first || !updated {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        first = sys::single_instance(mutex);
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
