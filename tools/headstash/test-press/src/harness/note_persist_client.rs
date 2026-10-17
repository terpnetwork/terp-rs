//! Product call site: after BridgeMintNote / claim, encrypt SEAM → put note.
//!
//! **SSOT store:** hash-market `HeadstashStore` (`PUT /notes/{hs_id}/{addr}`).
//! This module is the mint/bridge **client** that holds plaintext; the server
//! never re-derives secrets.
//!
//! # Config
//! - `hs_id` — contract bech32 or season slug
//! - `notes_base` — HTTP base of unified hash-market-server (optional)
//! - `data_dir` — when `notes_base` is `None`, write a HeadstashStore-shaped
//!   tree under `data_dir` (L2 smoke without Docker)
//! - `owner_key` — 32-byte AEAD key (wallet-derived in production)
//!
//! # Flow
//! ```text
//! BridgeMintNote success (chain)
//!   → client still holds SeamNoteOutV0 / openings
//!   → put_note_after_mint(note, &cfg)
//!   → encrypt 382B → envelope → PUT or local notes/{hs_id}/{addr}.json
//! ```
//!
//! Feature `l0-seams` required (links `terp_seams::dex::note` + compose helpers).

#[cfg(feature = "l0-seams")]
use super::note_persist_l0::{MiniNoteStore, NoteEnvelope as LocalEnvelope};
use super::L0Error;
use std::path::PathBuf;

/// Mint-client note persistence config (`notes_base` + keys).
#[derive(Clone, Debug)]
pub struct NotesPersistConfig {
    /// Headstash instance id: season slug or cw-headstash contract address.
    pub hs_id: String,
    /// Client AEAD key — never sent to the server.
    pub owner_key: [u8; 32],
    /// When set: `http(s)://host:port` for hash-market PUT/GET.
    pub notes_base: Option<String>,
    /// When `notes_base` is None: local data dir mirroring HeadstashStore layout.
    pub data_dir: Option<PathBuf>,
    /// Auth headers for HTTP (blossom Bearer / X-Auth-Type secp headers).
    pub auth_headers: Vec<(String, String)>,
    /// When true, attach envelope `sha256` of ciphertext (distribution on).
    pub include_ciphertext_sha256: bool,
}

impl NotesPersistConfig {
    /// Season slug + in-process store under a temp-ish path (tests).
    pub fn local_season(hs_id: impl Into<String>, owner_key: [u8; 32], data_dir: PathBuf) -> Self {
        Self {
            hs_id: hs_id.into(),
            owner_key,
            notes_base: None,
            data_dir: Some(data_dir),
            auth_headers: Vec::new(),
            include_ciphertext_sha256: false,
        }
    }

    /// HTTP mode against a running hash-market notes host.
    pub fn http(
        hs_id: impl Into<String>,
        owner_key: [u8; 32],
        notes_base: impl Into<String>,
        auth_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            hs_id: hs_id.into(),
            owner_key,
            notes_base: Some(notes_base.into()),
            data_dir: None,
            auth_headers,
            include_ciphertext_sha256: false,
        }
    }

    /// Enable optional ciphertext `sha256` on put (when distribution dual-index desired).
    pub fn with_ciphertext_sha256(mut self, on: bool) -> Self {
        self.include_ciphertext_sha256 = on;
        self
    }

    /// Build from env for corridor / lab film (PIR-3).
    ///
    /// | Env | Role |
    /// |-----|------|
    /// | `NOTES_BASE` | When set → HTTP host; else local `data_dir` |
    /// | `NOTES_HS_ID` | Default `corridor-post-mint` |
    /// | `NOTES_BEARER_TOKEN` | Optional `Authorization: Bearer` (modular notes_auth `bearer`) |
    /// | `NOTES_OWNER_KEY` | 64-hex 32B; default lab `[0xC0; 32]` |
    ///
    /// Auth stays **existing modular notes_auth** (bearer/secp/noop) — not light-client
    /// or Nostr until those modules ship. Client only attaches headers; server verifies.
    pub fn from_env_or_local(data_dir: PathBuf) -> Result<Self, L0Error> {
        let hs_id = std::env::var("NOTES_HS_ID").unwrap_or_else(|_| "corridor-post-mint".into());
        let owner_key = parse_owner_key_env().unwrap_or([0xC0; 32]);
        let mut auth_headers = Vec::new();
        if let Ok(tok) = std::env::var("NOTES_BEARER_TOKEN") {
            if !tok.is_empty() {
                auth_headers.push(("Authorization".into(), format!("Bearer {tok}")));
            }
        }
        if let Ok(base) = std::env::var("NOTES_BASE") {
            let base = base.trim().to_string();
            if !base.is_empty() {
                return Ok(Self::http(hs_id, owner_key, base, auth_headers));
            }
        }
        Ok(Self {
            hs_id,
            owner_key,
            notes_base: None,
            data_dir: Some(data_dir),
            auth_headers,
            include_ciphertext_sha256: false,
        })
    }
}

