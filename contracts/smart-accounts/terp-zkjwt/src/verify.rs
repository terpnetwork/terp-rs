//! ZK-JWT verification: structural envelope + CosmWasm **host** proof verify.
//!
//! ## Host API (feature `zk-host`)
//!
//! ```ignore
//! deps.api.proof_instance_verify(zkid, proof, public_inputs)?
//! ```
//!
//! - `zkid`: application-assigned circuit id (issuer config)
//! - `proof`: Halo2 (or registered scheme) proof bytes
//! - `public_inputs` / instances: fixed layout in [`crate::instances`]
//!
//! Architecture: CosmWasm `ZK_PROOF_VERIFICATION_ARCHITECTURE.md`
//! (`zkid → checksum → VK` in app state + VM cache).
//!
//! Without `zk-host`, [`StructuralZkJwtVerifier`] only checks envelopes (dev/CI).

use cosmwasm_std::{Binary, Deps};

use crate::error::ContractError;
use crate::instances::{parse_public_instances, CLAIM_COMMITMENT_LEN};
use crate::msg::{IssuerConfig, ZkJwtAuthPayload};

pub mod circuit_ids {
    pub const JWT_MEMBERSHIP: &str = "zkjwt.membership.v1";
    pub const JWT_EMAIL_DOMAIN: &str = "zkjwt.email_domain.v1";
    pub const JWT_PROFILE_LINK: &str = "zkjwt.profile_link.v1";
    pub const JWT_PASSKEY_P256: &str = "zkjwt.passkey.p256.v1";
    /// Start-hash Flock (BLAKE3 of public instances). Demo authenticator until Axiom store works.
    pub const JWT_PASSKEY_FLOCK: &str = "zkjwt.passkey.flock.v1";
}

pub trait ZkJwtVerifier {
    fn verify(
        &self,
        deps: Deps,
        issuer: &IssuerConfig,
        payload: &ZkJwtAuthPayload,
    ) -> Result<(), ContractError>;
}

fn check_envelope(issuer: &IssuerConfig, payload: &ZkJwtAuthPayload) -> Result<(), ContractError> {
    if payload.proof.is_empty() {
        return Err(ContractError::InvalidProof {
            reason: "empty proof".into(),
        });
    }
    if payload.claim_commitment.len() != CLAIM_COMMITMENT_LEN
        && !(payload.claim_commitment.len() > 0 && payload.claim_commitment.len() <= 64)
    {
        // allow 1..=64 during migration; prefer 32
        if payload.claim_commitment.is_empty() || payload.claim_commitment.len() > 64 {
            return Err(ContractError::InvalidProof {
                reason: "claim_commitment length invalid".into(),
            });
        }
    }

    // Circom full Fr vector (host Path A): ≥27 limbs → policy from codec, not fixed offsets.
    if crate::circom_codec::is_circom_host_instances(payload.public_inputs.as_slice()) {
        let claim = crate::circom_codec::decode_host_instances_bytes(payload.public_inputs.as_slice())?;
        if payload.claim_commitment.len() == CLAIM_COMMITMENT_LEN
            && claim.claim_commitment.as_slice() != payload.claim_commitment.as_slice()
        {
            return Err(ContractError::InvalidProof {
                reason: "claim_commitment does not match circom accountSalt (codec v1 idx 26)"
                    .into(),
            });
        }
        // Inclusion root is not in stock circom layout; only error if issuer requires root.
        if let Some(root) = &issuer.inclusion_set_root {
            if root.len() == 32 {
                return Err(ContractError::InvalidProof {
                    reason: "issuer requires inclusion_set_root but circom-jwt-v1 PI has no root slot (D3)"
                        .into(),
                });
            }
        }
    } else {
        let inst = parse_public_instances(&payload.public_inputs)?;
        // claim_commitment field must match instances[32..64)
        if payload.claim_commitment.len() == CLAIM_COMMITMENT_LEN
            && inst.claim_commitment != payload.claim_commitment.as_slice()
        {
            return Err(ContractError::InvalidProof {
                reason: "claim_commitment does not match public_inputs[32..64]".into(),
            });
        }
        // If issuer registered an inclusion root, PI must carry it (full check in on_auth_request)
        if let Some(root) = &issuer.inclusion_set_root {
            if root.len() == 32 && inst.inclusion_set_root.is_none() {
                return Err(ContractError::InvalidProof {
                    reason: "issuer requires inclusion_set_root in public_inputs".into(),
                });
            }
        }
    }
    match payload.circuit_id.as_str() {
        circuit_ids::JWT_MEMBERSHIP
        | circuit_ids::JWT_EMAIL_DOMAIN
        | circuit_ids::JWT_PROFILE_LINK
        | circuit_ids::JWT_PASSKEY_P256
        | circuit_ids::JWT_PASSKEY_FLOCK => {}
        other => {
            return Err(ContractError::InvalidProof {
                reason: format!("unknown circuit_id: {other}"),
            });
        }
    }
    let _ = issuer;
    Ok(())
}

