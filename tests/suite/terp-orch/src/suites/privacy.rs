//! Privacy-products unified suite (`--features privacy`).
//!
//! One run deploys and exercises every privacy product on a single cw-orch
//! environment: `MockBech32` offline, or a `Daemon` on an ict-rs Terp chain
//! (`tests/privacy_products_e2e.rs`, `--features privacy,ict -- --ignored`).
//!
//! Every product is a [`ProductModule`]. Workstream B owns `lc-registry`,
//! `seams-l0` and `bridge-tacit`; workstream A plugs `auctions-frost` and
//! `dao-zk-voting` into the slots registered in [`PrivacySuite::standard`].
//! Design: `~/grok-bot/privacy-products/e2e/SUITE.md` (groot-wan).
use anyhow::{anyhow, ensure, Context, Result};
use cosmwasm_std::Binary;
use cw_lc_registry::interface::LcRegistry;
use cw_lc_registry::msg::{
    ClientKind, ConsensusTipResponse, ExecuteMsg as LcExec, InstantiateMsg as LcInit, NodeState,
    PowAlgo, QueryMsg as LcQuery, TargetChain,
};
use cw_orch::prelude::*;
use ed25519_dalek::{Signer, SigningKey};

/// Workstream A: `auctions-frost` (svg-auction private bids) and `dao-zk-voting`.
pub mod auctions;
pub mod bid_view;
pub mod frost_util;
pub mod vote_circuit;
pub mod zk_voting;
pub use auctions::PrivateAuctionModule;
pub use zk_voting::ZkVotingModule;

/// True when the suite runs on the offline `MockBech32` chain.
pub fn is_mock<Chain: CwEnv + 'static>(chain: &Chain) -> bool {
    (chain as &dyn std::any::Any).is::<MockBech32>()
}

/// Offline only: mint `amount` of `denom` to every operator and the default sender
/// (live operators are funded by the harness).
pub fn fund_if_mock<Chain: CwEnv + 'static>(ctx: &SuiteCtx<Chain>, denom: &str, amount: u128) {
    if let Some(m) = (&ctx.terp as &dyn std::any::Any).downcast_ref::<MockBech32>() {
        let mut addrs: Vec<Addr> = ctx.operators.iter().map(|o| o.addr.clone()).collect();
        addrs.push(m.sender_addr());
        for a in addrs {
            let _ = m.add_balance(&a, vec![Coin::new(amount, denom)]);
        }
    }
}

/// Which workstream owns a module (A: auctions + zk-voting, B: LC/bridge/seams).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workstream {
    A,
    B,
}

/// Chains the harness may spawn. The live test spawns the union of what the
/// registered modules ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChainSlot {
    Terp,
    Juno,
    Osmosis,
    /// Zakura (Zcash) regtest / crosslink node(s) via ict-rs `zakura` feature.
    Zakura,
    /// Anvil (EVM) via ict-rs `ethereum` feature; tacit bridge mock.
    Anvil,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    /// Slot registered, implementation not landed yet.
    Pending(String),
    /// Implemented, but a required chain/service is not available in this run.
    Skipped(String),
}

#[derive(Clone, Debug)]
pub struct ProductReport {
    pub id: &'static str,
    pub workstream: Workstream,
    pub outcome: Outcome,
    pub notes: Vec<String>,
}

/// Source of internal zakura node state (`getblockchaininfo`-shaped) for an operator.
pub type NodeStateSource = Box<dyn FnMut(usize) -> Result<NodeState>>;

/// One operator identity: cw-orch sender + its Ed25519 attestation key.
pub struct OperatorId<Chain: TxHandler> {
    pub sender: <Chain as TxHandler>::Sender,
    pub addr: Addr,
    pub key: SigningKey,
}

/// Shared run context.
pub struct SuiteCtx<Chain: CwEnv> {
    pub terp: Chain,
    /// ≥ 3 operator identities (attestation services).
    pub operators: Vec<OperatorId<Chain>>,
    /// Internal node state per operator index. Offline: synthetic regtest.
    pub node_state: NodeStateSource,
    /// Live endpoints, when the harness spawned them (127.0.0.1 only).
    pub zakura_rpc: Option<String>,
    pub anvil_rpc: Option<String>,
    pub notes: Vec<String>,
}

