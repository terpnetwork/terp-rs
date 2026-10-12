//! Cosmos ADR-036 (`sign/MsgSignData`) as used by Keplr `signArbitrary`.
//! Offline proof of a terp1 address — not a chain tx, not the ZEC deposit UA.

use bech32::{ToBase32, Variant};
use k256::ecdsa::{Signature, VerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

/// Donation UA used for site donate QR — only allowlisted ZEC withdraw dest.
pub const DONATE_ZEC_UA: &str = "u1uml44spc4nn732t6605wjcwu3wn9dla4aehpyrvnhssq2ew3dk8mqk5e9jhnndzfny7fkrkx5ape05nzf8gwruqykkvc27fm0v9ln4hfrwkmtfrs27r06xu0uc9eneh777le3m8362v7xaurgtkgxtspy5eqhgpzw6axjwgwdy5wjdjt";

pub fn withdraw_ua_allowed(to: &str, allow: &[String]) -> bool {
    let t = to.trim();
    t.starts_with("u1") && allow.iter().any(|u| u == t)
}

pub fn terp1_from_secp256k1_compressed(pk33: &[u8]) -> Result<String, String> {
    if pk33.len() != 33 {
        return Err("secp256k1 compressed pubkey is 33 bytes".into());
    }
    let sha = Sha256::digest(pk33);
    let ripe = Ripemd160::digest(sha);
    bech32::encode("terp", ripe.to_base32(), Variant::Bech32).map_err(|e| e.to_string())
}

/// Amino JSON Keplr `signArbitrary` actually signs.
///
/// `chain_id` in the signed bytes is `""`. The chain id passed to
/// `keplr.enable` / `signArbitrary` only selects the key; it is not in the doc.
/// Sorted keys, no extra spaces — same as cosmjs `serializeSignDoc`.
pub const KEPLR_SIGN_DOC_CHAIN_ID: &str = "";

/// Bytes Keplr signs for an ops withdraw. `challenge_hex` is the live nonce, not a second encoding.
pub fn withdraw_statement(zat: u64, ua: &str, challenge_hex: &str) -> String {
    format!("permissionless.money withdraw {zat} zat to {ua} {challenge_hex}")
}

/// Keplr bytes for pushing a mint or refunding a paid checkout that never minted.
/// Not the withdraw sentence, and not the sign-in challenge bytes.
pub fn checkout_statement(action: &str, terp: &str, challenge_hex: &str) -> Result<String, String> {
    if action != "mint-to" && action != "refund" {
        return Err("checkout action is mint-to or refund".into());
    }
    if !terp.starts_with("terp1") || terp.chars().any(|c| c.is_whitespace()) {
        return Err("checkout terp".into());
    }
    if challenge_hex.is_empty() || challenge_hex.chars().any(|c| c.is_whitespace()) {
        return Err("checkout challenge".into());
    }
    Ok(format!(
        "permissionless.money {action} {terp} {challenge_hex}"
    ))
}


pub fn sign_doc_json(chain_id: &str, signer: &str, data: &[u8]) -> String {
    let data_b64 = b64(data);
    format!(
        r#"{{"account_number":"0","chain_id":"{chain}","fee":{{"amount":[],"gas":"0"}},"memo":"","msgs":[{{"type":"sign/MsgSignData","value":{{"data":"{data}","signer":"{signer}"}}}}],"sequence":"0"}}"#,
        chain = escape(chain_id),
        data = data_b64,
        signer = escape(signer),
    )
}

pub fn verify_keplr_arbitrary(
    chain_id: &str,
    signer: &str,
    data: &[u8],
    pub_key_b64: &str,
    sig_b64: &str,
) -> Result<String, String> {
    let pk = b64_decode(pub_key_b64)?;
    let sig_raw = b64_decode(sig_b64)?;
    if sig_raw.len() != 64 {
        return Err("ADR-036 signature is 64 bytes".into());
    }
    let vk = VerifyingKey::from_sec1_bytes(&pk).map_err(|e| e.to_string())?;
    let encoded = vk.to_encoded_point(true);
    let terp = terp1_from_secp256k1_compressed(encoded.as_bytes())?;
    if terp != signer {
        return Err(format!("signer {signer} does not match pubkey {terp}"));
    }
    // Ignore the caller's chain id. Keplr signs chain_id:"".
    let _ = chain_id;
    let doc = sign_doc_json(KEPLR_SIGN_DOC_CHAIN_ID, signer, data);
    let hash = Sha256::digest(doc.as_bytes());
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    let sig = Signature::from_slice(&sig_raw).map_err(|e| e.to_string())?;
    vk.verify_prehash(&hash, &sig)
        .map_err(|_| "ADR-036 signature mismatch".to_string())?;
    Ok(terp)
}

/// Lab / operator helper: secp256k1 secret → terp1 + Keplr pub_key/signature over `data`.
pub fn sign_keplr_arbitrary(
    secret32: &[u8],
    chain_id: &str,
    data: &[u8],
) -> Result<(String, String, String), String> {
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};
    let sk = SigningKey::from_slice(secret32).map_err(|e| e.to_string())?;
    let pk = sk.verifying_key().to_encoded_point(true);
    let terp = terp1_from_secp256k1_compressed(pk.as_bytes())?;
    let doc = sign_doc_json(chain_id, &terp, data);
    let hash = Sha256::digest(doc.as_bytes());
    let sig: Signature = sk.sign_prehash(&hash).map_err(|e| e.to_string())?;
    Ok((terp, b64(pk.as_bytes()), b64(sig.to_bytes().as_ref())))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < data.len() {
            out.push(T[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("b64".into()),
        }
    }
    let s: Vec<u8> = s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()).collect();
    if s.len() % 4 != 0 && s.len() % 4 == 1 {
        return Err("b64 length".into());
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < s.len() {
        let a = val(s[i])?;
        let b = val(s[i + 1])?;
        out.push((a << 2) | (b >> 4));
        if i + 2 < s.len() {
            let c = val(s[i + 2])?;
            out.push((b << 4) | (c >> 2));
            if i + 3 < s.len() {
                let d = val(s[i + 3])?;
                out.push((c << 6) | d);
            }
        }
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terp1_len() {
        let pk = [2u8; 33];
        let a = terp1_from_secp256k1_compressed(&pk).unwrap();
        assert!(a.starts_with("terp1"));
    }

    #[test]
    fn withdraw_only_allowlisted_donate_ua() {
        let allow = vec![DONATE_ZEC_UA.to_string()];
        assert!(withdraw_ua_allowed(DONATE_ZEC_UA, &allow));
        assert!(!withdraw_ua_allowed("u1notallowlistedxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", &allow));
        assert!(!withdraw_ua_allowed("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqg6p2x0", &allow));
        assert!(!withdraw_ua_allowed("", &allow));
    }

    #[test]
    fn adr36_sign_doc_is_msg_sign_data() {
        let doc = sign_doc_json("morocco-1", "terp1abc", b"\x01\x02");
        assert!(doc.contains("sign/MsgSignData"));
        assert!(doc.contains("\"signer\":\"terp1abc\""));
        assert!(doc.contains("\"chain_id\":\"morocco-1\""));
        assert!(doc.contains("\"account_number\":\"0\""));
        assert!(doc.contains("\"sequence\":\"0\""));
    }

    #[test]
    fn adr36_garbage_sig_fails() {
        let pk = [2u8; 33];
        let pk_b64 = b64(&pk);
        let sig_b64 = b64(&[3u8; 64]);
        let err = verify_keplr_arbitrary("morocco-1", "terp1nope", b"abcd", &pk_b64, &sig_b64)
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn adr36_sign_verify_roundtrip() {
        let mut secret = [0u8; 32];
        secret[31] = 1;
        let data = b"e2e-challenge";
        // Keplr signArbitrary signs chain_id:"" even when enable("morocco-1").
        let (terp, pk, sig) = sign_keplr_arbitrary(&secret, "", data).unwrap();
        assert!(terp.starts_with("terp1"));
        let got = verify_keplr_arbitrary("morocco-1", &terp, data, &pk, &sig).unwrap();
        assert_eq!(got, terp);
        let (_t, pk_bad, sig_bad) = sign_keplr_arbitrary(&secret, "morocco-1", data).unwrap();
        let err = verify_keplr_arbitrary("morocco-1", &terp, data, &pk_bad, &sig_bad).unwrap_err();
        assert!(err.contains("signature mismatch"), "{err}");
        let mut other = [0u8; 32];
        other[31] = 2;
        let (terp2, _, _) = sign_keplr_arbitrary(&other, "", data).unwrap();
        assert_ne!(terp, terp2);
    }

    #[test]
    fn adr36_keplr_doc_matches_empty_chain_id() {
        let data = b64_decode("fQqnl1zyCAQ+FUB08z91f4Y27vP7sk4tFlGLNpuyYQg=").unwrap();
        let signer = "terp1fc34h3q2af5lxdy2f4h7eq2h7smlerpsyz0zar";
        let doc = sign_doc_json("", signer, &data);
        assert_eq!(
            doc,
            r#"{"account_number":"0","chain_id":"","fee":{"amount":[],"gas":"0"},"memo":"","msgs":[{"type":"sign/MsgSignData","value":{"data":"fQqnl1zyCAQ+FUB08z91f4Y27vP7sk4tFlGLNpuyYQg=","signer":"terp1fc34h3q2af5lxdy2f4h7eq2h7smlerpsyz0zar"}}],"sequence":"0"}"#
        );
        assert!(!sign_doc_json("morocco-1", signer, &data).contains("\"chain_id\":\"\""));
    }

    #[test]
    fn withdraw_statement_matches_keplr_data() {
        let challenge = "8fea0767dd928658ca128e3d4ee365c8eec6578cb8b0349ed05a204730949b92";
        let statement = withdraw_statement(290000, DONATE_ZEC_UA, challenge);
        let doc = sign_doc_json("", "terp1fc34h3q2af5lxdy2f4h7eq2h7smlerpsyz0zar", statement.as_bytes());
        assert!(doc.contains("cGVybWlzc2lvbmxlc3MubW9uZXkgd2l0aGRyYXcgMjkwMDAw"));
        assert!(doc.contains("\"chain_id\":\"\""));
    }

    #[test]
    fn checkout_statement_is_not_a_withdraw_or_a_sign_in() {
        let challenge = "8fea0767dd928658ca128e3d4ee365c8eec6578cb8b0349ed05a204730949b92";
        let terp = "terp1fc34h3q2af5lxdy2f4h7eq2h7smlerpsyz0zar";
        let mint = checkout_statement("mint-to", terp, challenge).unwrap();
        let refund = checkout_statement("refund", terp, challenge).unwrap();
        let withdraw = withdraw_statement(100, DONATE_ZEC_UA, challenge);
        assert_ne!(mint, withdraw);
        assert_ne!(refund, withdraw);
        assert_ne!(mint, refund);
        assert!(mint.starts_with("permissionless.money mint-to "));
        assert!(checkout_statement("withdraw", terp, challenge).is_err());
        assert!(checkout_statement("mint-to", "not-a-terp", challenge).is_err());
        let mut secret = [0u8; 32];
        secret[31] = 1;
        let (signer, pk, sig) = sign_keplr_arbitrary(&secret, "", mint.as_bytes()).unwrap();
        verify_keplr_arbitrary("morocco-1", &signer, mint.as_bytes(), &pk, &sig).unwrap();
        let crossed = verify_keplr_arbitrary("morocco-1", &signer, withdraw.as_bytes(), &pk, &sig);
        assert!(crossed.is_err());
        let raw = hex_ok(challenge);
        let crossed_in = verify_keplr_arbitrary("morocco-1", &signer, &raw, &pk, &sig);
        assert!(crossed_in.is_err());
    }

    fn hex_ok(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
