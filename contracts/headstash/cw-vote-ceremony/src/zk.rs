//! ZK proposal module: a DAO DAO-compatible proposal module whose ballots are
//! vote-sdk ZKP #2 (`vote_proof`) proofs checked with the host `proof_instance_verify`.
//!
//! Spec: `~/grok-bot/privacy-products/zk-voting/SPEC.md`.
//!
//! * Added to an existing DAO with `UpdateProposalModules { to_add }`; dao-core is the
//!   instantiator, so `dao` defaults to `info.sender`. It sits next to the DAO's other
//!   proposal modules and reads voting power from the DAO's existing voting module.
//! * Each proposal is one vote-sdk round. Its ceremony session `zkp-<id>` lives in this
//!   contract's `ceremony` map and its nullifiers in the shared `spent` map (SSOT).
//! * Cast: 11 Pallas-Fp public inputs (vote-sdk host order, see [`slot`]). The module
//!   binds tree root, anchor height, question index, round id and election key, burns
//!   the VAN nullifier, and optionally checks a RedPallas `r_vpk` signature that binds
//!   the ballot ciphertext to the proof.
//! * Tally: after the voting period, an election-authority FROST group key signs the
//!   totals over the ballots digest, or a tally circuit proves them (same host call).
//! * Execute: passed proposals call dao-core `execute_proposal_hook`.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, Api, Binary, CosmosMsg, Deps, DepsMut, Empty, Env, Event, HexBinary, MessageInfo, Order,
    QueryRequest, Response, StdResult, Timestamp, Uint128, WasmMsg, WasmQuery, to_json_binary,
};
use cw_storage_plus::{Bound, Item, Map};

use crate::error::ContractError;
use crate::msg::{CeremonyStatus, RegistrationGate};
use crate::state::{CEREMONIES, Ceremony, SPENT, spent_key};

/// Nullifier domain for zk proposals in the shared `spent` map.
pub const ZK_DOMAIN: &str = "terp.zkvote.v1";
pub const ROUND_DOMAIN: &[u8] = b"terp/zkvote/round/v1";
pub const CAST_DOMAIN: &[u8] = b"terp/zkvote/cast/v1";
pub const BALLOT_DOMAIN: &[u8] = b"terp/zkvote/ballot/v1";
pub const TALLY_DOMAIN: &[u8] = b"terp/zkvote/tally/v1";
/// vote-sdk `vote_proof` (ZKP #2) footer: Plonkish, curve_id VoteCommitment, k=11, i=11.
/// k is the circuit's own `K` in voting-circuits 0.10.0 (the version vote-sdk pins;
/// `vote_proof::circuit::K = 11`, ported in `crates/voting-circuits-zakura`). The footer
/// k must equal the params k.
pub const CAST_PROVER_ID: u8 = 0;
pub const CAST_CURVE_ID: u8 = 2;
pub const CAST_K: u8 = 11;
pub const CAST_INSTANCES: usize = 11;
/// Tally circuit public inputs: round_id, ballots_digest, ballots, yes, no, abstain.
pub const TALLY_INSTANCES: usize = 6;
/// Each module proposal is one vote-sdk round with a single question (index 1).
pub const QUESTION_INDEX: u64 = 1;
pub const MAX_BALLOT: usize = 2048;

/// vote-sdk ZKP #2 public-input slots, in the order the host verifier takes them.
pub mod slot {
    pub const VAN_NULLIFIER: usize = 0;
    pub const R_VPK_X: usize = 1;
    pub const R_VPK_Y: usize = 2;
    pub const VAN_NEW: usize = 3;
    pub const VOTE_COMMITMENT: usize = 4;
    pub const TREE_ROOT: usize = 5;
    pub const ANCHOR_HEIGHT: usize = 6;
    pub const PROPOSAL_ID: usize = 7;
    pub const ROUND_ID: usize = 8;
    pub const EA_PK_X: usize = 9;
    pub const EA_PK_Y: usize = 10;
}

#[cw_serde]
pub enum TallyAuth {
    /// 32-byte FROST-Ed25519 group key of the election authority (t-of-n off chain).
    Committee { group_vk: HexBinary },
    /// A tally / decryption circuit with TALLY_INSTANCES public inputs.
    Circuit {
        zkid: u64,
        circuit_key: Option<HexBinary>,
    },
}

