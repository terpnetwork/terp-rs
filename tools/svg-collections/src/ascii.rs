//! Symbient ASCII frames: deterministic sanitizer + SVP1 frame template.
use cw_svg::{RgbRange, VariableDef, VariableKind};

pub const COLS: usize = 48;
pub const ROWS: usize = 24;
/// ~17 non-space cells; below this the frame is rejected and the harness re-rolls.
pub const MIN_INK_PERMILLE: u32 = 15;
const UNITS: [&str; 14] = ["mcg", "ug", "mg", "g", "ml", "ul", "iu", "kg", "tabs", "tab", "hits", "drops", "pills", "grams"];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Sanitized {
    pub rows: Vec<String>,
    pub replaced_chars: usize,
    pub masked_dose_tokens: usize,
    pub ink_permille: u32,
}

/// Mask "<number>[ ]<unit>" with '~' so model output can never carry a dose.
fn mask_doses(line: &str) -> (String, usize) {
    let b: Vec<u8> = line.bytes().collect();
    let mut out = b.clone();
    let mut n = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let s = i;
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.' || b[j] == b',') { j += 1; }
            let mut k = j;
            while k < b.len() && b[k] == b' ' && k - j < 2 { k += 1; }
            let rest = &line[k..].to_ascii_lowercase();
            if let Some(u) = UNITS.iter().find(|u| {
                rest.starts_with(*u) && !rest.as_bytes().get(u.len()).map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
            }) {
                for x in out.iter_mut().take(k + u.len()).skip(s) { *x = b'~'; }
                n += 1;
                i = k + u.len();
                continue;
            }
            i = j;
        } else { i += 1; }
    }
    (String::from_utf8(out).unwrap(), n)
}

/// Deterministic: same bytes in -> same grid out. Rejects (Err) near-empty or near-solid output.
pub fn sanitize(raw: &str) -> Result<Sanitized, String> {
    let mut replaced = 0;
    let mut lines: Vec<String> = Vec::new();
    for l in raw.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if l.trim_start().starts_with("```") { continue; }
        let mut s = String::new();
        for ch in l.chars() {
            match ch {
                '\t' => { s.push(' '); while s.len() % 4 != 0 { s.push(' ') } }
                ' '..='~' => s.push(ch),
                _ => { s.push('?'); replaced += 1; }
            }
        }
        lines.push(s.trim_end().to_string());
    }
    while lines.first().map(|l| l.is_empty()).unwrap_or(false) { lines.remove(0); }
    let mut masked = 0;
    let mut rows: Vec<String> = lines.into_iter().take(ROWS).map(|l| {
        let (m, n) = mask_doses(&l);
        masked += n;
        let mut r: String = m.chars().take(COLS).collect();
        while r.len() < COLS { r.push(' '); }
        r
    }).collect();
    while rows.len() < ROWS { rows.push(" ".repeat(COLS)); }
    let ink = rows.iter().flat_map(|r| r.bytes()).filter(|c| *c != b' ').count();
    let ink_permille = (ink * 1000 / (COLS * ROWS)) as u32;
    if ink_permille < MIN_INK_PERMILLE { return Err(format!("too empty ({ink_permille}‰)")); }
    if ink_permille > 950 { return Err(format!("too solid ({ink_permille}‰)")); }
    Ok(Sanitized { rows, replaced_chars: replaced, masked_dose_tokens: masked, ink_permille })
}

pub const RICH_MAX_ROWS: usize = 400;
pub const RICH_MAX_COLS: usize = 200;

