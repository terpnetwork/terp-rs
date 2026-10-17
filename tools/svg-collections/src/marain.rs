//! Marain orb v2 (scribe slot): the 3x3 core of the terp orb shield flashes the token's
//! `scribe` text in Marain glyphs. At mint the text is a theme picked from the curated
//! list; an allowlisted scribe may replace it once (cw721-svg `Scribe {}`), and the
//! contract derives the SMIL values from the text at render time. Nothing is prepacked.
use cw_svg::{RgbRange, VariableDef, VariableKind};

pub use cw_svg::marain::{derived, transliterate, validate_text, GLYPHS, MAX_GLYPHS, MAX_TEXT_BYTES};
pub const GLYPHS_JSON: &str = include_str!("../data/marain-glyphs.json");
pub const THEMES_JSON: &str = include_str!("../data/marain-themes.json");
/// Core-cell centres, row-major from top-left (matches bit order).
pub const CELLS: [(i32, i32); 9] = [(-25, -25), (0, -25), (25, -25), (-25, 0), (0, 0), (25, 0), (-25, 25), (0, 25), (25, 25)];

pub fn themes() -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(THEMES_JSON).unwrap();
    v["themes"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()
}

fn rr(r: (u8, u8), g: (u8, u8), b: (u8, u8)) -> RgbRange {
    RgbRange { r_min: r.0, r_max: r.1, g_min: g.0, g_max: g.1, b_min: b.0, b_max: b.1 }
}
fn range(n: &str, a: &str, b: &str, p: u32) -> VariableDef {
    VariableDef { name: n.into(), kind: VariableKind::Range { min: a.into(), max: b.into(), precision: p } }
}
fn placeholder(n: String) -> VariableDef {
    VariableDef { name: n, kind: VariableKind::Options(vec![String::new()]) }
}

pub fn variables(themes: &[String]) -> Vec<VariableDef> {
    let mut v = vec![VariableDef { name: "scribe".into(), kind: VariableKind::Options(themes.to_vec()) }];
    for c in 0..9 {
        v.push(placeholder(format!("scribe.c{c}")));
    }
    v.push(placeholder("scribe.dur".into()));
    v.extend([
        VariableDef { name: "color-core".into(), kind: VariableKind::RgbStyled(vec![rr((250, 255), (235, 250), (190, 220))]) },
        VariableDef { name: "color-rim".into(), kind: VariableKind::RgbStyled(vec![rr((220, 255), (150, 190), (60, 110)), rr((200, 240), (120, 160), (40, 80))]) },
        VariableDef { name: "color-halo".into(), kind: VariableKind::RgbStyled(vec![rr((255, 255), (170, 210), (90, 130))]) },
        VariableDef { name: "color-bg".into(), kind: VariableKind::RgbStyled(vec![rr((16, 28), (10, 18), (4, 10))]) },
        range("opacity-glow", "0.40", "0.90", 2),
        range("blur-glow", "2.5", "6.0", 1),
        range("pulse-glow", "1.60", "3.20", 2),
        range("scale-shield", "0.88", "0.98", 2),
    ]);
    v
}

const DISC: &str = "r=\"12.6\" fill=\"url(#disc-fill)\"";

/// Shield template with one animated 3x3 core. No rotation variable: rotated/mirrored
/// Marain glyphs are different letters.
pub fn template() -> String {
    let mut t = String::new();
    t.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-72 -72 144 144\" width=\"144\" height=\"144\">\n");
    t.push_str("<style>.c{font:4.2px monospace;fill:${color-core};text-anchor:middle;opacity:.7}</style>\n");
    t.push_str("<defs><radialGradient id=\"disc-fill\"><stop offset=\"0\" stop-color=\"${color-core}\"/><stop offset=\"0.36\" stop-color=\"${color-rim}\"/><stop offset=\"0.69\" stop-color=\"${color-rim}\"/><stop offset=\"0.74\" stop-color=\"${color-halo}\" stop-opacity=\"0.3\"/><stop offset=\"0.86\" stop-color=\"${color-halo}\" stop-opacity=\"0.09\"/><stop offset=\"1\" stop-color=\"${color-halo}\" stop-opacity=\"0\"/></radialGradient>\n");
    t.push_str("<filter id=\"glow\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"><feGaussianBlur stdDeviation=\"${blur-glow}\"/></filter></defs>\n");
    t.push_str("<rect x=\"-72\" y=\"-72\" width=\"144\" height=\"144\" fill=\"${color-bg}\"/>\n<g transform=\"scale(${scale-shield})\">\n");
    let outer = |t: &mut String| {
        for (x, y) in [(0, -48), (48, 0), (0, 48), (-48, 0)] {
            t.push_str(&format!("<circle cx=\"{x}\" cy=\"{y}\" {DISC}/>\n"));
        }
    };
    t.push_str("<g filter=\"url(#glow)\" opacity=\"${opacity-glow}\"><animate attributeName=\"opacity\" values=\"${opacity-glow};1;${opacity-glow}\" dur=\"${pulse-glow}s\" repeatCount=\"indefinite\"/>\n");
    outer(&mut t);
    t.push_str("</g>\n");
    outer(&mut t);
    t.push_str("<g opacity=\".12\">\n");
    for (x, y) in CELLS {
        t.push_str(&format!("<circle cx=\"{x}\" cy=\"{y}\" {DISC}/>\n"));
    }
    t.push_str("</g>\n");
    for (c, (x, y)) in CELLS.iter().enumerate() {
        t.push_str(&format!(
            "<circle cx=\"{x}\" cy=\"{y}\" {DISC} opacity=\"0\"><animate attributeName=\"opacity\" values=\"${{scribe.c{c}}}\" dur=\"${{scribe.dur}}s\" calcMode=\"discrete\" repeatCount=\"indefinite\"/></circle>\n"
        ));
    }
    t.push_str("<text class=\"c\" y=\"68\">${scribe}</text>\n</g>\n</svg>");
    t
}
