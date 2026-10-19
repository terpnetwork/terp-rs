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

const EPOCH_CAMPAIGN: Item<EpochCampaignParams> = Item::new("epoch_campaign");
const EPOCH_STAGE: Item<StagedBatch> = Item::new("epoch_stage");
const EPOCH_PAID: Item<bool> = Item::new("epoch_paid");

#[cw_serde]
pub struct AmountCap {
    pub amount: u64,
    pub cap: u32,
}

/// Creator parameters. Stored by the owner after instantiate so existing
/// campaigns keep the store-only sudo.
#[cw_serde]
pub struct EpochCampaignParams {
    pub ctx: Binary,
    pub backend: String,
    pub cutoff_height: u64,
    pub bech32_hrp: String,
    pub denom: Binary,
    pub amounts: Vec<AmountCap>,
    /// Nullifiers the public path owns. The epoch batch cannot pay them.
    pub reserved: Vec<Binary>,
    /// Trusted verifier keys. Stored, not checked against receipts here.
    pub trusted: Vec<Binary>,
    pub trusted_verify_threshold: u32,
    pub trusted_sign_threshold: u32,
    pub max_rows: u32,
    pub allocation_root: Binary,
    /// Eligibility root the nullifiers are spent under. Genesis is 0.
    pub root_id: u64,
    /// Sidecar registry id. Halo2 must not be 0. Flock may be 0.
    pub vk_id: u64,
    /// SHA-256 of the whole circuit blob. 32 bytes for both backends.
    pub vk_hash: Binary,
    /// Bech32 address that receives leftover escrow after the cutoff block.
    /// Empty disables the sweep. The sweep message does not carry an address.
    #[serde(default)]
    pub sweep_recipient: String,
}

#[cw_serde]
struct StagedBatch {
    digests: Vec<Binary>,
    rows: Vec<EpochPayout>,
    pending: Binary,
    accepted: Binary,
}

/// Sorts, then uses the contract preimage. `pending_root` itself does not sort.
pub fn pending_root_sorted(digests: &[[u8; 32]]) -> [u8; 32] {
    let mut ordered = digests.to_vec();
    ordered.sort();
    pending_root(&ordered)
}

pub fn set_campaign(
    deps: DepsMut,
    info: MessageInfo,
    campaign: EpochCampaignParams,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    validate_campaign(&campaign)?;
    if !campaign.sweep_recipient.is_empty() {
        deps.api.addr_validate(&campaign.sweep_recipient)?;
    }
    if let Some(existing) = EPOCH_CAMPAIGN.may_load(deps.storage)? {
        if existing.vk_id != campaign.vk_id || existing.vk_hash != campaign.vk_hash {
            return Err(StdError::msg("vk is fixed"));
        }
    }
    if EPOCH_STAGE.may_load(deps.storage)?.is_some()
        || EPOCH_PAID.may_load(deps.storage)?.unwrap_or(false)
    {
        return Err(StdError::msg("campaign is already staged"));
    }
    EPOCH_CAMPAIGN.save(deps.storage, &campaign)?;
    Ok(Response::new()
        .add_attribute("action", "set_epoch_campaign")
        .add_attribute("backend", campaign.backend)
        .add_attribute("cutoff", campaign.cutoff_height.to_string()))
}

/// Owner sends the campaign denom balance to the recipient stored on the
/// campaign. The block must be past the cutoff, and a staged batch that
/// has not been paid still owns its escrow.
pub fn sweep_unclaimed(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    let campaign = EPOCH_CAMPAIGN
        .may_load(deps.storage)?
        .ok_or_else(|| StdError::msg("epoch campaign is not set"))?;
    if campaign.sweep_recipient.is_empty() {
        return Err(StdError::msg("sweep recipient is not set"));
    }
    let recipient = deps.api.addr_validate(&campaign.sweep_recipient)?;
    if env.block.height <= campaign.cutoff_height {
        return Err(StdError::msg("campaign has not ended"));
    }
    let staged = EPOCH_STAGE.may_load(deps.storage)?;
    let paid = EPOCH_PAID.may_load(deps.storage)?.unwrap_or(false);
    if staged.is_some() && !paid {
        return Err(StdError::msg("staged batch is unpaid"));
    }
    let cfg = HEADSTASH_CFG.load(deps.storage)?;
    let strategy = strategy_for(&cfg.ts, &campaign.denom)?;
    let denom = strategy.denom(&env.contract.address);
    let balance = deps
        .querier
        .query_balance(env.contract.address, denom.clone())?;
    if balance.amount.is_zero() {
        return Err(StdError::msg("nothing to sweep"));
    }
    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: recipient.to_string(),
            amount: vec![balance],
        })
        .add_attribute("action", "sweep_unclaimed")
        .add_attribute("denom", denom))
}

