//! One-operator gate for `/trees` writes.
//!
//! Reads stay public. `POST`, `PUT`, and `DELETE` require an ADR-36 signature
//! from the configured `terp1` operator. The signed challenge binds method,
//! path, and the SHA-256 of the raw body.

use bech32::{Bech32, Hrp};
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

pub use crate::config::DEFAULT_TREE_OPERATOR;

pub const CHALLENGE_DOMAIN: &str = "hash-market.tree-write.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeWriteDeny {
    /// Missing or invalid signature material.
    Unauthorized,
    /// Signature verified for a different address.
    Forbidden,
}

/// Canonical UTF-8 string placed in the ADR-36 `data` field (then standard-base64'd).
pub fn tree_write_challenge(method: &str, path: &str, body: &[u8]) -> String {
    let body_hash = hex::encode(Sha256::digest(body));
    format!(
        "{CHALLENGE_DOMAIN}\n{}\n{path}\n{body_hash}",
        method.to_ascii_uppercase()
    )
}

/// Amino `sign/MsgSignData` document. `data` is standard base64 of the challenge.
/// `signer` is the bech32 account derived from the pubkey.
pub fn adr36_preamble(signer: &str, data_b64: &str) -> String {
    format!(
        "{{\"account_number\":\"0\",\"chain_id\":\"\",\"fee\":{{\"amount\":[],\"gas\":\"0\"}},\"memo\":\"\",\"msgs\":[{{\"type\":\"sign/MsgSignData\",\"value\":{{\"data\":\"{data_b64}\",\"signer\":\"{signer}\"}}}}],\"sequence\":\"0\"}}"
    )
}

pub fn adr36_digest(signer: &str, challenge: &str) -> [u8; 32] {
    let data_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        challenge.as_bytes(),
    );
    let preamble = adr36_preamble(signer, &data_b64);
    let digest = Sha256::digest(preamble.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn pubkey_to_terp(pubkey: &[u8]) -> Result<String, String> {
    if pubkey.len() != 33 {
        return Err("pubkey must be 33-byte compressed secp256k1".into());
    }
    let sha = Sha256::digest(pubkey);
    let addr = Ripemd160::digest(sha);
    let hrp = Hrp::parse("terp").map_err(|e| e.to_string())?;
    bech32::encode::<Bech32>(hrp, &addr).map_err(|e| e.to_string())
}

/// Fresh compressed-secp256k1 operator. The key stays in the returned `SigningKey`.
pub fn ephemeral_operator() -> (k256::ecdsa::SigningKey, String) {
    let sk = k256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
    let pk = sk.verifying_key().to_encoded_point(true);
    let addr = pubkey_to_terp(pk.as_bytes()).expect("33-byte compressed pubkey");
    (sk, addr)
}

/// Hex pubkey and 64-byte r||s signature over `tree_write_challenge`.
pub fn sign_tree_write(
    sk: &k256::ecdsa::SigningKey,
    method: &str,
    path: &str,
    body: &[u8],
) -> (String, String) {
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    let pk = sk.verifying_key().to_encoded_point(true);
    let pubkey = pk.as_bytes();
    let addr = pubkey_to_terp(pubkey).expect("33-byte compressed pubkey");
    let digest = adr36_digest(&addr, &tree_write_challenge(method, path, body));
    let sig: Signature = sk.sign_prehash(&digest).expect("sign tree write");
    (hex::encode(pubkey), hex::encode(sig.to_bytes()))
}

/// Verify `signature_hex` (64-byte r||s) over the ADR-36 digest of this write.
pub fn authorize_tree_write(
    operator: &str,
    method: &str,
    path: &str,
    body: &[u8],
    pubkey_hex: &str,
    signature_hex: &str,
) -> Result<(), TreeWriteDeny> {
    let operator = operator.trim();
    if operator.is_empty() {
        return Err(TreeWriteDeny::Unauthorized);
    }
    let pubkey = hex::decode(pubkey_hex.trim()).map_err(|_| TreeWriteDeny::Unauthorized)?;
    let signature = hex::decode(signature_hex.trim()).map_err(|_| TreeWriteDeny::Unauthorized)?;
    let signer = pubkey_to_terp(&pubkey).map_err(|_| TreeWriteDeny::Unauthorized)?;
    if signer != operator {
        return Err(TreeWriteDeny::Forbidden);
    }
    if signature.len() != 64 {
        return Err(TreeWriteDeny::Unauthorized);
    }
    let vk = VerifyingKey::from_sec1_bytes(&pubkey).map_err(|_| TreeWriteDeny::Unauthorized)?;
    let sig = Signature::from_slice(&signature).map_err(|_| TreeWriteDeny::Unauthorized)?;
    let digest = adr36_digest(&signer, &tree_write_challenge(method, path, body));
    vk.verify_prehash(&digest, &sig)
        .map_err(|_| TreeWriteDeny::Unauthorized)
}

/// `true` when `bind`'s host is IPv4 loopback, `localhost`, or `::1`.
pub fn bind_host_is_loopback(bind: &str) -> bool {
    match bind_host(bind) {
        "localhost" | "::1" => true,
        host => host
            .parse::<std::net::Ipv4Addr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false),
    }
}

