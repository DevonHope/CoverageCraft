//! The window's own frame on Windows and Linux, where craft-window gives the app's bar the job of
//! the title bar (`Frame::Integrated`): minimize, maximize and close buttons at the ends the desktop
//! puts them, in its family's look, and resize zones along the window's edges.
//!
//! Windows' family is full-height buttons, Close red on hover; Adwaita (GNOME), Breeze (KDE) and
//! the generic family are round buttons, Breeze's Close red on hover.

use craft_window::policy::CaptionFamily;
use craft_window::{Button, Layout};
use egui::{
    Color32, CursorIcon, Id, LayerId, Order, Rect, ResizeDirection, Sense, Stroke, Ui,
    ViewportCommand, pos2, vec2,
};

use crate::i18n::tr;
use crate::theme::TOKENS;

/// A caption button's width: Windows' 46 pt; the round families' 34 pt.
fn width(family: CaptionFamily) -> f32 {
    if family == CaptionFamily::Windows {
        46.0
    } else {
        34.0
    }
}

/// How wide the buttons at each end are, to keep the bar's own controls clear of them.
pub fn reserved(family: CaptionFamily, layout: &Layout) -> (f32, f32) {
    let w = width(family);
    (layout.left.len() as f32 * w, layout.right.len() as f32 * w)
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Draw the buttons at the ends of `bar` (the app bar's rect) and do what a click asks.
pub fn buttons(ui: &mut Ui, bar: Rect, family: CaptionFamily, layout: &Layout) {
    let ctx = ui.ctx().clone();
    let max = maximized(&ctx);
    let w = width(family);
    let left = layout
        .left
        .iter()
        .enumerate()
        .map(|(i, b)| (*b, bar.left() + i as f32 * w));
    let right_start = bar.right() - layout.right.len() as f32 * w;
    let right = layout
        .right
        .iter()
        .enumerate()
        .map(|(i, b)| (*b, right_start + i as f32 * w));
    let mut chosen = None;
    for (button, x) in left.chain(right) {
        let rect = Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height()));
        let resp = ui.interact(rect, Id::unique(("caption", button as u8)), Sense::click());
        let close = button == Button::Close;
        let (hovered, pressed) = (resp.hovered(), resp.is_pointer_button_down_on());
        let red_close =
            close && family != CaptionFamily::Adwaita && family != CaptionFamily::Generic;
        let fill = match (hovered || pressed, red_close) {
            (true, true) => TOKENS.caption_close,
            (true, false) => TOKENS.caption_hover,
            _ => Color32::TRANSPARENT,
        };
        let painter = ui.painter();
        if family == CaptionFamily::Windows {
            painter.rect_filled(rect, 0.0, fill);
        } else {
            // The round families: a circle behind the glyph, shown faintly at rest.
            let rest = if fill == Color32::TRANSPARENT {
                TOKENS.caption_rest
            } else {
                fill
            };
            painter.circle_filled(rect.center(), 11.0, rest);
        }
        let ink = if red_close && (hovered || pressed) {
            Color32::WHITE
        } else {
            TOKENS.text
        };
        glyph(ui, button, max, rect.center(), Stroke::new(1.2, ink));
        let tip = match button {
            Button::Minimize => tr("Minimize"),
            Button::Maximize if max => tr("Restore"),
            Button::Maximize => tr("Maximize"),
            Button::Close => tr("Close"),
        };
        if resp.on_hover_text(tip).clicked() {
            chosen = Some(button);
        }
    }
    match chosen {
        Some(Button::Minimize) => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
        Some(Button::Maximize) => ctx.send_viewport_cmd(ViewportCommand::Maximized(!max)),
        Some(Button::Close) => ctx.send_viewport_cmd(ViewportCommand::Close),
        None => {}
    }
}

/// The 10 pt line glyphs: a bar, a square (or two, to restore), a cross.
fn glyph(ui: &Ui, button: Button, maximized: bool, at: egui::Pos2, s: Stroke) {
    let p = ui.painter();
    let g = Rect::from_center_size(at, vec2(10.0, 10.0));
    match button {
        Button::Minimize => {
            p.line_segment([g.left_center(), g.right_center()], s);
        }
        Button::Maximize if maximized => {
            let front = Rect::from_min_max(g.min + vec2(0.0, 2.5), g.max - vec2(2.5, 0.0));
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Middle);
            p.line(
                vec![
                    g.min + vec2(2.5, 2.5),
                    g.min + vec2(2.5, 0.0),
                    g.right_top(),
                    g.max - vec2(0.0, 2.5),
                    g.max - vec2(2.5, 2.5),
                ],
                s,
            );
        }
        Button::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Middle);
        }
        Button::Close => {
            p.line_segment([g.left_top(), g.right_bottom()], s);
            p.line_segment([g.right_top(), g.left_bottom()], s);
        }
    }
}

/// The band along each edge (pt) and the corner squares that resize the window.
const EDGE: f32 = 5.0;
const CORNER: f32 = 12.0;

/// Resize zones along the window's edges, above everything; a press starts the system's resize.
/// None while the window is maximized or full screen.
pub fn resize_zones(ui: &mut Ui) {
    use ResizeDirection::*;
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let (e, c) = (EDGE, CORNER);
    // Corners last, so they win where they overlap the edges.
    let zones = [
        (
            Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)),
            North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()),
            South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())),
            West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()),
            East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(w.left_top(), vec2(c, c)),
            NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)),
            NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)),
            SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)),
            SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];
    let layer = LayerId::new(Order::Foreground, Id::unique("resize-zones"));
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        let resp = zui
            .interact(rect, Id::unique(("resize", i)), Sense::DRAG)
            .on_hover_cursor(cursor);
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            // The window manager owns the pointer now; egui would otherwise hold this drag until
            // the next press (on X11 and Wayland the release never comes back).
            ctx.stop_dragging();
        }
    }
}

/// For checking the look on another desktop: `RADIOCRAFT_CAPTIONS=windows|adwaita|breeze|generic`
/// draws that family (on any platform, the window's frame off).
pub fn forced_family() -> Option<CaptionFamily> {
    std::env::var("RADIOCRAFT_CAPTIONS")
        .ok()
        .and_then(|v| CaptionFamily::parse(&v))
}
