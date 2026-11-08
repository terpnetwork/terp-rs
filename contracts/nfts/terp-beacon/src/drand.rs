use crate::error::ContractError;
use crate::msg::DrandScheme;
use crate::state::Config;
use cosmwasm_std::{
    Api, BLS12_381_G1_GENERATOR, BLS12_381_G2_GENERATOR, HashFunction, HexBinary, Timestamp,
};
use sha2::{Digest, Sha256};

/// RFC 9380 hash-to-G2 suite used by drand for G2 signatures (pedersen-bls-chained).
/// Same constant as drand-verify `DOMAIN_HASH_TO_G2`.
const DST_G2: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
/// RFC 9380 hash-to-G1 suite used by drand for G1 signatures (bls-unchained-g1-rfc9380, quicknet).
/// Same constant as drand-verify `DOMAIN_HASH_TO_G1`.
const DST_G1: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_NUL_";

/// drand beacon message: `sha256(previous_signature || round_be)`.
/// Unchained schemes pass an empty `previous_signature`, giving `sha256(round_be)`.
fn beacon_message(round: u64, previous_signature: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(previous_signature);
    hasher.update(round.to_be_bytes());
    hasher.finalize().into()
}

/// First round whose publish time is strictly after `after`.
///
/// Same integer rule as nois `round_after`: round 1 when `after` is before
/// genesis, otherwise `floor((after - genesis) / period) + 2`.
pub fn round_after(
    genesis: Timestamp,
    period_seconds: u64,
    after: Timestamp,
) -> Result<u64, ContractError> {
    if period_seconds == 0 {
        return Err(ContractError::InvalidPeriod);
    }
    if after < genesis {
        return Ok(1);
    }
    let period_ns = period_seconds
        .checked_mul(1_000_000_000)
        .ok_or(ContractError::RoundTimeOverflow)?;
    let from_genesis = after
        .nanos()
        .checked_sub(genesis.nanos())
        .ok_or(ContractError::RoundTimeOverflow)?;
    let periods = from_genesis / period_ns;
    periods
        .checked_add(2)
        .ok_or(ContractError::RoundTimeOverflow)
}

/// Drand round time: `genesis + (round - 1) * period`.
pub fn round_publish_time(
    genesis: Timestamp,
    period_seconds: u64,
    round: u64,
) -> Result<Timestamp, ContractError> {
    if round == 0 {
        return Err(ContractError::RoundZero);
    }
    let period_ns = period_seconds
        .checked_mul(1_000_000_000)
        .ok_or(ContractError::RoundTimeOverflow)?;
    let delta = (round - 1)
        .checked_mul(period_ns)
        .ok_or(ContractError::RoundTimeOverflow)?;
    let nanos = genesis
        .nanos()
        .checked_add(delta)
        .ok_or(ContractError::RoundTimeOverflow)?;
    Ok(Timestamp::from_nanos(nanos))
}

/// Validates the configured public key with the host BLS API.
///
/// Aggregating a single point decodes it and checks curve and subgroup
/// membership, so an invalid encoding is rejected at instantiate.
pub fn ensure_pubkey(api: &dyn Api, config: &Config) -> Result<(), ContractError> {
    let valid = match config.scheme {
        DrandScheme::PedersenBlsChained => {
            config.pubkey.len() == 48 && api.bls12_381_aggregate_g1(&config.pubkey).is_ok()
        }
        DrandScheme::BlsUnchainedG1Rfc9380 => {
            config.pubkey.len() == 96 && api.bls12_381_aggregate_g2(&config.pubkey).is_ok()
        }
    };
    if !valid {
        return Err(ContractError::InvalidPubkey);
    }
    Ok(())
}

/// Verifies a drand beacon with the host BLS12-381 API and returns
/// `sha256(signature)` as the round randomness.
///
/// - pedersen-bls-chained (mainnet default): G1 pubkey, G2 signature,
///   `e(g1, sig) == e(pk, H2(sha256(prev_sig || round_be)))`.
/// - bls-unchained-g1-rfc9380 (quicknet): G2 pubkey, G1 signature,
///   `e(sig, g2) == e(H1(sha256(round_be)), pk)`.
pub fn verify_round(
    api: &dyn Api,
    config: &Config,
    round: u64,
    signature: &[u8],
    previous_signature: &[u8],
) -> Result<HexBinary, ContractError> {
    if round == 0 {
        return Err(ContractError::RoundZero);
    }
    let verified = match config.scheme {
        DrandScheme::PedersenBlsChained => {
            if previous_signature.is_empty() {
                return Err(ContractError::MissingPreviousSignature);
            }
            if config.pubkey.len() != 48 {
                return Err(ContractError::InvalidPubkey);
            }
            if signature.len() != 96 {
                return Err(ContractError::InvalidSignature);
            }
            let msg = beacon_message(round, previous_signature);
            let msg_g2 = api
                .bls12_381_hash_to_g2(HashFunction::Sha256, &msg, DST_G2)
                .map_err(|_| ContractError::InvalidSignature)?;
            api.bls12_381_pairing_equality(
                &BLS12_381_G1_GENERATOR,
                signature,
                &config.pubkey,
                &msg_g2,
            )
        }
        DrandScheme::BlsUnchainedG1Rfc9380 => {
            if !previous_signature.is_empty() {
                return Err(ContractError::UnexpectedPreviousSignature);
            }
            if config.pubkey.len() != 96 {
                return Err(ContractError::InvalidPubkey);
            }
            if signature.len() != 48 {
                return Err(ContractError::InvalidSignature);
            }
            let msg = beacon_message(round, b"");
            let msg_g1 = api
                .bls12_381_hash_to_g1(HashFunction::Sha256, &msg, DST_G1)
                .map_err(|_| ContractError::InvalidSignature)?;
            api.bls12_381_pairing_equality(
                signature,
                &BLS12_381_G2_GENERATOR,
                &msg_g1,
                &config.pubkey,
            )
        }
    };
    match verified {
        Ok(true) => {}
        Ok(false) | Err(_) => return Err(ContractError::InvalidSignature),
    }
    let randomness: [u8; 32] = Sha256::digest(signature).into();
    Ok(HexBinary::from(randomness))
}
