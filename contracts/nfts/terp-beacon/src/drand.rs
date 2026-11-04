use crate::error::ContractError;
use crate::msg::DrandScheme;
use crate::state::Config;
use cosmwasm_std::{HexBinary, Timestamp};
use drand_verify::{G1Pubkey, G2PubkeyRfc, Pubkey, derive_randomness};

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

pub fn ensure_pubkey(config: &Config) -> Result<(), ContractError> {
    match config.scheme {
        DrandScheme::PedersenBlsChained => {
            let pk_bytes: [u8; 48] = config
                .pubkey
                .as_slice()
                .try_into()
                .map_err(|_| ContractError::InvalidPubkey)?;
            G1Pubkey::from_fixed(pk_bytes).map_err(|_| ContractError::InvalidPubkey)?;
        }
        DrandScheme::BlsUnchainedG1Rfc9380 => {
            let pk_bytes: [u8; 96] = config
                .pubkey
                .as_slice()
                .try_into()
                .map_err(|_| ContractError::InvalidPubkey)?;
            G2PubkeyRfc::from_fixed(pk_bytes).map_err(|_| ContractError::InvalidPubkey)?;
        }
    }
    Ok(())
}

pub fn verify_round(
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
            let pk_bytes: [u8; 48] = config
                .pubkey
                .as_slice()
                .try_into()
                .map_err(|_| ContractError::InvalidPubkey)?;
            let pk = G1Pubkey::from_fixed(pk_bytes).map_err(|_| ContractError::InvalidPubkey)?;
            pk.verify(round, previous_signature, signature)
        }
        DrandScheme::BlsUnchainedG1Rfc9380 => {
            if !previous_signature.is_empty() {
                return Err(ContractError::UnexpectedPreviousSignature);
            }
            let pk_bytes: [u8; 96] = config
                .pubkey
                .as_slice()
                .try_into()
                .map_err(|_| ContractError::InvalidPubkey)?;
            let pk = G2PubkeyRfc::from_fixed(pk_bytes).map_err(|_| ContractError::InvalidPubkey)?;
            pk.verify(round, b"", signature)
        }
    };
    match verified {
        Ok(true) => {}
        Ok(false) | Err(_) => return Err(ContractError::InvalidSignature),
    }
    Ok(HexBinary::from(derive_randomness(signature)))
}
