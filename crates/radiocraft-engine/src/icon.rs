//! RadioCraft's logo, drawn from one description as pixels (the window and app icons) and as
//! SVG (the repository's source image): a router sending out rings of signal in the heatmap's
//! colours, past the corner of a floor plan. Where a ring has crossed a wall it is two colours
//! weaker, as in the coverage model itself.
//!
//! Coordinates are a unit square, y down; the tile follows the usual macOS icon grid (an 824/1024
//! rounded square centred in the canvas).

/// The tile: inset, side and corner radius.
const TILE_INSET: f64 = 100.0 / 1024.0;
const TILE_SIDE: f64 = 824.0 / 1024.0;
const TILE_RADIUS: f64 = 185.0 / 1024.0;
/// The tile's colour at its top and its bottom.
const TOP: [u8; 3] = [30, 39, 52];
const BOTTOM: [u8; 3] = [15, 19, 26];

/// The router.
const ROUTER: [f64; 2] = [0.39, 0.62];
const ROUTER_DOT: f64 = 0.048;
const ROUTER_RING: f64 = 0.068;
const ROUTER_FILL: [u8; 3] = [240, 244, 248];
const ROUTER_ACCENT: [u8; 3] = [64, 156, 255];

/// The rings: each band's outer radius, with a gap before the next.
const RING_STEP: f64 = 0.074;
const RING_GAP: f64 = 0.036;
/// How many rings there are.
const BANDS: usize = 5;
/// The heatmap's colours, strongest first (the coverage map's scale).
const RINGS: [[u8; 3]; 6] = [
    [42, 157, 96],
    [124, 191, 82],
    [214, 200, 55],
    [236, 160, 50],
    [226, 104, 52],
    [190, 58, 58],
];

/// The walls (rectangles: x0, y0, x1, y1) and the line each is crossed at.
const WALL_COLOUR: [u8; 3] = [214, 220, 228];
const WALL_V: [f64; 4] = [0.645, 0.215, 0.673, 0.80];
const WALL_H: [f64; 4] = [0.215, 0.215, 0.673, 0.243];

fn in_tile(x: f64, y: f64) -> bool {
    let (lo, hi) = (TILE_INSET, TILE_INSET + TILE_SIDE);
    if !(lo..=hi).contains(&x) || !(lo..=hi).contains(&y) {
        return false;
    }
    let cx = x.clamp(lo + TILE_RADIUS, hi - TILE_RADIUS);
    let cy = y.clamp(lo + TILE_RADIUS, hi - TILE_RADIUS);
    (x - cx).powi(2) + (y - cy).powi(2) <= TILE_RADIUS * TILE_RADIUS
}

fn in_rect(r: [f64; 4], x: f64, y: f64) -> bool {
    (r[0]..=r[2]).contains(&x) && (r[1]..=r[3]).contains(&y)
}

/// How many walls the straight path from the router to (x, y) crosses.
fn walls_crossed(x: f64, y: f64) -> usize {
    let [cx, cy] = ROUTER;
    let mut n = 0;
    // The vertical wall, crossed at its middle line.
    let xv = 0.5 * (WALL_V[0] + WALL_V[2]);
    if x > xv {
        let t = (xv - cx) / (x - cx);
        let yy = cy + t * (y - cy);
        if (WALL_V[1]..=WALL_V[3]).contains(&yy) {
            n += 1;
        }
    }
    let yh = 0.5 * (WALL_H[1] + WALL_H[3]);
    if y < yh {
        let t = (yh - cy) / (y - cy);
        let xx = cx + t * (x - cx);
        if (WALL_H[0]..=WALL_H[2]).contains(&xx) {
            n += 1;
        }
    }
    n
}

