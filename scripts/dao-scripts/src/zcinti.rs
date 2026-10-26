//! Consensus key for a zcinti validator.
//!
//! The private key is written to a file. Callers print the public key only.

use std::fs;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::SigningKey;
use rand08::rngs::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Public half of a new consensus key. `path` holds the CometBFT key file.
pub struct ConsensusPub {
    pub path: String,
    pub pubkey_b64: String,
    pub address_hex: String,
}

/// Create an Ed25519 consensus key and write the CometBFT key file.
/// The private key stays in that file.
pub fn generate_consensus_key(path: &Path) -> Result<ConsensusPub, String> {
    let signing = SigningKey::generate(&mut OsRng);
    let seed = signing.to_bytes();
    let public = signing.verifying_key().to_bytes();
    let mut secret = [0u8; 64];
    secret[..32].copy_from_slice(&seed);
    secret[32..].copy_from_slice(&public);
    let address = consensus_address(&public);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|err| format!("zcinti key dir refused: {err}"))?;
    }
    let body = json!({
        "address": address,
        "pub_key": {
            "type": "tendermint/PubKeyEd25519",
            "value": STANDARD.encode(public),
        },
        "priv_key": {
            "type": "tendermint/PrivKeyEd25519",
            "value": STANDARD.encode(secret),
        },
    });
    let text = serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?;
    fs::write(path, text).map_err(|err| format!("zcinti key write refused: {err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    secret.fill(0);
    Ok(ConsensusPub {
        path: path.display().to_string(),
        pubkey_b64: STANDARD.encode(public),
        address_hex: address,
    })
}

/// Read the public key from a CometBFT key file. The private field is not returned.
pub fn read_consensus_pubkey(path: &Path) -> Result<Vec<u8>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("zcinti key read refused: {err}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|err| format!("zcinti key json refused: {err}"))?;
    let encoded = value
        .get("pub_key")
        .and_then(|key| key.get("value"))
        .and_then(Value::as_str)
        .ok_or("zcinti key refused: missing pub_key.value")?;
    let raw = STANDARD
        .decode(encoded.trim())
        .map_err(|err| format!("zcinti key refused: {err}"))?;
    if raw.len() != 32 {
        return Err(format!("zcinti key refused: public key is {} bytes", raw.len()));
    }
    Ok(raw)
}

fn consensus_address(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    hex_encode(&digest[..20])
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_key_file_exposes_only_the_public_half() {
        let path = std::env::temp_dir().join(format!("zcinti-key-{}.json", std::process::id()));
        let made = generate_consensus_key(&path).unwrap();
        let read = read_consensus_pubkey(&path).unwrap();
        assert_eq!(STANDARD.encode(&read), made.pubkey_b64);
        assert_eq!(made.address_hex.len(), 40);
        let file = fs::read_to_string(&path).unwrap();
        assert!(file.contains("tendermint/PrivKeyEd25519"));
        assert!(!made.pubkey_b64.is_empty());
        let _ = fs::remove_file(path);
    }
}
