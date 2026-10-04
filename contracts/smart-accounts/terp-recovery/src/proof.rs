//! Optional preimage-proof arm on the recovery authenticator.
//!
//! Public inputs must start with the break-glass challenge produced by the
//! configured [`crate::RecoveryHashAlg`]. A non-empty proof envelope is not
//! accepted by itself.
//!
//! With the `zk-host` feature, the contract calls `Api::proof_instance_verify`.
//! Without that feature the arm is fail-closed, including when a witness of
//! the public challenge preimage is attached. Guardian approvals stay the
//! authenticating path.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Api, Binary};

use crate::error::ContractError;
use crate::hash::rehash;
use crate::RecoveryConfig;

/// Circuit id for a proof whose first public bytes are the recovery challenge.
pub const CIRCUIT_ID_RECOVERY_PREIMAGE: &str = "recovery.preimage.v1";

/// Proof attached to [`crate::RecoveryAuthPayload`].
#[cw_serde]
pub struct PreimageProof {
    pub circuit_id: String,
    /// Circuit id registered with the host. Required when `zk-host` is enabled.
    pub zkid: Option<u64>,
    pub proof: Binary,
    /// Challenge digest || optional circuit-specific bytes.
    pub public_inputs: Binary,
    /// Optional preimage. Checked against the configured hash when present.
    /// Matching the public preimage does not authenticate on its own.
    pub witness_preimage: Option<Binary>,
}

pub fn verify_preimage_proof(
    api: &dyn Api,
    cfg: &RecoveryConfig,
    challenge: &[u8],
    proof: &PreimageProof,
) -> Result<(), ContractError> {
    if proof.proof.is_empty() {
        return Err(ContractError::InvalidProof {
            reason: "empty proof".into(),
        });
    }
    if proof.circuit_id != CIRCUIT_ID_RECOVERY_PREIMAGE {
        return Err(ContractError::InvalidProof {
            reason: format!("unknown circuit_id: {}", proof.circuit_id),
        });
    }
    if challenge.is_empty()
        || proof.public_inputs.len() < challenge.len()
        || !proof.public_inputs.as_slice().starts_with(challenge)
    {
        return Err(ContractError::InvalidProof {
            reason: "public_inputs do not start with the break-glass challenge".into(),
        });
    }
    if let Some(pre) = &proof.witness_preimage {
        let got = rehash(cfg.hash_alg, pre.as_slice(), cfg.rehash_rounds)?;
        if got.as_slice() != challenge {
            return Err(ContractError::InvalidProof {
                reason: "witness digest != challenge".into(),
            });
        }
    }

    #[cfg(feature = "zk-host")]
    {
        let zkid = proof.zkid.ok_or_else(|| ContractError::InvalidProof {
            reason: "zkid required for proof_instance_verify".into(),
        })?;
        match api.proof_instance_verify(zkid, proof.proof.as_slice(), proof.public_inputs.as_slice())
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(ContractError::InvalidProof {
                reason: "proof_instance_verify returned false".into(),
            }),
            Err(e) => Err(ContractError::InvalidProof {
                reason: format!("proof_instance_verify: {e}"),
            }),
        }
    }
    #[cfg(not(feature = "zk-host"))]
    {
        let _ = api;
        Err(ContractError::ProofNotLinked {})
    }
}