/// Rich (off-chain) text: same char filter and dose masking, no 48x24 clip
/// (capped at 400x200 so the HTML stays one IPFS block). Returns (text, masked, replaced).
pub fn sanitize_rich(raw: &str) -> (String, usize, usize) {
    let (mut masked, mut replaced) = (0, 0);
    let mut out: Vec<String> = Vec::new();
    for l in raw.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if l.trim_start().starts_with("```") { continue; }
        let mut s = String::new();
        for ch in l.chars() {
            match ch {
                '\t' => { s.push(' '); while s.len() % 4 != 0 { s.push(' ') } }
                ' '..='~' => s.push(ch),
                _ => { s.push('?'); replaced += 1; }
            }
        }
        let (m, n) = mask_doses(s.trim_end());
        masked += n;
        out.push(m.chars().take(RICH_MAX_COLS).collect());
        if out.len() == RICH_MAX_ROWS { break; }
    }
    while out.first().map(|l| l.is_empty()).unwrap_or(false) { out.remove(0); }
    while out.last().map(|l| l.is_empty()).unwrap_or(false) { out.pop(); }
    (out.join("\n"), masked, replaced)
}

pub const W: u32 = 304;
pub const H: u32 = 344;

/// Gateways a token may link through (must stay within cw_svg::links::GATEWAY_ALLOWLIST).
pub const GATEWAYS: [&str; 2] = ["dweb.link", "ipfs.io"];

/// `frame` options are the rich-render CIDs, so the token param records its CID.
pub fn variables(cids: &[String]) -> Vec<VariableDef> {
    let rr = |r: (u8, u8), g: (u8, u8), b: (u8, u8)| RgbRange { r_min: r.0, r_max: r.1, g_min: g.0, g_max: g.1, b_min: b.0, b_max: b.1 };
    vec![
        VariableDef { name: "frame".into(), kind: VariableKind::Options(cids.to_vec()) },
        VariableDef { name: "gateway".into(), kind: VariableKind::Options(GATEWAYS.iter().map(|s| s.to_string()).collect()) },
        VariableDef { name: "color-ink".into(), kind: VariableKind::RgbStyled(vec![rr((150, 210), (230, 255), (150, 200)), rr((230, 255), (190, 230), (120, 170)), rr((170, 220), (190, 230), (240, 255))]) },
        VariableDef { name: "color-bg".into(), kind: VariableKind::RgbStyled(vec![rr((6, 16), (6, 16), (10, 22))]) },
    ]
}

/// `flicker`: day-level motif chosen by the harness from interaction severity
/// (opacity-only SMIL, so validate_animated_paint passes). `cids[i]` = rich render of frame i.
pub fn template(frames: &[Sanitized], cids: &[String], caption: &str, flicker: Option<&str>) -> String {
    assert_eq!(frames.len(), cids.len());
    let mut t = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {W} {H}\" width=\"{W}\" height=\"{H}\">\n");
    t.push_str("<style>.f{display:none}#f${frame}{display:inline}text{font:10px monospace;white-space:pre;fill:${color-ink}}.m{font-size:7px;opacity:.75}a text{text-decoration:underline}</style>\n");
    t.push_str(&format!("<rect x=\"0\" y=\"0\" width=\"{W}\" height=\"{H}\" fill=\"${{color-bg}}\"/>\n"));
    for (f, cid) in frames.iter().zip(cids) {
        t.push_str(&format!("<g id=\"f{cid}\" class=\"f\">\n"));
        if let Some(v) = flicker {
            t.push_str(&format!("<animate attributeName=\"opacity\" values=\"{v}\" dur=\"2.4s\" repeatCount=\"indefinite\"/>\n"));
        }
        for (r, row) in f.rows.iter().enumerate() {
            t.push_str(&format!("<text x=\"8\" y=\"{}\">{}</text>\n", 20 + r * 12, crate::common::xml_escape(row.trim_end())));
        }
        t.push_str("</g>\n");
    }
    t.push_str(&format!("<text class=\"m\" x=\"8\" y=\"312\">{}</text>\n", crate::common::xml_escape(caption)));
    t.push_str("<a href=\"https://${gateway}/ipfs/${frame}\"><text class=\"m\" x=\"8\" y=\"324\">full render: ipfs://${frame}</text></a>\n");
    t.push_str("<text class=\"m\" x=\"8\" y=\"336\">Latent Benders: A Study - data: psychedex.org</text>\n</svg>");
    t
}
