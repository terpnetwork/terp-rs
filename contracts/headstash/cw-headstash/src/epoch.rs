//! Halo2 claim epoch: a sidecar lane, a Hashmerchant root, and one mint.
//!
//! The chain does not verify the Halo2 proof. Hashmerchant stores the
//! aggregate of one custody-signed attestation (`headstash-batch`, height 1)
//! whose value is the accepted batch hash. A payout is authorized only when
//! that aggregate matches. `oracle_sources` on the sudo are not a mint source.

use std::collections::HashSet;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    BankMsg, Binary, Coin, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage,
    Uint128,
};
use cw_storage_plus::{Item, Map};
use ff::{Field, FromUniformBytes, PrimeField};
use pasta_curves::arithmetic::CurveExt;
use pasta_curves::group::Curve;
use pasta_curves::pallas::{Point, Scalar};
use sha2::{Digest, Sha256};

use crate::distro::{self, nullifier_storage_key};
use crate::headstash::verify_nullifier_stateful;
use crate::tokenfactory::TokenStrategy;
use crate::{msg_mint, HEADSTASH_CFG, NULLIFIERS};
use pasta_curves::group::GroupEncoding;

pub const CHAIN_UID: &str = "headstash-claim";
pub const ALGO_PENDING: &str = "headstash-pending";
pub const ALGO_ACCEPTED: &str = "headstash-accepted";
/// The one oracle source whose value is the accepted batch hash.
pub const BATCH_SOURCE_ID: &str = "headstash-batch";
/// Custody scope registered for [`BATCH_SOURCE_ID`].
pub const BATCH_SCOPE: &str = "headstash-batch";
/// Fixed attestation height. The module hashes this, not the block height.
pub const BATCH_ATTEST_HEIGHT: u64 = 1;
/// Fixed timestamp signed into the custody payload. Not part of the aggregate.
pub const BATCH_ATTEST_TIMESTAMP: i64 = 1;

const ACCEPTED_DOMAIN: &[u8] = b"terp-headstash-accepted-v1";
const PENDING_DOMAIN: &[u8] = b"terp-headstash-pending-v1";

const EPOCH_ROOTS: Map<&str, Binary> = Map::new("epoch_root");
const PUBLIC_RESERVED: Map<&str, ()> = Map::new("epoch_public_nf");
const PUBLIC_LOGS: Map<&str, ()> = Map::new("epoch_public_log");
const DENOM_TABLE: Item<Vec<u64>> = Item::new("epoch_denom_table");

#[cw_serde]
pub struct EpochPayout {
    /// 32-byte nullifier.
    pub nullifier: Binary,
    /// 32-byte circuit denomination.
    pub denom: Binary,
    pub destination: String,
    pub amount: u64,
}

#[cw_serde]
pub struct PublicOpen {
    pub index: u32,
    pub destination: String,
    /// Canonical Pallas scalar, 32 bytes. Not zero.
    pub blind: Binary,
    /// Compressed Pedersen commitment, 32 bytes.
    pub commitment: Binary,
    pub tx_hash: String,
    pub log_index: u64,
    /// 32-byte nullifier reserved for the public path.
    pub nullifier: Binary,
}

#[cw_serde]
pub struct HashMerchantSudoMsg {
    pub hash_merchant: HashMerchantSudoPayload,
}

#[cw_serde]
pub struct HashMerchantSudoPayload {
    pub chain_uid: String,
    pub algo: String,
    pub height: u64,
    pub root: String,
    pub attestation_count: u32,
    pub block_time: i64,
    /// Present when the module attaches oracle hints. Never a mint source.
    #[serde(default)]
    pub oracle_sources: Vec<OracleHint>,
}

#[cw_serde]
pub struct OracleHint {
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub height: u64,
}

#[cw_serde]
pub struct EpochRootResponse {
    pub algo: String,
    pub root: Binary,
    pub height: u64,
}

const EPOCH_HEIGHTS: Map<&str, u64> = Map::new("epoch_height");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    Reject,
}

#[derive(Clone, Debug, Default)]
pub struct ClaimLane {
    pending: Vec<[u8; 32]>,
    rows: Vec<EpochPayout>,
}

