//! RadioCraft: plan Wi-Fi coverage on floor plans from CADCraft and models from SolidCraft.

mod app;
mod i18n;
mod theme;

const USAGE: &str = "usage: radiocraft [plan.dxf|model.obj|model.stl|project.rcraft] [--script commands.json] [--screenshot out.png]";

fn main() -> eframe::Result {
    let (mut path, mut script, mut screenshot) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--script" => script = args.next(),
            "--screenshot" => screenshot = args.next(),
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => path = Some(a),
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RadioCraft")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: radiocraft_engine::icon::rgba(256),
                width: 256,
                height: 256,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "RadioCraft",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(path, script, screenshot)))),
    )
}
