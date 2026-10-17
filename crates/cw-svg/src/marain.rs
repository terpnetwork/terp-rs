//! Marain-style 3x3 glyph script for the scribe slot (on-chain safe, no alloc-heavy work).
//! Glyph values: community transcription (github.com/willow385/cmarain), 9 cells row-major,
//! bit 8 = top-left. Only "w" is confirmed against Banks' "A Few Notes on Marain".

/// (key, 9-bit mask). Keys: a-z letters, T=th S=sh C=ch G=ng, digits 0-7, ' ', '.', '(' (comma).
pub const GLYPHS: [(&str, u16); 44] = [
    ("w", 0b100111100),
    ("u", 0b100011001),
    ("m", 0b001001111),
    ("h", 0b101101010),
    ("d", 0b110010011),
    ("a", 0b111100010),
    ("p", 0b100101111),
    ("s", 0b011010101),
    ("t", 0b101111001),
    ("i", 0b110011001),
    ("l", 0b110111010),
    ("C", 0b111010100),
    ("k", 0b001111011),
    ("o", 0b011011100),
    ("b", 0b001011111),
    ("H", 0b011101010),
    ("f", 0b111111001),
    ("A", 0b010101111),
    ("v", 0b111101101),
    ("L", 0b101010111),
    ("n", 0b110111110),
    ("E", 0b101110111),
    ("g", 0b010011110),
    ("G", 0b011110001),
    ("z", 0b101111010),
    ("e", 0b110001111),
    ("j", 0b111101011),
    ("S", 0b110110101),
    ("y", 0b111011110),
    ("O", 0b110011101),
    ("r", 0b101111111),
    ("T", 0b111111011),
    (" ", 0b000000000),
    ("(", 0b000100110),
    (")", 0b011001000),
    (".", 0b100100100),
    ("0", 0b111101111),
    ("1", 0b000111110),
    ("2", 0b011110111),
    ("3", 0b111111111),
    ("4", 0b110011111),
    ("5", 0b110110111),
    ("6", 0b100111010),
    ("7", 0b010110111),
];
/// Longest accepted saying, in bytes.
pub const MAX_TEXT_BYTES: usize = 64;
/// Longest accepted saying, in glyphs (bounds the SMIL `values` lists).
pub const MAX_GLYPHS: usize = 40;
/// Seconds per animation step.
pub const STEP_CENTIS: u32 = 45;

fn glyph(key: &str) -> u16 {
    GLYPHS.iter().find(|(k, _)| *k == key).map(|(_, b)| *b).unwrap_or(0)
}

/// Accept only the transliteration charset: `a-z`, `0-7`, space, `.`, `,`, `'`, `-`;
/// single spaces, no leading/trailing space; 1..=MAX_GLYPHS glyphs.
pub fn validate_text(text: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(format!("length must be 1..={MAX_TEXT_BYTES} bytes"));
    }
    if text.starts_with(' ') || text.ends_with(' ') || text.contains("  ") {
        return Err("no leading, trailing or double spaces".into());
    }
    if let Some(c) = text.chars().find(|c| !matches!(c, 'a'..='z' | '0'..='7' | ' ' | '.' | ',' | '\'' | '-')) {
        return Err(format!("character {c:?} is outside the Marain charset"));
    }
    let n = transliterate(text).len();
    if n == 0 || n > MAX_GLYPHS {
        return Err(format!("{n} glyphs; must be 1..={MAX_GLYPHS}"));
    }
    Ok(())
}

/// English -> glyph keys. th/sh/ch/ng digraphs; c -> s before e/i/y else k; q -> k; x -> k s;
/// j -> ch; `'` and `-` are dropped; `,` -> the comma glyph. Assumes `validate_text` charset.
pub fn transliterate(text: &str) -> Vec<&'static str> {
    let s = text.as_bytes();
    let mut out: Vec<&'static str> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let (c, n) = (s[i], s.get(i + 1).copied());
        let di = match (c, n) {
            (b't', Some(b'h')) => Some("T"),
            (b's', Some(b'h')) => Some("S"),
            (b'c', Some(b'h')) => Some("C"),
            (b'n', Some(b'g')) => Some("G"),
            _ => None,
        };
        if let Some(d) = di {
            out.push(d);
            i += 2;
            continue;
        }
        match c {
            b'c' => out.push(if matches!(n, Some(b'e' | b'i' | b'y')) { "s" } else { "k" }),
            b'q' => out.push("k"),
            b'x' => { out.push("k"); out.push("s"); }
            b'j' => out.push("C"),
            b',' => out.push("("),
            b'\'' | b'-' => {}
            _ => {
                if let Some((k, _)) = GLYPHS.iter().find(|(k, _)| k.as_bytes() == [c]) {
                    out.push(k)
                }
            }
        }
        i += 1;
    }
    out
}

/// Derived SMIL values for one text: 9 `values` lists (cells row-major) and `dur` seconds.
/// Each glyph is shown for one step then a blank step; two blank steps before the loop.
pub fn derived(text: &str) -> ([String; 9], String) {
    let keys = transliterate(text);
    let mut frames: Vec<u16> = Vec::with_capacity(keys.len() * 2 + 2);
    for k in &keys {
        frames.push(glyph(k));
        frames.push(0);
    }
    frames.push(0);
    frames.push(0);
    let cells: [String; 9] = core::array::from_fn(|cell| {
        let bit = 1u16 << (8 - cell);
        let mut v = String::with_capacity(frames.len() * 2);
        for (i, f) in frames.iter().enumerate() {
            if i > 0 { v.push(';'); }
            v.push(if f & bit != 0 { '1' } else { '0' });
        }
        v
    });
    let centis = frames.len() as u32 * STEP_CENTIS;
    (cells, format!("{}.{:02}", centis / 100, centis % 100))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn charset_and_limits() {
        validate_text("the mind is a ship").unwrap();
        validate_text("light, then ships.").unwrap();
        for bad in ["", " a", "a ", "a  b", "Ab", "caf\u{e9}", "x<y", "8", "a\"b", "javascript:x"] {
            assert!(validate_text(bad).is_err(), "{bad:?}");
        }
        assert!(validate_text(&"a".repeat(41)).is_err());
        assert!(validate_text(&"ab ".repeat(22)).is_err()); // > 64 bytes
    }
    #[test]
    fn transliteration_and_frames() {
        assert_eq!(transliterate("the ship, quietly.").concat(), "Te Sip( kuietly.");
        let (cells, dur) = derived("w");
        // w = 100111100 -> cell0 on, cell1 off ...; frames: w, blank, blank, blank
        assert_eq!(cells[0], "1;0;0;0");
        assert_eq!(cells[1], "0;0;0;0");
        assert_eq!(dur, "1.80");
        assert_eq!(GLYPHS.len(), 44);
    }
}
