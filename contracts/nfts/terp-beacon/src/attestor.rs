use crate::error::ContractError;
use cosmwasm_std::Api;
use sha2::{Digest, Sha256};

/// Domain separator for attestor signatures. The signed hash is
/// `SHA-256(ATTESTOR_DOMAIN || round_be_u64 || randomness_32)`.
///
/// `randomness_32` is the raw drand randomness (`sha256(drand_signature)`), not hex.
/// The signature is 64-byte secp256k1 compact `r || s` over that hash.
/// The public key is 33-byte SEC1 compressed.
pub const ATTESTOR_DOMAIN: &[u8] = b"terp-beacon/attestor/v1\n";

pub const SECP256K1_PUBKEY_LEN: usize = 33;
pub const SECP256K1_SIG_LEN: usize = 64;
pub const RANDOMNESS_LEN: usize = 32;

pub fn attestor_message_hash(round: u64, randomness: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ATTESTOR_DOMAIN);
    hasher.update(round.to_be_bytes());
    hasher.update(randomness);
    hasher.finalize().into()
}

pub fn verify_attestor_signature(
    api: &dyn Api,
    round: u64,
    randomness: &[u8],
    pubkey: &[u8],
    signature: &[u8],
) -> Result<(), ContractError> {
    if pubkey.len() != SECP256K1_PUBKEY_LEN || (pubkey[0] != 0x02 && pubkey[0] != 0x03) {
        return Err(ContractError::InvalidAttestorPubkey);
    }
    if signature.len() != SECP256K1_SIG_LEN || randomness.len() != RANDOMNESS_LEN {
        return Err(ContractError::InvalidAttestation);
    }
    let hash = attestor_message_hash(round, randomness);
    match api.secp256k1_verify(&hash, signature, pubkey) {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => Err(ContractError::InvalidAttestation),
    }
}
