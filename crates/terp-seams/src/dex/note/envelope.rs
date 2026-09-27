//! Client-side encrypt / decrypt for Headstash private notes.
//!
//! Encrypts the full 382-byte [`SeamNoteOutV0::to_bytes`] cleartext with
//! **XChaCha20-Poly1305** (option A MVP). Server / hash-market stores the
//! envelope JSON opaquely under `notes/{hs_id}/{addr}.json`.
//!
//! ## Encoding
//! `ciphertext` and `nonce` are **lowercase hex** in JSON (matches PIR hex style).
//! Scheme string is exactly `"xchacha20poly1305"`.
//!
//! ## Envelope shape
//! ```json
//! {
//!   "ciphertext": "…",
//!   "nonce": "…",
//!   "scheme": "xchacha20poly1305",
//!   "cleartext_layout": "SEAM-NOTE-OUT-V0",
//!   "cleartext_len": 382
//! }
//! ```
//! Optional `"sha256"` of **ciphertext bytes only** when distribution is on
//! (BUD dual-index pointer). Primary fetch remains auth-gated `/notes/...`.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{validate_id, NotePersistError, SeamNoteOutV0, SEAM_NOTE_OUT_V0_LEN};

/// AEAD scheme identifier written into every envelope.
pub use crate::domain::SCHEME_XCHACHA20POLY1305;

/// Cleartext layout tag for V0 fixed 382B serialization.
pub use crate::domain::CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0;

/// Opaque note blob stored by HeadstashStore (JSON fields).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEnvelope {
    /// Lowercase hex of AEAD ciphertext (includes Poly1305 tag).
    pub ciphertext: String,
    /// Lowercase hex of 24-byte XChaCha20 nonce.
    pub nonce: String,
    /// Must be [`SCHEME_XCHACHA20POLY1305`].
    pub scheme: String,
    /// Must be [`CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0`] for this crate's decrypt path.
    pub cleartext_layout: String,
    /// Cleartext byte length before encryption (382 for V0).
    pub cleartext_len: u32,
    /// Optional lowercase hex SHA256 of **raw ciphertext bytes** (not JSON).
    /// Set when distribution dual-index is desired; private body stays on `/notes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// SHA256 of decoded ciphertext bytes → lowercase hex (64 chars).
pub fn ciphertext_sha256_hex(ciphertext_hex: &str) -> Result<String, NotePersistError> {
    let ct = hex::decode(ciphertext_hex).map_err(|_| NotePersistError::BadHex)?;
    Ok(hex::encode(Sha256::digest(&ct)))
}

impl NoteEnvelope {
    /// Serialize envelope to JSON value (for HTTP body or store write).
    pub fn to_json_value(&self) -> Result<serde_json::Value, NotePersistError> {
        serde_json::to_value(self).map_err(|_| NotePersistError::Json)
    }

    /// Serialize envelope to compact JSON bytes.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, NotePersistError> {
        serde_json::to_vec(self).map_err(|_| NotePersistError::Json)
    }

    /// Parse envelope from JSON value.
    pub fn from_json_value(v: &serde_json::Value) -> Result<Self, NotePersistError> {
        serde_json::from_value(v.clone()).map_err(|_| NotePersistError::Json)
    }

    /// Parse envelope from JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, NotePersistError> {
        serde_json::from_slice(bytes).map_err(|_| NotePersistError::Json)
    }

    /// Fill [`Self::sha256`] from ciphertext hex (idempotent).
    pub fn with_ciphertext_sha256(mut self) -> Result<Self, NotePersistError> {
        self.sha256 = Some(ciphertext_sha256_hex(&self.ciphertext)?);
        Ok(self)
    }
}

/// Options for [`encrypt_note_out_opts`].
#[derive(Clone, Copy, Debug, Default)]
pub struct EncryptNoteOpts {
    /// When true, set envelope `sha256` = SHA256(ciphertext bytes).
    pub include_ciphertext_sha256: bool,
}

/// Encrypt full 382B `SeamNoteOutV0` cleartext under a 32-byte key.
///
/// Nonce is random 24 bytes from `OsRng`. No AAD is bound (MVP).
pub fn encrypt_note_out(
    note: &SeamNoteOutV0,
    key: &[u8; 32],
) -> Result<NoteEnvelope, NotePersistError> {
    encrypt_note_out_opts(note, key, EncryptNoteOpts::default())
}

