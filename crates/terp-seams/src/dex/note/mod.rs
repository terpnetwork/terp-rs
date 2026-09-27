//! Pure structural implementation of SEAM-NOTE-OUT (`SeamNoteOutV0`).
//!
//! Lives in **`crate::dex::note`** (one seams crate).
//!
//! Spec: `docs/plans/spectrum/SEAM-NOTE-OUT.md`
//! Tests: SEAM-N1 … SEAM-N6 (no Halo2 / MockProver).
//!
//! ## Client persistence (Headstash private notes)
//!
//! Encrypt the full 382B cleartext with XChaCha20-Poly1305, then PUT the
//! opaque envelope to hash-market `notes/{hs_id}/{addr}` (not public content).
//!
//! - [`encrypt_note_out`] / [`decrypt_note_out`] / [`NoteEnvelope`]
//! - [`persist_plan_from_seam_note`] — pure plan (no HTTP)
//! - [`note_addr_claim`] / [`note_addr_cm`] / [`note_addr_pk`] / [`validate_id`]
//! - optional feature `http`: [`put_note_envelope`]

#![deny(unsafe_code)]

pub use crate::domain::{
    CLEARTEXT_LAYOUT_SEAM_NOTE_OUT_V0, CM_ABSTRACT_LEAF_V0, CM_ORCHARD_CMX, CM_TACIT_KECCAK_LEAF,
    CM_UNSPECIFIED, DOMAIN_TAG_NOTE_OUT, NF_BRIDGE_BURN, NF_HEADSTASH_CLAIM, NF_POOL_SPEND,
    NF_UNSPECIFIED, NOTE_ID_MAX_LEN, ORIGIN_BRIDGE_MINT, ORIGIN_HEADSTASH, SCHEME_XCHACHA20POLY1305,
    SEAM_NOTE_OUT_V0_LEN, VERSION_V0,
};

mod addr;
mod auth;
mod envelope;

#[cfg(feature = "http")]
mod http;

pub use addr::{note_addr_claim, note_addr_cm, note_addr_pk, validate_id};
pub use auth::{
    auth_headers_from_env, bearer_auth_headers, merge_auth_headers, now_timestamp_secs,
    snap_auth_message,
};
#[cfg(feature = "auth")]
pub use auth::snap_secp_auth_headers;
pub use envelope::{
    ciphertext_sha256_hex, decrypt_note_out, encrypt_note_out, encrypt_note_out_opts,
    persist_plan_from_seam_note, persist_plan_from_seam_note_opts, EncryptNoteOpts, NoteEnvelope,
    NotePersistPlan,
};

#[cfg(feature = "http")]
pub use http::{
    get_note_envelope, list_note_keys, pir_fetch_note_blob, pir_get_note_envelope, put_note_envelope,
};

use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Persistence errors (encrypt / addr / optional HTTP)
// ---------------------------------------------------------------------------

/// Errors from note envelope encrypt/decrypt, addr validation, or HTTP put.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotePersistError {
    InvalidId,
    Encrypt,
    Decrypt,
    BadHex,
    BadNonce,
    UnsupportedScheme,
    UnsupportedLayout,
    BadCleartextLen,
    BadCleartextLayout,
    Json,
    Http(String),
    HttpStatus { status: u16, body: String },
    Auth(String),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------


/// ASCII `terp-note-out-v0` + NUL pad to 16.




// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Canonical intermediate note egress (frozen V0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeamNoteOutV0 {
    pub version: u8,
    pub domain_tag: [u8; 16],
    pub origin: u8,
    pub asset_id: [u8; 32],
    pub value: u64,
    pub owner_binding: [u8; 32],
    pub cm_public: [u8; 32],
    pub cm_encoding: u8,
    pub nullifier_lineage: [u8; 32],
    pub nullifier_domain: u8,
    pub provenance_anchor: [u8; 32],
    pub claim_id: [u8; 32],
    pub source_chain_tag_hash: [u8; 32],
    pub rcm: [u8; 32],
    pub rcm_flag: u8,
    pub memo: [u8; 64],
    pub memo_flag: u8,
    pub pool_domain: [u8; 32],
}

