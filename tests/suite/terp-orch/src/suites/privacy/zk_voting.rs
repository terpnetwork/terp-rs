//! `dao-zk-voting` (workstream A): an existing DAO DAO (dao-core + dao-voting-cw4 +
//! proposal-single + pre-propose-single) adds the cw-vote-ceremony zk proposal module
//! through its own governance, then runs a private vote whose passed result is executed
//! by dao-core. Spec: `~/grok-bot/privacy-products/zk-voting/SPEC.md`.
//!
//! Casting needs a `vote_proof` prover ([`CastProver`]). On `MockBech32` the module uses
//! [`ValarVoteProver`]: valargroup's real ZKP #2 (voting-circuits 0.10.0, ported to the
//! zakura fork) at k=11 / 11 instances, its keys stored in the chain's circuit format and
//! checked by the same host dispatch wasmvm runs. [`DevVoteProver`] (shape circuit) is a
//! fast test fallback. On a live chain pass a prover with [`ZkVotingModule::with_prover`].
//! Without one the DAO-integration half still runs and the slot reports `Skipped`.
use super::frost_util::Group;
use super::vote_circuit::{DevVoteKeys, StoredCircuit};
use super::{ChainSlot, Outcome, ProductModule, ProductReport, SuiteCtx, Workstream};
use anyhow::{Context, Result, anyhow, ensure};
use cosmwasm_std::{Addr, Binary, CosmosMsg, Empty, HexBinary, Uint128, Uint256, WasmMsg, coins};
use cw_orch::{interface, prelude::*};
use cw_vote_ceremony::interface::CwVoteCeremonyContract;
use cw_vote_ceremony::msg::{
    AnchorRef, ExecuteMsg as ZkExec, InstantiateMsg as ZkInit, QueryMsg as ZkQuery,
    TallyAttestation, TallyAuth, ZkModuleInit, ZkProposal, ZkStatus,
};
use cw_vote_ceremony::zk::{self, slot};
use dao_interface::state::{Admin, ModuleInstantiateInfo, ProposalModule};
use dao_voting::pre_propose::{PreProposeInfo, PreProposeSubmissionPolicy, ProposalCreationPolicy};
use dao_voting::threshold::{PercentageThreshold, Threshold};
use dao_voting::voting::Vote;

fn bin<T: serde::Serialize>(m: &T) -> Result<Binary> {
    cosmwasm_std::to_json_binary(m).map_err(|e| anyhow!("json: {e}"))
}

// --- DAO DAO cw-orch interfaces (mock wrapper + artifact name for Daemon) -----------

macro_rules! dao_iface {
    ($name:ident, $id:literal, $init:ty, $exec:ty, $query:ty, $wrap:expr) => {
        #[interface($init, $exec, $query, Empty, id = $id)]
        pub struct $name;
        impl<Chain> Uploadable for $name<Chain> {
            fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
                artifacts_dir_from_workspace!().find_wasm_path($id).unwrap()
            }
            fn wrapper() -> Box<dyn MockContract<Empty>> {
                Box::new($wrap)
            }
        }
    };
}
dao_iface!(
    Cw4Group,
    "cw4_group",
    cw4_group::msg::InstantiateMsg,
    cw4_group::msg::ExecuteMsg,
    cw4_group::msg::QueryMsg,
    ContractWrapper::new_with_empty(
        cw4_group::contract::execute,
        cw4_group::contract::instantiate,
        cw4_group::contract::query
    )
);
dao_iface!(
    DaoVotingCw4,
    "dao_voting_cw4",
    dao_voting_cw4::msg::InstantiateMsg,
    dao_voting_cw4::msg::ExecuteMsg,
    dao_voting_cw4::msg::QueryMsg,
    ContractWrapper::new_with_empty(
        dao_voting_cw4::contract::execute,
        dao_voting_cw4::contract::instantiate,
        dao_voting_cw4::contract::query
    )
    .with_reply(dao_voting_cw4::contract::reply)
);
dao_iface!(
    DaoProposalSingle,
    "dao_proposal_single",
    dao_proposal_single::msg::InstantiateMsg,
    dao_proposal_single::msg::ExecuteMsg,
    dao_proposal_single::msg::QueryMsg,
    ContractWrapper::new_with_empty(
        dao_proposal_single::contract::execute,
        dao_proposal_single::contract::instantiate,
        dao_proposal_single::contract::query
    )
    .with_reply(dao_proposal_single::contract::reply)
);
dao_iface!(
    DaoPreProposeSingle,
    "dao_pre_propose_single",
    dao_pre_propose_single::InstantiateMsg,
    dao_pre_propose_single::ExecuteMsg,
    dao_pre_propose_single::QueryMsg,
    ContractWrapper::new_with_empty(
        dao_pre_propose_single::contract::execute,
        dao_pre_propose_single::contract::instantiate,
        dao_pre_propose_single::contract::query
    )
);
dao_iface!(
    DaoCore,
    "dao_dao_core",
    dao_interface::msg::InstantiateMsg,
    dao_interface::msg::ExecuteMsg,
    dao_interface::msg::QueryMsg,
    ContractWrapper::new_with_empty(
        dao_dao_core::contract::execute,
        dao_dao_core::contract::instantiate,
        dao_dao_core::contract::query
    )
    .with_reply(dao_dao_core::contract::reply)
);

