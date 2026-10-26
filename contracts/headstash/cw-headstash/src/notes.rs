//! Spec rev 5 note registration (vk_id 3). One signature per identity over the
//! ordered note list hash; chunks append `note_leaf(key_com, DENOMS[class])` into the
//! depth-17 claim tree. Proofs are NOT verified here: the sidecar lane verifies them
//! off-chain and payout goes through the epoch seal + hashmerchant aggregate.

use cosmwasm_std::{Binary, DepsMut, Response, StdError, StdResult, Storage};
use cw_storage_plus::Map;
use std::collections::HashSet;

use crate::epoch::EpochCampaignParams;
use crate::registration::{
    append_claim_leaf, key_com_used, parse_fp, path_bytes, precheck, registration_cfg,
    sha256, NOTE_DEPTH,
};
use zk_headstash::distro_poseidon::note_registration_leaf;

pub const NOTE_VK_ID: u64 = 3;
pub const RETIRED_VK_ID: u64 = 2;
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

pub fn register_notes_message(ctx: &[u8], list_hash: &[u8], n: u32) -> String {
    format!(
        "headstash-register-notes-v2:{}:{}:{n}",
        hex::encode(ctx),
        hex::encode(list_hash)
    )
}

pub fn register_notes_sol_message(ctx: &[u8], list_hash: &[u8], n: u32) -> String {
    format!(
        "headstash-register-notes-sol-v2:{}:{}:{n}",
        hex::encode(ctx),
        hex::encode(list_hash)
    )
}

#[allow(clippy::too_many_arguments)]
pub fn register_notes(
    deps: DepsMut,
    campaign: &EpochCampaignParams,
    signer: NoteSigner,
    signature: Option<Binary>,
    list: Vec<NoteListEntry>,
    start: u32,
    eligibility: Vec<NoteEligibility>,
) -> Result<Response, StdError> {
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
    let (id_tag, identity_bytes, identity) = match &signer {
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
    let list_hash = note_list_hash(ctx, id_tag, &identity_bytes, &list)?;
    let stored = NOTE_LIST.may_load(deps.storage, identity.as_str())?;
    let mut state = match stored {
        None => {
            if start != 0 {
                return Err(StdError::msg("chunk"));
            }
            if crate::registration::is_registered(deps.storage, &identity)? {
                return Err(StdError::msg("already registered"));
            }
            // N6: no repeated key_com inside the list, none already in the tree.
            let mut seen = HashSet::new();
            for entry in &list {
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
            verify_signer(deps.api, &signer, ctx, &list_hash, n as u32, sig.as_slice())?;
            NoteListState {
                list_hash: Binary::from(list_hash.to_vec()),
                n: n as u32,
                next: 0,
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
            state
        }
    };
    let mut first_leaf = None;
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
    Ok(Response::new()
        .add_attribute("action", "register_notes")
        .add_attribute("start", start.to_string())
        .add_attribute("count", count.to_string())
        .add_attribute("first_leaf", first_leaf.unwrap_or(0).to_string())
        .add_attribute("done", (state.next == state.n).to_string()))
}

fn verify_signer(
    api: &dyn cosmwasm_std::Api,
    signer: &NoteSigner,
    ctx: &[u8],
    list_hash: &[u8; 32],
    n: u32,
    signature: &[u8],
) -> StdResult<()> {
    match signer {
        NoteSigner::Evm { address } => {
            let message = register_notes_message(ctx, list_hash, n);
            let got = crate::registration::recover_address(api, &message, signature)?;
            if got.as_slice() != address.as_slice() {
                return Err(StdError::msg("signature"));
            }
        }
        NoteSigner::Sol { pubkey } => {
            if signature.len() != 64 {
                return Err(StdError::msg("signature"));
            }
            let message = register_notes_sol_message(ctx, list_hash, n);
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
            token_strategy: crate::tokenfactory::TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new("cht".into()),
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
            }),
            wavs: generate_test_wavs_proof(1),
            circuit_id: None,
        };
        instantiate(deps.as_mut(), env.clone(), message_info(&creator, &[]), msg).unwrap();
        (deps, env)
    }

    fn ctx() -> [u8; 32] {
        pallas::Base::from(5u64).to_repr()
    }

    fn campaign(root: [u8; 32], vk_id: u64) -> EpochCampaignParams {
        EpochCampaignParams {
            ctx: Binary::from(ctx().to_vec()),
            backend: "halo2".into(),
            cutoff_height: 10,
            bech32_hrp: "cosmwasm".into(),
            denom: derive_nd("cht"),
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
    }

    fn sign_with(key: &SigningKey, message: &str) -> Vec<u8> {
        let mut pre = b"\x19Ethereum Signed Message:\n".to_vec();
        pre.extend_from_slice(message.len().to_string().as_bytes());
        pre.extend_from_slice(message.as_bytes());
        let digest = zk_headstash::claim_auth::keccak256(&pre);
        let (signature, recid) = key.sign_prehash_recoverable(&digest).unwrap();
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
        Holder { key, address, classes, first, word }
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
            let msg = register_notes_message(&ctx(), &h, list.len() as u32);
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
        let anchor = tree_state.anchors.last().unwrap().root.clone();
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
}