/// Refuse `0.0.0.0`, `::`, and every other non-loopback host unless `allow_public`.
/// Returns `Ok(true)` when a public bind was explicitly allowed.
pub fn inspect_bind(bind: &str, allow_public: bool) -> Result<bool, String> {
    let host = bind_host(bind);
    if host.is_empty() {
        return Err(format!("bind address '{bind}' has no host"));
    }
    if bind_host_is_loopback(bind) {
        return Ok(false);
    }
    if !allow_public {
        return Err(format!(
            "refusing non-loopback bind '{bind}'. Default is 127.0.0.1. Set allow_public_bind or HASH_MARKET_ALLOW_PUBLIC_BIND=1 to listen on this address."
        ));
    }
    Ok(true)
}

fn bind_host(bind: &str) -> &str {
    let bind = bind.trim();
    if let Some(rest) = bind.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("");
    }
    if bind.matches(':').count() == 1 {
        return bind.split(':').next().unwrap_or(bind);
    }
    bind
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    fn sign(method: &str, path: &str, body: &[u8]) -> (String, String, String) {
        let sk = SigningKey::random(&mut rand_core::OsRng);
        let pk = sk.verifying_key().to_encoded_point(true);
        let pubkey = pk.as_bytes();
        let addr = pubkey_to_terp(pubkey).unwrap();
        let digest = adr36_digest(&addr, &tree_write_challenge(method, path, body));
        let sig: Signature = sk.sign_prehash(&digest).unwrap();
        (addr, hex::encode(pubkey), hex::encode(sig.to_bytes()))
    }

    #[test]
    fn operator_signature_roundtrip_and_rejects() {
        let body = br#"{"merkle_root":"aa"}"#;
        let path = "/trees/f5c68fb4b1e98358963cdc43e83b653875f3f4794ed46d2927e65884451253b1";
        let (addr, pk, sig) = sign("post", path, body);
        authorize_tree_write(&addr, "POST", path, body, &pk, &sig).unwrap();
        assert!(matches!(
            authorize_tree_write(DEFAULT_TREE_OPERATOR, "POST", path, body, &pk, &sig),
            Err(TreeWriteDeny::Forbidden)
        ));
        assert!(matches!(
            authorize_tree_write(&addr, "PUT", path, body, &pk, &sig),
            Err(TreeWriteDeny::Unauthorized)
        ));
        assert!(matches!(
            authorize_tree_write(&addr, "POST", path, b"other", &pk, &sig),
            Err(TreeWriteDeny::Unauthorized)
        ));
        assert!(matches!(
            authorize_tree_write(&addr, "POST", path, body, "", ""),
            Err(TreeWriteDeny::Unauthorized)
        ));
    }

    #[test]
    fn loopback_is_the_default_and_public_bind_needs_override() {
        assert!(!inspect_bind("127.0.0.1:9090", false).unwrap());
        assert!(!inspect_bind("[::1]:9090", false).unwrap());
        assert!(!inspect_bind("localhost:9090", false).unwrap());
        assert!(inspect_bind("0.0.0.0:9090", false).is_err());
        assert!(inspect_bind("[::]:9090", false).is_err());
        assert!(inspect_bind("10.0.0.8:9090", false).is_err());
        assert!(inspect_bind("0.0.0.0:9090", true).unwrap());
    }
}
