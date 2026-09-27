//! File-based key-value store for headstash data.
//!
//! Layout:
//! ```text
//! {data_dir}/
//!   headstash/{id}.json          — headstash registration records
//!   notes/{hs_id}/{addr}.json    — encrypted notes
//!   keys/{key_id}.bin            — circuit key binaries
//!   keys/{key_id}.blake3         — BLAKE3 hash of the key file
//! File-based tree storage.  One JSON file per tree_id under data_dir.
//!
//! Upload format mirrors the gen_merkle script output:
//!   { "merkle_root": "<hex>", "accounts": { "<addr>": { "tier": N, "allocation": N, "proof_hashes": [...] } } }
//!
//! Stored format (on disk):
//!   { "root": "<hex>", "members": { "<addr>": { "allocation": N, "tier": N, "proof_hashes": [...] } }, ... }
//! ```
//!
//! # Blob Storage (/f/ prefix with content-ID index)
//!
//! Implements the `BlobStore` trait from cw721-nips using the `/f/` key prefix
//! for content-addressed file storage. A QMD-style content-ID index (`ContentIndex`)
//! layers SHA256 hash → file metadata on top of the file store so lookups are
//! O(1) in memory while blobs are durable on disk.
//!
//! This is the sole `BlobStore` implementation — `HashMarketBlobStore` and
//! `QmdbBlobStore` have been removed. All blob storage (BUD, blossom, headstash)
//! delegates through `TreeStore`.

use anyhow::bail;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

// Re-export from nips (BUD-02)
use cw721_nips::buds::{BlobDescriptor, BlobStore, BlobStoreError};

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ── QMD Content-ID Index ─────────────────────────────────────────────────────
//
// Lightweight content-addressable index layered on top of the `/f/` prefix file
// store. Maps SHA256 content hash → file metadata. Synced with disk on startup
// and updated atomically on put/delete.
//
// This mirrors the QMDB pattern: an authenticated key-value layer where the key
// is the content hash and the value is blob metadata. The actual blob bytes live
// on disk at `/f/{hex_hash}`.

/// Metadata tracked in the content-ID index.
#[derive(Debug, Clone)]
pub struct ContentEntry {
    /// Size in bytes.
    pub size: u64,
    /// Unix timestamp of upload.
    pub uploaded: u64,
}

/// QMD-style content-ID index — SHA256 hash → file metadata.
///
/// Synced with the `/f/` directory on disk on first access. Updates are
/// transactional: the index write happens only after the file write succeeds.
#[derive(Clone)]
pub struct ContentIndex {
    /// SHA256 hash → metadata
    index: Arc<RwLock<HashMap<[u8; 32], ContentEntry>>>,
    /// Path to the `/f/` directory
    f_dir: PathBuf,
    /// Whether we've synced from disk
    synced: Arc<std::sync::atomic::AtomicBool>,
}

impl ContentIndex {
    fn new(f_dir: PathBuf) -> Self {
        Self {
            index: Arc::new(RwLock::new(HashMap::new())),
            f_dir,
            synced: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Ensure the index is populated from disk on first use.
    fn ensure_synced(&self) {
        if self.synced.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let mut idx = self.index.write().unwrap();
        if self.synced.load(std::sync::atomic::Ordering::Acquire) {
            return; // double-check
        }
        // Scan the /f/ directory and populate the index
        if let Ok(rd) = fs::read_dir(&self.f_dir) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name_str = match name.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                let hash: Option<[u8; 32]> =
                    hex::decode(name_str).ok().and_then(|b| b.try_into().ok());
                if let Some(hash) = hash {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let uploaded = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.created().ok())
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    idx.insert(hash, ContentEntry { size, uploaded });
                }
            }
        }
        self.synced
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Get entry by hash.
    pub(crate) fn get(&self, hash: &[u8; 32]) -> Option<ContentEntry> {
        self.ensure_synced();
        self.index.read().unwrap().get(hash).cloned()
    }

    /// Insert entry.
    fn insert(&self, hash: [u8; 32], entry: ContentEntry) {
        self.ensure_synced();
        self.index.write().unwrap().insert(hash, entry);
    }

    /// Remove entry.
    fn remove(&self, hash: &[u8; 32]) -> Option<ContentEntry> {
        self.ensure_synced();
        self.index.write().unwrap().remove(hash)
    }

    /// Check if hash exists.
    fn contains(&self, hash: &[u8; 32]) -> bool {
        self.ensure_synced();
        self.index.read().unwrap().contains_key(hash)
    }

    /// List all hashes.
    fn keys(&self) -> Vec<[u8; 32]> {
        self.ensure_synced();
        self.index.read().unwrap().keys().copied().collect()
    }
}

// ── File-based store for headstash data ──────────────────────────────────────

/// File-based store for headstash data.
pub struct HeadstashStore {
    data_dir: PathBuf,
}

