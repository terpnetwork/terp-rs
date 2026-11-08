//! Spec rev 5 note registration (vk_id 3). One signature per identity over the
//! ordered note list hash and the reward address (v3 message); chunks append
//! `note_leaf(key_com, DENOMS[class])` into the depth-17 claim tree. vk_id 3 proofs are
//! verified on-chain by the direct path (`crate::direct`, `ClaimNote`); the sidecar lane
//! (epoch seal + hashmerchant aggregate) is the optional lane path. When the list
//! completes, an optional registration hook contract is called (see `REG_HOOK`).

use cosmwasm_std::{to_json_binary, Addr, Api, Binary, Deps, DepsMut, Response, StdError, StdResult, Storage, SubMsg, WasmMsg};
use cw_storage_plus::{Item, Map};
use std::collections::HashSet;

use crate::epoch::EpochCampaignParams;
use crate::registration::{
    append_claim_leaf, key_com_used, parse_fp, path_bytes, precheck, registration_cfg,
    sha256, NOTE_DEPTH,
};
use zk_headstash::distro_poseidon::note_registration_leaf;

pub const NOTE_VK_ID: u64 = 3;
pub const RETIRED_VK_ID: u64 = 2;
/// Error text for any fund-moving path on a vk_id 2 campaign (finding F1).
pub const RETIRED_VK_ERR: &str = "vk id 2 is retired";

/// F1: vk_id 2 loads its Poseidon tags as free advice, so one allocation can be
/// claimed under many nullifiers. No path that pays a claimant accepts it.
pub fn reject_retired_vk(campaign: &EpochCampaignParams) -> StdResult<()> {
    if campaign.vk_id == RETIRED_VK_ID {
        return Err(StdError::msg(RETIRED_VK_ERR));
    }
    Ok(())
}
/// sha256 of the vk_id 3 circuit blob (`note_membership::NOTE_VK_HASH`).
pub const NOTE_VK_HASH: [u8; 32] = [
    0x34, 0xd1, 0x14, 0x37, 0xd9, 0x57, 0x55, 0x4f, 0x40, 0x08, 0x37, 0x7d, 0xec, 0xe5, 0xfa,
    0xc0, 0xf0, 0x47, 0xe0, 0xe9, 0xcc, 0x01, 0x2d, 0x15, 0xb3, 0xd0, 0x94, 0x48, 0xa6, 0xcc,
    0x53, 0x98,
];
/// 1-2-5 series 1..5,000 TERP in uterp. Index = denom class (N7).
pub const DENOMS_UTERP: [u64; 12] = [
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    20_000_000,
    50_000_000,
    100_000_000,
    200_000_000,
    500_000_000,
    1_000_000_000,
    2_000_000_000,
    5_000_000_000,
];
pub const MAX_NOTES_PER_IDENTITY: usize = 256;
/// Notes appended per RegisterNotes message.
pub const MAX_CHUNK: u32 = 32;
pub const EVM_ID_TAG: u8 = 0x01;
pub const SOL_ID_TAG: u8 = 0x02;

#[cosmwasm_schema::cw_serde]
pub enum NoteSigner {
    /// `signature` is 65-byte EIP-191 over [`register_notes_message`] (first chunk only).
    Evm { address: Binary },
    /// `signature` is 64-byte ed25519 over the raw UTF-8 of [`register_notes_sol_message`].
    Sol { pubkey: Binary },
}

#[cosmwasm_schema::cw_serde]
pub struct NoteListEntry {
    pub class: u8,
    pub key_com: Binary,
}

#[cosmwasm_schema::cw_serde]
pub struct NoteEligibility {
    pub index: u32,
    pub path: Vec<Binary>,
}

#[cosmwasm_schema::cw_serde]
pub struct NoteListState {
    pub list_hash: Binary,
    pub n: u32,
    pub next: u32,
    /// Terp address bound by the v3 signature, passed to the registration hook.
    #[serde(default)]
    pub reward_addr: Option<String>,
}

/// Optional contract called once per identity when its note list completes.
#[cosmwasm_schema::cw_serde]
pub struct RegistrationHook {
    pub contract: Addr,
    pub gas_limit: u64,
}

/// Message sent to the hook (no funds). The hook links identity to reward_addr publicly.
#[cosmwasm_schema::cw_serde]
pub enum RegistrationHookMsg {
    HeadstashRegistered {
        /// hex campaign ctx
        campaign: String,
        id_tag: u8,
        /// hex EVM address (20 bytes) or Solana pubkey (32 bytes)
        identity: String,
        note_count: u32,
        reward_addr: Option<String>,
    },
}

pub const REG_HOOK: Item<RegistrationHook> = Item::new("reg_hook");
pub const HOOK_REPLY_ID: u64 = 0x4855_4b;
pub const DEFAULT_HOOK_GAS: u64 = 500_000;
pub const MAX_HOOK_GAS: u64 = 5_000_000;

/// Instantiate-time only (no update path): validate and store the hook.
pub fn set_registration_hook(storage: &mut dyn Storage, api: &dyn Api, hook: Option<String>, gas_limit: Option<u64>) -> StdResult<()> {
    let Some(hook) = hook.filter(|h| !h.is_empty()) else {
        if gas_limit.is_some() {
            return Err(StdError::msg("registration hook gas limit without a hook"));
        }
        return Ok(());
    };
    let contract = api.addr_validate(&hook)?;
    let gas_limit = gas_limit.unwrap_or(DEFAULT_HOOK_GAS);
    if gas_limit == 0 || gas_limit > MAX_HOOK_GAS {
        return Err(StdError::msg("registration hook gas limit"));
    }
    REG_HOOK.save(storage, &RegistrationHook { contract, gas_limit })
}

pub fn query_registration_hook(storage: &dyn Storage) -> StdResult<Option<RegistrationHook>> {
    REG_HOOK.may_load(storage)
}

/// Bech32 Terp address under the campaign hrp, canonical form.
fn check_reward_addr(api: &dyn Api, campaign: &EpochCampaignParams, addr: &str) -> StdResult<()> {
    if !addr.starts_with(&format!("{}1", campaign.bech32_hrp)) {
        return Err(StdError::msg("reward_addr prefix"));
    }
    let a = api.addr_validate(addr).map_err(|_| StdError::msg("reward_addr"))?;
    if a.as_str() != addr {
        return Err(StdError::msg("reward_addr"));
    }
    Ok(())
}

/// identity key (`evm_identity` / `sol_identity`) -> signed list state.
const NOTE_LIST: Map<&str, NoteListState> = Map::new("note_list");
/// (identity key, note index) -> claim tree leaf index.
const NOTE_REG: Map<(&str, u16), u32> = Map::new("note_reg");

/// N1 + N7: a vk_id 3 campaign is a halo2 registration campaign under the pinned vk
/// whose every amount class is one of the 12 denominations.
pub fn validate_note_campaign(campaign: &EpochCampaignParams) -> StdResult<()> {
    if campaign.vk_id != NOTE_VK_ID {
        return Ok(());
    }
    if campaign.backend != "halo2" || campaign.vk_hash.as_slice() != NOTE_VK_HASH {
        return Err(StdError::msg("vk hash"));
    }
    if campaign.registration.is_none() {
        return Err(StdError::msg("registration"));
    }
    if campaign
        .amounts
        .iter()
        .any(|class| !DENOMS_UTERP.contains(&class.amount))
    {
        return Err(StdError::msg("amount is not a denomination"));
    }
    Ok(())
}

pub fn denom_amount(class: u8) -> StdResult<u64> {
    DENOMS_UTERP
        .get(class as usize)
        .copied()
        .ok_or_else(|| StdError::msg("denom class"))
}

/// `sha256("terp-hs-note-list-v2" || ctx || id_tag || identity || n u16_le ||
/// (class u8 || index u16_le || key_com 32)*)`.
pub fn note_list_hash(ctx: &[u8], id_tag: u8, identity: &[u8], list: &[NoteListEntry]) -> StdResult<[u8; 32]> {
    let mut pre = b"terp-hs-note-list-v2".to_vec();
    pre.extend_from_slice(ctx);
    pre.push(id_tag);
    pre.extend_from_slice(identity);
    pre.extend_from_slice(&(list.len() as u16).to_le_bytes());
    for (i, entry) in list.iter().enumerate() {
        if entry.key_com.len() != 32 {
            return Err(StdError::msg("key commitment"));
        }
        pre.push(entry.class);
        pre.extend_from_slice(&(i as u16).to_le_bytes());
        pre.extend_from_slice(entry.key_com.as_slice());
    }
    Ok(sha256(&pre))
}

/// Eligibility leaf, identical to `headstash_epoch::denominations::note_leaf`.
pub fn note_eligibility_leaf(id_tag: u8, identity: &[u8], class: u8, index: u16) -> [u8; 32] {
    let mut pre = b"terp-hs-note-elig-v2".to_vec();
    pre.push(id_tag);
    pre.extend_from_slice(identity);
    pre.push(class);
    pre.extend_from_slice(&index.to_le_bytes());
    sha256(&pre)
}

pub fn note_eligibility_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-note-node-v2".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

pub fn open_note_eligibility(leaf: [u8; 32], index: u32, path: &[[u8; 32]]) -> [u8; 32] {
    let mut node = leaf;
    for (level, sibling) in path.iter().enumerate() {
        node = if (index >> level) & 1 == 0 {
            note_eligibility_node(&node, sibling)
        } else {
            note_eligibility_node(sibling, &node)
        };
    }
    node
}

/// v3: the reward address (empty when none) is part of the signed string, so a
/// relayer cannot swap it. v2 is retired.
pub fn register_notes_message(ctx: &[u8], list_hash: &[u8], n: u32, reward_addr: &str) -> String {
    format!(
        "headstash-register-notes-v3:{}:{}:{n}:{reward_addr}",
        hex::encode(ctx),
        hex::encode(list_hash)
    )
}

pub fn register_notes_sol_message(ctx: &[u8], list_hash: &[u8], n: u32, reward_addr: &str) -> String {
    format!(
        "headstash-register-notes-sol-v3:{}:{}:{n}:{reward_addr}",
        hex::encode(ctx),
        hex::encode(list_hash)
    )
}

#[allow(clippy::too_many_arguments)]
/// Result of the read-only half of `RegisterNotes`.
pub(crate) struct NoteRegCheck {
    id_tag: u8,
    identity_bytes: Vec<u8>,
    identity: String,
    state: NoteListState,
    eligibility_root: Binary,
}

/// Every read-only check `RegisterNotes` performs (signature, N6, eligibility
/// openings, chunk order, already-registered). Shared by the execute and the
/// `CheckRegisterNotes` query the fee-grant sidecar uses before granting.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_register_notes(
    deps: Deps,
    campaign: &EpochCampaignParams,
    signer: &NoteSigner,
    signature: Option<&Binary>,
    list: &[NoteListEntry],
    start: u32,
    eligibility: &[NoteEligibility],
    reward_addr: Option<&str>,
) -> StdResult<NoteRegCheck> {
    if let Some(r) = reward_addr {
        check_reward_addr(deps.api, campaign, r)?;
    }
    if campaign.vk_id != NOTE_VK_ID {
        return Err(StdError::msg("not a note campaign"));
    }
    let cfg = registration_cfg(campaign)?;
    if cfg.eligibility_depth as usize != NOTE_DEPTH {
        return Err(StdError::msg("eligibility"));
    }
    let n = list.len();
    if n == 0 || n > MAX_NOTES_PER_IDENTITY {
        return Err(StdError::msg("note list"));
    }
    let count = eligibility.len() as u32;
    if count == 0 || count > MAX_CHUNK || start as usize + count as usize > n {
        return Err(StdError::msg("chunk"));
    }
    let ctx = campaign.ctx.as_slice();
    let (id_tag, identity_bytes, identity) = match signer {
        NoteSigner::Evm { address } => {
            let a: [u8; 20] = address
                .as_slice()
                .try_into()
                .map_err(|_| StdError::msg("address"))?;
            (EVM_ID_TAG, a.to_vec(), crate::registration::evm_identity(&a))
        }
        NoteSigner::Sol { pubkey } => {
            let p: [u8; 32] = pubkey
                .as_slice()
                .try_into()
                .map_err(|_| StdError::msg("pubkey"))?;
            (SOL_ID_TAG, p.to_vec(), crate::registration::sol_identity(&p))
        }
    };
    let list_hash = note_list_hash(ctx, id_tag, &identity_bytes, list)?;
    let stored = NOTE_LIST.may_load(deps.storage, identity.as_str())?;
    let state = match stored {
        None => {
            if start != 0 {
                return Err(StdError::msg("chunk"));
            }
            if crate::registration::is_registered(deps.storage, &identity)? {
                return Err(StdError::msg("already registered"));
            }
            // N6: no repeated key_com inside the list, none already in the tree.
            let mut seen = HashSet::new();
            for entry in list {
                if !seen.insert(entry.key_com.to_vec()) {
                    return Err(StdError::msg("duplicate key commitment"));
                }
                parse_fp(entry.key_com.as_slice())?;
                denom_amount(entry.class)?;
                if key_com_used(deps.storage, entry.key_com.as_slice())? {
                    return Err(StdError::msg("key commitment"));
                }
            }
            let sig = signature.ok_or_else(|| StdError::msg("signature"))?;
            verify_signer(deps.api, signer, ctx, &list_hash, n as u32, reward_addr.unwrap_or(""), sig.as_slice())?;
            NoteListState {
                list_hash: Binary::from(list_hash.to_vec()),
                n: n as u32,
                next: 0,
                reward_addr: reward_addr.map(str::to_string),
            }
        }
        Some(state) => {
            if state.list_hash.as_slice() != list_hash || state.n != n as u32 {
                return Err(StdError::msg("list hash"));
            }
            if state.next >= state.n {
                return Err(StdError::msg("already registered"));
            }
            if start != state.next {
                return Err(StdError::msg("chunk"));
            }
            if reward_addr.is_some() && reward_addr != state.reward_addr.as_deref() {
                return Err(StdError::msg("reward_addr"));
            }
            state
        }
    };
    for (offset, elig) in eligibility.iter().enumerate() {
        let i = start + offset as u32;
        let entry = &list[i as usize];
        let amount = denom_amount(entry.class)?;
        precheck(deps.storage, amount)?;
        let path = path_bytes(&elig.path)?;
        if path.len() != NOTE_DEPTH || elig.index >= 1u32 << NOTE_DEPTH {
            return Err(StdError::msg("eligibility"));
        }
        let leaf = note_eligibility_leaf(id_tag, &identity_bytes, entry.class, i as u16);
        if open_note_eligibility(leaf, elig.index, &path).as_slice() != cfg.eligibility_root.as_slice() {
            return Err(StdError::msg("eligibility"));
        }
        if NOTE_REG
            .may_load(deps.storage, (identity.as_str(), i as u16))?
            .is_some()
        {
            return Err(StdError::msg("already registered"));
        }
        parse_fp(entry.key_com.as_slice())?;
    }
    Ok(NoteRegCheck { id_tag, identity_bytes, identity, state, eligibility_root: cfg.eligibility_root.clone() })
}

