mod app;
mod diff;
mod imaging;
mod model;
mod pdf;
mod preview;
mod review;
mod secrets;
mod services;
mod ui;
mod update;
use eframe::egui;

fn main() -> eframe::Result {
    if let Some(path) = std::env::args()
        .nth(1)
        .filter(|s| s == "--preview")
        .and_then(|_| std::env::args().nth(2))
    {
        preview::render(&path);
        return Ok(());
    }
    eframe::run_native(
        "documentaSHion",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1440., 920.])
                .with_min_inner_size([1100., 680.])
                .with_app_id("documentashion"),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx, true)))),
    )
}
