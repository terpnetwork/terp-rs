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

const DEPTH: usize = 16;
const MAX_LEAVES: u32 = 1 << DEPTH;

const FRONTIER: Item<Vec<Binary>> = Item::new("reg_frontier");
const COUNT: Item<u32> = Item::new("reg_count");
const ROOT: Item<Binary> = Item::new("reg_root");
const NEXT_ANCHOR: Item<u64> = Item::new("reg_next_anchor");
const ANCHORS: Item<Vec<Binary>> = Item::new("reg_anchors");
const ADDRESSES: Map<&str, ()> = Map::new("reg_addr");
const KEYS: Map<&str, ()> = Map::new("reg_key");
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
pub struct RegistrationTreeResponse {
    pub count: u32,
    pub root: Binary,
    pub next_anchor_height: u64,
    pub anchors: Vec<Binary>,
}

pub fn validate_config(cfg: &RegistrationConfig, ctx: &[u8]) -> StdResult<()> {
    if cfg.eligibility_root.len() != 32 {
        return Err(StdError::msg("eligibility"));
    }
    if !(1..=DEPTH as u32).contains(&cfg.eligibility_depth) {
        return Err(StdError::msg("eligibility"));
    }
    if cfg.min_anonymity_set == 0 || cfg.min_anonymity_set > MAX_LEAVES {
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

pub fn prepare_window(storage: &mut dyn Storage, cfg: &RegistrationConfig) -> StdResult<()> {
    if COUNT.may_load(storage)?.unwrap_or(0) > 0 {
        return Ok(());
    }
    if FRONTIER.may_load(storage)?.is_none() {
        FRONTIER.save(storage, &vec![Binary::from(vec![0u8; 32]); DEPTH])?;
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
        anchors: ANCHORS.may_load(storage)?.unwrap_or_default(),
    })
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
    let cfg = campaign
        .registration
        .as_ref()
        .ok_or_else(|| StdError::msg("registration"))?;
    let count = leaf_count(deps.storage)?;
    if count >= MAX_LEAVES {
        return Err(StdError::msg("tree full"));
    }
    if amount == 0 {
        return Err(StdError::msg("amount"));
    }
    let key_fp = parse_fp(key_com.as_slice()).map_err(|_| StdError::msg("key commitment"))?;
    let message = register_message(campaign.ctx.as_slice(), key_com.as_slice(), amount);
    let address = recover_address(deps.api, &message, signature.as_slice())?;
    let path = path_bytes(&eligibility_path)?;
    let opened = open_eligibility(
        &address,
        amount,
        eligibility_index,
        &path,
        cfg.eligibility_depth,
    )?;
    if opened.as_slice() != cfg.eligibility_root.as_slice() {
        return Err(StdError::msg("eligibility"));
    }
    let addr_key = hex::encode(address);
    if ADDRESSES
        .may_load(deps.storage, addr_key.as_str())?
        .is_some()
    {
        return Err(StdError::msg("already registered"));
    }
    let key_hex = hex::encode(key_com.as_slice());
    if KEYS.may_load(deps.storage, key_hex.as_str())?.is_some() {
        return Err(StdError::msg("key commitment"));
    }
    let leaf_fp = registration_leaf(key_fp, amount);
    let leaf = leaf_fp.to_repr();
    let mut frontier = load_frontier(deps.storage)?;
    let root = append_leaf(&mut frontier, count, leaf)?;
    FRONTIER.save(
        deps.storage,
        &frontier
            .iter()
            .map(|node| Binary::from(node.to_vec()))
            .collect(),
    )?;
    COUNT.save(deps.storage, &(count + 1))?;
    ROOT.save(deps.storage, &Binary::from(root.to_vec()))?;
    ADDRESSES.save(deps.storage, addr_key.as_str(), &())?;
    KEYS.save(deps.storage, key_hex.as_str(), &())?;
    Ok(Response::new()
        .add_attribute("action", "register_headstash")
        .add_attribute("index", count.to_string())
        .add_attribute("leaf", hex::encode(leaf)))
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
    if anchors.last().is_some_and(|latest| latest == &root) {
        return Err(StdError::msg("anchor unchanged"));
    }
    anchors.push(root.clone());
    ANCHORS.save(deps.storage, &anchors)?;
    NEXT_ANCHOR.save(deps.storage, &(env.block.height + cfg.anchor_interval))?;
    Ok(Response::new()
        .add_attribute("action", "accept_anchor")
        .add_attribute("anchors", anchors.len().to_string()))
}

pub fn anchor_known(storage: &dyn Storage, anchor: &[u8]) -> StdResult<bool> {
    let anchors = ANCHORS.may_load(storage)?.unwrap_or_default();
    Ok(anchors.iter().any(|item| item.as_slice() == anchor))
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

fn path_bytes(path: &[Binary]) -> StdResult<Vec<[u8; 32]>> {
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
    if depth == 0 || depth > DEPTH as u32 || path.len() != depth as usize {
        return Err(StdError::msg("eligibility"));
    }
    if index >= 1u32 << depth {
        return Err(StdError::msg("eligibility"));
    }
    let mut node = eligibility_leaf(address, amount);
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

pub fn eligibility_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = b"terp-hs-elig-node-v1".to_vec();
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    sha256(&pre)
}

pub fn append_leaf(
    frontier: &mut [[u8; 32]; DEPTH],
    index: u32,
    leaf: [u8; 32],
) -> StdResult<[u8; 32]> {
    let mut current = parse_fp(&leaf)?;
    let mut cursor = index;
    for level in 0..DEPTH {
        if cursor & 1 == 0 {
            frontier[level] = current.to_repr();
            current = poseidon_distro_crh(level as u32, current, pallas::Base::ZERO);
        } else {
            let left = parse_fp(&frontier[level])?;
            current = poseidon_distro_crh(level as u32, left, current);
        }
        cursor >>= 1;
    }
    Ok(current.to_repr())
}

fn load_frontier(storage: &dyn Storage) -> StdResult<[[u8; 32]; DEPTH]> {
    let stored = FRONTIER
        .may_load(storage)?
        .unwrap_or_else(|| vec![Binary::from(vec![0u8; 32]); DEPTH]);
    if stored.len() != DEPTH {
        return Err(StdError::msg("registration"));
    }
    let mut out = [[0u8; 32]; DEPTH];
    for (slot, node) in out.iter_mut().enumerate() {
        *node = stored[slot]
            .as_slice()
            .try_into()
            .map_err(|_| StdError::msg("registration"))?;
    }
    Ok(out)
}

fn parse_fp(bytes: &[u8]) -> StdResult<pallas::Base> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StdError::msg("key commitment"))?;
    Option::from(pallas::Base::from_repr(arr)).ok_or_else(|| StdError::msg("key commitment"))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let dig = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::{
        attested_root, root_b64, EpochPayout, HashMerchantSudoMsg, HashMerchantSudoPayload,
        OracleHint, ALGO_ACCEPTED, BATCH_SOURCE_ID, CHAIN_UID,
    };
    use crate::msg::{ExecuteMsg, InstantiateMsg};
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
            cutoff_height: 10,
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
            registration: Some(RegistrationConfig {
                eligibility_root: Binary::from(eligibility_root.to_vec()),
                eligibility_depth: 1,
                min_anonymity_set: min_set,
                anchor_start_height: 1,
                anchor_interval: 1,
            }),
        }
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
                    height: 10,
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
                    height: 11,
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

    fn row_root() -> Vec<u8> {
        vec![0u8; 32]
    }
}