impl Default for SeamNoteOutV0 {
    fn default() -> Self {
        Self {
            version: VERSION_V0,
            domain_tag: DOMAIN_TAG_NOTE_OUT,
            origin: 0,
            asset_id: [0u8; 32],
            value: 0,
            owner_binding: [0u8; 32],
            cm_public: [0u8; 32],
            cm_encoding: CM_UNSPECIFIED,
            nullifier_lineage: [0u8; 32],
            nullifier_domain: NF_UNSPECIFIED,
            provenance_anchor: [0u8; 32],
            claim_id: [0u8; 32],
            source_chain_tag_hash: [0u8; 32],
            rcm: [0u8; 32],
            rcm_flag: 0,
            memo: [0u8; 64],
            memo_flag: 0,
            pool_domain: [0u8; 32],
        }
    }
}

impl SeamNoteOutV0 {
    /// Canonical fixed serialization (382 bytes).
    pub fn to_bytes(&self) -> [u8; SEAM_NOTE_OUT_V0_LEN] {
        let mut out = [0u8; SEAM_NOTE_OUT_V0_LEN];
        let mut o = 0usize;
        write_u8(&mut out, &mut o, self.version);
        write_arr(&mut out, &mut o, &self.domain_tag);
        write_u8(&mut out, &mut o, self.origin);
        write_arr(&mut out, &mut o, &self.asset_id);
        write_u64_le(&mut out, &mut o, self.value);
        write_arr(&mut out, &mut o, &self.owner_binding);
        write_arr(&mut out, &mut o, &self.cm_public);
        write_u8(&mut out, &mut o, self.cm_encoding);
        write_arr(&mut out, &mut o, &self.nullifier_lineage);
        write_u8(&mut out, &mut o, self.nullifier_domain);
        write_arr(&mut out, &mut o, &self.provenance_anchor);
        write_arr(&mut out, &mut o, &self.claim_id);
        write_arr(&mut out, &mut o, &self.source_chain_tag_hash);
        write_arr(&mut out, &mut o, &self.rcm);
        write_u8(&mut out, &mut o, self.rcm_flag);
        write_arr(&mut out, &mut o, &self.memo);
        write_u8(&mut out, &mut o, self.memo_flag);
        write_arr(&mut out, &mut o, &self.pool_domain);
        debug_assert_eq!(o, SEAM_NOTE_OUT_V0_LEN);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SeamError> {
        if bytes.len() != SEAM_NOTE_OUT_V0_LEN {
            return Err(SeamError::BadLayout);
        }
        let mut i = 0usize;
        let version = read_u8(bytes, &mut i);
        let mut domain_tag = [0u8; 16];
        read_arr(bytes, &mut i, &mut domain_tag);
        let origin = read_u8(bytes, &mut i);
        let mut asset_id = [0u8; 32];
        read_arr(bytes, &mut i, &mut asset_id);
        let value = read_u64_le(bytes, &mut i);
        let mut owner_binding = [0u8; 32];
        read_arr(bytes, &mut i, &mut owner_binding);
        let mut cm_public = [0u8; 32];
        read_arr(bytes, &mut i, &mut cm_public);
        let cm_encoding = read_u8(bytes, &mut i);
        let mut nullifier_lineage = [0u8; 32];
        read_arr(bytes, &mut i, &mut nullifier_lineage);
        let nullifier_domain = read_u8(bytes, &mut i);
        let mut provenance_anchor = [0u8; 32];
        read_arr(bytes, &mut i, &mut provenance_anchor);
        let mut claim_id = [0u8; 32];
        read_arr(bytes, &mut i, &mut claim_id);
        let mut source_chain_tag_hash = [0u8; 32];
        read_arr(bytes, &mut i, &mut source_chain_tag_hash);
        let mut rcm = [0u8; 32];
        read_arr(bytes, &mut i, &mut rcm);
        let rcm_flag = read_u8(bytes, &mut i);
        let mut memo = [0u8; 64];
        read_arr(bytes, &mut i, &mut memo);
        let memo_flag = read_u8(bytes, &mut i);
        let mut pool_domain = [0u8; 32];
        read_arr(bytes, &mut i, &mut pool_domain);
        debug_assert_eq!(i, SEAM_NOTE_OUT_V0_LEN);
        Ok(Self {
            version,
            domain_tag,
            origin,
            asset_id,
            value,
            owner_binding,
            cm_public,
            cm_encoding,
            nullifier_lineage,
            nullifier_domain,
            provenance_anchor,
            claim_id,
            source_chain_tag_hash,
            rcm,
            rcm_flag,
            memo,
            memo_flag,
            pool_domain,
        })
    }
}

// ---------------------------------------------------------------------------
// Headstash Instance-shaped bytes (A §1.4, 168 bytes)
// ---------------------------------------------------------------------------

/// Synthetic Headstash public instance layout (structural only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadstashInstanceBytes {
    pub anchor: [u8; 32],
    pub nd: [u8; 32],
    pub v: u64,
    pub nf: [u8; 32],
    pub recp_raw: [u8; 32],
    pub cmx: [u8; 32],
}