#[cw_serde]
pub struct ZkModuleInit {
    /// Defaults to the instantiator (dao-core when added via UpdateProposalModules).
    pub dao: Option<String>,
    /// Defaults to dao-core's `voting_module {}`.
    pub voting_module: Option<String>,
    /// zkid of the stored vote-sdk `vote_proof` circuit.
    pub cast_zkid: u64,
    /// Expected 72-byte wasmvm circuit key. When set, its footer prefix is checked
    /// (prover 0, curve 2, k 13) and it must equal `CircuitInfo { zk_id }` on chain.
    pub cast_circuit_key: Option<HexBinary>,
    /// cw-vote-anchor registry; when set, proposals must take their root from it.
    pub anchor_registry: Option<String>,
    pub tally: TallyAuth,
    /// Seconds.
    pub voting_period: u64,
    /// Yes share of (yes + no) in basis points that must be exceeded.
    pub threshold_bps: u16,
    /// Turnout (yes + no + abstain) of total power in basis points.
    pub quorum_bps: u16,
    #[serde(default = "yes")]
    pub require_rvpk_sig: bool,
}
fn yes() -> bool {
    true
}

#[cw_serde]
pub struct ZkConfig {
    pub dao: Addr,
    pub voting_module: Addr,
    pub cast_zkid: u64,
    pub cast_circuit_key: Option<HexBinary>,
    pub anchor_registry: Option<Addr>,
    pub tally: TallyAuth,
    pub voting_period: u64,
    pub threshold_bps: u16,
    pub quorum_bps: u16,
    pub require_rvpk_sig: bool,
}

#[cw_serde]
pub enum AnchorRef {
    Registry { session_id: String, height: u64 },
    Root { root: HexBinary, height: u64 },
}

#[cw_serde]
pub enum TallyAttestation {
    Committee { signature: HexBinary },
    Proof { proof: Binary },
}

#[cw_serde]
pub enum ZkStatus {
    Open,
    Passed,
    Rejected,
    Executed,
}

#[cw_serde]
pub struct ZkTally {
    pub yes: Uint128,
    pub no: Uint128,
    pub abstain: Uint128,
}

#[cw_serde]
pub struct ZkProposal {
    pub id: u64,
    pub proposer: Addr,
    pub title: String,
    pub description: String,
    pub msgs: Vec<CosmosMsg<Empty>>,
    pub start_height: u64,
    pub expiration: Timestamp,
    pub total_power: Uint128,
    pub tree_root: HexBinary,
    pub anchor_height: u64,
    /// 64 bytes: ea_pk x || y, LE Pallas Fp each.
    pub ea_pk: HexBinary,
    pub round_id: HexBinary,
    pub session_id: String,
    pub ballots: u64,
    pub ballots_digest: HexBinary,
    pub tally: Option<ZkTally>,
    pub status: ZkStatus,
}

#[cw_serde]
pub struct ZkBallot {
    pub nullifier: HexBinary,
    pub vote_commitment: HexBinary,
    pub van_new: HexBinary,
    /// Opaque vote-share ciphertext for the election authority.
    pub ballot: Binary,
}

#[cw_serde]
pub struct ContractVersion {
    pub contract: String,
    pub version: String,
}
#[cw_serde]
pub struct InfoResponse {
    pub info: ContractVersion,
}

pub(crate) const ZK_CONFIG: Item<ZkConfig> = Item::new("zk_config");
const ZK_NEXT_ID: Item<u64> = Item::new("zk_next_id");
const ZK_PROPOSALS: Map<u64, ZkProposal> = Map::new("zk_props");
const ZK_BALLOTS: Map<(u64, u64), ZkBallot> = Map::new("zk_ballots");

// ---- DAO DAO query shapes (dao-interface JSON, no crate dependency) ----
#[cw_serde]
enum DaoCoreQuery {
    VotingModule {},
}
#[cw_serde]
enum VotingQuery {
    VotingPowerAtHeight {
        address: String,
        height: Option<u64>,
    },
    TotalPowerAtHeight {
        height: Option<u64>,
    },
}
#[cw_serde]
struct PowerResponse {
    power: Uint128,
    height: u64,
}
#[cw_serde]
enum DaoCoreExec {
    ExecuteProposalHook { msgs: Vec<CosmosMsg<Empty>> },
}
#[cw_serde]
enum AnchorQuery {
    GetAnchorAtHeight { session_id: String, height: u64 },
}
#[cw_serde]
struct AnchorResponse {
    session_id: String,
    height: u64,
    root: Option<Binary>,
}