// --- prover hook ----------------------------------------------------------------------

/// Everything a voter's wallet + vote-sdk prover produce for one ballot.
pub struct CastBundle {
    pub proof: Binary,
    /// 11 x 32-byte host-order public inputs (see SPEC.md).
    pub instances: Vec<u8>,
    /// Opaque ElGamal share ciphertext for the election authority.
    pub ballot: Binary,
}

pub trait CastProver {
    /// zkid of the stored `vote_proof` circuit and its pinned 72-byte key (if checked).
    fn cast_zkid(&self) -> u64;
    fn circuit_key(&self) -> Option<HexBinary> {
        None
    }
    /// Called once before the module is instantiated (e.g. register a test circuit).
    fn prepare(&mut self) -> Result<()> {
        Ok(())
    }
    /// Prove ballot `voter` (fresh nullifier) for `proposal`.
    fn prove(&mut self, proposal: &ZkProposal, voter: usize) -> Result<CastBundle>;
    /// Re-prove with a previously used nullifier (replay check). `None` skips the check.
    fn prove_replay(
        &mut self,
        _proposal: &ZkProposal,
        _voter: usize,
    ) -> Option<Result<CastBundle>> {
        None
    }
    /// RedPallas SpendAuth signature over `ZkCastSighash` by the bundle's `r_vpk`.
    fn sign(&mut self, bundle: &CastBundle, sighash: &[u8]) -> Result<HexBinary>;
    /// Anchor for the next proposal, given the `round_id` the module will derive for it
    /// (`zk::round_id`). Default: placeholder EA key and root (shape-circuit fallback).
    fn election(&mut self, _round_id: [u8; 32]) -> Result<Election> {
        Ok(Election {
            ea_pk: EA,
            tree_root: ROOT,
            anchor_height: 1,
        })
    }
    /// Proof-generation times so far (benchmarks).
    fn prove_times(&self) -> &[std::time::Duration] {
        &[]
    }
}

/// What a proposal pins for its voters: the EA public key (x || y, LE Pallas Fp) and the
/// vote-commitment-tree anchor every voter proves membership against.
pub struct Election {
    pub ea_pk: [u8; 64],
    pub tree_root: [u8; 32],
    pub anchor_height: u64,
}

fn r_vpk(b: &CastBundle) -> [u8; 32] {
    zk::compress_point(
        &b.instances[slot::R_VPK_X * 32..slot::R_VPK_X * 32 + 32],
        &b.instances[slot::R_VPK_Y * 32..slot::R_VPK_Y * 32 + 32],
    )
}

fn sign_with(rsk: [u8; 32], rng: &mut rand::rngs::StdRng, sighash: &[u8]) -> Result<HexBinary> {
    let sk = reddsa::SigningKey::<reddsa::orchard::SpendAuth>::try_from(rsk)
        .map_err(|e| anyhow!("rsk: {e}"))?;
    let sig: [u8; 64] = sk.sign(rng, sighash).into();
    Ok(HexBinary::from(sig.to_vec()))
}

fn seeded(seed: u64) -> rand::rngs::StdRng {
    <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(seed)
}

// --- real prover: valar vote-sdk ZKP #2 ------------------------------------------------

use halo2_proofs::arithmetic::CurveAffine;
use halo2_proofs::pasta::{
    group::{Curve, ff::{Field, PrimeField}},
    pallas,
};
use voting_circuits::vote_proof::{anchor, build_vote_proof_from_delegation};

/// Voters in the offline election (all are in the vote commitment tree).
const VOTERS: usize = 5;
/// Per-voter weight in zatoshi (vote-sdk `BALLOT_DIVISOR` = 1 ballot).
const WEIGHT_ZAT: u64 = 12_500_000;
/// Proposal-authority bitmask: every question index 0..15 still unvoted.
const AUTHORITY: u64 = 0xffff;

struct ValarVoter {
    sk: anchor::SpendingKey,
    van_rand: pallas::Base,
}

struct Round {
    id: pallas::Base,
    ea_pk: pallas::Affine,
    anchor_height: u32,
    paths: Vec<anchor::AuthPath>,
}