impl ClaimLane {
    pub fn submit(
        &mut self,
        proof: &[u8],
        row: Option<EpochPayout>,
        verdict: Verdict,
        reserved: &HashSet<Vec<u8>>,
    ) -> StdResult<()> {
        let digest = sha256(proof);
        if self.pending.iter().any(|d| d == &digest) {
            return Err(StdError::msg("proof already in the pending lane"));
        }
        if verdict == Verdict::Accept {
            let row = row.ok_or_else(|| StdError::msg("accepted claim is missing its payout"))?;
            check_row(&row)?;
            if reserved.contains(row.nullifier.as_slice()) {
                return Err(StdError::msg("note is on the public path"));
            }
            if self
                .rows
                .iter()
                .any(|existing| existing.nullifier == row.nullifier)
            {
                return Err(StdError::msg("nullifier already in the accepted batch"));
            }
            self.rows.push(row);
        }
        self.pending.push(digest);
        Ok(())
    }

    pub fn pending_root(&self) -> [u8; 32] {
        pending_root(&self.pending)
    }

    pub fn accepted_root(&self, allocation_root: &[u8]) -> StdResult<[u8; 32]> {
        accepted_root(allocation_root, &self.rows)
    }

    pub fn rows(&self) -> &[EpochPayout] {
        &self.rows
    }
}

pub fn pending_root(digests: &[[u8; 32]]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(4 + ACCEPTED_DOMAIN.len() + digests.len() * 32);
    pre.extend_from_slice(PENDING_DOMAIN);
    pre.extend_from_slice(&(digests.len() as u32).to_le_bytes());
    for digest in digests {
        pre.extend_from_slice(digest);
    }
    sha256(&pre)
}

pub fn accepted_root(allocation_root: &[u8], rows: &[EpochPayout]) -> StdResult<[u8; 32]> {
    if allocation_root.len() != 32 {
        return Err(StdError::msg("allocation root must be 32 bytes"));
    }
    let mut ordered = rows.to_vec();
    ordered.sort_by(|a, b| a.nullifier.as_slice().cmp(b.nullifier.as_slice()));
    let mut pre = Vec::new();
    pre.extend_from_slice(ACCEPTED_DOMAIN);
    pre.extend_from_slice(allocation_root);
    pre.extend_from_slice(&(ordered.len() as u32).to_le_bytes());
    for row in &ordered {
        check_row(row)?;
        pre.extend_from_slice(&row.nullifier);
        pre.extend_from_slice(&row.denom);
        let dest = row.destination.as_bytes();
        pre.extend_from_slice(&(dest.len() as u16).to_le_bytes());
        pre.extend_from_slice(dest);
        pre.extend_from_slice(&row.amount.to_le_bytes());
    }
    Ok(sha256(&pre))
}

/// Root Hashmerchant actually stores: SHA-256 of `source_id || batch_hash || height_le`.
///
/// One attestation, so sort-and-dedupe does not change the digest. Timestamp
/// is a custody input only.
pub fn attested_root(batch_hash: &[u8; 32]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(BATCH_SOURCE_ID.len() + 32 + 8);
    pre.extend_from_slice(BATCH_SOURCE_ID.as_bytes());
    pre.extend_from_slice(batch_hash);
    pre.extend_from_slice(&BATCH_ATTEST_HEIGHT.to_le_bytes());
    sha256(&pre)
}

/// Message an ed25519 custody key signs for the fixed headstash attestation.
pub fn custody_message(value: &[u8]) -> Vec<u8> {
    let domain = sha256(
        format!("hashmerchant/custody/{BATCH_SCOPE}/{BATCH_SOURCE_ID}").as_bytes(),
    );
    let mut payload = Vec::with_capacity(32 + value.len() + 16);
    payload.extend_from_slice(&domain);
    payload.extend_from_slice(value);
    payload.extend_from_slice(&BATCH_ATTEST_HEIGHT.to_le_bytes());
    payload.extend_from_slice(&(BATCH_ATTEST_TIMESTAMP as u64).to_le_bytes());
    payload
}

