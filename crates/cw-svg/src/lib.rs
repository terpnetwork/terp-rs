use cosmwasm_schema::cw_serde;

pub mod callback;
pub mod dao;
pub mod links;
pub mod marain;
pub use callback::{AUTH_MSG_MAX_EXTRA, MintMsg, SvgMintCallbackAction};

/// Domain-separation tag prefix for ADR-36 mint auth messages.
/// Message formats: `{DST}{nonce}` or `{DST}{nonce}|{digest_hex}` (see AUTH_MSG_MAX_EXTRA).
pub const CW721SVG_DST: &str = "DST_cw721svg_morocco-1";

#[cosmwasm_schema::cw_serde]
pub struct RgbRange {
    pub r_min: u8,
    pub r_max: u8,
    pub g_min: u8,
    pub g_max: u8,
    pub b_min: u8,
    pub b_max: u8,
}

#[cw_serde]
pub enum VariableKind {
    /// Pick from a list of string values
    Options(Vec<String>),
    /// Generate a decimal string in [min, max] at given precision.
    /// `min` and `max` are decimal strings (e.g. "-2.5", "80").
    Range {
        min: String,
        max: String,
        precision: u32,
    },
    /// Generate a random `rgb(R,G,B)` color string.
    /// Each channel is independently random in 0–255.
    Rgb,
    /// Pick a random range from the list, then generate a random shade within it.
    /// Each channel is constrained to [min, max] of the selected range.
    RgbStyled(Vec<RgbRange>),
}

#[cw_serde]
pub struct VariableDef {
    pub name: String,
    pub kind: VariableKind,
}

#[cw_serde]
pub struct TokenParam {
    pub name: String,
    pub value: String,
}

#[cw_serde]
pub struct TemplateSlot {
    pub start: u32,
    pub end: u32,
    pub var_idx: u16,
}

/// Compute template slots by scanning a template for `${varname}` placeholders.
/// This is the off-chain counterpart to the on-chain `validate_template_slots`.
pub fn compute_template_slots(template: &str, variables: &[VariableDef]) -> Vec<TemplateSlot> {
    let bytes = template.as_bytes();
    let len = bytes.len();
    let mut slots = Vec::new();
    let mut i = 0;

    while i < len.saturating_sub(1) {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i;
            let name_start = i + 2;
            let mut j = name_start;
            while j < len && bytes[j] != b'}' {
                j += 1;
            }
            if j >= len {
                panic!("unclosed placeholder at byte offset {}", start);
            }
            let name = &template[name_start..j];
            let end = j + 1;
            let var_idx = variables
                .iter()
                .position(|v| v.name == name)
                .unwrap_or_else(|| panic!("undefined variable '{}' in template", name));
            slots.push(TemplateSlot {
                start: start as u32,
                end: end as u32,
                var_idx: var_idx as u16,
            });
            i = end;
        } else {
            i += 1;
        }
    }

    slots
}

/// Parse a decimal string (e.g. "-2.5", "80", "0.45") into a scaled integer
/// at the given precision. For example: "-2.5" with precision=2 → -250.
pub fn parse_decimal_scaled(s: &str, precision: u32) -> Result<i128, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty string".to_string());
    }

    let (negative, abs_str) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };

    let parts: Vec<&str> = abs_str.split('.').collect();
    if parts.len() > 2 || parts.is_empty() || parts[0].is_empty() {
        return Err(format!("invalid decimal: {}", s));
    }

    let int_part: i128 = parts[0]
        .parse()
        .map_err(|_| format!("invalid integer part: {}", s))?;

    let frac_digits = if parts.len() == 2 { parts[1] } else { "" };

    let ovf = || format!("decimal out of range: {}", s);
    let factor = 10i128.checked_pow(precision).ok_or_else(ovf)?;
    let mut scaled = int_part.checked_mul(factor).ok_or_else(ovf)?;

    if !frac_digits.is_empty() {
        let frac_len = frac_digits.len() as u32;
        let frac_val: i128 = frac_digits
            .parse()
            .map_err(|_| format!("invalid fractional part: {}", s))?;

        let add = if frac_len <= precision {
            frac_val
                .checked_mul(10i128.checked_pow(precision - frac_len).ok_or_else(ovf)?)
                .ok_or_else(ovf)?
        } else {
            // More digits than precision — truncate
            match 10i128.checked_pow(frac_len - precision) {
                Some(d) => frac_val / d,
                None => 0,
            }
        };
        scaled = scaled.checked_add(add).ok_or_else(ovf)?;
    }

    if negative {
        scaled = -scaled;
    }

    Ok(scaled)
}

