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
            .with_icon(std::sync::Arc::new(icon())),
        ..Default::default()
    };
    eframe::run_native(
        "RadioCraft",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(path, script, screenshot)))),
    )
}

/// The window's icon: the logo drawn in VectorCraft (`assets/icon.vectorcraft`) and exported as
/// `assets/icon-1024.png`. Empty (the platform's default) if it can't be read.
fn icon() -> egui::IconData {
    let read = || -> Option<egui::IconData> {
        let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
            "../../../assets/icon-1024.png"
        )));
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        buf.truncate(info.buffer_size());
        Some(egui::IconData {
            rgba: buf,
            width: info.width,
            height: info.height,
        })
    };
    read().unwrap_or_default()
}
