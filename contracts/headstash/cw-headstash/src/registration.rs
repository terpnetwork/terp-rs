//! Append-only registration tree. The Poseidon hashes are the circuit helpers.
//! The eligibility path is SHA-256 and is not a circuit input.

use cosmwasm_std::{Binary, Deps, DepsMut, Env, Response, StdError, StdResult, Storage};
use cw_storage_plus::{Item, Map};
use ff::{Field, PrimeField};
use pasta_curves::pallas;
use sha2::{Digest, Sha256};

use crate::epoch::EpochCampaignParams;
use zk_headstash::claim_auth::keccak256;
use zk_headstash::distro_poseidon::{
    poseidon_distro_crh, registration_key_commitment, registration_leaf,
};

/// Claim tree depth of the legacy (vk_id 2 / flock) registration window.
pub const LEGACY_DEPTH: usize = 16;
/// Claim tree depth of a vk_id 3 note campaign (spec rev 5).
pub const NOTE_DEPTH: usize = 17;
/// Largest claim tree depth this contract stores.
pub const MAX_DEPTH: usize = NOTE_DEPTH;
#[cfg(test)]
const DEPTH: usize = LEGACY_DEPTH;

/// Tree depth chosen when the window opened. Missing means the legacy depth.
const TREE_DEPTH: Item<u8> = Item::new("reg_depth");

fn max_leaves(depth: usize) -> u32 {
    1u32 << depth
}

pub fn tree_depth(storage: &dyn Storage) -> StdResult<usize> {
    Ok(TREE_DEPTH
        .may_load(storage)?
        .map(|d| d as usize)
        .unwrap_or(LEGACY_DEPTH))
}

/// Depth a campaign's claim tree uses: 17 for vk_id 3 note campaigns, else 16.
pub fn depth_for(campaign: &EpochCampaignParams) -> usize {
    if campaign.vk_id == crate::notes::NOTE_VK_ID {
        NOTE_DEPTH
    } else {
        LEGACY_DEPTH
    }
}

const FRONTIER: Item<Vec<Binary>> = Item::new("reg_frontier");
const COUNT: Item<u32> = Item::new("reg_count");
const ROOT: Item<Binary> = Item::new("reg_root");
const NEXT_ANCHOR: Item<u64> = Item::new("reg_next_anchor");
const ANCHORS: Item<Vec<AcceptedAnchor>> = Item::new("reg_anchors");
const ADDRESSES: Map<&str, ()> = Map::new("reg_addr");
const KEYS: Map<&str, RegisteredKey> = Map::new("reg_key");
const LEAVES: Map<u32, Binary> = Map::new("reg_leaf");
/// Completed subtree roots keyed by (level, node index), level >= 1. A node is written
/// once, when its last leaf lands, and never changes (append-only tree). Lets
/// `query_path` build siblings for any past anchor in O(DEPTH^2) reads instead of
/// loading every leaf. Missing entries (contracts that registered before this map
/// existed) fall back to recomputing from `LEAVES`, so no migration is needed.
const NODES: Map<(u8, u32), Binary> = Map::new("reg_node");
const PAID_BATCH: Map<&str, ()> = Map::new("reg_paid_batch");

#[cosmwasm_schema::cw_serde]
pub struct RegistrationConfig {
    pub eligibility_root: Binary,
    pub eligibility_depth: u32,
    pub min_anonymity_set: u32,
    pub anchor_start_height: u64,
    pub anchor_interval: u64,
}

#[cosmwasm_schema::cw_serde]
pub struct AcceptedAnchor {
    pub root: Binary,
    pub leaf_count: u32,
}

#[cosmwasm_schema::cw_serde]
struct RegisteredKey {
    index: u32,
    amount: u64,
}

#[cosmwasm_schema::cw_serde]
pub struct RegistrationTreeResponse {
    pub count: u32,
    pub root: Binary,
    pub next_anchor_height: u64,
    pub min_anonymity_set: u32,
    pub anchors: Vec<AcceptedAnchor>,
}

#[cosmwasm_schema::cw_serde]
pub struct RegistrationPathResponse {
    pub index: u32,
    pub leaf: String,
    pub amount: u64,
    pub siblings: Vec<String>,
}

pub fn validate_config(cfg: &RegistrationConfig, ctx: &[u8], depth: usize) -> StdResult<()> {
    if cfg.eligibility_root.len() != 32 {
        return Err(StdError::msg("eligibility"));
    }
    if !(1..=depth as u32).contains(&cfg.eligibility_depth) {
        return Err(StdError::msg("eligibility"));
    }
    if cfg.min_anonymity_set == 0 || cfg.min_anonymity_set > max_leaves(depth) {
        return Err(StdError::msg("anonymity set"));
    }
    if cfg.anchor_interval == 0 {
        return Err(StdError::msg("registration"));
    }
    if parse_fp(ctx).is_err() {
        return Err(StdError::msg("registration"));
    }
    Ok(())
}

pub fn prepare_window(
    storage: &mut dyn Storage,
    cfg: &RegistrationConfig,
    depth: usize,
) -> StdResult<()> {
    if COUNT.may_load(storage)?.unwrap_or(0) > 0 {
        if tree_depth(storage)? != depth {
            return Err(StdError::msg("registration depth"));
        }
        return Ok(());
    }
    if FRONTIER
        .may_load(storage)?
        .is_some_and(|frontier| frontier.len() != depth)
    {
        FRONTIER.remove(storage);
    }
    if FRONTIER.may_load(storage)?.is_none() {
        TREE_DEPTH.save(storage, &(depth as u8))?;
        FRONTIER.save(storage, &vec![Binary::from(vec![0u8; 32]); depth])?;
        COUNT.save(storage, &0)?;
        ROOT.save(storage, &Binary::from(vec![0u8; 32]))?;
        ANCHORS.save(storage, &Vec::new())?;
    }
    NEXT_ANCHOR.save(storage, &cfg.anchor_start_height)?;
    Ok(())
}

pub fn leaf_count(storage: &dyn Storage) -> StdResult<u32> {
    Ok(COUNT.may_load(storage)?.unwrap_or(0))
}

pub fn query_tree(
    storage: &dyn Storage,
    cfg: &RegistrationConfig,
) -> StdResult<RegistrationTreeResponse> {
    Ok(RegistrationTreeResponse {
        count: leaf_count(storage)?,
        root: ROOT
            .may_load(storage)?
            .unwrap_or_else(|| Binary::from(vec![0u8; 32])),
        next_anchor_height: NEXT_ANCHOR
            .may_load(storage)?
            .unwrap_or(cfg.anchor_start_height),
        min_anonymity_set: cfg.min_anonymity_set,
        anchors: ANCHORS.may_load(storage)?.unwrap_or_default(),
    })
}

pub fn query_path(
    storage: &dyn Storage,
    key_com: &[u8],
    anchor: &[u8],
) -> StdResult<RegistrationPathResponse> {
    if key_com.len() != 32 || parse_fp(key_com).is_err() {
        return Err(StdError::msg("key commitment"));
    }
    let record = KEYS
        .may_load(storage, hex::encode(key_com).as_str())?
        .ok_or_else(|| StdError::msg("key commitment"))?;
    let stored = ANCHORS.may_load(storage)?.unwrap_or_default();
    let matched = stored
        .iter()
        .find(|item| item.root.as_slice() == anchor)
        .ok_or_else(|| StdError::msg("anchor"))?;
    if record.index >= matched.leaf_count {
        return Err(StdError::msg("anchor"));
    }
    let n = matched.leaf_count;
    let leaf = load_leaf(storage, record.index)?;
    let depth = tree_depth(storage)?;
    let mut siblings = Vec::with_capacity(depth);
    for level in 0..depth {
        let sibling_at = (record.index >> level) ^ 1;
        let bytes = match node_at(storage, level, sibling_at, n)? {
            Some(node) => node.to_repr(),
            None => [0u8; 32],
        };
        siblings.push(hex::encode(bytes));
    }
    Ok(RegistrationPathResponse {
        index: record.index,
        leaf: hex::encode(leaf),
        amount: record.amount,
        siblings,
    })
}