impl HeadstashInstanceBytes {
    pub const LEN: usize = 168;

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        let mut o = 0;
        write_arr(&mut out, &mut o, &self.anchor);
        write_arr(&mut out, &mut o, &self.nd);
        write_u64_le(&mut out, &mut o, self.v);
        write_arr(&mut out, &mut o, &self.nf);
        write_arr(&mut out, &mut o, &self.recp_raw);
        write_arr(&mut out, &mut o, &self.cmx);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SeamError> {
        if bytes.len() != Self::LEN {
            return Err(SeamError::BadLayout);
        }
        let mut i = 0;
        let mut anchor = [0u8; 32];
        read_arr(bytes, &mut i, &mut anchor);
        let mut nd = [0u8; 32];
        read_arr(bytes, &mut i, &mut nd);
        let v = read_u64_le(bytes, &mut i);
        let mut nf = [0u8; 32];
        read_arr(bytes, &mut i, &mut nf);
        let mut recp_raw = [0u8; 32];
        read_arr(bytes, &mut i, &mut recp_raw);
        let mut cmx = [0u8; 32];
        read_arr(bytes, &mut i, &mut cmx);
        Ok(Self {
            anchor,
            nd,
            v,
            nf,
            recp_raw,
            cmx,
        })
    }
}

/// H6 claim output stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimOutputNoteV0 {
    pub asset_tag: [u8; 32],
    pub value: u64,
    pub owner: [u8; 32],
    pub cmx: [u8; 32],
    pub nf_claim: [u8; 32],
    pub anchor_distro: [u8; 32],
}

impl From<&HeadstashInstanceBytes> for ClaimOutputNoteV0 {
    fn from(i: &HeadstashInstanceBytes) -> Self {
        Self {
            asset_tag: i.nd,
            value: i.v,
            owner: i.recp_raw,
            cmx: i.cmx,
            nf_claim: i.nf,
            anchor_distro: i.anchor,
        }
    }
}

// ---------------------------------------------------------------------------
// Bridge mint public packet (structural)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeMintPublic {
    pub source_chain_tag: String,
    pub tacit_asset_id: [u8; 32],
    pub value_u64: u64,
    pub nullifier: [u8; 32],
    pub dest_commitment: [u8; 32],
    pub claim_id: [u8; 32],
    pub source_pool_root: [u8; 32],
    pub source_burn_root: [u8; 32],
    pub unit_scale: u64,
    pub pool_domain: [u8; 32],
    /// Optional Pedersen blinding; if all-zero, rcm_flag=0.
    pub rcm: [u8; 32],
    /// Destination leaf fingerprint (or stub).
    pub cm_public: [u8; 32],
    pub cm_encoding: u8,
}

