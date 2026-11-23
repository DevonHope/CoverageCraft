//! RadioCraft: plan Wi-Fi coverage on floor plans from CADCraft and models from SolidCraft.

mod app;
mod i18n;
mod theme;

use craft_window::Prefix;
use craft_window::policy::{Backend, Frame, TitleBarPref, apply, decide_for};

const USAGE: &str = "usage: radiocraft [plan.dxf|design.sc|model.obj|model.stl|project.rcraft] [--script commands.json] [--screenshot out.png]";

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
    // The title bar every Craft app wears: on macOS the app's bar sits under the window buttons
    // (craft-window's Inset frame); elsewhere the system draws it, for now.
    let pref = if cfg!(target_os = "macos") {
        TitleBarPref::Integrated
    } else {
        TitleBarPref::System
    };
    let decision = decide_for(Prefix::new("RADIOCRAFT"), Backend::from_env(), pref);
    let leading = match &decision.frame {
        Frame::Inset(inset) => inset.leading,
        _ => 0.0,
    };
    let viewport = apply(
        egui::ViewportBuilder::default()
            .with_title("RadioCraft")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(std::sync::Arc::new(icon())),
        &decision.frame,
    );
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "RadioCraft",
        options,
        Box::new(move |_cc| {
            #[allow(unused_mut)]
            let mut app = app::App::new(path, script, screenshot, leading);
            // The menus go in the macOS menu bar; if AppKit refuses them they stay in the window.
            #[cfg(target_os = "macos")]
            {
                app.native = craft_menu::mac::install(
                    &_cc.egui_ctx,
                    &craft_menu::mac_layout(
                        &app.menu_model(),
                        &craft_menu::LayoutOptions::new("RadioCraft"),
                    )
                    .bar,
                );
            }
            Ok(Box::new(app))
        }),
    )
}

/// The logo drawn in VectorCraft (`assets/icon.vectorcraft`), exported as `assets/icon-1024.png`.
fn logo_rgba() -> Option<(Vec<u8>, u32, u32)> {
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
    Some((buf, info.width, info.height))
}

/// The logo as an image for the app bar, Home and About; a blank one if it can't be read.
pub fn logo_image() -> egui::ColorImage {
    match logo_rgba() {
        Some((rgba, w, h)) => {
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
        }
        None => egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]),
    }
}

/// The window's icon: the logo. Empty (the platform's default) if it can't be read.
fn icon() -> egui::IconData {
    logo_rgba()
        .map(|(rgba, width, height)| egui::IconData {
            rgba,
            width,
            height,
        })
        .unwrap_or_default()
}