fn load_leaf(storage: &dyn Storage, index: u32) -> StdResult<[u8; 32]> {
    LEAVES
        .may_load(storage, index)?
        .ok_or_else(|| StdError::msg("registration"))?
        .as_slice()
        .try_into()
        .map_err(|_| StdError::msg("registration"))
}

/// Root of the subtree at (`level`, `node_index`) as it stood when the tree held `n`
/// leaves; same zero-padding rules as [`subtree`]. A subtree entirely below `n` is
/// immutable and read from `NODES`; only the one subtree straddling `n` per level is
/// recursed into, so a full path costs O(DEPTH^2) reads.
fn node_at(
    storage: &dyn Storage,
    level: usize,
    node_index: u32,
    n: u32,
) -> StdResult<Option<pallas::Base>> {
    let start = (node_index as u64) << level;
    if start >= n as u64 {
        return Ok(None);
    }
    if level == 0 {
        return Ok(Some(parse_fp(&load_leaf(storage, node_index)?)?));
    }
    if start + (1u64 << level) <= n as u64 {
        if let Some(stored) = NODES.may_load(storage, (level as u8, node_index))? {
            return Ok(Some(parse_fp(stored.as_slice())?));
        }
    }
    let left = node_at(storage, level - 1, node_index << 1, n)?;
    let right = node_at(storage, level - 1, (node_index << 1) + 1, n)?;
    match (left, right) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(StdError::msg("registration")),
        (Some(left), None) => Ok(Some(poseidon_distro_crh(
            (level - 1) as u32,
            left,
            pallas::Base::ZERO,
        ))),
        (Some(left), Some(right)) => Ok(Some(poseidon_distro_crh((level - 1) as u32, left, right))),
    }
}

/// Reference implementation (loads every leaf); kept for the equivalence tests.
#[cfg(test)]
fn siblings_under(leaves: &[[u8; 32]], index: u32) -> StdResult<Vec<String>> {
    let mut out = Vec::with_capacity(DEPTH);
    for level in 0..DEPTH {
        let sibling_at = (index >> level) ^ 1;
        let bytes = match subtree(leaves, level, sibling_at)? {
            Some(node) => node.to_repr(),
            None => [0u8; 32],
        };
        out.push(hex::encode(bytes));
    }
    Ok(out)
}

#[cfg(test)]
fn subtree(leaves: &[[u8; 32]], level: usize, node_index: u32) -> StdResult<Option<pallas::Base>> {
    let start = node_index << (level as u32);
    if start >= leaves.len() as u32 {
        return Ok(None);
    }
    if level == 0 {
        return Ok(Some(parse_fp(&leaves[start as usize])?));
    }
    let left = subtree(leaves, level - 1, node_index << 1)?;
    let right = subtree(leaves, level - 1, (node_index << 1) + 1)?;
    match (left, right) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(StdError::msg("registration")),
        (Some(left), None) => Ok(Some(poseidon_distro_crh(
            (level - 1) as u32,
            left,
            pallas::Base::ZERO,
        ))),
        (Some(left), Some(right)) => Ok(Some(poseidon_distro_crh((level - 1) as u32, left, right))),
    }
}

pub fn register(
    deps: DepsMut,
    campaign: &EpochCampaignParams,
    key_com: Binary,
    amount: u64,
    signature: Binary,
    eligibility_index: u32,
    eligibility_path: Vec<Binary>,
) -> Result<Response, StdError> {
    let cfg = registration_cfg(campaign)?;
    legacy_only(campaign)?;
    precheck(deps.storage, amount)?;
    parse_fp(key_com.as_slice()).map_err(|_| StdError::msg("key commitment"))?;
    let message = register_message(campaign.ctx.as_slice(), key_com.as_slice(), amount);
    let address = recover_address(deps.api, &message, signature.as_slice())?;
    let leaf = eligibility_leaf(&address, amount);
    insert_registration(
        deps,
        cfg,
        &evm_identity(&address),
        leaf,
        key_com,
        amount,
        eligibility_index,
        &eligibility_path,
        "evm",
    )
}

/// Solana registration. `signature` is a 64-byte ed25519 signature by the
/// 32-byte `pubkey` over the raw UTF-8 bytes of [`register_sol_message`]
/// (what Phantom/Solflare `signMessage` signs). The eligible identity is the pubkey.
#[allow(clippy::too_many_arguments)]
pub fn register_sol(
    deps: DepsMut,
    campaign: &EpochCampaignParams,
    key_com: Binary,
    amount: u64,
    pubkey: Binary,
    signature: Binary,
    eligibility_index: u32,
    eligibility_path: Vec<Binary>,
) -> Result<Response, StdError> {
    let cfg = registration_cfg(campaign)?;
    legacy_only(campaign)?;
    precheck(deps.storage, amount)?;
    parse_fp(key_com.as_slice()).map_err(|_| StdError::msg("key commitment"))?;
    let pk: [u8; 32] = pubkey
        .as_slice()
        .try_into()
        .map_err(|_| StdError::msg("pubkey"))?;
    if signature.len() != 64 {
        return Err(StdError::msg("signature"));
    }
    let message = register_sol_message(campaign.ctx.as_slice(), key_com.as_slice(), amount);
    let ok = deps
        .api
        .ed25519_verify(message.as_bytes(), signature.as_slice(), &pk)
        .map_err(|_| StdError::msg("signature"))?;
    if !ok {
        return Err(StdError::msg("signature"));
    }
    let leaf = sol_eligibility_leaf(&pk, amount);
    insert_registration(
        deps,
        cfg,
        &sol_identity(&pk),
        leaf,
        key_com,
        amount,
        eligibility_index,
        &eligibility_path,
        "sol",
    )
}

/// Single-amount registrations do not exist on a vk_id 3 note campaign.
fn legacy_only(campaign: &EpochCampaignParams) -> StdResult<()> {
    if campaign.vk_id == crate::notes::NOTE_VK_ID {
        return Err(StdError::msg("note campaign: use register_notes"));
    }
    Ok(())
}

/// Registration closes with the claim window: nothing registered after the
/// cutoff could ever be claimed.
pub(crate) fn assert_registration_open(height: u64, campaign: &EpochCampaignParams) -> StdResult<()> {
    if height > campaign.cutoff_height {
        return Err(StdError::msg("registration closed"));
    }
    Ok(())
}

pub(crate) fn registration_cfg(campaign: &EpochCampaignParams) -> StdResult<&RegistrationConfig> {
    campaign
        .registration
        .as_ref()
        .ok_or_else(|| StdError::msg("registration"))
}

pub(crate) fn precheck(storage: &dyn Storage, amount: u64) -> StdResult<()> {
    if leaf_count(storage)? >= max_leaves(tree_depth(storage)?) {
        return Err(StdError::msg("tree full"));
    }
    if amount == 0 {
        return Err(StdError::msg("amount"));
    }
    Ok(())
}