/// Cast prover on valargroup's real `vote_proof` (voting-circuits 0.10.0 on the zakura
/// fork): VAN leaves -> vote commitment tree -> proposal anchor, then
/// `build_vote_proof_from_delegation` per ballot (16 ElGamal shares to the EA key).
/// `prepare` registers the stored blob's host verifier for `zkid`.
pub struct ValarVoteProver {
    pub zkid: u64,
    pub stored: StoredCircuit,
    pin: bool,
    voters: Vec<ValarVoter>,
    /// Test EA secret; a real EA key comes from the election authority.
    ea_sk: pallas::Scalar,
    round: Option<Round>,
    signers: Vec<([u8; 32], [u8; 32])>,
    times: Vec<std::time::Duration>,
    rng: rand::rngs::StdRng,
}

impl ValarVoteProver {
    pub fn new(zkid: u64) -> Result<Self> {
        let mut rng = seeded(0x7a6b_0002);
        let voters = (0..VOTERS)
            .map(|_| loop {
                let mut b = [0u8; 32];
                rand::Rng::fill_bytes(&mut rng, &mut b);
                if let Some(sk) = Option::from(anchor::SpendingKey::from_bytes(b)) {
                    break ValarVoter {
                        sk,
                        van_rand: pallas::Base::random(&mut rng),
                    };
                }
            })
            .collect();
        Ok(Self {
            zkid,
            stored: StoredCircuit::vote_proof()?,
            pin: false,
            voters,
            ea_sk: pallas::Scalar::random(&mut rng),
            round: None,
            signers: vec![],
            times: vec![],
            rng,
        })
    }
    /// Pin the 72-byte circuit key in the module (needs `CircuitInfo`, i.e. a live chain).
    pub fn pinned(mut self) -> Self {
        self.pin = true;
        self
    }
}

impl CastProver for ValarVoteProver {
    fn cast_zkid(&self) -> u64 {
        self.zkid
    }
    fn circuit_key(&self) -> Option<HexBinary> {
        self.pin
            .then(|| self.stored.circuit_key().ok())
            .flatten()
            .map(|k| HexBinary::from(k.to_vec()))
    }
    fn prepare(&mut self) -> Result<()> {
        cosmwasm_std::testing::register_test_circuit(
            self.zkid,
            "vote_proof",
            self.stored.host_verifier()?,
        );
        Ok(())
    }
    fn election(&mut self, round_id: [u8; 32]) -> Result<Election> {
        let id = Option::from(pallas::Base::from_repr(round_id))
            .ok_or_else(|| anyhow!("round_id is not a canonical Fp"))?;
        let leaves: Vec<_> = self
            .voters
            .iter()
            .map(|v| anchor::van_commitment(&v.sk, 0, WEIGHT_ZAT, id, AUTHORITY, v.van_rand))
            .collect();
        let (root, paths) = anchor::vote_comm_tree(&leaves);
        let ea_pk = (voting_circuits::spend_auth_g_affine() * self.ea_sk).to_affine();
        let c = ea_pk.coordinates().into_option().ok_or_else(|| anyhow!("EA identity"))?;
        let mut ea = [0u8; 64];
        ea[..32].copy_from_slice(&c.x().to_repr());
        ea[32..].copy_from_slice(&c.y().to_repr());
        self.round = Some(Round {
            id,
            ea_pk,
            anchor_height: 1,
            paths,
        });
        Ok(Election {
            ea_pk: ea,
            tree_root: root.to_repr(),
            anchor_height: 1,
        })
    }
    fn prove(&mut self, _p: &ZkProposal, voter: usize) -> Result<CastBundle> {
        let r = self.round.as_ref().ok_or_else(|| anyhow!("election() not called"))?;
        let v = self.voters.get(voter).ok_or_else(|| anyhow!("voter {voter} not in tree"))?;
        let alpha = pallas::Scalar::random(&mut self.rng);
        let t = std::time::Instant::now();
        let b = build_vote_proof_from_delegation(
            &v.sk,
            0,
            WEIGHT_ZAT,
            v.van_rand,
            r.id,
            r.paths[voter],
            voter as u32,
            r.anchor_height,
            zk::QUESTION_INDEX,
            (voter % 2) as u64,
            r.ea_pk,
            alpha,
            AUTHORITY,
            true,
        )
        .map_err(|e| anyhow!("vote_proof builder: {e:?}"))?;
        self.times.push(t.elapsed());
        self.signers
            .push((b.r_vpk_bytes, anchor::randomized_signing_key(&v.sk, alpha)));
        let instances: Vec<u8> = b
            .instance
            .to_halo2_instance()
            .iter()
            .flat_map(|x| x.to_repr())
            .collect();
        let ballot: Vec<u8> = b
            .encrypted_shares
            .iter()
            .flat_map(|s| [s.c1, s.c2].concat())
            .collect();
        Ok(CastBundle {
            proof: Binary::from(b.proof),
            instances,
            ballot: Binary::from(ballot),
        })
    }
    /// Same voter, same round: the VAN nullifier repeats.
    fn prove_replay(&mut self, p: &ZkProposal, voter: usize) -> Option<Result<CastBundle>> {
        Some(self.prove(p, voter))
    }
    fn sign(&mut self, b: &CastBundle, sighash: &[u8]) -> Result<HexBinary> {
        let rk = r_vpk(b);
        let rsk = self
            .signers
            .iter()
            .find(|(k, _)| *k == rk)
            .map(|(_, s)| *s)
            .ok_or_else(|| anyhow!("no signer for r_vpk"))?;
        sign_with(rsk, &mut self.rng, sighash)
    }
    fn prove_times(&self) -> &[std::time::Duration] {
        &self.times
    }
}