/// Dev/CI verifier: layout only (no host crypto).
pub struct StructuralZkJwtVerifier;

impl ZkJwtVerifier for StructuralZkJwtVerifier {
    fn verify(
        &self,
        _deps: Deps,
        issuer: &IssuerConfig,
        payload: &ZkJwtAuthPayload,
    ) -> Result<(), ContractError> {
        if issuer.verifying_key.is_empty() && issuer.zkid.is_none() {
            return Err(ContractError::InvalidProof {
                reason: "issuer has neither verifying_key nor zkid".into(),
            });
        }
        check_envelope(issuer, payload)
    }
}

/// Production path: CosmWasm VM `proof_instance_verify`.
///
/// Requires `cosmwasm-std` built with feature `zk` and a chain that registered `zkid`.
#[cfg(feature = "zk-host")]
pub struct HostZkJwtVerifier;

#[cfg(feature = "zk-host")]
impl ZkJwtVerifier for HostZkJwtVerifier {
    fn verify(
        &self,
        deps: Deps,
        issuer: &IssuerConfig,
        payload: &ZkJwtAuthPayload,
    ) -> Result<(), ContractError> {
        check_envelope(issuer, payload)?;
        let zkid = issuer.zkid.ok_or_else(|| ContractError::InvalidProof {
            reason: "issuer.zkid required for host proof_instance_verify".into(),
        })?;
        match deps
            .api
            .proof_instance_verify(zkid, payload.proof.as_slice(), payload.public_inputs.as_slice())
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(ContractError::InvalidProof {
                reason: "proof_instance_verify returned false".into(),
            }),
            Err(e) => Err(ContractError::InvalidProof {
                reason: format!("proof_instance_verify error: {e}"),
            }),
        }
    }
}

/// Select verifier: host when feature enabled, else structural.
pub fn default_verifier() -> impl ZkJwtVerifier {
    #[cfg(feature = "zk-host")]
    {
        HostZkJwtVerifier
    }
    #[cfg(not(feature = "zk-host"))]
    {
        StructuralZkJwtVerifier
    }
}

// re-export name used by lib
pub use StructuralZkJwtVerifier as DefaultZkJwtVerifier;

/// @deprecated use instances::nullifier_bytes — kept for callers
pub fn nullifier_from_public_inputs(public_inputs: &Binary) -> Result<Vec<u8>, ContractError> {
    crate::instances::nullifier_bytes(public_inputs)
}

#[cfg(all(test, feature = "zk-host"))]
mod host_tests {
    use super::*;
    use crate::instances::build_public_inputs;
    use cosmwasm_std::testing::mock_dependencies;
    use cosmwasm_std::Binary;

    #[test]
    fn host_verifier_calls_proof_instance_verify() {
        let deps = mock_dependencies();
        let claim = [1u8; 32];
        let nf = [2u8; 32];
        let root = [0xABu8; 32];
        let issuer = IssuerConfig {
            issuer: "https://idp.example".into(),
            verifying_key: Binary::from(b"vk"),
            zkid: Some(42),
            audience: None,
            inclusion_set_root: Some(Binary::from(root)),
        };
        let payload = ZkJwtAuthPayload {
            issuer: issuer.issuer.clone(),
            claim_commitment: Binary::from(claim),
            public_inputs: build_public_inputs(&nf, &claim, Some(&root), None),
            proof: Binary::from(b"proof-bytes"),
            circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
        };
        let err = HostZkJwtVerifier
            .verify(deps.as_ref(), &issuer, &payload)
            .expect_err("unregistered zkid must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("proof_instance_verify") || msg.contains("invalid zk-jwt"),
            "got: {msg}"
        );
    }

    #[test]
    fn host_verifier_requires_zkid() {
        let deps = mock_dependencies();
        let claim = [1u8; 32];
        let nf = [2u8; 32];
        let issuer = IssuerConfig {
            issuer: "https://idp.example".into(),
            verifying_key: Binary::from(b"vk"),
            zkid: None,
            audience: None,
            inclusion_set_root: None,
        };
        let payload = ZkJwtAuthPayload {
            issuer: issuer.issuer.clone(),
            claim_commitment: Binary::from(claim),
            public_inputs: build_public_inputs(&nf, &claim, None, None),
            proof: Binary::from(b"proof"),
            circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
        };
        let err = HostZkJwtVerifier
            .verify(deps.as_ref(), &issuer, &payload)
            .expect_err("zkid required");
        assert!(
            err.to_string().contains("zkid"),
            "got: {err}"
        );
    }
}
