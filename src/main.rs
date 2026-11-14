mod app;
mod claude;
mod diff;
mod imaging;
mod marking;
mod model;
mod pagecopy;
mod pdf;
mod preview;
mod review;
mod secrets;
mod services;
mod ui;
mod update;
mod worksheet;
use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "ws") {
        if let Err(e) = worksheet::cli::main(&args[1..]) {
            eprintln!("documentashion ws: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
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