pub fn pedersen_commit(destination: &str, blind: &[u8; 32]) -> StdResult<[u8; 32]> {
    let (g, h) = pedersen_generators();
    let value = scalar_uniform(b"dest", destination.as_bytes());
    let blind = scalar_canonical(blind)?;
    if bool::from(blind.is_zero()) {
        return Err(StdError::msg("blind must be non-zero"));
    }
    let commitment = (g * value + h * blind).to_affine();
    let bytes = commitment.to_bytes();
    let slice: &[u8] = bytes.as_ref();
    slice
        .try_into()
        .map_err(|_| StdError::msg("pedersen point is not 32 bytes"))
}

pub fn sudo(deps: DepsMut, msg: HashMerchantSudoMsg) -> Result<Response, StdError> {
    let payload = msg.hash_merchant;
    if payload.chain_uid != CHAIN_UID {
        return Err(StdError::msg(format!(
            "unexpected chain_uid {}",
            payload.chain_uid
        )));
    }
    if payload.algo != ALGO_PENDING && payload.algo != ALGO_ACCEPTED {
        return Err(StdError::msg(format!("unexpected algo {}", payload.algo)));
    }
    let root = decode_root(&payload.root)?;
    EPOCH_ROOTS.save(deps.storage, payload.algo.as_str(), &Binary::from(root))?;
    EPOCH_HEIGHTS.save(deps.storage, payload.algo.as_str(), &payload.height)?;
    let _hints = payload.oracle_sources.len();
    Ok(Response::new()
        .add_attribute("action", "sudo_hash_merchant")
        .add_attribute("chain_uid", payload.chain_uid)
        .add_attribute("algo", payload.algo)
        .add_attribute("minted", "0"))
}

pub fn query_root(storage: &dyn Storage, algo: String) -> StdResult<EpochRootResponse> {
    let root = EPOCH_ROOTS
        .may_load(storage, algo.as_str())?
        .ok_or_else(|| StdError::msg("epoch root not found"))?;
    let height = EPOCH_HEIGHTS.may_load(storage, algo.as_str())?.unwrap_or(0);
    Ok(EpochRootResponse { algo, root, height })
}

pub fn set_denom_table(
    deps: DepsMut,
    info: MessageInfo,
    amounts: Vec<u64>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    if amounts.is_empty() || amounts.iter().any(|amount| *amount == 0) {
        return Err(StdError::msg("denomination table needs positive amounts"));
    }
    let mut seen = HashSet::new();
    for amount in &amounts {
        if !seen.insert(*amount) {
            return Err(StdError::msg("denomination indexes must be unique"));
        }
    }
    DENOM_TABLE.save(deps.storage, &amounts)?;
    Ok(Response::new()
        .add_attribute("action", "set_denom_table")
        .add_attribute("len", amounts.len().to_string()))
}

pub fn assign_public_notes(
    deps: DepsMut,
    info: MessageInfo,
    nullifiers: Vec<Binary>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    if nullifiers.is_empty() {
        return Err(StdError::msg("no notes to assign"));
    }
    for nullifier in &nullifiers {
        require_32(nullifier, "nullifier")?;
        let hex = hex::encode(nullifier);
        for root_id in 0..8 {
            if NULLIFIERS
                .may_load(deps.storage, nullifier_storage_key(root_id, &hex))?
                .is_some()
            {
                return Err(StdError::msg("nullifier already exists"));
            }
        }
        PUBLIC_RESERVED.save(deps.storage, hex.as_str(), &())?;
    }
    Ok(Response::new()
        .add_attribute("action", "assign_public_notes")
        .add_attribute("count", nullifiers.len().to_string()))
}