// --- test fallback: shape circuit ------------------------------------------------------

/// Wallet side for the shape circuit: RedPallas `r_vpk` keys per voter and synthetic
/// host-order public inputs (see SPEC.md slot table).
pub struct Voters {
    keys: Vec<(reddsa::SigningKey<reddsa::orchard::SpendAuth>, [u8; 32])>,
    rng: rand::rngs::StdRng,
}

impl Default for Voters {
    fn default() -> Self {
        Self {
            keys: vec![],
            rng: seeded(0x7a6b),
        }
    }
}

impl Voters {
    /// Instances for `voter` with VAN nullifier seed `nf` (equal seeds = same nullifier).
    pub fn instances(&mut self, p: &ZkProposal, voter: usize, nf: u8) -> Result<Vec<u8>> {
        while self.keys.len() <= voter {
            let sk = reddsa::SigningKey::<reddsa::orchard::SpendAuth>::new(&mut self.rng);
            let rk: [u8; 32] = reddsa::VerificationKey::from(&sk).into();
            self.keys.push((sk, rk));
        }
        let rk = self.keys[voter].1;
        let mut x = rk;
        x[31] &= 0x7f;
        let mut y = [0u8; 32];
        y[0] = rk[31] >> 7;
        let ea = p.ea_pk.as_slice();
        let s: Vec<[u8; 32]> = vec![
            zk::to_fe([nf; 32]),
            x,
            y,
            zk::to_fe([nf ^ 0x0f; 32]),
            zk::to_fe([nf ^ 0x55; 32]),
            p.tree_root.as_slice().try_into()?,
            zk::fe_u64(p.anchor_height),
            zk::fe_u64(zk::QUESTION_INDEX),
            p.round_id.as_slice().try_into()?,
            ea[..32].try_into()?,
            ea[32..].try_into()?,
        ];
        Ok(s.concat())
    }
    pub fn sign(&mut self, b: &CastBundle, sighash: &[u8]) -> Result<HexBinary> {
        let rk = r_vpk(b);
        let (sk, _) = self
            .keys
            .iter()
            .find(|(_, k)| *k == rk)
            .ok_or_else(|| anyhow!("no signer for r_vpk"))?;
        let sig: [u8; 64] = sk.sign(&mut self.rng, sighash).into();
        Ok(HexBinary::from(sig.to_vec()))
    }
}

/// Test fallback prover: shape-circuit keys from [`DevVoteKeys`] (stored-circuit blob in
/// the chain format), proofs from `create_proof`. Fast, but proves nothing about ballots.
pub struct DevVoteProver {
    pub zkid: u64,
    pub keys: DevVoteKeys,
    pin: bool,
    voters: Voters,
}

impl DevVoteProver {
    pub fn new(zkid: u64) -> Result<Self> {
        Ok(Self {
            zkid,
            keys: DevVoteKeys::load_or_generate()?,
            pin: false,
            voters: Voters::default(),
        })
    }
    /// Pin the 72-byte circuit key in the module (needs `CircuitInfo`, i.e. a live chain).
    pub fn pinned(mut self) -> Self {
        self.pin = true;
        self
    }
    fn bundle(&mut self, p: &ZkProposal, voter: usize, nf: u8) -> Result<CastBundle> {
        let instances = self.voters.instances(p, voter, nf)?;
        Ok(CastBundle {
            proof: Binary::from(self.keys.prove(&instances)?),
            instances,
            ballot: Binary::from(format!("elgamal-share-{nf}").into_bytes()),
        })
    }
}