pub trait ProductModule<Chain: CwEnv> {
    fn id(&self) -> &'static str;
    fn workstream(&self) -> Workstream;
    fn chains(&self) -> &'static [ChainSlot];
    /// Upload + instantiate. Pending modules return Ok(()).
    fn deploy(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<()>;
    /// Exercise the product flow end to end and report.
    fn exercise(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport>;
}

/// Placeholder for a product another workstream will land.
pub struct PendingSlot {
    pub id: &'static str,
    pub workstream: Workstream,
    pub chains: &'static [ChainSlot],
    pub reason: &'static str,
}

impl<Chain: CwEnv> ProductModule<Chain> for PendingSlot {
    fn id(&self) -> &'static str {
        self.id
    }
    fn workstream(&self) -> Workstream {
        self.workstream
    }
    fn chains(&self) -> &'static [ChainSlot] {
        self.chains
    }
    fn deploy(&mut self, _ctx: &mut SuiteCtx<Chain>) -> Result<()> {
        Ok(())
    }
    fn exercise(&mut self, _ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport> {
        Ok(ProductReport {
            id: self.id,
            workstream: self.workstream,
            outcome: Outcome::Pending(self.reason.into()),
            notes: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// lc-registry (workstream B): cw-lc-registry operators, client, signed state
// ---------------------------------------------------------------------------

pub const LC_REGISTRY_LABEL: &str = "terp/lc/registry/zcash-regtest/v1";
pub const LC_CLIENT_ID: &str = "zcash-regtest-0";

pub struct LcRegistryModule<Chain: CwEnv> {
    pub contract: Option<LcRegistry<Chain>>,
    pub min_attestations: u32,
}

impl<Chain: CwEnv> Default for LcRegistryModule<Chain> {
    fn default() -> Self {
        Self {
            contract: None,
            min_attestations: 2,
        }
    }
}

impl<Chain: CwEnv> ProductModule<Chain> for LcRegistryModule<Chain> {
    fn id(&self) -> &'static str {
        "lc-registry"
    }
    fn workstream(&self) -> Workstream {
        Workstream::B
    }
    fn chains(&self) -> &'static [ChainSlot] {
        &[ChainSlot::Terp, ChainSlot::Zakura]
    }
    fn deploy(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<()> {
        let c = LcRegistry::new("privacy:lc-registry", ctx.terp.clone());
        c.upload().context("upload cw-lc-registry")?;
        c.instantiate(
            &LcInit {
                admin: None,
                label: LC_REGISTRY_LABEL.into(),
                // hm-vext capability quorum; not required so the suite runs
                // on chains without x/hashmerchant (verdict = Unavailable).
                hash_market_capability: Some(cw_lc_registry::quorum::HashMarketCapability {
                    chain_uid: "zcash-regtest".into(),
                    algo: "block-hash".into(),
                    require: false,
                }),
                max_report_age_blocks: 100,
            },
            None,
            &[],
        )
        .context("instantiate cw-lc-registry")?;
        self.contract = Some(c);
        Ok(())
    }
    fn exercise(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport> {
        let c = self
            .contract
            .as_ref()
            .ok_or_else(|| anyhow!("not deployed"))?;
        ensure!(ctx.operators.len() >= 3, "lc-registry needs ≥3 operators");
        let mut notes = vec![];
        for (i, op) in ctx.operators.iter().enumerate() {
            c.call_as(&op.sender).execute(
                &LcExec::RegisterOperator {
                    peer_id: format!("zakura-p2pv2-op{i}"),
                    endpoint: format!("/ip4/127.0.0.1/tcp/{}", 18233 + i),
                    pubkey: Binary::from(op.key.verifying_key().to_bytes().to_vec()),
                    capabilities: vec!["pow-headers".into(), "node-state".into()],
                },
                &[],
            )?;
        }
        c.execute(
            &LcExec::RegisterClient {
                client_id: LC_CLIENT_ID.into(),
                kind: ClientKind::Permissionless,
                target: TargetChain {
                    chain_id: "zcash-regtest".into(),
                    algo: PowAlgo::Equihash { n: 48, k: 5 },
                    target_block_time_secs: 25,
                },
                attestors: ctx.operators.iter().map(|o| o.addr.to_string()).collect(),
                min_attestations: self.min_attestations,
                ibc_client_id: None,
            },
            &[],
        )?;
        for i in 0..ctx.operators.len() {
            let state = (ctx.node_state)(i)?;
            let op = &ctx.operators[i];
            let digest: Binary = c.query(&LcQuery::ReportDigest {
                client_id: LC_CLIENT_ID.into(),
                operator: op.addr.to_string(),
                state: state.clone(),
            })?;
            let sig = op.key.sign(digest.as_slice()).to_bytes().to_vec();
            c.call_as(&op.sender).execute(
                &LcExec::ReportNodeState {
                    client_id: LC_CLIENT_ID.into(),
                    state,
                    signature: Binary::from(sig),
                },
                &[],
            )?;
        }
        let tip: ConsensusTipResponse = c.query(&LcQuery::ConsensusTip {
            client_id: LC_CLIENT_ID.into(),
        })?;
        let agreed = tip
            .tip
            .ok_or_else(|| anyhow!("no {}-of-n agreed tip", self.min_attestations))?;
        notes.push(format!(
            "agreed tip h={} hash={} by {} attestors (fresh={})",
            agreed.height,
            &agreed.hash[..16],
            agreed.attestors.len(),
            tip.fresh_reports
        ));
        Ok(ProductReport {
            id: "lc-registry",
            workstream: Workstream::B,
            outcome: Outcome::Passed,
            notes,
        })
    }
}

/// Offline node-state source: every operator sees the same regtest tip.
pub fn synthetic_regtest_state() -> NodeStateSource {
    let mut t = 0u64;
    Box::new(move |_i| {
        t += 1;
        Ok(NodeState {
            tip_height: 120,
            tip_hash: "ab".repeat(32),
            prev_hash: "aa".repeat(32),
            chain_work: "0000000000000000000000000000000000000000000000000000000000f00d".into(),
            finalized_height: Some(110),
            peer_count: 3,
            observed_at_unix: 1_800_000_000 + t,
        })
    })
}

// ---------------------------------------------------------------------------
// seams-l0 (workstream B): recovered pure seams (Omni LP + FROST escrow release)
// ---------------------------------------------------------------------------

/// The nine demo pairs (base, quote). Chain deployment per pair is tracked in
/// `~/grok-bot/privacy-products/seams/RECOVERY.md`.
pub const SEAM_PAIRS: [(&str, &str); 9] = [
    ("zec", "btc"),
    ("zec", "um"),
    ("um", "btc"),
    ("terp", "btc"),
    ("terp", "um"),
    ("terp", "zec"),
    ("thiol", "zec"),
    ("thiol", "btc"),
    ("thiol", "um"),
];

#[derive(Default)]
pub struct SeamsL0Module;

impl<Chain: CwEnv> ProductModule<Chain> for SeamsL0Module {
    fn id(&self) -> &'static str {
        "seams-l0"
    }
    fn workstream(&self) -> Workstream {
        Workstream::B
    }
    fn chains(&self) -> &'static [ChainSlot] {
        &[ChainSlot::Terp]
    }
    fn deploy(&mut self, _ctx: &mut SuiteCtx<Chain>) -> Result<()> {
        Ok(())
    }
    fn exercise(&mut self, _ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport> {
        use terp_seams::dex::{
            asset_id_b, asset_id_hub, authorize_escrow_release, bootstrap_omni_liquidity_pure,
            DualHomePrepV0, EgressBurnEvidenceV0, ThresholdEscrowAuth, MODE_FROST_ESCROW_RELEASE,
        };
        let prep = DualHomePrepV0::omni_fixture();
        let (pool, _, liability, receipt, _btc, zec) = bootstrap_omni_liquidity_pure(
            asset_id_hub(),
            asset_id_b(),
            &prep,
            [0x4c; 32],
            [0xb1; 32],
            [0xb2; 32],
            1,
            997,
            1000,
            "mock_verify_lab",
        )
        .map_err(|e| anyhow!("omni bootstrap: {e:?}"))?;
        receipt
            .assert_reserves_match_notes()
            .map_err(|e| anyhow!("reserves vs notes: {e:?}"))?;
        liability
            .check_invariant()
            .map_err(|e| anyhow!("escrow liability: {e:?}"))?;

        // FROST FE01 escrow release over a synthetic burn of the LP ZEC note.
        let dest = [0xd1u8; 32];
        let evidence = EgressBurnEvidenceV0 {
            terp_tx_hash: Some("suite".into()),
            burn: terp_seams::dex::EgressBurnPublic {
                nullifier: [0x5a; 32],
                asset_id: zec.asset_id,
                value: 1_000,
                dest_commitment: dest,
                dest_kind: terp_seams::dex::DestKind::Shielded,
                cm_spent: [0x33; 32],
                root: [0u8; 32],
                source_pool_id: Some(1),
            },
            proof_mode: "mock_verify_lab".into(),
            settle_receipt_ref: None,
        };
        let committee =
            frost_escrow::FrostLabCommittee::dkg(2, 3).map_err(|e| anyhow!("frost dkg: {e}"))?;
        let digest = threshold_committee::escrow_release_digest(
            &evidence.burn.nullifier,
            &evidence.burn.dest_commitment,
            evidence.burn.value,
            &evidence.burn.asset_id,
        );
        let sig = committee
            .sign_message(&digest)
            .map_err(|e| anyhow!("frost sign: {e}"))?;
        let receipt_zec = authorize_escrow_release(
            &evidence,
            &dest,
            &ThresholdEscrowAuth {
                committee_pk: committee.encoded_public.clone(),
                combined_sig: sig,
            },
        )
        .map_err(|e| anyhow!("escrow release: {e:?}"))?;
        ensure!(receipt_zec.mode == MODE_FROST_ESCROW_RELEASE, "mode");
        Ok(ProductReport {
            id: "seams-l0",
            workstream: Workstream::B,
            outcome: Outcome::Passed,
            notes: vec![
                format!("omni pool r_a={} r_b={}", pool.r_a, pool.r_b),
                "FROST 2-of-3 FE01 escrow release authorized".into(),
                format!(
                    "pairs catalogued: {} (on-chain per-pair pools: pending)",
                    SEAM_PAIRS.len()
                ),
            ],
        })
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub struct PrivacySuite<Chain: CwEnv> {
    pub modules: Vec<Box<dyn ProductModule<Chain>>>,
}

impl<Chain: CwEnv + 'static> PrivacySuite<Chain> {
    /// All products, in launch order (auctions, zk-voting, bridge, seams) plus
    /// the light-client registry they depend on.
    pub fn standard() -> Self {
        let modules: Vec<Box<dyn ProductModule<Chain>>> = vec![
            Box::new(LcRegistryModule::<Chain>::default()),
            Box::new(PrivateAuctionModule::<Chain>::default()),
            Box::new(ZkVotingModule::<Chain>::default()),
            Box::new(PendingSlot {
                id: "bridge-fcr",
                workstream: Workstream::B,
                chains: &[ChainSlot::Terp, ChainSlot::Anvil],
                reason: "workstream B: FCR-powered bridge (team draft) is next; tacit on hold (bridge/SPEC.md)",
            }),
            Box::new(SeamsL0Module),
        ];
        Self { modules }
    }

    pub fn required_chains(&self) -> Vec<ChainSlot> {
        let mut v: Vec<ChainSlot> = self
            .modules
            .iter()
            .flat_map(|m| m.chains().iter().copied())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Deploy everything, then exercise everything. A module error becomes a
    /// failed report (the run continues so one product cannot hide another).
    pub fn run(&mut self, ctx: &mut SuiteCtx<Chain>) -> Vec<Result<ProductReport>> {
        let mut deployed = vec![];
        for m in self.modules.iter_mut() {
            deployed.push(m.deploy(ctx).with_context(|| format!("deploy {}", m.id())));
        }
        self.modules
            .iter_mut()
            .zip(deployed)
            .map(|(m, d)| {
                d?;
                m.exercise(ctx)
                    .with_context(|| format!("exercise {}", m.id()))
            })
            .collect()
    }
}

/// Offline context on `MockBech32` with three operators.
pub fn mock_ctx(chain: MockBech32) -> SuiteCtx<MockBech32> {
    let operators = (0..3u8)
        .map(|i| {
            let addr = chain.addr_make(format!("zakura-op-{i}"));
            OperatorId {
                sender: addr.clone(),
                addr,
                key: SigningKey::from_bytes(&[0x40 + i; 32]),
            }
        })
        .collect();
    SuiteCtx {
        terp: chain,
        operators,
        node_state: synthetic_regtest_state(),
        zakura_rpc: None,
        anvil_rpc: None,
        notes: vec![],
    }
}
