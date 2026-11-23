//! The heatmap as a picture seen from above: north up, each wall outlined in its material's
//! colour (so a pane thinner than a cell shows), routers marked.

use crate::engine::Coverage;
use crate::materials::rf_preset_of;
use crate::model::Emitter;

/// The colours of the picture (sRGB): data colours for a file, not the window's theme.
const BACKGROUND: [u8; 3] = [255, 255, 255];
const ROUTER_RING: [u8; 4] = [11, 107, 203, 255];
const ROUTER_DOT: [u8; 4] = [255, 255, 255, 255];

/// A wall outline's colour, by its material: glazing blue, metal red, wood brown, drywall grey,
/// concrete and anything else near black.
pub fn outline_colour(preset: Option<&str>, guessed: bool) -> [u8; 4] {
    if guessed {
        return [60, 66, 74, 255];
    }
    match preset {
        Some("low-e-glass") => [40, 120, 210, 255],
        Some("interior-glass") => [120, 180, 230, 255],
        Some("metal") => [200, 60, 60, 255],
        Some("wood-door") => [160, 110, 60, 255],
        Some("drywall") => [130, 136, 144, 255],
        _ => [30, 34, 40, 255],
    }
}

/// The PNG's bytes.
pub fn png(c: &Coverage, emitters: &[Emitter]) -> Result<Vec<u8>, String> {
    let grid = &c.grid;
    let scale = (1600 / grid.columns.max(grid.rows).max(1)).clamp(1, 8);
    let (w, h) = (grid.columns * scale, grid.rows * scale);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        // The image's first row is the grid's highest (north up).
        let row = grid.rows - 1 - y / scale;
        for x in 0..w {
            let cell = row * grid.columns + x / scale;
            let src = c.rgba.get(cell * 4..cell * 4 + 4).unwrap_or(&[0, 0, 0, 0]);
            let a = f64::from(src.get(3).copied().unwrap_or(0)) / 255.0;
            let blend = |i: usize| {
                (f64::from(src.get(i).copied().unwrap_or(0)) * a
                    + f64::from(BACKGROUND.get(i).copied().unwrap_or(255)) * (1.0 - a))
                    .round() as u8
            };
            if let Some(px) = out.get_mut((y * w + x) * 4..(y * w + x) * 4 + 4) {
                px.copy_from_slice(&[blend(0), blend(1), blend(2), 255]);
            }
        }
    }
    let to_px = |p: [f64; 2]| {
        [
            (p[0] - grid.origin[0]) / grid.step * scale as f64,
            h as f64 - (p[1] - grid.origin[1]) / grid.step * scale as f64,
        ]
    };
    let mut put = |x: f64, y: f64, colour: [u8; 4]| {
        if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
            return;
        }
        let at = (y as usize * w + x as usize) * 4;
        if let Some(dst) = out.get_mut(at..at + 4) {
            dst.copy_from_slice(&colour);
        }
    };
    for r in &c.walls.regions {
        let preset = c
            .walls
            .materials
            .get(r.material)
            .and_then(|(_, l)| rf_preset_of(l))
            .map(|p| p.id);
        let colour = outline_colour(preset, r.thick.is_some());
        for ring in &r.rings {
            for (i, a) in ring.iter().enumerate() {
                let Some(b) = ring.get((i + 1) % ring.len()) else {
                    continue;
                };
                let (pa, pb) = (to_px(*a), to_px(*b));
                let n = ((pb[0] - pa[0]).abs().max((pb[1] - pa[1]).abs()) * 2.0)
                    .ceil()
                    .clamp(1.0, 1.0e5) as usize;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let (x, y) = (pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t);
                    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                        put((x - 0.5 + dx).floor(), (y - 0.5 + dy).floor(), colour);
                    }
                }
            }
        }
    }
    let radius = (scale * 3).max(6) as f64;
    for e in emitters {
        let [cx, cy] = to_px(e.at);
        let (x0, x1) = (
            (cx - radius).floor().max(0.0) as usize,
            ((cx + radius).ceil().max(0.0) as usize).min(w),
        );
        let (y0, y1) = (
            (cy - radius).floor().max(0.0) as usize,
            ((cy + radius).ceil().max(0.0) as usize).min(h),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
                if d <= radius * 0.6 {
                    put(x as f64, y as f64, ROUTER_DOT);
                } else if d <= radius {
                    put(x as f64, y as f64, ROUTER_RING);
                }
            }
        }
    }
    let (w32, h32) = (
        u32::try_from(w).map_err(|_| "picture too large".to_string())?,
        u32::try_from(h).map_err(|_| "picture too large".to_string())?,
    );
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, w32, h32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut writer = e.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&out).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}