/// Encrypt with optional distribution-side ciphertext hash.
pub fn encrypt_note_out_opts(
    note: &SeamNoteOutV0,
    key: &[u8; 32],
    opts: EncryptNoteOpts,
) -> Result<NoteEnvelope, NotePersistError> {
    let plaintext = note.to_bytes();
    debug_assert_eq!(plaintext.len(), SEAM_NOTE_OUT_V0_LEN);

    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: b"",
            },
        )
        .map_err(|_| NotePersistError::Encrypt)?;

    let mut env = NoteEnvelope {
        ciphertext: hex::encode(&ct),
        nonce: hex::encode(nonce.as_slice()),
        scheme: SCHEME_XCHACHA20POLY1305.to_string(),
        cleartext_layout: CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0.to_string(),
        cleartext_len: SEAM_NOTE_OUT_V0_LEN as u32,
        sha256: None,
    };
    if opts.include_ciphertext_sha256 {
        env = env.with_ciphertext_sha256()?;
    }
    Ok(env)
}

/// Decrypt envelope back to [`SeamNoteOutV0`].
///
/// Rejects unknown scheme/layout, wrong cleartext_len, bad hex, or AEAD failure.
pub fn decrypt_note_out(
    env: &NoteEnvelope,
    key: &[u8; 32],
) -> Result<SeamNoteOutV0, NotePersistError> {
    if env.scheme != SCHEME_XCHACHA20POLY1305 {
        return Err(NotePersistError::UnsupportedScheme);
    }
    if env.cleartext_layout != CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0 {
        return Err(NotePersistError::UnsupportedLayout);
    }
    if env.cleartext_len as usize != SEAM_NOTE_OUT_V0_LEN {
        return Err(NotePersistError::BadCleartextLen);
    }

    let ct = hex::decode(&env.ciphertext).map_err(|_| NotePersistError::BadHex)?;
    let nonce_bytes = hex::decode(&env.nonce).map_err(|_| NotePersistError::BadHex)?;
    if nonce_bytes.len() != 24 {
        return Err(NotePersistError::BadNonce);
    }
    let nonce = XNonce::from_slice(&nonce_bytes);

    let cipher = XChaCha20Poly1305::new(key.into());
    let pt = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &ct,
                aad: b"",
            },
        )
        .map_err(|_| NotePersistError::Decrypt)?;

    if pt.len() != SEAM_NOTE_OUT_V0_LEN {
        return Err(NotePersistError::BadCleartextLen);
    }
    SeamNoteOutV0::from_bytes(&pt).map_err(|_| NotePersistError::BadCleartextLayout)
}

/// Pure plan for a client PUT: envelope + path segments (no HTTP).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotePersistPlan {
    pub hs_id: String,
    pub addr: String,
    pub envelope: NoteEnvelope,
    /// Logical store path: `notes/{hs_id}/{addr}` (no `.json` suffix).
    pub store_path: String,
}

/// Build encrypt plan for a note without contacting the server.
///
/// `hs_id` — contract bech32 or season slug.  
/// `addr` — already-formed path segment (use [`super::note_addr_claim`] /
/// [`super::note_addr_cm`] / [`super::note_addr_pk`]).
pub fn persist_plan_from_seam_note(
    note: &SeamNoteOutV0,
    key: &[u8; 32],
    hs_id: &str,
    addr: &str,
) -> Result<NotePersistPlan, NotePersistError> {
    persist_plan_from_seam_note_opts(note, key, hs_id, addr, EncryptNoteOpts::default())
}