impl HeadstashStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let store = Self {
            data_dir: data_dir.to_path_buf(),
        };
        // Ensure directories exist
        std::fs::create_dir_all(store.headstash_dir())?;
        std::fs::create_dir_all(store.keys_dir())?;
        Ok(store)
    }

    fn headstash_dir(&self) -> PathBuf {
        self.data_dir.join("headstash")
    }

    fn notes_dir(&self, hs_id: &str) -> PathBuf {
        self.data_dir.join("notes").join(hs_id)
    }

    fn keys_dir(&self) -> PathBuf {
        self.data_dir.join("keys")
    }

    /// Data directory root (`notes/`, `pir/`, `headstash/`).
    pub fn base_dir(&self) -> &Path {
        &self.data_dir
    }

    // ── Headstash records ────────────────────────────────────────────

    pub fn get_headstash(&self, id: &str) -> Result<Option<serde_json::Value>> {
        validate_id(id)?;
        let path = self.headstash_dir().join(format!("{id}.json"));
        read_json_opt(&path)
    }

    pub fn set_headstash(&self, id: &str, data: &serde_json::Value) -> Result<()> {
        validate_id(id)?;
        let path = self.headstash_dir().join(format!("{id}.json"));
        write_json_atomic(&path, data)
    }

    pub fn get_headstash_root(&self, id: &str) -> Result<Option<String>> {
        validate_id(id)?;
        let path = self.headstash_dir().join(format!("{id}.json"));
        match read_json_opt::<serde_json::Value>(&path)? {
            Some(v) => Ok(v.get("root").and_then(|r| r.as_str()).map(String::from)),
            None => Ok(None),
        }
    }

    // ── Notes ────────────────────────────────────────────────────────

    pub fn get_note(&self, hs_id: &str, addr: &str) -> Result<Option<serde_json::Value>> {
        validate_id(hs_id)?;
        validate_id(addr)?;
        let path = self.notes_dir(hs_id).join(format!("{addr}.json"));
        read_json_opt(&path)
    }

    /// Atomically write an encrypted note envelope under `notes/{hs_id}/{addr}.json`.
    ///
    /// Envelope is opaque to the store (SEAM cleartext is never parsed here). Expected
    /// production shape (from docs/headstash.md):
    /// ```json
    /// { "ciphertext": "…", "nonce": "…", "scheme": "xchacha20poly1305",
    ///   "cleartext_layout": "SEAM-NOTE-OUT-V0", "cleartext_len": 382 }
    /// ```
    /// Optional `sha256` may reference a dual-index of the **ciphertext only** when
    /// distribution is on — private note bodies stay on this path, not public `/content`.
    pub fn set_note(&self, hs_id: &str, addr: &str, data: &serde_json::Value) -> Result<()> {
        validate_id(hs_id)?;
        validate_id(addr)?;
        validate_note_envelope(data)?;
        let dir = self.notes_dir(hs_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{addr}.json"));
        write_json_atomic(&path, data)
    }

    pub fn list_note_keys(&self, hs_id: &str) -> Result<Vec<String>> {
        validate_id(hs_id)?;
        let dir = self.notes_dir(hs_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                if let Some(key) = name.strip_suffix(".json") {
                    keys.push(key.to_string());
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// Get all note blobs for PIR (padded to equal length).
    pub fn get_note_blobs(&self, hs_id: &str) -> Result<(Vec<String>, Vec<Vec<u8>>)> {
        let keys = self.list_note_keys(hs_id)?;
        let mut blobs = Vec::with_capacity(keys.len());
        for key in &keys {
            let path = self.notes_dir(hs_id).join(format!("{key}.json"));
            let data = std::fs::read(&path).with_context(|| format!("read note {key}"))?;
            blobs.push(data);
        }
        // Pad to max length
        let max_len = blobs.iter().map(|b| b.len()).max().unwrap_or(0);
        for blob in &mut blobs {
            blob.resize(max_len, 0);
        }
        Ok((keys, blobs))
    }

    // ── Circuit keys ─────────────────────────────────────────────────

    pub fn get_key(&self, key_id: &str) -> Result<Option<Vec<u8>>> {
        validate_id(key_id)?;
        let path = self.keys_dir().join(format!("{key_id}.bin"));
        if path.exists() {
            Ok(Some(std::fs::read(&path)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_keys(&self) -> Result<Vec<String>> {
        let dir = self.keys_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                if let Some(key) = name.strip_suffix(".bin") {
                    keys.push(key.to_string());
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// Get all key blobs for PIR (padded to equal length).
    pub fn get_key_blobs(&self) -> Result<(Vec<String>, Vec<Vec<u8>>)> {
        let keys = self.list_keys()?;
        let mut blobs = Vec::with_capacity(keys.len());
        for key in &keys {
            let path = self.keys_dir().join(format!("{key}.bin"));
            let data = std::fs::read(&path).with_context(|| format!("read key {key}"))?;
            blobs.push(data);
        }
        let max_len = blobs.iter().map(|b| b.len()).max().unwrap_or(0);
        for blob in &mut blobs {
            blob.resize(max_len, 0);
        }
        Ok((keys, blobs))
    }
}

// ── Cashu mesh store (mint discovery cache + encrypted wallet stash) ─────────
//
// Layout (never dual-index wallet proofs on public /content):
//   {data_dir}/cashu/mints/{mint_id}.json
//   {data_dir}/cashu/wallets/{wallet_id}/{item_id}.json
//
// Schema SSOT: docs/plans/cashu/CANONICAL-MINT-REGISTRY.md (canonical, not "official").

/// Mint status for the **canonical** discovery registry (mesh cache may lag chain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MintStatus {
    Active,
    Paused,
    Revoked,
}

impl MintStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// `MintDescriptor` v0 — shared with on-chain `cw-cashu-registry` (canonical schema).
///
/// Off-chain mesh may additionally carry `source` (`manual` | `poll` | `nostr` | `chain`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintDescriptor {
    pub mint_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyset_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub nuts: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    #[serde(default = "default_mint_status")]
    pub status: MintStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default)]
    pub registered_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrar: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    /// Mesh-only: how this row was written (`manual` | `poll` | `nostr` | `chain`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn default_mint_status() -> MintStatus {
    MintStatus::Active
}

/// File-based store for Cashu mint discovery cache + encrypted wallet backup.
///
/// Separate from Headstash notes (`notes/…`). Wallet envelopes are opaque ciphertext
/// and must **never** be dual-indexed as public `/content` SSOT.
pub struct CashuMeshStore {
    data_dir: PathBuf,
}

impl CashuMeshStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let store = Self {
            data_dir: data_dir.to_path_buf(),
        };
        std::fs::create_dir_all(store.mints_dir())?;
        std::fs::create_dir_all(store.wallets_root())?;
        Ok(store)
    }

    fn mints_dir(&self) -> PathBuf {
        self.data_dir.join("cashu").join("mints")
    }

    fn wallets_root(&self) -> PathBuf {
        self.data_dir.join("cashu").join("wallets")
    }

    fn wallet_dir(&self, wallet_id: &str) -> PathBuf {
        self.wallets_root().join(wallet_id)
    }

    // ── Mints (canonical discovery cache) ────────────────────────────

    pub fn get_mint(&self, mint_id: &str) -> Result<Option<MintDescriptor>> {
        validate_id(mint_id)?;
        let path = self.mints_dir().join(format!("{mint_id}.json"));
        read_json_opt(&path)
    }

    /// Upsert a mint descriptor. Path `mint_id` wins over body field if both set.
    pub fn set_mint(&self, mint_id: &str, mut desc: MintDescriptor) -> Result<()> {
        validate_id(mint_id)?;
        validate_mint_descriptor(mint_id, &desc)?;
        desc.mint_id = mint_id.to_string();
        if desc.updated_at == 0 {
            desc.updated_at = now_secs();
        }
        if desc.registered_at == 0 {
            desc.registered_at = desc.updated_at;
        }
        if desc.nuts.is_null() {
            desc.nuts = serde_json::json!({});
        }
        if desc.metadata.is_null() {
            desc.metadata = serde_json::json!({});
        }
        let path = self.mints_dir().join(format!("{mint_id}.json"));
        let value = serde_json::to_value(&desc)?;
        write_json_atomic(&path, &value)
    }

    /// Upsert from raw JSON (HTTP body). Injects path `mint_id` into the object.
    pub fn set_mint_json(&self, mint_id: &str, data: &serde_json::Value) -> Result<()> {
        validate_id(mint_id)?;
        let mut obj = data
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("mint descriptor must be a JSON object"))?
            .clone();
        obj.insert(
            "mint_id".into(),
            serde_json::Value::String(mint_id.to_string()),
        );
        if !obj.contains_key("nuts") || obj.get("nuts").map(|v| v.is_null()).unwrap_or(true) {
            obj.insert("nuts".into(), serde_json::json!({}));
        }
        if !obj.contains_key("metadata") || obj.get("metadata").map(|v| v.is_null()).unwrap_or(true)
        {
            obj.insert("metadata".into(), serde_json::json!({}));
        }
        let desc: MintDescriptor = serde_json::from_value(serde_json::Value::Object(obj))
            .map_err(|e| anyhow::anyhow!("invalid mint descriptor: {e}"))?;
        self.set_mint(mint_id, desc)
    }

    pub fn delete_mint(&self, mint_id: &str) -> Result<bool> {
        validate_id(mint_id)?;
        let path = self.mints_dir().join(format!("{mint_id}.json"));
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)?;
        Ok(true)
    }

    /// List mint IDs (sorted). Optional `status_filter` (e.g. `active`) filters descriptors.
    pub fn list_mint_keys(&self) -> Result<Vec<String>> {
        list_json_stem_keys(&self.mints_dir())
    }

    /// List full descriptors; default filter is active-only when `status_filter` is `None`
    /// if `default_active_only` is true, else all.
    pub fn list_mints(
        &self,
        status_filter: Option<&str>,
        default_active_only: bool,
    ) -> Result<Vec<MintDescriptor>> {
        let keys = self.list_mint_keys()?;
        let filter = match status_filter {
            Some(s) => Some(
                MintStatus::parse(s)
                    .ok_or_else(|| anyhow::anyhow!("invalid status filter: {s}"))?,
            ),
            None if default_active_only => Some(MintStatus::Active),
            None => None,
        };
        let mut out = Vec::new();
        for id in keys {
            if let Some(d) = self.get_mint(&id)? {
                if filter.as_ref().map(|f| &d.status == f).unwrap_or(true) {
                    out.push(d);
                }
            }
        }
        Ok(out)
    }

    // ── Wallet encrypted stash (opaque envelopes) ────────────────────

    pub fn get_wallet_item(
        &self,
        wallet_id: &str,
        item_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        validate_id(wallet_id)?;
        validate_id(item_id)?;
        let path = self.wallet_dir(wallet_id).join(format!("{item_id}.json"));
        read_json_opt(&path)
    }

    /// Store an encrypted Cashu wallet backup envelope (opaque to the server).
    ///
    /// Expected production shape (mesh-native, pre-CDK):
    /// ```json
    /// { "ciphertext": "…", "nonce": "…", "scheme": "xchacha20poly1305",
    ///   "cleartext_layout": "CASHU-TOKEN-BACKUP-V0", "cleartext_len": N }
    /// ```
    pub fn set_wallet_item(
        &self,
        wallet_id: &str,
        item_id: &str,
        data: &serde_json::Value,
    ) -> Result<()> {
        validate_id(wallet_id)?;
        validate_id(item_id)?;
        validate_opaque_envelope(data)?;
        let dir = self.wallet_dir(wallet_id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{item_id}.json"));
        write_json_atomic(&path, data)
    }

    pub fn list_wallet_item_keys(&self, wallet_id: &str) -> Result<Vec<String>> {
        validate_id(wallet_id)?;
        list_json_stem_keys(&self.wallet_dir(wallet_id))
    }

    pub fn delete_wallet_item(&self, wallet_id: &str, item_id: &str) -> Result<bool> {
        validate_id(wallet_id)?;
        validate_id(item_id)?;
        let path = self.wallet_dir(wallet_id).join(format!("{item_id}.json"));
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)?;
        Ok(true)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Validate an ID to prevent path traversal.
/// Charset: `[A-Za-z0-9._-]{1,200}`, no `..` (shared with notes + cashu mesh).
pub(crate) fn validate_id(id: &str) -> Result<()> {
    anyhow::ensure!(!id.is_empty(), "ID must not be empty");
    anyhow::ensure!(id.len() <= 200, "ID too long");
    anyhow::ensure!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'),
        "ID contains invalid characters: {id}"
    );
    anyhow::ensure!(!id.contains(".."), "ID must not contain '..'");
    Ok(())
}

/// Require minimal note envelope fields. Cleartext layout is optional metadata.
fn validate_note_envelope(data: &serde_json::Value) -> Result<()> {
    validate_opaque_envelope(data)
}

/// Opaque encrypted envelope: `ciphertext`, `nonce`, `scheme` required non-empty strings.
fn validate_opaque_envelope(data: &serde_json::Value) -> Result<()> {
    let obj = data
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("envelope must be a JSON object"))?;
    for key in ["ciphertext", "nonce", "scheme"] {
        match obj.get(key) {
            Some(v) if v.is_string() && !v.as_str().unwrap_or("").is_empty() => {}
            _ => bail!("envelope missing non-empty string field `{key}`"),
        }
    }
    Ok(())
}

fn validate_mint_descriptor(path_id: &str, desc: &MintDescriptor) -> Result<()> {
    anyhow::ensure!(
        !desc.url.trim().is_empty(),
        "mint descriptor requires non-empty url"
    );
    if !desc.mint_id.is_empty() && desc.mint_id != path_id {
        bail!("mint_id mismatch: path={path_id} body={}", desc.mint_id);
    }
    if let Some(ref sha) = desc.content_sha256 {
        if !sha.is_empty() {
            anyhow::ensure!(
                sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()),
                "content_sha256 must be 64-hex when set"
            );
        }
    }
    Ok(())
}

