//! BLS12-381 signature checks.
//!
//! Min-pk (public key on G1, signature on G2):
//! `e(G1, sig) = e(pk, H_G2(msg))`.
//!
//! Min-sig (public key on G2, signature on G1):
//! `e(sig, G2) = e(H_G1(msg), pk)`.
//!
//! Each registered key is checked on its own. Signers of one `sign_mode_direct`
//! message are counted toward the threshold. Keys are not aggregated, so a
//! caller cannot cancel a honest key with a rogue sum.

use cosmwasm_std::{Api, HashFunction, BLS12_381_G1_GENERATOR, BLS12_381_G2_GENERATOR};

use crate::error::ContractError;
use crate::msg::BlsVariant;

const DST_MIN_PK: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";
const DST_MIN_SIG: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

pub fn verify_one(
    api: &dyn Api,
    variant: &BlsVariant,
    pubkey: &[u8],
    signature: &[u8],
    message: &[u8],
) -> Result<(), ContractError> {
    if pubkey.len() != variant.pubkey_len() {
        return Err(ContractError::InvalidSignature {
            reason: format!(
                "pubkey len {} != {}",
                pubkey.len(),
                variant.pubkey_len()
            ),
        });
    }
    if signature.len() != variant.signature_len() {
        return Err(ContractError::InvalidSignature {
            reason: format!(
                "signature len {} != {}",
                signature.len(),
                variant.signature_len()
            ),
        });
    }
    if is_infinity(pubkey) || is_infinity(signature) {
        return Err(ContractError::InvalidSignature {
            reason: "infinity point".into(),
        });
    }
    if message.is_empty() {
        return Err(ContractError::InvalidSignature {
            reason: "empty sign_mode_direct".into(),
        });
    }

    let ok = match variant {
        BlsVariant::MinPk => {
            let hashed = api.bls12_381_hash_to_g2(HashFunction::Sha256, message, DST_MIN_PK)?;
            api.bls12_381_pairing_equality(
                &BLS12_381_G1_GENERATOR,
                signature,
                pubkey,
                &hashed,
            )?
        }
        BlsVariant::MinSig => {
            let hashed = api.bls12_381_hash_to_g1(HashFunction::Sha256, message, DST_MIN_SIG)?;
            api.bls12_381_pairing_equality(
                signature,
                &BLS12_381_G2_GENERATOR,
                &hashed,
                pubkey,
            )?
        }
    };
    if ok {
        Ok(())
    } else {
        Err(ContractError::InvalidSignature {
            reason: "pairing rejected".into(),
        })
    }
}

/// Compressed infinity has the infinity bit set (0x40) next to the compression bit.
fn is_infinity(point: &[u8]) -> bool {
    point.first().is_some_and(|b| b & 0x40 != 0)
}

pub fn validate_key(api: &dyn Api, variant: &BlsVariant, key: &[u8]) -> Result<(), ContractError> {
    if key.len() != variant.pubkey_len() {
        return Err(ContractError::InvalidConfig {
            reason: format!("key len {} != {}", key.len(), variant.pubkey_len()),
        });
    }
    if is_infinity(key) {
        return Err(ContractError::InvalidConfig {
            reason: "infinity public key".into(),
        });
    }
    // Subgroup check via the host aggregate of a single point.
    let checked = match variant {
        BlsVariant::MinPk => api
            .bls12_381_aggregate_g1(key)
            .map(|p| p.to_vec())
            .map_err(ContractError::from)?,
        BlsVariant::MinSig => api
            .bls12_381_aggregate_g2(key)
            .map(|p| p.to_vec())
            .map_err(ContractError::from)?,
    };
    if checked.as_slice() != key {
        return Err(ContractError::InvalidConfig {
            reason: "public key is not a canonical point".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bls12_381::{Fr, G1Affine, G1Projective, G2Affine, G2Projective};
    use ark_ec::PrimeGroup;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use cosmwasm_std::testing::MockApi;

    fn sk() -> Fr {
        Fr::from(7u64)
    }

    fn compressed<T: CanonicalSerialize>(p: &T) -> Vec<u8> {
        let mut out = Vec::new();
        p.serialize_compressed(&mut out).unwrap();
        out
    }

    #[test]
    fn min_pk_roundtrip() {
        let api = MockApi::default();
        let secret = sk();
        let pk = compressed(&G1Affine::from(G1Projective::generator() * secret));
        let msg = b"sign-mode-direct";
        let hashed = api
            .bls12_381_hash_to_g2(HashFunction::Sha256, msg, DST_MIN_PK)
            .unwrap();
        let h = G2Affine::deserialize_compressed(&hashed[..]).unwrap();
        let sig = compressed(&G2Affine::from(G2Projective::from(h) * secret));
        verify_one(&api, &BlsVariant::MinPk, &pk, &sig, msg).unwrap();
        let err = verify_one(&api, &BlsVariant::MinPk, &pk, &sig, b"other").unwrap_err();
        assert!(matches!(err, ContractError::InvalidSignature { .. }));
    }

    #[test]
    fn min_sig_roundtrip() {
        let api = MockApi::default();
        let secret = sk();
        let pk = compressed(&G2Affine::from(G2Projective::generator() * secret));
        let msg = b"sign-mode-direct";
        let hashed = api
            .bls12_381_hash_to_g1(HashFunction::Sha256, msg, DST_MIN_SIG)
            .unwrap();
        let h = G1Affine::deserialize_compressed(&hashed[..]).unwrap();
        let sig = compressed(&G1Affine::from(G1Projective::from(h) * secret));
        verify_one(&api, &BlsVariant::MinSig, &pk, &sig, msg).unwrap();
    }

    #[test]
    fn rejects_wrong_lengths() {
        let api = MockApi::default();
        let err = verify_one(&api, &BlsVariant::MinPk, &[0u8; 48], &[0u8; 48], b"m").unwrap_err();
        assert!(matches!(err, ContractError::InvalidSignature { .. }));
    }
}
