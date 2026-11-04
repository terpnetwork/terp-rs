//! Direct on-chain note claims (Headstash v1, vk_id 3).
//!
//! The claimant's fresh wallet submits its own vk_id 3 proof. The contract
//! rebuilds the six public limbs itself and calls `proof_instance_verify`
//! against the pinned circuit id. Nothing here touches hashmerchant, a lane,
//! or a seal.
//!
//! Limbs (32-byte little-endian pallas reprs, concatenated, 192 bytes):
//!   ctx      = campaign.ctx, which must equal `direct_ctx(chain_id, contract)`
//!   root     = a closed-epoch registration anchor (never the live tree)
//!   nullifier= Poseidon(nf tag, secret, ctx) (root-independent: one spend per note
//!              across every anchor)
//!   dest_lo/hi = `pack_destination(info.sender)` (payout bound to the sender)
//!   amount   = a denomination in the campaign table
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError, StdResult, Storage};
use cw_storage_plus::{Item, Map};
use sha2::{Digest, Sha256};

use crate::distro::nullifier_storage_key;
use crate::epoch::EpochCampaignParams;
use crate::headstash::verify_nullifier_stateful;
use crate::notes::{DENOMS_UTERP, NOTE_VK_ID};
use crate::HEADSTASH_CFG;

pub const CTX_TAG: &[u8] = b"terp-hs-direct-ctx-v1";

#[cw_serde]
pub struct DirectClaimCfg {
    /// wasmvm circuit id. Bound once; its stored blob hashed to campaign.vk_hash.
    pub zk_id: u64,
    pub vk_hash: Binary,
    /// CircuitInfo key at bind time (diagnostic).
    pub circuit_key: Binary,
}

#[cw_serde]
pub struct DirectClaimInfo {
    pub cfg: Option<DirectClaimCfg>,
    pub expected_ctx: Binary,
    pub claimed: u64,
}

const DIRECT_CFG: Item<DirectClaimCfg> = Item::new("direct_cfg");
const DIRECT_CLAIMED: Item<u64> = Item::new("direct_claimed");
/// Count paid per denomination (enforces AmountCap.cap).
const DIRECT_CLASS_PAID: Map<u64, u32> = Map::new("direct_class_paid");

/// `sha256(tag || 0 || chain_id || 0 || contract)` with the top two bits cleared, so
/// it is a canonical pallas element. Binds every proof to this chain and contract.
pub fn direct_ctx(chain_id: &str, contract: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(CTX_TAG);
    h.update([0u8]);
    h.update(chain_id.as_bytes());
    h.update([0u8]);
    h.update(contract.as_bytes());
    let mut out: [u8; 32] = h.finalize().into();
    out[31] &= 0x3f;
    out
}

/// Same packing as the circuit's `pack_destination`.
pub fn pack_destination(dest: &str) -> StdResult<([u8; 32], [u8; 32])> {
    let b = dest.as_bytes();
    if b.len() > 62 {
        return Err(StdError::msg("destination"));
    }
    let mut lo = [0u8; 32];
    let mut hi = [0u8; 32];
    let split = core::cmp::min(31, b.len());
    lo[..split].copy_from_slice(&b[..split]);
    if b.len() > 31 {
        hi[..b.len() - 31].copy_from_slice(&b[31..]);
    }
    Ok((lo, hi))
}

pub fn amount_limb(amount: u64) -> [u8; 32] {
    let mut l = [0u8; 32];
    l[..8].copy_from_slice(&amount.to_le_bytes());
    l
}

pub fn instances(ctx: &[u8], root: &[u8], nf: &[u8], dest: &str, amount: u64) -> StdResult<Vec<u8>> {
    let (lo, hi) = pack_destination(dest)?;
    let mut v = Vec::with_capacity(192);
    v.extend_from_slice(ctx);
    v.extend_from_slice(root);
    v.extend_from_slice(nf);
    v.extend_from_slice(&lo);
    v.extend_from_slice(&hi);
    v.extend_from_slice(&amount_limb(amount));
    Ok(v)
}