pub fn register_notes(
    deps: DepsMut,
    campaign: &EpochCampaignParams,
    signer: NoteSigner,
    signature: Option<Binary>,
    list: Vec<NoteListEntry>,
    start: u32,
    eligibility: Vec<NoteEligibility>,
    reward_addr: Option<String>,
) -> Result<Response, StdError> {
    let NoteRegCheck { id_tag, identity_bytes, identity, mut state, eligibility_root } =
        check_register_notes(deps.as_ref(), campaign, &signer, signature.as_ref(), &list, start, &eligibility, reward_addr.as_deref())?;
    let count = eligibility.len() as u32;
    let mut first_leaf = None;
    for (offset, elig) in eligibility.iter().enumerate() {
        let i = start + offset as u32;
        let entry = &list[i as usize];
        let amount = denom_amount(entry.class)?;
        // Re-run per leaf: earlier appends in this chunk move the caps.
        precheck(deps.storage, amount)?;
        let path = path_bytes(&elig.path)?;
        let leaf = note_eligibility_leaf(id_tag, &identity_bytes, entry.class, i as u16);
        debug_assert_eq!(open_note_eligibility(leaf, elig.index, &path).as_slice(), eligibility_root.as_slice());
        let key_fp = parse_fp(entry.key_com.as_slice())?;
        let (index, _) = append_claim_leaf(
            deps.storage,
            &entry.key_com,
            note_registration_leaf(key_fp, amount),
            amount,
        )?;
        NOTE_REG.save(deps.storage, (identity.as_str(), i as u16), &index)?;
        first_leaf.get_or_insert(index);
    }
    state.next = start + count;
    NOTE_LIST.save(deps.storage, identity.as_str(), &state)?;
    let done = state.next == state.n;
    let mut res = Response::new()
        .add_attribute("action", "register_notes")
        .add_attribute("start", start.to_string())
        .add_attribute("count", count.to_string())
        .add_attribute("first_leaf", first_leaf.unwrap_or(0).to_string())
        .add_attribute("done", done.to_string());
    // Fires once per identity: a completed list rejects every later chunk.
    if done {
        if let Some(hook) = REG_HOOK.may_load(deps.storage)? {
            let msg = RegistrationHookMsg::HeadstashRegistered {
                campaign: hex::encode(campaign.ctx.as_slice()),
                id_tag,
                identity: hex::encode(&identity_bytes),
                note_count: state.n,
                reward_addr: state.reward_addr.clone(),
            };
            let exec = WasmMsg::Execute { contract_addr: hook.contract.to_string(), msg: to_json_binary(&msg)?, funds: vec![] };
            res = res
                .add_attribute("registration_hook", hook.contract.as_str())
                .add_submessage(SubMsg::reply_on_error(exec, HOOK_REPLY_ID).with_gas_limit(hook.gas_limit));
        }
    }
    Ok(res)
}

fn verify_signer(
    api: &dyn cosmwasm_std::Api,
    signer: &NoteSigner,
    ctx: &[u8],
    list_hash: &[u8; 32],
    n: u32,
    reward_addr: &str,
    signature: &[u8],
) -> StdResult<()> {
    match signer {
        NoteSigner::Evm { address } => {
            let message = register_notes_message(ctx, list_hash, n, reward_addr);
            let got = crate::registration::recover_address(api, &message, signature)?;
            if got.as_slice() != address.as_slice() {
                return Err(StdError::msg("signature"));
            }
        }
        NoteSigner::Sol { pubkey } => {
            if signature.len() != 64 {
                return Err(StdError::msg("signature"));
            }
            let message = register_notes_sol_message(ctx, list_hash, n, reward_addr);
            let ok = api
                .ed25519_verify(message.as_bytes(), signature, pubkey.as_slice())
                .map_err(|_| StdError::msg("signature"))?;
            if !ok {
                return Err(StdError::msg("signature"));
            }
        }
    }
    Ok(())
}