fn parse_owner_key_env() -> Option<[u8; 32]> {
    let s = std::env::var("NOTES_OWNER_KEY").ok()?;
    let b = hex::decode(s.trim()).ok()?;
    if b.len() != 32 {
        return None;
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&b);
    Some(k)
}

/// Receipt from a successful client put (path for snap/PIR list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotePersistReceipt {
    pub hs_id: String,
    pub addr: String,
    /// Relative `notes/{hs_id}/{addr}` (no `.json`).
    pub store_path: String,
}

/// **Product call site** after successful `BridgeMintNote` / claim.
///
/// Caller must still hold cleartext [`terp_seams::dex::note::SeamNoteOutV0`] (rcm /
/// openings). Encrypts full 382B and writes to HeadstashStore-shaped backend.
#[cfg(feature = "l0-seams")]
pub fn put_note_after_mint(
    note: &terp_seams::dex::note::SeamNoteOutV0,
    cfg: &NotesPersistConfig,
    addr_override: Option<&str>,
) -> Result<NotePersistReceipt, L0Error> {
    use terp_seams::dex::note::{
        note_addr_cm, persist_plan_from_seam_note_opts, EncryptNoteOpts,
    };

    let addr_owned;
    let addr = if let Some(a) = addr_override {
        a
    } else {
        addr_owned = note_addr_cm(&note.cm_public);
        &addr_owned
    };
    let mut plan = persist_plan_from_seam_note_opts(
        note,
        &cfg.owner_key,
        &cfg.hs_id,
        addr,
        EncryptNoteOpts {
            include_ciphertext_sha256: cfg.include_ciphertext_sha256,
        },
    )
    .map_err(|e| L0Error(format!("persist_plan: {e:?}")))?;

    if cfg.include_ciphertext_sha256 && plan.envelope.sha256.is_none() {
        plan.envelope = plan
            .envelope
            .with_ciphertext_sha256()
            .map_err(|e| L0Error(format!("sha256: {e:?}")))?;
    }

    if let Some(base) = cfg.notes_base.as_ref() {
        #[cfg(feature = "note-http")]
        {
            let headers: Vec<(&str, &str)> = cfg
                .auth_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            terp_seams::dex::note::put_note_envelope(base, &plan.hs_id, &plan.addr, &plan.envelope, &headers)
                .map_err(|e| L0Error(format!("put_note_envelope: {e:?}")))?;
        }
        #[cfg(not(feature = "note-http"))]
        {
            let _ = base;
            return Err(L0Error(
                "notes_base set but feature `note-http` disabled (enable for live PUT)".into(),
            ));
        }
    } else {
        let dir = cfg.data_dir.as_ref().ok_or_else(|| {
            L0Error("NotesPersistConfig: set notes_base or data_dir".into())
        })?;
        let store = MiniNoteStore::new(dir).map_err(|e| L0Error(e.0))?;
        let json = plan
            .envelope
            .to_json_value()
            .map_err(|e| L0Error(format!("envelope json: {e:?}")))?;
        let local = LocalEnvelope::from_json(&json).map_err(|e| L0Error(e.0))?;
        store
            .set_note(&plan.hs_id, &plan.addr, &local)
            .map_err(|e| L0Error(e.0))?;
    }

    Ok(NotePersistReceipt {
        hs_id: plan.hs_id,
        addr: plan.addr,
        store_path: plan.store_path,
    })
}