/// Build encrypt plan with optional ciphertext `sha256` for distribution.
pub fn persist_plan_from_seam_note_opts(
    note: &SeamNoteOutV0,
    key: &[u8; 32],
    hs_id: &str,
    addr: &str,
    opts: EncryptNoteOpts,
) -> Result<NotePersistPlan, NotePersistError> {
    validate_id(hs_id)?;
    validate_id(addr)?;
    let envelope = encrypt_note_out_opts(note, key, opts)?;
    Ok(NotePersistPlan {
        store_path: format!("notes/{hs_id}/{addr}"),
        hs_id: hs_id.to_string(),
        addr: addr.to_string(),
        envelope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::note::{
        from_bridge_mint, from_headstash_instance, note_addr_claim, note_addr_cm, BridgeMintPublic,
        HeadstashInstanceBytes, CM_TACIT_KECCAK_LEAF, SEAM_NOTE_OUT_V0_LEN,
    };

    fn arr32(seed: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        for (i, b) in a.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        a
    }

    fn fixture_claim_instance() -> HeadstashInstanceBytes {
        HeadstashInstanceBytes {
            anchor: arr32(0x10),
            nd: arr32(0x20),
            v: 1_000_000,
            nf: arr32(0x30),
            recp_raw: arr32(0x40),
            cmx: arr32(0x50),
        }
    }

    fn fixture_bridge_mint() -> BridgeMintPublic {
        BridgeMintPublic {
            source_chain_tag: "bitcoin-mainnet".into(),
            tacit_asset_id: arr32(0xA1),
            value_u64: 42_000,
            nullifier: arr32(0xB2),
            dest_commitment: arr32(0xC3),
            claim_id: arr32(0xD4),
            source_pool_root: arr32(0xE5),
            source_burn_root: arr32(0xF6),
            unit_scale: 1,
            pool_domain: arr32(0x77),
            rcm: arr32(0x88),
            cm_public: arr32(0x99),
            cm_encoding: CM_TACIT_KECCAK_LEAF,
        }
    }

    #[test]
    fn encrypt_decrypt_claim_fixture_recovers_382b() {
        let key = arr32(0xE1);
        let note =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let clear = note.to_bytes();
        assert_eq!(clear.len(), SEAM_NOTE_OUT_V0_LEN);

        let env = encrypt_note_out(&note, &key).unwrap();
        assert_eq!(env.scheme, SCHEME_XCHACHA20POLY1305);
        assert_eq!(env.cleartext_layout, CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0);
        assert_eq!(env.cleartext_len, 382);
        assert_eq!(hex::decode(&env.nonce).unwrap().len(), 24);
        // Ciphertext is not cleartext hex
        assert_ne!(env.ciphertext, hex::encode(clear));

        let recovered = decrypt_note_out(&env, &key).unwrap();
        assert_eq!(recovered, note);
        assert_eq!(recovered.to_bytes(), clear);

        // Wrong key fails
        assert!(decrypt_note_out(&env, &arr32(0xFF)).is_err());
    }

    #[test]
    fn encrypt_decrypt_bridge_fixture_recovers_exact() {
        let key = arr32(0xB7);
        let note = from_bridge_mint(&fixture_bridge_mint(), 42_000).unwrap();
        let env = encrypt_note_out(&note, &key).unwrap();
        let recovered = decrypt_note_out(&env, &key).unwrap();
        assert_eq!(recovered, note);
        assert_eq!(recovered.to_bytes().len(), SEAM_NOTE_OUT_V0_LEN);
    }

    #[test]
    fn envelope_json_roundtrip() {
        let key = arr32(0x11);
        let note =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let env = encrypt_note_out(&note, &key).unwrap();
        let v = env.to_json_value().unwrap();
        assert_eq!(v["scheme"], SCHEME_XCHACHA20POLY1305);
        assert_eq!(v["cleartext_len"], 382);
        let parsed = NoteEnvelope::from_json_value(&v).unwrap();
        assert_eq!(decrypt_note_out(&parsed, &key).unwrap(), note);
    }

    #[test]
    fn persist_plan_claim_and_bridge_addrs() {
        let key = arr32(0x22);
        let claim =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let bridge = from_bridge_mint(&fixture_bridge_mint(), 42_000).unwrap();

        let claim_addr =
            note_addr_claim("terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsr2pd5").unwrap();
        let plan = persist_plan_from_seam_note(&claim, &key, "season-1", &claim_addr).unwrap();
        assert_eq!(
            plan.store_path,
            format!("notes/season-1/{claim_addr}")
        );
        assert_eq!(decrypt_note_out(&plan.envelope, &key).unwrap(), claim);

        let cm_addr = note_addr_cm(&bridge.cm_public);
        assert!(cm_addr.starts_with("cm."));
        let plan_b =
            persist_plan_from_seam_note(&bridge, &key, "terp1contractxxxxxxxxxxxxxxxxxxxx", &cm_addr)
                .unwrap();
        assert!(plan_b.store_path.starts_with("notes/"));
        assert_eq!(decrypt_note_out(&plan_b.envelope, &key).unwrap(), bridge);
    }

    #[test]
    fn reject_wrong_scheme_or_layout() {
        let key = arr32(0x33);
        let note =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let mut env = encrypt_note_out(&note, &key).unwrap();
        env.scheme = "aes-gcm".into();
        assert_eq!(
            decrypt_note_out(&env, &key),
            Err(NotePersistError::UnsupportedScheme)
        );
        env.scheme = SCHEME_XCHACHA20POLY1305.into();
        env.cleartext_layout = "OTHER".into();
        assert_eq!(
            decrypt_note_out(&env, &key),
            Err(NotePersistError::UnsupportedLayout)
        );
    }

    #[test]
    fn optional_ciphertext_sha256_when_distribution_on() {
        let key = arr32(0x44);
        let note =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let env = encrypt_note_out_opts(
            &note,
            &key,
            EncryptNoteOpts {
                include_ciphertext_sha256: true,
            },
        )
        .unwrap();
        let sha = env.sha256.as_ref().expect("sha256 set");
        assert_eq!(sha.len(), 64);
        assert_eq!(sha, &ciphertext_sha256_hex(&env.ciphertext).unwrap());
        // Still decrypts
        assert_eq!(decrypt_note_out(&env, &key).unwrap(), note);
        // JSON includes sha256
        let v = env.to_json_value().unwrap();
        assert_eq!(v["sha256"], *sha);
        // Default encrypt omits field
        let plain = encrypt_note_out(&note, &key).unwrap();
        assert!(plain.sha256.is_none());
        let v2 = plain.to_json_value().unwrap();
        assert!(v2.get("sha256").is_none());
    }
}