fn zerr<T>(m: impl Into<String>) -> Result<T, ContractError> {
    Err(ContractError::Zk(m.into()))
}

fn lp(v: &mut Vec<u8>, b: &[u8]) {
    v.extend_from_slice(&(b.len() as u16).to_be_bytes());
    v.extend_from_slice(b);
}

/// u64 as a 32-byte little-endian Pallas field element.
pub fn fe_u64(x: u64) -> [u8; 32] {
    let mut o = [0u8; 32];
    o[..8].copy_from_slice(&x.to_le_bytes());
    o
}
pub fn fe_u128(x: u128) -> [u8; 32] {
    let mut o = [0u8; 32];
    o[..16].copy_from_slice(&x.to_le_bytes());
    o
}
/// Clear the top two bits so a 32-byte hash is a canonical Pallas Fp (p > 2^254).
pub fn to_fe(mut h: [u8; 32]) -> [u8; 32] {
    h[31] &= 0x3f;
    h
}

/// Expected first 4 bytes of each 36-byte half of the cast circuit key:
/// `u32::from_be_bytes([prover_id, curve_id, k, 0]).to_le_bytes()`.
pub fn cast_appstate_prefix() -> [u8; 4] {
    u32::from_be_bytes([CAST_PROVER_ID, CAST_CURVE_ID, CAST_K, 0]).to_le_bytes()
}

pub fn round_id(api: &dyn Api, chain_id: &str, contract: &str, proposal_id: u64) -> [u8; 32] {
    let mut v = ROUND_DOMAIN.to_vec();
    lp(&mut v, chain_id.as_bytes());
    lp(&mut v, contract.as_bytes());
    v.extend_from_slice(&proposal_id.to_be_bytes());
    to_fe(api.blake2b_256(&v))
}

pub fn cast_sighash(
    api: &dyn Api,
    chain_id: &str,
    contract: &str,
    proposal_id: u64,
    nullifier: &[u8],
    vote_commitment: &[u8],
    ballot: &[u8],
) -> [u8; 32] {
    let mut v = CAST_DOMAIN.to_vec();
    lp(&mut v, chain_id.as_bytes());
    lp(&mut v, contract.as_bytes());
    v.extend_from_slice(&proposal_id.to_be_bytes());
    lp(&mut v, nullifier);
    lp(&mut v, vote_commitment);
    v.extend_from_slice(&api.blake2b_256(ballot));
    api.blake2b_256(&v)
}

fn next_ballots_digest(api: &dyn Api, prev: &[u8], b: &ZkBallot) -> [u8; 32] {
    let mut v = BALLOT_DOMAIN.to_vec();
    v.extend_from_slice(prev);
    v.extend_from_slice(b.nullifier.as_slice());
    v.extend_from_slice(b.vote_commitment.as_slice());
    v.extend_from_slice(&api.blake2b_256(b.ballot.as_slice()));
    api.blake2b_256(&v)
}

#[allow(clippy::too_many_arguments)]
pub fn tally_digest(
    api: &dyn Api,
    chain_id: &str,
    contract: &str,
    p: &ZkProposal,
    yes: Uint128,
    no: Uint128,
    abstain: Uint128,
) -> [u8; 32] {
    let mut v = TALLY_DOMAIN.to_vec();
    lp(&mut v, chain_id.as_bytes());
    lp(&mut v, contract.as_bytes());
    v.extend_from_slice(&p.id.to_be_bytes());
    v.extend_from_slice(p.round_id.as_slice());
    v.extend_from_slice(p.ballots_digest.as_slice());
    v.extend_from_slice(&p.ballots.to_be_bytes());
    for x in [yes, no, abstain] {
        v.extend_from_slice(&x.u128().to_be_bytes());
    }
    api.blake2b_256(&v)
}