fn canonical32(b: &[u8], what: &str) -> StdResult<()> {
    if b.len() != 32 || b[31] > 0x3f {
        return Err(StdError::msg(what.to_string()));
    }
    Ok(())
}

fn note_campaign(campaign: &EpochCampaignParams) -> StdResult<()> {
    if campaign.vk_id != NOTE_VK_ID || campaign.registration.is_none() {
        return Err(StdError::msg("not a note campaign"));
    }
    Ok(())
}

/// Owner, once. Pins the circuit id whose stored blob hashes to the campaign vk_hash.
pub fn bind_circuit(deps: DepsMut, env: &Env, info: MessageInfo, campaign: &EpochCampaignParams, zk_id: u64) -> Result<Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;
    note_campaign(campaign)?;
    if DIRECT_CFG.may_load(deps.storage)?.is_some() {
        return Err(StdError::msg("circuit is fixed"));
    }
    let expected = direct_ctx(&env.block.chain_id, env.contract.address.as_str());
    if campaign.ctx.as_slice() != expected {
        return Err(StdError::msg("ctx is not bound to this contract"));
    }
    if zk_id == 0 {
        return Err(StdError::msg("zk id"));
    }
    let (hash, key) = circuit_hash(deps.as_ref(), zk_id)?;
    if hash.as_slice() != campaign.vk_hash.as_slice() {
        return Err(StdError::msg("circuit does not match vk hash"));
    }
    DIRECT_CFG.save(deps.storage, &DirectClaimCfg { zk_id, vk_hash: campaign.vk_hash.clone(), circuit_key: key })?;
    Ok(Response::new().add_attribute("action", "bind_direct_circuit").add_attribute("zk_id", zk_id.to_string()))
}

#[cfg(feature = "zk-api")]
fn circuit_hash(deps: Deps, zk_id: u64) -> StdResult<([u8; 32], Binary)> {
    let data = deps.querier.query_circuit(zk_id)?.data;
    let info = deps.querier.query_circuit_info(zk_id)?;
    Ok((Sha256::digest(data.as_slice()).into(), info.circuit_key))
}

#[cfg(not(feature = "zk-api"))]
fn circuit_hash(_deps: Deps, _zk_id: u64) -> StdResult<([u8; 32], Binary)> {
    #[cfg(test)]
    if let Some(h) = test_hook::get() {
        return Ok((h, Binary::default()));
    }
    Err(StdError::msg("direct claims require the zk-api feature"))
}

#[cfg(test)]
pub mod test_hook {
    use std::cell::Cell;
    thread_local!(static H: Cell<Option<[u8; 32]>> = const { Cell::new(None) });
    thread_local!(static OK: Cell<bool> = const { Cell::new(true) });
    pub fn set(h: Option<[u8; 32]>) { H.with(|c| c.set(h)) }
    pub fn get() -> Option<[u8; 32]> { H.with(|c| c.get()) }
    pub fn set_ok(v: bool) { OK.with(|c| c.set(v)) }
    pub fn ok() -> bool { OK.with(|c| c.get()) }
}

fn verify(deps: Deps, zk_id: u64, proof: &[u8], inst: &[u8]) -> StdResult<()> {
    if proof.is_empty() {
        return Err(StdError::msg("invalid note proof"));
    }
    #[cfg(feature = "zk-api")]
    {
        let ok = deps.api.proof_instance_verify(zk_id, proof, inst).map_err(|_| StdError::msg("invalid note proof"))?;
        if !ok {
            return Err(StdError::msg("invalid note proof"));
        }
        Ok(())
    }
    #[cfg(not(feature = "zk-api"))]
    {
        let _ = (deps, zk_id, inst);
        #[cfg(test)]
        if test_hook::ok() {
            return Ok(());
        }
        Err(StdError::msg("invalid note proof"))
    }
}