pub fn apply_accepted_batch(
    deps: DepsMut,
    env: Env,
    allocation_root: Binary,
    root_id: u64,
    rows: Vec<EpochPayout>,
) -> Result<Response, StdError> {
    let stored = EPOCH_ROOTS
        .may_load(deps.storage, ALGO_ACCEPTED)?
        .ok_or_else(|| StdError::msg("no accepted root"))?;
    let batch = accepted_root(allocation_root.as_slice(), &rows)?;
    let got = attested_root(&batch);
    if stored.as_slice() != got {
        return Err(StdError::msg("batch does not match the accepted root"));
    }
    distro::assert_claim_root(deps.storage, root_id, Some(&allocation_root))?;
    let mut seen = HashSet::new();
    for row in &rows {
        check_row(row)?;
        if !seen.insert(row.nullifier.clone()) {
            return Err(StdError::msg("duplicate nullifier in batch"));
        }
        let hex = hex::encode(&row.nullifier);
        if PUBLIC_RESERVED
            .may_load(deps.storage, hex.as_str())?
            .is_some()
        {
            return Err(StdError::msg("note is on the public path"));
        }
        deps.api.addr_validate(&row.destination)?;
    }
    let cfg = HEADSTASH_CFG.load(deps.storage)?;
    let mut res = Response::new().add_attribute("action", "apply_accepted_batch");
    for row in &rows {
        let strategy = strategy_for(&cfg.ts, &row.denom)?;
        let hex = hex::encode(&row.nullifier);
        verify_nullifier_stateful(deps.storage, nullifier_storage_key(root_id, &hex))?;
        res = push_credit(
            res,
            &env,
            strategy,
            &row.destination,
            row.amount,
        )?;
    }
    Ok(res.add_attribute("paid", rows.len().to_string()))
}

pub fn apply_public_batch(
    deps: DepsMut,
    env: Env,
    rows: Vec<PublicOpen>,
) -> Result<Response, StdError> {
    if rows.is_empty() {
        return Err(StdError::msg("public batch is empty"));
    }
    let table = DENOM_TABLE
        .may_load(deps.storage)?
        .ok_or_else(|| StdError::msg("denomination table is not set"))?;
    let cfg = HEADSTASH_CFG.load(deps.storage)?;
    if cfg.ts.len() != 1 {
        return Err(StdError::msg("public batch needs one token strategy"));
    }
    let strategy = &cfg.ts[0];
    let mut res = Response::new().add_attribute("action", "apply_public_batch");
    for row in &rows {
        let amount = *table
            .get(row.index as usize)
            .ok_or_else(|| StdError::msg("denomination index out of range"))?;
        let blind: [u8; 32] = require_32(&row.blind, "blind")?
            .try_into()
            .expect("32");
        let commitment: [u8; 32] = require_32(&row.commitment, "commitment")?
            .try_into()
            .expect("32");
        let expect = pedersen_commit(&row.destination, &blind)?;
        if expect != commitment {
            return Err(StdError::msg("pedersen opening does not match"));
        }
        deps.api.addr_validate(&row.destination)?;
        if row.tx_hash.is_empty() {
            return Err(StdError::msg("missing ethereum log"));
        }
        let log_key = format!("{}:{}", row.tx_hash, row.log_index);
        if PUBLIC_LOGS.may_load(deps.storage, log_key.as_str())?.is_some() {
            return Err(StdError::msg("ethereum log already paid"));
        }
        require_32(&row.nullifier, "nullifier")?;
        let hex = hex::encode(&row.nullifier);
        if PUBLIC_RESERVED
            .may_load(deps.storage, hex.as_str())?
            .is_none()
        {
            return Err(StdError::msg("note is not on the public path"));
        }
        PUBLIC_LOGS.save(deps.storage, log_key.as_str(), &())?;
        verify_nullifier_stateful(deps.storage, nullifier_storage_key(0, &hex))?;
        res = push_credit(res, &env, strategy, &row.destination, amount)?;
    }
    Ok(res.add_attribute("paid", rows.len().to_string()))
}

fn push_credit(
    res: Response,
    env: &Env,
    strategy: &TokenStrategy,
    destination: &str,
    amount: u64,
) -> StdResult<Response> {
    if amount == 0 {
        return Err(StdError::msg("zero payout"));
    }
    let denom = strategy.denom(&env.contract.address);
    match strategy {
        TokenStrategy::NewFungible(_) => {
            let mint = msg_mint(
                env.contract.address.to_string(),
                Uint128::new(amount as u128),
                denom.clone(),
            )?;
            Ok(res
                .add_message(mint)
                .add_message(BankMsg::Send {
                    to_address: destination.to_string(),
                    amount: vec![Coin::new(amount as u128, denom)],
                }))
        }
        TokenStrategy::ExistingFungible(_) => Ok(res.add_message(BankMsg::Send {
            to_address: destination.to_string(),
            amount: vec![Coin::new(amount as u128, denom)],
        })),
    }
}