/// The colour (straight RGBA, alpha 0–1) at (x, y).
fn sample(x: f64, y: f64) -> [f64; 4] {
    if !in_tile(x, y) {
        return [0.0; 4];
    }
    let mix = |a: [u8; 3], b: [u8; 3], t: f64| -> [f64; 3] {
        [0, 1, 2].map(|i| {
            f64::from(a.get(i).copied().unwrap_or(0)) * (1.0 - t)
                + f64::from(b.get(i).copied().unwrap_or(0)) * t
        })
    };
    let t = ((y - TILE_INSET) / TILE_SIDE).clamp(0.0, 1.0);
    let mut c = mix(TOP, BOTTOM, t);
    let solid = |rgb: [u8; 3]| rgb.map(f64::from);
    let d = ((x - ROUTER[0]).powi(2) + (y - ROUTER[1]).powi(2)).sqrt();
    if in_rect(WALL_V, x, y) || in_rect(WALL_H, x, y) {
        c = solid(WALL_COLOUR);
    } else if d <= ROUTER_DOT {
        c = solid(ROUTER_FILL);
    } else if d <= ROUTER_RING {
        c = solid(ROUTER_ACCENT);
    } else {
        let band = ((d - ROUTER_RING) / RING_STEP).floor();
        let within = d - ROUTER_RING - band * RING_STEP;
        if band >= 0.0 && (band as usize) < BANDS && within > RING_GAP {
            let k = band as usize + 2 * walls_crossed(x, y);
            if let Some(rgb) = RINGS.get(k) {
                c = solid(*rgb);
            }
        }
    }
    [c[0], c[1], c[2], 1.0]
}