impl CastProver for DevVoteProver {
    fn cast_zkid(&self) -> u64 {
        self.zkid
    }
    fn circuit_key(&self) -> Option<HexBinary> {
        self.pin
            .then(|| self.keys.stored.circuit_key().ok())
            .flatten()
            .map(|k| HexBinary::from(k.to_vec()))
    }
    fn prepare(&mut self) -> Result<()> {
        cosmwasm_std::testing::register_test_circuit(
            self.zkid,
            "vote_proof_dev",
            self.keys.stored.host_verifier()?,
        );
        Ok(())
    }
    fn prove(&mut self, p: &ZkProposal, voter: usize) -> Result<CastBundle> {
        self.bundle(p, voter, voter as u8 + 1)
    }
    fn prove_replay(&mut self, p: &ZkProposal, voter: usize) -> Option<Result<CastBundle>> {
        Some(self.bundle(p, voter + 100, 1))
    }
    fn sign(&mut self, b: &CastBundle, sighash: &[u8]) -> Result<HexBinary> {
        self.voters.sign(b, sighash)
    }
}

// --- deployment -------------------------------------------------------------------------

pub struct DaoDeployment<Chain: CwEnv> {
    pub core: DaoCore<Chain>,
    pub proposal_single: Addr,
    pub pre_propose: Addr,
    pub zk_code_id: u64,
}

/// Upload DAO DAO + cw-vote-ceremony and instantiate a cw4 DAO whose members are
/// `members` (weights 1, 2, 3, ...), funded with `treasury` from the default sender.
pub fn deploy_dao<Chain: CwEnv>(
    chain: &Chain,
    members: &[Addr],
    treasury: &[Coin],
) -> Result<DaoDeployment<Chain>> {
    let group = Cw4Group::new(chain.clone());
    let voting = DaoVotingCw4::new(chain.clone());
    let single = DaoProposalSingle::new(chain.clone());
    let pre = DaoPreProposeSingle::new(chain.clone());
    let core = DaoCore::new(chain.clone());
    let zkm = CwVoteCeremonyContract::new(chain.clone());
    for (n, r) in [
        ("cw4-group", group.upload().map(|_| ())),
        ("dao-voting-cw4", voting.upload().map(|_| ())),
        ("dao-proposal-single", single.upload().map(|_| ())),
        ("dao-pre-propose-single", pre.upload().map(|_| ())),
        ("dao-core", core.upload().map(|_| ())),
        ("cw-vote-ceremony", zkm.upload().map(|_| ())),
    ] {
        r.with_context(|| format!("upload {n}"))?;
    }
    let mi = |code_id: u64, msg: Binary, label: &str| ModuleInstantiateInfo {
        code_id,
        msg,
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: label.into(),
        salt: None,
    };
    let voting_info = mi(
        voting.code_id()?,
        bin(&dao_voting_cw4::msg::InstantiateMsg {
            group_contract: dao_voting_cw4::msg::GroupContract::New {
                cw4_group_code_id: group.code_id()?,
                cw4_group_salt: None,
                initial_members: members
                    .iter()
                    .enumerate()
                    .map(|(i, a)| cw4::Member {
                        addr: a.to_string(),
                        weight: i as u64 + 1,
                    })
                    .collect(),
            },
        })?,
        "voting module",
    );
    let single_info = mi(
        single.code_id()?,
        bin(&dao_proposal_single::msg::InstantiateMsg {
            threshold: Threshold::AbsolutePercentage {
                percentage: PercentageThreshold::Majority {},
            },
            max_voting_period: cw_utils::Duration::Height(1_000),
            min_voting_period: None,
            only_members_execute: true,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::ModuleMayPropose {
                info: mi(
                    pre.code_id()?,
                    bin(&dao_pre_propose_single::InstantiateMsg {
                        deposit_info: None,
                        submission_policy: PreProposeSubmissionPolicy::Specific {
                            dao_members: true,
                            allowlist: vec![],
                            denylist: vec![],
                        },
                        extension: Empty {},
                    })?,
                    "pre-propose single",
                ),
            },
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })?,
        "proposal single",
    );
    core.instantiate(
        &dao_interface::msg::InstantiateMsg {
            admin: None,
            name: "Privacy DAO".into(),
            description: "existing DAO that adds zk proposals".into(),
            image_url: None,
            automatically_add_cw20s: false,
            automatically_add_cw721s: false,
            voting_module_instantiate_info: voting_info,
            proposal_modules_instantiate_info: vec![single_info],
            initial_items: None,
            dao_uri: None,
            initial_actions: None,
        },
        None,
        treasury,
    )
    .context("instantiate dao-core")?;
    let mods: Vec<ProposalModule> = core.query(&dao_interface::msg::QueryMsg::ProposalModules {
        start_after: None,
        limit: None,
    })?;
    let proposal_single = mods
        .first()
        .ok_or_else(|| anyhow!("no proposal module"))?
        .address
        .clone();
    let policy: ProposalCreationPolicy = chain
        .wasm_querier()
        .smart_query(
            &proposal_single,
            &dao_proposal_single::msg::QueryMsg::ProposalCreationPolicy {},
        )
        .map_err(Into::into)?;
    let pre_propose = match policy {
        ProposalCreationPolicy::Module { addr } => addr,
        ProposalCreationPolicy::Anyone {} => return Err(anyhow!("expected a pre-propose module")),
    };
    Ok(DaoDeployment {
        core,
        proposal_single,
        pre_propose,
        zk_code_id: zkm.code_id()?,
    })
}

