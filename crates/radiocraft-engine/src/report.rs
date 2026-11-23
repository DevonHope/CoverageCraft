//! The coverage report: one self-contained HTML page (maps embedded as PNG), readable in any
//! browser and printable to PDF from there.

use crate::views::LegendRow;

/// One map in the report.
pub struct Map {
    pub title: String,
    pub png: Vec<u8>,
    pub key_note: String,
    pub legend: Vec<LegendRow>,
}

/// One band's part of the report.
pub struct BandSection {
    pub band: String,
    /// Label and value, in order.
    pub figures: Vec<(String, String)>,
    pub maps: Vec<Map>,
}

/// Everything the report shows.
pub struct Report {
    pub title: String,
    pub notes: Option<String>,
    /// The plan's file name (not its folder).
    pub plan: String,
    /// Rows of name, position, height, device, power, channels.
    pub routers: Vec<[String; 6]>,
    /// Rows of layer or object, kind, material.
    pub materials: Vec<[String; 3]>,
    /// Label and value.
    pub settings: Vec<(String, String)>,
    pub bands: Vec<BandSection>,
}

/// `bytes` as standard base64.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(
                    A.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A'),
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Text made safe to put in HTML.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn table(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut s = String::from("<table><thead><tr>");
    for h in head {
        s += &format!("<th>{}</th>", escape(h));
    }
    s += "</tr></thead><tbody>";
    for r in rows {
        s += "<tr>";
        for c in r {
            s += &format!("<td>{}</td>", escape(c));
        }
        s += "</tr>";
    }
    s + "</tbody></table>"
}

/// The report as an HTML page.
pub fn html(r: &Report) -> String {
    let mut s = String::new();
    s += "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">";
    s += "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">";
    s += &format!("<title>{}</title>", escape(&r.title));
    s += "<style>
body{font:15px/1.45 system-ui,sans-serif;margin:0 auto;max-width:960px;padding:24px 16px;color:#1d2228;background:#fff}
h1{font-size:26px;margin:0 0 4px}h2{font-size:20px;margin:32px 0 8px;border-bottom:1px solid #d5dae0;padding-bottom:4px}
h3{font-size:16px;margin:20px 0 6px}.dim{color:#5d6670}
table{border-collapse:collapse;margin:6px 0 12px;width:100%}th,td{text-align:left;padding:4px 8px;border-bottom:1px solid #e4e8ec;vertical-align:top}
th{font-weight:600;background:#f4f6f8}td.n{font-variant-numeric:tabular-nums}
figure{margin:12px 0 24px;break-inside:avoid}figure img{max-width:100%;border:1px solid #d5dae0;image-rendering:pixelated}
.key{display:flex;flex-wrap:wrap;gap:4px 16px;margin-top:6px;font-size:13px}.sw{display:inline-block;width:12px;height:12px;border-radius:2px;margin-right:6px;vertical-align:-1px}
.notes{white-space:pre-wrap;background:#f4f6f8;padding:8px 12px;border-radius:4px}
@media print{body{max-width:none}h2{break-before:page}}
</style></head><body>";
    s += &format!("<h1>{}</h1>", escape(&r.title));
    s += &format!(
        "<p class=\"dim\">Plan: {} · made with RadioCraft</p>",
        escape(&r.plan)
    );
    if let Some(n) = &r.notes {
        s += &format!("<div class=\"notes\">{}</div>", escape(n));
    }
    s += "<h2>Routers</h2>";
    s += &table(
        &[
            "Name",
            "Position (m)",
            "Height (m)",
            "Device",
            "Power (EIRP)",
            "Channels",
        ],
        &r.routers.iter().map(|row| row.to_vec()).collect::<Vec<_>>(),
    );
    for b in &r.bands {
        s += &format!("<h2>{} GHz</h2>", escape(&b.band));
        s += &table(
            &["Figure", "Value"],
            &b.figures
                .iter()
                .map(|(k, v)| vec![k.clone(), v.clone()])
                .collect::<Vec<_>>(),
        );
        for m in &b.maps {
            s += &format!(
                "<figure><h3>{}</h3><img alt=\"{}\" src=\"data:image/png;base64,{}\"><figcaption><span class=\"dim\">{}</span><div class=\"key\">",
                escape(&m.title),
                escape(&m.title),
                base64(&m.png),
                escape(&m.key_note)
            );
            for row in &m.legend {
                let [cr, cg, cb] = row.colour;
                s += &format!(
                    "<span><span class=\"sw\" style=\"background:rgb({cr},{cg},{cb})\"></span>{} {:.0}%</span>",
                    escape(&row.label),
                    row.share
                );
            }
            s += "</div></figcaption></figure>";
        }
    }
    s += "<h2>Walls</h2>";
    s += &table(
        &["Layer or object", "Kind", "Material"],
        &r.materials
            .iter()
            .map(|row| row.to_vec())
            .collect::<Vec<_>>(),
    );
    s += "<h2>Settings</h2>";
    s += &table(
        &["Setting", "Value"],
        &r.settings
            .iter()
            .map(|(k, v)| vec![k.clone(), v.clone()])
            .collect::<Vec<_>>(),
    );
    s += "<h2>What the numbers mean</h2><p>The signal is predicted from the routers' power, the distance and the walls on the way (each crossing counted once, by its material), with a path-loss exponent for the open space between. Reflections, furniture, people, interference and the client's own radio aren't modelled, so real signal is often a few dB either side. The link rate is the most a signal allows with nothing else on the air, from the 802.11 receiver sensitivity table. Use the maps to compare placements, then check on site.</p>";
    s += "</body></html>\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_escaping() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(
            escape("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }
}