/// The logo as straight RGBA8, `size` × `size`, each pixel the average of 4 × 4 samples.
pub fn rgba(size: u32) -> Vec<u8> {
    let n = size.max(1) as usize;
    let mut out = vec![0u8; n * n * 4];
    const S: usize = 4;
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (row, col) = (i / n, i % n);
        let mut acc = [0.0f64; 4];
        for sy in 0..S {
            for sx in 0..S {
                let x = (col as f64 + (sx as f64 + 0.5) / S as f64) / n as f64;
                let y = (row as f64 + (sy as f64 + 0.5) / S as f64) / n as f64;
                let [r, g, b, a] = sample(x, y);
                acc = [acc[0] + r * a, acc[1] + g * a, acc[2] + b * a, acc[3] + a];
            }
        }
        let a = acc[3] / (S * S) as f64;
        let un = |v: f64| {
            if acc[3] > 0.0 {
                (v / acc[3]).round().clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        *px = [
            un(acc[0]),
            un(acc[1]),
            un(acc[2]),
            (a * 255.0).round() as u8,
        ];
    }
    out
}

/// The logo as a PNG file, `size` × `size`.
pub fn png(size: u32) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut e = png::Encoder::new(&mut bytes, size.max(1), size.max(1));
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&rgba(size))
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The logo as SVG (1024 × 1024): the same shapes, with each wall's shadow as a clipped copy of
/// the rings two colours on.
pub fn svg() -> String {
    let u = |v: f64| format!("{:.2}", v * 1024.0);
    let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let [cx, cy] = ROUTER;
    // A wall's shadow: the router's view of the wall's crossing line, pushed far out.
    let shadow = |a: [f64; 2], b: [f64; 2]| {
        let far = |p: [f64; 2]| [cx + (p[0] - cx) * 40.0, cy + (p[1] - cy) * 40.0];
        let (fa, fb) = (far(a), far(b));
        format!(
            "M{} {}L{} {}L{} {}L{} {}Z",
            u(a[0]),
            u(a[1]),
            u(b[0]),
            u(b[1]),
            u(fb[0]),
            u(fb[1]),
            u(fa[0]),
            u(fa[1])
        )
    };
    let xv = 0.5 * (WALL_V[0] + WALL_V[2]);
    let yh = 0.5 * (WALL_H[1] + WALL_H[3]);
    let shadow_v = shadow([xv, WALL_V[1]], [xv, WALL_V[3]]);
    let shadow_h = shadow([WALL_H[0], yh], [WALL_H[2], yh]);
    // The rings from colour `shift` on; past the last colour, the tile shows through.
    let rings = |shift: usize| {
        let mut s = String::new();
        for band in 0..BANDS {
            let r_out = ROUTER_RING + (band as f64 + 1.0) * RING_STEP;
            let r_in = ROUTER_RING + band as f64 * RING_STEP + RING_GAP;
            let colour = RINGS.get(band + shift).map(|c| hex(*c));
            // A band is a thick circle outline; bands without a colour hide the ones below.
            let stroke = colour.clone().unwrap_or_else(|| "url(#bg)".to_string());
            s += &format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\"/>",
                u(cx),
                u(cy),
                u(0.5 * (r_in + r_out)),
                u(r_out - r_in)
            );
            if colour.is_none() {
                // The gap beside a hidden band would leave a stripe of the colour below.
                s += &format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"url(#bg)\" stroke-width=\"{}\"/>",
                    u(cx),
                    u(cy),
                    u(r_in - 0.5 * RING_GAP),
                    u(RING_GAP + 0.002)
                );
            }
        }
        s
    };
    let lo = TILE_INSET;
    format!(
        concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\" width=\"1024\" height=\"1024\">",
            "<title>RadioCraft</title>",
            "<defs>",
            "<linearGradient id=\"bg\" x1=\"0\" y1=\"{lo}\" x2=\"0\" y2=\"{hi}\" gradientUnits=\"userSpaceOnUse\">",
            "<stop offset=\"0\" stop-color=\"{top}\"/><stop offset=\"1\" stop-color=\"{bottom}\"/></linearGradient>",
            "<clipPath id=\"tile\"><rect x=\"{lo}\" y=\"{lo}\" width=\"{side}\" height=\"{side}\" rx=\"{r}\"/></clipPath>",
            "<clipPath id=\"sv\"><path d=\"{sv}\"/></clipPath>",
            "<clipPath id=\"sh\"><path d=\"{sh}\"/></clipPath>",
            "</defs>",
            "<g clip-path=\"url(#tile)\">",
            "<rect width=\"1024\" height=\"1024\" fill=\"url(#bg)\"/>",
            "{rings0}",
            "<g clip-path=\"url(#sv)\"><rect width=\"1024\" height=\"1024\" fill=\"url(#bg)\"/>{rings2}</g>",
            "<g clip-path=\"url(#sh)\"><rect width=\"1024\" height=\"1024\" fill=\"url(#bg)\"/>{rings2}",
            "<g clip-path=\"url(#sv)\"><rect width=\"1024\" height=\"1024\" fill=\"url(#bg)\"/>{rings4}</g></g>",
            "<rect x=\"{wvx}\" y=\"{wvy}\" width=\"{wvw}\" height=\"{wvh}\" fill=\"{wall}\"/>",
            "<rect x=\"{whx}\" y=\"{why}\" width=\"{whw}\" height=\"{whh}\" fill=\"{wall}\"/>",
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{ring}\" fill=\"{accent}\"/>",
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{dot}\" fill=\"{fill}\"/>",
            "</g></svg>\n"
        ),
        lo = u(lo),
        hi = u(lo + TILE_SIDE),
        side = u(TILE_SIDE),
        r = u(TILE_RADIUS),
        top = hex(TOP),
        bottom = hex(BOTTOM),
        sv = shadow_v,
        sh = shadow_h,
        rings0 = rings(0),
        rings2 = rings(2),
        rings4 = rings(4),
        wvx = u(WALL_V[0]),
        wvy = u(WALL_V[1]),
        wvw = u(WALL_V[2] - WALL_V[0]),
        wvh = u(WALL_V[3] - WALL_V[1]),
        whx = u(WALL_H[0]),
        why = u(WALL_H[1]),
        whw = u(WALL_H[2] - WALL_H[0]),
        whh = u(WALL_H[3] - WALL_H[1]),
        wall = hex(WALL_COLOUR),
        cx = u(cx),
        cy = u(cy),
        ring = u(ROUTER_RING),
        accent = hex(ROUTER_ACCENT),
        dot = u(ROUTER_DOT),
        fill = hex(ROUTER_FILL),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_draws() {
        let px = rgba(64);
        assert_eq!(px.len(), 64 * 64 * 4);
        // Corners are clear (outside the rounded tile), the middle is solid.
        assert_eq!(px.get(3), Some(&0));
        assert_eq!(px.get((32 * 64 + 32) * 4 + 3), Some(&255));
        // Past one wall a ring is two colours on: behind the vertical wall the colour differs
        // from the same distance on the open side.
        let open = sample(ROUTER[0] - 0.30, ROUTER[1]);
        let behind = sample(ROUTER[0] + 0.30, ROUTER[1]);
        assert_ne!(open, behind);
        assert_eq!(walls_crossed(ROUTER[0] + 0.30, ROUTER[1]), 1);
        assert_eq!(walls_crossed(0.75, 0.084), 2);
        assert!(svg().starts_with("<svg") && png(16).is_ok());
    }
}