/// Governance step: members pass a proposal-single proposal that adds the zk module.
/// Returns the new module's address.
pub fn add_zk_module<Chain: CwEnv>(
    chain: &Chain,
    d: &DaoDeployment<Chain>,
    voters: &[Chain::Sender],
    init: ZkModuleInit,
) -> Result<Addr> {
    let before: Vec<ProposalModule> =
        d.core
            .query(&dao_interface::msg::QueryMsg::ActiveProposalModules {
                start_after: None,
                limit: None,
            })?;
    let add = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: d.core.address()?.to_string(),
        msg: bin(&dao_interface::msg::ExecuteMsg::UpdateProposalModules {
            to_add: vec![ModuleInstantiateInfo {
                code_id: d.zk_code_id,
                msg: bin(&ZkInit {
                    admin: None,
                    max_nullifiers_per_mark: 8,
                    zk_proposals: Some(init),
                })?,
                admin: Some(Admin::CoreModule {}),
                funds: None,
                label: "zk proposals".into(),
                salt: None,
            }],
            to_disable: vec![],
        })?,
        funds: vec![],
    });
    let pre = DaoPreProposeSingle::new(chain.clone());
    pre.set_address(&d.pre_propose);
    let single = DaoProposalSingle::new(chain.clone());
    single.set_address(&d.proposal_single);
    pre.call_as(&voters[0]).execute(
        &dao_pre_propose_single::ExecuteMsg::Propose {
            msg: dao_pre_propose_single::ProposeMessage::Propose {
                title: "Add zk proposals".into(),
                description: "cw-vote-ceremony zk proposal module".into(),
                msgs: vec![add],
                vote: None,
            },
        },
        &[],
    )?;
    let pid: u64 = single.query(&dao_proposal_single::msg::QueryMsg::ProposalCount {})?;
    for v in voters {
        single.call_as(v).execute(
            &dao_proposal_single::msg::ExecuteMsg::Vote {
                proposal_id: pid,
                vote: Vote::Yes,
                rationale: None,
            },
            &[],
        )?;
    }
    single.call_as(&voters[0]).execute(
        &dao_proposal_single::msg::ExecuteMsg::Execute { proposal_id: pid },
        &[],
    )?;
    let after: Vec<ProposalModule> =
        d.core
            .query(&dao_interface::msg::QueryMsg::ActiveProposalModules {
                start_after: None,
                limit: None,
            })?;
    ensure!(
        after.len() == before.len() + 1,
        "zk module not added alongside existing modules"
    );
    after
        .into_iter()
        .find(|m| before.iter().all(|b| b.address != m.address))
        .map(|m| m.address)
        .ok_or_else(|| anyhow!("new module not found"))
}

// --- module ---------------------------------------------------------------------------

/// zkid the in-process host serves the cast circuit under.
pub const CAST_ZKID: u64 = 7;
pub const VOTING_PERIOD: u64 = 30;
const ROOT: [u8; 32] = [0x11; 32];
const EA: [u8; 64] = [0x22; 64];
pub const PAYOUT: u128 = 250;

pub struct ZkVotingModule<Chain: CwEnv> {
    pub denom: String,
    /// `None`: choose [`ValarVoteProver`] on `MockBech32`, otherwise run DAO integration only.
    pub prover: Option<Box<dyn CastProver>>,
    pub dao: Option<DaoDeployment<Chain>>,
    pub zk_module: Option<Addr>,
    committee: Option<Group>,
}

impl<Chain: CwEnv> Default for ZkVotingModule<Chain> {
    fn default() -> Self {
        Self {
            denom: "uterp".into(),
            prover: None,
            dao: None,
            zk_module: None,
            committee: None,
        }
    }
}

impl<Chain: CwEnv> ZkVotingModule<Chain> {
    pub fn with_prover(prover: Box<dyn CastProver>) -> Self {
        Self {
            prover: Some(prover),
            ..Default::default()
        }
    }
}

