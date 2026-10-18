//! Filter operator logs before they are stored on a step.
//!
//! A standalone hex run of 64 or more digits is replaced with `[hex]`,
//! including a `0x` prefix. Transaction hashes of that length are redacted
//! along with key material. Shorter hex stays.

pub fn scrub_log(text: &str) -> String {
    text.lines()
        .filter(|line| !phrase_secret(line))
        .map(redact_long_hex)
        .map(|line| clip400(&line))
        .take(8)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn phrase_secret(line: &str) -> bool {
    let low = line.to_ascii_lowercase();
    let phrase = line.split_whitespace().count() >= 12
        && line
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_whitespace());
    phrase || low.contains("mnemonic") || low.contains("priv_key") || low.contains("private key")
}

fn clip400(line: &str) -> String {
    if line.len() <= 400 {
        return line.to_string();
    }
    let mut end = 400;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

fn redact_long_hex(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if prev_ok && bytes[i].is_ascii_hexdigit() {
            let (hex_at, token_start) = if i + 2 < bytes.len()
                && bytes[i] == b'0'
                && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
                && bytes[i + 2].is_ascii_hexdigit()
            {
                (i + 2, i)
            } else {
                (i, i)
            };
            let mut k = hex_at;
            while k < bytes.len() && bytes[k].is_ascii_hexdigit() {
                k += 1;
            }
            let next_ok = k == bytes.len() || !bytes[k].is_ascii_alphanumeric();
            if next_ok && k - hex_at >= 64 {
                out.push_str("[hex]");
                i = k;
                continue;
            }
            out.push_str(&line[token_start..k]);
            i = k;
            continue;
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_drops_phrases_and_long_hex() {
        let key = "ab".repeat(32);
        let short = "ab".repeat(31);
        let prefixed = format!("0x{}", "cd".repeat(32));
        let text = format!(
            "ok {short}\nmnemonic phrase here\n{key}\nprivate key here\n{prefixed}\npriv_key=1\n"
        );
        let out = scrub_log(&text);
        assert!(out.contains("ok "));
        assert!(out.contains(&short));
        assert!(!out.contains("mnemonic"));
        assert!(!out.contains(&key));
        assert!(!out.contains(&prefixed));
        assert!(out.contains("[hex]"));
        assert!(!out.to_ascii_lowercase().contains("private key"));
        assert!(!out.contains("priv_key"));
        let phrase = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
        assert!(!scrub_log(&phrase).contains("alpha"));
    }
}
