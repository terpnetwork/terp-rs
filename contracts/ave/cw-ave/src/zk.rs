//! Halo2 Pasta verify via zk-wasmvm `Api::proof_instance_verify(zkid, proof, instances)`.
//! Instances are concatenated 32-byte LE Pasta Fp limbs: guest_root || nullifier || event_id.

use cosmwasm_std::{Api, Binary, VerificationError};

pub fn verify_halo2_proof(
    api: &dyn Api,
    zkid: u64,
    proof: &[u8],
    public_inputs: &[Binary],
) -> Result<bool, VerificationError> {
    if zkid == 0 {
        return Err(VerificationError::GenericErr);
    }
    let mut instances = Vec::with_capacity(public_inputs.iter().map(|b| b.len()).sum());
    for p in public_inputs {
        instances.extend_from_slice(p);
    }
    api.proof_instance_verify(zkid, proof, &instances)
}
