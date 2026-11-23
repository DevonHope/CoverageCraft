//! Colours and sizes, in one place: panels and the plan view take them from here, so a theme
//! changes everything at once.

use egui::Color32;

pub struct Tokens {
    pub outside: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub router_fill: Color32,
    pub selection: Color32,
    pub error: Color32,
    /// The pointer probe's background.
    pub tooltip: Color32,
    /// The app bar's background, and the open plan's tab on it.
    pub bar: Color32,
    pub tab: Color32,
    /// Window buttons (Windows, Linux): Close on hover, the others on hover, the round ones at rest.
    pub caption_close: Color32,
    pub caption_hover: Color32,
    pub caption_rest: Color32,
    /// Heatmap opacity over the canvas (0–255).
    pub heatmap_alpha: u8,
    pub router_radius: f32,
}

pub const TOKENS: Tokens = Tokens {
    outside: Color32::from_rgb(22, 25, 30),
    text: Color32::from_rgb(226, 232, 238),
    text_dim: Color32::from_rgb(150, 160, 170),
    accent: Color32::from_rgb(64, 156, 255),
    router_fill: Color32::from_rgb(240, 244, 248),
    selection: Color32::from_rgb(255, 196, 64),
    error: Color32::from_rgb(255, 110, 100),
    tooltip: Color32::from_rgba_premultiplied(16, 18, 22, 230),
    bar: Color32::from_rgb(18, 21, 26),
    tab: Color32::from_rgb(40, 46, 56),
    caption_close: Color32::from_rgb(196, 43, 28),
    caption_hover: Color32::from_rgb(52, 58, 68),
    caption_rest: Color32::from_rgb(36, 41, 49),
    heatmap_alpha: 220,
    router_radius: 8.0,
};

/// A wall outline's colour by its material (the same roles the PNG uses).
pub fn wall_colour(preset: Option<&str>, guessed: bool) -> Color32 {
    let [r, g, b, _] = coveragecraft_engine::picture::outline_colour(preset, guessed);
    // The PNG's near-black concrete is lifted so it reads on the dark canvas.
    if guessed || preset.is_none() || preset == Some("concrete") {
        return Color32::from_rgb(200, 205, 212);
    }
    Color32::from_rgb(r, g, b)
}