/// Tally circuit instances: 6 x 32-byte LE Fp.
pub fn tally_instances(p: &ZkProposal, yes: Uint128, no: Uint128, abstain: Uint128) -> Vec<u8> {
    let mut d = [0u8; 32];
    d.copy_from_slice(p.ballots_digest.as_slice());
    let mut v = Vec::with_capacity(32 * TALLY_INSTANCES);
    v.extend_from_slice(p.round_id.as_slice());
    v.extend_from_slice(&to_fe(d));
    v.extend_from_slice(&fe_u64(p.ballots));
    for x in [yes, no, abstain] {
        v.extend_from_slice(&fe_u128(x.u128()));
    }
    v
}

/// Compressed Pallas point from affine x and the parity of y (pasta encoding).
pub fn compress_point(x: &[u8], y: &[u8]) -> [u8; 32] {
    let mut o = [0u8; 32];
    o.copy_from_slice(x);
    o[31] |= (y[0] & 1) << 7;
    o
}

fn check_circuit_key(
    deps: Deps,
    zkid: u64,
    key: &HexBinary,
    prefix: Option<[u8; 4]>,
) -> Result<(), ContractError> {
    if key.len() != 72 {
        return zerr("circuit_key must be 72 bytes");
    }
    if let Some(p) = prefix
        && (key.as_slice()[..4] != p || key.as_slice()[36..40] != p)
    {
        return zerr(format!(
            "circuit footer prefix mismatch: want prover {CAST_PROVER_ID} curve {CAST_CURVE_ID} k {CAST_K}"
        ));
    }
    let info: cosmwasm_std::CircuitInfoResponse = deps
        .querier
        .query(&QueryRequest::Wasm(WasmQuery::CircuitInfo { zk_id: zkid }))?;
    if info.circuit_key.as_slice() != key.as_slice() {
        return zerr(format!(
            "zkid {zkid} circuit_key on chain does not match the pinned key"
        ));
    }
    Ok(())
}

pub fn init(deps: DepsMut, info: &MessageInfo, z: ZkModuleInit) -> Result<Response, ContractError> {
    let dao = match z.dao {
        Some(d) => deps.api.addr_validate(&d)?,
        None => info.sender.clone(),
    };
    let voting_module = match z.voting_module {
        Some(v) => deps.api.addr_validate(&v)?,
        None => deps
            .querier
            .query_wasm_smart(&dao, &DaoCoreQuery::VotingModule {})?,
    };
    if z.voting_period == 0
        || z.threshold_bps == 0
        || z.threshold_bps > 10_000
        || z.quorum_bps > 10_000
    {
        return zerr("voting_period > 0, 0 < threshold_bps <= 10000, quorum_bps <= 10000");
    }
    if let Some(k) = &z.cast_circuit_key {
        check_circuit_key(deps.as_ref(), z.cast_zkid, k, Some(cast_appstate_prefix()))?;
    }
    match &z.tally {
        TallyAuth::Committee { group_vk } if group_vk.len() != 32 => {
            return zerr("group_vk must be 32 bytes");
        }
        TallyAuth::Circuit {
            zkid,
            circuit_key: Some(k),
        } => check_circuit_key(deps.as_ref(), *zkid, k, None)?,
        _ => {}
    }
    let anchor_registry = match z.anchor_registry {
        Some(a) => Some(deps.api.addr_validate(&a)?),
        None => None,
    };
    ZK_CONFIG.save(
        deps.storage,
        &ZkConfig {
            dao: dao.clone(),
            voting_module,
            cast_zkid: z.cast_zkid,
            cast_circuit_key: z.cast_circuit_key,
            anchor_registry,
            tally: z.tally,
            voting_period: z.voting_period,
            threshold_bps: z.threshold_bps,
            quorum_bps: z.quorum_bps,
            require_rvpk_sig: z.require_rvpk_sig,
        },
    )?;
    ZK_NEXT_ID.save(deps.storage, &1)?;
    Ok(Response::new()
        .add_attribute("zk_proposals", "enabled")
        .add_attribute("dao", dao))
}

fn power(deps: Deps, voting: &Addr, address: Option<String>) -> StdResult<Uint128> {
    let q = match address {
        Some(address) => VotingQuery::VotingPowerAtHeight {
            address,
            height: None,
        },
        None => VotingQuery::TotalPowerAtHeight { height: None },
    };
    let r: PowerResponse = deps.querier.query_wasm_smart(voting, &q)?;
    Ok(r.power)
}