impl<Chain: CwEnv + 'static> ProductModule<Chain> for ZkVotingModule<Chain> {
    fn id(&self) -> &'static str {
        "dao-zk-voting"
    }
    fn workstream(&self) -> Workstream {
        Workstream::A
    }
    fn chains(&self) -> &'static [ChainSlot] {
        &[ChainSlot::Terp]
    }
    fn deploy(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<()> {
        ensure!(ctx.operators.len() >= 3, "dao-zk-voting needs ≥3 operators");
        if self.prover.is_none() && super::is_mock(&ctx.terp) {
            self.prover = Some(Box::new(ValarVoteProver::new(CAST_ZKID)?));
        }
        if let Some(p) = self.prover.as_mut() {
            p.prepare()?;
        }
        super::fund_if_mock(ctx, &self.denom, 100_000);
        let members: Vec<Addr> = ctx.operators.iter().map(|o| o.addr.clone()).collect();
        let dao = deploy_dao(&ctx.terp, &members, &coins(1_000, &self.denom))?;
        ctx.terp.wait_blocks(1).map_err(Into::into)?;
        let committee = Group::dealer(3, 2)?;
        let voters: Vec<Chain::Sender> = ctx.operators.iter().map(|o| o.sender.clone()).collect();
        let zkm = add_zk_module(
            &ctx.terp,
            &dao,
            &voters,
            ZkModuleInit {
                dao: None,
                voting_module: None,
                cast_zkid: self
                    .prover
                    .as_ref()
                    .map(|p| p.cast_zkid())
                    .unwrap_or(CAST_ZKID),
                cast_circuit_key: self.prover.as_ref().and_then(|p| p.circuit_key()),
                anchor_registry: None,
                tally: TallyAuth::Committee {
                    group_vk: committee.vk()?,
                },
                voting_period: VOTING_PERIOD,
                threshold_bps: 5_000,
                quorum_bps: 3_000,
                require_rvpk_sig: true,
            },
        )?;
        self.dao = Some(dao);
        self.zk_module = Some(zkm);
        self.committee = Some(committee);
        Ok(())
    }

    fn exercise(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport> {
        let (dao, zkm, committee) = match (&self.dao, &self.zk_module, &self.committee) {
            (Some(d), Some(z), Some(c)) => (d, z.clone(), c),
            _ => return Err(anyhow!("not deployed")),
        };
        let chain = &ctx.terp;
        let m = CwVoteCeremonyContract::new(chain.clone());
        m.set_address(&zkm);
        let mut notes = vec![format!(
            "zk module {zkm} added to dao-core via proposal-single"
        )];
        let dao_of: Addr = m.query(&ZkQuery::Dao {})?;
        ensure!(dao_of == dao.core.address()?, "module dao");
        chain.wait_blocks(1).map_err(Into::into)?;

        // A member opens a private proposal paying from the treasury to operator 1.
        let recipient = ctx.operators[1].addr.clone();
        // The prover anchors its voters for the round id the module will derive.
        let next = m.query::<u64>(&ZkQuery::ProposalCount {})? + 1;
        let chain_id = chain.block_info().map_err(Into::into)?.chain_id;
        let round_id = zk::round_id(
            &cosmwasm_std::testing::MockApi::default(),
            &chain_id,
            zkm.as_str(),
            next,
        );
        let election = match self.prover.as_mut() {
            Some(p) => p.election(round_id)?,
            None => Election {
                ea_pk: EA,
                tree_root: ROOT,
                anchor_height: 1,
            },
        };
        let propose = ZkExec::ZkPropose {
            title: "Fund the ZEC corridor".into(),
            description: "private vote".into(),
            msgs: vec![CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
                to_address: recipient.to_string(),
                amount: coins(PAYOUT, &self.denom),
            })],
            ea_pk: HexBinary::from(election.ea_pk.to_vec()),
            anchor: AnchorRef::Root {
                root: HexBinary::from(election.tree_root.to_vec()),
                height: election.anchor_height,
            },
        };
        ensure!(
            m.execute(&propose, &[]).is_err(),
            "non-member must not propose"
        );
        m.call_as(&ctx.operators[2].sender).execute(&propose, &[])?;
        let id: u64 = m.query::<u64>(&ZkQuery::ProposalCount {})?;
        let p: ZkProposal = m.query(&ZkQuery::ZkProposal { proposal_id: id })?;
        ensure!(p.status == ZkStatus::Open, "open");
        ensure!(p.round_id.as_slice() == round_id, "round_id prediction");
        notes.push(format!("proposal {id} open, total power {}", p.total_power));

        let Some(prover) = self.prover.as_mut() else {
            return Ok(ProductReport {
                id: "dao-zk-voting",
                workstream: Workstream::A,
                outcome: Outcome::Skipped("no vote_proof prover/stored circuit for this chain (ZkVotingModule::with_prover)".into()),
                notes,
            });
        };

        // Anonymous ballots, relayed by the (non-member) default sender.
        let cast = |prover: &mut Box<dyn CastProver>, b: CastBundle| -> Result<()> {
            let sighash: HexBinary = m.query(&ZkQuery::ZkCastSighash {
                proposal_id: id,
                nullifier: HexBinary::from(&b.instances[..32]),
                vote_commitment: HexBinary::from(
                    &b.instances[slot::VOTE_COMMITMENT * 32..slot::VOTE_COMMITMENT * 32 + 32],
                ),
                ballot: b.ballot.clone(),
            })?;
            let sig = prover.sign(&b, sighash.as_slice())?;
            m.execute(
                &ZkExec::ZkCastVote {
                    proposal_id: id,
                    proof: b.proof,
                    instances: Binary::from(b.instances),
                    ballot: b.ballot,
                    rvpk_sig: Some(sig),
                },
                &[],
            )?;
            Ok(())
        };
        for v in 0..3 {
            let b = prover.prove(&p, v)?;
            cast(prover, b)?;
        }
        if let Some(r) = prover.prove_replay(&p, 0) {
            ensure!(
                cast(prover, r?).is_err(),
                "nullifier replay must be rejected"
            );
            notes.push("nullifier replay rejected".into());
        }
        // A proof does not transfer to other public inputs: swap the vote commitment of a
        // fresh ballot for another one and the verifier rejects it.
        let mut forged = prover.prove(&p, 3)?;
        let vc = slot::VOTE_COMMITMENT * 32..slot::VOTE_COMMITMENT * 32 + 32;
        let other = prover.prove(&p, 4)?.instances[vc.clone()].to_vec();
        forged.instances[vc].copy_from_slice(&other);
        ensure!(
            cast(prover, forged).is_err(),
            "proof over altered public inputs must be rejected"
        );
        notes.push("proof with an altered public input rejected".into());
        let p: ZkProposal = m.query(&ZkQuery::ZkProposal { proposal_id: id })?;
        ensure!(p.ballots == 3, "3 ballots");

        // EA committee (2-of-3) attests the decrypted tally after the window.
        let (yes, no, abstain) = (Uint128::new(5), Uint128::new(1), Uint128::zero());
        let digest: HexBinary = m.query(&ZkQuery::ZkTallyDigest {
            proposal_id: id,
            yes,
            no,
            abstain,
        })?;
        let sig = committee.sign(digest.as_slice(), &[1, 2])?;
        let tally = ZkExec::ZkSubmitTally {
            proposal_id: id,
            yes,
            no,
            abstain,
            attestation: TallyAttestation::Committee { signature: sig },
        };
        ensure!(
            m.execute(&tally, &[]).is_err(),
            "tally before expiry must fail"
        );
        chain.wait_seconds(VOTING_PERIOD + 1).map_err(Into::into)?;
        m.execute(&tally, &[])?;
        let p: ZkProposal = m.query(&ZkQuery::ZkProposal { proposal_id: id })?;
        ensure!(p.status == ZkStatus::Passed, "passed");

        // Anyone executes; dao-core pays out from the treasury.
        let bal = |a: &Addr| -> Result<Uint256> {
            Ok(chain
                .balance(a, Some(self.denom.clone()))
                .map_err(Into::into)?
                .first()
                .map(|c| c.amount)
                .unwrap_or_default())
        };
        let before = bal(&recipient)?;
        m.execute(&ZkExec::ZkExecute { proposal_id: id }, &[])?;
        ensure!(
            bal(&recipient)? - before == Uint256::from(PAYOUT),
            "treasury payout"
        );
        let p: ZkProposal = m.query(&ZkQuery::ZkProposal { proposal_id: id })?;
        ensure!(p.status == ZkStatus::Executed, "executed");
        notes.push(format!(
            "3 anonymous ballots, EA tally {yes}/{no}/{abstain}, executed payout {PAYOUT}"
        ));

        // proposal-single still governs.
        let single = DaoProposalSingle::new(chain.clone());
        single.set_address(&dao.proposal_single);
        let n0: u64 = single.query(&dao_proposal_single::msg::QueryMsg::ProposalCount {})?;
        let pre = DaoPreProposeSingle::new(chain.clone());
        pre.set_address(&dao.pre_propose);
        pre.call_as(&ctx.operators[0].sender).execute(
            &dao_pre_propose_single::ExecuteMsg::Propose {
                msg: dao_pre_propose_single::ProposeMessage::Propose {
                    title: "still works".into(),
                    description: String::new(),
                    msgs: vec![],
                    vote: None,
                },
            },
            &[],
        )?;
        let n1: u64 = single.query(&dao_proposal_single::msg::QueryMsg::ProposalCount {})?;
        ensure!(n1 == n0 + 1, "proposal-single still works");
        Ok(ProductReport {
            id: "dao-zk-voting",
            workstream: Workstream::A,
            outcome: Outcome::Passed,
            notes,
        })
    }
}
