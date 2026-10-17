//! L0 design-usage tests: bridge note → opaque envelope → addr `cm.<hex>` persist.
//!
//! Opaque dummy envelopes exercise store rules without crypto. With feature
//! `l0-seams`, one test uses production `terp_seams::dex::note::encrypt_note_out` /
//! `decrypt_note_out` (XChaCha20-Poly1305) then MiniNoteStore round-trip.
//!
//! In-process mini-store mirrors `HeadstashStore` path layout
//! `notes/{hs_id}/{addr}.json` and envelope validation so this crate stays
//! Docker-free and does not pull the terp-rs `hash-market` workspace.
//!
//! SSOT production store: `crates/terp-rs/tools/hash-market` (`HeadstashStore`).
//!
//! ```bash
//! cargo test -p zk-test-press --lib note_persist --features l0-seams
//! cargo test -p zk-test-press --lib harness::note_persist_l0
//! ```

use super::bridge_l0::{assert_policy_bridge_mint_happy, compose_bridge_mint_to_seam_bytes, L0Error};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// SEAM-NOTE-OUT V0 cleartext length (locked).
pub const SEAM_NOTE_OUT_LEN: usize = 382;

/// Opaque encrypted-note envelope (server-side SSOT fields).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteEnvelope {
    pub ciphertext: String,
    pub nonce: String,
    pub scheme: String,
    pub cleartext_layout: Option<String>,
    pub cleartext_len: Option<u64>,
    /// Optional SHA256 of ciphertext bytes (distribution dual-index pointer).
    pub sha256: Option<String>,
}

impl NoteEnvelope {
    /// Test-only envelope: dummy ciphertext over a known cleartext length.
    /// Prefer this until Agent A lands real XChaCha so harness stays green.
    pub fn opaque_dummy(cleartext_len: usize) -> Self {
        Self {
            // Distinct dummy body so overwrite/round-trip asserts are meaningful.
            ciphertext: format!("dummy-ct-{}", hex::encode(&[cleartext_len as u8; 8])),
            nonce: "00112233445566778899aabbccddeeff".into(),
            scheme: "xchacha20poly1305".into(),
            cleartext_layout: Some("SEAM-NOTE-OUT-V0".into()),
            cleartext_len: Some(cleartext_len as u64),
            sha256: None,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("ciphertext".into(), Value::String(self.ciphertext.clone()));
        m.insert("nonce".into(), Value::String(self.nonce.clone()));
        m.insert("scheme".into(), Value::String(self.scheme.clone()));
        if let Some(ref layout) = self.cleartext_layout {
            m.insert("cleartext_layout".into(), Value::String(layout.clone()));
        }
        if let Some(len) = self.cleartext_len {
            m.insert("cleartext_len".into(), Value::Number(len.into()));
        }
        if let Some(ref sha) = self.sha256 {
            m.insert("sha256".into(), Value::String(sha.clone()));
        }
        Value::Object(m)
    }

    pub fn from_json(v: &Value) -> Result<Self, L0Error> {
        let obj = v
            .as_object()
            .ok_or_else(|| L0Error("envelope must be object".into()))?;
        let req = |k: &str| -> Result<String, L0Error> {
            match obj.get(k).and_then(|x| x.as_str()) {
                Some(s) if !s.is_empty() => Ok(s.to_string()),
                _ => Err(L0Error(format!(
                    "note envelope missing non-empty string field `{k}`"
                ))),
            }
        };
        Ok(Self {
            ciphertext: req("ciphertext")?,
            nonce: req("nonce")?,
            scheme: req("scheme")?,
            cleartext_layout: obj
                .get("cleartext_layout")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            cleartext_len: obj.get("cleartext_len").and_then(|x| x.as_u64()),
            sha256: obj
                .get("sha256")
                .and_then(|x| x.as_str())
                .map(str::to_string),
        })
    }
}

/// Suggested bridge note key: `cm.` + hex of commitment-like field (`cm_public`).
pub fn bridge_note_addr_from_cm_public(cm_public: &[u8; 32]) -> String {
    format!("cm.{}", hex::encode(cm_public))
}

/// Validate id charset (mirrors hash-market `validate_id`).
pub fn validate_id(id: &str) -> Result<(), L0Error> {
    if id.is_empty() {
        return Err(L0Error("ID must not be empty".into()));
    }
    if id.len() > 200 {
        return Err(L0Error("ID too long".into()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(L0Error(format!("ID contains invalid characters: {id}")));
    }
    if id.contains("..") {
        return Err(L0Error("ID must not contain '..'".into()));
    }
    Ok(())
}

/// Minimal file store: `notes/{hs_id}/{addr}.json` (design path only).
pub struct MiniNoteStore {
    data_dir: PathBuf,
}

impl MiniNoteStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self, L0Error> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir).map_err(|e| L0Error(e.to_string()))?;
        Ok(Self { data_dir })
    }