/// Fetch envelope and decrypt with client key (owner recover path / L2 assert).
#[cfg(feature = "l0-seams")]
pub fn get_and_decrypt_note(
    cfg: &NotesPersistConfig,
    addr: &str,
) -> Result<terp_seams::dex::note::SeamNoteOutV0, L0Error> {
    use terp_seams::dex::note::{decrypt_note_out, NoteEnvelope};

    let env = if let Some(base) = cfg.notes_base.as_ref() {
        #[cfg(feature = "note-http")]
        {
            let headers: Vec<(&str, &str)> = cfg
                .auth_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            terp_seams::dex::note::get_note_envelope(base, &cfg.hs_id, addr, &headers)
                .map_err(|e| L0Error(format!("get_note_envelope: {e:?}")))?
        }
        #[cfg(not(feature = "note-http"))]
        {
            let _ = base;
            return Err(L0Error(
                "notes_base set but feature `note-http` disabled".into(),
            ));
        }
    } else {
        let dir = cfg.data_dir.as_ref().ok_or_else(|| {
            L0Error("NotesPersistConfig: set notes_base or data_dir".into())
        })?;
        let store = MiniNoteStore::new(dir).map_err(|e| L0Error(e.0))?;
        let local = store
            .get_note(&cfg.hs_id, addr)
            .map_err(|e| L0Error(e.0))?
            .ok_or_else(|| L0Error(format!("note missing at notes/{}/{addr}", cfg.hs_id)))?;
        NoteEnvelope::from_json_value(&local.to_json())
            .map_err(|e| L0Error(format!("envelope parse: {e:?}")))?
    };

    decrypt_note_out(&env, &cfg.owner_key).map_err(|e| L0Error(format!("decrypt: {e:?}")))
}

/// L2 smoke (no Docker): client SEAM from pure bridge mint → put → get → decrypt.
///
/// When combined with Mock `BridgeMintNote`, call this **after** chain mint with
/// the same client-held note material (plaintext never on chain).
#[cfg(feature = "l0-seams")]
pub fn l2_smoke_put_get_decrypt(
    note: &terp_seams::dex::note::SeamNoteOutV0,
    cfg: &NotesPersistConfig,
) -> Result<NotePersistReceipt, L0Error> {
    let receipt = put_note_after_mint(note, cfg, None)?;
    let recovered = get_and_decrypt_note(cfg, &receipt.addr)?;
    if recovered.to_bytes() != note.to_bytes() {
        return Err(L0Error(
            "l2 smoke: decrypt does not match SEAM cleartext".into(),
        ));
    }
    Ok(receipt)
}

/// Snap recover UX: list note addrs under `hs_id` (HTTP only).
#[cfg(all(feature = "l0-seams", feature = "note-http"))]
pub fn list_note_addrs(cfg: &NotesPersistConfig) -> Result<Vec<String>, L0Error> {
    let base = cfg.notes_base.as_ref().ok_or_else(|| {
        L0Error("list_note_addrs requires notes_base (live host)".into())
    })?;
    let headers: Vec<(&str, &str)> = cfg
        .auth_headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    terp_seams::dex::note::list_note_keys(base, &cfg.hs_id, &headers)
        .map_err(|e| L0Error(format!("list_note_keys: {e:?}")))
}

/// Snap recover UX: PIR-fetch envelope then decrypt (HTTP only).
#[cfg(all(feature = "l0-seams", feature = "note-http"))]
pub fn recover_note_via_pir(
    cfg: &NotesPersistConfig,
    addr: &str,
) -> Result<terp_seams::dex::note::SeamNoteOutV0, L0Error> {
    use terp_seams::dex::note::decrypt_note_out;
    let base = cfg.notes_base.as_ref().ok_or_else(|| {
        L0Error("recover_note_via_pir requires notes_base".into())
    })?;
    let headers: Vec<(&str, &str)> = cfg
        .auth_headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let env = terp_seams::dex::note::pir_get_note_envelope(base, &cfg.hs_id, addr, &headers)
        .map_err(|e| L0Error(format!("pir_get: {e:?}")))?;
    decrypt_note_out(&env, &cfg.owner_key).map_err(|e| L0Error(format!("decrypt: {e:?}")))
}