fn strategy_for<'a>(
    strategies: &'a [TokenStrategy],
    denom: &Binary,
) -> Result<&'a TokenStrategy, StdError> {
    strategies
        .iter()
        .find(|strategy| strategy.proof_representation() == *denom)
        .ok_or_else(|| StdError::msg("incorrect token denom"))
}

fn check_row(row: &EpochPayout) -> StdResult<()> {
    require_32(&row.nullifier, "nullifier")?;
    require_32(&row.denom, "denom")?;
    if row.destination.is_empty() || row.amount == 0 {
        return Err(StdError::msg("payout needs a destination and a positive amount"));
    }
    if row.destination.len() > 128 {
        return Err(StdError::msg("destination is too long"));
    }
    Ok(())
}

fn require_32<'a>(bytes: &'a Binary, name: &str) -> StdResult<&'a [u8]> {
    if bytes.len() != 32 {
        return Err(StdError::msg(format!("{name} must be 32 bytes")));
    }
    Ok(bytes.as_slice())
}

fn decode_root(raw: &str) -> StdResult<[u8; 32]> {
    let bytes = Binary::from_base64(raw)
        .or_else(|_| Binary::from_base64(raw.trim()))
        .map_err(|err| StdError::msg(format!("root is not base64: {err}")))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| StdError::msg("root must be 32 bytes"))
}

fn pedersen_generators() -> (Point, Point) {
    let hash = Point::hash_to_curve("terp-headstash-pedersen-v1");
    (hash(b"G"), hash(b"H"))
}

fn scalar_uniform(label: &[u8], data: &[u8]) -> Scalar {
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&sha256_parts(&[label, data]));
    wide[32..].copy_from_slice(&sha256_parts(&[b"wide", label, data]));
    Scalar::from_uniform_bytes(&wide)
}