#[allow(clippy::too_many_arguments)]
pub fn propose(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    title: String,
    description: String,
    msgs: Vec<CosmosMsg<Empty>>,
    ea_pk: HexBinary,
    anchor: AnchorRef,
) -> Result<Response, ContractError> {
    let c = ZK_CONFIG
        .load(deps.storage)
        .map_err(|_| ContractError::Zk("zk proposals not enabled".into()))?;
    if power(
        deps.as_ref(),
        &c.voting_module,
        Some(info.sender.to_string()),
    )?
    .is_zero()
    {
        return Err(ContractError::Unauthorized);
    }
    let total_power = power(deps.as_ref(), &c.voting_module, None)?;
    if total_power.is_zero() {
        return zerr("voting module reports zero total power");
    }
    if ea_pk.len() != 64 {
        return zerr("ea_pk must be 64 bytes (x || y)");
    }
    let (tree_root, anchor_height) = match (anchor, &c.anchor_registry) {
        (AnchorRef::Registry { session_id, height }, Some(reg)) => {
            let r: AnchorResponse = deps
                .querier
                .query_wasm_smart(reg, &AnchorQuery::GetAnchorAtHeight { session_id, height })?;
            match r.root {
                Some(root) => (HexBinary::from(root.to_vec()), height),
                None => return zerr("no anchor posted at that height"),
            }
        }
        (AnchorRef::Registry { .. }, None) => return zerr("no anchor_registry configured"),
        (AnchorRef::Root { .. }, Some(_)) => {
            return zerr("roots must come from the anchor registry");
        }
        (AnchorRef::Root { root, height }, None) => (root, height),
    };
    if tree_root.len() != 32 {
        return zerr("tree root must be 32 bytes");
    }
    let id = ZK_NEXT_ID.load(deps.storage)?;
    ZK_NEXT_ID.save(deps.storage, &(id + 1))?;
    let session_id = format!("zkp-{id}");
    if CEREMONIES.has(deps.storage, &session_id) {
        return Err(ContractError::CeremonyExists { session_id });
    }
    CEREMONIES.save(
        deps.storage,
        &session_id,
        &Ceremony {
            domain: ZK_DOMAIN.into(),
            status: CeremonyStatus::Active,
            registration_open: false,
            anchor_policy: Some(format!("zk-proposal:{}", tree_root.to_hex())),
            registration_gate: RegistrationGate::Open,
            clerk: None,
            membership_module: Some(c.voting_module.to_string()),
            vm_verifier: None,
        },
    )?;
    let rid = round_id(
        deps.api,
        &env.block.chain_id,
        env.contract.address.as_str(),
        id,
    );
    let p = ZkProposal {
        id,
        proposer: info.sender.clone(),
        title,
        description,
        msgs,
        start_height: env.block.height,
        expiration: env.block.time.plus_seconds(c.voting_period),
        total_power,
        tree_root,
        anchor_height,
        ea_pk,
        round_id: HexBinary::from(rid.to_vec()),
        session_id: session_id.clone(),
        ballots: 0,
        ballots_digest: HexBinary::from(vec![0u8; 32]),
        tally: None,
        status: ZkStatus::Open,
    };
    ZK_PROPOSALS.save(deps.storage, id, &p)?;
    Ok(Response::new().add_event(
        Event::new("zk-propose")
            .add_attribute("proposal_id", id.to_string())
            .add_attribute("proposer", info.sender)
            .add_attribute("session_id", session_id)
            .add_attribute("round_id", p.round_id.to_hex())
            .add_attribute("expiration", p.expiration.seconds().to_string()),
    ))
}