    fn note_path(&self, hs_id: &str, addr: &str) -> Result<PathBuf, L0Error> {
        validate_id(hs_id)?;
        validate_id(addr)?;
        Ok(self
            .data_dir
            .join("notes")
            .join(hs_id)
            .join(format!("{addr}.json")))
    }

    pub fn set_note(&self, hs_id: &str, addr: &str, env: &NoteEnvelope) -> Result<(), L0Error> {
        // Re-validate via JSON path (store rules)
        let v = env.to_json();
        let _ = NoteEnvelope::from_json(&v)?;
        let path = self.note_path(hs_id, addr)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| L0Error(e.to_string()))?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap())
            .map_err(|e| L0Error(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| L0Error(e.to_string()))?;
        Ok(())
    }

    pub fn get_note(&self, hs_id: &str, addr: &str) -> Result<Option<NoteEnvelope>, L0Error> {
        let path = self.note_path(hs_id, addr)?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| L0Error(e.to_string()))?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| L0Error(e.to_string()))?;
        Ok(Some(NoteEnvelope::from_json(&v)?))
    }
}

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("zk-test-press-note-persist-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// After pure bridge L0 happy path: derive `cm.` addr and round-trip envelope.
pub fn assert_bridge_note_persist_l0() -> Result<(), L0Error> {
    let note = assert_policy_bridge_mint_happy()?;
    let seam = note.to_seam_bytes();
    if seam.len() != SEAM_NOTE_OUT_LEN {
        return Err(L0Error(format!(
            "expected {SEAM_NOTE_OUT_LEN} seam bytes, got {}",
            seam.len()
        )));
    }

    let addr = bridge_note_addr_from_cm_public(&note.cm_public);
    if !addr.starts_with("cm.") {
        return Err(L0Error(format!("bridge addr must use cm. prefix: {addr}")));
    }
    validate_id(&addr)?;

    // hs_id: season slug (production also allows contract bech32)
    let hs_id = "season-1";
    validate_id(hs_id)?;

    let env = NoteEnvelope::opaque_dummy(SEAM_NOTE_OUT_LEN);
    if env.cleartext_len != Some(SEAM_NOTE_OUT_LEN as u64) {
        return Err(L0Error("cleartext_len must be 382".into()));
    }

    let dir = tmp_dir("bridge-l0");
    let store = MiniNoteStore::new(&dir)?;
    store.set_note(hs_id, &addr, &env)?;
    let got = store
        .get_note(hs_id, &addr)?
        .ok_or_else(|| L0Error("note missing after set".into()))?;
    if got != env {
        return Err(L0Error(format!(
            "round-trip mismatch: got={got:?} want={env:?}"
        )));
    }

    // Path SSOT
    let path = dir.join("notes").join(hs_id).join(format!("{addr}.json"));
    if !path.is_file() {
        return Err(L0Error(format!("missing file {}", path.display())));
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Compose sketch (382B) + envelope metadata lock without real crypto.
pub fn assert_compose_seam_envelope_meta() -> Result<(), L0Error> {
    let note = compose_bridge_mint_to_seam_bytes()?;
    let bytes = note.to_seam_bytes();
    if bytes.len() != SEAM_NOTE_OUT_LEN {
        return Err(L0Error(format!("seam len {}", bytes.len())));
    }
    let env = NoteEnvelope::opaque_dummy(bytes.len());
    let v = env.to_json();
    if v["cleartext_layout"] != "SEAM-NOTE-OUT-V0" {
        return Err(L0Error("cleartext_layout lock".into()));
    }
    if v["cleartext_len"].as_u64() != Some(SEAM_NOTE_OUT_LEN as u64) {
        return Err(L0Error("cleartext_len lock 382".into()));
    }
    if v["scheme"] != "xchacha20poly1305" {
        return Err(L0Error("scheme lock".into()));
    }
    // Reject empty ciphertext locally (store rule)
    let bad = serde_json::json!({
        "ciphertext": "",
        "nonce": "aa",
        "scheme": "xchacha20poly1305"
    });
    if NoteEnvelope::from_json(&bad).is_ok() {
        return Err(L0Error("empty ciphertext must fail".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_persist_bridge_l0_cm_addr_round_trip() {
        assert_bridge_note_persist_l0().expect("bridge note persist L0");
    }

    #[test]
    fn note_persist_compose_seam_envelope_meta() {
        assert_compose_seam_envelope_meta().expect("envelope meta");
    }

    #[test]
    fn note_persist_validate_id_rejects_path_chars() {
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("has space").is_err());
        assert!(validate_id("..").is_err());
        assert!(validate_id("foo..bar").is_err());
        assert!(validate_id("cm.deadbeef").is_ok());
        assert!(validate_id("season-1").is_ok());
    }

    #[test]
    fn note_persist_addr_from_cm_public_hex() {
        let mut cm = [0u8; 32];
        cm[0] = 0xab;
        cm[31] = 0xcd;
        let addr = bridge_note_addr_from_cm_public(&cm);
        assert_eq!(addr, format!("cm.{}", hex::encode(cm)));
        assert!(addr.starts_with("cm."));
        validate_id(&addr).unwrap();
    }

    #[cfg(feature = "l0-seams")]
    #[test]
    fn note_persist_l0_seams_decode_then_envelope() {
        use terp_seams::dex::note::SeamNoteOutV0;
        let note = compose_bridge_mint_to_seam_bytes().expect("compose");
        let bytes = note.to_seam_bytes();
        let seam = SeamNoteOutV0::from_bytes(&bytes).expect("decode seam");
        assert_eq!(bytes.len(), SEAM_NOTE_OUT_LEN);
        // Opaque path still valid for store-only scenarios
        let env = NoteEnvelope::opaque_dummy(SEAM_NOTE_OUT_LEN);
        let hs = "terp1dummyheadstashcontract000000000001";
        let addr = bridge_note_addr_from_cm_public(&note.cm_public);
        let dir = tmp_dir("l0-seams");
        let store = MiniNoteStore::new(&dir).unwrap();
        store.set_note(hs, &addr, &env).unwrap();
        let got = store.get_note(hs, &addr).unwrap().unwrap();
        assert_eq!(got.cleartext_len, Some(382));
        assert_eq!(got.cleartext_layout.as_deref(), Some("SEAM-NOTE-OUT-V0"));
        assert_ne!(seam.cm_public, [0u8; 32]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real XChaCha encrypt → mini-store → decrypt recovers exact SEAM-NOTE-OUT.
    #[cfg(feature = "l0-seams")]
    #[test]
    fn note_persist_l0_seams_encrypt_store_decrypt_roundtrip() {
        use terp_seams::dex::note::{
            decrypt_note_out, encrypt_note_out, note_addr_cm, persist_plan_from_seam_note,
            SeamNoteOutV0,
        };
        let note_sketch = compose_bridge_mint_to_seam_bytes().expect("compose");
        let bytes = note_sketch.to_seam_bytes();
        let seam = SeamNoteOutV0::from_bytes(&bytes).expect("decode");
        let key = [0x42u8; 32]; // test-only key; production is wallet-derived
        let hs = "season-1";
        let addr = note_addr_cm(&seam.cm_public);
        assert_eq!(addr, bridge_note_addr_from_cm_public(&note_sketch.cm_public));

        let plan = persist_plan_from_seam_note(&seam, &key, hs, &addr).expect("plan");
        assert_eq!(plan.store_path, format!("notes/{hs}/{addr}"));
        assert_eq!(plan.envelope.scheme, "xchacha20poly1305");
        assert_eq!(plan.envelope.cleartext_len, 382);

        let dir = tmp_dir("encrypt-rt");
        let store = MiniNoteStore::new(&dir).unwrap();
        let json = plan.envelope.to_json_value().expect("json");
        let env = NoteEnvelope::from_json(&json).expect("env");
        store.set_note(hs, &addr, &env).unwrap();
        let got = store.get_note(hs, &addr).unwrap().unwrap();
        let recovered = decrypt_note_out(
            &terp_seams::dex::note::NoteEnvelope::from_json_value(&got.to_json()).expect("parse"),
            &key,
        )
        .expect("decrypt");
        assert_eq!(recovered.to_bytes(), seam.to_bytes());
        let env2 = encrypt_note_out(&seam, &key).unwrap();
        assert_eq!(
            decrypt_note_out(&env2, &key).unwrap().to_bytes(),
            seam.to_bytes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