fn scalar_canonical(bytes: &[u8; 32]) -> StdResult<Scalar> {
    Option::from(Scalar::from_repr(*bytes))
        .ok_or_else(|| StdError::msg("blind is not a canonical scalar"))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    let dig = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

pub fn root_b64(root: &[u8; 32]) -> String {
    Binary::from(*root).to_base64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{ExecuteMsg, InstantiateMsg};
    use crate::tokenfactory::{
        derive_nd, DenomUnit, HeadstashTokenObject, Metadata, NewTokenConfig,
    };
    use crate::wavs::generate_test_wavs_proof;
    use crate::{execute, instantiate};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{CosmosMsg, Empty, OwnedDeps, Uint256};

    type TestDeps = OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
        Empty,
    >;

    fn nf(byte: u8) -> Binary {
        Binary::from(vec![byte; 32])
    }

    fn denom() -> Binary {
        derive_nd("cht")
    }

    fn row(byte: u8, dest: &str, amount: u64, denom: Binary) -> EpochPayout {
        EpochPayout {
            nullifier: nf(byte),
            denom,
            destination: dest.into(),
            amount,
        }
    }

    #[test]
    fn row_order_does_not_change_the_accepted_root() {
        let root = [7u8; 32];
        let denom = denom();
        let a = row(1, "terp1a", 5, denom.clone());
        let b = row(2, "terp1b", 8, denom);
        let left = accepted_root(&root, &[a.clone(), b.clone()]).unwrap();
        let right = accepted_root(&root, &[b, a]).unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn lane_keeps_a_reject_out_of_the_accepted_root() {
        let mut lane = ClaimLane::default();
        let denom = denom();
        lane.submit(b"proof-ok", Some(row(1, "terp1a", 5, denom.clone())), Verdict::Accept, &HashSet::new())
            .unwrap();
        lane.submit(b"proof-bad", None, Verdict::Reject, &HashSet::new())
            .unwrap();
        assert_eq!(lane.rows().len(), 1);
        assert!(lane
            .submit(b"proof-ok", Some(row(3, "terp1c", 5, denom.clone())), Verdict::Accept, &HashSet::new())
            .is_err());
        let mut reserved = HashSet::new();
        reserved.insert(nf(4).to_vec());
        assert!(lane
            .submit(
                b"proof-public",
                Some(row(4, "terp1d", 5, denom)),
                Verdict::Accept,
                &reserved
            )
            .is_err());
        assert_eq!(lane.pending.len(), 2);
    }

    #[test]
    fn pedersen_opening_rejects_a_different_destination() {
        let blind = [3u8; 32];
        let commitment = pedersen_commit("terp1dest", &blind).unwrap();
        assert_eq!(pedersen_commit("terp1dest", &blind).unwrap(), commitment);
        assert_ne!(pedersen_commit("terp1other", &blind).unwrap(), commitment);
        assert!(pedersen_commit("terp1dest", &[0u8; 32]).is_err());
    }

    fn setup() -> (TestDeps, cosmwasm_std::Env, Binary) {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let root = Binary::from(vec![0u8; 32]);
        let msg = InstantiateMsg {
            genesis_root: root.clone(),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new("cht".into()),
                metadata: Metadata {
                    description: None,
                    denom_units: vec![DenomUnit {
                        denom: "ucht".into(),
                        exponent: 0,
                        aliases: vec![],
                    }],
                    base: Some("ucht".into()),
                    display: Some("CHT".into()),
                    name: Some("CHT".into()),
                    symbol: Some("CHT".into()),
                },
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: generate_test_wavs_proof(1),
            circuit_id: None,
        };
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg).unwrap();
        (deps, env, root)
    }

    #[test]
    fn accepted_batch_pays_once_and_blocks_the_insanity() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let denom = denom();
        let payout = row(1, &dest, 15, denom.clone());
        let hash = accepted_root(root.as_slice(), &[payout.clone()]).unwrap();
        let attested = attested_root(&hash);
        assert_ne!(attested, hash);
        let sudo_msg = HashMerchantSudoMsg {
            hash_merchant: HashMerchantSudoPayload {
                chain_uid: CHAIN_UID.into(),
                algo: ALGO_ACCEPTED.into(),
                height: 4,
                root: root_b64(&attested),
                attestation_count: 2,
                block_time: 1,
                oracle_sources: vec![OracleHint {
                    source_id: "hint".into(),
                    value: "ff".into(),
                    height: 4,
                }],
            },
        };
        let stored = sudo(deps.as_mut(), sudo_msg).unwrap();
        assert_eq!(
            stored.attributes.iter().find(|a| a.key == "minted").unwrap().value,
            "0"
        );
        assert!(NULLIFIERS.is_empty(deps.as_ref().storage));

        let pending = HashMerchantSudoMsg {
            hash_merchant: HashMerchantSudoPayload {
                chain_uid: CHAIN_UID.into(),
                algo: ALGO_PENDING.into(),
                height: 4,
                root: root_b64(&[9u8; 32]),
                attestation_count: 2,
                block_time: 1,
                oracle_sources: vec![],
            },
        };
        sudo(deps.as_mut(), pending).unwrap();

        let wrong = execute(
            deps.as_mut(),
            env.clone(),
            message_info(&creator, &[]),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![row(2, &dest, 15, denom.clone())],
            },
        );
        assert!(wrong.unwrap_err().to_string().contains("does not match"));

        let paid = execute(
            deps.as_mut(),
            env.clone(),
            message_info(&creator, &[]),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![payout.clone()],
            },
        )
        .unwrap();
        assert_eq!(paid.messages.len(), 2);
        match &paid.messages[1].msg {
            CosmosMsg::Bank(BankMsg::Send { amount, .. }) => {
                assert_eq!(amount[0].amount, Uint256::from(15u64));
            }
            other => panic!("expected one bank send, got {other:?}"),
        }
        let again = execute(
            deps.as_mut(),
            env,
            message_info(&creator, &[]),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root,
                root_id: 0,
                rows: vec![payout],
            },
        );
        assert!(again.unwrap_err().to_string().contains("nullifier already exists"));
    }

    #[test]
    fn public_path_and_private_batch_cannot_pay_the_same_note() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let info = message_info(&creator, &[]);
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetDenomTable {
                amounts: vec![10, 20],
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::AssignPublicNotes {
                nullifiers: vec![nf(4)],
            },
        )
        .unwrap();

        let private = row(4, &dest, 10, denom());
        let hash = accepted_root(root.as_slice(), &[private.clone()]).unwrap();
        sudo(
            deps.as_mut(),
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 1,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap();
        let blocked = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root,
                root_id: 0,
                rows: vec![private],
            },
        );
        assert!(blocked.unwrap_err().to_string().contains("public path"));

        let blind = [8u8; 32];
        let commitment = pedersen_commit(&dest, &blind).unwrap();
        let open = PublicOpen {
            index: 1,
            destination: dest,
            blind: Binary::from(blind.to_vec()),
            commitment: Binary::from(commitment.to_vec()),
            tx_hash: "0xabc".into(),
            log_index: 2,
            nullifier: nf(4),
        };
        let paid = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::ApplyPublicBatch {
                rows: vec![open.clone()],
            },
        )
        .unwrap();
        assert_eq!(paid.messages.len(), 2);
        match &paid.messages[1].msg {
            CosmosMsg::Bank(BankMsg::Send { amount, .. }) => {
                assert_eq!(amount[0].amount, Uint256::from(20u64));
            }
            other => panic!("expected the table amount, got {other:?}"),
        }
        let replay = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::ApplyPublicBatch { rows: vec![open] },
        );
        assert!(replay.unwrap_err().to_string().contains("already"));

        let other = deps.api.addr_make("other").to_string();
        let bad = PublicOpen {
            index: 0,
            destination: other.clone(),
            blind: Binary::from(blind.to_vec()),
            commitment: Binary::from(pedersen_commit(&other, &blind).unwrap().to_vec()),
            tx_hash: "0xdef".into(),
            log_index: 0,
            nullifier: nf(5),
        };
        let unreserved = execute(
            deps.as_mut(),
            env,
            info,
            ExecuteMsg::ApplyPublicBatch { rows: vec![bad] },
        );
        assert!(unreserved.unwrap_err().to_string().contains("not on the public path"));
    }

    #[test]
    fn wrong_chain_and_a_bad_opening_are_rejected() {
        let (mut deps, env, _) = setup();
        let err = sudo(
            deps.as_mut(),
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: "ethereum-mainnet".into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 1,
                    root: root_b64(&[1u8; 32]),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        );
        assert!(err.unwrap_err().to_string().contains("chain_uid"));

        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let info = message_info(&creator, &[]);
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetDenomTable { amounts: vec![10] },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::AssignPublicNotes {
                nullifiers: vec![nf(1)],
            },
        )
        .unwrap();
        let blind = [4u8; 32];
        let mut open = PublicOpen {
            index: 0,
            destination: dest.clone(),
            blind: Binary::from(blind.to_vec()),
            commitment: Binary::from(pedersen_commit(&dest, &blind).unwrap().to_vec()),
            tx_hash: "0x1".into(),
            log_index: 0,
            nullifier: nf(1),
        };
        open.commitment = Binary::from(vec![9u8; 32]);
        let bad = execute(
            deps.as_mut(),
            env,
            info,
            ExecuteMsg::ApplyPublicBatch { rows: vec![open] },
        );
        assert!(bad.unwrap_err().to_string().contains("pedersen"));
    }

    #[test]
    fn two_credits_of_the_same_recipient_add_once_each() {
        let mut map: std::collections::HashMap<(String, String), u64> =
            std::collections::HashMap::new();
        let key = ("nd".to_string(), "rr".to_string());
        crate::headstash::add_credit(&mut map, key.clone(), 15);
        crate::headstash::add_credit(&mut map, key.clone(), 15);
        assert_eq!(map[&key], 30);
    }
}