pub fn cast(
    deps: DepsMut,
    env: Env,
    proposal_id: u64,
    proof: Binary,
    instances: Binary,
    ballot: Binary,
    rvpk_sig: Option<HexBinary>,
) -> Result<Response, ContractError> {
    let c = ZK_CONFIG.load(deps.storage)?;
    let mut p = ZK_PROPOSALS.load(deps.storage, proposal_id)?;
    if p.status != ZkStatus::Open || env.block.time >= p.expiration {
        return zerr("voting is closed");
    }
    if instances.len() != 32 * CAST_INSTANCES {
        return zerr(format!("instances must be {} bytes", 32 * CAST_INSTANCES));
    }
    if ballot.len() > MAX_BALLOT {
        return zerr("ballot too large");
    }
    let s = |i: usize| &instances.as_slice()[i * 32..(i + 1) * 32];
    if s(slot::TREE_ROOT) != p.tree_root.as_slice() {
        return zerr("vote_comm_tree_root does not match the proposal anchor");
    }
    if s(slot::ANCHOR_HEIGHT) != fe_u64(p.anchor_height) {
        return zerr("anchor height mismatch");
    }
    if s(slot::PROPOSAL_ID) != fe_u64(QUESTION_INDEX) {
        return zerr("question index mismatch");
    }
    if s(slot::ROUND_ID) != p.round_id.as_slice() {
        return zerr("voting_round_id mismatch");
    }
    if s(slot::EA_PK_X) != &p.ea_pk.as_slice()[..32]
        || s(slot::EA_PK_Y) != &p.ea_pk.as_slice()[32..]
    {
        return zerr("election authority key mismatch");
    }
    let nf = s(slot::VAN_NULLIFIER).to_vec();
    if SPENT.has(deps.storage, spent_key(ZK_DOMAIN, &p.session_id, &nf)) {
        return Err(ContractError::AlreadySpent {
            domain: ZK_DOMAIN.into(),
            session: p.session_id,
        });
    }
    let contract = env.contract.address.to_string();
    if c.require_rvpk_sig {
        let Some(sig) = rvpk_sig else {
            return zerr("rvpk_sig required");
        };
        let msg = cast_sighash(
            deps.api,
            &env.block.chain_id,
            &contract,
            proposal_id,
            &nf,
            s(slot::VOTE_COMMITMENT),
            ballot.as_slice(),
        );
        let rk = compress_point(s(slot::R_VPK_X), s(slot::R_VPK_Y));
        match deps
            .api
            .redpallas_spendauth_verify(&msg, sig.as_slice(), &rk)
        {
            Ok(true) => {}
            Ok(false) => return zerr("rvpk_sig invalid"),
            Err(e) => return zerr(format!("rvpk_sig: {e}")),
        }
    }
    match deps
        .api
        .proof_instance_verify(c.cast_zkid, proof.as_slice(), instances.as_slice())
    {
        Ok(true) => {}
        Ok(false) => return zerr("vote proof rejected"),
        Err(e) => return zerr(format!("proof_instance_verify: {e}")),
    }
    SPENT.save(
        deps.storage,
        spent_key(ZK_DOMAIN, &p.session_id, &nf),
        &Empty {},
    )?;
    let b = ZkBallot {
        nullifier: HexBinary::from(nf),
        vote_commitment: HexBinary::from(s(slot::VOTE_COMMITMENT).to_vec()),
        van_new: HexBinary::from(s(slot::VAN_NEW).to_vec()),
        ballot,
    };
    let d = next_ballots_digest(deps.api, p.ballots_digest.as_slice(), &b);
    p.ballots += 1;
    p.ballots_digest = HexBinary::from(d.to_vec());
    ZK_BALLOTS.save(deps.storage, (proposal_id, p.ballots), &b)?;
    ZK_PROPOSALS.save(deps.storage, proposal_id, &p)?;
    // No voter address: the sender is any relayer.
    Ok(Response::new().add_event(
        Event::new("zk-vote")
            .add_attribute("proposal_id", proposal_id.to_string())
            .add_attribute("nullifier", b.nullifier.to_hex())
            .add_attribute("ballots", p.ballots.to_string()),
    ))
}