fn list_json_stem_keys(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut keys = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if let Some(key) = name.strip_suffix(".json") {
                keys.push(key.to_string());
            }
        }
    }
    keys.sort();
    Ok(keys)
}

fn read_json_opt<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&data)?))
}

fn write_json_atomic(path: &Path, data: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(data)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod headstash_note_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hash-market-notes-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_envelope() -> serde_json::Value {
        serde_json::json!({
            "ciphertext": "aabbccdd",
            "nonce": "112233445566778899001122",
            "scheme": "xchacha20poly1305",
            "cleartext_layout": "SEAM-NOTE-OUT-V0",
            "cleartext_len": 382
        })
    }

    fn envelope_with_ct(ciphertext: &str) -> serde_json::Value {
        serde_json::json!({
            "ciphertext": ciphertext,
            "nonce": "112233445566778899001122",
            "scheme": "xchacha20poly1305",
            "cleartext_layout": "SEAM-NOTE-OUT-V0",
            "cleartext_len": 382
        })
    }

    #[test]
    fn set_note_get_note_round_trip() {
        let dir = tmp_dir("rt");
        let store = HeadstashStore::new(&dir).unwrap();
        let hs = "terp1contractaddrseason1";
        let addr = "cm.deadbeefcafebabe";

        assert!(store.get_note(hs, addr).unwrap().is_none());
        store.set_note(hs, addr, &sample_envelope()).unwrap();

        let got = store.get_note(hs, addr).unwrap().expect("note present");
        assert_eq!(got["ciphertext"], "aabbccdd");
        assert_eq!(got["scheme"], "xchacha20poly1305");
        assert_eq!(got["cleartext_layout"], "SEAM-NOTE-OUT-V0");
        assert_eq!(got["cleartext_len"], 382);

        let keys = store.list_note_keys(hs).unwrap();
        assert_eq!(keys, vec![addr.to_string()]);

        let (pir_keys, blobs) = store.get_note_blobs(hs).unwrap();
        assert_eq!(pir_keys, keys);
        assert_eq!(blobs.len(), 1);
        assert!(!blobs[0].is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_missing_ciphertext() {
        let dir = tmp_dir("bad");
        let store = HeadstashStore::new(&dir).unwrap();
        let bad = serde_json::json!({"nonce": "aa", "scheme": "xchacha20poly1305"});
        assert!(store.set_note("hs1", "0xabc", &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_empty_ciphertext() {
        let dir = tmp_dir("empty-ct");
        let store = HeadstashStore::new(&dir).unwrap();
        let bad = serde_json::json!({
            "ciphertext": "",
            "nonce": "aa",
            "scheme": "xchacha20poly1305"
        });
        assert!(store.set_note("hs1", "0xabc", &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_empty_nonce() {
        let dir = tmp_dir("empty-nonce");
        let store = HeadstashStore::new(&dir).unwrap();
        let bad = serde_json::json!({
            "ciphertext": "aa",
            "nonce": "",
            "scheme": "xchacha20poly1305"
        });
        assert!(store.set_note("hs1", "0xabc", &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_missing_scheme() {
        let dir = tmp_dir("no-scheme");
        let store = HeadstashStore::new(&dir).unwrap();
        let bad = serde_json::json!({
            "ciphertext": "aa",
            "nonce": "bb"
        });
        assert!(store.set_note("hs1", "0xabc", &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_non_object_envelope() {
        let dir = tmp_dir("non-obj");
        let store = HeadstashStore::new(&dir).unwrap();
        let arr = serde_json::json!(["ciphertext", "nonce", "scheme"]);
        assert!(store.set_note("hs1", "0xabc", &arr).is_err());
        let s = serde_json::json!("not-an-object");
        assert!(store.set_note("hs1", "0xabc", &s).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_rejects_invalid_addr() {
        let dir = tmp_dir("path");
        let store = HeadstashStore::new(&dir).unwrap();
        assert!(store
            .set_note("hs1", "../escape", &sample_envelope())
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_id_rejects_slash_spaces_dotdot() {
        let dir = tmp_dir("vid");
        let store = HeadstashStore::new(&dir).unwrap();
        let env = sample_envelope();
        // slash
        assert!(store.set_note("hs1", "a/b", &env).is_err());
        assert!(store.set_note("a/b", "okaddr", &env).is_err());
        // spaces
        assert!(store.set_note("hs1", "has space", &env).is_err());
        assert!(store.set_note("has space", "okaddr", &env).is_err());
        // path traversal
        assert!(store.set_note("hs1", "..", &env).is_err());
        assert!(store.set_note("hs1", "foo..bar", &env).is_err());
        assert!(store.set_note("..", "okaddr", &env).is_err());
        // empty
        assert!(store.set_note("", "okaddr", &env).is_err());
        assert!(store.set_note("hs1", "", &env).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_bridge_cm_and_claim_addr_keys() {
        let dir = tmp_dir("keys");
        let store = HeadstashStore::new(&dir).unwrap();
        let hs = "season-1";
        store
            .set_note(hs, "terp1abcdefghijklmnopqrstuvwxyz", &sample_envelope())
            .unwrap();
        store
            .set_note(
                hs,
                "0xabcdef0123456789abcdef0123456789abcdef01",
                &sample_envelope(),
            )
            .unwrap();
        store
            .set_note(hs, "pk.aabbccdd", &sample_envelope())
            .unwrap();
        let mut keys = store.list_note_keys(hs).unwrap();
        keys.sort();
        assert_eq!(keys.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_overwrite_same_addr_replaces_envelope() {
        let dir = tmp_dir("ow");
        let store = HeadstashStore::new(&dir).unwrap();
        let hs = "season-1";
        let addr = "cm.abc123";
        store
            .set_note(hs, addr, &envelope_with_ct("first-ciphertext-value"))
            .unwrap();
        store
            .set_note(
                hs,
                addr,
                &envelope_with_ct("second-ciphertext-value-longer"),
            )
            .unwrap();
        let got = store.get_note(hs, addr).unwrap().expect("present");
        assert_eq!(got["ciphertext"], "second-ciphertext-value-longer");
        // Only one key after overwrite
        assert_eq!(store.list_note_keys(hs).unwrap(), vec![addr.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_note_preserves_cleartext_layout_and_len() {
        let dir = tmp_dir("layout");
        let store = HeadstashStore::new(&dir).unwrap();
        let env = sample_envelope();
        store.set_note("hs-layout", "cm.layout01", &env).unwrap();
        let got = store
            .get_note("hs-layout", "cm.layout01")
            .unwrap()
            .expect("present");
        assert_eq!(got["cleartext_layout"], "SEAM-NOTE-OUT-V0");
        assert_eq!(got["cleartext_len"], 382);
        // Server treats envelope as opaque: extra fields round-trip too
        assert_eq!(got["scheme"], "xchacha20poly1305");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_note_list_ordering_and_equal_pir_padding() {
        let dir = tmp_dir("multi");
        let store = HeadstashStore::new(&dir).unwrap();
        let hs = "season-pir";
        // Insert out of order; list_note_keys must sort lexicographically
        store
            .set_note(hs, "cm.zz", &envelope_with_ct("short"))
            .unwrap();
        store
            .set_note(
                hs,
                "cm.aa",
                &envelope_with_ct(
                    "this-is-a-much-longer-ciphertext-payload-to-force-padding-difference",
                ),
            )
            .unwrap();
        store
            .set_note(hs, "cm.mm", &envelope_with_ct("mid-length-ct-xx"))
            .unwrap();

        let keys = store.list_note_keys(hs).unwrap();
        assert_eq!(
            keys,
            vec![
                "cm.aa".to_string(),
                "cm.mm".to_string(),
                "cm.zz".to_string()
            ]
        );

        let (pir_keys, blobs) = store.get_note_blobs(hs).unwrap();
        assert_eq!(pir_keys, keys);
        assert_eq!(blobs.len(), 3);
        let pad_len = blobs[0].len();
        assert!(pad_len > 0);
        for b in &blobs {
            assert_eq!(
                b.len(),
                pad_len,
                "PIR blobs must be equal-padded for xor_pir"
            );
        }
        // Unpadded raw file sizes differ; pad length equals max raw size
        let raw_aa = std::fs::read(dir.join("notes").join(hs).join("cm.aa.json")).unwrap();
        let raw_mm = std::fs::read(dir.join("notes").join(hs).join("cm.mm.json")).unwrap();
        let raw_zz = std::fs::read(dir.join("notes").join(hs).join("cm.zz.json")).unwrap();
        assert_ne!(
            raw_aa.len(),
            raw_zz.len(),
            "fixture should differ in raw size"
        );
        assert_eq!(
            pad_len,
            raw_aa.len().max(raw_mm.len()).max(raw_zz.len()),
            "PIR pad length must equal max on-disk note JSON size"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_note_blobs_xor_pir_select_index() {
        let dir = tmp_dir("xor-pir");
        let store = HeadstashStore::new(&dir).unwrap();
        let hs = "hs-pir-unit";
        store
            .set_note(hs, "cm.zero", &envelope_with_ct("note-zero-ciphertext"))
            .unwrap();
        store
            .set_note(hs, "cm.one", &envelope_with_ct("note-one-ciphertext-XXXX"))
            .unwrap();

        let (keys, blobs) = store.get_note_blobs(hs).unwrap();
        assert_eq!(keys, vec!["cm.one".to_string(), "cm.zero".to_string()]);
        assert_eq!(blobs[0].len(), blobs[1].len());

        let refs: Vec<&[u8]> = blobs.iter().map(|b| b.as_slice()).collect();
        // Select index 0 (cm.one)
        let r0 = crate::pir::xor_pir(&refs, &[1, 0]).unwrap();
        assert_eq!(r0, blobs[0]);
        // Select index 1 (cm.zero)
        let r1 = crate::pir::xor_pir(&refs, &[0, 1]).unwrap();
        assert_eq!(r1, blobs[1]);
        // XOR both
        let both = crate::pir::xor_pir(&refs, &[1, 1]).unwrap();
        let mut expect = blobs[0].clone();
        for (e, b) in expect.iter_mut().zip(blobs[1].iter()) {
            *e ^= *b;
        }
        assert_eq!(both, expect);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod cashu_mesh_store_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hash-market-cashu-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_mint(id: &str, url: &str) -> MintDescriptor {
        MintDescriptor {
            mint_id: id.into(),
            url: url.into(),
            name: Some("lab".into()),
            units: vec!["sat".into()],
            keyset_ids: vec!["00abc".into()],
            nuts: serde_json::json!({}),
            pubkey: None,
            status: MintStatus::Active,
            content_sha256: None,
            registered_at: 0,
            updated_at: 0,
            registrar: None,
            metadata: serde_json::json!({}),
            source: Some("manual".into()),
        }
    }

    fn wallet_envelope(ct: &str) -> serde_json::Value {
        serde_json::json!({
            "ciphertext": ct,
            "nonce": "00112233445566778899aabb",
            "scheme": "xchacha20poly1305",
            "cleartext_layout": "CASHU-TOKEN-BACKUP-V0",
            "cleartext_len": 64
        })
    }

    #[test]
    fn mint_set_get_round_trip() {
        let dir = tmp_dir("mint-rt");
        let store = CashuMeshStore::new(&dir).unwrap();
        let id = "a1b2c3d4e5f6";
        store
            .set_mint(id, sample_mint(id, "https://mint.example"))
            .unwrap();
        let got = store.get_mint(id).unwrap().expect("stored");
        assert_eq!(got.mint_id, id);
        assert_eq!(got.url, "https://mint.example");
        assert_eq!(got.units, vec!["sat".to_string()]);
        assert_eq!(got.status, MintStatus::Active);
        assert_eq!(got.source.as_deref(), Some("manual"));
        assert!(got.updated_at > 0);
        assert!(dir
            .join("cashu")
            .join("mints")
            .join(format!("{id}.json"))
            .is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mint_list_default_active_only() {
        let dir = tmp_dir("mint-list");
        let store = CashuMeshStore::new(&dir).unwrap();
        store
            .set_mint("mint-a", sample_mint("mint-a", "https://a.example"))
            .unwrap();
        let mut paused = sample_mint("mint-p", "https://p.example");
        paused.status = MintStatus::Paused;
        store.set_mint("mint-p", paused).unwrap();
        let active = store.list_mints(None, true).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].mint_id, "mint-a");
        let all = store.list_mints(None, false).unwrap();
        assert_eq!(all.len(), 2);
        let paused_only = store.list_mints(Some("paused"), false).unwrap();
        assert_eq!(paused_only.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wallet_envelope_round_trip() {
        let dir = tmp_dir("wallet-rt");
        let store = CashuMeshStore::new(&dir).unwrap();
        let wid = "wallet-user-1";
        let iid = "token-backup-01";
        store
            .set_wallet_item(wid, iid, &wallet_envelope("deadbeef-ct"))
            .unwrap();
        let got = store.get_wallet_item(wid, iid).unwrap().expect("stored");
        assert_eq!(got["ciphertext"], "deadbeef-ct");
        assert_eq!(got["cleartext_layout"], "CASHU-TOKEN-BACKUP-V0");
        assert_eq!(
            store.list_wallet_item_keys(wid).unwrap(),
            vec![iid.to_string()]
        );
        // Path shape; not under notes/ or content/
        assert!(dir
            .join("cashu")
            .join("wallets")
            .join(wid)
            .join(format!("{iid}.json"))
            .is_file());
        assert!(!dir.join("content").exists());
        assert!(!dir.join("notes").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cashu_ids_reject_path_traversal() {
        let dir = tmp_dir("ids");
        let store = CashuMeshStore::new(&dir).unwrap();
        let m = sample_mint("ok", "https://m.example");
        assert!(store.set_mint("../escape", m.clone()).is_err());
        assert!(store.set_mint("a/b", m).is_err());
        let env = wallet_envelope("ct");
        assert!(store.set_wallet_item("w/../x", "item", &env).is_err());
        assert!(store.set_wallet_item("wallet", "has space", &env).is_err());
        assert!(store.set_wallet_item("wallet", "..", &env).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wallet_rejects_incomplete_envelope() {
        let dir = tmp_dir("env-bad");
        let store = CashuMeshStore::new(&dir).unwrap();
        let bad = serde_json::json!({"nonce": "aa", "scheme": "xchacha20poly1305"});
        assert!(store.set_wallet_item("w1", "i1", &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mint_requires_url() {
        let dir = tmp_dir("no-url");
        let store = CashuMeshStore::new(&dir).unwrap();
        let mut m = sample_mint("m1", "");
        m.url = "  ".into();
        assert!(store.set_mint("m1", m).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ── Stored tree ─────────────────────────────────────────────────────────────

/// Per-address data stored in the tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberData {
    pub allocation: u32,
    pub tier: u8,
    pub proof_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub root: String,
    /// address → { allocation, tier, proof_hashes }
    pub members: HashMap<String, MemberData>,
    pub created_at: u64,
    pub updated_at: u64,
}

// ── Upload body ── matches gen_merkle output format ─────────────────────────

/// Per-address entry in the upload payload (matches gen_merkle's `accounts` map).
#[derive(Debug, Deserialize)]
pub struct RawAccount {
    tier: u8,
    allocation: u32,
    proof_hashes: Vec<String>,
}

/// Accepted upload body — the direct output of `gen_merkle --output`.
#[derive(Debug, Deserialize)]
pub struct TreeInput {
    pub merkle_root: String,
    /// May contain extra fields from gen_merkle (total_addresses, tier_summary) — they are ignored.
    pub accounts: HashMap<String, RawAccount>,
}

impl TreeInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.merkle_root.len() != 64 {
            return Err("merkle_root must be a 64-char hex string".into());
        }
        if hex::decode(&self.merkle_root).is_err() {
            return Err("merkle_root is not valid hex".into());
        }
        for (addr, acc) in &self.accounts {
            for ph in &acc.proof_hashes {
                if ph.len() != 64 {
                    return Err(format!(
                        "proof_hash for {addr} must be 64 hex chars, got {} chars",
                        ph.len()
                    ));
                }
                if hex::decode(ph).is_err() {
                    return Err(format!("proof_hash for {addr} is not valid hex: {ph}"));
                }
            }
        }
        Ok(())
    }
}

// ── Tree Store ──────────────────────────────────────────────────────────────
//
// Single file-backed key-value store for:
//   1. Merkle tree data (JSON files by tree ID under {dir}/{id}.json)
//   2. Blob storage via the `BlobStore` trait (SHA256-addressed files under {dir}/f/{hex_hash})
//
// The `BlobStore` impl uses the `/f/` prefix for content-addressed file storage
// with a QMD-style content-ID index layered on top for O(1) hash lookups.

#[derive(Clone)]
pub struct TreeStore {
    dir: PathBuf,
    /// QMD-style content-ID index layered on the `/f/` prefix file store.
    content_index: ContentIndex,
}

impl TreeStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let f_dir = dir.join("f");
        fs::create_dir_all(&f_dir)?;
        Ok(Self {
            content_index: ContentIndex::new(f_dir),
            dir,
        })
    }

    pub fn list(&self) -> Vec<String> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut ids: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension()?.to_str()? == "json" {
                    p.file_stem()?.to_str().map(str::to_owned)
                } else {
                    None
                }
            })
            .collect();
        ids.sort();
        ids
    }

    pub fn load(&self, id: &str) -> Option<Tree> {
        if !valid_id(id) {
            return None;
        }
        let path = tree_file(&self.dir, id);
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn exists(&self, id: &str) -> bool {
        valid_id(id) && tree_file(&self.dir, id).exists()
    }

    pub fn save(&self, id: &str, input: TreeInput) -> Result<()> {
        if !valid_id(id) {
            bail!("invalid tree id: {id}");
        }
        let path = tree_file(&self.dir, id);
        let now = now_secs();
        let existing_created_at = self.load(id).map(|t| t.created_at).unwrap_or(now);

        let members: HashMap<String, MemberData> = input
            .accounts
            .into_iter()
            .map(|(addr, acc)| {
                (
                    addr,
                    MemberData {
                        allocation: acc.allocation,
                        tier: acc.tier,
                        proof_hashes: acc.proof_hashes,
                    },
                )
            })
            .collect();

        let tree = Tree {
            root: input.merkle_root,
            members,
            created_at: existing_created_at,
            updated_at: now,
        };
        // Atomic write via temp file
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(&tree)?)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !valid_id(id) {
            bail!("invalid tree id: {id}");
        }
        let path = tree_file(&self.dir, id);
        fs::remove_file(path)?;
        Ok(())
    }
}

// Use validate_id directly — it correctly blocks ".." path traversal.
// The regex-based valid_id had a bug: it allowed ".." because \. only matches
// a single dot with no negative lookahead for consecutive dots.
fn valid_id(id: &str) -> bool {
    validate_id(id).is_ok()
}

fn tree_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

// ── BlobStore trait implementation ─────────────────────────────────────────
//
// Delegates to the `/f/` prefix file store with the QMD content-ID index layer.
// This is the soLE `BlobStore` implementation in the crate.

impl TreeStore {
    /// Look up blob metadata (size + uploaded timestamp) from the content-ID index.
    pub fn blob_meta(&self, hash: &[u8; 32]) -> Option<ContentEntry> {
        self.content_index.get(hash)
    }

    /// Bulk lookup: one lock acquisition, one sync check, N hashmap lookups.
    pub fn blob_metas(&self, hashes: &[[u8; 32]]) -> HashMap<[u8; 32], ContentEntry> {
        self.content_index.ensure_synced();
        let index = self.content_index.index.read().unwrap();
        hashes
            .iter()
            .filter_map(|h| index.get(h).cloned().map(|e| (*h, e)))
            .collect()
    }
}

impl BlobStore for TreeStore {
    fn get(&self, hash: &[u8; 32]) -> Option<Vec<u8>> {
        let path = self.dir.join("f").join(hex::encode(hash));
        fs::read(path).ok()
    }

    fn put(&self, content: Vec<u8>) -> std::prelude::v1::Result<BlobDescriptor, BlobStoreError> {
        let hash: [u8; 32] = Sha256::digest(&content).into();
        let uploaded = now_secs();
        let size = content.len() as u64;

        let descriptor = BlobDescriptor {
            url: format!("/blobs/{}", hex::encode(hash)),
            sha256: hash,
            size,
            mime_type: None,
            uploaded,
        };

        let path = self.dir.join("f").join(hex::encode(&hash));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| BlobStoreError::Other(e.to_string()))?;
        }
        // Atomic write via temp file
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &content).map_err(|e| BlobStoreError::Other(e.to_string()))?;
        fs::rename(&tmp, &path).map_err(|e| BlobStoreError::Other(e.to_string()))?;

        // Update QMD content-ID index
        self.content_index
            .insert(hash, ContentEntry { size, uploaded });

        Ok(descriptor)
    }

    fn delete(&self, hash: &[u8; 32]) -> std::prelude::v1::Result<(), BlobStoreError> {
        let path = self.dir.join("f").join(hex::encode(hash));
        if path.exists() {
            fs::remove_file(&path).map_err(|e| BlobStoreError::Other(e.to_string()))?;
            self.content_index.remove(hash);
            Ok(())
        } else {
            Err(BlobStoreError::NotFound)
        }
    }

    fn exists(&self, hash: &[u8; 32]) -> bool {
        self.content_index.contains(hash)
    }

    fn list(&self) -> Vec<[u8; 32]> {
        self.content_index.keys()
    }
}