pub fn stage_batch(
    deps: DepsMut,
    env: &Env,
    info: MessageInfo,
    digests: Vec<Binary>,
    rows: Vec<EpochPayout>,
) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    if EPOCH_PAID.may_load(deps.storage)?.unwrap_or(false) {
        return Err(StdError::msg("epoch batch already paid"));
    }
    let campaign = EPOCH_CAMPAIGN
        .may_load(deps.storage)?
        .ok_or_else(|| StdError::msg("epoch campaign is not set"))?;
    let staged = validate_stage(&campaign, &digests, &rows)?;
    for row in &rows {
        deps.api.addr_validate(&row.destination)?;
        let prefix = format!("{}1", campaign.bech32_hrp);
        if !row.destination.starts_with(&prefix) {
            return Err(StdError::msg("destination hrp does not match the campaign"));
        }
    }
    EPOCH_STAGE.save(deps.storage, &staged)?;
    // No oracle value on a stage. Flock (a zero vk hash) can still pay
    // against a root that is already stored. Halo2 is checked on sudo.
    let (paid, n) = settle(deps, env, &[])?;
    Ok(paid
        .add_attribute("action", "stage_epoch_batch")
        .add_attribute("rows", rows.len().to_string())
        .add_attribute("minted", n.to_string()))
}

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
    attested_of(batch_hash)
}

fn attested_of(value: &[u8]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(BATCH_SOURCE_ID.len() + value.len() + 8);
    pre.extend_from_slice(BATCH_SOURCE_ID.as_bytes());
    pre.extend_from_slice(value);
    pre.extend_from_slice(&BATCH_ATTEST_HEIGHT.to_le_bytes());
    sha256(&pre)
}

fn stored_root_for(batch_hash: &[u8; 32], vk_hash: &[u8]) -> [u8; 32] {
    if vk_hash.len() == 32 && vk_hash.iter().any(|byte| *byte != 0) {
        let mut value = Vec::with_capacity(64);
        value.extend_from_slice(batch_hash);
        value.extend_from_slice(vk_hash);
        return attested_of(&value);
    }
    attested_root(batch_hash)
}

