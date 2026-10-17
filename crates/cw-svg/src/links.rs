//! Link allowlist for templates: every `href` / `xlink:href` must be a same-document
//! `#id`, an `ipfs://<cid>[/path]` URL, or `https://<allowlisted gateway>/ipfs/<cid>[/path]`.
//! Placeholders inside a link must be `Options` variables; every combination is checked.
use crate::{SvgError, VariableDef, VariableKind};

/// HTTPS IPFS gateways a template may link to (exact host match).
pub const GATEWAY_ALLOWLIST: [&str; 3] = ["ipfs.io", "dweb.link", "w3s.link"];
/// Upper bound on expanded link combinations checked per attribute.
pub const MAX_LINK_COMBOS: usize = 256;
const SCHEMES_BANNED: [&str; 4] = ["javascript:", "vbscript:", "data:text", "data:application"];

fn err(reason: String) -> SvgError {
    SvgError::UnsafeLink { reason }
}

fn cid_path_ok(s: &str) -> bool {
    let (cid, path) = match s.find('/') {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let cid_ok = cid.len() >= 20
        && cid.len() <= 120
        && cid.starts_with('b')
        && cid.bytes().all(|c| matches!(c, b'a'..=b'z' | b'2'..=b'7'));
    let path_ok = path.is_empty()
        || (path.split('/').skip(1).all(|seg| {
            !seg.is_empty() && seg != "." && seg != ".."
                && seg.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        }) && path.len() <= 128);
    cid_ok && path_ok
}

/// One fully expanded link value.
pub fn link_ok(v: &str) -> bool {
    if let Some(id) = v.strip_prefix('#') {
        return !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-'));
    }
    if let Some(rest) = v.strip_prefix("ipfs://") {
        return cid_path_ok(rest);
    }
    if let Some(rest) = v.strip_prefix("https://") {
        if let Some(i) = rest.find("/ipfs/") {
            return GATEWAY_ALLOWLIST.contains(&&rest[..i]) && cid_path_ok(&rest[i + 6..]);
        }
    }
    false
}

/// Split an attribute value into literal / `${var}` parts and check every expansion.
fn check_value(raw: &str, vars: &[VariableDef]) -> Result<(), SvgError> {
    let mut parts: Vec<Vec<&str>> = Vec::new();
    let mut rest = raw;
    let mut combos = 1usize;
    while let Some(i) = rest.find("${") {
        if i > 0 {
            parts.push(vec![&rest[..i]]);
        }
        let j = rest[i..].find('}').ok_or_else(|| err(format!("unclosed placeholder in link {raw:?}")))? + i;
        let name = &rest[i + 2..j];
        let var = vars.iter().find(|v| v.name == name).ok_or_else(|| err(format!("undefined variable {name:?} in link")))?;
        match &var.kind {
            VariableKind::Options(o) => {
                combos = combos.saturating_mul(o.len().max(1));
                parts.push(o.iter().map(|s| s.as_str()).collect());
            }
            _ => return Err(err(format!("link variable {name:?} must be an options variable"))),
        }
        rest = &rest[j + 1..];
    }
    if !rest.is_empty() {
        parts.push(vec![rest]);
    }
    if combos > MAX_LINK_COMBOS {
        return Err(err(format!("link {raw:?} expands to {combos} values (max {MAX_LINK_COMBOS})")));
    }
    let mut idx = vec![0usize; parts.len()];
    loop {
        let s: String = parts.iter().zip(&idx).map(|(p, i)| p[*i]).collect();
        if !link_ok(&s) {
            return Err(err(format!("link {s:?} is not #id, ipfs:// or an allowlisted https gateway")));
        }
        let mut k = 0;
        loop {
            if k == parts.len() {
                return Ok(());
            }
            idx[k] += 1;
            if idx[k] < parts[k].len() {
                break;
            }
            idx[k] = 0;
            k += 1;
        }
    }
}

/// Validate every link attribute in the template and ban script-capable schemes anywhere
/// in the template or in any options value.
pub fn validate_links(template: &str, vars: &[VariableDef]) -> Result<(), SvgError> {
    let lower = template.to_ascii_lowercase();
    for s in SCHEMES_BANNED {
        if lower.contains(s) {
            return Err(err(format!("template contains {s}")));
        }
    }
    for v in vars {
        if let VariableKind::Options(o) = &v.kind {
            for val in o {
                let l = val.to_ascii_lowercase();
                if SCHEMES_BANNED.iter().any(|s| l.contains(s)) {
                    return Err(err(format!("variable {:?} has an unsafe value", v.name)));
                }
            }
        }
    }
    let b = lower.as_bytes();
    let mut i = 0;
    while let Some(off) = lower[i..].find("href") {
        let at = i + off;
        i = at + 4;
        let prev = if at == 0 { b' ' } else { b[at - 1] };
        if !(prev.is_ascii_whitespace() || prev == b':') {
            continue;
        }
        let mut j = at + 4;
        while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
        if j >= b.len() || b[j] != b'=' {
            continue;
        }
        j += 1;
        while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
        let q = *b.get(j).ok_or_else(|| err("dangling href".into()))?;
        if q != b'"' && q != b'\'' {
            return Err(err("unquoted href".into()));
        }
        let end = lower[j + 1..].find(q as char).ok_or_else(|| err("unterminated href".into()))? + j + 1;
        check_value(&template[j + 1..end], vars)?;
        i = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const CID: &str = "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku";
    fn opt(n: &str, v: &[&str]) -> VariableDef {
        VariableDef { name: n.into(), kind: VariableKind::Options(v.iter().map(|s| s.to_string()).collect()) }
    }
    #[test]
    fn allows_ipfs_gateway_and_same_doc() {
        let vars = vec![opt("cid", &[CID]), opt("gw", &["ipfs.io", "dweb.link"])];
        let t = format!("<svg><a href=\"ipfs://${{cid}}\"><text>x</text></a><a href=\"https://${{gw}}/ipfs/${{cid}}/index.html\"/><use xlink:href=\"#p\"/></svg>");
        validate_links(&t, &vars).unwrap();
    }
    #[test]
    fn rejects_bad_links() {
        let vars = vec![opt("gw", &["ipfs.io", "evil.example"]), opt("r", &["1"]), opt("cid", &[CID])];
        for t in [
            "<a href=\"javascript:alert(1)\"/>".to_string(),
            "<a href=\"JaVaScRiPt:alert(1)\"/>".to_string(),
            "<a href=\"&#106;avascript:alert(1)\"/>".to_string(),
            "<a href=\"http://ipfs.io/ipfs/x\"/>".to_string(),
            format!("<a href=\"https://evil.example/ipfs/{CID}\"/>"),
            format!("<a href=\"https://${{gw}}/ipfs/{CID}\"/>"),
            format!("<a href=\"ipfs://{CID}/../x\"/>"),
            "<a href=ipfs://x/>".to_string(),
            "<a xlink:href=\"data:text/html,hi\"/>".to_string(),
            "<a href=\"https://ipfs.io.evil/ipfs/x\"/>".to_string(),
            format!("<a href=\"ipfs://${{nope}}\"/>"),
        ] {
            assert!(validate_links(&t, &vars).is_err(), "{t}");
        }
        let rng = vec![VariableDef { name: "n".into(), kind: VariableKind::Rgb }];
        assert!(validate_links("<a href=\"ipfs://${n}\"/>", &rng).is_err());
        let js = vec![opt("x", &["javascript:alert(1)"])];
        assert!(validate_links("<text>${x}</text>", &js).is_err());
    }
}