/// Every read-only check a claim must pass (shared by `ClaimNote` and the
/// `CheckNoteClaim` query the fee-grant sidecar uses before granting).
pub fn check_claim(
    deps: Deps,
    env: &Env,
    campaign: &EpochCampaignParams,
    dest: &str,
    root: &Binary,
    nullifier: &Binary,
    amount: u64,
    proof: &Binary,
) -> StdResult<(u32, String)> {
    note_campaign(campaign)?;
    let cfg = DIRECT_CFG.may_load(deps.storage)?.ok_or_else(|| StdError::msg("direct claims are not enabled"))?;
    if cfg.vk_hash != campaign.vk_hash {
        return Err(StdError::msg("vk hash"));
    }
    if env.block.height > campaign.cutoff_height {
        return Err(StdError::msg("claim window closed"));
    }
    canonical32(root, "root")?;
    canonical32(nullifier, "nullifier")?;
    if !crate::registration::anchor_known(deps.storage, root.as_slice())? {
        return Err(StdError::msg("root is not an accepted anchor"));
    }
    if !DENOMS_UTERP.contains(&amount) {
        return Err(StdError::msg("amount is not a denomination"));
    }
    let class = campaign.amounts.iter().find(|c| c.amount == amount).ok_or_else(|| StdError::msg("amount not in campaign"))?;
    let paid = DIRECT_CLASS_PAID.may_load(deps.storage, amount)?.unwrap_or(0);
    if paid >= class.cap {
        return Err(StdError::msg("denomination cap reached"));
    }
    let nf_hex = hex::encode(nullifier.as_slice());
    if crate::epoch::is_public_reserved(deps.storage, &nf_hex)? {
        return Err(StdError::msg("note is on the public path"));
    }
    let key = nullifier_storage_key(campaign.root_id, &nf_hex);
    if crate::NULLIFIERS.has(deps.storage, key.clone()) {
        return Err(StdError::msg("nullifier already exists"));
    }
    let inst = instances(campaign.ctx.as_slice(), root.as_slice(), nullifier.as_slice(), dest, amount)?;
    verify(deps, cfg.zk_id, proof.as_slice(), &inst)?;
    Ok((paid, key))
}

/// Anyone. Pays `amount` to `info.sender` when the proof verifies under the pinned circuit.
/// Proof check for `dest` plus the spend bookkeeping shared by `ClaimNote` and
/// `ClaimNoteForward`. Returns the nullifier hex.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_claim(
    deps: DepsMut,
    env: &Env,
    campaign: &EpochCampaignParams,
    dest: &str,
    root: &Binary,
    nullifier: &Binary,
    amount: u64,
    proof: &Binary,
) -> Result<String, StdError> {
    let (paid, key) = check_claim(deps.as_ref(), env, campaign, dest, root, nullifier, amount, proof)?;
    // Shared nullifier set with the lane path, same root scope: one spend per note.
    verify_nullifier_stateful(deps.storage, key)?;
    DIRECT_CLASS_PAID.save(deps.storage, amount, &(paid + 1))?;
    let n = DIRECT_CLAIMED.may_load(deps.storage)?.unwrap_or(0) + 1;
    DIRECT_CLAIMED.save(deps.storage, &n)?;
    Ok(hex::encode(nullifier.as_slice()))
}

pub fn claim_note(
    mut deps: DepsMut,
    env: &Env,
    info: MessageInfo,
    campaign: &EpochCampaignParams,
    root: Binary,
    nullifier: Binary,
    amount: u64,
    proof: Binary,
) -> Result<Response, StdError> {
    record_claim(deps.branch(), env, campaign, info.sender.as_str(), &root, &nullifier, amount, &proof)?;
    let hcfg = HEADSTASH_CFG.load(deps.storage)?;
    let strategy = crate::epoch::strategy_for(&hcfg.ts, &campaign.denom)?;
    let res = Response::new().add_attribute("action", "claim_note").add_attribute("amount", amount.to_string());
    crate::epoch::push_credit(res, env, strategy, info.sender.as_str(), amount)
}

pub fn query_info(storage: &dyn Storage, env: &Env) -> StdResult<DirectClaimInfo> {
    Ok(DirectClaimInfo {
        cfg: DIRECT_CFG.may_load(storage)?,
        expected_ctx: Binary::from(direct_ctx(&env.block.chain_id, env.contract.address.as_str()).to_vec()),
        claimed: DIRECT_CLAIMED.may_load(storage)?.unwrap_or(0),
    })
}
