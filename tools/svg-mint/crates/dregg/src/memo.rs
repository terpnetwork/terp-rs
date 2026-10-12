//! Payment memo: Terp bech32 of the NFT recipient, carried in ETH calldata or BTC OP_RETURN.

/// First `terp1` bech32 in utf-8 (or hex-decoded) payment data.
pub fn parse_terp_recipient(raw: &[u8]) -> Option<String> {
    if let Some(s) = from_utf8_lossy_terp(raw) {
        return Some(s);
    }
    let hexish = std::str::from_utf8(raw).unwrap_or("");
    let h = hexish.trim().trim_start_matches("0x");
    if let Ok(b) = hex::decode(h) {
        return from_utf8_lossy_terp(&b);
    }
    None
}

fn from_utf8_lossy_terp(raw: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(raw);
    let i = s.find("terp1")?;
    let rest = &s[i..];
    let addr: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if addr.len() >= 20 && addr.starts_with("terp1") {
        Some(addr)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_terp1_in_op_return() {
        let a = parse_terp_recipient(b"terp1cyyzpxplxdzkeea7kwsydadg87357qnacvvp23 extra");
        assert_eq!(
            a.as_deref(),
            Some("terp1cyyzpxplxdzkeea7kwsydadg87357qnacvvp23")
        );
    }
}