/// Alias for owner GET+decrypt (snap recover happy path).
#[cfg(feature = "l0-seams")]
pub fn recover_note(
    cfg: &NotesPersistConfig,
    addr: &str,
) -> Result<terp_seams::dex::note::SeamNoteOutV0, L0Error> {
    get_and_decrypt_note(cfg, addr)
}

/// Corridor film recovery: prefer **PIR** when `notes_base` is set (+ `note-http`);
/// otherwise direct GET decrypt (offline / local store).
///
/// Product path for multi-device sync; direct recover remains default offline.
#[cfg(feature = "l0-seams")]
pub fn recover_note_film(
    cfg: &NotesPersistConfig,
    addr: &str,
) -> Result<terp_seams::dex::note::SeamNoteOutV0, L0Error> {
    if cfg.notes_base.is_some() {
        #[cfg(feature = "note-http")]
        {
            return recover_note_via_pir(cfg, addr);
        }
        #[cfg(not(feature = "note-http"))]
        {
            return Err(L0Error(
                "NOTES_BASE set but feature `note-http` disabled — enable for PIR film".into(),
            ));
        }
    }
    recover_note(cfg, addr)
}

/// Put after mint then film recover (PIR when HTTP, direct when local).
#[cfg(feature = "l0-seams")]
pub fn put_and_film_recover_after_mint(
    note: &terp_seams::dex::note::SeamNoteOutV0,
    cfg: &NotesPersistConfig,
) -> Result<NotePersistReceipt, L0Error> {
    let receipt = put_note_after_mint(note, cfg, None)?;
    let recovered = recover_note_film(cfg, &receipt.addr)?;
    if recovered.to_bytes() != note.to_bytes() {
        return Err(L0Error(
            "film recover: decrypt does not match SEAM cleartext".into(),
        ));
    }
    Ok(receipt)
}

#[cfg(all(test, feature = "l0-seams"))]
mod tests {
    use super::*;
    use crate::harness::compose_bridge_mint_to_seam_bytes;
    use terp_seams::dex::note::SeamNoteOutV0;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("note-persist-client-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn product_call_site_put_get_decrypt_local() {
        let sketch = compose_bridge_mint_to_seam_bytes().expect("compose");
        let note = SeamNoteOutV0::from_bytes(&sketch.to_seam_bytes()).expect("seam");
        let dir = tmp();
        let cfg = NotesPersistConfig::local_season("season-1", [7u8; 32], dir.clone());
        let receipt = l2_smoke_put_get_decrypt(&note, &cfg).expect("L2 smoke");
        assert!(receipt.store_path.starts_with("notes/season-1/cm."));
        assert!(dir
            .join("notes")
            .join("season-1")
            .join(format!("{}.json", receipt.addr))
            .is_file());
        let recovered = recover_note(&cfg, &receipt.addr).expect("recover");
        assert_eq!(recovered.to_bytes(), note.to_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// PIR-3: film recover on local store uses direct path (no NOTES_BASE).
    #[test]
    fn film_recover_local_is_direct_path() {
        let sketch = compose_bridge_mint_to_seam_bytes().expect("compose");
        let note = SeamNoteOutV0::from_bytes(&sketch.to_seam_bytes()).expect("seam");
        let dir = tmp();
        let cfg = NotesPersistConfig::local_season("season-film", [0xC0; 32], dir.clone());
        let receipt = put_and_film_recover_after_mint(&note, &cfg).expect("film local");
        assert!(receipt.store_path.contains("season-film"));
        let recovered = recover_note_film(&cfg, &receipt.addr).expect("film again");
        assert_eq!(recovered.to_bytes(), note.to_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn product_call_site_with_ciphertext_sha256() {
        let sketch = compose_bridge_mint_to_seam_bytes().expect("compose");
        let note = SeamNoteOutV0::from_bytes(&sketch.to_seam_bytes()).expect("seam");
        let dir = tmp();
        let cfg = NotesPersistConfig::local_season("season-1", [9u8; 32], dir.clone())
            .with_ciphertext_sha256(true);
        let receipt = put_note_after_mint(&note, &cfg, None).expect("put");
        let path = dir
            .join("notes")
            .join("season-1")
            .join(format!("{}.json", receipt.addr));
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v.get("sha256").and_then(|s| s.as_str()).is_some());
        assert_eq!(v["sha256"].as_str().unwrap().len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