/// Registered-identity key for an EVM address (unchanged from v1 storage: 40 hex chars).
pub fn evm_identity(address: &[u8; 20]) -> String {
    hex::encode(address)
}

/// Registered-identity key for a Solana pubkey. The `sol:` prefix keeps it
/// disjoint from EVM keys in the shared one-registration-per-identity map.
pub fn sol_identity(pubkey: &[u8; 32]) -> String {
    format!("sol:{}", hex::encode(pubkey))
}

pub fn is_registered(storage: &dyn Storage, identity: &str) -> StdResult<bool> {
    Ok(ADDRESSES.may_load(storage, identity)?.is_some())
}

#[allow(clippy::too_many_arguments)]
fn insert_registration(
    deps: DepsMut,
    cfg: &RegistrationConfig,
    identity: &str,
    eligible_leaf: [u8; 32],
    key_com: Binary,
    amount: u64,
    eligibility_index: u32,
    eligibility_path: &[Binary],
    kind: &str,
) -> Result<Response, StdError> {
    let key_fp = parse_fp(key_com.as_slice()).map_err(|_| StdError::msg("key commitment"))?;
    let path = path_bytes(eligibility_path)?;
    let opened = open_eligibility_leaf(
        eligible_leaf,
        eligibility_index,
        &path,
        cfg.eligibility_depth,
    )?;
    if opened.as_slice() != cfg.eligibility_root.as_slice() {
        return Err(StdError::msg("eligibility"));
    }
    if ADDRESSES.may_load(deps.storage, identity)?.is_some() {
        return Err(StdError::msg("already registered"));
    }
    let leaf_fp = registration_leaf(key_fp, amount);
    let (count, leaf) = append_claim_leaf(deps.storage, &key_com, leaf_fp, amount)?;
    ADDRESSES.save(deps.storage, identity, &())?;
    Ok(Response::new()
        .add_attribute("action", "register_headstash")
        .add_attribute("kind", kind)
        .add_attribute("index", count.to_string())
        .add_attribute("leaf", hex::encode(leaf)))
}

/// Append one claim leaf for `key_com` (globally unique, N6) and return its index.
pub(crate) fn append_claim_leaf(
    storage: &mut dyn Storage,
    key_com: &Binary,
    leaf_fp: pallas::Base,
    amount: u64,
) -> StdResult<(u32, [u8; 32])> {
    let count = leaf_count(storage)?;
    if count >= max_leaves(tree_depth(storage)?) {
        return Err(StdError::msg("tree full"));
    }
    let key_hex = hex::encode(key_com.as_slice());
    if KEYS.may_load(storage, key_hex.as_str())?.is_some() {
        return Err(StdError::msg("key commitment"));
    }
    let leaf = leaf_fp.to_repr();
    let mut frontier = load_frontier(storage)?;
    let mut done: Vec<(u8, u32, pallas::Base)> = Vec::new();
    let root = append_leaf_with(&mut frontier, count, leaf, |l, j, node| {
        done.push((l, j, *node));
        Ok(())
    })?;
    for (l, j, node) in done {
        NODES.save(storage, (l, j), &Binary::from(node.to_repr().to_vec()))?;
    }
    FRONTIER.save(
        storage,
        &frontier
            .iter()
            .map(|node| Binary::from(node.to_vec()))
            .collect(),
    )?;
    COUNT.save(storage, &(count + 1))?;
    ROOT.save(storage, &Binary::from(root.to_vec()))?;
    LEAVES.save(storage, count, &Binary::from(leaf.to_vec()))?;
    KEYS.save(
        storage,
        key_hex.as_str(),
        &RegisteredKey {
            index: count,
            amount,
        },
    )?;
    Ok((count, leaf))
}

pub(crate) fn key_com_used(storage: &dyn Storage, key_com: &[u8]) -> StdResult<bool> {
    Ok(KEYS
        .may_load(storage, hex::encode(key_com).as_str())?
        .is_some())
}

pub fn accept_anchor(
    deps: DepsMut,
    env: &Env,
    campaign: &EpochCampaignParams,
) -> Result<Response, StdError> {
    let cfg = campaign
        .registration
        .as_ref()
        .ok_or_else(|| StdError::msg("registration"))?;
    let next = NEXT_ANCHOR
        .may_load(deps.storage)?
        .unwrap_or(cfg.anchor_start_height);
    if env.block.height < next {
        return Err(StdError::msg("anchor early"));
    }
    let count = leaf_count(deps.storage)?;
    if count < cfg.min_anonymity_set {
        return Err(StdError::msg("anonymity set"));
    }
    let root = ROOT
        .may_load(deps.storage)?
        .unwrap_or_else(|| Binary::from(vec![0u8; 32]));
    let mut anchors = ANCHORS.may_load(deps.storage)?.unwrap_or_default();
    if anchors.last().is_some_and(|latest| latest.root == root) {
        return Err(StdError::msg("anchor unchanged"));
    }
    anchors.push(AcceptedAnchor {
        root: root.clone(),
        leaf_count: count,
    });
    ANCHORS.save(deps.storage, &anchors)?;
    NEXT_ANCHOR.save(deps.storage, &(env.block.height + cfg.anchor_interval))?;
    Ok(Response::new()
        .add_attribute("action", "accept_anchor")
        .add_attribute("anchors", anchors.len().to_string()))
}

pub fn anchor_known(storage: &dyn Storage, anchor: &[u8]) -> StdResult<bool> {
    let anchors = ANCHORS.may_load(storage)?.unwrap_or_default();
    Ok(anchors.iter().any(|item| item.root.as_slice() == anchor))
}

pub(crate) fn has_accepted_anchor(storage: &dyn Storage) -> StdResult<bool> {
    Ok(ANCHORS
        .may_load(storage)?
        .is_some_and(|anchors| !anchors.is_empty()))
}

pub fn batch_already_paid(storage: &dyn Storage, attested: &[u8]) -> StdResult<bool> {
    Ok(PAID_BATCH
        .may_load(storage, hex::encode(attested).as_str())?
        .is_some())
}

pub fn note_paid_batch(storage: &mut dyn Storage, attested: &[u8]) -> StdResult<()> {
    PAID_BATCH.save(storage, hex::encode(attested).as_str(), &())
}

pub fn register_message(ctx: &[u8], key_com: &[u8], amount: u64) -> String {
    format!(
        "headstash-register-v1:{}:{}:{amount}",
        hex::encode(ctx),
        hex::encode(key_com)
    )
}

/// Solana registration message. Signed as raw UTF-8 bytes (no prefix), as
/// Phantom/Solflare `signMessage` does. `ctx` is the same campaign ctx the EVM
/// message binds (it commits to the chain id and contract off-chain).
pub fn register_sol_message(ctx: &[u8], key_com: &[u8], amount: u64) -> String {
    format!(
        "headstash-register-sol-v1:{}:{}:{amount}",
        hex::encode(ctx),
        hex::encode(key_com)
    )
}

