//! Deterministic oracle-attestation aggregate for GET /vote-extension.
//!
//! Preimage matches `x/hashmerchant/keeper.aggregateAttestationRoot`:
//! sort by (source_id, value, height, timestamp), dedup source_id (first
//! after sort), SHA-256 of `source_id || value_bytes || height_le`.
//! `value` in JSON is hex (Go hex-decodes before hashing).

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attestation {
    pub source_id: String,
    /// Hex-encoded payload (same as Go `sidecarAttestationJSON.value`).
    pub value: String,
    #[serde(default)]
    pub height: u64,
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custody_signature: String,
}

fn value_bytes(hex_or_raw: &str) -> Vec<u8> {
    let t = hex_or_raw.trim();
    if t.len().is_multiple_of(2)
        && !t.is_empty()
        && t.chars().all(|c| c.is_ascii_hexdigit())
        && let Ok(b) = decode_hex(t)
    {
        return b;
    }
    t.as_bytes().to_vec()
}

fn decode_hex(s: &str) -> Result<Vec<u8>, ()> {
    if !s.len().is_multiple_of(2) {
        return Err(());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

pub fn sort_attestations(atts: &mut [Attestation]) {
    atts.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| value_bytes(&a.value).cmp(&value_bytes(&b.value)))
            .then_with(|| a.height.cmp(&b.height))
            .then_with(|| a.timestamp.cmp(&b.timestamp))
    });
}

pub fn dedup_source_id(sorted: Vec<Attestation>) -> Vec<Attestation> {
    let mut out = Vec::with_capacity(sorted.len());
    for a in sorted {
        if out.last().map(|p: &Attestation| p.source_id == a.source_id) == Some(true) {
            continue;
        }
        out.push(a);
    }
    out
}

pub fn aggregate_attestation_root(mut atts: Vec<Attestation>) -> [u8; 32] {
    sort_attestations(&mut atts);
    let atts = dedup_source_id(atts);
    let mut h = Sha256::new();
    for a in atts {
        h.update(a.source_id.as_bytes());
        h.update(value_bytes(&a.value));
        h.update(a.height.to_le_bytes());
    }
    h.finalize().into()
}

pub fn load_env_attestations() -> Vec<Attestation> {
    let Ok(raw) = std::env::var("HASHMERCHANT_ATTESTATIONS") else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn present_reversed() -> bool {
    match std::env::var("HASHMERCHANT_ATTESTATION_REVERSE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "desc" => true,
        "auto" => {
            let host = std::env::var("HOSTNAME").unwrap_or_default();
            host.ends_with("-1") || host.ends_with("_1") || host.contains("-val-1")
        }
        _ => false,
    }
}

/// `(present_list, root_hex)` when `HASHMERCHANT_ATTESTATIONS` is set.
pub fn env_attestation_payload() -> Option<(Vec<Attestation>, String)> {
    let mut atts = load_env_attestations();
    if atts.is_empty() {
        return None;
    }
    let root = hex::encode(aggregate_attestation_root(atts.clone()));
    sort_attestations(&mut atts);
    atts = dedup_source_id(atts);
    if present_reversed() {
        atts.reverse();
    }
    Some((atts, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Attestation> {
        vec![
            Attestation {
                source_id: "src-zeta".into(),
                value: "313030".into(),
                height: 42,
                timestamp: 1700000002,
                custody_signature: String::new(),
            },
            Attestation {
                source_id: "src-alpha".into(),
                value: "3939".into(),
                height: 41,
                timestamp: 1700000001,
                custody_signature: String::new(),
            },
        ]
    }

    #[test]
    fn reverse_order_same_root() {
        let a = fixture();
        let mut b = a.clone();
        b.reverse();
        assert_ne!(a[0].source_id, b[0].source_id);
        assert_eq!(aggregate_attestation_root(a), aggregate_attestation_root(b));
    }

    #[test]
    fn sort_is_source_id_first() {
        let mut atts = fixture();
        sort_attestations(&mut atts);
        assert_eq!(atts[0].source_id, "src-alpha");
    }

    #[test]
    fn hex_value_matches_go_preimage() {
        // Go hashes decoded 0x3939 / 0x313030 == b"99" / b"100".
        let got = hex::encode(aggregate_attestation_root(fixture()));
        assert_eq!(
            got,
            "a2e9f599d262ed589375679f66d65a5a127b0c10492b54ecf396ce4f63edabd2"
        );
    }
}