/// Format a scaled integer back into a decimal string.
/// For example: -250 with precision=2 → "-2.50".
pub fn format_decimal(scaled: i128, precision: u32) -> String {
    if precision == 0 {
        return scaled.to_string();
    }

    let factor = 10i128.pow(precision);
    let (negative, abs_val) = if scaled < 0 {
        (true, -scaled)
    } else {
        (false, scaled)
    };

    let int_part = abs_val / factor;
    let frac_part = abs_val % factor;

    let sign = if negative { "-" } else { "" };
    format!(
        "{}{}.{:0>width$}",
        sign,
        int_part,
        frac_part,
        width = precision as usize
    )
}

/// Validate all variable definitions in isolation (options non-empty, range
/// parseable / ordered, rgb_styled ranges non-empty with min <= max).
/// This is the same check that runs inside `instantiate`.
pub fn validate_variables(variables: &[VariableDef]) -> Result<(), SvgError> {
    for var in variables {
        match &var.kind {
            VariableKind::Options(opts) => {
                if opts.is_empty() {
                    return Err(SvgError::InvalidVariableDef {
                        reason: format!("variable '{}': options list must not be empty", var.name),
                    });
                }
            }
            VariableKind::Range {
                min,
                max,
                precision,
            } => {
                if *precision > 18 {
                    return Err(SvgError::InvalidVariableDef {
                        reason: format!(
                            "variable '{}': precision {} exceeds maximum of 18",
                            var.name, precision
                        ),
                    });
                }
                let min_scaled = parse_decimal_scaled(min, *precision).map_err(|e| {
                    SvgError::InvalidVariableDef {
                        reason: format!("variable '{}' min: {}", var.name, e),
                    }
                })?;
                let max_scaled = parse_decimal_scaled(max, *precision).map_err(|e| {
                    SvgError::InvalidVariableDef {
                        reason: format!("variable '{}' max: {}", var.name, e),
                    }
                })?;
                if min_scaled > max_scaled {
                    return Err(SvgError::InvalidVariableDef {
                        reason: format!(
                            "variable '{}': min ({}) must be <= max ({})",
                            var.name, min, max
                        ),
                    });
                }
            }
            VariableKind::Rgb => {}
            VariableKind::RgbStyled(ranges) => {
                if ranges.is_empty() {
                    return Err(SvgError::InvalidVariableDef {
                        reason: format!(
                            "variable '{}': rgb_styled ranges list must not be empty",
                            var.name
                        ),
                    });
                }
                for (i, range) in ranges.iter().enumerate() {
                    if range.r_min > range.r_max
                        || range.g_min > range.g_max
                        || range.b_min > range.b_max
                    {
                        return Err(SvgError::InvalidVariableDef {
                            reason: format!(
                                "variable '{}': range {} has min > max for a channel",
                                var.name, i
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn validate_template_slots(
    template: &str,
    variables: &[VariableDef],
    slots: &[TemplateSlot],
) -> Result<(), SvgError> {
    let tpl_len = template.len() as u32;
    let var_count = variables.len() as u16;
    let mut prev_end: u32 = 0;

    for (i, slot) in slots.iter().enumerate() {
        // Bounds check
        if slot.start >= slot.end || slot.end > tpl_len {
            return Err(SvgError::InvalidTemplatePlaceholder {
                reason: format!(
                    "slot {} out of bounds: start={}, end={}, template_len={}",
                    i, slot.start, slot.end, tpl_len
                ),
            });
        }

        // Slots must be sorted and non-overlapping
        if slot.start < prev_end {
            return Err(SvgError::InvalidTemplatePlaceholder {
                reason: format!(
                    "slot {} overlaps or is out of order: start={} < prev_end={}",
                    i, slot.start, prev_end
                ),
            });
        }
        prev_end = slot.end;

        // var_idx must reference a valid variable
        if slot.var_idx >= var_count {
            return Err(SvgError::InvalidTemplatePlaceholder {
                reason: format!(
                    "slot {} var_idx {} out of range (only {} variables)",
                    i, slot.var_idx, var_count
                ),
            });
        }

        // Verify the template actually contains ${varname} at this position
        let start = slot.start as usize;
        let end = slot.end as usize;
        let expected = format!("${{{}}}", variables[slot.var_idx as usize].name);
        let actual = &template[start..end];
        if actual != expected {
            return Err(SvgError::InvalidTemplatePlaceholder {
                reason: format!(
                    "slot {} mismatch at [{}, {}): expected '{}', found '{}'",
                    i, start, end, expected, actual
                ),
            });
        }
    }

    Ok(())
}

/// Reject an element whose `fill` or `stroke` is painted only by `<animate>` or `<set>`.
///
/// SVG's initial `fill` is black, so a missing fill still paints. Initial `stroke` is
/// `none`. An explicit `none` or `transparent` on the element or an ancestor does not
/// count as static paint. A `style` declaration wins over the presentation attribute.
pub fn validate_animated_paint(template: &str) -> Result<(), SvgError> {
    let mut stack: Vec<PaintFrame> = Vec::new();
    let mut rest = template;
    while let Some(idx) = rest.find('<') {
        rest = &rest[idx..];
        if rest.starts_with("<!--") {
            rest = skip_until(rest, "-->", 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            rest = skip_until(rest, "]]>", 3);
            continue;
        }
        if rest.starts_with("<?") {
            rest = skip_until(rest, "?>", 2);
            continue;
        }
        if rest.starts_with("<!") {
            rest = skip_until(rest, ">", 1);
            continue;
        }
        let Some(tag) = take_tag(rest) else {
            break;
        };
        rest = &rest[tag.raw_len..];
        if tag.closing {
            stack.pop();
            continue;
        }

        let parent = stack.last();
        let name = local_name(&tag.name);
        if name == "animate" || name == "set" {
            if let Some(attribute) = animated_paint_attr(&tag.attrs) {
                let effective = match attribute {
                    "fill" => parent.and_then(|p| p.fill.as_deref()),
                    "stroke" => parent.and_then(|p| p.stroke.as_deref()),
                    _ => None,
                };
                let visible = match attribute {
                    "fill" => effective.map(is_visible_paint).unwrap_or(true),
                    "stroke" => effective.map(is_visible_paint).unwrap_or(false),
                    _ => true,
                };
                if !visible {
                    return Err(SvgError::AnimationOnlyPaint {
                        element: parent
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| name.to_string()),
                        attribute: attribute.to_string(),
                    });
                }
            }
            if !tag.self_closing {
                stack.push(PaintFrame {
                    name: name.to_string(),
                    fill: parent.and_then(|p| p.fill.clone()),
                    stroke: parent.and_then(|p| p.stroke.clone()),
                });
            }
            continue;
        }

        let fill = paint_from_tag(&tag.attrs, "fill").or_else(|| parent.and_then(|p| p.fill.clone()));
        let stroke =
            paint_from_tag(&tag.attrs, "stroke").or_else(|| parent.and_then(|p| p.stroke.clone()));
        if !tag.self_closing {
            stack.push(PaintFrame {
                name: name.to_string(),
                fill,
                stroke,
            });
        }
    }
    Ok(())
}

struct PaintFrame {
    name: String,
    fill: Option<String>,
    stroke: Option<String>,
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    self_closing: bool,
    closing: bool,
    raw_len: usize,
}

fn skip_until<'a>(s: &'a str, marker: &str, extra: usize) -> &'a str {
    match s.find(marker) {
        Some(end) => &s[end + extra..],
        None => "",
    }
}

fn take_tag(s: &str) -> Option<Tag> {
    if !s.starts_with('<') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = 1;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'>' {
            break;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let inner = s[1..i].trim();
    let self_closing = inner.ends_with('/');
    let closing = inner.starts_with('/');
    let body = if closing { inner[1..].trim_start() } else { inner };
    let body = body.trim_end_matches('/').trim_end();
    let (name, attrs) = parse_tag_body(body);
    Some(Tag {
        name,
        attrs,
        self_closing,
        closing,
        raw_len: i + 1,
    })
}

fn parse_tag_body(body: &str) -> (String, Vec<(String, String)>) {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_name_byte(bytes[i]) {
        i += 1;
    }
    let name = body[..i].to_string();
    let mut attrs = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && is_name_byte(bytes[i]) {
            i += 1;
        }
        if start == i {
            break;
        }
        let key = body[start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            attrs.push((key, String::new()));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let value = if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            let vs = i;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            let v = body[vs..i].to_string();
            if i < bytes.len() {
                i += 1;
            }
            v
        } else {
            let vs = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                i += 1;
            }
            body[vs..i].to_string()
        };
        attrs.push((key, value));
    }
    (name, attrs)
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b':' || c == b'_'
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn animated_paint_attr(attrs: &[(String, String)]) -> Option<&'static str> {
    let value = attrs
        .iter()
        .find(|(k, _)| k == "attributename")
        .map(|(_, v)| v.as_str())?;
    match value.trim().to_ascii_lowercase().as_str() {
        "fill" => Some("fill"),
        "stroke" => Some("stroke"),
        _ => None,
    }
}

fn paint_from_tag(attrs: &[(String, String)], name: &str) -> Option<String> {
    if let Some((_, style)) = attrs.iter().find(|(k, _)| k == "style") {
        if let Some(value) = style_value(style, name) {
            return Some(value);
        }
    }
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

fn style_value(style: &str, name: &str) -> Option<String> {
    for decl in style.split(';') {
        let mut parts = decl.splitn(2, ':');
        let key = parts.next()?.trim();
        if key.eq_ignore_ascii_case(name) {
            return Some(parts.next()?.trim().to_string());
        }
    }
    None
}

fn is_visible_paint(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("none")
        && !value.eq_ignore_ascii_case("transparent")
}

pub fn resolve_variable<F>(
    token_seed: &[u8],
    var_idx: usize,
    var_def: &VariableDef,
    derive_fn: F,
) -> TokenParam
where
    F: FnOnce(&[u8], usize) -> u64,
{
    let entropy = derive_fn(token_seed, var_idx);

    let value = match &var_def.kind {
        VariableKind::Options(options) => {
            let index = (entropy as usize) % options.len();
            options[index].clone()
        }
        VariableKind::Range {
            min,
            max,
            precision,
        } => {
            // These were validated at instantiation time, so unwrap is safe.
            let min_scaled = parse_decimal_scaled(min, *precision).unwrap();
            let max_scaled = parse_decimal_scaled(max, *precision).unwrap();
            let scaled = value_from_range(entropy, min_scaled, max_scaled);
            format_decimal(scaled, *precision)
        }
        VariableKind::Rgb => {
            let bytes = entropy.to_le_bytes();
            format!("rgb({},{},{})", bytes[0], bytes[1], bytes[2])
        }
        VariableKind::RgbStyled(ranges) => {
            let bytes = entropy.to_le_bytes();
            // Use upper bytes to select range, lower bytes for channel values
            let range_idx = (bytes[3] as usize) % ranges.len();
            let range = &ranges[range_idx];
            let r = channel_in_range(bytes[0], range.r_min, range.r_max);
            let g = channel_in_range(bytes[1], range.g_min, range.g_max);
            let b = channel_in_range(bytes[2], range.b_min, range.b_max);
            format!("rgb({},{},{})", r, g, b)
        }
    };

    TokenParam {
        name: var_def.name.clone(),
        value,
    }
}

/// Generate a decimal string value from entropy within [min, max] at the given precision.
pub fn value_from_range(entropy: u64, min_scaled: i128, max_scaled: i128) -> i128 {
    let range = (max_scaled - min_scaled) as u128;
    if range == 0 {
        return min_scaled;
    }
    let offset = (entropy as u128) % (range + 1);
    min_scaled + offset as i128
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SvgError {
    #[error("SVG template exceeds max size of {max} bytes (got {got})")]
    SvgTemplateTooLarge { max: usize, got: usize },

    #[error("Invalid variable definition: {reason}")]
    InvalidVariableDef { reason: String },

    #[error("Invalid template placeholder: {reason}")]
    InvalidTemplatePlaceholder { reason: String },

    #[error("SVG element <{element}> animates {attribute} with no static {attribute}")]
    AnimationOnlyPaint { element: String, attribute: String },

    #[error("Unsafe link: {reason}")]
    UnsafeLink { reason: String },
}

#[cfg(test)]
mod animated_paint_tests {
    use super::validate_animated_paint;
    use std::fs;
    use std::path::Path;

    #[test]
    fn dao_eyes_keep_a_static_fill() {
        let template = include_str!("../../../scripts/cw-infuser/svgs/interchain/dao/template.svg");
        validate_animated_paint(template).unwrap();
        assert!(template.contains(r#"<circle cx="8.5" cy="14" r="1.5" fill="${color-yang}">"#));
        assert!(template.contains(r#"<circle cx="19.5" cy="14" r="1.5" fill="${color-yin}">"#));
    }

    #[test]
    fn shipped_templates_have_static_paint() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cw-infuser/svgs");
        let mut seen = 0;
        visit_svgs(&root, &mut |path, text| {
            seen += 1;
            validate_animated_paint(text).unwrap_or_else(|e| {
                panic!("{}: {e}", path.display());
            });
            crate::links::validate_links(text, &[]).unwrap_or_else(|e| {
                if !e.to_string().contains("undefined variable") { panic!("{}: {e}", path.display()); }
            });
        });
        assert!(seen > 10, "expected the interchain svg tree, saw {seen}");
    }

    #[test]
    fn animation_only_fill_is_rejected() {
        let bad = r#"<svg fill="none"><circle cx="1" cy="1" r="1"><animate attributeName="fill" values="red;blue"/></circle></svg>"#;
        let err = validate_animated_paint(bad).unwrap_err().to_string();
        assert!(err.contains("<circle>"), "{err}");
        assert!(err.contains("fill"), "{err}");
    }

    #[test]
    fn static_fill_and_inherited_fill_are_accepted() {
        let own = r#"<svg fill="none"><circle fill="${color-yang}"><animate attributeName="fill" values="${color-yang};blue"/></circle></svg>"#;
        validate_animated_paint(own).unwrap();
        let inherited = r#"<svg fill="red"><circle><animate attributeName="fill" values="red;blue"/></circle></svg>"#;
        validate_animated_paint(inherited).unwrap();
        let style = r#"<svg fill="none"><circle style="fill: rgb(1,1,1)"><set attributeName="fill" to="rgb(2,2,2)"/></circle></svg>"#;
        validate_animated_paint(style).unwrap();
    }

    #[test]
    fn animation_only_stroke_is_rejected() {
        let bad = r#"<svg><path d="M0 0L1 1"><animate attributeName="stroke" values="red;blue"/></path></svg>"#;
        let err = validate_animated_paint(bad).unwrap_err().to_string();
        assert!(err.contains("stroke"), "{err}");
        let good = r#"<svg><path d="M0 0L1 1" stroke="rgb(1,1,1)"><animate attributeName="stroke" values="rgb(1,1,1);blue"/></path></svg>"#;
        validate_animated_paint(good).unwrap();
    }

    fn visit_svgs(dir: &Path, f: &mut dyn FnMut(&Path, &str)) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit_svgs(&path, f);
            } else if path.extension().and_then(|e| e.to_str()) == Some("svg") {
                let text = fs::read_to_string(&path).unwrap();
                f(&path, &text);
            }
        }
    }
}

/// Map a byte into a [min, max] range (inclusive).
pub fn channel_in_range(byte: u8, min: u8, max: u8) -> u8 {
    if min == max {
        return min;
    }
    let span = (max - min) as u16 + 1;
    min + (byte as u16 % span) as u8
}