pub fn recover_address(
    api: &dyn cosmwasm_std::Api,
    message: &str,
    signature: &[u8],
) -> StdResult<[u8; 20]> {
    if signature.len() != 65 {
        return Err(StdError::msg("signature"));
    }
    let recid = match signature[64] {
        0 | 27 => 0,
        1 | 28 => 1,
        _ => return Err(StdError::msg("signature")),
    };
    let digest = keccak256(&eip191(message));
    let pubkey = api
        .secp256k1_recover_pubkey(&digest, &signature[..64], recid)
        .map_err(|_| StdError::msg("signature"))?;
    if pubkey.len() != 65 || pubkey[0] != 0x04 {
        return Err(StdError::msg("signature"));
    }
    let hash = keccak256(&pubkey[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Ok(address)
}

fn eip191(message: &str) -> Vec<u8> {
    let mut out = b"\x19Ethereum Signed Message:\n".to_vec();
    out.extend_from_slice(message.len().to_string().as_bytes());
    out.extend_from_slice(message.as_bytes());
    out
}

pub(crate) fn path_bytes(path: &[Binary]) -> StdResult<Vec<[u8; 32]>> {
    path.iter()
        .map(|sibling| {
            sibling
                .as_slice()
                .try_into()
                .map_err(|_| StdError::msg("eligibility"))
        })
        .collect()
}

pub fn open_eligibility(
    address: &[u8; 20],
    amount: u64,
    index: u32,
    path: &[[u8; 32]],
    depth: u32,
) -> StdResult<[u8; 32]> {
    if depth == 0 || depth > MAX_DEPTH as u32 || path.len() != depth as usize {
        return Err(StdError::msg("eligibility"));
    }
    if index >= 1u32 << depth {
        return Err(StdError::msg("eligibility"));
    }
    open_eligibility_leaf(eligibility_leaf(address, amount), index, path, depth)
}

/// Fold a pre-hashed eligibility leaf (EVM or Solana) up to the root.
pub fn open_eligibility_leaf(
    leaf: [u8; 32],
    index: u32,
    path: &[[u8; 32]],
    depth: u32,
) -> StdResult<[u8; 32]> {
    if depth == 0 || depth > MAX_DEPTH as u32 || path.len() != depth as usize {
        return Err(StdError::msg("eligibility"));
    }
    if index >= 1u32 << depth {
        return Err(StdError::msg("eligibility"));
    }
    let mut node = leaf;
    for level in 0..depth {
        let sibling = path[level as usize];
        node = if (index >> level) & 1 == 0 {
            eligibility_node(&node, &sibling)
        } else {
            eligibility_node(&sibling, &node)
        };
    }
    Ok(node)
}

pub fn eligibility_leaf(address: &[u8; 20], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.extend_from_slice(address);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

/// Domain tag for Solana eligibility leaves.
pub const SOL_LEAF_TAG: u8 = 0x02;

/// Solana eligibility leaf:
/// `sha256("terp-hs-elig-leaf-v1" || 0x02 || pubkey[32] || amount_le[8])` (61-byte preimage).
/// EVM leaves stay `sha256("terp-hs-elig-leaf-v1" || address[20] || amount_le[8])`
/// (48-byte preimage), so the two families have different preimage lengths and
/// one can never open as the other. Both share the same node hash and tree.
pub fn sol_eligibility_leaf(pubkey: &[u8; 32], amount: u64) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-leaf-v1".to_vec();
    pre.push(SOL_LEAF_TAG);
    pre.extend_from_slice(pubkey);
    pre.extend_from_slice(&amount.to_le_bytes());
    sha256(&pre)
}

pub fn eligibility_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

pub fn append_leaf(
    frontier: &mut [[u8; 32]],
    index: u32,
    leaf: [u8; 32],
) -> StdResult<[u8; 32]> {
    append_leaf_with(frontier, index, leaf, |_, _, _| Ok(()))
}

/// [`append_leaf`] that also reports each subtree completed by this leaf as
/// (level, node index, root), level >= 1.
fn append_leaf_with(
    frontier: &mut [[u8; 32]],
    index: u32,
    leaf: [u8; 32],
    mut completed: impl FnMut(u8, u32, &pallas::Base) -> StdResult<()>,
) -> StdResult<[u8; 32]> {
    let mut current = parse_fp(&leaf)?;
    let mut cursor = index;
    for level in 0..frontier.len() {
        if cursor & 1 == 0 {
            frontier[level] = current.to_repr();
            current = poseidon_distro_crh(level as u32, current, pallas::Base::ZERO);
        } else {
            let left = parse_fp(&frontier[level])?;
            current = poseidon_distro_crh(level as u32, left, current);
            if cursor >> 1 == index >> (level + 1) && (index + 1) % (1u32 << (level + 1)) == 0 {
                completed((level + 1) as u8, index >> (level + 1), &current)?;
            }
        }
        cursor >>= 1;
    }
    Ok(current.to_repr())
}

fn load_frontier(storage: &dyn Storage) -> StdResult<Vec<[u8; 32]>> {
    let depth = tree_depth(storage)?;
    let stored = FRONTIER
        .may_load(storage)?
        .unwrap_or_else(|| vec![Binary::from(vec![0u8; 32]); depth]);
    if stored.len() != depth {
        return Err(StdError::msg("registration"));
    }
    let mut out = vec![[0u8; 32]; depth];
    for (slot, node) in out.iter_mut().enumerate() {
        *node = stored[slot]
            .as_slice()
            .try_into()
            .map_err(|_| StdError::msg("registration"))?;
    }
    Ok(out)
}

pub(crate) fn parse_fp(bytes: &[u8]) -> StdResult<pallas::Base> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StdError::msg("key commitment"))?;
    Option::from(pallas::Base::from_repr(arr)).ok_or_else(|| StdError::msg("key commitment"))
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distro::{self, NEXT_ROOT_ID};
    use crate::epoch::{
        attested_root, root_b64, EpochPayout, HashMerchantSudoMsg, HashMerchantSudoPayload,
        OracleHint, ALGO_ACCEPTED, BATCH_SOURCE_ID, CHAIN_UID,
    };
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
    use crate::tokenfactory::{
        derive_nd, DenomUnit, HeadstashTokenObject, Metadata, NewTokenConfig,
    };
    use crate::wavs::generate_test_wavs_proof;
    use crate::{execute, instantiate, sudo};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Binary;
    use ff::{Field, PrimeField};
    use k256::ecdsa::SigningKey;

    fn setup() -> (
        cosmwasm_std::OwnedDeps<
            cosmwasm_std::testing::MockStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
        cosmwasm_std::Env,
    ) {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: crate::tokenfactory::TokenStrategy::NewFungible(NewTokenConfig {
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
            registration_hook: None,
            registration_hook_gas_limit: None,
        };
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg).unwrap();
        (deps, env)
    }

    fn sign(secret: &[u8; 32], message: &str) -> (Vec<u8>, [u8; 20]) {
        let key = SigningKey::from_slice(secret).unwrap();
        let digest = keccak256(&eip191(message));
        let (signature, recid) = key.sign_prehash_recoverable(&digest).unwrap();
        let mut bytes = signature.to_bytes().as_slice().to_vec();
        bytes.push(recid.to_byte());
        let deps = mock_dependencies();
        let address = recover_address(deps.as_ref().api, message, &bytes).unwrap();
        (bytes, address)
    }

    fn key_bytes(secret_word: u64) -> [u8; 32] {
        registration_key_commitment(pallas::Base::from(secret_word)).to_repr()
    }

    fn signer(byte: u8) -> [u8; 32] {
        let mut secret = [0u8; 32];
        secret[31] = byte;
        secret
    }

    #[derive(Clone)]
    struct Eligible {
        secret: [u8; 32],
        key_com: [u8; 32],
        address: [u8; 20],
        signature: Vec<u8>,
        amount: u64,
    }

    fn eligible(byte: u8, ctx: &[u8], word: u64, amount: u64) -> Eligible {
        let key_com = key_bytes(word);
        let message = register_message(ctx, &key_com, amount);
        let (signature, address) = sign(&signer(byte), &message);
        Eligible {
            secret: signer(byte),
            key_com,
            address,
            signature,
            amount,
        }
    }

    fn campaign_with(ctx: &[u8], eligibility_root: [u8; 32], min_set: u32) -> EpochCampaignParams {
        EpochCampaignParams {
            ctx: Binary::from(ctx.to_vec()),
            backend: "flock".into(),
            cutoff_height: 1_000_000, // above mock_env height: registration stays open
            bech32_hrp: "cosmwasm".into(),
            denom: derive_nd("cht"),
            amounts: vec![crate::epoch::AmountCap { amount: 15, cap: 1 }],
            reserved: vec![Binary::from(vec![9u8; 32])],
            trusted: vec![Binary::from(vec![8u8; 32])],
            trusted_verify_threshold: 1,
            trusted_sign_threshold: 1,
            max_rows: 4,
            allocation_root: Binary::from(vec![0u8; 32]),
            root_id: 0,
            vk_id: 0,
            vk_hash: Binary::from(vec![0u8; 32]),
            sweep_recipient: String::new(),
            burn_unclaimed: None,
            registration: Some(RegistrationConfig {
                eligibility_root: Binary::from(eligibility_root.to_vec()),
                eligibility_depth: 1,
                min_anonymity_set: min_set,
                anchor_start_height: 1,
                anchor_interval: 1,
            }),
        }
    }

    #[test]
    fn campaign_creation_uses_an_accepted_anchor_and_refuses_a_stranger() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let eligibility = [4u8; 32];
        let (mut deps, env) = setup();
        let anchor = Binary::from([0x11u8; 32].to_vec());
        ANCHORS
            .save(
                deps.as_mut().storage,
                &vec![AcceptedAnchor {
                    root: anchor.clone(),
                    leaf_count: 1,
                }],
            )
            .unwrap();
        let creator = deps.api.addr_make("creator");
        let mut stranger = campaign_with(&ctx, eligibility, 1);
        stranger.allocation_root = Binary::from([0x22u8; 32].to_vec());
        let err = execute(
            deps.as_mut(),
            env.clone(),
            message_info(&creator, &[]),
            ExecuteMsg::SetEpochCampaign { campaign: stranger },
        )
        .unwrap_err();
        assert!(err.to_string().contains("anchor"));
        assert!(distro::require_root(deps.as_ref().storage, 0)
            .unwrap()
            .root
            .as_slice()
            .iter()
            .all(|byte| *byte == 0));

        let mut params = campaign_with(&ctx, eligibility, 1);
        params.allocation_root = anchor.clone();
        execute(
            deps.as_mut(),
            env,
            message_info(&creator, &[]),
            ExecuteMsg::SetEpochCampaign { campaign: params },
        )
        .unwrap();
        assert_eq!(
            distro::require_root(deps.as_ref().storage, 0).unwrap().root,
            anchor
        );
        assert_eq!(NEXT_ROOT_ID.load(deps.as_ref().storage).unwrap(), 1);
    }

    fn install(deps: DepsMut, env: &cosmwasm_std::Env, params: EpochCampaignParams) {
        let creator = mock_dependencies().api.addr_make("creator");
        execute(
            deps,
            env.clone(),
            message_info(&creator, &[]),
            ExecuteMsg::SetEpochCampaign { campaign: params },
        )
        .unwrap();
    }

    fn do_register(
        deps: DepsMut,
        env: &cosmwasm_std::Env,
        who: &Eligible,
        index: u32,
        sibling: [u8; 32],
    ) -> Result<Response, StdError> {
        let sender = mock_dependencies().api.addr_make("relayer");
        execute(
            deps,
            env.clone(),
            message_info(&sender, &[]),
            ExecuteMsg::RegisterHeadstash {
                key_com: Binary::from(who.key_com.to_vec()),
                amount: who.amount,
                signature: Binary::from(who.signature.clone()),
                eligibility_index: index,
                eligibility_path: vec![Binary::from(sibling.to_vec())],
            },
        )
    }

    fn snapshot(storage: &dyn Storage) -> (u32, Vec<u8>, Vec<Binary>) {
        (
            leaf_count(storage).unwrap(),
            ROOT.load(storage).unwrap().to_vec(),
            FRONTIER.load(storage).unwrap(),
        )
    }

    #[test]
    fn registration_root_matches_an_offchain_rebuild() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let first = eligible(1, &ctx, 11, 15);
        let other = eligible(2, &ctx, 12, 15);
        let leaf0 = eligibility_leaf(&first.address, first.amount);
        let leaf1 = eligibility_leaf(&other.address, other.amount);
        let root = eligibility_node(&leaf0, &leaf1);
        let (mut deps, env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, root, 1));
        let res = do_register(deps.as_mut(), &env, &first, 0, leaf1).unwrap();
        let index = res.attributes.iter().find(|a| a.key == "index").unwrap();
        assert_eq!(index.value, "0");
        let mut frontier = [[0u8; 32]; DEPTH];
        let leaf = registration_leaf(parse_fp(&first.key_com).unwrap(), 15).to_repr();
        let rebuilt = append_leaf(&mut frontier, 0, leaf).unwrap();
        let stored = ROOT.load(deps.as_ref().storage).unwrap();
        assert_eq!(stored.as_slice(), rebuilt.as_slice());
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "leaf")
                .unwrap()
                .value,
            hex::encode(leaf)
        );
        let key = registration_key_commitment(pallas::Base::from(11u64));
        assert_eq!(key.to_repr().as_slice(), first.key_com.as_slice());
        let leaf_fp = registration_leaf(key, 15);
        assert_eq!(leaf_fp.to_repr().as_slice(), leaf.as_slice());
        let mut rolled = leaf_fp;
        for level in 0..DEPTH {
            rolled = poseidon_distro_crh(level as u32, rolled, pallas::Base::ZERO);
        }
        assert_eq!(rebuilt.as_slice(), rolled.to_repr().as_slice());
    }

    #[test]
    fn rejects_a_bad_signature_an_ineligible_address_a_repeat_and_a_duplicate_key() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let first = eligible(1, &ctx, 11, 15);
        let second = eligible(2, &ctx, 12, 15);
        let leaf0 = eligibility_leaf(&first.address, 15);
        let leaf1 = eligibility_leaf(&second.address, 15);
        let root = eligibility_node(&leaf0, &leaf1);
        let (mut deps, env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, root, 1));
        let before = snapshot(deps.as_ref().storage);
        let mut bad = first.signature.clone();
        bad[64] = 2;
        let bad_sig = Eligible {
            signature: bad,
            ..first.clone()
        };
        let err = do_register(deps.as_mut(), &env, &bad_sig, 0, leaf1).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), before);

        let stranger = eligible(3, &ctx, 13, 15);
        let err = do_register(deps.as_mut(), &env, &stranger, 0, leaf1).unwrap_err();
        assert!(err.to_string().contains("eligibility"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), before);

        do_register(deps.as_mut(), &env, &first, 0, leaf1).unwrap();
        let after = snapshot(deps.as_ref().storage);
        let err = do_register(deps.as_mut(), &env, &first, 0, leaf1).unwrap_err();
        assert!(err.to_string().contains("already registered"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), after);

        let mut cloned = second.clone();
        cloned.key_com = first.key_com;
        let message = register_message(&ctx, &cloned.key_com, cloned.amount);
        let (signature, address) = sign(&cloned.secret, &message);
        cloned.signature = signature;
        cloned.address = address;
        let err = do_register(deps.as_mut(), &env, &cloned, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("key commitment"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), after);
    }

    #[test]
    fn accept_anchor_below_the_minimum_stores_nothing() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let first = eligible(1, &ctx, 11, 15);
        let leaf0 = eligibility_leaf(&first.address, 15);
        let root = eligibility_node(&leaf0, &[0u8; 32]);
        let (mut deps, env) = setup();
        let mut params = campaign_with(&ctx, root, 2);
        params.registration.as_mut().unwrap().eligibility_depth = 1;
        install(deps.as_mut(), &env, params);
        do_register(deps.as_mut(), &env, &first, 0, [0u8; 32]).unwrap();
        let before = ANCHORS.load(deps.as_ref().storage).unwrap();
        let next = NEXT_ANCHOR.load(deps.as_ref().storage).unwrap();
        let sender = deps.api.addr_make("anyone");
        let err = execute(
            deps.as_mut(),
            env,
            message_info(&sender, &[]),
            ExecuteMsg::AcceptAnchor {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("anonymity set"), "{err}");
        assert_eq!(ANCHORS.load(deps.as_ref().storage).unwrap(), before);
        assert_eq!(NEXT_ANCHOR.load(deps.as_ref().storage).unwrap(), next);
    }

    #[test]
    fn older_anchor_pays_and_a_later_root_rejects_the_nullifier() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let first = eligible(1, &ctx, 11, 15);
        let second = eligible(2, &ctx, 12, 15);
        let leaf0 = eligibility_leaf(&first.address, 15);
        let leaf1 = eligibility_leaf(&second.address, 15);
        let elig = eligibility_node(&leaf0, &leaf1);
        let (mut deps, mut env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, elig, 1));
        do_register(deps.as_mut(), &env, &first, 0, leaf1).unwrap();
        let anyone = deps.api.addr_make("anyone");
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&anyone, &[]),
            ExecuteMsg::AcceptAnchor {},
        )
        .unwrap();
        let anchor_a = ROOT.load(deps.as_ref().storage).unwrap();
        env.block.height += 1;
        do_register(deps.as_mut(), &env, &second, 1, leaf0).unwrap();
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&anyone, &[]),
            ExecuteMsg::AcceptAnchor {},
        )
        .unwrap();
        let anchor_b = ROOT.load(deps.as_ref().storage).unwrap();
        assert_ne!(anchor_a, anchor_b);
        let owner = deps.api.addr_make("creator");
        let dest = deps.api.addr_make("dest").to_string();
        let row = EpochPayout {
            nullifier: Binary::from(vec![1u8; 32]),
            denom: derive_nd("cht"),
            destination: dest,
            amount: 15,
            anchor: anchor_a.clone(),
        };
        let hash =
            crate::epoch::accepted_batch(row_root().as_slice(), &[row.clone()], true).unwrap();
        let attested = attested_root(&hash);
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&owner, &[]),
            ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from(vec![4u8; 32])],
                rows: vec![row.clone()],
            },
        )
        .unwrap();
        let paid = sudo(
            deps.as_mut(),
            env.clone(),
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 1_000_000, // attested at the cutoff
                    root: root_b64(&attested),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![OracleHint {
                        source_id: BATCH_SOURCE_ID.into(),
                        value: Binary::from(hash.to_vec()).to_base64(),
                        height: 1,
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
        let row_b = EpochPayout {
            anchor: anchor_b,
            ..row
        };
        let hash_b =
            crate::epoch::accepted_batch(row_root().as_slice(), &[row_b.clone()], true).unwrap();
        let attested_b = attested_root(&hash_b);
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&owner, &[]),
            ExecuteMsg::StageEpochBatch {
                digests: vec![Binary::from(vec![5u8; 32])],
                rows: vec![row_b],
            },
        )
        .unwrap();
        let err = sudo(
            deps.as_mut(),
            env.clone(),
            HashMerchantSudoMsg {
                hash_merchant: HashMerchantSudoPayload {
                    chain_uid: CHAIN_UID.into(),
                    algo: ALGO_ACCEPTED.into(),
                    height: 1_000_001,
                    root: root_b64(&attested_b),
                    attestation_count: 1,
                    block_time: 1,
                    oracle_sources: vec![],
                },
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("nullifier already exists"),
            "{err}"
        );
    }

    #[test]
    fn path_under_an_anchor_rebuilds_that_root() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let first = eligible(1, &ctx, 11, 15);
        let second = eligible(2, &ctx, 12, 15);
        let leaf0 = eligibility_leaf(&first.address, 15);
        let leaf1 = eligibility_leaf(&second.address, 15);
        let elig = eligibility_node(&leaf0, &leaf1);
        let (mut deps, mut env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, elig, 1));
        do_register(deps.as_mut(), &env, &first, 0, leaf1).unwrap();
        let anyone = deps.api.addr_make("anyone");
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&anyone, &[]),
            ExecuteMsg::AcceptAnchor {},
        )
        .unwrap();
        let anchor_a = ROOT.load(deps.as_ref().storage).unwrap();
        let opened = path_for(deps.as_ref(), &env, &first.key_com, &anchor_a);
        assert_eq!(opened.index, 0);
        assert_eq!(opened.amount, 15);
        assert_eq!(opened.siblings.len(), DEPTH);
        assert!(opened
            .siblings
            .iter()
            .all(|sibling| sibling == &"0".repeat(64)));
        assert_eq!(
            fold_path(opened.index, &opened.leaf, &opened.siblings),
            anchor_a.to_vec()
        );
        let tree = tree_for(deps.as_ref(), &env);
        assert_eq!(tree.min_anonymity_set, 1);
        assert_eq!(tree.count, 1);
        assert_eq!(tree.anchors.len(), 1);
        assert_eq!(tree.anchors[0].leaf_count, 1);
        assert_eq!(tree.anchors[0].root, anchor_a);

        env.block.height += 1;
        do_register(deps.as_mut(), &env, &second, 1, leaf0).unwrap();
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&anyone, &[]),
            ExecuteMsg::AcceptAnchor {},
        )
        .unwrap();
        let anchor_b = ROOT.load(deps.as_ref().storage).unwrap();
        let later = path_for(deps.as_ref(), &env, &first.key_com, &anchor_b);
        assert_ne!(later.siblings[0], "0".repeat(64));
        assert_eq!(
            fold_path(later.index, &later.leaf, &later.siblings),
            anchor_b.to_vec()
        );
        let err = crate::query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::RegistrationPath {
                key_com: Binary::from(second.key_com.to_vec()),
                anchor: anchor_a,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("anchor"), "{err}");
    }

    fn tree_for(deps: cosmwasm_std::Deps, env: &cosmwasm_std::Env) -> RegistrationTreeResponse {
        let raw = crate::query(deps, env.clone(), QueryMsg::RegistrationTree {}).unwrap();
        cosmwasm_std::from_json(raw).unwrap()
    }

    fn path_for(
        deps: cosmwasm_std::Deps,
        env: &cosmwasm_std::Env,
        key_com: &[u8],
        anchor: &Binary,
    ) -> RegistrationPathResponse {
        let raw = crate::query(
            deps,
            env.clone(),
            QueryMsg::RegistrationPath {
                key_com: Binary::from(key_com.to_vec()),
                anchor: anchor.clone(),
            },
        )
        .unwrap();
        cosmwasm_std::from_json(raw).unwrap()
    }

    fn fold_path(index: u32, leaf_hex: &str, siblings: &[String]) -> Vec<u8> {
        let leaf = hex::decode(leaf_hex).unwrap();
        let mut current = parse_fp(&leaf).unwrap();
        for (level, sibling) in siblings.iter().enumerate() {
            let bytes = hex::decode(sibling).unwrap();
            let sib = parse_fp(&bytes).unwrap();
            current = if (index >> level) & 1 == 0 {
                poseidon_distro_crh(level as u32, current, sib)
            } else {
                poseidon_distro_crh(level as u32, sib, current)
            };
        }
        current.to_repr().to_vec()
    }

    fn row_root() -> Vec<u8> {
        vec![0u8; 32]
    }

    // ---- Solana (ed25519) registration ----

    struct SolEligible {
        pubkey: [u8; 32],
        key_com: [u8; 32],
        signature: Vec<u8>,
        amount: u64,
    }

    fn sol_key(byte: u8) -> ed25519_zebra::SigningKey {
        ed25519_zebra::SigningKey::from([byte; 32])
    }

    fn sol_eligible(byte: u8, ctx: &[u8], word: u64, amount: u64) -> SolEligible {
        let sk = sol_key(byte);
        let pubkey: [u8; 32] = ed25519_zebra::VerificationKey::from(&sk).into();
        let key_com = key_bytes(word);
        let message = register_sol_message(ctx, &key_com, amount);
        let sig: [u8; 64] = sk.sign(message.as_bytes()).into();
        SolEligible {
            pubkey,
            key_com,
            signature: sig.to_vec(),
            amount,
        }
    }

    fn do_register_sol(
        deps: DepsMut,
        env: &cosmwasm_std::Env,
        who: &SolEligible,
        amount: u64,
        index: u32,
        sibling: [u8; 32],
    ) -> Result<Response, StdError> {
        let sender = mock_dependencies().api.addr_make("sidecar");
        execute(
            deps,
            env.clone(),
            message_info(&sender, &[]),
            ExecuteMsg::RegisterHeadstashSol {
                key_com: Binary::from(who.key_com.to_vec()),
                amount,
                pubkey: Binary::from(who.pubkey.to_vec()),
                signature: Binary::from(who.signature.clone()),
                eligibility_index: index,
                eligibility_path: vec![Binary::from(sibling.to_vec())],
            },
        )
    }

    /// root = node(evm_leaf(first), sol_leaf(sol)).
    fn mixed() -> (
        cosmwasm_std::OwnedDeps<
            cosmwasm_std::testing::MockStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
        cosmwasm_std::Env,
        Eligible,
        SolEligible,
        [u8; 32],
        [u8; 32],
    ) {
        let ctx = pallas::Base::from(3u64).to_repr();
        let evm = eligible(1, &ctx, 11, 15);
        let sol = sol_eligible(7, &ctx, 21, 15);
        let leaf0 = eligibility_leaf(&evm.address, 15);
        let leaf1 = sol_eligibility_leaf(&sol.pubkey, 15);
        let root = eligibility_node(&leaf0, &leaf1);
        let (mut deps, env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, root, 1));
        (deps, env, evm, sol, leaf0, leaf1)
    }

    #[test]
    fn sol_valid_signature_registers_alongside_evm() {
        let (mut deps, env, evm, sol, leaf0, leaf1) = mixed();
        let res = do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap();
        assert!(res.attributes.iter().any(|a| a.key == "kind" && a.value == "sol"));
        do_register(deps.as_mut(), &env, &evm, 0, leaf1).unwrap();
        assert_eq!(leaf_count(deps.as_ref().storage).unwrap(), 2);
        assert!(is_registered(deps.as_ref().storage, &sol_identity(&sol.pubkey)).unwrap());
        assert!(is_registered(deps.as_ref().storage, &evm_identity(&evm.address)).unwrap());
        let q: bool = cosmwasm_std::from_json(
            crate::query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Registered {
                    identity: sol_identity(&sol.pubkey),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(q);
    }

    #[test]
    fn sol_invalid_signature_is_rejected() {
        let (mut deps, env, _evm, mut sol, leaf0, _) = mixed();
        let before = snapshot(deps.as_ref().storage);
        sol.signature[3] ^= 1;
        let err = do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        sol.signature.truncate(63);
        let err = do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), before);
    }

    #[test]
    fn sol_wrong_pubkey_length_is_rejected() {
        let (mut deps, env, _evm, sol, leaf0, _) = mixed();
        let sender = mock_dependencies().api.addr_make("sidecar");
        let err = execute(
            deps.as_mut(),
            env,
            message_info(&sender, &[]),
            ExecuteMsg::RegisterHeadstashSol {
                key_com: Binary::from(sol.key_com.to_vec()),
                amount: 15,
                pubkey: Binary::from(sol.pubkey[..31].to_vec()),
                signature: Binary::from(sol.signature.clone()),
                eligibility_index: 1,
                eligibility_path: vec![Binary::from(leaf0.to_vec())],
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("pubkey"), "{err}");
    }

    #[test]
    fn sol_wrong_ctx_is_rejected() {
        let (mut deps, env, _evm, sol, leaf0, _) = mixed();
        let other_ctx = pallas::Base::from(4u64).to_repr();
        let forged = sol_eligible(7, &other_ctx, 21, 15);
        assert_eq!(forged.pubkey, sol.pubkey);
        let err = do_register_sol(deps.as_mut(), &env, &forged, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
    }

    #[test]
    fn sol_wrong_amount_is_rejected() {
        let (mut deps, env, _evm, sol, leaf0, _) = mixed();
        let err = do_register_sol(deps.as_mut(), &env, &sol, 16, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        // Signed for 16 but only eligible for 15: signature passes, eligibility fails.
        let ctx = pallas::Base::from(3u64).to_repr();
        let greedy = sol_eligible(7, &ctx, 21, 16);
        let err = do_register_sol(deps.as_mut(), &env, &greedy, 16, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("eligibility"), "{err}");
    }

    #[test]
    fn sol_replay_and_duplicate_identity_are_rejected() {
        let (mut deps, env, _evm, sol, leaf0, _) = mixed();
        do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap();
        let after = snapshot(deps.as_ref().storage);
        let err = do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("already registered"), "{err}");
        // Same pubkey, fresh key commitment: still one registration per identity.
        let ctx = pallas::Base::from(3u64).to_repr();
        let again = sol_eligible(7, &ctx, 99, 15);
        let err = do_register_sol(deps.as_mut(), &env, &again, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("already registered"), "{err}");
        assert_eq!(snapshot(deps.as_ref().storage), after);
    }

    #[test]
    fn sol_malleated_signature_is_rejected() {
        let (mut deps, env, _evm, mut sol, leaf0, _) = mixed();
        // s' = s + L (group order) verifies under lax checks; must be refused.
        const L: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let mut carry = 0u16;
        for i in 0..32 {
            let v = sol.signature[32 + i] as u16 + L[i] as u16 + carry;
            sol.signature[32 + i] = v as u8;
            carry = v >> 8;
        }
        let err = do_register_sol(deps.as_mut(), &env, &sol, 15, 1, leaf0).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        assert_eq!(leaf_count(deps.as_ref().storage).unwrap(), 0);
    }

    #[test]
    fn evm_and_sol_leaves_cannot_be_confused() {
        let ctx = pallas::Base::from(3u64).to_repr();
        let evm = eligible(1, &ctx, 11, 15);
        let sol = sol_eligible(7, &ctx, 21, 15);
        let mut padded = [0u8; 32];
        padded[..20].copy_from_slice(&evm.address);
        assert_ne!(sol_eligibility_leaf(&padded, 15), eligibility_leaf(&evm.address, 15));
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&sol.pubkey[..20]);
        assert_ne!(eligibility_leaf(&addr, 15), sol_eligibility_leaf(&sol.pubkey, 15));
        assert_ne!(evm_identity(&addr), sol_identity(&sol.pubkey));

        // Root holding only a Solana leaf + filler: an EVM signer cannot open it.
        let filler = [5u8; 32];
        let root = eligibility_node(&sol_eligibility_leaf(&sol.pubkey, 15), &filler);
        let (mut deps, env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, root, 1));
        let err = do_register(deps.as_mut(), &env, &evm, 0, filler).unwrap_err();
        assert!(err.to_string().contains("eligibility"), "{err}");

        // Root holding only an EVM leaf: no Solana registration opens it.
        let root = eligibility_node(&eligibility_leaf(&evm.address, 15), &filler);
        let (mut deps, env) = setup();
        install(deps.as_mut(), &env, campaign_with(&ctx, root, 1));
        let err = do_register_sol(deps.as_mut(), &env, &sol, 15, 0, filler).unwrap_err();
        assert!(err.to_string().contains("eligibility"), "{err}");
        assert_eq!(leaf_count(deps.as_ref().storage).unwrap(), 0);
    }
}