/// Message an ed25519 custody key signs for the fixed headstash attestation.
pub fn custody_message(value: &[u8]) -> Vec<u8> {
    let domain = sha256(format!("hashmerchant/custody/{BATCH_SCOPE}/{BATCH_SOURCE_ID}").as_bytes());
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

pub fn sudo(deps: DepsMut, env: &Env, msg: HashMerchantSudoMsg) -> Result<Response, StdError> {
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
    let (paid, n) = if payload.algo == ALGO_ACCEPTED {
        settle(deps, env, &payload.oracle_sources)?
    } else {
        (Response::new(), 0)
    };
    let _hints = payload.oracle_sources.len();
    Ok(paid
        .add_attribute("action", "sudo_hash_merchant")
        .add_attribute("chain_uid", payload.chain_uid)
        .add_attribute("algo", payload.algo)
        .add_attribute("minted", n.to_string()))
}

pub fn query_campaign(storage: &dyn Storage) -> StdResult<EpochCampaignParams> {
    EPOCH_CAMPAIGN
        .may_load(storage)?
        .ok_or_else(|| StdError::msg("epoch campaign is not set"))
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
    vk_hash: Binary,
) -> Result<Response, StdError> {
    let campaign = EPOCH_CAMPAIGN.may_load(deps.storage)?;
    if let Some(campaign) = campaign.as_ref() {
        if campaign.vk_hash.iter().any(|byte| *byte != 0) {
            let presented = vk_hash.as_slice();
            if presented.len() != 32
                || presented.iter().all(|byte| *byte == 0)
                || presented != campaign.vk_hash.as_slice()
            {
                return Err(StdError::msg("vk hash"));
            }
        }
    }
    let stored = EPOCH_ROOTS
        .may_load(deps.storage, ALGO_ACCEPTED)?
        .ok_or_else(|| StdError::msg("no accepted root"))?;
    let batch = accepted_root(allocation_root.as_slice(), &rows)?;
    let got = match campaign.as_ref() {
        Some(campaign) => stored_root_for(&batch, campaign.vk_hash.as_slice()),
        None => attested_root(&batch),
    };
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
        res = push_credit(res, &env, strategy, &row.destination, row.amount)?;
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
        let blind: [u8; 32] = require_32(&row.blind, "blind")?.try_into().expect("32");
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
        if PUBLIC_LOGS
            .may_load(deps.storage, log_key.as_str())?
            .is_some()
        {
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

fn validate_campaign(campaign: &EpochCampaignParams) -> StdResult<()> {
    require_32(&campaign.ctx, "ctx")?;
    if campaign.backend != "flock" && campaign.backend != "halo2" {
        return Err(StdError::msg("backend must be flock or halo2"));
    }
    if campaign.bech32_hrp.is_empty() {
        return Err(StdError::msg("bech32 hrp is empty"));
    }
    require_32(&campaign.denom, "denom")?;
    require_32(&campaign.allocation_root, "allocation root")?;
    if campaign.amounts.is_empty() || campaign.max_rows == 0 {
        return Err(StdError::msg("campaign needs amounts and a row cap"));
    }
    let mut amounts = HashSet::new();
    for class in &campaign.amounts {
        if class.amount == 0 || class.cap == 0 || !amounts.insert(class.amount) {
            return Err(StdError::msg("amount classes must be positive and unique"));
        }
    }
    let mut reserved = HashSet::new();
    for nullifier in &campaign.reserved {
        require_32(nullifier, "reserved nullifier")?;
        if !reserved.insert(nullifier.to_vec()) {
            return Err(StdError::msg("reserved nullifiers must be unique"));
        }
    }
    let mut trusted = HashSet::new();
    for key in &campaign.trusted {
        require_32(key, "trusted key")?;
        if !trusted.insert(key.to_vec()) {
            return Err(StdError::msg("trusted keys must be unique"));
        }
    }
    let n = campaign.trusted.len() as u32;
    if campaign.trusted_verify_threshold == 0
        || campaign.trusted_verify_threshold > n
        || campaign.trusted_sign_threshold == 0
        || campaign.trusted_sign_threshold > n
    {
        return Err(StdError::msg("thresholds must sit inside the trusted set"));
    }
    if campaign.vk_hash.len() != 32 {
        return Err(StdError::msg("vk hash"));
    }
    if campaign.backend == "halo2" && campaign.vk_id == 0 {
        return Err(StdError::msg("vk id"));
    }
    if campaign.backend == "halo2" && campaign.vk_hash.iter().all(|byte| *byte == 0) {
        return Err(StdError::msg("vk hash"));
    }
    if !campaign.sweep_recipient.is_empty() {
        let prefix = format!("{}1", campaign.bech32_hrp);
        if !campaign.sweep_recipient.starts_with(&prefix) {
            return Err(StdError::msg(
                "sweep recipient hrp does not match the campaign",
            ));
        }
    }
    Ok(())
}

fn validate_stage(
    campaign: &EpochCampaignParams,
    digests: &[Binary],
    rows: &[EpochPayout],
) -> StdResult<StagedBatch> {
    if rows.is_empty() || digests.len() != rows.len() || rows.len() > campaign.max_rows as usize {
        return Err(StdError::msg("batch rows do not match the campaign"));
    }
    let mut raw_digests = Vec::with_capacity(digests.len());
    let mut seen_digests = HashSet::new();
    for digest in digests {
        let bytes = require_32(digest, "digest")?;
        if !seen_digests.insert(bytes.to_vec()) {
            return Err(StdError::msg("duplicate proof digest"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        raw_digests.push(arr);
    }
    let mut seen_nf = HashSet::new();
    let mut used: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
    let reserved: HashSet<Vec<u8>> = campaign.reserved.iter().map(|nf| nf.to_vec()).collect();
    for row in rows {
        check_row(row)?;
        if row.denom != campaign.denom {
            return Err(StdError::msg("denom does not match the campaign"));
        }
        if reserved.contains(row.nullifier.as_slice()) {
            return Err(StdError::msg("note is on the public path"));
        }
        if !seen_nf.insert(row.nullifier.to_vec()) {
            return Err(StdError::msg("duplicate nullifier in batch"));
        }
        let cap = campaign
            .amounts
            .iter()
            .find(|class| class.amount == row.amount)
            .map(|class| class.cap)
            .ok_or_else(|| StdError::msg("amount is not in the published set"))?;
        let count = used.entry(row.amount).or_insert(0);
        if *count >= cap {
            return Err(StdError::msg("amount class is full"));
        }
        *count += 1;
    }
    let pending = pending_root_sorted(&raw_digests);
    let accepted = accepted_root(campaign.allocation_root.as_slice(), rows)?;
    let mut ordered: Vec<Binary> = raw_digests
        .into_iter()
        .map(|word| Binary::from(word.to_vec()))
        .collect();
    ordered.sort_by(|a, b| a.as_slice().cmp(b.as_slice()));
    Ok(StagedBatch {
        digests: ordered,
        rows: rows.to_vec(),
        pending: Binary::from(pending.to_vec()),
        accepted: Binary::from(accepted.to_vec()),
    })
}

/// Pay the staged rows when the stored accepted attestation matches and the
/// stored height is at or past the cutoff. A miss returns no messages.
fn settle(deps: DepsMut, env: &Env, oracle: &[OracleHint]) -> StdResult<(Response, u64)> {
    let empty = || (Response::new(), 0u64);
    if EPOCH_PAID.may_load(deps.storage)?.unwrap_or(false) {
        return Ok(empty());
    }
    let Some(campaign) = EPOCH_CAMPAIGN.may_load(deps.storage)? else {
        return Ok(empty());
    };
    let Some(stage) = EPOCH_STAGE.may_load(deps.storage)? else {
        return Ok(empty());
    };
    let Some(stored) = EPOCH_ROOTS.may_load(deps.storage, ALGO_ACCEPTED)? else {
        return Ok(empty());
    };
    let height = EPOCH_HEIGHTS
        .may_load(deps.storage, ALGO_ACCEPTED)?
        .unwrap_or(0);
    if height < campaign.cutoff_height {
        return Ok(empty());
    }
    let batch = accepted_root(campaign.allocation_root.as_slice(), &stage.rows)?;
    if stage.accepted.as_slice() != batch {
        return Err(StdError::msg(
            "staged batch does not match its accepted root",
        ));
    }
    if stored.as_slice() != stored_root_for(&batch, campaign.vk_hash.as_slice()) {
        return Ok(empty());
    }
    if campaign.vk_hash.iter().any(|byte| *byte != 0) {
        let got = attested_vk(oracle)?;
        if got.as_slice() != campaign.vk_hash.as_slice() {
            return Err(StdError::msg("vk hash"));
        }
    }
    distro::assert_claim_root(
        deps.storage,
        campaign.root_id,
        Some(&campaign.allocation_root),
    )?;
    for row in &stage.rows {
        let hex = hex::encode(&row.nullifier);
        if PUBLIC_RESERVED
            .may_load(deps.storage, hex.as_str())?
            .is_some()
        {
            return Err(StdError::msg("note is on the public path"));
        }
    }
    EPOCH_PAID.save(deps.storage, &true)?;
    let mut res = Response::new();
    let cfg = HEADSTASH_CFG.load(deps.storage)?;
    for row in &stage.rows {
        let strategy = strategy_for(&cfg.ts, &row.denom)?;
        let hex = hex::encode(&row.nullifier);
        verify_nullifier_stateful(deps.storage, nullifier_storage_key(campaign.root_id, &hex))?;
        res = push_credit(res, env, strategy, &row.destination, row.amount)?;
    }
    Ok((res, stage.rows.len() as u64))
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
            Ok(res.add_message(mint).add_message(BankMsg::Send {
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
        return Err(StdError::msg(
            "payout needs a destination and a positive amount",
        ));
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

fn attested_vk(oracle: &[OracleHint]) -> StdResult<[u8; 32]> {
    let hint = oracle
        .iter()
        .find(|hint| hint.source_id == BATCH_SOURCE_ID)
        .ok_or_else(|| StdError::msg("vk hash"))?;
    let bytes = Binary::from_base64(&hint.value).map_err(|_| StdError::msg("vk hash"))?;
    if bytes.len() != 64 {
        return Err(StdError::msg("vk hash"));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[32..]);
    if out.iter().all(|byte| *byte == 0) {
        return Err(StdError::msg("vk hash"));
    }
    Ok(out)
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
    use crate::{execute, instantiate, query};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{coins, Addr, CosmosMsg, Empty, OwnedDeps, Uint256};
    use cw_multi_test::{App, BankSudo, ContractWrapper, Executor, SudoMsg};

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
        lane.submit(
            b"proof-ok",
            Some(row(1, "terp1a", 5, denom.clone())),
            Verdict::Accept,
            &HashSet::new(),
        )
        .unwrap();
        lane.submit(b"proof-bad", None, Verdict::Reject, &HashSet::new())
            .unwrap();
        assert_eq!(lane.rows().len(), 1);
        assert!(lane
            .submit(
                b"proof-ok",
                Some(row(3, "terp1c", 5, denom.clone())),
                Verdict::Accept,
                &HashSet::new()
            )
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
        let stored = sudo(deps.as_mut(), &env, sudo_msg).unwrap();
        assert_eq!(
            stored
                .attributes
                .iter()
                .find(|a| a.key == "minted")
                .unwrap()
                .value,
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
        sudo(deps.as_mut(), &env, pending).unwrap();

        let wrong = execute(
            deps.as_mut(),
            env.clone(),
            message_info(&creator, &[]),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![row(2, &dest, 15, denom.clone())],
                vk_hash: Binary::default(),
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
                vk_hash: Binary::default(),
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
                vk_hash: Binary::default(),
            },
        );
        assert!(again
            .unwrap_err()
            .to_string()
            .contains("nullifier already exists"));
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
            &env,
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
                vk_hash: Binary::default(),
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
        assert!(unreserved
            .unwrap_err()
            .to_string()
            .contains("not on the public path"));
    }

    #[test]
    fn wrong_chain_and_a_bad_opening_are_rejected() {
        let (mut deps, env, _) = setup();
        let err = sudo(
            deps.as_mut(),
            &env,
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

    #[test]
    fn halo2_vk_is_immutable_and_zero_is_refused() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let info = message_info(&creator, &[]);
        let mut params = campaign(&root);
        params.backend = "halo2".into();
        params.vk_id = 1;
        params.vk_hash = Binary::from([0u8; 32].to_vec());
        let zero = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign {
                campaign: params.clone(),
            },
        );
        assert!(zero.unwrap_err().to_string().contains("vk hash"));

        params.vk_hash = Binary::from([9u8; 32].to_vec());
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign {
                campaign: params.clone(),
            },
        )
        .unwrap();
        let mut changed = params.clone();
        changed.vk_hash = Binary::from([8u8; 32].to_vec());
        let err = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign { campaign: changed },
        );
        assert!(err.unwrap_err().to_string().contains("vk is fixed"));

        let mut same = params.clone();
        same.max_rows = 3;
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign { campaign: same },
        )
        .unwrap();
        let mut changed_id = params.clone();
        changed_id.vk_id = 2;
        let id_err = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign {
                campaign: changed_id,
            },
        );
        assert!(id_err.unwrap_err().to_string().contains("vk is fixed"));

        let dest = deps.api.addr_make("dest").to_string();
        let mismatch = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![row(1, &dest, 15, denom())],
                vk_hash: Binary::from([1u8; 32].to_vec()),
            },
        );
        assert!(mismatch.unwrap_err().to_string().contains("vk hash"));

        let hash = accepted_root(root.as_slice(), &[row(1, &dest, 15, denom())]).unwrap();
        let attested = stored_root_for(&hash, &[9u8; 32]);
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from([4u8; 32].to_vec())],
                rows: vec![row(1, &dest, 15, denom())],
            },
        )
        .unwrap();
        let mut bad_value = hash.to_vec();
        bad_value.extend_from_slice(&[7u8; 32]);
        let sudo_err = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 10,
                    root: root_b64(&attested),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![OracleHint {
                        source_id: BATCH_SOURCE_ID.into(),
                        value: Binary::from(bad_value).to_base64(),
                        height: 1,
                    }],
                },
            },
        );
        assert!(sudo_err.unwrap_err().to_string().contains("vk hash"));
        assert!(NULLIFIERS.is_empty(deps.as_ref().storage));

        let mut good_value = hash.to_vec();
        good_value.extend_from_slice(&[9u8; 32]);
        let paid = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 10,
                    root: root_b64(&attested),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![OracleHint {
                        source_id: BATCH_SOURCE_ID.into(),
                        value: Binary::from(good_value).to_base64(),
                        height: 1,
                    }],
                },
            },
        )
        .unwrap();
        assert_eq!(
            paid.attributes
                .iter()
                .find(|attr| attr.key == "minted")
                .unwrap()
                .value,
            "1"
        );
    }

    fn campaign(root: &Binary) -> EpochCampaignParams {
        EpochCampaignParams {
            ctx: Binary::from([1u8; 32].to_vec()),
            backend: "flock".into(),
            cutoff_height: 10,
            bech32_hrp: "cosmwasm".into(),
            denom: denom(),
            amounts: vec![AmountCap { amount: 15, cap: 1 }],
            reserved: vec![nf(9)],
            trusted: vec![Binary::from([8u8; 32].to_vec())],
            trusted_verify_threshold: 1,
            trusted_sign_threshold: 1,
            max_rows: 4,
            allocation_root: root.clone(),
            root_id: 0,
            vk_id: 0,
            vk_hash: Binary::from([0u8; 32].to_vec()),
            sweep_recipient: String::new(),
        }
    }

    #[test]
    fn sorted_pending_root_matches_either_digest_order() {
        let hi = [2u8; 32];
        let lo = [1u8; 32];
        assert_eq!(
            pending_root_sorted(&[hi, lo]),
            pending_root_sorted(&[lo, hi])
        );
        assert_ne!(pending_root_sorted(&[hi, lo]), pending_root(&[hi, lo]));
    }

    #[test]
    fn accepted_sudo_pays_the_staged_batch_only_at_the_cutoff() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let info = message_info(&creator, &[]);
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign {
                campaign: campaign(&root),
            },
        )
        .unwrap();

        let payout = row(1, &dest, 15, denom());
        let digest = Binary::from([4u8; 32].to_vec());
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::StageEpochBatch {
                digests: vec![digest],
                rows: vec![payout.clone()],
            },
        )
        .unwrap();
        assert!(NULLIFIERS.is_empty(deps.as_ref().storage));

        let hash = accepted_root(root.as_slice(), &[payout.clone()]).unwrap();
        let early = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 9,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap();
        assert_eq!(
            early
                .attributes
                .iter()
                .find(|a| a.key == "minted")
                .unwrap()
                .value,
            "0"
        );
        assert!(NULLIFIERS.is_empty(deps.as_ref().storage));

        let pending = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_PENDING.into(),
                    height: 11,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap();
        assert_eq!(
            pending
                .attributes
                .iter()
                .find(|a| a.key == "minted")
                .unwrap()
                .value,
            "0"
        );

        let paid = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 10,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![OracleHint {
                        source_id: "hint".into(),
                        value: "ff".into(),
                        height: 10,
                    }],
                },
            },
        )
        .unwrap();
        assert_eq!(
            paid.attributes
                .iter()
                .find(|a| a.key == "minted")
                .unwrap()
                .value,
            "1"
        );
        let bank = paid
            .messages
            .iter()
            .find_map(|msg| match &msg.msg {
                CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                    Some((to_address.clone(), amount[0].amount))
                }
                _ => None,
            })
            .expect("bank send");
        assert_eq!(bank.0, dest);
        assert_eq!(bank.1, Uint256::from(15u64));

        let again = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 12,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap();
        assert_eq!(
            again
                .attributes
                .iter()
                .find(|a| a.key == "minted")
                .unwrap()
                .value,
            "0"
        );
        let restage = execute(
            deps.as_mut(),
            env,
            info,
            ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from([5u8; 32].to_vec())],
                rows: vec![payout],
            },
        );
        assert!(restage.unwrap_err().to_string().contains("already paid"));
    }

    #[test]
    fn stage_rejects_a_full_class_a_reserved_note_and_a_bad_denom() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let info = message_info(&creator, &[]);
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign {
                campaign: campaign(&root),
            },
        )
        .unwrap();
        let digest = Binary::from([4u8; 32].to_vec());
        let over = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::StageEpochBatch {
                digests: vec![digest.clone(), Binary::from([6u8; 32].to_vec())],
                rows: vec![row(1, &dest, 15, denom()), row(2, &dest, 15, denom())],
            },
        );
        assert!(over
            .unwrap_err()
            .to_string()
            .contains("amount class is full"));

        let reserved = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::StageEpochBatch {
                digests: vec![digest.clone()],
                rows: vec![row(9, &dest, 15, denom())],
            },
        );
        assert!(reserved.unwrap_err().to_string().contains("public path"));

        let bad_denom = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::StageEpochBatch {
                digests: vec![digest],
                rows: vec![row(3, &dest, 15, Binary::from([7u8; 32].to_vec()))],
            },
        );
        assert!(bad_denom.unwrap_err().to_string().contains("denom"));

        let stranger = deps.api.addr_make("stranger");
        let denied = execute(
            deps.as_mut(),
            env,
            message_info(&stranger, &[]),
            ExecuteMsg::SetEpochCampaign {
                campaign: campaign(&root),
            },
        );
        assert!(denied.is_err());
    }

    #[test]
    fn accepted_sudo_pays_an_amount_above_u32_max() {
        let wide = u64::from(u32::MAX) + 9;
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let info = message_info(&creator, &[]);
        let mut params = campaign(&root);
        params.amounts = vec![AmountCap {
            amount: wide,
            cap: 1,
        }];
        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetEpochCampaign { campaign: params },
        )
        .unwrap();
        let payout = row(1, &dest, wide, denom());
        let low = row(1, &dest, u64::from(wide as u32), denom());
        assert_ne!(
            accepted_root(root.as_slice(), &[payout.clone()]).unwrap(),
            accepted_root(root.as_slice(), &[low]).unwrap()
        );
        execute(
            deps.as_mut(),
            env.clone(),
            info,
            ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from([4u8; 32].to_vec())],
                rows: vec![payout.clone()],
            },
        )
        .unwrap();
        let hash = accepted_root(root.as_slice(), &[payout]).unwrap();
        let paid = sudo(
            deps.as_mut(),
            &env,
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 10,
                    root: root_b64(&attested_root(&hash)),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap();
        let bank = paid
            .messages
            .iter()
            .find_map(|msg| match &msg.msg {
                CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                    Some((to_address.clone(), amount[0].amount))
                }
                _ => None,
            })
            .expect("bank send");
        assert_eq!(bank.0, dest);
        assert_eq!(bank.1, Uint256::from(wide));
    }
    fn uterp_of(app: &App, addr: &Addr) -> Uint256 {
        app.wrap().query_balance(addr, "uterp").unwrap().amount
    }

    fn sweep_err(app: &mut App, sender: &Addr, contract: &Addr) -> String {
        app.execute_contract(
            sender.clone(),
            contract.clone(),
            &ExecuteMsg::SweepUnclaimed {},
            &[],
        )
        .unwrap_err()
        .to_string()
    }

    /// Existing-fungible escrow. The bank in this app actually holds the coins.
    fn escrow_app(cutoff: u64, height: u64, with_recipient: bool) -> (App, Addr, Addr, Addr) {
        let mut app = App::default();
        let owner = app.api().addr_make("owner");
        let treasury = app.api().addr_make("treasury");
        app.sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: owner.to_string(),
            amount: coins(1_000_000, "uterp"),
        }))
        .unwrap();
        let code_id = app.store_code(Box::new(ContractWrapper::new(
            execute,
            instantiate,
            query,
        )));
        let root = Binary::from(vec![0u8; 32]);
        let contract = app
            .instantiate_contract(
                code_id,
                owner.clone(),
                &InstantiateMsg {
                    genesis_root: root.clone(),
                    distro_hash_domain: Default::default(),
                    genesis_label: None,
                    token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                        "uterp".into(),
                    )),
                    wavs: generate_test_wavs_proof(1),
                    circuit_id: None,
                },
                &coins(1_000_000, "uterp"),
                "headstash",
                None,
            )
            .unwrap();
        app.update_block(|block| {
            block.height = height;
        });
        let mut params = campaign(&root);
        params.backend = "halo2".into();
        params.vk_id = 1;
        params.vk_hash = Binary::from([9u8; 32].to_vec());
        params.cutoff_height = cutoff;
        params.bech32_hrp = "cosmwasm".into();
        params.denom = derive_nd("uterp");
        params.sweep_recipient = if with_recipient {
            treasury.to_string()
        } else {
            String::new()
        };
        app.execute_contract(
            owner.clone(),
            contract.clone(),
            &ExecuteMsg::SetEpochCampaign { campaign: params },
            &[],
        )
        .unwrap();
        (app, owner, contract, treasury)
    }

    #[test]
    fn sweep_recipient_must_match_the_campaign_hrp() {
        let (mut deps, env, root) = setup();
        let creator = deps.api.addr_make("creator");
        let mut params = campaign(&root);
        params.sweep_recipient = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw".into();
        let err = execute(
            deps.as_mut(),
            env,
            message_info(&creator, &[]),
            ExecuteMsg::SetEpochCampaign { campaign: params },
        )
        .unwrap_err();
        assert!(err.to_string().contains("sweep recipient hrp"));
    }

    #[test]
    fn sweep_waits_until_the_block_after_cutoff() {
        let (mut app, owner, contract, _) = escrow_app(20, 20, true);
        let err = sweep_err(&mut app, &owner, &contract);
        assert!(err.contains("campaign has not ended"), "{err}");
        assert_eq!(uterp_of(&app, &contract), Uint256::from(1_000_000u128));

        let (mut app, owner, contract, _) = escrow_app(19, 20, false);
        let err = sweep_err(&mut app, &owner, &contract);
        assert!(err.contains("sweep recipient is not set"), "{err}");
        assert_eq!(uterp_of(&app, &contract), Uint256::from(1_000_000u128));
    }

    #[test]
    fn sweep_sends_unclaimed_uterp_to_the_configured_recipient() {
        let (mut app, owner, contract, _treasury) = escrow_app(19, 20, true);
        let stranger = app.api().addr_make("stranger");
        let err = sweep_err(&mut app, &stranger, &contract);
        assert!(err.contains("owner"), "{err}");

        let dest = app.api().addr_make("dest");
        app.execute_contract(
            owner.clone(),
            contract.clone(),
            &ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from(vec![4u8; 32])],
                rows: vec![row(1, dest.as_str(), 15, derive_nd("uterp"))],
            },
            &[],
        )
        .unwrap();
        let err = sweep_err(&mut app, &owner, &contract);
        assert!(err.contains("staged batch is unpaid"), "{err}");
        assert_eq!(uterp_of(&app, &contract), Uint256::from(1_000_000u128));

        let (mut app, owner, contract, treasury) = escrow_app(19, 20, true);
        app.execute_contract(
            owner.clone(),
            contract.clone(),
            &ExecuteMsg::SweepUnclaimed {},
            &[],
        )
        .unwrap();
        assert!(uterp_of(&app, &contract).is_zero());
        assert_eq!(uterp_of(&app, &treasury), Uint256::from(1_000_000u128));
        let err = sweep_err(&mut app, &owner, &contract);
        assert!(err.contains("nothing to sweep"), "{err}");
    }
}
