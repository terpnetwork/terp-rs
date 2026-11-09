//! Idiomatic CosmWasm ed25519 verification helpers.
//!
//! Wire format (matches `cosmwasm_std::Api` / `cosmwasm_crypto`):
//! - **public key**: 32 bytes (raw Ed25519, not compressed flag)
//! - **signature**: 64 bytes (R || S)
//! - **message**: arbitrary-length preimage (here: `sign_mode_direct` bytes, NOT pre-hashed)
//!
//! Single verify:
//! ```ignore
//! deps.api.ed25519_verify(message, signature, public_key)?
//! ```
//!
//! Batch verify (variable-time; still constant-format inputs):
//! ```ignore
//! deps.api.ed25519_batch_verify(&messages, &signatures, &public_keys)?
//! ```
//! Batch rules (from CosmWasm / ed25519-zebra):
//! - `messages.len() == signatures.len() == public_keys.len()`, **or**
//! - `messages.len() == 1` and `signatures.len() == public_keys.len()` (one msg, many sigs/keys), **or**
//! - `public_keys.len() == 1` and `messages.len() == signatures.len()` (one key, many msgs/sigs)
//!
//! Our authenticator uses the **one message, N signatures/keys** form for multi-sig,
//! and N messages with N keys when the payload supplies per-item messages.

use cosmwasm_std::{Api, Binary};

use crate::error::ContractError;

pub const ED25519_PUBKEY_LEN: usize = 32;
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// Validate raw Ed25519 public key length before calling the host (gives clearer errors).
pub fn require_pubkey(pk: &[u8]) -> Result<(), ContractError> {
    if pk.len() != ED25519_PUBKEY_LEN {
        return Err(ContractError::InvalidPubkey {
            reason: format!("expected {ED25519_PUBKEY_LEN} bytes, got {}", pk.len()),
        });
    }
    Ok(())
}

pub fn require_signature(sig: &[u8]) -> Result<(), ContractError> {
    if sig.len() != ED25519_SIGNATURE_LEN {
        return Err(ContractError::BadSignature {
            reason: format!(
                "expected {ED25519_SIGNATURE_LEN}-byte signature, got {}",
                sig.len()
            ),
        });
    }
    Ok(())
}

/// Single-signature verify via host import `ed25519_verify`.
pub fn verify_single(
    api: &dyn Api,
    message: &[u8],
    signature: &[u8],
    public_key: &[u8],
) -> Result<(), ContractError> {
    require_pubkey(public_key)?;
    require_signature(signature)?;
    match api.ed25519_verify(message, signature, public_key) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ContractError::BadSignature {
            reason: "ed25519_verify returned false".into(),
        }),
        Err(e) => Err(ContractError::Verification(e)),
    }
}

/// Batch verify via host import `ed25519_batch_verify`.
///
/// Callers must already enforce length rules between slices.
pub fn verify_batch(
    api: &dyn Api,
    messages: &[&[u8]],
    signatures: &[&[u8]],
    public_keys: &[&[u8]],
) -> Result<(), ContractError> {
    if messages.is_empty() || signatures.is_empty() || public_keys.is_empty() {
        return Err(ContractError::BadSignature {
            reason: "batch verify requires non-empty messages, signatures, public_keys".into(),
        });
    }
    for pk in public_keys {
        require_pubkey(pk)?;
    }
    for sig in signatures {
        require_signature(sig)?;
    }
    // CosmWasm accepts (n,n,n), (1,n,n), (n,n,1)
    let n_msg = messages.len();
    let n_sig = signatures.len();
    let n_pk = public_keys.len();
    let shape_ok = (n_msg == n_sig && n_sig == n_pk)
        || (n_msg == 1 && n_sig == n_pk)
        || (n_pk == 1 && n_msg == n_sig);
    if !shape_ok {
        return Err(ContractError::BadSignature {
            reason: format!(
                "invalid batch shape messages={n_msg} signatures={n_sig} public_keys={n_pk}"
            ),
        });
    }

    match api.ed25519_batch_verify(messages, signatures, public_keys) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ContractError::BadSignature {
            reason: "ed25519_batch_verify returned false".into(),
        }),
        Err(e) => Err(ContractError::Verification(e)),
    }
}

/// Payload in `AuthenticationRequest.signature` for multi-sig / batch cases.
///
/// **Single-sig idiom (preferred when only one key):** raw 64-byte `Binary` signature
/// (not JSON) — see `parse_auth_signature`.
///
/// **Batch idiom:** JSON of this struct.
#[cosmwasm_schema::cw_serde]
pub struct Ed25519AuthPayload {
    /// If empty/None, each signature verifies the same `sign_mode_direct` message.
    /// If set, length must equal `signatures.len()` (n,n,n or n,n,1 shapes).
    pub messages: Option<Vec<Binary>>,
    /// Each entry must be 64 bytes.
    pub signatures: Vec<Binary>,
    /// If None, the contract's registered pubkey is used for every signature
    /// (`messages` must be len 1 or match signatures; host shape `(1,n,1)` invalid —
    /// we expand registered key to N copies for `(1,n,n)`).
    pub public_keys: Option<Vec<Binary>>,
}

/// Result of decoding `AuthenticationRequest.signature`.
pub enum ParsedAuth {
    /// Raw 64-byte signature over default message (sign_mode_direct).
    Single { signature: Binary },
    /// Explicit batch payload.
    Batch(Ed25519AuthPayload),
}

pub fn parse_auth_signature(raw: &Binary) -> Result<ParsedAuth, ContractError> {
    // Fast path: raw ed25519 signature (exactly 64 bytes) — not valid JSON object
    if raw.len() == ED25519_SIGNATURE_LEN {
        return Ok(ParsedAuth::Single {
            signature: raw.clone(),
        });
    }
    // JSON batch / multi-sig payload
    match cosmwasm_std::from_json::<Ed25519AuthPayload>(raw) {
        Ok(p) if !p.signatures.is_empty() => Ok(ParsedAuth::Batch(p)),
        Ok(_) => Err(ContractError::BadSignature {
            reason: "batch payload has empty signatures".into(),
        }),
        Err(e) => Err(ContractError::BadSignature {
            reason: format!("signature must be 64 raw bytes or Ed25519AuthPayload JSON: {e}"),
        }),
    }
}