// ---------------------------------------------------------------------------
// DEX consumability
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DexSpendInputs {
    pub asset_id: [u8; 32],
    pub value: u64,
    pub cm_public: [u8; 32],
    pub owner_binding: [u8; 32],
    pub rcm: Option<[u8; 32]>,
    /// Ingress lineage — NOT pool-spend nullifier.
    pub ingress_nullifier_lineage: [u8; 32],
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamError {
    BadLayout,
    BadVersion,
    BadDomainTag,
    BadOriginNfPair,
    BadCmEncoding,
    BadNullifierDomain,
    ZeroAssetId,
    ZeroCmPublic,
    NotDexConsumable,
    ConservationBreak,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

pub fn validate_seam_note_out_v0(n: &SeamNoteOutV0) -> Result<(), SeamError> {
    if n.version != VERSION_V0 {
        return Err(SeamError::BadVersion);
    }
    if n.domain_tag != DOMAIN_TAG_NOTE_OUT {
        return Err(SeamError::BadDomainTag);
    }
    // Origin × nullifier_domain (SEAM-N2)
    match (n.origin, n.nullifier_domain) {
        (ORIGIN_HEADSTASH, NF_HEADSTASH_CLAIM) => {}
        (ORIGIN_BRIDGE_MINT, NF_BRIDGE_BURN) => {}
        _ => return Err(SeamError::BadOriginNfPair),
    }
    if n.cm_encoding == CM_UNSPECIFIED {
        return Err(SeamError::BadCmEncoding);
    }
    if n.nullifier_domain == NF_POOL_SPEND || n.nullifier_domain == NF_UNSPECIFIED {
        return Err(SeamError::BadNullifierDomain);
    }
    if n.asset_id == [0u8; 32] {
        return Err(SeamError::ZeroAssetId);
    }
    if n.cm_public == [0u8; 32] {
        return Err(SeamError::ZeroCmPublic);
    }
    if n.rcm_flag > 1 || n.memo_flag > 1 {
        return Err(SeamError::BadLayout);
    }
    // Fixed layout always serializes to 382
    if n.to_bytes().len() != SEAM_NOTE_OUT_V0_LEN {
        return Err(SeamError::BadLayout);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

/// Map Headstash Instance / H6 → SeamNoteOutV0 (§2.2).
pub fn from_headstash_instance(
    inst: &HeadstashInstanceBytes,
    pool_domain: [u8; 32],
    rcm: Option<[u8; 32]>,
) -> SeamNoteOutV0 {
    let mut n = SeamNoteOutV0::default();
    n.origin = ORIGIN_HEADSTASH;
    n.asset_id = inst.nd;
    n.value = inst.v;
    n.owner_binding = inst.recp_raw;
    n.cm_public = inst.cmx;
    n.cm_encoding = CM_ORCHARD_CMX;
    n.nullifier_lineage = inst.nf;
    n.nullifier_domain = NF_HEADSTASH_CLAIM;
    n.provenance_anchor = inst.anchor;
    n.claim_id = [0u8; 32];
    n.source_chain_tag_hash = [0u8; 32];
    if let Some(r) = rcm {
        n.rcm = r;
        n.rcm_flag = 1;
    }
    n.pool_domain = pool_domain;
    n
}

pub fn from_claim_output_h6(
    h6: &ClaimOutputNoteV0,
    pool_domain: [u8; 32],
    rcm: Option<[u8; 32]>,
) -> SeamNoteOutV0 {
    let inst = HeadstashInstanceBytes {
        anchor: h6.anchor_distro,
        nd: h6.asset_tag,
        v: h6.value,
        nf: h6.nf_claim,
        recp_raw: h6.owner,
        cmx: h6.cmx,
    };
    from_headstash_instance(&inst, pool_domain, rcm)
}

/// B §5 terp asset id: SHA256("terp-tacit-asset-v1" ‖ chain ‖ tacit_id ‖ unit_scale_be)
pub fn terp_asset_id(
    source_chain_tag: &str,
    tacit_asset_id: &[u8; 32],
    unit_scale: u64,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"terp-tacit-asset-v1");
    h.update(source_chain_tag.as_bytes());
    h.update(tacit_asset_id);
    h.update(unit_scale.to_be_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

pub fn source_chain_tag_hash(tag: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Map bridge mint public → SeamNoteOutV0 (§3.3).
/// `v_burn` used for companion conservation check (must equal value_u64 for accept).
pub fn from_bridge_mint(
    m: &BridgeMintPublic,
    v_burn: u64,
) -> Result<SeamNoteOutV0, SeamError> {
    if m.value_u64 != v_burn {
        return Err(SeamError::ConservationBreak);
    }
    let mut n = SeamNoteOutV0::default();
    n.origin = ORIGIN_BRIDGE_MINT;
    n.asset_id = terp_asset_id(&m.source_chain_tag, &m.tacit_asset_id, m.unit_scale);
    n.value = m.value_u64;
    n.owner_binding = m.dest_commitment;
    n.cm_public = m.cm_public;
    n.cm_encoding = m.cm_encoding;
    if n.cm_encoding != CM_TACIT_KECCAK_LEAF && n.cm_encoding != CM_ABSTRACT_LEAF_V0 {
        return Err(SeamError::BadCmEncoding);
    }
    n.nullifier_lineage = m.nullifier;
    n.nullifier_domain = NF_BRIDGE_BURN;
    n.provenance_anchor = m.source_burn_root; // H-1 preferred
    n.claim_id = m.claim_id;
    n.source_chain_tag_hash = source_chain_tag_hash(&m.source_chain_tag);
    if m.rcm != [0u8; 32] {
        n.rcm = m.rcm;
        n.rcm_flag = 1;
    }
    n.pool_domain = m.pool_domain;
    Ok(n)
}

/// §4.3 DEX consumability (structural).
pub fn is_dex_consumable(n: &SeamNoteOutV0) -> bool {
    if validate_seam_note_out_v0(n).is_err() {
        return false;
    }
    if n.rcm_flag != 1 {
        return false;
    }
    if n.asset_id == [0u8; 32] || n.cm_public == [0u8; 32] {
        return false;
    }
    if n.cm_encoding == CM_UNSPECIFIED {
        return false;
    }
    true
}

pub fn to_dex_spend_inputs(n: &SeamNoteOutV0) -> Result<DexSpendInputs, SeamError> {
    if !is_dex_consumable(n) {
        return Err(SeamError::NotDexConsumable);
    }
    Ok(DexSpendInputs {
        asset_id: n.asset_id,
        value: n.value,
        cm_public: n.cm_public,
        owner_binding: n.owner_binding,
        rcm: Some(n.rcm),
        ingress_nullifier_lineage: n.nullifier_lineage,
    })
}

/// Synthetic pool-spend nullifier (must ≠ ingress lineage) for SEAM-N5.
pub fn synthetic_pool_spend_nf(note_cm: &[u8; 32], opening: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pool-nf-v0");
    h.update(note_cm);
    h.update(opening);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

// ---------------------------------------------------------------------------
// Serde helpers
// ---------------------------------------------------------------------------

fn write_u8(out: &mut [u8], o: &mut usize, v: u8) {
    out[*o] = v;
    *o += 1;
}
fn write_u64_le(out: &mut [u8], o: &mut usize, v: u64) {
    out[*o..*o + 8].copy_from_slice(&v.to_le_bytes());
    *o += 8;
}
fn write_arr(out: &mut [u8], o: &mut usize, a: &[u8]) {
    out[*o..*o + a.len()].copy_from_slice(a);
    *o += a.len();
}
fn read_u8(bytes: &[u8], i: &mut usize) -> u8 {
    let v = bytes[*i];
    *i += 1;
    v
}
fn read_u64_le(bytes: &[u8], i: &mut usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[*i..*i + 8]);
    *i += 8;
    u64::from_le_bytes(b)
}
fn read_arr(bytes: &[u8], i: &mut usize, dest: &mut [u8]) {
    dest.copy_from_slice(&bytes[*i..*i + dest.len()]);
    *i += dest.len();
}

// ---------------------------------------------------------------------------
// Fixtures (test-only)
// ---------------------------------------------------------------------------

#[cfg(test)]
fn arr32(seed: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    for (i, b) in a.iter_mut().enumerate() {
        *b = seed.wrapping_add(i as u8);
    }
    a
}

#[cfg(test)]
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

#[cfg(test)]
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

// ---------------------------------------------------------------------------
// Tests SEAM-N1 … SEAM-N6
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// SEAM-N1 — Layout widths and version
    #[test]
    fn seam_n1_layout_widths_and_version() {
        let claim = from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        let bridge = from_bridge_mint(&fixture_bridge_mint(), 42_000).unwrap();

        for n in [&claim, &bridge] {
            let bytes = n.to_bytes();
            assert_eq!(bytes.len(), SEAM_NOTE_OUT_V0_LEN);
            assert_eq!(n.version, VERSION_V0);
            assert_eq!(&n.domain_tag, b"terp-note-out-v0");
            assert_eq!(n.asset_id.len(), 32);
            assert_eq!(n.owner_binding.len(), 32);
            assert_eq!(n.cm_public.len(), 32);
            assert_eq!(n.nullifier_lineage.len(), 32);
            assert_eq!(n.provenance_anchor.len(), 32);
            assert_eq!(n.claim_id.len(), 32);
            assert_eq!(n.source_chain_tag_hash.len(), 32);
            assert_eq!(n.rcm.len(), 32);
            assert_eq!(n.memo.len(), 64);
            assert_eq!(n.pool_domain.len(), 32);
            // round-trip
            let decoded = SeamNoteOutV0::from_bytes(&bytes).unwrap();
            assert_eq!(&decoded, n);
            validate_seam_note_out_v0(n).unwrap();
        }
    }

    /// SEAM-N2 — Origin / nullifier_domain pairing
    #[test]
    fn seam_n2_origin_nullifier_pairing() {
        let mut base = from_headstash_instance(&fixture_claim_instance(), arr32(0x01), Some(arr32(0x02)));
        // good claim
        base.origin = ORIGIN_HEADSTASH;
        base.nullifier_domain = NF_HEADSTASH_CLAIM;
        assert!(validate_seam_note_out_v0(&base).is_ok());

        // good bridge
        base.origin = ORIGIN_BRIDGE_MINT;
        base.nullifier_domain = NF_BRIDGE_BURN;
        assert!(validate_seam_note_out_v0(&base).is_ok());

        // bad cross pairs
        base.origin = ORIGIN_HEADSTASH;
        base.nullifier_domain = NF_BRIDGE_BURN;
        assert_eq!(
            validate_seam_note_out_v0(&base),
            Err(SeamError::BadOriginNfPair)
        );
        base.origin = ORIGIN_BRIDGE_MINT;
        base.nullifier_domain = NF_HEADSTASH_CLAIM;
        assert_eq!(
            validate_seam_note_out_v0(&base),
            Err(SeamError::BadOriginNfPair)
        );
    }

    /// SEAM-N3 — Headstash Instance / H6 → seam map
    #[test]
    fn seam_n3_headstash_instance_map() {
        let inst = fixture_claim_instance();
        let wire = inst.to_bytes();
        assert_eq!(wire.len(), 168);
        let parsed = HeadstashInstanceBytes::from_bytes(&wire).unwrap();
        assert_eq!(parsed, inst);

        let h6 = ClaimOutputNoteV0::from(&inst);
        let seam = from_claim_output_h6(&h6, [0u8; 32], None);

        assert_eq!(seam.origin, ORIGIN_HEADSTASH);
        assert_eq!(seam.cm_encoding, CM_ORCHARD_CMX);
        assert_eq!(seam.nullifier_domain, NF_HEADSTASH_CLAIM);
        assert_eq!(seam.asset_id, inst.nd);
        assert_eq!(seam.value, inst.v);
        assert_eq!(seam.owner_binding, inst.recp_raw);
        assert_eq!(seam.cm_public, inst.cmx);
        assert_eq!(seam.nullifier_lineage, inst.nf);
        assert_eq!(seam.provenance_anchor, inst.anchor);
        assert_eq!(seam.claim_id, [0u8; 32]);
        assert_eq!(seam.source_chain_tag_hash, [0u8; 32]);
        validate_seam_note_out_v0(&seam).unwrap();
    }

    /// SEAM-N4 — Bridge mint public packet → seam map
    #[test]
    fn seam_n4_bridge_mint_map() {
        let m = fixture_bridge_mint();
        let seam = from_bridge_mint(&m, 42_000).unwrap();
        assert_eq!(seam.origin, ORIGIN_BRIDGE_MINT);
        assert_eq!(seam.nullifier_domain, NF_BRIDGE_BURN);
        assert_eq!(
            seam.asset_id,
            terp_asset_id(&m.source_chain_tag, &m.tacit_asset_id, m.unit_scale)
        );
        assert_ne!(seam.asset_id, m.tacit_asset_id); // not raw tacit id
        assert_eq!(seam.value, m.value_u64);
        assert_eq!(seam.nullifier_lineage, m.nullifier);
        assert_eq!(seam.claim_id, m.claim_id);
        assert_eq!(
            seam.source_chain_tag_hash,
            source_chain_tag_hash(&m.source_chain_tag)
        );
        assert!(
            seam.cm_encoding == CM_TACIT_KECCAK_LEAF || seam.cm_encoding == CM_ABSTRACT_LEAF_V0
        );
        assert_eq!(seam.provenance_anchor, m.source_burn_root);
        validate_seam_note_out_v0(&seam).unwrap();

        // conservation break
        assert_eq!(
            from_bridge_mint(&m, 41_999),
            Err(SeamError::ConservationBreak)
        );
    }

    /// SEAM-N5 — Seam → DEX spend consumability
    #[test]
    fn seam_n5_dex_consumability() {
        let mut good =
            from_headstash_instance(&fixture_claim_instance(), arr32(0xAA), Some(arr32(0xBB)));
        good.rcm_flag = 1;
        assert!(is_dex_consumable(&good));
        let spend = to_dex_spend_inputs(&good).unwrap();
        assert_eq!(spend.asset_id, good.asset_id);
        assert_eq!(spend.value, good.value);
        assert_eq!(spend.cm_public, good.cm_public);
        assert_eq!(spend.ingress_nullifier_lineage, good.nullifier_lineage);

        let pool_nf = synthetic_pool_spend_nf(&good.cm_public, &good.rcm);
        assert_ne!(
            pool_nf, good.nullifier_lineage,
            "pool spend ν must not equal ingress lineage"
        );

        // bad: rcm_flag=0
        let mut bad = good.clone();
        bad.rcm_flag = 0;
        bad.rcm = [0u8; 32];
        assert!(!is_dex_consumable(&bad));
        assert_eq!(to_dex_spend_inputs(&bad), Err(SeamError::NotDexConsumable));

        // bad: zero cm
        let mut bad2 = good.clone();
        bad2.cm_public = [0u8; 32];
        assert!(!is_dex_consumable(&bad2));

        // bad: unspecified encoding
        let mut bad3 = good.clone();
        bad3.cm_encoding = CM_UNSPECIFIED;
        assert!(!is_dex_consumable(&bad3));
    }

    /// SEAM-N6 — Claim vs bridge structural equality (C7.2)
    #[test]
    fn seam_n6_claim_bridge_schema_equality() {
        let claim =
            from_headstash_instance(&fixture_claim_instance(), arr32(0x11), Some(arr32(0x22)));
        let bridge = from_bridge_mint(&fixture_bridge_mint(), 42_000).unwrap();

        assert_eq!(claim.to_bytes().len(), 382);
        assert_eq!(bridge.to_bytes().len(), 382);
        validate_seam_note_out_v0(&claim).unwrap();
        validate_seam_note_out_v0(&bridge).unwrap();
        assert!(is_dex_consumable(&claim));
        assert!(is_dex_consumable(&bridge));

        // Allowed to differ:
        assert_ne!(claim.origin, bridge.origin);
        assert_ne!(claim.nullifier_domain, bridge.nullifier_domain);
        assert_eq!(claim.claim_id, [0u8; 32]);
        assert_ne!(bridge.claim_id, [0u8; 32]);

        // Same type / field set — not byte-identical
        assert_ne!(claim.to_bytes(), bridge.to_bytes());
    }
}