#[cfg(test)]
mod node_index_tests {
    use super::*;
    use cosmwasm_std::testing::MockStorage;
    use cosmwasm_std::Order;
    use std::cell::Cell;

    /// Storage wrapper that counts reads, standing in for per-read query gas.
    struct Counting<'a> {
        inner: &'a MockStorage,
        reads: Cell<u64>,
    }
    impl Storage for Counting<'_> {
        fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
            self.reads.set(self.reads.get() + 1);
            self.inner.get(key)
        }
        fn range<'b>(
            &'b self,
            start: Option<&[u8]>,
            end: Option<&[u8]>,
            order: Order,
        ) -> Box<dyn Iterator<Item = cosmwasm_std::Record> + 'b> {
            self.inner.range(start, end, order)
        }
        fn set(&mut self, _: &[u8], _: &[u8]) {
            unreachable!()
        }
        fn remove(&mut self, _: &[u8]) {
            unreachable!()
        }
    }

    fn leaf(i: u32) -> [u8; 32] {
        // Deterministic pseudo-random field elements.
        let x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        pallas::Base::from(x).to_repr()
    }

    /// Mirrors insert_registration's tree writes (frontier, LEAVES, NODES).
    fn build(n: u32) -> (MockStorage, Vec<[u8; 32]>, Vec<[u8; 32]>) {
        let mut st = MockStorage::new();
        let mut frontier = [[0u8; 32]; DEPTH];
        let mut leaves = Vec::new();
        let mut roots = Vec::new();
        for i in 0..n {
            let l = leaf(i);
            let mut done = Vec::new();
            let root = append_leaf_with(&mut frontier, i, l, |lv, j, node| {
                done.push((lv, j, *node));
                Ok(())
            })
            .unwrap();
            for (lv, j, node) in done {
                NODES
                    .save(&mut st, (lv, j), &Binary::from(node.to_repr().to_vec()))
                    .unwrap();
            }
            LEAVES.save(&mut st, i, &Binary::from(l.to_vec())).unwrap();
            leaves.push(l);
            roots.push(root);
        }
        (st, leaves, roots)
    }

    fn indexed(st: &dyn Storage, index: u32, n: u32) -> Vec<String> {
        (0..DEPTH)
            .map(|level| {
                let b = match node_at(st, level, (index >> level) ^ 1, n).unwrap() {
                    Some(node) => node.to_repr(),
                    None => [0u8; 32],
                };
                hex::encode(b)
            })
            .collect()
    }

    fn fold(index: u32, leaf: [u8; 32], sib: &[String]) -> [u8; 32] {
        let mut cur = parse_fp(&leaf).unwrap();
        for (level, s) in sib.iter().enumerate() {
            let s = parse_fp(&hex::decode(s).unwrap()).unwrap();
            cur = if (index >> level) & 1 == 0 {
                poseidon_distro_crh(level as u32, cur, s)
            } else {
                poseidon_distro_crh(level as u32, s, cur)
            };
        }
        cur.to_repr()
    }

    #[test]
    fn indexed_siblings_match_reference_for_every_anchor_and_index() {
        let total = 70u32;
        let (st, leaves, roots) = build(total);
        for n in 1..=total {
            for index in 0..n {
                let got = indexed(&st, index, n);
                let want = siblings_under(&leaves[..n as usize], index).unwrap();
                assert_eq!(got, want, "n={n} index={index}");
                assert_eq!(fold(index, leaves[index as usize], &got), roots[n as usize - 1]);
            }
        }
    }

    #[test]
    fn indexed_siblings_match_reference_at_sampled_large_counts() {
        let total = 1100u32;
        let (st, leaves, roots) = build(total);
        let mut seed = 0x1234_5678u64;
        for n in [127, 128, 129, 255, 256, 511, 512, 513, 1023, 1024, 1025, 1100] {
            for _ in 0..6 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let index = ((seed >> 33) as u32) % n;
                for idx in [index, 0, n - 1] {
                    let got = indexed(&st, idx, n);
                    assert_eq!(got, siblings_under(&leaves[..n as usize], idx).unwrap(), "n={n} index={idx}");
                    assert_eq!(fold(idx, leaves[idx as usize], &got), roots[n as usize - 1]);
                }
            }
        }
    }

    #[test]
    fn missing_nodes_fall_back_to_leaves() {
        // A contract that registered before NODES existed: no node entries at all.
        let (mut st, leaves, _) = build(37);
        let keys: Vec<(u8, u32)> = NODES
            .keys(&st, None, None, Order::Ascending)
            .map(|k| k.unwrap())
            .collect();
        assert!(!keys.is_empty());
        for k in keys {
            NODES.remove(&mut st, k);
        }
        for n in [1u32, 2, 3, 16, 17, 32, 37] {
            for index in 0..n {
                assert_eq!(
                    indexed(&st, index, n),
                    siblings_under(&leaves[..n as usize], index).unwrap()
                );
            }
        }
    }

    #[test]
    fn read_count_at_large_n() {
        let n = 4096u32;
        let (st, leaves, _) = build(n);
        let probe = Counting { inner: &st, reads: Cell::new(0) };
        let mut worst = 0;
        for (anchor, index) in [(n, 0u32), (n, n - 1), (n - 1, 1234), (3001, 2999), (2049, 2048)] {
            probe.reads.set(0);
            let got = indexed(&probe, index, anchor);
            assert_eq!(got, siblings_under(&leaves[..anchor as usize], index).unwrap());
            worst = worst.max(probe.reads.get());
        }
        // Old query_path read every leaf (anchor reads); new bound is DEPTH^2.
        println!("reads: indexed worst {worst} vs old {n}");
        assert!(worst <= (DEPTH * DEPTH) as u64, "worst {worst}");
    }
}