pub fn submit_tally(
    deps: DepsMut,
    env: Env,
    proposal_id: u64,
    yes: Uint128,
    no: Uint128,
    abstain: Uint128,
    attestation: TallyAttestation,
) -> Result<Response, ContractError> {
    let c = ZK_CONFIG.load(deps.storage)?;
    let mut p = ZK_PROPOSALS.load(deps.storage, proposal_id)?;
    if p.status != ZkStatus::Open {
        return zerr("proposal already tallied");
    }
    if env.block.time < p.expiration {
        return zerr("voting period has not ended");
    }
    let turnout = yes.checked_add(no)?.checked_add(abstain)?;
    if turnout > p.total_power {
        return zerr("tally exceeds total voting power");
    }
    match (&c.tally, attestation) {
        (TallyAuth::Committee { group_vk }, TallyAttestation::Committee { signature }) => {
            let d = tally_digest(
                deps.api,
                &env.block.chain_id,
                env.contract.address.as_str(),
                &p,
                yes,
                no,
                abstain,
            );
            match deps
                .api
                .ed25519_verify(&d, signature.as_slice(), group_vk.as_slice())
            {
                Ok(true) => {}
                _ => return zerr("tally signature invalid"),
            }
        }
        (TallyAuth::Circuit { zkid, .. }, TallyAttestation::Proof { proof }) => {
            let inst = tally_instances(&p, yes, no, abstain);
            match deps
                .api
                .proof_instance_verify(*zkid, proof.as_slice(), &inst)
            {
                Ok(true) => {}
                Ok(false) => return zerr("tally proof rejected"),
                Err(e) => return zerr(format!("proof_instance_verify: {e}")),
            }
        }
        _ => return zerr("attestation kind does not match the module tally policy"),
    }
    let quorum = turnout.full_mul(10_000u128) >= p.total_power.full_mul(c.quorum_bps as u128);
    let threshold =
        yes.full_mul(10_000u128) > yes.checked_add(no)?.full_mul(c.threshold_bps as u128);
    p.status = if quorum && threshold {
        ZkStatus::Passed
    } else {
        ZkStatus::Rejected
    };
    p.tally = Some(ZkTally { yes, no, abstain });
    let mut cer = CEREMONIES.load(deps.storage, &p.session_id)?;
    cer.status = CeremonyStatus::Closed;
    CEREMONIES.save(deps.storage, &p.session_id, &cer)?;
    ZK_PROPOSALS.save(deps.storage, proposal_id, &p)?;
    Ok(Response::new().add_event(
        Event::new("zk-tally")
            .add_attribute("proposal_id", proposal_id.to_string())
            .add_attribute("yes", yes)
            .add_attribute("no", no)
            .add_attribute("abstain", abstain)
            .add_attribute("status", format!("{:?}", p.status)),
    ))
}

pub fn execute_proposal(deps: DepsMut, proposal_id: u64) -> Result<Response, ContractError> {
    let c = ZK_CONFIG.load(deps.storage)?;
    let mut p = ZK_PROPOSALS.load(deps.storage, proposal_id)?;
    if p.status != ZkStatus::Passed {
        return zerr("proposal has not passed");
    }
    p.status = ZkStatus::Executed;
    ZK_PROPOSALS.save(deps.storage, proposal_id, &p)?;
    let hook = WasmMsg::Execute {
        contract_addr: c.dao.to_string(),
        msg: to_json_binary(&DaoCoreExec::ExecuteProposalHook { msgs: p.msgs })?,
        funds: vec![],
    };
    Ok(Response::new()
        .add_message(hook)
        .add_event(Event::new("zk-execute").add_attribute("proposal_id", proposal_id.to_string())))
}

pub fn query_proposals(
    deps: Deps,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<Vec<ZkProposal>> {
    ZK_PROPOSALS
        .range(
            deps.storage,
            start_after.map(Bound::exclusive),
            None,
            Order::Ascending,
        )
        .take(limit.unwrap_or(30).min(100) as usize)
        .map(|r| r.map(|(_, p)| p))
        .collect()
}
pub fn query_proposal(deps: Deps, id: u64) -> StdResult<ZkProposal> {
    ZK_PROPOSALS.load(deps.storage, id)
}
pub fn query_ballots(
    deps: Deps,
    id: u64,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<Vec<ZkBallot>> {
    ZK_BALLOTS
        .prefix(id)
        .range(
            deps.storage,
            start_after.map(Bound::exclusive),
            None,
            Order::Ascending,
        )
        .take(limit.unwrap_or(30).min(100) as usize)
        .map(|r| r.map(|(_, b)| b))
        .collect()
}
pub fn next_id(deps: Deps) -> StdResult<u64> {
    Ok(ZK_NEXT_ID.may_load(deps.storage)?.unwrap_or(1))
}
pub fn info() -> InfoResponse {
    InfoResponse {
        info: ContractVersion {
            contract: "crates.io:cw-vote-ceremony".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    }
}
