//! Fixed public-instance layout for JWT circuits (v1.1 — inclusion root).
//!
//! See `JWT_AUTH_FLOW.md` for the full authentication story.
//!
//! ```text
//! public_inputs =
//!   nullifier(32) || claim_commitment(32)
//!   || [inclusion_set_root(32)]   // required when IssuerConfig.inclusion_set_root is set
//!   || [msg_bind(32)]            // optional host msg binding
//!   || rest…
//! ```

use cosmwasm_std::Binary;
use sha2::{Digest, Sha256};

use crate::error::ContractError;

pub const NULLIFIER_LEN: usize = 32;
pub const CLAIM_COMMITMENT_LEN: usize = 32;
pub const INCLUSION_ROOT_LEN: usize = 32;
pub const MSG_BIND_LEN: usize = 32;

/// Minimum instances: nullifier || claim_commitment
pub const MIN_INSTANCES_LEN: usize = NULLIFIER_LEN + CLAIM_COMMITMENT_LEN;
/// With inclusion set root (issuer-registered merkle / membership root)
pub const WITH_INCLUSION_ROOT_LEN: usize = MIN_INSTANCES_LEN + INCLUSION_ROOT_LEN;
/// With inclusion root + msg_bind
pub const WITH_ROOT_AND_MSG_BIND_LEN: usize = WITH_INCLUSION_ROOT_LEN + MSG_BIND_LEN;

/// Domain for host-side msg binding (circuit should use the same string).
pub const MSG_BIND_DOMAIN: &[u8] = b"terp-auth/zkjwt/v1";

/// Parsed view over `public_inputs` bytes.
#[derive(Debug, Clone)]
pub struct JwtPublicInstances<'a> {
    pub nullifier: &'a [u8],
    pub claim_commitment: &'a [u8],
    /// Public inclusion-set root (merkle root of allowed subjects / JWKS epoch / mesh set).
    /// Present when `public_inputs.len() >= WITH_INCLUSION_ROOT_LEN`.
    pub inclusion_set_root: Option<&'a [u8]>,
    /// Optional msg_bind at [96..128) when root present, or [64..96) in legacy short form.
    pub msg_bind: Option<&'a [u8]>,
    pub rest: &'a [u8],
}

pub fn parse_public_instances(public_inputs: &Binary) -> Result<JwtPublicInstances<'_>, ContractError> {
    let b = public_inputs.as_slice();
    if b.len() < MIN_INSTANCES_LEN {
        return Err(ContractError::InvalidProof {
            reason: format!(
                "public_inputs len {} < minimum {MIN_INSTANCES_LEN} (nullifier||claim_commitment)",
                b.len()
            ),
        });
    }

    let nullifier = &b[0..NULLIFIER_LEN];
    let claim_commitment = &b[NULLIFIER_LEN..MIN_INSTANCES_LEN];

    // Preferred layout with inclusion root at [64..96)
    if b.len() >= WITH_INCLUSION_ROOT_LEN {
        let inclusion_set_root = &b[MIN_INSTANCES_LEN..WITH_INCLUSION_ROOT_LEN];
        let (msg_bind, rest) = if b.len() >= WITH_ROOT_AND_MSG_BIND_LEN {
            (
                Some(&b[WITH_INCLUSION_ROOT_LEN..WITH_ROOT_AND_MSG_BIND_LEN]),
                &b[WITH_ROOT_AND_MSG_BIND_LEN..],
            )
        } else {
            (None, &b[WITH_INCLUSION_ROOT_LEN..])
        };
        return Ok(JwtPublicInstances {
            nullifier,
            claim_commitment,
            inclusion_set_root: Some(inclusion_set_root),
            msg_bind,
            rest,
        });
    }

    // Legacy short form: only nullifier||claim (no root) — allowed only if issuer has no root
    Ok(JwtPublicInstances {
        nullifier,
        claim_commitment,
        inclusion_set_root: None,
        msg_bind: None,
        rest: &b[MIN_INSTANCES_LEN..],
    })
}

pub fn nullifier_bytes(public_inputs: &Binary) -> Result<Vec<u8>, ContractError> {
    Ok(parse_public_instances(public_inputs)?.nullifier.to_vec())
}

/// Build a test/demo public_inputs blob.
pub fn build_public_inputs(
    nullifier: &[u8; 32],
    claim_commitment: &[u8; 32],
    inclusion_set_root: Option<&[u8; 32]>,
    msg_bind: Option<&[u8; 32]>,
) -> Binary {
    let mut v = Vec::with_capacity(128);
    v.extend_from_slice(nullifier);
    v.extend_from_slice(claim_commitment);
    if let Some(root) = inclusion_set_root {
        v.extend_from_slice(root);
        if let Some(mb) = msg_bind {
            v.extend_from_slice(mb);
        }
    }
    Binary::from(v)
}

/// Host-computed message binding (for circuits that include msg_bind).
pub fn compute_msg_bind(
    chain_id: &str,
    account: &str,
    authenticator_id: &str,
    msg_index: u64,
    sign_mode_direct: &[u8],
) -> [u8; 32] {
    let mut inner = Sha256::new();
    inner.update(sign_mode_direct);
    let msg_hash = inner.finalize();

    let mut h = Sha256::new();
    h.update(MSG_BIND_DOMAIN);
    h.update(chain_id.as_bytes());
    h.update([0u8]);
    h.update(account.as_bytes());
    h.update([0u8]);
    h.update(authenticator_id.as_bytes());
    h.update([0u8]);
    h.update(msg_index.to_le_bytes());
    h.update(msg_hash);
    h.finalize().into()
}

/// Compare circuit instance[3] to [`compute_msg_bind`].
///
/// Accepts raw SHA-256 or the BN254 field reduction of that digest
/// (big-endian integer mod r, 32-byte little-endian limb).
pub fn msg_bind_matches(instance32: &[u8], expected: &[u8; 32]) -> bool {
    if instance32 == expected.as_slice() {
        return true;
    }
    if instance32.len() != 32 {
        return false;
    }
    const R_BE: [u8; 32] = [
        0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6,
        0x81, 0x81, 0x58, 0x5d, 0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91,
        0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
    ];
    let mut n = [0u8; 32];
    n.copy_from_slice(expected);
    while n.as_slice() >= R_BE.as_slice() {
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = n[i] as i16 - R_BE[i] as i16 - borrow;
            if d < 0 {
                n[i] = (d + 256) as u8;
                borrow = 1;
            } else {
                n[i] = d as u8;
                borrow = 0;
            }
        }
    }
    let mut le = n;
    le.reverse();
    instance32 == le.as_slice()
}