pub fn query_note_list(storage: &dyn Storage, identity: &str) -> StdResult<Option<NoteListState>> {
    NOTE_LIST.may_load(storage, identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::AmountCap;
    use crate::msg::{ExecuteMsg, InstantiateMsg};
    use crate::registration::{recover_address, RegistrationConfig};
    use crate::tokenfactory::{derive_nd, DenomUnit, HeadstashTokenObject, Metadata, NewTokenConfig};
    use crate::wavs::generate_test_wavs_proof;
    use crate::{execute, instantiate};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{Env, OwnedDeps};
    use ff::PrimeField;
    use k256::ecdsa::SigningKey;
    use pasta_curves::pallas;
    use zk_headstash::distro_poseidon::registration_key_commitment;

    type TDeps = OwnedDeps<MockStorage, MockApi, MockQuerier>;

    fn setup() -> (TDeps, Env) {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: crate::tokenfactory::TokenStrategy::NewFungible(Box::new(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(&cosmwasm_std::testing::MockApi::default(), "cht".into()),
                metadata: Metadata {
                    description: None,
                    denom_units: vec![DenomUnit { denom: "ucht".into(), exponent: 0, aliases: vec![] }],
                    base: Some("ucht".into()),
                    display: Some("CHT".into()),
                    name: Some("CHT".into()),
                    symbol: Some("CHT".into()),
                },
                initial_mint: None,
                manager: None,
                minters: vec![],
            })),
            wavs: None, // WAVS disabled (mainnet config)
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg).unwrap();
        (deps, env)
    }

    fn ctx() -> [u8; 32] {
        // Bound to the mock chain + contract so the same fixture drives direct claims.
        let env = mock_env();
        crate::direct::direct_ctx(&env.block.chain_id, env.contract.address.as_str())
    }

    fn campaign(root: [u8; 32], vk_id: u64) -> EpochCampaignParams {
        EpochCampaignParams {
            ctx: Binary::from(ctx().to_vec()),
            backend: "halo2".into(),
            cutoff_height: 1_000_000, // above mock_env height: registration stays open
            bech32_hrp: "cosmwasm".into(),
            denom: derive_nd(&cosmwasm_std::testing::MockApi::default(), "cht"),
            amounts: DENOMS_UTERP.iter().map(|amount| AmountCap { amount: *amount, cap: 50 }).collect(),
            reserved: vec![],
            trusted: vec![Binary::from(vec![8u8; 32])],
            trusted_verify_threshold: 1,
            trusted_sign_threshold: 1,
            max_rows: 64,
            allocation_root: Binary::from(vec![0u8; 32]),
            root_id: 0,
            vk_id,
            vk_hash: Binary::from(NOTE_VK_HASH.to_vec()),
            sweep_recipient: String::new(),
            burn_unclaimed: None,
            registration: Some(RegistrationConfig {
                eligibility_root: Binary::from(root.to_vec()),
                eligibility_depth: NOTE_DEPTH as u32,
                min_anonymity_set: 1,
                anchor_start_height: 1,
                anchor_interval: 1,
            }),
        }
    }

    fn set(deps: &mut TDeps, env: &Env, c: EpochCampaignParams) -> Result<Response, StdError> {
        let creator = deps.api.addr_make("creator");
        execute(deps.as_mut(), env.clone(), message_info(&creator, &[]), ExecuteMsg::SetEpochCampaign { campaign: c })
    }

    /// Sparse depth-17 SHA tree over `leaves` (zero padding).
    struct Elig {
        levels: Vec<Vec<[u8; 32]>>,
        zeros: Vec<[u8; 32]>,
    }

    impl Elig {
        fn new(leaves: Vec<[u8; 32]>) -> Self {
            let mut zeros = vec![[0u8; 32]];
            for l in 0..NOTE_DEPTH {
                let z = zeros[l];
                zeros.push(note_eligibility_node(&z, &z));
            }
            let mut levels = vec![leaves];
            for l in 0..NOTE_DEPTH {
                let cur = &levels[l];
                let mut next = Vec::new();
                for pair in cur.chunks(2) {
                    let right = if pair.len() == 2 { pair[1] } else { zeros[l] };
                    next.push(note_eligibility_node(&pair[0], &right));
                }
                levels.push(next);
            }
            Self { levels, zeros }
        }
        fn root(&self) -> [u8; 32] {
            self.levels[NOTE_DEPTH][0]
        }
        fn path(&self, index: usize) -> Vec<Binary> {
            (0..NOTE_DEPTH)
                .map(|l| {
                    let j = (index >> l) ^ 1;
                    let node = self.levels[l].get(j).copied().unwrap_or(self.zeros[l]);
                    Binary::from(node.to_vec())
                })
                .collect()
        }
    }

    struct Holder {
        key: SigningKey,
        address: [u8; 20],
        classes: Vec<u8>,
        first: usize,
        word: u64,
        reward: String,
    }

    fn sign_with(key: &SigningKey, message: &str) -> Vec<u8> {
        let mut pre = b"\x19Ethereum Signed Message:\n".to_vec();
        pre.extend_from_slice(message.len().to_string().as_bytes());
        pre.extend_from_slice(message.as_bytes());
        let digest = zk_headstash::claim_auth::keccak256(&pre);
        let (signature, recid) = key.sign_prehash_recoverable(&digest);
        let mut bytes = signature.to_bytes().as_slice().to_vec();
        bytes.push(recid.to_byte());
        bytes
    }

    fn holder(byte: u8, classes: Vec<u8>, first: usize, word: u64) -> Holder {
        let mut secret = [0u8; 32];
        secret[31] = byte;
        let key = SigningKey::from_slice(&secret).unwrap();
        let sig = sign_with(&key, "probe");
        let deps = mock_dependencies();
        let address = recover_address(deps.as_ref().api, "probe", &sig).unwrap();
        let reward = deps.api.addr_make(&format!("reward{byte}")).to_string();
        Holder { key, address, classes, first, word, reward }
    }

    impl Holder {
        fn leaves(&self) -> Vec<[u8; 32]> {
            self.classes
                .iter()
                .enumerate()
                .map(|(i, c)| note_eligibility_leaf(EVM_ID_TAG, &self.address, *c, i as u16))
                .collect()
        }
        fn list(&self) -> Vec<NoteListEntry> {
            self.classes
                .iter()
                .enumerate()
                .map(|(i, c)| NoteListEntry {
                    class: *c,
                    key_com: Binary::from(
                        registration_key_commitment(pallas::Base::from(self.word + i as u64)).to_repr().to_vec(),
                    ),
                })
                .collect()
        }
        fn signature(&self, list: &[NoteListEntry]) -> Binary {
            let h = note_list_hash(&ctx(), EVM_ID_TAG, &self.address, list).unwrap();
            let msg = register_notes_message(&ctx(), &h, list.len() as u32, &self.reward);
            Binary::from(sign_with(&self.key, &msg))
        }
        fn chunk(&self, tree: &Elig, list: &[NoteListEntry], start: usize, len: usize, sig: bool) -> ExecuteMsg {
            ExecuteMsg::RegisterNotes {
                signer: NoteSigner::Evm { address: Binary::from(self.address.to_vec()) },
                signature: sig.then(|| self.signature(list)),
                list: list.to_vec(),
                start: start as u32,
                eligibility: (start..start + len)
                    .map(|i| NoteEligibility { index: (self.first + i) as u32, path: tree.path(self.first + i) })
                    .collect(),
                reward_addr: Some(self.reward.clone()),
            }
        }
    }

    fn run(deps: &mut TDeps, env: &Env, msg: ExecuteMsg) -> Result<Response, StdError> {
        let relayer = deps.api.addr_make("relayer");
        execute(deps.as_mut(), env.clone(), message_info(&relayer, &[]), msg)
    }

    fn fixture() -> (TDeps, Env, Holder, Holder, Elig) {
        let a = holder(1, (0..40).map(|i| (i % 12) as u8).collect(), 0, 1000);
        let b = holder(2, vec![11, 3, 0], 40, 2000);
        let mut leaves = a.leaves();
        leaves.extend(b.leaves());
        let tree = Elig::new(leaves);
        let (mut deps, env) = setup();
        set(&mut deps, &env, campaign(tree.root(), NOTE_VK_ID)).unwrap();
        (deps, env, a, b, tree)
    }

    fn err(r: Result<Response, StdError>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn registers_a_list_across_chunks_and_rejects_replays() {
        let (mut deps, env, a, b, tree) = fixture();
        let list = a.list();
        let r = run(&mut deps, &env, a.chunk(&tree, &list, 0, 32, true)).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "done" && x.value == "false"));
        assert!(err(run(&mut deps, &env, a.chunk(&tree, &list, 33, 7, false))).contains("chunk"));
        let mut forged = list.clone();
        forged[39].key_com =
            Binary::from(registration_key_commitment(pallas::Base::from(9999u64)).to_repr().to_vec());
        assert!(err(run(&mut deps, &env, a.chunk(&tree, &forged, 32, 8, false))).contains("list hash"));
        let r = run(&mut deps, &env, a.chunk(&tree, &list, 32, 8, false)).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "done" && x.value == "true"));
        assert_eq!(crate::registration::leaf_count(deps.as_ref().storage).unwrap(), 40);
        assert!(err(run(&mut deps, &env, a.chunk(&tree, &list, 0, 1, true))).contains("already registered"));
        run(&mut deps, &env, b.chunk(&tree, &b.list(), 0, 3, true)).unwrap();
        assert_eq!(crate::registration::tree_depth(deps.as_ref().storage).unwrap(), NOTE_DEPTH);
        // claim tree leaf 40 is note_leaf(key_com, DENOMS[11]) under the depth-17 frontier
        let mut env2 = env.clone();
        env2.block.height += 5;
        run(&mut deps, &env2, ExecuteMsg::AcceptAnchor {}).unwrap();
        let tree_state = crate::registration::query_tree(
            deps.as_ref().storage,
            campaign([0u8; 32], NOTE_VK_ID).registration.as_ref().unwrap(),
        )
        .unwrap();
        let anchor = tree_state.latest_anchor.unwrap().root.clone();
        let p = crate::registration::query_path(deps.as_ref().storage, b.list()[0].key_com.as_slice(), &anchor)
            .unwrap();
        assert_eq!(p.index, 40);
        assert_eq!(p.amount, DENOMS_UTERP[11]);
        assert_eq!(p.siblings.len(), NOTE_DEPTH);
        let key = parse_fp(b.list()[0].key_com.as_slice()).unwrap();
        assert_eq!(p.leaf, hex::encode(note_registration_leaf(key, DENOMS_UTERP[11]).to_repr()));
        let mut node = note_registration_leaf(key, DENOMS_UTERP[11]);
        for (l, s) in p.siblings.iter().enumerate() {
            let sib = parse_fp(&hex::decode(s).unwrap()).unwrap();
            node = if (p.index >> l) & 1 == 0 {
                zk_headstash::distro_poseidon::poseidon_distro_crh(l as u32, node, sib)
            } else {
                zk_headstash::distro_poseidon::poseidon_distro_crh(l as u32, sib, node)
            };
        }
        assert_eq!(node.to_repr().as_slice(), anchor.as_slice());
    }

    fn hook_msgs(r: &Response) -> Vec<(u64, u64, RegistrationHookMsg)> {
        r.messages
            .iter()
            .map(|m| match &m.msg {
                cosmwasm_std::CosmosMsg::Wasm(WasmMsg::Execute { msg, funds, .. }) => {
                    assert!(funds.is_empty());
                    assert_eq!(m.reply_on, cosmwasm_std::ReplyOn::Error);
                    (m.id, m.gas_limit.unwrap(), cosmwasm_std::from_json(msg).unwrap())
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn hook_fires_once_on_completion_with_reward_addr() {
        let (mut deps, env, a, b, tree) = fixture();
        let hook = deps.api.addr_make("hook");
        set_registration_hook(&mut deps.storage, &deps.api, Some(hook.to_string()), None).unwrap();
        let list = a.list();
        let r = run(&mut deps, &env, a.chunk(&tree, &list, 0, 32, true)).unwrap();
        assert!(r.messages.is_empty(), "no hook before completion");
        let r = run(&mut deps, &env, a.chunk(&tree, &list, 32, 8, false)).unwrap();
        let m = hook_msgs(&r);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, HOOK_REPLY_ID);
        assert_eq!(m[0].1, DEFAULT_HOOK_GAS);
        assert_eq!(
            m[0].2,
            RegistrationHookMsg::HeadstashRegistered {
                campaign: hex::encode(ctx()),
                id_tag: EVM_ID_TAG,
                identity: hex::encode(a.address),
                note_count: 40,
                reward_addr: Some(a.reward.clone()),
            }
        );
        // a completed identity can never re-trigger the hook
        assert!(run(&mut deps, &env, a.chunk(&tree, &list, 0, 1, true)).is_err());
        let r = run(&mut deps, &env, b.chunk(&tree, &b.list(), 0, 3, true)).unwrap();
        assert_eq!(hook_msgs(&r).len(), 1);
    }

    #[test]
    fn no_hook_means_no_submsg() {
        let (mut deps, env, _a, b, tree) = fixture();
        let r = run(&mut deps, &env, b.chunk(&tree, &b.list(), 0, 3, true)).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "done" && x.value == "true"));
        assert!(r.messages.is_empty());
        assert_eq!(query_registration_hook(&deps.storage).unwrap(), None);
    }

    #[test]
    fn failing_hook_reply_is_swallowed() {
        let (mut deps, env, _a, b, tree) = fixture();
        set_registration_hook(&mut deps.storage, &deps.api, Some(deps.api.addr_make("hook").to_string()), Some(123_456)).unwrap();
        let r = run(&mut deps, &env, b.chunk(&tree, &b.list(), 0, 3, true)).unwrap();
        assert_eq!(hook_msgs(&r)[0].1, 123_456);
        #[allow(deprecated)]
        let reply = cosmwasm_std::Reply {
            id: HOOK_REPLY_ID,
            payload: Binary::default(),
            gas_used: 0,
            result: cosmwasm_std::SubMsgResult::Err("hook exploded".into()),
        };
        let r = crate::reply(deps.as_mut(), env.clone(), reply).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "action" && x.value == "registration_hook_failed"));
        assert!(r.attributes.iter().any(|x| x.key == "hook_error" && x.value.contains("hook exploded")));
        // registration state persisted
        assert_eq!(crate::registration::leaf_count(deps.as_ref().storage).unwrap(), 3);
    }

    #[test]
    fn hook_config_is_validated() {
        let mut deps = mock_dependencies();
        let api = deps.api;
        assert!(set_registration_hook(&mut deps.storage, &api, Some("not an addr".into()), None).is_err());
        assert!(set_registration_hook(&mut deps.storage, &api, Some(api.addr_make("h").to_string()), Some(0)).is_err());
        assert!(set_registration_hook(&mut deps.storage, &api, Some(api.addr_make("h").to_string()), Some(MAX_HOOK_GAS + 1)).is_err());
        assert!(set_registration_hook(&mut deps.storage, &api, None, Some(1)).is_err());
        set_registration_hook(&mut deps.storage, &api, None, None).unwrap();
        assert_eq!(query_registration_hook(&deps.storage).unwrap(), None);
    }

    #[test]
    fn reward_addr_is_signed_and_validated() {
        let (mut deps, env, a, b, tree) = fixture();
        let list = b.list();
        // relayer swaps the reward address: signature no longer matches
        let mut m = b.chunk(&tree, &list, 0, 3, true);
        if let ExecuteMsg::RegisterNotes { reward_addr, .. } = &mut m {
            *reward_addr = Some(deps.api.addr_make("thief").to_string());
        }
        assert!(run(&mut deps, &env, m).is_err());
        // dropping it also breaks the signature
        let mut m = b.chunk(&tree, &list, 0, 3, true);
        if let ExecuteMsg::RegisterNotes { reward_addr, .. } = &mut m {
            *reward_addr = None;
        }
        assert!(run(&mut deps, &env, m).is_err());
        // malformed / wrong prefix
        for bad in ["cosmwasm1notbech32", "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw", ""] {
            let mut m = b.chunk(&tree, &list, 0, 3, true);
            if let ExecuteMsg::RegisterNotes { reward_addr, .. } = &mut m {
                *reward_addr = Some(bad.into());
            }
            assert!(err(run(&mut deps, &env, m)).contains("reward_addr"), "{bad}");
        }
        // later chunks must not change it
        let la = a.list();
        run(&mut deps, &env, a.chunk(&tree, &la, 0, 32, true)).unwrap();
        let mut m = a.chunk(&tree, &la, 32, 8, false);
        if let ExecuteMsg::RegisterNotes { reward_addr, .. } = &mut m {
            *reward_addr = Some(b.reward.clone());
        }
        assert!(err(run(&mut deps, &env, m)).contains("reward_addr"));
        let mut m = a.chunk(&tree, &la, 32, 8, false);
        if let ExecuteMsg::RegisterNotes { reward_addr, .. } = &mut m {
            *reward_addr = None;
        }
        run(&mut deps, &env, m).unwrap();
        run(&mut deps, &env, b.chunk(&tree, &list, 0, 3, true)).unwrap();
    }

    #[test]
    fn n6_duplicate_key_com_inside_a_list_and_across_identities() {
        let (mut deps, env, a, b, tree) = fixture();
        let mut list = b.list();
        list[2].key_com = list[0].key_com.clone();
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &list, 0, 3, true))).contains("duplicate key commitment"));
        run(&mut deps, &env, a.chunk(&tree, &a.list(), 0, 1, true)).unwrap();
        let mut stolen = b.list();
        stolen[1].key_com = a.list()[0].key_com.clone();
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &stolen, 0, 3, true))).contains("key commitment"));
    }

    #[test]
    fn n7_wrong_class_bad_signature_and_oversized_chunk() {
        let (mut deps, env, _a, b, tree) = fixture();
        let mut list = b.list();
        list[1].class = 10;
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &list, 0, 3, true))).contains("eligibility"));
        // mock storage does not roll back a failed tx: start clean for each case
        let (mut deps, env, _a, b, tree) = fixture();
        let mut list = b.list();
        list[1].class = 12;
        let e = err(run(&mut deps, &env, b.chunk(&tree, &list, 0, 3, true)));
        assert!(e.contains("denom class"), "{e}");
        let (mut deps, env, _a, b, tree) = fixture();
        let other = holder(9, b.classes.clone(), 40, 2000);
        let mut msg = b.chunk(&tree, &b.list(), 0, 3, true);
        if let ExecuteMsg::RegisterNotes { signature, .. } = &mut msg {
            *signature = Some(other.signature(&b.list()));
        }
        assert!(err(run(&mut deps, &env, msg)).contains("signature"));
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &b.list(), 0, 3, false))).contains("signature"));
        let (mut deps, env, a, _b, tree) = fixture();
        assert!(err(run(&mut deps, &env, a.chunk(&tree, &a.list(), 0, 33, true))).contains("chunk"));
    }

    #[test]
    fn n1_vk2_is_retired_and_vk3_is_pinned() {
        let (mut deps, env) = setup();
        let mut c = campaign([1u8; 32], RETIRED_VK_ID);
        assert!(err(set(&mut deps, &env, c.clone())).contains("retired"));
        c.vk_id = NOTE_VK_ID;
        c.vk_hash = Binary::from(vec![7u8; 32]);
        assert!(err(set(&mut deps, &env, c.clone())).contains("vk hash"));
        c.vk_hash = Binary::from(NOTE_VK_HASH.to_vec());
        c.amounts.push(AmountCap { amount: 3_000_000, cap: 1 });
        assert!(err(set(&mut deps, &env, c.clone())).contains("denomination"));
        c.amounts.pop();
        set(&mut deps, &env, c).unwrap();
    }

    #[test]
    fn legacy_register_is_refused_on_a_note_campaign() {
        let (mut deps, env, _a, _b, _tree) = fixture();
        let e = run(
            &mut deps,
            &env,
            ExecuteMsg::RegisterHeadstash {
                key_com: Binary::from(vec![0u8; 32]),
                amount: 1_000_000,
                signature: Binary::from(vec![0u8; 65]),
                eligibility_index: 0,
                eligibility_path: vec![],
            },
        );
        assert!(err(e).contains("register_notes"));
    }

    #[test]
    fn vk_hash_matches_circuit_constant() {
        assert_eq!(
            hex::encode(NOTE_VK_HASH),
            "34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398"
        );
    }

    // ---- direct on-chain claims (crate::direct) ----
    use crate::direct::{self, test_hook};

    /// Serve circuit 3 to the bind check. Without zk-api the contract reads the
    /// test hook; with zk-api the real path queries the chain, so the mock querier
    /// serves the real vk_id 3 blob (or a tampered copy) and its CircuitInfo.
    fn serve_circuit(deps: &mut TDeps, genuine: bool) {
        #[cfg(not(feature = "zk-api"))]
        {
            let _ = &deps;
            test_hook::set(Some(if genuine { NOTE_VK_HASH } else { [9u8; 32] }));
        }
        #[cfg(feature = "zk-api")]
        real_zk::serve(deps, genuine);
    }

    /// Make the next proofs invalid. With zk-api the proof bytes are really corrupted
    /// and rejected by the real halo2 verifier registered for zk_id 3.
    fn bad_proofs(bad: bool) {
        #[cfg(not(feature = "zk-api"))]
        test_hook::set_ok(!bad);
        #[cfg(feature = "zk-api")]
        real_zk::BAD.with(|c| c.set(bad));
    }

    #[cfg(feature = "zk-api")]
    mod real_zk {
        use super::*;
        use cosmwasm_std::{to_json_binary, Addr, CircuitInfoResponse, CircuitResponse, ContractResult, SystemError, SystemResult, WasmQuery};
        use std::cell::{Cell, RefCell};
        use std::collections::HashMap;
        use zk_headstash::note_membership as nm;

        thread_local!(pub static BAD: Cell<bool> = const { Cell::new(false) });
        thread_local!(static BLOB: Vec<u8> = nm::note_circuit_blob().unwrap());
        thread_local!(static NOTES: RefCell<HashMap<u8, usize>> = RefCell::new(HashMap::new()));
        thread_local!(static PROOFS: RefCell<HashMap<(u8, Vec<u8>, String, u64, bool), (Vec<u8>, Vec<u8>)>> = RefCell::new(HashMap::new()));

        pub fn serve(deps: &mut TDeps, genuine: bool) {
            let mut data = BLOB.with(|b| b.clone());
            if !genuine {
                let last = data.len() - 1;
                data[last] ^= 1;
            }
            deps.querier.update_wasm(move |q| match q {
                WasmQuery::Circuit { zk_id: 3 } => {
                    SystemResult::Ok(ContractResult::Ok(to_json_binary(&CircuitResponse::new(Binary::from(data.clone()))).unwrap()))
                }
                WasmQuery::CircuitInfo { zk_id: 3 } => SystemResult::Ok(ContractResult::Ok(
                    to_json_binary(&CircuitInfoResponse::new(3, Addr::unchecked("circuit-creator"), Binary::from(vec![0xABu8; 72]))).unwrap(),
                )),
                WasmQuery::Circuit { zk_id } | WasmQuery::CircuitInfo { zk_id } => SystemResult::Err(SystemError::NoSuchCircuit { zk_id: *zk_id }),
                WasmQuery::ContractInfo { .. } => super::contract_info(),
                _ => SystemResult::Err(SystemError::UnsupportedRequest { kind: "wasm".into() }),
            });
        }

        /// Real halo2 verification of a note proof against the vk_id 3 verifying key.
        pub fn register_verifier() {
            cosmwasm_std::testing::register_test_circuit(
                3,
                "headstash-note-membership-vk3",
                Box::new(|proof: &[u8], inst: &[u8]| {
                    if inst.len() != 6 * 32 {
                        return Ok(false);
                    }
                    let mut public = Vec::new();
                    for c in inst.chunks(32) {
                        let mut r = [0u8; 32];
                        r.copy_from_slice(c);
                        match Option::<pallas::Base>::from(pallas::Base::from_repr(r)) {
                            Some(f) => public.push(f),
                            None => return Ok(false),
                        }
                    }
                    let params = nm::note_params();
                    let vk = nm::note_vk(&params).unwrap();
                    Ok(zk_headstash::no_ecdsa::verify_membership(&params, &vk, proof, &public).is_ok())
                }),
            );
            NOTES.with(|m| m.borrow_mut().clear());
            PROOFS.with(|m| m.borrow_mut().clear());
            BAD.with(|c| c.set(false));
        }

        /// Note `nf` is the holder note bound to that label on first proof (first unused
        /// index of the amount's class). Returns the real nullifier and a real proof
        /// when the root is an accepted anchor holding that note; otherwise checks fail
        /// before verification and a placeholder proof is sent.
        pub fn witness(deps: &TDeps, h: &Holder, who: &str, root: &Binary, nf: u8, amount: u64) -> (Vec<u8>, Vec<u8>) {
            let bad = BAD.with(|c| c.get());
            let key = (nf, root.to_vec(), who.to_string(), amount, bad);
            if let Some(v) = PROOFS.with(|m| m.borrow().get(&key).cloned()) {
                return v;
            }
            let class = DENOMS_UTERP.iter().position(|d| *d == amount);
            let idx = NOTES.with(|m| {
                let mut m = m.borrow_mut();
                if let Some(i) = m.get(&nf) {
                    return Some(*i);
                }
                let class = class?;
                let used: Vec<usize> = m.values().copied().collect();
                (0..h.classes.len()).find(|i| h.classes[*i] as usize == class && !used.contains(i))
            });
            let mut fallback = [0u8; 32];
            fallback[0] = nf;
            let Some(idx) = idx else { return (fallback.to_vec(), vec![1u8; 64]) };
            let secret = pallas::Base::from(h.word + idx as u64);
            let c = pallas::Base::from_repr(ctx()).unwrap();
            let nullifier = zk_headstash::no_ecdsa::claim_nullifier(secret, c).to_repr().to_vec();
            let key_com = registration_key_commitment(secret).to_repr();
            let dest = if who.starts_with("hsfwd1") { who.to_string() } else { format!("terp1{who}") };
            let path = crate::registration::query_path(deps.as_ref().storage, &key_com, root.as_slice());
            let out = match path {
                Ok(p) if class == Some(h.classes[idx] as usize) => {
                    // label bound to this note once it is first proved against an anchor
                    NOTES.with(|m| {
                        m.borrow_mut().entry(nf).or_insert(idx);
                    });
                    let sib: Vec<pallas::Base> = p
                        .siblings
                        .iter()
                        .map(|x| {
                            let mut r = [0u8; 32];
                            r.copy_from_slice(&hex::decode(x).unwrap());
                            pallas::Base::from_repr(r).unwrap()
                        })
                        .collect();
                    let path: [pallas::Base; NOTE_DEPTH] = sib.try_into().unwrap();
                    let proved = nm::prove_note(secret, c, amount, p.index, path, &dest).unwrap();
                    assert_eq!(proved.public[2].to_repr().to_vec(), nullifier);
                    assert_eq!(proved.public[1].to_repr().as_slice(), root.as_slice());
                    let mut proof = proved.proof;
                    if bad {
                        proof[100] ^= 1;
                    }
                    (nullifier, proof)
                }
                _ => (nullifier, vec![1u8; 64]),
            };
            PROOFS.with(|m| m.borrow_mut().insert(key, out.clone()));
            out
        }
    }

    /// Nullifier and proof for note label `nf` claimed by `who`.
    fn witness(deps: &TDeps, h: &Holder, who: &str, root: &Binary, nf: u8, amount: u64) -> (Binary, Binary) {
        #[cfg(feature = "zk-api")]
        {
            let (n, p) = real_zk::witness(deps, h, who, root, nf, amount);
            (Binary::from(n), Binary::from(p))
        }
        #[cfg(not(feature = "zk-api"))]
        {
            let _ = (deps, h, who, root, amount);
            let mut n = [0u8; 32];
            n[0] = nf;
            (Binary::from(n.to_vec()), Binary::from(vec![1u8; 64]))
        }
    }

    fn direct_fixture() -> (TDeps, Env, Holder, Holder, Elig) {
        let a = holder(1, (0..40).map(|i| (i % 12) as u8).collect(), 0, 1000);
        let b = holder(2, vec![11, 3, 0], 40, 2000);
        let mut leaves = a.leaves();
        leaves.extend(b.leaves());
        let tree = Elig::new(leaves);
        let (mut deps, env) = setup();
        let mut c = campaign(tree.root(), NOTE_VK_ID);
        c.cutoff_height = env.block.height + 1000;
        c.amounts = DENOMS_UTERP.iter().map(|amount| AmountCap { amount: *amount, cap: if *amount == DENOMS_UTERP[11] { 1 } else { 50 } }).collect();
        set(&mut deps, &env, c).unwrap();
        #[cfg(feature = "zk-api")]
        real_zk::register_verifier();
        serve_circuit(&mut deps, true);
        bad_proofs(false);
        (deps, env, a, b, tree)
    }

    fn owner_run(deps: &mut TDeps, env: &Env, msg: ExecuteMsg) -> Result<Response, StdError> {
        let creator = deps.api.addr_make("creator");
        execute(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg)
    }

    fn register_all(deps: &mut TDeps, env: &Env, h: &Holder, tree: &Elig) {
        let list = h.list();
        let mut start = 0;
        while start < list.len() {
            let len = core::cmp::min(32, list.len() - start);
            run(deps, env, h.chunk(tree, &list, start, len, start == 0)).unwrap();
            start += len;
        }
    }

    fn tree_root(deps: &TDeps) -> (Binary, Vec<crate::registration::AcceptedAnchor>) {
        let c = crate::epoch::query_campaign(deps.as_ref().storage).unwrap();
        let t = crate::registration::query_tree(deps.as_ref().storage, c.registration.as_ref().unwrap()).unwrap();
        let a = crate::registration::query_anchors(deps.as_ref().storage, None, Some(100)).unwrap().anchors;
        (t.root, a.into_iter().map(|e| crate::registration::AcceptedAnchor { root: e.root, leaf_count: e.leaf_count }).collect())
    }

    fn claim(deps: &mut TDeps, env: &Env, h: &Holder, who: &str, root: &Binary, nf: u8, amount: u64) -> Result<Response, StdError> {
        let sender = cosmwasm_std::Addr::unchecked(format!("terp1{who}"));
        let (nullifier, proof) = witness(deps, h, who, root, nf, amount);
        execute(deps.as_mut(), env.clone(), message_info(&sender, &[]), ExecuteMsg::ClaimNote {
            root: root.clone(),
            nullifier,
            amount,
            proof,
        })
    }

    #[test]
    fn direct_instances_match_circuit_public_inputs() {
        let secret = pallas::Base::from(77u64);
        let c = pallas::Base::from_repr(ctx()).unwrap();
        let root = pallas::Base::from(99u64);
        let dest = "terp1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5lzv7xu";
        let public = zk_headstash::note_membership::note_public(secret, c, DENOMS_UTERP[4], root, dest).unwrap();
        let want: Vec<u8> = public.iter().flat_map(|f| f.to_repr().to_vec()).collect();
        let got = direct::instances(&ctx(), &root.to_repr(), &public[2].to_repr(), dest, DENOMS_UTERP[4]).unwrap();
        assert_eq!(got, want);
        // a different sender gives different dest limbs, so a copied proof cannot verify
        let other = direct::instances(&ctx(), &root.to_repr(), &public[2].to_repr(), "terp1other", DENOMS_UTERP[4]).unwrap();
        assert_ne!(other[96..160], want[96..160]);
    }

    #[test]
    fn direct_bind_pins_circuit_once_and_checks_ctx_and_hash() {
        let (mut deps, env, ..) = direct_fixture();
        let stranger = deps.api.addr_make("stranger");
        assert!(execute(deps.as_mut(), env.clone(), message_info(&stranger, &[]), ExecuteMsg::BindNoteCircuit { zk_id: 3 }).is_err());
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 0 })).contains("zk id"));
        serve_circuit(&mut deps, false);
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 })).contains("vk hash"));
        serve_circuit(&mut deps, true);
        // another chain id => ctx not bound
        let mut env2 = env.clone();
        env2.block.chain_id = "other-chain".into();
        assert!(err(owner_run(&mut deps, &env2, ExecuteMsg::BindNoteCircuit { zk_id: 3 })).contains("ctx"));
        owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 }).unwrap();
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 4 })).contains("fixed"));
        let info = direct::query_info(deps.as_ref().storage, &env).unwrap();
        assert_eq!(info.cfg.unwrap().zk_id, 3);
        assert_eq!(info.expected_ctx.as_slice(), ctx());
    }

    #[test]
    fn direct_claims_against_first_and_latest_of_2100_anchors() {
        let (mut deps, mut env, a, b, tree) = direct_fixture();
        register_all(&mut deps, &env, &a, &tree);
        // the whole register -> anchor -> claim flow below runs with WAVS disabled
        assert!(crate::wavs_operator_set(&deps.storage).unwrap_err().to_string().contains("wavs disabled"));
        owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 }).unwrap();
        run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
        let first = tree_root(&deps).1[0].root.clone();
        // 2,100 more accepted anchors (synthetic roots; past the old 128 KiB Vec limit of ~1,735)
        for i in 0..2100u32 {
            let mut r = [0u8; 32];
            r[..4].copy_from_slice(&(i + 1).to_be_bytes());
            r[30] = 0xAB;
            crate::registration::test_push_anchor(deps.as_mut().storage, &Binary::from(r.to_vec()), 40);
        }
        register_all(&mut deps, &env, &b, &tree);
        env.block.height += 1;
        run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
        let c = crate::epoch::query_campaign(deps.as_ref().storage).unwrap();
        let t = crate::registration::query_tree(deps.as_ref().storage, c.registration.as_ref().unwrap()).unwrap();
        assert_eq!(t.anchor_count, 2102);
        let latest = t.latest_anchor.unwrap();
        assert!(latest.leaf_count > 40);
        let page = crate::registration::query_anchors(deps.as_ref().storage, Some(2000), Some(500)).unwrap().anchors;
        assert_eq!((page.len(), page[0].seq, page.last().unwrap().seq), (100, 2001, 2100));
        claim(&mut deps, &env, &a, "w1", &first, 1, DENOMS_UTERP[0]).unwrap();
        claim(&mut deps, &env, &a, "w2", &latest.root, 2, DENOMS_UTERP[0]).unwrap();
        assert!(err(claim(&mut deps, &env, &a, "w3", &latest.root, 1, DENOMS_UTERP[0])).contains("nullifier"));
        assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 2);
    }

    #[test]
    fn direct_claim_roots_amounts_replay_window_and_payout() {
        let (mut deps, mut env, a, b, tree) = direct_fixture();
        register_all(&mut deps, &env, &a, &tree);
        let (live, _) = tree_root(&deps);
        assert!(err(claim(&mut deps, &env, &a, "w1", &live, 1, DENOMS_UTERP[0])).contains("not enabled"));
        owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 }).unwrap();
        // live root, nothing sealed yet
        assert!(err(claim(&mut deps, &env, &a, "w1", &live, 1, DENOMS_UTERP[0])).contains("anchor"));
        run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
        let (_, anchors) = tree_root(&deps);
        let epoch_n = anchors[0].root.clone();
        assert_eq!(anchors[0].leaf_count, 40);
        // epoch N+1 registration moves the live root; it is not claimable until sealed
        register_all(&mut deps, &env, &b, &tree);
        let (live2, _) = tree_root(&deps);
        assert_ne!(live2, epoch_n);
        assert!(err(claim(&mut deps, &env, &a, "w2", &live2, 2, DENOMS_UTERP[0])).contains("anchor"));
        // fabricated root
        let mut fake = [7u8; 32];
        fake[31] = 0;
        assert!(err(claim(&mut deps, &env, &a, "w2", &Binary::from(fake.to_vec()), 2, DENOMS_UTERP[0])).contains("anchor"));
        // non-canonical limbs
        let mut nc = epoch_n.to_vec();
        nc[31] |= 0x80;
        assert!(err(claim(&mut deps, &env, &a, "w2", &Binary::from(nc), 2, DENOMS_UTERP[0])).contains("root"));
        // amounts
        assert!(err(claim(&mut deps, &env, &a, "w2", &epoch_n, 2, 3_000_000)).contains("denomination"));
        // ok: payout to sender, exactly amount
        // CheckNoteClaim dry-run: same verdict as the execute, writes nothing.
        let dry = |deps: &TDeps, who: &str, nf: u8| {
            let (n, proof) = witness(deps, &a, who, &epoch_n, nf, DENOMS_UTERP[3]);
            let campaign = crate::epoch::query_campaign(deps.as_ref().storage).unwrap();
            direct::check_claim(deps.as_ref(), &env, &campaign, &format!("terp1{who}"), &epoch_n, &n, DENOMS_UTERP[3], &proof).map(|_| ())
        };
        dry(&deps, "w1", 1).unwrap();
        dry(&deps, "w1", 1).unwrap();
        let r = claim(&mut deps, &env, &a, "w1", &epoch_n, 1, DENOMS_UTERP[3]).unwrap();
        assert!(dry(&deps, "w1", 1).unwrap_err().to_string().contains("nullifier"));
        let w1 = "terp1w1".to_string();
        let sent = r.messages.iter().any(|m| matches!(&m.msg, cosmwasm_std::CosmosMsg::Bank(cosmwasm_std::BankMsg::Send { to_address, amount }) if *to_address == w1 && amount[0].amount.to_string() == DENOMS_UTERP[3].to_string()));
        assert!(sent);
        // replay of the nullifier from any sender, against any anchor
        assert!(err(claim(&mut deps, &env, &a, "w1", &epoch_n, 1, DENOMS_UTERP[3])).contains("nullifier"));
        // seal N+1; the older anchor still works
        env.block.height += 1;
        run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
        let (_, anchors) = tree_root(&deps);
        assert_eq!(anchors.len(), 2);
        assert!(err(claim(&mut deps, &env, &a, "w9", &anchors[1].root, 1, DENOMS_UTERP[0])).contains("nullifier"));
        claim(&mut deps, &env, &a, "w2", &epoch_n, 2, DENOMS_UTERP[0]).unwrap();
        claim(&mut deps, &env, &a, "w3", &anchors[1].root, 3, DENOMS_UTERP[0]).unwrap();
        // per-denomination cap
        claim(&mut deps, &env, &a, "w4", &epoch_n, 4, DENOMS_UTERP[11]).unwrap();
        assert!(err(claim(&mut deps, &env, &a, "w5", &epoch_n, 5, DENOMS_UTERP[11])).contains("cap"));
        // failed verification consumes nothing
        bad_proofs(true);
        assert!(err(claim(&mut deps, &env, &a, "w6", &epoch_n, 6, DENOMS_UTERP[0])).contains("invalid note proof"));
        bad_proofs(false);
        claim(&mut deps, &env, &a, "w6", &epoch_n, 6, DENOMS_UTERP[0]).unwrap();
        // window
        env.block.height += 2000;
        assert!(err(claim(&mut deps, &env, &a, "w7", &epoch_n, 7, DENOMS_UTERP[0])).contains("window"));
        assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 5);
    }


    // ---- emergency reclaim (crate::reclaim, spec Appendix R) ----
    use cosmwasm_std::{BankMsg, Coin, ContractResult, CosmosMsg, DistributionMsg, SystemError, SystemResult, WasmQuery};
    use std::cell::RefCell;

    thread_local!(static ADMIN: RefCell<Option<String>> = const { RefCell::new(None) });
    const ESCROW: u128 = 10_000_000_000;

    fn contract_info() -> SystemResult<ContractResult<Binary>> {
        let admin = ADMIN.with(|a| a.borrow().clone());
        let v = serde_json::json!({"code_id": 1, "creator": "creator", "admin": admin, "pinned": false, "ibc_port": null});
        SystemResult::Ok(ContractResult::Ok(Binary::from(serde_json::to_vec(&v).unwrap())))
    }

    /// Wasm admin as reported by the chain (x/wasm ContractInfo).
    fn set_admin(deps: &mut TDeps, admin: Option<String>) {
        ADMIN.with(|a| *a.borrow_mut() = admin);
        #[cfg(not(feature = "zk-api"))]
        deps.querier.update_wasm(|q| match q {
            WasmQuery::ContractInfo { .. } => contract_info(),
            _ => SystemResult::Err(SystemError::UnsupportedRequest { kind: "wasm".into() }),
        });
        #[cfg(feature = "zk-api")]
        serve_circuit(deps, true);
    }

    /// Existing-denom (uterp) escrow, owner = admin = "creator" (stands in for x/gov),
    /// registered, circuit bound and one anchor sealed: claims are live.
    fn reclaim_fixture() -> (TDeps, Env, Holder, Binary) {
        tf_fixture(Tok::Uterp)
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Tok { Uterp, FactoryExisting, Mint }
    const FDENOM: &str = "factory/terp1funder/hstest";
    const SUB: &str = "hstest";

    fn tf_fixture(tok: Tok) -> (TDeps, Env, Holder, Binary) {
        let a = holder(1, (0..40).map(|i| (i % 12) as u8).collect(), 0, 1000);
        let tree = Elig::new(a.leaves());
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let api = cosmwasm_std::testing::MockApi::default();
        let (ts, nd_raw) = match tok {
            Tok::Uterp => {
                deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(ESCROW, "uterp")]);
                (crate::tokenfactory::TokenStrategy::ExistingFungible(HeadstashTokenObject::new(&api, "uterp".into())), "uterp")
            }
            Tok::FactoryExisting => {
                deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(ESCROW, FDENOM)]);
                (crate::tokenfactory::TokenStrategy::ExistingFungible(HeadstashTokenObject::new(&api, FDENOM.into())), FDENOM)
            }
            Tok::Mint => (
                crate::tokenfactory::TokenStrategy::NewFungible(Box::new(NewTokenConfig {
                    subdenom: HeadstashTokenObject::new(&api, SUB.into()),
                    metadata: Metadata { description: None, denom_units: vec![DenomUnit { denom: SUB.into(), exponent: 0, aliases: vec![] }], base: None, display: None, name: None, symbol: None },
                    initial_mint: None,
                    manager: None,
                    minters: vec![],
                })),
                SUB,
            ),
        };
        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: ts,
            wavs: None,
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg).unwrap();
        let mut c = campaign(tree.root(), NOTE_VK_ID);
        c.denom = derive_nd(&cosmwasm_std::testing::MockApi::default(), nd_raw);
        c.cutoff_height = env.block.height + 1000;
        c.burn_unclaimed = Some(true);
        if tok == Tok::Mint {
            // AmountCap is the only bound on total mint: keep one class tight.
            for ac in c.amounts.iter_mut() { if ac.amount == DENOMS_UTERP[0] { ac.cap = 2; } }
        }
        set(&mut deps, &env, c).unwrap();
        #[cfg(feature = "zk-api")]
        real_zk::register_verifier();
        serve_circuit(&mut deps, true);
        bad_proofs(false);
        set_admin(&mut deps, Some(creator.to_string()));
        register_all(&mut deps, &env, &a, &tree);
        owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 }).unwrap();
        run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
        let (_, anchors) = tree_root(&deps);
        (deps, env, a, anchors[0].root.clone())
    }

    /// Apply the bank effect of a response to the mock balance (the mock does not).
    fn settle(deps: &mut TDeps, env: &Env, r: &Response) -> u128 {
        let mut out = 0u128;
        for m in &r.messages {
            match &m.msg {
                CosmosMsg::Bank(BankMsg::Send { amount, .. }) | CosmosMsg::Bank(BankMsg::Burn { amount }) => out += amount[0].amount.to_string().parse::<u128>().unwrap(),
                CosmosMsg::Distribution(DistributionMsg::FundCommunityPool { amount }) => out += amount[0].amount.to_string().parse::<u128>().unwrap(),
                _ => {}
            }
        }
        let bal = deps.as_ref().querier.query_balance(env.contract.address.as_str(), "uterp").unwrap().amount.to_string().parse::<u128>().unwrap();
        deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(bal - out, "uterp")]);
        out
    }

    fn reclaim_as(deps: &mut TDeps, env: &Env, who: &str) -> Result<Response, StdError> {
        let sender = deps.api.addr_make(who);
        execute(deps.as_mut(), env.clone(), message_info(&sender, &[]), ExecuteMsg::EmergencyReclaim {})
    }

    fn pool_amount(r: &Response) -> Option<Vec<Coin>> {
        r.messages.iter().find_map(|m| match &m.msg {
            CosmosMsg::Distribution(DistributionMsg::FundCommunityPool { amount }) => Some(amount.clone()),
            _ => None,
        })
    }

    fn status(deps: &TDeps) -> Option<crate::reclaim::ReclaimRecord> {
        cosmwasm_std::from_json(crate::query(deps.as_ref(), mock_env(), crate::msg::QueryMsg::ReclaimStatus {}).unwrap()).unwrap()
    }

    fn mdenom(env: &Env) -> String { format!("factory/{}/{}", env.contract.address, SUB) }

    /// (minted amount, sent amount, sent denom) of a claim response.
    fn mint_and_send(r: &Response) -> (u128, u128, String, bool) {
        let mut sent = (0u128, String::new());
        let mut minted = false;
        for m in &r.messages {
            match &m.msg {
                CosmosMsg::Bank(BankMsg::Send { amount, .. }) => sent = (amount[0].amount.to_string().parse::<u128>().unwrap(), amount[0].denom.clone()),
                #[allow(deprecated)]
                CosmosMsg::Stargate { .. } | CosmosMsg::Any(_) => minted = true,
                _ => {}
            }
        }
        (if minted { sent.0 } else { 0 }, sent.0, sent.1, minted)
    }

    #[test]
    fn tf_mint_1_claim_mints_then_sends_minted_denom() {
        let (mut deps, env, a, root) = tf_fixture(Tok::Mint);
        let r = claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3]).unwrap();
        let (minted, sent, denom, has_mint) = mint_and_send(&r);
        assert!(has_mint);
        assert_eq!((minted, sent), (DENOMS_UTERP[3] as u128, DENOMS_UTERP[3] as u128));
        assert_eq!(denom, mdenom(&env));
        assert_eq!(r.messages.len(), 2);
    }

    #[test]
    fn tf_mint_2_replay_refused() {
        let (mut deps, env, a, root) = tf_fixture(Tok::Mint);
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3]).unwrap();
        assert!(err(claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3])).contains("nullifier already exists"));
        assert!(err(claim(&mut deps, &env, &a, "w9", &root, 1, DENOMS_UTERP[3])).len() > 0);
    }

    #[test]
    fn tf_mint_3_amount_cap_bounds_mint() {
        let (mut deps, env, a, root) = tf_fixture(Tok::Mint);
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
        claim(&mut deps, &env, &a, "w2", &root, 2, DENOMS_UTERP[0]).unwrap();
        assert!(err(claim(&mut deps, &env, &a, "w3", &root, 3, DENOMS_UTERP[0])).contains("denomination cap reached"));
    }

    #[test]
    fn tf_mint_4_sweep_refused_before_and_after_cutoff_even_with_balance() {
        let (mut deps, mut env, _, _) = tf_fixture(Tok::Mint);
        let d = mdenom(&env);
        deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(777u128, d)]);
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains(crate::epoch::SWEEP_MINT_REFUSED));
        env.block.height += 1001;
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains(crate::epoch::SWEEP_MINT_REFUSED));
        // still non-owner gated first
        let relayer = deps.api.addr_make("relayer");
        assert!(execute(deps.as_mut(), env.clone(), message_info(&relayer, &[]), ExecuteMsg::SweepUnclaimed {}).is_err());
    }

    #[test]
    fn tf_mint_5_reclaim_moves_nothing_but_halts_forever() {
        let (mut deps, env, a, root) = tf_fixture(Tok::Mint);
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
        // stray minted coins in the contract are not moved either
        let d = mdenom(&env);
        deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(5u128, d)]);
        assert!(reclaim_as(&mut deps, &env, "relayer").is_err());
        assert!(status(&deps).is_none());
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        assert!(r.messages.is_empty());
        let st = status(&deps).unwrap();
        assert_eq!((st.height, st.amounts.len()), (env.block.height, 0));
        assert!(err(claim(&mut deps, &env, &a, "w2", &root, 2, DENOMS_UTERP[0])).contains("reclaimed"));
        assert!(err(run(&mut deps, &env, ExecuteMsg::AcceptAnchor {})).contains("reclaimed"));
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains("reclaimed"));
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("reclaimed"));
    }

    #[test]
    fn tf_mint_6_claims_refused_after_cutoff_reclaim_too() {
        let (mut deps, mut env, a, root) = tf_fixture(Tok::Mint);
        env.block.height += 1000; // == cutoff: still open
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
        env.block.height += 1;
        assert!(err(claim(&mut deps, &env, &a, "w2", &root, 2, DENOMS_UTERP[0])).contains("window"));
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("campaign has ended"));
    }

    #[test]
    fn tf_existing_factory_denom_claim_sweep_reclaim_unchanged() {
        let (mut deps, mut env, a, root) = tf_fixture(Tok::FactoryExisting);
        let r = claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3]).unwrap();
        let (_, sent, denom, has_mint) = mint_and_send(&r);
        assert!(!has_mint);
        assert_eq!((sent, denom.as_str()), (DENOMS_UTERP[3] as u128, FDENOM));
        assert!(err(claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3])).contains("nullifier already exists"));
        // reclaim before cutoff returns the factory denom escrow
        let (mut d2, e2, _, _) = tf_fixture(Tok::FactoryExisting);
        let rr = reclaim_as(&mut d2, &e2, "creator").unwrap();
        assert_eq!(pool_amount(&rr).unwrap(), vec![Coin::new(ESCROW, FDENOM)]);
        // sweep after cutoff burns what the mock reports
        deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(ESCROW - sent, FDENOM)]);
        env.block.height += 1001;
        let s = owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {}).unwrap();
        assert!(s.messages.iter().any(|m| matches!(&m.msg, CosmosMsg::Bank(BankMsg::Burn { amount }) if amount == &vec![Coin::new(ESCROW - sent, FDENOM)])));
    }

    #[test]
    fn reclaim_1_gov_before_cutoff_returns_all_to_pool_and_halts() {
        let (mut deps, env, a, root) = reclaim_fixture();
        assert!(status(&deps).is_none());
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        assert_eq!(pool_amount(&r).unwrap(), vec![Coin::new(ESCROW, "uterp")]);
        assert_eq!(r.messages.len(), 1);
        assert!(!r.messages.iter().any(|m| matches!(m.msg, CosmosMsg::Bank(BankMsg::Burn { .. }))));
        assert_eq!(settle(&mut deps, &env, &r), ESCROW);
        let st = status(&deps).unwrap();
        assert_eq!((st.height, st.amounts), (env.block.height, vec![Coin::new(ESCROW, "uterp")]));
        // halted: claims, registration, anchors, sweep, second reclaim, owner ops
        assert!(err(claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0])).contains("reclaimed"));
        assert!(err(run(&mut deps, &env, ExecuteMsg::AcceptAnchor {})).contains("reclaimed"));
        let b = holder(2, vec![11, 3, 0], 40, 2000);
        let tree = Elig::new(b.leaves());
        let list = b.list();
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &list, 0, list.len(), true))).contains("reclaimed"));
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains("reclaimed"));
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("reclaimed"));
        // dry-run query halts too
        let (n, proof) = witness(&deps, &a, "w1", &root, 1, DENOMS_UTERP[0]);
        let q = crate::query(deps.as_ref(), env.clone(), crate::msg::QueryMsg::CheckNoteClaim {
            dest: "terp1w1".into(), root: root.clone(), nullifier: Binary::from(n), amount: DENOMS_UTERP[0], proof: Binary::from(proof),
        });
        assert!(q.unwrap_err().to_string().contains("reclaimed"));
    }

    #[test]
    fn reclaim_2_non_gov_refused() {
        let (mut deps, env, _, _) = reclaim_fixture();
        // neither owner nor admin
        assert!(reclaim_as(&mut deps, &env, "relayer").is_err());
        assert!(reclaim_as(&mut deps, &env, "frontend-uploader").is_err());
        // owner but not the wasm admin (e.g. ownership moved but admin did not)
        let other = deps.api.addr_make("other");
        set_admin(&mut deps, Some(other.to_string()));
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("admin"));
        // wasm admin but not the owner
        assert!(err(reclaim_as(&mut deps, &env, "other")).to_lowercase().contains("owner"));
        // no admin at all
        set_admin(&mut deps, None);
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("admin"));
        assert!(status(&deps).is_none());
    }

    #[test]
    fn reclaim_3_after_cutoff_refused_boundary_allowed() {
        let (mut deps, mut env, _, _) = reclaim_fixture();
        let cutoff = env.block.height + 1000;
        env.block.height = cutoff + 1;
        assert!(err(reclaim_as(&mut deps, &env, "creator")).contains("campaign has ended"));
        assert!(status(&deps).is_none());
        // at the cutoff block claims are still open, so reclaim is too
        env.block.height = cutoff;
        assert!(pool_amount(&reclaim_as(&mut deps, &env, "creator").unwrap()).is_some());
    }

    #[test]
    fn reclaim_4_burn_before_cutoff_refused() {
        let (mut deps, mut env, _, _) = reclaim_fixture();
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains("has not ended"));
        env.block.height += 1000; // == cutoff
        assert!(err(owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {})).contains("has not ended"));
        // not owner, after cutoff
        env.block.height += 1;
        let relayer = deps.api.addr_make("relayer");
        let r = execute(deps.as_mut(), env.clone(), message_info(&relayer, &[]), ExecuteMsg::SweepUnclaimed {});
        assert!(r.is_err());
    }

    #[test]
    fn reclaim_5_burn_after_cutoff_works() {
        let (mut deps, mut env, a, root) = reclaim_fixture();
        let r = claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3]).unwrap();
        let paid = settle(&mut deps, &env, &r);
        env.block.height += 1001;
        let r = owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {}).unwrap();
        assert!(r.messages.iter().any(|m| matches!(&m.msg, CosmosMsg::Bank(BankMsg::Burn { amount }) if amount == &vec![Coin::new(ESCROW - paid, "uterp")])));
        assert!(pool_amount(&r).is_none());
        assert_eq!(settle(&mut deps, &env, &r), ESCROW - paid);
        // claims closed after the cutoff
        assert!(err(claim(&mut deps, &env, &a, "w2", &root, 2, DENOMS_UTERP[0])).contains("window"));
    }

    #[test]
    fn reclaim_6_claim_after_reclaim_refused() {
        let (mut deps, env, a, root) = reclaim_fixture();
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        settle(&mut deps, &env, &r);
        for (i, who) in ["w2", "w3"].iter().enumerate() {
            assert!(err(claim(&mut deps, &env, &a, who, &root, 2 + i as u8, DENOMS_UTERP[0])).contains("reclaimed"));
        }
        assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 1);
    }

    #[test]
    fn reclaim_7_partial_claims_then_reclaim_returns_exact_remainder() {
        let (mut deps, env, a, root) = reclaim_fixture();
        let mut paid = 0u128;
        for (i, amt) in [DENOMS_UTERP[0], DENOMS_UTERP[3], DENOMS_UTERP[5], DENOMS_UTERP[0]].iter().enumerate() {
            let r = claim(&mut deps, &env, &a, &format!("p{i}"), &root, 1 + i as u8, *amt).unwrap();
            let s = settle(&mut deps, &env, &r);
            assert_eq!(s, *amt as u128);
            paid += s;
        }
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        assert_eq!(pool_amount(&r).unwrap(), vec![Coin::new(ESCROW - paid, "uterp")]);
        settle(&mut deps, &env, &r);
        let left = deps.as_ref().querier.query_balance(env.contract.address.as_str(), "uterp").unwrap().amount.to_string().parse::<u128>().unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn reclaim_phase_a_without_campaign_and_empty_escrow() {
        // Phase A: instantiated, no campaign, nothing funded yet beyond the 1 uterp
        // instantiate deposit. Gov can still close it; an empty escrow still halts.
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        deps.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(1u128, "uterp")]);
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: crate::tokenfactory::TokenStrategy::ExistingFungible(HeadstashTokenObject::new(&cosmwasm_std::testing::MockApi::default(), "uterp".into())),
            wavs: None,
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        }).unwrap();
        set_admin(&mut deps, Some(creator.to_string()));
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        assert_eq!(pool_amount(&r).unwrap(), vec![Coin::new(1u128, "uterp")]);
        settle(&mut deps, &env, &r);
        assert!(status(&deps).is_some());
        // empty escrow: no message, still terminal
        let mut d2 = mock_dependencies();
        d2.querier.bank.update_balance(env.contract.address.as_str(), vec![Coin::new(1u128, "uterp")]);
        let c2 = d2.api.addr_make("creator");
        instantiate(d2.as_mut(), env.clone(), message_info(&c2, &[]), InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]), distro_hash_domain: Default::default(), genesis_label: None,
            token_strategy: crate::tokenfactory::TokenStrategy::ExistingFungible(HeadstashTokenObject::new(&cosmwasm_std::testing::MockApi::default(), "uterp".into())),
            wavs: None, circuit_id: None, registration_hook: None, registration_hook_gas_limit: None, frontend_contract: None, pauser: None,
        }).unwrap();
        d2.querier.bank.update_balance(env.contract.address.as_str(), vec![]);
        set_admin(&mut d2, Some(c2.to_string()));
        let r = reclaim_as(&mut d2, &env, "creator").unwrap();
        assert!(r.messages.is_empty());
        assert!(status(&d2).unwrap().amounts.is_empty());
        assert!(err(owner_run(&mut d2, &env, ExecuteMsg::SweepUnclaimed {})).contains("reclaimed"));
    }

    fn as_(deps: &mut TDeps, env: &Env, who: &str, m: ExecuteMsg) -> Result<Response, StdError> {
        let sender = deps.api.addr_make(who);
        execute(deps.as_mut(), env.clone(), message_info(&sender, &[]), m)
    }

    fn pstatus(deps: &TDeps) -> crate::pause::PauseStatusResponse {
        cosmwasm_std::from_json(crate::query(deps.as_ref(), mock_env(), crate::msg::QueryMsg::PauseStatus {}).unwrap()).unwrap()
    }

    fn pause_fixture() -> (TDeps, Env, Holder, Binary) {
        let (mut deps, env, a, root) = reclaim_fixture();
        let p = deps.api.addr_make("dao").to_string();
        as_(&mut deps, &env, "creator", ExecuteMsg::UpdatePauser { pauser: Some(p) }).unwrap();
        (deps, env, a, root)
    }

    #[test]
    fn pause_1_unauthorized_pause_resume_refused() {
        let (mut deps, env, _, _) = pause_fixture();
        assert!(err(as_(&mut deps, &env, "relayer", ExecuteMsg::Pause {})).contains("unauthorized"));
        assert!(!pstatus(&deps).paused);
        as_(&mut deps, &env, "dao", ExecuteMsg::Pause {}).unwrap();
        assert!(err(as_(&mut deps, &env, "relayer", ExecuteMsg::Resume {})).contains("unauthorized"));
        assert!(err(as_(&mut deps, &env, "dao", ExecuteMsg::Pause {})).contains("already paused"));
        assert!(pstatus(&deps).paused);
    }

    #[test]
    fn pause_2_blocks_claim_and_register_resume_restores() {
        let (mut deps, env, a, root) = pause_fixture();
        let r = as_(&mut deps, &env, "dao", ExecuteMsg::Pause {}).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "action" && x.value == "pause"));
        let st = pstatus(&deps);
        assert_eq!((st.paused, st.paused_at_height, st.paused_by), (true, Some(env.block.height), Some(deps.api.addr_make("dao"))));
        assert!(err(claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0])).contains("paused"));
        let b = holder(2, vec![11, 3, 0], 40, 2000);
        let tree = Elig::new(b.leaves());
        let list = b.list();
        assert!(err(run(&mut deps, &env, b.chunk(&tree, &list, 0, list.len(), true))).contains("paused"));
        let (n, proof) = witness(&deps, &a, "w1", &root, 1, DENOMS_UTERP[0]);
        let q = crate::query(deps.as_ref(), env.clone(), crate::msg::QueryMsg::CheckNoteClaim {
            dest: "terp1w1".into(), root: root.clone(), nullifier: Binary::from(n), amount: DENOMS_UTERP[0], proof: Binary::from(proof),
        });
        assert!(q.unwrap_err().to_string().contains("paused"));
        // v0.0.1: anchors are paused too (spec Appendix P / V)
        assert!(err(run(&mut deps, &env, ExecuteMsg::AcceptAnchor {})).contains("paused"));
        let r = as_(&mut deps, &env, "dao", ExecuteMsg::Resume {}).unwrap();
        assert!(r.attributes.iter().any(|x| x.key == "action" && x.value == "resume"));
        assert!(!pstatus(&deps).paused);
        assert!(err(as_(&mut deps, &env, "dao", ExecuteMsg::Resume {})).contains("not paused"));
        claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
        assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 1);
        // gov can pause/resume too
        as_(&mut deps, &env, "creator", ExecuteMsg::Pause {}).unwrap();
        as_(&mut deps, &env, "creator", ExecuteMsg::Resume {}).unwrap();
    }

    #[test]
    fn pause_3_pauser_has_no_other_power() {
        let (mut deps, mut env, _, _) = pause_fixture();
        assert!(as_(&mut deps, &env, "dao", ExecuteMsg::EmergencyReclaim {}).is_err());
        assert!(as_(&mut deps, &env, "dao", ExecuteMsg::UpdatePauser { pauser: None }).is_err());
        let other = deps.api.addr_make("x").to_string();
        assert!(as_(&mut deps, &env, "dao", ExecuteMsg::UpdatePauser { pauser: Some(other) }).is_err());
        assert!(as_(&mut deps, &env, "dao", ExecuteMsg::BindNoteCircuit { zk_id: 3 }).is_err());
        env.block.height += 1001;
        assert!(as_(&mut deps, &env, "dao", ExecuteMsg::SweepUnclaimed {}).is_err());
        assert!(status(&deps).is_none());
        assert_eq!(pstatus(&deps).pauser, Some(deps.api.addr_make("dao")));
    }

    #[test]
    fn pause_4_gov_rotates_and_clears_pauser() {
        let (mut deps, env, _, _) = pause_fixture();
        let n = deps.api.addr_make("dao2").to_string();
        as_(&mut deps, &env, "creator", ExecuteMsg::UpdatePauser { pauser: Some(n) }).unwrap();
        assert!(err(as_(&mut deps, &env, "dao", ExecuteMsg::Pause {})).contains("unauthorized"));
        as_(&mut deps, &env, "dao2", ExecuteMsg::Pause {}).unwrap();
        // clearing does not resume
        as_(&mut deps, &env, "creator", ExecuteMsg::UpdatePauser { pauser: None }).unwrap();
        let st = pstatus(&deps);
        assert!(st.pauser.is_none() && st.paused);
        assert!(err(as_(&mut deps, &env, "dao2", ExecuteMsg::Resume {})).contains("unauthorized"));
        as_(&mut deps, &env, "creator", ExecuteMsg::Resume {}).unwrap();
        assert!(as_(&mut deps, &env, "creator", ExecuteMsg::UpdatePauser { pauser: Some("".into()) }).is_err());
    }

    #[test]
    fn pause_5_reclaim_works_while_paused_and_wins() {
        let (mut deps, env, _, _) = pause_fixture();
        as_(&mut deps, &env, "dao", ExecuteMsg::Pause {}).unwrap();
        let r = reclaim_as(&mut deps, &env, "creator").unwrap();
        assert_eq!(pool_amount(&r).unwrap(), vec![Coin::new(ESCROW, "uterp")]);
        assert!(status(&deps).is_some());
        assert!(err(as_(&mut deps, &env, "dao", ExecuteMsg::Resume {})).contains("reclaimed"));
        assert!(err(as_(&mut deps, &env, "creator", ExecuteMsg::Resume {})).contains("reclaimed"));
    }

    #[test]
    fn pause_6_sweep_after_cutoff_works_while_paused() {
        let (mut deps, mut env, _, _) = pause_fixture();
        as_(&mut deps, &env, "dao", ExecuteMsg::Pause {}).unwrap();
        env.block.height += 1001;
        let r = owner_run(&mut deps, &env, ExecuteMsg::SweepUnclaimed {}).unwrap();
        assert!(r.messages.iter().any(|m| matches!(&m.msg, CosmosMsg::Bank(BankMsg::Burn { amount }) if amount == &vec![Coin::new(ESCROW, "uterp")])));
    }

    #[test]
    fn pause_7_instantiate_with_pauser_and_default_disabled() {
        let (deps, _, _, _) = reclaim_fixture();
        let st = pstatus(&deps);
        assert!(st.pauser.is_none() && !st.paused);
        let j = serde_json::json!({"genesis_root": "AAAA", "token_strategy": {"existing_fungible": {"denom": "uterp"}}});
        let _ = serde_json::from_value::<InstantiateMsg>(j); // pauser omitted still parses (serde default)
    }

    // ---- v0.0.1 surface (legacy feature OFF): removed variants and pause coverage ----

    fn snap(deps: &TDeps) -> Vec<(Vec<u8>, Vec<u8>)> {
        use cosmwasm_std::Storage;
        deps.storage.range(None, None, cosmwasm_std::Order::Ascending).collect()
    }

    fn removed_variants() -> Vec<(&'static str, serde_json::Value)> {
        use serde_json::json;
        let b = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        vec![
            ("process_headstash", json!({"process_headstash": {"claims": []}})),
            ("set_circuit_id", json!({"set_circuit_id": {"circuit_id": 3}})),
            ("register_eligibility_root", json!({"register_eligibility_root": {"root": b}})),
            ("mint", json!({"mint": {"to_address": "terp1x", "amount": "1"}})),
            ("burn", json!({"burn": {"from_address": "terp1x", "amount": "1"}})),
            ("update_reflection", json!({"update_reflection": {"pool_root": b, "spent_root": b, "burn_root": b, "source_height": 1, "tip_height": 9}})),
            ("set_reflection_snapshot", json!({"set_reflection_snapshot": {"snapshot": {"pool_root": b, "spent_root": b, "burn_root": b, "source_height": 1, "tip_height": 9, "confirmations_k": 6, "max_lc_lag": 64, "frozen": false}}})),
            ("register_asset", json!({"register_asset": {"asset_id": b, "local_denom": "uterp"}})),
            ("set_external_asset_registry", json!({"set_external_asset_registry": {"addr": null}})),
            ("set_bridge_cfg", json!({"set_bridge_cfg": {"cfg": {"dest_domain": b, "confirmations_k": 6, "max_lc_lag": 64, "lc_client_id": "x", "mock_verify": true}}})),
            ("bridge_mint_note", json!({"bridge_mint_note": {"proof": "", "claim": {"source_chain_tag": "x", "tacit_asset_id": b, "value_u64": 1, "nullifier": b, "dest_commitment": b, "dest_domain": b, "claim_id": b, "source_pool_root": b, "source_burn_root": b, "source_height": 1, "domain_binding": b, "unit_scale": 1, "pool_domain": b, "cm_public": b, "in_burn_set": true, "in_pool_root": true, "src_chain_id": b, "dst_chain_id": b, "lc_client_id": b, "burn_dest_commitment": b}}})),
            ("bridge_egress_burn", json!({"bridge_egress_burn": {"proof": "", "statement": {"asset_id": b, "value": 1, "cm_spent": b, "nullifier": b, "dest_commitment": b, "dest_kind": "transparent", "root": b, "rcm": b, "owner_binding": b}}})),
            ("set_denom_table", json!({"set_denom_table": {"amounts": [1]}})),
            ("assign_public_notes", json!({"assign_public_notes": {"nullifiers": [b]}})),
            ("stage_epoch_batch", json!({"stage_epoch_batch": {"digests": [], "rows": []}})),
            ("apply_accepted_batch", json!({"apply_accepted_batch": {"allocation_root": b, "root_id": 0, "rows": []}})),
            ("apply_public_batch", json!({"apply_public_batch": {"rows": []}})),
        ]
    }

    #[cfg(not(feature = "legacy"))]
    #[test]
    fn v001_removed_variants_error_and_move_no_funds() {
        let (mut deps, env, _, _) = reclaim_fixture();
        let before = snap(&deps);
        let list = removed_variants();
        assert_eq!(list.len(), 17);
        for (name, j) in list {
            // Compiled out of ExecuteMsg (keeps the execute entry under the optimizer's
            // function budget): must be rejected at parse time. Only
            // register_eligibility_root stays parseable (manifold builds it).
            let m: ExecuteMsg = match serde_json::from_value(j.clone()) {
                Err(e) => {
                    assert!(e.to_string().contains("unknown variant"), "{name}: {e}");
                    assert!(cosmwasm_std::from_json::<ExecuteMsg>(serde_json::to_vec(&j).unwrap()).is_err(), "{name}");
                    continue;
                }
                Ok(m) => {
                    assert_eq!(name, "register_eligibility_root");
                    m
                }
            };
            // as the owner (gov) and as a stranger, with funds attached
            for who in ["creator", "relayer"] {
                let s = deps.api.addr_make(who);
                let r = execute(deps.as_mut(), env.clone(), message_info(&s, &[Coin::new(5u128, "uterp")]), m.clone());
                assert!(err(r).contains(crate::DISABLED_IN_BUILD), "{name} as {who}");
            }
        }
        // sudo settlement is compiled out too
        let s = crate::sudo(deps.as_mut(), env.clone(), crate::epoch::HashMerchantSudoMsg {
            hash_merchant: crate::epoch::HashMerchantSudoPayload {
                chain_uid: "x".into(), algo: "accepted".into(), height: 1, root: "AA==".into(),
                attestation_count: 2, block_time: 1, oracle_sources: vec![],
            },
        });
        assert!(err(s).contains(crate::DISABLED_IN_BUILD));
        assert_eq!(snap(&deps), before, "no state (and so no funds) changed");
        assert_eq!(crate::direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 0);
    }

    #[test]
    fn v001_pause_covers_every_fund_or_registration_entry_point() {
        use serde_json::json;
        let (mut deps, env, _, _) = pause_fixture();
        as_(&mut deps, &env, "dao", ExecuteMsg::Pause {}).unwrap();
        let before = snap(&deps);
        let b = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let gated = vec![
            json!({"claim_note": {"root": b, "nullifier": b, "amount": 1, "proof": ""}}),
            json!({"register_headstash": {"key_com": b, "amount": 1, "signature": b, "eligibility_index": 0, "eligibility_path": []}}),
            json!({"register_headstash_sol": {"key_com": b, "amount": 1, "pubkey": b, "signature": b, "eligibility_index": 0, "eligibility_path": []}}),
            json!({"accept_anchor": {}}),
        ];
        for j in gated {
            let m: ExecuteMsg = serde_json::from_value(j.clone()).unwrap();
            for who in ["creator", "relayer", "dao"] {
                assert!(err(as_(&mut deps, &env, who, m.clone())).contains(crate::pause::PAUSED_ERR), "{j} as {who}");
            }
        }
        // RegisterNotes is refused with a real signed chunk in pause_2.
        assert_eq!(snap(&deps), before);
        // not gated: gov config, reclaim/sweep (pause_5/6), pause controls
        let c = as_(&mut deps, &env, "creator", ExecuteMsg::SetFrontendContract { addr: None });
        assert!(c.is_ok(), "{c:?}");
        let c = as_(&mut deps, &env, "creator", ExecuteMsg::BindNoteCircuit { zk_id: 3 });
        if let Err(e) = c { assert!(!e.to_string().contains("paused"), "{e}"); }
        as_(&mut deps, &env, "creator", ExecuteMsg::Resume {}).unwrap();
        // after resume, anchors roll again (no leaves yet: either ok or a non-pause error)
        if let Err(e) = as_(&mut deps, &env, "relayer", ExecuteMsg::AcceptAnchor {}) {
            assert!(!e.to_string().contains("paused"), "{e}");
        }
    }

    // ---- v2 forwarded claims (crate::forward, V2-SPEC §F) ----
    mod forward_v2 {
        use super::*;
        use crate::forward::{self as fw, ForwardTarget, HalSource};
        use crate::{query, msg::{MigrateMsg, QueryMsg}};
        use cosmwasm_std::{
            HexBinary, IbcAckCallbackMsg, IbcAcknowledgement, IbcEndpoint, IbcMsg, IbcPacket, IbcSourceCallbackMsg,
            IbcTimeout, IbcTimeoutCallbackMsg, MsgResponse, Order, Reply, SubMsgResponse, SubMsgResult, Timestamp,
        };

        const RCV: &str = "penumbra1testreceiveraddressxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

        fn target(deps: &TDeps, ch: &str, rcv: &str, fb: &str, memo: Option<&str>) -> ForwardTarget {
            ForwardTarget { channel_id: ch.into(), receiver: rcv.into(), memo: memo.map(Into::into), fallback: deps.api.addr_make(fb).to_string() }
        }

        /// Bound circuit, one accepted anchor, channel-0 allowed. Returns the anchor root.
        fn fwd_fixture() -> (TDeps, Env, Holder, Binary) {
            let (mut deps, env, a, _b, tree) = direct_fixture();
            register_all(&mut deps, &env, &a, &tree);
            owner_run(&mut deps, &env, ExecuteMsg::BindNoteCircuit { zk_id: 3 }).unwrap();
            run(&mut deps, &env, ExecuteMsg::AcceptAnchor {}).unwrap();
            let root = tree_root(&deps).1[0].root.clone();
            owner_run(&mut deps, &env, ExecuteMsg::SetForwardChannel { channel_id: "channel-0".into(), timeout_seconds: 600, enabled: true }).unwrap();
            (deps, env, a, root)
        }

        fn fclaim(deps: &mut TDeps, env: &Env, h: &Holder, root: &Binary, nf: u8, amount: u64, proved: &ForwardTarget, sent: ForwardTarget) -> Result<Response, StdError> {
            let dest = fw::forward_destination(&env.block.chain_id, env.contract.address.as_str(), proved);
            let (nullifier, proof) = witness(deps, h, &dest, root, nf, amount);
            let anyone = deps.api.addr_make("relayer-anyone");
            execute(deps.as_mut(), env.clone(), message_info(&anyone, &[]), ExecuteMsg::ClaimNoteForward { root: root.clone(), nullifier, amount, proof, forward: sent })
        }

        #[allow(deprecated)]
        fn ack_reply(deps: &mut TDeps, env: &Env, seq: u64) -> Result<Response, StdError> {
            let mut v = vec![0x08u8];
            let mut s = seq;
            loop {
                let b = (s & 0x7f) as u8;
                s >>= 7;
                if s == 0 { v.push(b); break; }
                v.push(b | 0x80);
            }
            crate::reply(deps.as_mut(), env.clone(), Reply {
                id: fw::FORWARD_REPLY_ID,
                payload: Binary::default(),
                gas_used: 0,
                result: SubMsgResult::Ok(SubMsgResponse { events: vec![], data: None, msg_responses: vec![MsgResponse {
                    type_url: "/ibc.applications.transfer.v1.MsgTransferResponse".into(), value: Binary::from(v) }] }),
            })
        }

        fn packet(ch: &str, seq: u64, port: &str) -> IbcPacket {
            IbcPacket::new(Binary::from(b"{}".to_vec()), IbcEndpoint { port_id: port.into(), channel_id: ch.into() },
                IbcEndpoint { port_id: "transfer".into(), channel_id: "channel-9".into() }, seq, IbcTimeout::with_timestamp(Timestamp::from_seconds(1)))
        }

        fn refund_of(r: &cosmwasm_std::IbcBasicResponse) -> Option<(String, Coin)> {
            r.messages.iter().find_map(|m| match &m.msg {
                CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => Some((to_address.clone(), amount[0].clone())),
                _ => None,
            })
        }

        #[test]
        fn forward_destination_shape_and_binding() {
            let (deps, env, ..) = fwd_fixture();
            let t = target(&deps, "channel-0", RCV, "fb1", None);
            let d = fw::forward_destination(&env.block.chain_id, env.contract.address.as_str(), &t);
            assert!(d.starts_with("hsfwd1") && d.len() == 49 && d.len() <= 62, "{d}");
            assert!(d[6..].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
            direct::pack_destination(&d).unwrap();
            // every field changes the destination
            let base = d.clone();
            for t2 in [target(&deps, "channel-1", RCV, "fb1", None), target(&deps, "channel-0", "penumbra1other", "fb1", None),
                       target(&deps, "channel-0", RCV, "fb2", None), target(&deps, "channel-0", RCV, "fb1", Some("{\"a\":1}"))] {
                assert_ne!(fw::forward_destination(&env.block.chain_id, env.contract.address.as_str(), &t2), base);
            }
            assert_ne!(fw::forward_destination("other-chain", env.contract.address.as_str(), &t), base);
            assert_ne!(fw::forward_destination(&env.block.chain_id, "terp1othercontract", &t), base);
            // query helper returns the same string
            let q: String = cosmwasm_std::from_json(query(deps.as_ref(), env.clone(), QueryMsg::ForwardDestination { forward: t }).unwrap()).unwrap();
            assert_eq!(q, base);
        }

        #[test]
        fn forward_claim_sends_ics20_with_callback_and_records_pending() {
            let (mut deps, env, a, root) = fwd_fixture();
            let t = target(&deps, "channel-0", RCV, "fb1", Some("{\"note\":\"hi\"}"));
            let r = fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[3], &t, t.clone()).unwrap();
            // mint-on-claim fixture: MsgMint to the contract precedes the transfer
            assert!(matches!(&r.messages[0].msg, CosmosMsg::Stargate { type_url, .. } if type_url.ends_with("MsgMint")));
            let sm = r.messages.iter().find(|m| matches!(m.msg, CosmosMsg::Ibc(_))).unwrap();
            assert!(r.messages.iter().all(|m| !matches!(&m.msg, CosmosMsg::Bank(BankMsg::Send { .. }))), "no local payout");
            assert_eq!((sm.id, sm.reply_on.clone()), (fw::FORWARD_REPLY_ID, cosmwasm_std::ReplyOn::Success));
            let CosmosMsg::Ibc(IbcMsg::Transfer { channel_id, to_address, amount, timeout, memo }) = &sm.msg else { panic!("not a transfer") };
            assert_eq!((channel_id.as_str(), to_address.as_str()), ("channel-0", RCV));
            assert_eq!(amount.amount.to_string(), DENOMS_UTERP[3].to_string());
            assert_eq!(timeout.timestamp().unwrap().seconds(), env.block.time.seconds() + 600);
            let m: serde_json::Value = serde_json::from_str(memo.as_deref().unwrap()).unwrap();
            assert_eq!(m["src_callback"]["address"], env.contract.address.as_str());
            assert_eq!(m["note"], "hi");
            assert!(m.get("dest_callback").is_none());
            // reply stores the pending record under (channel, sequence)
            ack_reply(&mut deps, &env, 300).unwrap();
            let p: Option<fw::PendingForward> = cosmwasm_std::from_json(query(deps.as_ref(), env.clone(),
                QueryMsg::PendingForward { channel_id: "channel-0".into(), sequence: 300 }).unwrap()).unwrap();
            let p = p.unwrap();
            assert_eq!((p.fallback.as_str(), p.receiver.as_str(), p.coin.amount.to_string()), (t.fallback.as_str(), RCV, DENOMS_UTERP[3].to_string()));
            assert!(!fw::INFLIGHT.exists(deps.as_ref().storage));
            // one spend per note: the same nullifier cannot be forwarded again, nor claimed directly
            let t2 = target(&deps, "channel-0", "penumbra1elsewhere", "fb9", None);
            assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[3], &t2, t2.clone())).contains("nullifier"));
            assert!(err(claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[3])).contains("nullifier"));
            assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 1);
        }

        #[test]
        fn address_reuse_is_not_enforced_on_chain_only_note_spend_is() {
            // V2-SPEC F.5: fresh fallback / unused receiver are frontend rules. The contract
            // accepts a repeated receiver and fallback for a different note.
            let (mut deps, env, a, root) = fwd_fixture();
            let t = target(&deps, "channel-0", RCV, "fb1", None);
            fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone()).unwrap();
            ack_reply(&mut deps, &env, 1).unwrap();
            fclaim(&mut deps, &env, &a, &root, 2, DENOMS_UTERP[0], &t, t.clone()).unwrap();
            ack_reply(&mut deps, &env, 2).unwrap();
            assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 2);
            assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone())).contains("nullifier"));
        }

        #[test]
        fn forward_rejects_unallowed_channel_and_bad_fields() {
            let (mut deps, env, a, root) = fwd_fixture();
            let stranger = deps.api.addr_make("stranger");
            assert!(execute(deps.as_mut(), env.clone(), message_info(&stranger, &[]),
                ExecuteMsg::SetForwardChannel { channel_id: "channel-7".into(), timeout_seconds: 600, enabled: true }).is_err());
            for (ch, to) in [("chan-1", 600), ("channel-", 600), ("channel-1", 59), ("channel-1", 7 * 86_400 + 1)] {
                assert!(owner_run(&mut deps, &env, ExecuteMsg::SetForwardChannel { channel_id: ch.into(), timeout_seconds: to, enabled: true }).is_err(), "{ch} {to}");
            }
            let t = target(&deps, "channel-7", RCV, "fb1", None);
            assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone())).contains("channel not allowed"));
            owner_run(&mut deps, &env, ExecuteMsg::SetForwardChannel { channel_id: "channel-0".into(), timeout_seconds: 600, enabled: false }).unwrap();
            let t = target(&deps, "channel-0", RCV, "fb1", None);
            assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone())).contains("channel not allowed"));
            owner_run(&mut deps, &env, ExecuteMsg::SetForwardChannel { channel_id: "channel-0".into(), timeout_seconds: 600, enabled: true }).unwrap();
            let long = "x".repeat(fw::MAX_MEMO + 1);
            let cases: Vec<(ForwardTarget, &str)> = vec![
                (target(&deps, "channel-0", "", "fb1", None), "receiver"),
                (target(&deps, "channel-0", &"p".repeat(fw::MAX_RECEIVER + 1), "fb1", None), "receiver"),
                (target(&deps, "channel-0", "pen umbra", "fb1", None), "receiver"),
                (target(&deps, "channel-0", RCV, "fb1", Some(&long)), "memo too long"),
                (target(&deps, "channel-0", RCV, "fb1", Some("not json")), "JSON object"),
                (target(&deps, "channel-0", RCV, "fb1", Some("[1]")), "JSON object"),
                (target(&deps, "channel-0", RCV, "fb1", Some("{\"src_callback\":{\"address\":\"x\"}}")), "callbacks"),
                (target(&deps, "channel-0", RCV, "fb1", Some("{\"dest_callback\":{\"address\":\"x\"}}")), "callbacks"),
                (ForwardTarget { fallback: "notanaddress".into(), ..target(&deps, "channel-0", RCV, "fb1", None) }, "fallback"),
                (ForwardTarget { fallback: env.contract.address.to_string(), ..target(&deps, "channel-0", RCV, "fb1", None) }, "fallback"),
            ];
            for (t, want) in cases {
                assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone())).contains(want), "{want}");
            }
            // nothing was spent by the rejected attempts
            assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 0);
            // paused: forward is gated like ClaimNote
            let c = deps.api.addr_make("creator");
            execute(deps.as_mut(), env.clone(), message_info(&c, &[]), ExecuteMsg::Pause {}).ok();
            if crate::pause::PAUSED.may_load(deps.as_ref().storage).unwrap().is_some() {
                let t = target(&deps, "channel-0", RCV, "fb1", None);
                assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone())).contains("paused"));
            }
        }

        /// Real halo2 proofs (zk-api): a proof made for one forward target does not verify
        /// for any other channel, receiver, memo or fallback.
        #[cfg(feature = "zk-api")]
        #[test]
        fn tampered_forward_destination_fails_real_proof() {
            let (mut deps, env, a, root) = fwd_fixture();
            owner_run(&mut deps, &env, ExecuteMsg::SetForwardChannel { channel_id: "channel-1".into(), timeout_seconds: 600, enabled: true }).unwrap();
            let t = target(&deps, "channel-0", RCV, "fb1", None);
            for bad in [target(&deps, "channel-1", RCV, "fb1", None), target(&deps, "channel-0", "penumbra1attacker", "fb1", None),
                        target(&deps, "channel-0", RCV, "attacker", None), target(&deps, "channel-0", RCV, "fb1", Some("{\"forward\":{}}"))] {
                assert!(err(fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, bad)).contains("invalid note proof"));
            }
            assert_eq!(direct::query_info(deps.as_ref().storage, &env).unwrap().claimed, 0);
            fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone()).unwrap();
        }

        #[test]
        fn callbacks_refund_timeout_and_ack_error_to_fallback() {
            let (mut deps, env, a, root) = fwd_fixture();
            let mut fbs = vec![];
            for (i, nf) in [1u8, 2, 3].into_iter().enumerate() {
                let t = target(&deps, "channel-0", &format!("penumbra1r{i}"), &format!("fb{i}"), None);
                fclaim(&mut deps, &env, &a, &root, nf, DENOMS_UTERP[2], &t, t.clone()).unwrap();
                ack_reply(&mut deps, &env, 10 + i as u64).unwrap();
                fbs.push(t.fallback);
            }
            let relayer = cosmwasm_std::Addr::unchecked("relayer");
            // timeout -> refund to the bound fallback, exactly the coin
            let r = crate::ibc_source_callback(deps.as_mut(), env.clone(),
                IbcSourceCallbackMsg::Timeout(IbcTimeoutCallbackMsg::new(packet("channel-0", 10, "transfer"), relayer.clone()))).unwrap();
            let (to, coin) = refund_of(&r).unwrap();
            assert_eq!((to.as_str(), coin.amount.to_string()), (fbs[0].as_str(), DENOMS_UTERP[2].to_string()));
            // a second callback for the same packet pays nothing
            let r = crate::ibc_source_callback(deps.as_mut(), env.clone(),
                IbcSourceCallbackMsg::Timeout(IbcTimeoutCallbackMsg::new(packet("channel-0", 10, "transfer"), relayer.clone()))).unwrap();
            assert!(refund_of(&r).is_none());
            // error ack -> refund
            let err_ack = IbcAcknowledgement::new(br#"{"error":"ABCI code: 1: error handling packet: see events for details"}"#.to_vec());
            let r = crate::ibc_source_callback(deps.as_mut(), env.clone(), IbcSourceCallbackMsg::Acknowledgement(
                IbcAckCallbackMsg::new(err_ack, packet("channel-0", 11, "transfer"), relayer.clone()))).unwrap();
            assert_eq!(refund_of(&r).unwrap().0, fbs[1]);
            // success ack -> no refund, pending cleared
            let ok_ack = IbcAcknowledgement::new(br#"{"result":"AQ=="}"#.to_vec());
            let r = crate::ibc_source_callback(deps.as_mut(), env.clone(), IbcSourceCallbackMsg::Acknowledgement(
                IbcAckCallbackMsg::new(ok_ack.clone(), packet("channel-0", 12, "transfer"), relayer.clone()))).unwrap();
            assert!(refund_of(&r).is_none());
            assert_eq!(fw::PENDING.range(deps.as_ref().storage, None, None, Order::Ascending).count(), 0);
            // unknown packet / other port: no-op
            for p in [packet("channel-0", 99, "transfer"), packet("channel-0", 12, "wasm.x")] {
                let r = crate::ibc_source_callback(deps.as_mut(), env.clone(), IbcSourceCallbackMsg::Timeout(IbcTimeoutCallbackMsg::new(p, relayer.clone()))).unwrap();
                assert!(refund_of(&r).is_none());
            }
        }

        #[test]
        fn resolve_forward_owner_only_after_grace_to_bound_fallback() {
            let (mut deps, mut env, a, root) = fwd_fixture();
            let t = target(&deps, "channel-0", RCV, "fb1", None);
            fclaim(&mut deps, &env, &a, &root, 1, DENOMS_UTERP[0], &t, t.clone()).unwrap();
            ack_reply(&mut deps, &env, 4).unwrap();
            let m = ExecuteMsg::ResolveForward { channel_id: "channel-0".into(), sequence: 4 };
            assert!(err(owner_run(&mut deps, &env, m.clone())).contains("grace"));
            env.block.time = env.block.time.plus_seconds(600 + fw::RESOLVE_GRACE + 1);
            let stranger = deps.api.addr_make("stranger");
            assert!(execute(deps.as_mut(), env.clone(), message_info(&stranger, &[]), m.clone()).is_err());
            let r = owner_run(&mut deps, &env, m.clone()).unwrap();
            assert!(r.messages.iter().any(|x| matches!(&x.msg, CosmosMsg::Bank(BankMsg::Send { to_address, .. }) if *to_address == t.fallback)));
            assert!(err(owner_run(&mut deps, &env, m)).contains("no pending"));
        }

        #[test]
        fn hal_source_validation_owner_only_and_query() {
            let (mut deps, env, ..) = fwd_fixture();
            let sha = Some(HexBinary::from(vec![7u8; 32]));
            let cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string();
            let ok = |u: &str| HalSource { uri: u.into(), sha256: sha.clone(), cid: None };
            for u in ["ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi/elig.json", "ar://abc_DEF-123", "https://x.example/e.json",
                      "ipns://k51q/e", "headstash.terp.network/v2/elig.json", "host.example:8443/e.json"] {
                fw::validate_hal(&ok(u)).unwrap_or_else(|e| panic!("{u}: {e}"));
            }
            for u in ["", "javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,x", "file:///etc/passwd", "vbscript:x", "blob:x",
                      "http://x.example/e", "ftp://x", "ipfs:bafy", "a b", "x\"y", "x<y", "x\\y", &"a".repeat(513), "host/?q=javascript:x"] {
                assert!(fw::validate_hal(&ok(u)).is_err(), "{u:?}");
            }
            assert!(fw::validate_hal(&HalSource { uri: "ipfs://x".into(), sha256: None, cid: None }).unwrap_err().to_string().contains("sha256 or cid"));
            assert!(fw::validate_hal(&HalSource { uri: "ipfs://x".into(), sha256: Some(HexBinary::from(vec![1u8; 31])), cid: None }).is_err());
            assert!(fw::validate_hal(&HalSource { uri: "ipfs://x".into(), sha256: None, cid: Some("short".into()) }).is_err());
            fw::validate_hal(&HalSource { uri: "ipfs://x".into(), sha256: None, cid: Some(cid.clone()) }).unwrap();
            let stranger = deps.api.addr_make("stranger");
            let src = HalSource { uri: format!("ipfs://{cid}"), sha256: sha.clone(), cid: Some(cid.clone()) };
            assert!(execute(deps.as_mut(), env.clone(), message_info(&stranger, &[]), ExecuteMsg::SetHalSource { source: Some(src.clone()) }).is_err());
            owner_run(&mut deps, &env, ExecuteMsg::SetHalSource { source: Some(src.clone()) }).unwrap();
            let q: Option<HalSource> = cosmwasm_std::from_json(query(deps.as_ref(), env.clone(), QueryMsg::HalSource {}).unwrap()).unwrap();
            assert_eq!(q, Some(src));
            owner_run(&mut deps, &env, ExecuteMsg::SetHalSource { source: None }).unwrap();
            let q: Option<HalSource> = cosmwasm_std::from_json(query(deps.as_ref(), env.clone(), QueryMsg::HalSource {}).unwrap()).unwrap();
            assert!(q.is_none());
        }

        #[test]
        fn transfer_sequence_decodes_protobuf() {
            assert_eq!(fw::transfer_sequence(&[0x08, 0x05]).unwrap(), 5);
            assert_eq!(fw::transfer_sequence(&[0x08, 0xac, 0x02]).unwrap(), 300);
            assert_eq!(fw::transfer_sequence(&[0x12, 0x01, 0x41, 0x08, 0x07]).unwrap(), 7);
            assert!(fw::transfer_sequence(&[]).is_err());
            assert!(fw::transfer_sequence(&[0x08]).is_err());
        }

        fn state(deps: &TDeps) -> Vec<(Vec<u8>, Vec<u8>)> {
            deps.storage.range(None, None, Order::Ascending).filter(|(k, _)| k.as_slice() != b"contract_info").collect()
        }

        #[test]
        fn migrate_preserves_state_from_v001_without_cw2_and_refuses_bad_sources() {
            let (mut deps, env, a, root) = fwd_fixture();
            claim(&mut deps, &env, &a, "w1", &root, 1, DENOMS_UTERP[0]).unwrap();
            // mainnet code 105 instances have no cw2 entry
            deps.storage.remove(b"contract_info");
            let before = state(&deps);
            let r = crate::migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap();
            assert!(r.attributes.iter().any(|x| x.key == "from_version" && x.value == "none"));
            assert_eq!(state(&deps), before, "migrate rewrote state");
            let v = cw2::get_contract_version(deps.as_ref().storage).unwrap();
            assert_eq!((v.contract.as_str(), v.version.as_str()), ("cw-headstash", env!("CARGO_PKG_VERSION")));
            // idempotent same-version migrate
            crate::migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap();
            assert!(err(claim(&mut deps, &env, &a, "w2", &root, 1, DENOMS_UTERP[0])).contains("nullifier"));
            // downgrade and foreign contract refused
            cw2::set_contract_version(deps.as_mut().storage, "cw-headstash", "99.0.0").unwrap();
            assert!(crate::migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap_err().to_string().contains("downgrade"));
            cw2::set_contract_version(deps.as_mut().storage, "cw20-base", "0.0.1").unwrap();
            assert!(crate::migrate(deps.as_mut(), env.clone(), MigrateMsg {}).is_err());
            // not a headstash at all
            let mut empty = cosmwasm_std::testing::mock_dependencies();
            assert!(crate::migrate(empty.as_mut(), env, MigrateMsg {}).unwrap_err().to_string().contains("not a cw-headstash"));
        }
    }
}
