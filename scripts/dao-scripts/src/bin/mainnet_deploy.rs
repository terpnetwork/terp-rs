//! Phased, resumable cw-orch deploy runtime for Terp mainnet (morocco-1).
//!
//! ```sh
//! cargo run -p dao-scripts --bin mainnet_deploy -- preflight          # dry-run
//! cargo run -p dao-scripts --bin mainnet_deploy -- all                # dry-run plan for every phase
//! TERP_SCRIPT_BROADCAST=1 cargo run -p dao-scripts --bin mainnet_deploy -- \
//!     --chain-id morocco-1 --broadcast --only upload                  # [APPROVAL]
//! ```
//!
//! Phases (each a subcommand; `all` runs them in order, `--from` / `--only` select):
//! 0 preflight  artifact checksums, chain-id, broadcast gate, plan
//! 1 upload     batch store every code later phases use; skip codes already on chain
//! 2 daos       FIAB + liquidity-steward Foundation proposals, submitted and voted by a Foundation
//!              member through the cw-orch-kms session (MsgExec as the member; no terpd)
//! 3 oracles    registry -> aggregator -> twap-probe
//! 4 nftfi      marketplace, reserve-auction, svg-auction, raffle, nft-loan
//! 5 authenticators  passkey + recovery: upload only (code ids in state.json); --smoke is local-test only
//! 6 svg        svg-minter factory owned by the kms granter (or --svg-owner), series, drops
//! 7 verify     admins vs the Foundation, deploy report in out/mainnet-deploy/
//!
//! Signing: only through cw-orch-kms (one authz+feegrant session per phase, scoped to
//! exactly the stores / instantiate2 / executes still to do; the AuthzSession revokes on
//! close and on Drop). Without `--broadcast` + `TERP_SCRIPT_BROADCAST=1` nothing is signed;
//! each phase writes its plan to out/mainnet-deploy/plan-<phase>.json.
//! `--signer local-test` exists for rehearsals only: it is refused unless the gRPC endpoint
//! is 127.0.0.1/localhost and the chain-id is not morocco-1 / 120u-1. It signs with an
//! in-process ephemeral key instead of a cold-key authz session.
//!
//! State: every tx result is written to `--state` (default out/mainnet-deploy/state.json)
//! immediately; reruns skip finished work, so a second `all` sends no transactions.
//! No migrate entrypoints are touched or added.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use cosmwasm_std::{Addr, Binary, Timestamp};
use cw_infuser_scripts::collection;
use cw_infuser_scripts::drops::{self, KARA_GIPPY, MARAIN_ORBS, YIJING_HARMONY};
use cw_orch::daemon::{CosmTxResponse, TxSender};
use cw_orch::environment::{ChainKind, WasmQuerier};
use cw_orch::prelude::*;
use cw_orch_kms::{
    ApprovalSession, AuthzSession, ScopedCall, SigningRequest, enable_session, instantiate2_bech32,
    planned_spend_limit, wasm_checksum,
};
use dao_foundation_suite::ibc_ops::PARENT_CORE;
use dao_scripts::kms_gate::{self, BroadcastRequest, Scope};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PHASES: &[&str] = &[
    "preflight",
    "upload",
    "daos",
    "oracles",
    "nftfi",
    "authenticators",
    "svg",
    "verify",
];
/// Run only by name (`polytone`), never by `all`: needs a live relayer + connection.
const EXTRA_PHASES: &[&str] = &[
    "polytone",
    "dao-upload",
    "bounty-upload",
    "hs-upload",
    "vote",
    "dao-propose",
    "dao-vote",
    "dao-execute",
    "circuit-propose",
    "gov-propose",
    "gov-vote",
    "chunk-upload",
];

/// Already-instantiated morocco-1 code ids superseded by the fresh build (no migrate, by design).
/// Recorded as `previous` so the report shows old vs new.
const PREVIOUS_CODES: &[(&str, u64)] = &[("passkey", 84), ("recovery", 85)];

/// (logical name, artifact file). Every code a later phase instantiates.
const CODES: &[(&str, &str)] = &[
    ("oracle-registry", "cw_oracle_registry.wasm"),
    ("oracle-aggregator", "cw_oracle_aggregator.wasm"),
    ("oracle-twap-probe", "cw_oracle_twap_probe.wasm"),
    ("marketplace", "revenue_marketplace.wasm"),
    ("reserve-auction", "revenue_reserve_auction.wasm"),
    ("svg-auction", "svg_auction.wasm"),
    ("raffle", "nftfi_raffle.wasm"),
    ("nft-loan", "nftfi_loan.wasm"),
    ("passkey", "terp_passkey.wasm"),
    ("recovery", "terp_recovery.wasm"),
    ("cw721-svg", "cw721_svg.wasm"),
    ("svg-minter", "cw_svg_minter.wasm"),
];

/// DAO DAO subset re-stored from crates/dao-contracts/artifacts (paths are relative to the
/// terp-rs artifacts dir): audited rewards-distributor and vote-delegation fixes, dao-calendar
/// and gauge-adapter (gauge-orchestrator fails arm64 compile; later). Only the `dao-upload` phase stores these.
const DAO_CODES: &[(&str, &str)] = &[
    (
        "dao-rewards-distributor",
        "../../dao-contracts/artifacts/dao_rewards_distributor.wasm",
    ),
    (
        "dao-vote-delegation",
        "../../dao-contracts/artifacts/dao_vote_delegation.wasm",
    ),
    (
        "dao-calendar",
        "../../dao-contracts/artifacts/dao_calendar.wasm",
    ),
    (
        "gauge-adapter",
        "../../dao-contracts/artifacts/gauge_adapter.wasm",
    ),
];

/// Bounty escrow (contracts/revenue/bounty-escrow), mainnet build: `--no-default-features`
/// (no dev-chain-guard), optimizer terpnetwork/optimizer-arm64:0.17.0-rust1.91, written to
/// artifacts/mainnet/ so the guarded dev artifact in artifacts/ is untouched. It keeps its
/// `migrate` entrypoint: an explicit, owner-approved exception (2026-10-27). Only the
/// `bounty-upload` phase stores it; nothing here instantiates it.
const BOUNTY_CODES: &[(&str, &str)] = &[("bounty-escrow", "mainnet/terp_bounty_escrow.wasm")];

/// Headstash v0.0.1 (cw-headstash + its frontend), hermetic arm64 builds (terpnetwork/optimizer-arm64:0.17.0-rust1.91,
/// src 4de39b1 / 129fe5e), written to artifacts/mainnet/. Plain MsgStoreCode (code upload is Everybody on morocco-1):
/// no x/gov and no DAO proposal. Instantiation is the Foundation DAO's (dao-flow prop 1); only the vk_id 3 circuit goes
/// through x/gov (phase-b/12). Only the `hs-upload` phase stores these, and only when each file matches its pinned sha256.
const HS_CODES: &[(&str, &str, &str)] = &[
    (
        "cw-headstash",
        "mainnet/cw_headstash.wasm",
        "dd40ded55eb436f296fa8eac0d1794f69c5c7369f5a137b1b52dc2cb18e22c52",
    ),
    (
        "cw-headstash-frontend",
        "mainnet/cw_headstash_frontend.wasm",
        "e72eee9b90012ce42fed2c0abd0b1fcc31b868168fa79d30d2f9d6fdd616cac2",
    ),
];

/// Codes already on morocco-1 that this runtime reuses and never re-stores.
const EXTERNAL_CODES: &[(&str, u64)] = &[
    ("polytone-note", 78),
    ("polytone-voice", 79),
    ("polytone-proxy", 80),
];

/// Contracts deliberately left without a wasm admin.
const ADMINLESS: &[&str] = &["smoke-recovery"];

const MARAIN_SERIES: &str = "marain-orbs";
const BENDERS_SERIES: &str = "latent-benders";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum SignerKind {
    /// cw-orch-kms authz session approved by the cold key on a random loopback approval page (`CW_ORCH_KMS_PORT` pins it).
    Kms,
    /// Rehearsal only: in-process ephemeral key on a 127.0.0.1 chain.
    LocalTest,
}

#[derive(Parser, Debug)]
#[command(name = "mainnet_deploy")]
struct Cli {
    /// preflight|upload|daos|oracles|nftfi|authenticators|svg|verify|all
    phase: String,
    /// With `all`: start at this phase.
    #[arg(long)]
    from: Option<String>,
    /// With `all`: run only this phase.
    #[arg(long)]
    only: Option<String>,
    /// Keep only these step names (comma-separated), e.g. `--steps nft-loan`.
    #[arg(long = "steps", value_delimiter = ',')]
    steps_filter: Vec<String>,
    /// oracles: instantiate the registry with open_submit=false (admin + SetRelayer allow-list).
    #[arg(long)]
    registry_closed: bool,
    /// oracles: polytone note allowed to deliver cross-chain Callback prices to the registry.
    #[arg(long)]
    registry_note: Option<String>,
    /// Wasm admin / fee address / final owner. Foundation DAO by default.
    #[arg(long, default_value = PARENT_CORE)]
    admin: String,
    #[arg(long, default_value = "out/mainnet-deploy/state.json")]
    state: PathBuf,
    #[arg(long, default_value = "out/mainnet-deploy")]
    out: PathBuf,
    /// Sign and broadcast. Also needs TERP_SCRIPT_BROADCAST=1 (and
    /// TERP_SCRIPT_BROADCAST_CHAIN=<id> off morocco-1) and a matching --chain-id.
    #[arg(long)]
    broadcast: bool,
    /// Operator-confirmed chain id; must equal the configured chain.
    #[arg(long, default_value = "")]
    chain_id: String,
    /// Override the chain: gRPC endpoint (default morocco_info()).
    #[arg(long)]
    grpc: Option<String>,
    /// CometBFT RPC fallback for broadcast (comma-separated). Default: env MOROCCO_RPC on morocco-1.
    #[arg(long)]
    rpc: Option<String>,
    /// LCD/REST fallback for broadcast, after --rpc. Default: env MOROCCO_LCD on morocco-1.
    #[arg(long)]
    lcd: Option<String>,
    /// Chain id when --grpc points at another chain.
    #[arg(long, default_value = "morocco-1")]
    network: String,
    #[arg(long, default_value = "uthiol")]
    gas_denom: String,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    #[arg(long, value_enum, default_value_t = SignerKind::Kms)]
    signer: SignerKind,
    /// Cold key; empty = whichever key signs the granter challenge.
    #[arg(long, env = cw_orch_kms::CW_ORCH_GRANTER_ENV, default_value = "")]
    granter: String,
    #[arg(long, default_value_t = 0)]
    account_index: u32,
    /// Final svg-minter factory owner and series operator. Default: the kms granter
    /// (never the ephemeral session key). If it differs from the granter, the granter
    /// sets up the factory and offers ownership to this address (it must accept).
    #[arg(long)]
    svg_owner: Option<String>,
    /// Curator keys (public addresses) per drop: cw721 creator owner and series operator (the
    /// daily runtime signer, see svg_curator). Withdraw/royalties go to --admin. The factory makes
    /// the creator the wasm admin; `svg_curator handoff` then moves it to --admin (migration only).
    /// --creator-yijing defaults to --admin (no daily runtime: no handoff needed).
    #[arg(long, env = "SVG_CREATOR_MARAIN")]
    creator_marain: Option<String>,
    #[arg(long, env = "SVG_CREATOR_BENDERS")]
    creator_benders: Option<String>,
    #[arg(long, env = "SVG_CREATOR_YIJING")]
    creator_yijing: Option<String>,
    #[arg(long, default_value_t = 3600)]
    ttl_secs: u64,
    /// Keep grants after a phase (default: revoke).
    #[arg(long)]
    retain: bool,
    /// Fee denom for market / auction configs.
    #[arg(long, default_value = "uthiol")]
    market_denom: String,
    /// twap-probe `mid` (open decision, see runbook).
    #[arg(long, default_value = "terp-osmo")]
    twap_mid: String,
    /// raffle / nft-loan fee rate (Decimal). 4%.
    #[arg(long, default_value = "0.04")]
    nftfi_fee: String,
    /// marketplace / svg-auction protocol fee in bps (reserve-auction gets the same as a Decimal). 4%.
    #[arg(long, default_value_t = 400)]
    fee_bps: u64,
    /// Foundation pre-propose module (members submit here).
    #[arg(long, default_value = dao_foundation_suite::ibc_ops::PRE_PROPOSE_A)]
    foundation_pre_propose: String,
    /// dao-dao-core checksum hex for the steward instantiate2 predictions (default: query CODE_CORE).
    #[arg(long)]
    dao_core_checksum: Option<String>,
    /// steward-accounts.json (default: repo root).
    #[arg(long)]
    steward_accounts: Option<PathBuf>,
    /// Steward/custody DAO parent. Default: the x/gov module account queried from the chain.
    #[arg(long)]
    gov_parent: Option<String>,
    /// Submit the Foundation proposals but do not cast the member's yes vote.
    #[arg(long)]
    no_vote: bool,
    /// yes | no | abstain. `vote`/`dao-vote`: default yes. `dao-propose`/`dao-execute`: omitted = no vote.
    #[arg(long = "vote")]
    vote_choice: Option<String>,
    /// vote phase: Foundation proposal ids, e.g. `9,10`.
    #[arg(long = "proposals", value_delimiter = ',')]
    vote_proposals: Vec<u64>,
    /// vote phase: after voting, execute every listed proposal whose status is `passed`.
    #[arg(long = "execute")]
    vote_execute: bool,
    /// dao-propose phase: rendered pre-propose `{"propose":{...}}` body (e.g. 20-dao-prop1-deploy.propose.json).
    #[arg(long)]
    propose_file: Option<PathBuf>,
    /// dao-propose phase: required sha256 (hex) of --propose-file; a mismatch refuses before any grant.
    #[arg(long)]
    propose_sha: Option<String>,
    /// generic DAO phases: dao-dao-core address; proposal module + pre-propose are resolved from chain.
    #[arg(long)]
    dao: Option<String>,
    /// generic DAO phases: proposal module override (default: first enabled module of --dao).
    #[arg(long)]
    proposal_module: Option<String>,
    /// generic DAO phases: pre-propose override (default: the module's proposal_creation_policy).
    #[arg(long)]
    pre_propose: Option<String>,
    /// dao-propose: JSON array of CosmosMsg (instead of --propose-file); needs --title. --propose-sha pins it if given.
    #[arg(long)]
    msgs_file: Option<PathBuf>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, default_value = "")]
    description: String,
    /// dao-propose: an existing contract that a proposal with instantiate2 msgs may also execute
    /// (repeatable). Refused unless it exists on chain with admin or cw-ownable owner == the DAO core.
    #[arg(long = "allow-target")]
    allow_target: Vec<String>,
    /// chunk-upload: JSON plan {"contract"?, "version", "files":[{"path"?, "chunks":[base64]}]}.
    #[arg(long)]
    chunks_file: Option<PathBuf>,
    /// chunk-upload: sha256 pin of --chunks-file; a mismatch refuses before any grant.
    #[arg(long)]
    chunks_sha: Option<String>,
    /// chunk-upload: frontend (cw-headstash-frontend) contract; overrides the plan's "contract".
    #[arg(long = "frontend")]
    frontend_addr: Option<String>,
    /// dao-propose --execute: poll until the proposal is `passed` (bounded by --wait-secs).
    #[arg(long)]
    wait_pass: bool,
    #[arg(long, default_value_t = 300)]
    wait_secs: u64,
    /// circuit-propose / gov-propose: a rendered cosmos.gov.v1.MsgSubmitProposal JSON (any messages).
    #[arg(long)]
    proposal_file: Option<PathBuf>,
    /// sha256 pin of --proposal-file; a mismatch refuses before any grant.
    #[arg(long)]
    proposal_sha: Option<String>,
    /// gov-propose: replace the file's proposer with the signer (the kms granter). Without it a mismatch refuses.
    #[arg(long)]
    set_proposer: bool,
    /// gov-propose: do not top the deposit up to min_deposit with a MsgDeposit (default: top up).
    #[arg(long)]
    no_top_up: bool,
    /// dao-propose: substitute {{ZK_ID}} in a template proposal (shorthand for --param ZK_ID=N).
    #[arg(long)]
    zk_id: Option<u64>,
    /// dao-propose: template substitution K=V for {{K}} inside wasm msgs / title / description.
    #[arg(long)]
    param: Vec<String>,
    /// JSON array of terp_oracle::AltAsset for raffle/svg-minter oracle_pricing ("$AGGREGATOR" placeholder).
    #[arg(long)]
    oracle_alt_assets: Option<PathBuf>,
    /// Enforce oracle pricing at instantiate (default off).
    #[arg(long)]
    oracle_enforce: bool,
    /// Run the authenticator smoke test (throwaway account adds a CosmwasmAuthenticatorV1).
    #[arg(long)]
    smoke: bool,
    /// local-test only: fund the ephemeral key from this test-keyring key with terpd.
    #[arg(long)]
    fund_from: Option<String>,
    #[arg(long, default_value = "terpd")]
    terpd: String,
    #[arg(long, default_value = "")]
    terpd_home: String,
    #[arg(long, default_value = "tcp://127.0.0.1:26657")]
    terpd_node: String,
    /// local-test only: skip storing these codes (e.g. a wasm the local VM cannot compile).
    /// Phases depending on a skipped code are skipped and reported.
    #[arg(long)]
    skip_code: Vec<String>,
}

// Channel type (tonic) is not a direct dependency here; macros let the compiler infer it.
macro_rules! gov_proposal {
    ($ch:expr, $rt:expr, $pid:expr) => {{
        use cosmos_sdk_proto::cosmos::gov::v1::{QueryProposalRequest, query_client::QueryClient};
        let pid: u64 = $pid;
        $rt.block_on(QueryClient::new($ch).proposal(QueryProposalRequest { proposal_id: pid }))
            .map_err(|e| anyhow!("gov proposal {pid}: {e}"))
            .and_then(|r| {
                r.into_inner()
                    .proposal
                    .ok_or_else(|| anyhow!("gov proposal {pid}: empty"))
            })
    }};
}
macro_rules! gov_deposit_params {
    ($ch:expr, $rt:expr) => {{
        use cosmos_sdk_proto::cosmos::gov::v1::{QueryParamsRequest, query_client::QueryClient};
        $rt.block_on(QueryClient::new($ch).params(QueryParamsRequest {
            params_type: "deposit".into(),
        }))
        .map_err(|e| anyhow!("gov params: {e}"))
        .and_then(|r| {
            r.into_inner()
                .params
                .ok_or_else(|| anyhow!("gov params: empty (pre-v0.47 chain?)"))
        })
    }};
}
macro_rules! bank_balance {
    ($ch:expr, $rt:expr, $addr:expr, $denom:expr) => {{
        use cosmos_sdk_proto::cosmos::bank::v1beta1::{
            QueryBalanceRequest, query_client::QueryClient,
        };
        let (addr, denom): (&str, &str) = ($addr, $denom);
        $rt.block_on(QueryClient::new($ch).balance(QueryBalanceRequest {
            address: addr.into(),
            denom: denom.into(),
        }))
        .map_err(|e| anyhow!("balance {addr}: {e}"))
        .map(|r| {
            r.into_inner()
                .balance
                .map(|c| c.amount.parse::<u128>().unwrap_or(0))
                .unwrap_or(0)
        })
    }};
}

// ---------------------------------------------------------------- state

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct CodeRec {
    code_id: u64,
    checksum: String,
    tx: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wasm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    crate_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_dirty: Option<bool>,
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct ContractRec {
    address: String,
    code: String,
    admin: Option<String>,
    tx: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct TxRec {
    phase: String,
    step: String,
    hash: String,
    gas_used: u64,
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct State {
    chain_id: String,
    admin: String,
    codes: BTreeMap<String, CodeRec>,
    external_codes: BTreeMap<String, u64>,
    #[serde(default)]
    previous_codes: BTreeMap<String, u64>,
    contracts: BTreeMap<String, ContractRec>,
    done: BTreeSet<String>,
    /// Foundation proposals submitted by the daos phase: step id -> proposal id.
    #[serde(default)]
    proposals: BTreeMap<String, u64>,
    txs: Vec<TxRec>,
    #[serde(skip)]
    path: PathBuf,
    /// `proposals` entries that are not a bare u64 (e.g. custody-akash records), kept verbatim.
    #[serde(skip)]
    extra_proposals: serde_json::Map<String, Value>,
}

impl State {
    fn load(path: &Path, chain_id: &str, admin: &str) -> Result<Self> {
        let mut s: State = if path.exists() {
            let mut v: Value = serde_json::from_slice(&std::fs::read(path)?)
                .with_context(|| format!("parse {}", path.display()))?;
            let mut extra = serde_json::Map::new();
            if let Some(m) = v.get_mut("proposals").and_then(Value::as_object_mut) {
                let odd: Vec<String> =
                    m.iter().filter(|(_, x)| !x.is_u64()).map(|(k, _)| k.clone()).collect();
                for k in odd {
                    extra.insert(k.clone(), m.remove(&k).unwrap());
                }
            }
            let mut s: State = serde_json::from_value(v)
                .with_context(|| format!("parse {}", path.display()))?;
            s.extra_proposals = extra;
            s
        } else {
            State::default()
        };
        if !s.chain_id.is_empty() && s.chain_id != chain_id {
            bail!(
                "{} belongs to {}, not {chain_id}",
                path.display(),
                s.chain_id
            );
        }
        s.chain_id = chain_id.into();
        s.admin = admin.into();
        if chain_id == "morocco-1" {
            for (k, v) in EXTERNAL_CODES {
                s.external_codes.entry((*k).into()).or_insert(*v);
            }
            for (k, v) in PREVIOUS_CODES {
                s.previous_codes.entry((*k).into()).or_insert(*v);
            }
        }
        s.path = path.into();
        Ok(s)
    }
    fn save(&self) -> Result<()> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut v = serde_json::to_value(self)?;
        if let Some(m) = v.get_mut("proposals").and_then(Value::as_object_mut) {
            for (k, x) in &self.extra_proposals {
                m.entry(k.clone()).or_insert_with(|| x.clone());
            }
        }
        if self.path.exists() {
            std::fs::copy(&self.path, self.path.with_extension("json.bak"))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
    fn tx(&mut self, phase: &str, step: &str, r: &CosmTxResponse) -> Result<()> {
        println!("  tx {} gas {} ({step})", r.txhash, r.gas_used);
        self.txs.push(TxRec {
            phase: phase.into(),
            step: step.into(),
            hash: r.txhash.clone(),
            gas_used: r.gas_used,
        });
        self.save()
    }
    fn code_id(&self, name: &str) -> Result<u64> {
        self.codes
            .get(name)
            .map(|c| c.code_id)
            .ok_or_else(|| anyhow!("code {name} not stored yet: run the upload phase"))
    }
    fn addr(&self, name: &str) -> Option<String> {
        self.contracts.get(name).map(|c| c.address.clone())
    }
}

// ---------------------------------------------------------------- plan steps

#[derive(Clone)]
enum Step {
    Store {
        name: &'static str,
        file: &'static str,
    },
    Inst {
        name: String,
        code: String,
        msg: Value,
        admin: Option<String>,
    },
    Exec {
        id: String,
        contract: String,
        key: String,
        msg: Value,
    },
    /// Execute with the granter (cold key / DAO member) as the inner sender, via MsgExec.
    ExecAs {
        id: String,
        contract: String,
        key: String,
        msg: Value,
    },
    /// Granter votes yes on the proposal created by step `of`.
    Vote {
        id: String,
        module: String,
        of: String,
    },
    /// Generic: granter casts `choice` on the proposal created by step `of`.
    DaoVote {
        id: String,
        module: String,
        of: String,
        choice: String,
    },
    /// Generic: execute the proposal created by step `of` once `passed` (polls up to `wait_secs`).
    DaoExec {
        id: String,
        module: String,
        of: String,
        wait_secs: u64,
    },
    /// x/gov: granter submits a MsgSubmitProposal (proto bytes, proposer = granter).
    GovSubmit {
        id: String,
        title: String,
        msg: Vec<u8>,
        deposit: String,
    },
    /// x/gov: granter deposits `amount` on the proposal created by step `of`.
    GovDeposit {
        id: String,
        of: String,
        amount: u128,
        denom: String,
    },
    /// x/gov: granter votes `choice` on proposal `pid` or the one created by step `of`.
    GovVote {
        id: String,
        of: Option<String>,
        pid: Option<u64>,
        choice: String,
    },
    /// x/gov: poll until the proposal is PASSED (no tx).
    GovWait {
        id: String,
        of: Option<String>,
        pid: Option<u64>,
        wait_secs: u64,
    },
    /// Free-form step the runtime does itself (e.g. smoke test).
    Custom { id: String, what: String },
}

impl Step {
    fn describe(&self) -> Value {
        match self {
            Step::Store { name, file } => json!({"store": name, "file": file}),
            Step::Inst {
                name,
                code,
                msg,
                admin,
            } => json!({"instantiate2": name, "code": code, "admin": admin, "msg": msg}),
            Step::Exec {
                id,
                contract,
                key,
                msg,
            } => json!({"execute": id, "contract": contract, "key": key, "msg": elide(msg)}),
            Step::ExecAs {
                id,
                contract,
                key,
                msg,
            } => json!({"execute_as_granter": id, "contract": contract, "key": key, "msg": msg}),
            Step::Vote { id, module, of } => {
                json!({"vote_as_granter": id, "module": module, "proposal_of": of, "vote": "yes"})
            }
            Step::DaoVote {
                id,
                module,
                of,
                choice,
            } => {
                json!({"vote_as_granter": id, "module": module, "proposal_of": of, "vote": choice})
            }
            Step::DaoExec {
                id,
                module,
                of,
                wait_secs,
            } => {
                json!({"execute_as_granter": id, "module": module, "proposal_of": of, "wait_secs": wait_secs})
            }
            Step::GovSubmit {
                id,
                title,
                msg,
                deposit,
            } => json!({"gov_submit_as_granter": id, "title": title, "initial_deposit": deposit,
                "msg_bytes": msg.len(), "msg_sha256": hex_lower(&Sha256::digest(msg))}),
            Step::GovDeposit {
                id,
                of,
                amount,
                denom,
            } => {
                json!({"gov_deposit_as_granter": id, "proposal_of": of, "amount": format!("{amount}{denom}")})
            }
            Step::GovVote {
                id,
                of,
                pid,
                choice,
            } => {
                json!({"gov_vote_as_granter": id, "proposal_of": of, "proposal_id": pid, "vote": choice})
            }
            Step::GovWait {
                id,
                of,
                pid,
                wait_secs,
            } => {
                json!({"gov_wait_passed": id, "proposal_of": of, "proposal_id": pid, "wait_secs": wait_secs})
            }
            Step::Custom { id, what } => json!({"custom": id, "what": what}),
        }
    }
}

struct Ctx {
    cli: Cli,
    info: ChainInfoOwned,
    dir: PathBuf,
    state: State,
    /// Address that will sign (ephemeral grantee or local-test key); drives instantiate2 predictions.
    creator: Option<String>,
    daemon_builder: Option<DaemonBuilder>,
    local: Option<Daemon>,
    gas_by_phase: BTreeMap<String, (u64, u64)>,
}

fn artifacts_dir() -> PathBuf {
    std::env::var("ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts"))
}

fn file_of(name: &str) -> Result<&'static str> {
    CODES
        .iter()
        .chain(DAO_CODES)
        .copied()
        .chain(HS_CODES.iter().map(|(n, f, _)| (*n, *f)))
        .find(|(n, _)| *n == name)
        .map(|(_, f)| f)
        .ok_or_else(|| anyhow!("unknown code {name}"))
}

// Store provenance needs terp-scripts (optional since headstash-v2 d3bf2e1; feature `ibc-info`).
#[cfg(feature = "ibc-info")]
fn crate_of(name: &str) -> Option<&'static str> {
    terp_scripts::ibc::CODE_SOURCE_TABLE
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, p)| *p)
}

#[cfg(feature = "ibc-info")]
fn git_stamp_at_store(repo: &Path) -> (Option<String>, Option<bool>) {
    let v = terp_scripts::ibc::git_build_stamp(repo);
    (
        v.get("commit").and_then(|c| c.as_str()).map(str::to_string),
        v.get("dirty").and_then(|d| d.as_bool()),
    )
}

#[cfg(not(feature = "ibc-info"))]
fn crate_of(_name: &str) -> Option<&'static str> {
    None
}

#[cfg(not(feature = "ibc-info"))]
fn git_stamp_at_store(_repo: &Path) -> (Option<String>, Option<bool>) {
    (None, None)
}

fn read(dir: &Path, file: &str) -> Result<Vec<u8>> {
    let b = std::fs::read(dir.join(file))
        .with_context(|| format!("read {}", dir.join(file).display()))?;
    if b.is_empty() {
        bail!("{file} is empty");
    }
    Ok(b)
}

fn salt(name: &str) -> Vec<u8> {
    format!("mainnet-deploy/{name}").into_bytes()
}

fn canonical(addr: &str) -> Result<Vec<u8>> {
    let (_, data) = bech32::decode(addr).map_err(|e| anyhow!("{addr}: {e}"))?;
    Ok(data)
}

impl Ctx {
    fn query(&self) -> Result<cw_orch::daemon::senders::QueryOnlyDaemon> {
        kms_gate::query_only(&self.info)
    }

    fn predict(&self, code: &str, name: &str) -> Result<String> {
        // Dry-run without a session key: predict from --granter (preview only, the real
        // creator is the ephemeral session key), else a labelled placeholder.
        let creator = match (&self.creator, self.cli.broadcast) {
            (Some(c), _) => c.clone(),
            (None, false) if !self.cli.granter.is_empty() => self.cli.granter.clone(),
            (None, false) => return Ok(format!("<instantiate2:{name}>")),
            (None, true) => bail!("signer address unknown"),
        };
        let checksum = match self.state.codes.get(code) {
            Some(rec) => (0..rec.checksum.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&rec.checksum[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()?,
            None => wasm_checksum(&read(&self.dir, file_of(code)?)?)
                .map_err(|e| anyhow!(e))?
                .to_vec(),
        };
        instantiate2_bech32(
            &self.info.network_info.pub_address_prefix,
            &checksum,
            &canonical(&creator)?,
            &salt(name),
        )
        .map_err(|e| anyhow!(e))
    }

    /// Address of a contract: from state, else the instantiate2 prediction for this signer.
    fn addr_or_predict(&self, name: &str, code: &str) -> Result<String> {
        match self.state.addr(name) {
            Some(a) => Ok(a),
            None => self.predict(code, name),
        }
    }

    fn code_on_chain(&self, name: &str) -> Result<bool> {
        let Some(rec) = self.state.codes.get(name) else {
            return Ok(false);
        };
        // A rebuilt artifact is a new code: store it again rather than reuse the stale id.
        let local =
            hex_lower(&wasm_checksum(&read(&self.dir, file_of(name)?)?).map_err(|e| anyhow!(e))?);
        if local != rec.checksum {
            println!(
                "  {name}: artifact {} differs from stored code {} ({}); storing again",
                &local[..12],
                rec.code_id,
                &rec.checksum[..12]
            );
            return Ok(false);
        }
        let q = self.query()?;
        match q.wasm_querier().code(rec.code_id) {
            Ok(info) => {
                let on_chain = hex_lower(info.checksum.as_slice());
                if on_chain != rec.checksum {
                    bail!(
                        "code {name} id {} on chain has checksum {on_chain}, state says {}",
                        rec.code_id,
                        rec.checksum
                    );
                }
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    fn contract_on_chain(&self, name: &str) -> bool {
        let Some(a) = self.state.addr(name) else {
            return false;
        };
        self.query()
            .ok()
            .map(|q| q.wasm_querier().contract_info(&Addr::unchecked(a)).is_ok())
            .unwrap_or(false)
    }

    fn pending(&self, steps: Vec<Step>) -> Result<Vec<Step>> {
        let mut out = Vec::new();
        for s in steps {
            let done = match &s {
                Step::Store { name, .. } => self.code_on_chain(name)?,
                Step::Inst { name, .. } => self.contract_on_chain(name),
                Step::Exec { id, .. }
                | Step::ExecAs { id, .. }
                | Step::Vote { id, .. }
                | Step::DaoVote { id, .. }
                | Step::DaoExec { id, .. }
                | Step::Custom { id, .. }
                | Step::GovSubmit { id, .. }
                | Step::GovDeposit { id, .. }
                | Step::GovVote { id, .. } => self.state.done.contains(id),
                Step::GovWait { .. } => false,
            };
            if done {
                println!("  skip {}", s.describe());
            } else {
                out.push(s);
            }
        }
        Ok(out)
    }
}

fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------- signer

/// Inner sender: the kms granter, or (local-test) the local key standing in as the member itself.
fn granter_or_local(signer: &Signer, id: &str) -> Result<String> {
    match signer {
        Signer::Local(l) => Ok(l.sender_addr().to_string()),
        _ => signer
            .granter()
            .ok_or_else(|| anyhow!("{id}: needs a kms session (granter)")),
    }
}

/// One signing context per phase. The KMS variant owns the AuthzSession whose Drop revokes.
enum Signer {
    Kms(AuthzSession),
    Local(Daemon),
}
impl Signer {
    fn daemon(&self) -> &Daemon {
        match self {
            Signer::Kms(r) => &r.daemon,
            Signer::Local(d) => d,
        }
    }
    fn note(&self, l: &str) {
        if let Signer::Kms(r) = self {
            r.note(l)
        }
    }
    fn granter(&self) -> Option<String> {
        match self {
            Signer::Kms(r) => Some(r.granter().to_string()),
            Signer::Local(_) => None,
        }
    }
    fn close(self) -> Result<()> {
        match self {
            Signer::Kms(r) => r.close().map_err(|e| anyhow!(e)),
            Signer::Local(_) => Ok(()),
        }
    }
}

/// The kms granter (the cold key that signs the session grants). Captured once per run.
fn ensure_granter(ctx: &mut Ctx, phase: &str) -> Result<String> {
    if ctx.cli.granter.is_empty() {
        println!("[{phase}] sign the granter challenge on the approval page below");
        let g = cw_orch_kms::capture_granter_sync(
            kms_gate::granter_page(),
            ctx.cli.account_index,
            &format!("mainnet-deploy-{phase}"),
        )
        .map_err(|e| anyhow!(e))?;
        ctx.cli.granter = g;
    }
    Ok(ctx.cli.granter.clone())
}

/// Operational owner for owner-gated contracts: the kms granter, or the local test key.
/// Dry-runs without a known granter get a placeholder (plan only).
fn operational_owner(ctx: &mut Ctx, phase: &str) -> Result<String> {
    if ctx.cli.signer == SignerKind::LocalTest {
        return ctx
            .creator
            .clone()
            .ok_or_else(|| anyhow!("local signer unknown"));
    }
    if ctx.cli.granter.is_empty() && !ctx.cli.broadcast {
        return Ok("<kms-granter>".into());
    }
    ensure_granter(ctx, phase)
}

fn open_signer(ctx: &mut Ctx, phase: &str, steps: &[Step]) -> Result<Signer> {
    if ctx.cli.signer == SignerKind::LocalTest {
        return Ok(Signer::Local(
            ctx.local
                .clone()
                .ok_or_else(|| anyhow!("local signer missing"))?,
        ));
    }
    let builder = ctx
        .daemon_builder
        .clone()
        .ok_or_else(|| anyhow!("no builder"))?;
    let granter = ensure_granter(ctx, phase)?;
    if ["vote", "dao-vote", "dao-execute"].contains(&phase) && ctx.cli.vote_execute {
        // Wrong-signer check right after the granter challenge, before any grant/fee scan.
        let module = steps
            .iter()
            .find_map(|s| match s {
                Step::Custom { id, what } if id == "foundation:vote" => Some(what.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow!("vote: no proposal module"))?;
        let q = ctx.query()?;
        let m = Addr::unchecked(&module);
        for pid in ctx.cli.vote_proposals.clone() {
            let mine: Value = q
                .wasm_querier()
                .smart_query(
                    &m,
                    &json!({"get_vote": {"proposal_id": pid, "voter": granter}}),
                )
                .map_err(|e| anyhow!("{e}"))?;
            let p: Value = q
                .wasm_querier()
                .smart_query(&m, &json!({"proposal": {"proposal_id": pid}}))
                .map_err(|e| anyhow!("{e}"))?;
            if !mine["vote"].is_null() && p["proposal"]["status"] == "open" {
                bail!(
                    "refused before any grant: {granter} already voted on open proposal {pid}; the --execute run must be signed by another Foundation member (nothing was broadcast)"
                );
            }
        }
        println!(
            "  signer check ok: {granter} has not voted on open proposals {:?}",
            ctx.cli.vote_proposals
        );
    }
    let mut session = ApprovalSession::open(&granter);
    let mut execs: BTreeMap<String, (BTreeSet<String>, u64)> = BTreeMap::new();
    let mut actions = Vec::new();
    let mut sdk: BTreeSet<&str> = BTreeSet::new();
    let mut extra_fee_txs = 0u64;
    for s in steps {
        match s {
            Step::Store { file, .. } => {
                session = session
                    .allow_store(&read(&ctx.dir, file)?)
                    .map_err(|e| anyhow!(e))?;
            }
            Step::Custom { id, what } if id == "foundation:vote" => {
                // what = proposal module; scope: vote (+ execute) on it, MsgExec as the granter.
                let e = execs.entry(what.clone()).or_default();
                if phase != "dao-execute" || ctx.cli.vote_choice.is_some() {
                    e.0.insert("vote".into());
                    e.1 += ctx.cli.vote_proposals.len() as u64;
                }
                if ctx.cli.vote_execute {
                    e.0.insert("execute".into());
                    e.1 += ctx.cli.vote_proposals.len() as u64;
                }
            }
            Step::Inst { .. } | Step::Custom { .. } => {
                session = session.allow_instantiate2();
            }
            Step::Exec { contract, key, .. } | Step::ExecAs { contract, key, .. } => {
                let e = execs.entry(contract.clone()).or_default();
                e.0.insert(key.clone());
                e.1 += 1;
            }
            Step::Vote { module, .. } | Step::DaoVote { module, .. } => {
                let e = execs.entry(module.clone()).or_default();
                e.0.insert("vote".into());
                e.1 += 1;
            }
            Step::DaoExec { module, .. } => {
                let e = execs.entry(module.clone()).or_default();
                e.0.insert("execute".into());
                e.1 += 1;
            }
            Step::GovSubmit { msg, .. } => {
                sdk.insert(MSG_SUBMIT_PROPOSAL);
                extra_fee_txs += 1 + msg.len() as u64 / 25_000;
            }
            Step::GovDeposit { .. } => {
                sdk.insert(MSG_DEPOSIT);
            }
            Step::GovVote { .. } => {
                sdk.insert(MSG_GOV_VOTE);
            }
            Step::GovWait { .. } => {}
        }
        actions.push(SigningRequest::new("session-key").body(format!("{phase}: {}", s.describe())));
    }
    for (address, (keys, calls)) in execs {
        // +1 headroom: x/wasm deletes a MaxCallsLimit grant when it reaches 0, and the
        // session close then fails revoking a grant that no longer exists.
        session = session
            .allow_execute(ScopedCall {
                address,
                keys: keys.into_iter().collect(),
                calls: calls + 1,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?;
    }
    for t in sdk {
        session = session.allow_sdk(t).map_err(|e| anyhow!(e))?;
    }
    session.ttl_secs = ctx.cli.ttl_secs;
    session.account_index = ctx.cli.account_index;
    session.allowance_limit = planned_spend_limit(steps.len() as u64 + 1 + extra_fee_txs);
    session.retain = ctx.cli.retain;
    let run = enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    if Some(run.daemon.sender_addr().to_string()) != ctx.creator {
        bail!(
            "session key {} differs from the predicted creator {:?}",
            run.daemon.sender_addr(),
            ctx.creator
        );
    }
    Ok(Signer::Kms(run))
}

// ---------------------------------------------------------------- run a step list

fn execute_steps(ctx: &mut Ctx, phase: &str, steps: Vec<Step>) -> Result<()> {
    let mut steps = ctx.pending(steps)?;
    if !ctx.cli.steps_filter.is_empty() {
        steps.retain(|s| match s {
            Step::Store { name, .. } => ctx.cli.steps_filter.iter().any(|f| f == name),
            Step::Inst { name, .. } => ctx.cli.steps_filter.contains(name),
            Step::Exec { id, .. }
            | Step::ExecAs { id, .. }
            | Step::Vote { id, .. }
            | Step::DaoVote { id, .. }
            | Step::DaoExec { id, .. }
            | Step::GovSubmit { id, .. }
            | Step::GovDeposit { id, .. }
            | Step::GovVote { id, .. }
            | Step::GovWait { id, .. }
            | Step::Custom { id, .. } => ctx.cli.steps_filter.contains(id),
        });
    }
    let creator_basis = match (&ctx.creator, ctx.cli.granter.is_empty()) {
        (Some(_), _) => "session signer".to_string(),
        (None, false) => format!(
            "DRY-RUN PREVIEW: instantiate2 addresses predicted with creator = --granter {}; the broadcast creator is the ephemeral session key, so real addresses will differ",
            ctx.cli.granter
        ),
        (None, true) => "DRY-RUN PLACEHOLDER: no signer or --granter, addresses are <instantiate2:name>".into(),
    };
    let plan = json!({"phase": phase, "chain_id": ctx.info.chain_id, "admin": ctx.cli.admin,
        "signer": format!("{:?}", ctx.cli.signer), "creator": ctx.creator, "creator_basis": creator_basis,
        "steps_filter": ctx.cli.steps_filter,
        "pending": steps.iter().map(Step::describe).collect::<Vec<_>>()});
    kms_gate::write_plan(
        ctx.cli
            .out
            .join(format!("plan-{phase}.json"))
            .to_str()
            .unwrap(),
        &plan,
    )?;
    if steps.is_empty() {
        println!("[{phase}] nothing to do");
        return Ok(());
    }
    if !ctx.cli.broadcast {
        println!(
            "[{phase}] dry-run: {} step(s) pending, plan written",
            steps.len()
        );
        return Ok(());
    }
    let signer = open_signer(ctx, phase, &steps)?;
    let result = run_steps(ctx, phase, &signer, &steps);
    let closed = signer.close();
    result?;
    closed?;
    Ok(())
}

fn run_steps(ctx: &mut Ctx, phase: &str, signer: &Signer, steps: &[Step]) -> Result<()> {
    let d = signer.daemon();
    for s in steps {
        match s {
            Step::Store { name, file } => {
                let bytes = read(&ctx.dir, file)?;
                let r = store_with_retry(
                    d,
                    file,
                    &ctx.dir.join(file),
                    ctx.cli.signer == SignerKind::Kms,
                )?;
                let code_id = r.uploaded_code_id().map_err(|e| anyhow!("{e}"))?;
                let on_chain = hex_lower(d.wasm_querier().code(code_id)?.checksum.as_slice());
                let local = hex_lower(&Sha256::digest(&bytes));
                if on_chain != local {
                    bail!("stored {file} as {code_id} but chain checksum {on_chain} != {local}");
                }
                let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
                let (git_commit, git_dirty) = git_stamp_at_store(&repo);
                ctx.state.codes.insert(
                    (*name).into(),
                    CodeRec {
                        code_id,
                        checksum: local,
                        tx: Some(r.txhash.clone()),
                        wasm: Some((*file).into()),
                        crate_path: crate_of(name).map(str::to_string),
                        git_commit,
                        git_dirty,
                    },
                );
                ctx.state.tx(phase, &format!("store {name}"), &r)?;
                signer.note(&format!("ok stored {name} code {code_id}"));
            }
            Step::Inst {
                name,
                code,
                msg,
                admin,
            } => {
                let code_id = ctx.state.code_id(code)?;
                let predicted = ctx.predict(code, name)?;
                let admin_addr = admin.as_ref().map(Addr::unchecked);
                let r = d
                    .instantiate2(
                        code_id,
                        msg,
                        Some(name.as_str()),
                        admin_addr.as_ref(),
                        &[],
                        Binary::from(salt(name)),
                    )
                    .map_err(|e| anyhow!("instantiate2 {name}: {e}"))?;
                let address = r
                    .instantiated_contract_address()
                    .map_err(|e| anyhow!("{e}"))?
                    .to_string();
                if address != predicted {
                    bail!("{name} landed on {address}, predicted {predicted}");
                }
                ctx.state.contracts.insert(
                    name.clone(),
                    ContractRec {
                        address: address.clone(),
                        code: code.clone(),
                        admin: admin.clone(),
                        tx: Some(r.txhash.clone()),
                    },
                );
                ctx.state.tx(phase, &format!("instantiate2 {name}"), &r)?;
                println!("  {name} = {address}");
                signer.note(&format!("ok instantiated {name} {address}"));
            }
            Step::Exec {
                id,
                contract,
                key,
                msg,
            } => {
                let r = d
                    .execute(msg, &[], &Addr::unchecked(contract))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                record_drop(ctx, id, key, &r);
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::ExecAs {
                id,
                contract,
                key,
                msg,
            } => {
                let granter = granter_or_local(signer, id)?;
                let any = kms_gate::execute_any(&granter, contract, msg)?;
                let r = d
                    .rt_handle
                    .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                if key == "propose" {
                    let pid: u64 = r
                        .event_attr_value("wasm", "proposal_id")
                        .map_err(|e| anyhow!("{id}: no proposal_id event: {e}"))?
                        .parse()?;
                    ctx.state.proposals.insert(id.clone(), pid);
                    println!("  {id}: proposal {pid}");
                }
                record_drop(ctx, id, key, &r);
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::Vote { id, module, of } => {
                let granter = granter_or_local(signer, id)?;
                let pid = *ctx
                    .state
                    .proposals
                    .get(of)
                    .ok_or_else(|| anyhow!("{id}: proposal of {of} unknown"))?;
                let any = kms_gate::execute_any(
                    &granter,
                    module,
                    &json!({"vote": {"proposal_id": pid, "vote": "yes", "rationale": null}}),
                )?;
                let r = d
                    .rt_handle
                    .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                println!("  {id}: voted yes on {pid}");
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::DaoVote {
                id,
                module,
                of,
                choice,
            } => {
                let granter = granter_or_local(signer, id)?;
                let pid = *ctx
                    .state
                    .proposals
                    .get(of)
                    .ok_or_else(|| anyhow!("{id}: proposal of {of} unknown"))?;
                let any = kms_gate::execute_any(
                    &granter,
                    module,
                    &json!({"vote": {"proposal_id": pid, "vote": choice, "rationale": null}}),
                )?;
                let r = d
                    .rt_handle
                    .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                println!("  {id}: voted {choice} on {pid}");
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::DaoExec {
                id,
                module,
                of,
                wait_secs,
            } => {
                let granter = granter_or_local(signer, id)?;
                let pid = *ctx
                    .state
                    .proposals
                    .get(of)
                    .ok_or_else(|| anyhow!("{id}: proposal of {of} unknown"))?;
                let m = Addr::unchecked(module);
                let t0 = std::time::Instant::now();
                loop {
                    let p: Value = d
                        .wasm_querier()
                        .smart_query(&m, &json!({"proposal": {"proposal_id": pid}}))
                        .map_err(|e| anyhow!("{id}: {e}"))?;
                    let st = p["proposal"]["status"].as_str().unwrap_or("").to_string();
                    if st == "passed" {
                        break;
                    }
                    if st != "open" || t0.elapsed().as_secs() >= *wait_secs {
                        bail!(
                            "{id}: proposal {pid} status {st} after {}s (not executed; --execute needs passed, use --wait-pass)",
                            t0.elapsed().as_secs()
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
                let any = kms_gate::execute_any(
                    &granter,
                    module,
                    &json!({"execute": {"proposal_id": pid}}),
                )?;
                let r = d
                    .rt_handle
                    .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                let p: Value = d
                    .wasm_querier()
                    .smart_query(&m, &json!({"proposal": {"proposal_id": pid}}))
                    .map_err(|e| anyhow!("{e}"))?;
                println!(
                    "  {id}: executed {pid} -> status {}",
                    p["proposal"]["status"]
                );
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::GovSubmit { id, msg, .. } => {
                let any = cosmrs::Any {
                    type_url: MSG_SUBMIT_PROPOSAL.into(),
                    value: msg.clone(),
                };
                let r = d
                    .rt_handle
                    .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                let pid: u64 = r
                    .event_attr_value("submit_proposal", "proposal_id")
                    .map_err(|e| anyhow!("{id}: no proposal_id event: {e}"))?
                    .parse()?;
                ctx.state.proposals.insert(id.clone(), pid);
                println!(
                    "  {id}: gov proposal {pid} (tx {}, gas {})",
                    r.txhash, r.gas_used
                );
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id} -> gov proposal {pid}"));
            }
            Step::GovDeposit {
                id,
                of,
                amount,
                denom,
            } => {
                let depositor = granter_or_local(signer, id)?;
                let pid = *ctx
                    .state
                    .proposals
                    .get(of)
                    .ok_or_else(|| anyhow!("{id}: proposal of {of} unknown"))?;
                let m = cosmos_sdk_proto::cosmos::gov::v1::MsgDeposit {
                    proposal_id: pid,
                    depositor,
                    amount: vec![cosmos_sdk_proto::cosmos::base::v1beta1::Coin {
                        denom: denom.clone(),
                        amount: amount.to_string(),
                    }],
                };
                let r = d
                    .rt_handle
                    .block_on(
                        d.sender()
                            .commit_tx_any(vec![kms_gate::any(MSG_DEPOSIT, &m)], Some(id.as_str())),
                    )
                    .map_err(|e| anyhow!("{id}: {e}"))?;
                println!("  {id}: deposited {amount}{denom} on gov proposal {pid}");
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::GovVote {
                id,
                of,
                pid,
                choice,
            } => {
                let voter = granter_or_local(signer, id)?;
                let pid = gov_pid(ctx, id, of, pid)?;
                let m = cosmos_sdk_proto::cosmos::gov::v1::MsgVote {
                    proposal_id: pid,
                    voter,
                    option: gov_option(choice)?,
                    metadata: String::new(),
                };
                let r =
                    d.rt_handle
                        .block_on(d.sender().commit_tx_any(
                            vec![kms_gate::any(MSG_GOV_VOTE, &m)],
                            Some(id.as_str()),
                        ))
                        .map_err(|e| anyhow!("{id}: {e}"))?;
                println!("  {id}: voted {choice} on gov proposal {pid}");
                ctx.state.done.insert(id.clone());
                ctx.state.tx(phase, id, &r)?;
                signer.note(&format!("ok {id}"));
            }
            Step::GovWait {
                id,
                of,
                pid,
                wait_secs,
            } => {
                let pid = gov_pid(ctx, id, of, pid)?;
                let t0 = std::time::Instant::now();
                loop {
                    let p = gov_proposal!(d.channel(), &d.rt_handle, pid)?;
                    match p.status {
                        3 => {
                            println!(
                                "  {id}: gov proposal {pid} PASSED (messages executed by x/gov)"
                            );
                            break;
                        }
                        4 | 5 => bail!(
                            "{id}: gov proposal {pid} ended with status {} (failed_reason {:?})",
                            p.status,
                            p.failed_reason
                        ),
                        _ if t0.elapsed().as_secs() >= *wait_secs => bail!(
                            "{id}: gov proposal {pid} still status {} after {wait_secs}s",
                            p.status
                        ),
                        _ => std::thread::sleep(std::time::Duration::from_secs(5)),
                    }
                }
            }
            Step::Custom { id, .. } if id == "authenticators:smoke" => smoke(ctx, phase, d)?,
            Step::Custom { id, what } if id == "foundation:vote" => {
                foundation_vote(ctx, phase, signer, d, what)?
            }
            Step::Custom { id, .. } => bail!("no handler for {id}"),
        }
    }
    Ok(())
}

/// Record a factory-created drop collection from the tx events.
fn record_drop(ctx: &mut Ctx, id: &str, key: &str, r: &CosmTxResponse) {
    if (key == "create_series_collection" || key == "create_svg_collection")
        && let Ok(a) = r.event_attr_value("wasm", "contract")
    {
        let label = id.trim_start_matches("svg:create:").to_string();
        ctx.state.contracts.insert(
            format!("drop:{label}"),
            ContractRec {
                address: a.clone(),
                code: "cw721-svg".into(),
                admin: None,
                tx: Some(r.txhash.clone()),
            },
        );
        println!("  drop {label} = {a}");
    }
}

fn store_with_retry(chain: &Daemon, file: &str, path: &Path, raw: bool) -> Result<CosmTxResponse> {
    let mut last = String::new();
    for attempt in 1..=3 {
        let res = if raw {
            // terpd's ante chain has no wasmkeeper GasRegisterDecorator, so x/wasm's
            // StoreCodeAuthorization rejects gzip bytecode inside MsgExec ("gas register not
            // found in context"). Under an authz session, store the raw wasm instead.
            let bytes = std::fs::read(path).with_context(|| format!("{file}: read"))?;
            let msg = cosmrs::proto::cosmwasm::wasm::v1::MsgStoreCode {
                sender: chain.sender().account_id().to_string(),
                wasm_byte_code: bytes,
                // Explicit (never nil): x/wasm CodeGrant.Accept derefs it when the grant restricts
                // instantiation (docs/issues/wasm-authz-codegrant-nil-instantiate-permission.md).
                // The session StoreCode grant has no restriction, so Everybody is accepted.
                instantiate_permission: Some(cosmrs::proto::cosmwasm::wasm::v1::AccessConfig {
                    permission: cosmrs::proto::cosmwasm::wasm::v1::AccessType::Everybody as i32,
                    addresses: vec![],
                }),
            };
            let any = cosmrs::Any {
                type_url: "/cosmwasm.wasm.v1.MsgStoreCode".into(),
                value: cosmos_sdk_proto::traits::Message::encode_to_vec(&msg),
            };
            chain
                .rt_handle
                .block_on(chain.sender().commit_tx_any(vec![any], None))
                .map_err(|e| anyhow!("{e}"))
        } else {
            let wasm = WasmPath::new(path).map_err(|e| anyhow!("{file}: {e}"))?;
            chain
                .rt_handle
                .block_on(cw_orch::daemon::upload_wasm(chain.sender(), wasm, None))
                .map_err(|e| anyhow!("{e}"))
        };
        match res {
            Ok(r) => return Ok(r),
            Err(e) => {
                last = e.to_string();
                let transient = [
                    "h2 protocol",
                    "stream error",
                    "connection reset",
                    "error reading a body",
                ]
                .iter()
                .any(|t| last.contains(t));
                if !transient || attempt == 3 {
                    bail!("{file}: {last}");
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
    bail!("{file}: {last}")
}

// ---------------------------------------------------------------- phases

fn preflight(ctx: &mut Ctx) -> Result<()> {
    let sums = std::fs::read_to_string(ctx.dir.join("checksums.txt"))
        .context("artifacts/checksums.txt")?;
    let expected: BTreeMap<&str, &str> = sums
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next_back()?, it.next()?))
        })
        .collect();
    let mut bad = Vec::new();
    for (name, file) in CODES {
        let got = hex_lower(&Sha256::digest(read(&ctx.dir, file)?));
        match expected.get(file) {
            Some(e) if *e == got => println!("  ok {name} {file} {}", &got[..12]),
            Some(e) => bad.push(format!("{file}: checksums.txt {e} != {got}")),
            None => bad.push(format!("{file}: missing from checksums.txt")),
        }
    }
    if !bad.is_empty() {
        bail!(
            "artifact checksum mismatch (adoption still running? retry):\n{}",
            bad.join("\n")
        );
    }
    // Chain-id from the node itself.
    let q = ctx.query()?;
    let reported = q
        .rt_handle
        .block_on(cw_orch::daemon::queriers::Node::new_async(q.channel())._block_info())?
        .chain_id;
    cw_orch_kms::ensure_chain_id(&ctx.info.chain_id, &reported).map_err(|e| anyhow!(e))?;
    println!("  node chain-id {reported}");
    if ctx.cli.broadcast && ctx.cli.signer == SignerKind::Kms {
        kms_gate::check_broadcast_preconditions(
            &BroadcastRequest {
                info: ctx.info.clone(),
                expected_chain_id: ctx.cli.chain_id.clone(),
                granter: ctx.cli.granter.clone(),
                purpose: "mainnet-deploy".into(),
                account_index: ctx.cli.account_index,
                ttl_secs: ctx.cli.ttl_secs,
                scope: Scope::Instantiate2,
                summary: "preflight".into(),
            },
            std::env::var(kms_gate::BROADCAST_ENV).ok().as_deref(),
        )?;
        let page = kms_gate::granter_page();
        println!("  kms gate ok; approval page will bind {page} (re-picked if taken)");
    }
    println!(
        "  admin {}  signer {:?}  creator {:?}",
        ctx.cli.admin, ctx.cli.signer, ctx.creator
    );
    println!(
        "  state {} ({} codes, {} contracts, {} txs)",
        ctx.state.path.display(),
        ctx.state.codes.len(),
        ctx.state.contracts.len(),
        ctx.state.txs.len()
    );
    for p in PHASES {
        println!("  phase {p}");
    }
    Ok(())
}

fn upload(ctx: &mut Ctx) -> Result<()> {
    if !ctx.cli.skip_code.is_empty() && local_guard(&ctx.cli).is_err() {
        bail!("--skip-code is only allowed on a 127.0.0.1 rehearsal chain");
    }
    let steps = CODES
        .iter()
        .filter(|(n, _)| !ctx.cli.skip_code.iter().any(|s| s == n))
        .map(|(name, file)| Step::Store { name, file })
        .collect();
    execute_steps(ctx, "upload", steps)
}

fn bounty_upload(ctx: &mut Ctx) -> Result<()> {
    let steps = BOUNTY_CODES
        .iter()
        .map(|(name, file)| Step::Store { name, file })
        .collect();
    execute_steps(ctx, "bounty-upload", steps)
}

fn hs_upload(ctx: &mut Ctx) -> Result<()> {
    for (name, file, pinned) in HS_CODES {
        let got = hex_lower(&Sha256::digest(&read(&ctx.dir, file)?));
        if got != *pinned {
            bail!(
                "{name}: {file} sha256 {got} != pinned {pinned} (rebuild hermetically, do not upload)"
            );
        }
    }
    let steps = HS_CODES
        .iter()
        .filter(|(n, _, _)| !ctx.cli.skip_code.iter().any(|s| s == n))
        .map(|(name, file, _)| Step::Store { name, file })
        .collect();
    execute_steps(ctx, "hs-upload", steps)
}

fn dao_upload(ctx: &mut Ctx) -> Result<()> {
    let steps = DAO_CODES
        .iter()
        .filter(|(n, _)| !ctx.cli.skip_code.iter().any(|s| s == n))
        .map(|(name, file)| Step::Store { name, file })
        .collect();
    execute_steps(ctx, "dao-upload", steps)
}

/// FIAB + steward are Foundation proposals. A Foundation member (the kms granter) submits
/// each one to the Foundation pre-propose module and votes yes, as MsgExec through the
/// cw-orch-kms session; execution stays with the Foundation DAO.
/// FIAB: wasm admin and DAO admin are the Foundation (`--admin`).
/// Stewards: not blocked on FIAB; their dao admin and wasm admin are the x/gov module
/// account, queried from the chain (auth ModuleAccountByName "gov").
fn daos(ctx: &mut Ctx) -> Result<()> {
    use dao_foundation_suite::ibc_ops::steward::{
        JUNO_DAO_CODES, JUNO_GOV, JunoRemote, assert_steward_admin, build_steward_plan_tuned,
        decode_hex, load_steward_accounts,
    };
    use dao_foundation_suite::ibc_ops::{
        CODE_CORE, FIAB_PROPOSAL_TITLE, FOUNDATION_DESCRIPTION, foundation_proposal_wrapper,
        morocco_fiab_instantiate,
    };
    let foundation = ctx.cli.admin.clone();
    let pre = ctx.cli.foundation_pre_propose.clone();
    let q = ctx.query()?;
    // FIAB
    let fiab = morocco_fiab_instantiate(&foundation);
    let fiab_v = serde_json::to_value(&fiab)?;
    let inst = &fiab_v["wasm"]["instantiate"];
    if inst["admin"].as_str() != Some(foundation.as_str()) {
        bail!(
            "FIAB wasm admin is {:?}, expected the Foundation",
            inst["admin"]
        );
    }
    let fiab_init: Value = serde_json::from_slice(
        &Binary::from_base64(inst["msg"].as_str().unwrap_or_default())
            .map_err(|e| anyhow!("{e}"))?,
    )?;
    if fiab_init["admin"].as_str() != Some(foundation.as_str()) {
        bail!(
            "FIAB dao admin is {:?}, expected the Foundation",
            fiab_init["admin"]
        );
    }
    let fiab_propose =
        foundation_proposal_wrapper(FIAB_PROPOSAL_TITLE, FOUNDATION_DESCRIPTION, vec![fiab]);
    // Stewards
    let gov = match &ctx.cli.gov_parent {
        Some(g) => g.clone(),
        None => {
            kms_gate::query_module_account(&q, "gov", &ctx.info.network_info.pub_address_prefix)?
        }
    };
    if gov == foundation {
        bail!("gov module parent equals the Foundation");
    }
    println!("  steward parent = x/gov module account {gov}");
    let sum = match &ctx.cli.dao_core_checksum {
        Some(h) => decode_hex(h).map_err(|e| anyhow!(e))?,
        None => q
            .wasm_querier()
            .code(CODE_CORE)
            .map_err(|e| anyhow!("dao-dao-core code {CODE_CORE}: {e}"))?
            .checksum
            .as_slice()
            .to_vec(),
    };
    let accounts_path = ctx.cli.steward_accounts.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../steward-accounts.json")
    });
    let accounts = load_steward_accounts(&accounts_path).map_err(|e| anyhow!(e))?;
    // Tuned plan (4abf137): steward-akash + custody-akash + steward-juno on Terp, gov-admin.
    // Polytone follow-ups (custody-juno via Juno note, custody-akash Osmosis account) are
    // separate proposals after the new-channel notes are approved: no note here.
    if kms_gate::module_address("gov", "juno")? != JUNO_GOV {
        bail!("juno gov module mismatch");
    }
    let jr = JunoRemote {
        note: None,
        proxy: None,
        juno_gov: JUNO_GOV,
        codes: JUNO_DAO_CODES,
    };
    let plan = build_steward_plan_tuned(&accounts, &foundation, &sum, None, Some(&gov), &jr)
        .map_err(|e| anyhow!(e))?;
    let msgs: Vec<cosmwasm_std::CosmosMsg> = serde_json::from_value(
        plan.foundation_proposal["propose"]["msg"]["propose"]["msgs"].clone(),
    )?;
    if msgs.len() != 3 {
        bail!(
            "expected 3 steward instantiate2 msgs (tuned), got {}",
            msgs.len()
        );
    }
    println!(
        "  steward addresses {}",
        serde_json::to_string(&plan.addresses)?
    );
    for m in &msgs {
        assert_steward_admin(m, &foundation, Some(&gov)).map_err(|e| anyhow!(e))?;
    }
    std::fs::create_dir_all(ctx.cli.out.join("steward"))?;
    for sd in &plan.subdaos {
        std::fs::write(
            ctx.cli
                .out
                .join("steward")
                .join(format!("{}.json", sd.file_stem)),
            serde_json::to_string_pretty(&sd.body)?,
        )?;
    }
    // Proposal module (votes) from the pre-propose module, when it exists on this chain.
    let module: Option<String> = q
        .wasm_querier()
        .smart_query::<_, String>(&Addr::unchecked(&pre), &json!({"proposal_module": {}}))
        .ok();
    let mut steps = vec![
        Step::ExecAs {
            id: "daos:propose:fiab".into(),
            contract: pre.clone(),
            key: "propose".into(),
            msg: fiab_propose,
        },
        Step::ExecAs {
            id: "daos:propose:steward".into(),
            contract: pre.clone(),
            key: "propose".into(),
            msg: plan.foundation_proposal.clone(),
        },
    ];
    if !ctx.cli.no_vote {
        let m = module.clone().ok_or_else(|| {
            anyhow!("pre-propose {pre}: proposal_module query failed (not on this chain?)")
        })?;
        steps.push(Step::Vote {
            id: "daos:vote:fiab".into(),
            module: m.clone(),
            of: "daos:propose:fiab".into(),
        });
        steps.push(Step::Vote {
            id: "daos:vote:steward".into(),
            module: m,
            of: "daos:propose:steward".into(),
        });
    }
    println!("  foundation pre-propose {pre}, proposal module {module:?}");
    drop(q); // release the cw-orch state-file lock before the session daemon opens
    execute_steps(ctx, "daos", steps)?;
    for (k, v) in &ctx.state.proposals {
        println!("  {k} -> proposal {v}");
    }
    Ok(())
}

/// vote: a Foundation member (whoever signs the granter challenge, any member key) votes
/// `--vote yes` on `--proposals`, as MsgExec through the cw-orch-kms session (scope:
/// the proposal module, keys vote [+ execute]). Skips proposals the signer already voted on
/// or that are no longer open. `--execute`: then executes each listed proposal whose
/// status is `passed`, and records created DAO cores (code CODE_CORE) by label in state.json;
/// steward cores are checked against their instantiate2 predictions.
struct DaoTarget {
    core: String,
    module: String,
    pre: Option<String>,
}

/// Resolve core / proposal module / pre-propose from chain. Precedence: --proposal-module,
/// then --pre-propose, then --dao (first enabled module), else the Foundation pre-propose A.
fn resolve_dao(ctx: &Ctx) -> Result<DaoTarget> {
    let q = ctx.query()?;
    let w = q.wasm_querier();
    let sq = |c: &str, m: Value| -> Result<Value> {
        w.smart_query::<_, Value>(&Addr::unchecked(c), &m)
            .map_err(|e| anyhow!("{c} {m}: {e}"))
    };
    let policy = |module: &str| -> Result<Option<String>> {
        let p = sq(module, json!({"proposal_creation_policy": {}}))?;
        Ok(p["module"]["addr"].as_str().map(String::from))
    };
    let t = if let Some(m) = &ctx.cli.proposal_module {
        let core = sq(m, json!({"dao": {}}))?
            .as_str()
            .unwrap_or_default()
            .to_string();
        let pre = match &ctx.cli.pre_propose {
            Some(p) => Some(p.clone()),
            None => policy(m)?,
        };
        DaoTarget {
            core,
            module: m.clone(),
            pre,
        }
    } else if let Some(dao) = &ctx.cli.dao {
        let mods = sq(dao, json!({"proposal_modules": {}}))?;
        let m = mods
            .as_array()
            .and_then(|a| {
                a.iter().find(|x| {
                    x["status"]
                        .as_str()
                        .map_or(false, |s| s.eq_ignore_ascii_case("enabled"))
                })
            })
            .and_then(|x| x["address"].as_str())
            .ok_or_else(|| anyhow!("{dao}: no enabled proposal module"))?
            .to_string();
        let pre = match &ctx.cli.pre_propose {
            Some(p) => Some(p.clone()),
            None => policy(&m)?,
        };
        DaoTarget {
            core: dao.clone(),
            module: m,
            pre,
        }
    } else {
        let pre = ctx
            .cli
            .pre_propose
            .clone()
            .unwrap_or_else(|| ctx.cli.foundation_pre_propose.clone());
        let module = sq(&pre, json!({"proposal_module": {}}))?
            .as_str()
            .unwrap_or_default()
            .to_string();
        let core = sq(&pre, json!({"dao": {}}))?
            .as_str()
            .unwrap_or_default()
            .to_string();
        DaoTarget {
            core,
            module,
            pre: Some(pre),
        }
    };
    if t.module.is_empty() || t.core.is_empty() {
        bail!("could not resolve dao core / proposal module");
    }
    if let Some(pre) = &t.pre {
        let cfg = sq(pre, json!({"config": {}}))?;
        if !cfg["deposit_info"].is_null() {
            bail!(
                "pre-propose {pre} requires a deposit {}: not supported by this phase (no funds in the scoped grant); refusing before any grant",
                cfg["deposit_info"]
            );
        }
    }
    println!(
        "  dao {} | proposal module {} | pre-propose {:?}",
        t.core, t.module, t.pre
    );
    Ok(t)
}

/// Generic DAO proposal: sha-pinned --propose-file (pre-propose body) or --msgs-file + --title.
/// Submitted as the kms granter (MsgExec via the cw-orch-kms session) to the pre-propose module,
/// or straight to the proposal module when it has none. Optional --vote, --execute [--wait-pass].
/// Every instantiate2 in the msgs is re-predicted from the on-chain checksum with the DAO core
/// as creator; every in-proposal wasm execute / bank send target must be one of them, or (with
/// no instantiate2 at all) the check is reported as skipped.
fn dao_propose(ctx: &mut Ctx) -> Result<()> {
    let (bytes, src) = match (&ctx.cli.propose_file, &ctx.cli.msgs_file) {
        (Some(f), None) => (
            std::fs::read(f).map_err(|e| anyhow!("{}: {e}", f.display()))?,
            f.clone(),
        ),
        (None, Some(f)) => (
            std::fs::read(f).map_err(|e| anyhow!("{}: {e}", f.display()))?,
            f.clone(),
        ),
        _ => bail!("dao-propose: exactly one of --propose-file / --msgs-file"),
    };
    let got = hex_lower(&Sha256::digest(&bytes));
    match (&ctx.cli.propose_sha, ctx.cli.propose_file.is_some()) {
        (Some(w), _) if w.to_lowercase() != got => bail!(
            "refused before any grant: {} sha256 {got} != --propose-sha {w} (nothing was broadcast)",
            src.display()
        ),
        (None, true) => bail!("dao-propose: --propose-sha required with --propose-file"),
        _ => {}
    }
    let (title, description, msgs) = if ctx.cli.propose_file.is_some() {
        let body: Value = serde_json::from_slice(&bytes)?;
        let p = body["propose"]["msg"]["propose"].clone();
        let p = if p.is_null() {
            body["propose"].clone()
        } else {
            p
        };
        (
            p["title"].as_str().unwrap_or_default().to_string(),
            p["description"].as_str().unwrap_or_default().to_string(),
            p["msgs"].as_array().cloned().unwrap_or_default(),
        )
    } else {
        let m: Vec<Value> = serde_json::from_slice(&bytes)?;
        (
            ctx.cli
                .title
                .clone()
                .ok_or_else(|| anyhow!("--msgs-file needs --title"))?,
            ctx.cli.description.clone(),
            m,
        )
    };
    let mut params: Vec<(String, String)> = Vec::new();
    for kv in &ctx.cli.param {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow!("--param {kv}: expected K=V"))?;
        params.push((k.to_string(), v.to_string()));
    }
    if let Some(z) = ctx.cli.zk_id {
        params.push(("ZK_ID".into(), z.to_string()));
    }
    let (title, description, msgs) = substitute_template(&title, &description, msgs, &params)?;
    let got = if params.is_empty() {
        got
    } else {
        let p: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!("  template params {} applied", p.join(","));
        hex_lower(&Sha256::digest(format!("{got}|{}", p.join(";")).as_bytes()))
    };
    if msgs.is_empty() || title.is_empty() {
        bail!("dao-propose: {} has no msgs / title", src.display());
    }
    if let Some(c) = &ctx.cli.vote_choice {
        if !["yes", "no", "abstain"].contains(&c.as_str()) {
            bail!("--vote: yes | no | abstain");
        }
    }
    println!(
        "  {} sha256 {got}: {} msgs, title {title:?}",
        src.display(),
        msgs.len()
    );
    let t = resolve_dao(ctx)?;
    let prefix = ctx.info.network_info.pub_address_prefix.clone();
    let q = ctx.query()?;
    let mut predicted: Vec<String> = Vec::new();
    for m in &msgs {
        let i = &m["wasm"]["instantiate2"];
        if i.is_null() {
            continue;
        }
        let code_id = i["code_id"].as_u64().ok_or_else(|| {
            anyhow!(
                "instantiate2 {}: code_id {:?} is not final",
                i["label"],
                i["code_id"]
            )
        })?;
        let info = q
            .wasm_querier()
            .code(code_id)
            .map_err(|e| anyhow!("code {code_id}: {e}"))?;
        let salt_b = Binary::from_base64(i["salt"].as_str().unwrap_or_default())
            .map_err(|e| anyhow!("{e}"))?;
        let a = instantiate2_bech32(
            &prefix,
            info.checksum.as_slice(),
            &canonical(&t.core)?,
            salt_b.as_slice(),
        )
        .map_err(|e| anyhow!(e))?;
        println!(
            "  instantiate2 {} code {code_id} ({}) admin {} -> {a}",
            i["label"],
            &hex_lower(info.checksum.as_slice())[..12],
            i["admin"]
        );
        predicted.push(a);
    }
    let mut allowed: Vec<String> = Vec::new();
    for a in &ctx.cli.allow_target {
        let ci = q
            .wasm_querier()
            .contract_info(&Addr::unchecked(a))
            .map_err(|e| anyhow!("--allow-target {a}: not a contract on chain: {e}"))?;
        let admin = ci.admin.as_ref().map(|x| x.to_string());
        let owner: Option<String> = q
            .wasm_querier()
            .smart_query::<_, Value>(&Addr::unchecked(a), &json!({"ownership": {}}))
            .ok()
            .and_then(|o| o["owner"].as_str().map(String::from));
        if admin.as_deref() != Some(t.core.as_str()) && owner.as_deref() != Some(t.core.as_str()) {
            bail!(
                "--allow-target {a}: neither admin {admin:?} nor owner {owner:?} is the DAO {} (refusing before any grant)",
                t.core
            );
        }
        println!(
            "  allow-target {a}: code {} admin {admin:?} owner {owner:?} (DAO-controlled)",
            ci.code_id
        );
        allowed.push(a.clone());
    }
    drop(q);
    if predicted.is_empty() {
        println!("  instantiate2 check: skipped (no instantiate2 msgs)");
    } else {
        for m in &msgs {
            let tg = m["wasm"]["execute"]["contract_addr"]
                .as_str()
                .or(m["bank"]["send"]["to_address"].as_str());
            if let Some(tg) = tg {
                if !predicted.iter().any(|p| p == tg) && !allowed.iter().any(|p| p == tg) {
                    bail!(
                        "dao-propose: target {tg} is not an instantiate2 address of this proposal nor an --allow-target (refusing before any grant)"
                    );
                }
            }
        }
        println!(
            "  instantiate2 check: ok ({} addresses, all in-proposal targets match; {} allow-target)",
            predicted.len(),
            allowed.len()
        );
    }
    let (contract, msg) = match &t.pre {
        Some(pre) => (
            pre.clone(),
            json!({"propose": {"msg": {"propose": {"title": title, "description": description, "msgs": msgs}}}}),
        ),
        None => (
            t.module.clone(),
            json!({"propose": {"title": title, "description": description, "msgs": msgs, "proposer": null}}),
        ),
    };
    let pid_key = format!("dao-propose:{}", &got[..16]);
    let mut steps = vec![Step::ExecAs {
        id: pid_key.clone(),
        contract,
        key: "propose".into(),
        msg,
    }];
    if let Some(c) = ctx.cli.vote_choice.clone() {
        steps.push(Step::DaoVote {
            id: format!("{pid_key}:vote"),
            module: t.module.clone(),
            of: pid_key.clone(),
            choice: c,
        });
    }
    if ctx.cli.vote_execute {
        let w = if ctx.cli.wait_pass {
            ctx.cli.wait_secs
        } else {
            0
        };
        steps.push(Step::DaoExec {
            id: format!("{pid_key}:execute"),
            module: t.module.clone(),
            of: pid_key.clone(),
            wait_secs: w,
        });
    }
    execute_steps(ctx, "dao-propose", steps)
}

/// Shorten huge string fields (e.g. chunk `data`) in plan / approval text to their size + sha256.
fn elide(v: &Value) -> Value {
    match v {
        Value::String(s) if s.len() > 4096 => json!(format!(
            "<{} chars sha256 {}>",
            s.len(),
            hex_lower(&Sha256::digest(s.as_bytes()))
        )),
        Value::Array(a) => Value::Array(a.iter().map(elide).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), elide(x))).collect()),
        x => x.clone(),
    }
}

/// `chunk-upload`: generic cw-headstash-frontend chunk upload. `UploadChunk` is permissionless and
/// sha-checked on chain; every chunk is re-checked here against the on-chain manifest before any
/// grant, chunks already stored are skipped, and the session key is the sender (fees via feegrant).
fn chunk_upload(ctx: &mut Ctx) -> Result<()> {
    let f = ctx
        .cli
        .chunks_file
        .clone()
        .ok_or_else(|| anyhow!("chunk-upload: --chunks-file required"))?;
    let bytes = std::fs::read(&f).map_err(|e| anyhow!("{}: {e}", f.display()))?;
    let got = hex_lower(&Sha256::digest(&bytes));
    if let Some(w) = &ctx.cli.chunks_sha {
        if w.to_lowercase() != got {
            bail!(
                "refused before any grant: {} sha256 {got} != --chunks-sha {w} (nothing was broadcast)",
                f.display()
            );
        }
    }
    let plan: Value = serde_json::from_slice(&bytes)?;
    let contract = ctx
        .cli
        .frontend_addr
        .clone()
        .or_else(|| plan["contract"].as_str().map(String::from))
        .ok_or_else(|| anyhow!("chunk-upload: --frontend or plan \"contract\" required"))?;
    let version = plan["version"]
        .as_u64()
        .ok_or_else(|| anyhow!("chunk-upload: plan \"version\" missing"))? as u32;
    let files = plan["files"]
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("chunk-upload: plan \"files\" missing"))?;
    println!(
        "  {} sha256 {got}: {} files -> {contract} v{version}",
        f.display(),
        files.len()
    );
    let q = ctx.query()?;
    let c = Addr::unchecked(&contract);
    q.wasm_querier()
        .contract_info(&c)
        .map_err(|e| anyhow!("chunk-upload: {contract} is not a contract on chain: {e}"))?;
    let m: Value = q.wasm_querier().smart_query(&c, &json!({"manifest": {"version": version}}))
        .map_err(|e| anyhow!("chunk-upload: manifest v{version} not on {contract} (execute the publish proposal first): {e}"))?;
    let mfiles = m["files"].as_array().cloned().unwrap_or_default();
    if mfiles.len() != files.len() {
        bail!(
            "chunk-upload: plan has {} files, on-chain manifest v{version} has {}",
            files.len(),
            mfiles.len()
        );
    }
    let tag = &contract[contract.len().saturating_sub(12)..];
    let (mut total, mut stored, mut steps) = (0usize, 0usize, Vec::new());
    for (fi, (pf, mf)) in files.iter().zip(&mfiles).enumerate() {
        let chunks = pf["chunks"].as_array().cloned().unwrap_or_default();
        let hashes = mf["chunks"].as_array().cloned().unwrap_or_default();
        if chunks.len() != hashes.len() {
            bail!(
                "chunk-upload: file {fi} ({}) has {} chunks, manifest {}",
                mf["path"],
                chunks.len(),
                hashes.len()
            );
        }
        for (ci, (b, h)) in chunks.iter().zip(&hashes).enumerate() {
            let data = Binary::from_base64(b.as_str().unwrap_or_default())
                .map_err(|e| anyhow!("file {fi} chunk {ci}: {e}"))?;
            let hh = hex_lower(&Sha256::digest(data.as_slice()));
            if Some(hh.as_str()) != h.as_str().map(|x| x.to_lowercase()).as_deref() {
                bail!(
                    "refused before any grant: file {fi} chunk {ci} sha256 {hh} != on-chain manifest {h}"
                );
            }
            total += 1;
            let have = q
                .wasm_querier()
                .smart_query::<_, Binary>(
                    &c,
                    &json!({"chunk": {"version": version, "file": fi, "index": ci}}),
                )
                .is_ok();
            if have {
                stored += 1;
                continue;
            }
            steps.push(Step::Exec { id: format!("chunk-upload:{tag}:v{version}:{fi}:{ci}"), contract: contract.clone(), key: "upload_chunk".into(),
                msg: json!({"upload_chunk": {"version": version, "file": fi, "index": ci, "data": data}}) });
        }
    }
    println!(
        "  chunk check: ok ({total} chunks match the on-chain manifest; {stored} already stored, {} to upload)",
        steps.len()
    );
    drop(q);
    execute_steps(ctx, "chunk-upload", steps)?;
    if ctx.cli.broadcast {
        let st: Value = ctx
            .query()?
            .wasm_querier()
            .smart_query(&c, &json!({"status": {"version": version}}))
            .map_err(|e| anyhow!("{e}"))?;
        println!("  status v{version}: {st}");
    }
    Ok(())
}

/// `vote` (legacy alias of `dao-vote`, default the Foundation module A) / `dao-vote` / `dao-execute`.
fn vote(ctx: &mut Ctx, phase: &str) -> Result<()> {
    if phase == "dao-execute" {
        ctx.cli.vote_execute = true;
    }
    if let Some(c) = &ctx.cli.vote_choice {
        if !["yes", "no", "abstain"].contains(&c.as_str()) {
            bail!("--vote: yes | no | abstain");
        }
    }
    if ctx.cli.vote_proposals.is_empty() {
        bail!("--proposals 9,10 required");
    }
    let r = resolve_dao(ctx)?;
    let module = r.module.clone();
    let q = ctx.query()?;
    for pid in &ctx.cli.vote_proposals {
        let p: Value = q
            .wasm_querier()
            .smart_query(
                &Addr::unchecked(&module),
                &json!({"proposal": {"proposal_id": pid}}),
            )
            .map_err(|e| anyhow!("proposal {pid}: {e}"))?;
        println!(
            "  proposal {pid}: {} | {} | votes {}",
            p["proposal"]["status"], p["proposal"]["title"], p["proposal"]["votes"]
        );
    }
    println!(
        "  dao {} proposal module {module}; vote={:?} execute={}",
        r.core, ctx.cli.vote_choice, ctx.cli.vote_execute
    );
    drop(q);
    execute_steps(
        ctx,
        phase,
        vec![Step::Custom {
            id: "foundation:vote".into(),
            what: module,
        }],
    )
}

fn foundation_vote(
    ctx: &mut Ctx,
    phase: &str,
    signer: &Signer,
    d: &Daemon,
    module: &str,
) -> Result<()> {
    use dao_foundation_suite::ibc_ops::{CODE_CORE, steward::decode_bech32};
    let granter = granter_or_local(signer, "vote")?;
    let m = Addr::unchecked(module);
    let status = |pid: u64| -> Result<Value> {
        Ok(d.wasm_querier()
            .smart_query::<_, Value>(&m, &json!({"proposal": {"proposal_id": pid}}))
            .map_err(|e| anyhow!("{e}"))?["proposal"]
            .clone())
    };
    println!("  voter (granter) {granter}");
    if ctx.cli.vote_execute {
        // Run 2 must be a second member: refuse a signer that already voted on an open proposal.
        for pid in ctx.cli.vote_proposals.clone() {
            let mine: Value = d
                .wasm_querier()
                .smart_query(
                    &m,
                    &json!({"get_vote": {"proposal_id": pid, "voter": granter}}),
                )
                .map_err(|e| anyhow!("{e}"))?;
            if !mine["vote"].is_null() && status(pid)?["status"] == "open" {
                bail!(
                    "refused: {granter} already voted on open proposal {pid}; --execute run must be signed by another Foundation member"
                );
            }
        }
    }
    let choice = ctx.cli.vote_choice.clone().unwrap_or_else(|| "yes".into());
    let cast = phase != "dao-execute" || ctx.cli.vote_choice.is_some();
    for pid in ctx.cli.vote_proposals.clone() {
        if !cast {
            break;
        }
        let mine: Value = d
            .wasm_querier()
            .smart_query(
                &m,
                &json!({"get_vote": {"proposal_id": pid, "voter": granter}}),
            )
            .map_err(|e| anyhow!("{e}"))?;
        let p = status(pid)?;
        if !mine["vote"].is_null() {
            println!("  {pid}: {granter} already voted {}", mine["vote"]["vote"]);
            continue;
        }
        if p["status"] != "open" {
            println!("  {pid}: status {} (no vote cast)", p["status"]);
            continue;
        }
        let id = format!("vote:{pid}:{granter}");
        let any = kms_gate::execute_any(
            &granter,
            module,
            &json!({"vote": {"proposal_id": pid, "vote": choice, "rationale": null}}),
        )?;
        let r = d
            .rt_handle
            .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
            .map_err(|e| anyhow!("{id}: {e}"))?;
        ctx.state.tx(phase, &id, &r)?;
        let p = status(pid)?;
        println!(
            "  {pid}: voted {choice} -> status {} votes {}",
            p["status"], p["votes"]
        );
        signer.note(&format!("ok {id}"));
    }
    if !ctx.cli.vote_execute {
        return Ok(());
    }
    // Foundation steward labelling only; other chains/DAOs may not have CODE_CORE.
    let core_sum = d
        .wasm_querier()
        .code(CODE_CORE)
        .map(|c| c.checksum.as_slice().to_vec())
        .unwrap_or_default();
    let fbytes = decode_bech32(&ctx.cli.admin).map_err(|e| anyhow!(e))?.1;
    for pid in ctx.cli.vote_proposals.clone() {
        let p = status(pid)?;
        if p["status"] != "passed" {
            println!("  {pid}: status {} (not executed)", p["status"]);
            continue;
        }
        let id = format!("execute:{pid}");
        let any =
            kms_gate::execute_any(&granter, module, &json!({"execute": {"proposal_id": pid}}))?;
        let r = d
            .rt_handle
            .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
            .map_err(|e| anyhow!("{id}: {e}"))?;
        ctx.state.tx(phase, &id, &r)?;
        println!("  {pid}: executed -> status {}", status(pid)?["status"]);
        for ev in r.get_events("instantiate") {
            let (Some(a), Some(c)) = (
                ev.get_first_attribute_value("_contract_address"),
                ev.get_first_attribute_value("code_id"),
            ) else {
                continue;
            };
            if core_sum.is_empty() || c.parse::<u64>().ok() != Some(CODE_CORE) {
                continue;
            }
            let info = d.wasm_querier().contract_info(&Addr::unchecked(&a))?;
            let mut label = None;
            for salt in [
                "steward-akash",
                "custody-akash",
                "steward-juno",
                "bounty-subdao",
            ] {
                let want =
                    cw_orch_kms::instantiate2_bech32("terp", &core_sum, &fbytes, salt.as_bytes())
                        .map_err(|e| anyhow!(e))?;
                if want == a {
                    label = Some(salt.to_string());
                }
            }
            let name: Value = d
                .wasm_querier()
                .smart_query(&Addr::unchecked(&a), &json!({"config": {}}))
                .unwrap_or(Value::Null);
            let label = match label {
                Some(l) => {
                    println!(
                        "  {l} = {a} (matches instantiate2 prediction; name {})",
                        name["name"]
                    );
                    l
                }
                None if pid == 10 => bail!("{a} (proposal 10) matches no steward prediction"),
                None => {
                    println!("  fiab-subdao = {a} (name {})", name["name"]);
                    "fiab-subdao".to_string()
                }
            };
            ctx.state.contracts.insert(
                label,
                ContractRec {
                    address: a,
                    code: "dao-dao-core".into(),
                    admin: info.admin.map(|x| x.to_string()),
                    tx: Some(r.txhash.clone()),
                },
            );
        }
        ctx.state.save()?;
        signer.note(&format!("ok {id}"));
    }
    Ok(())
}

/// `oracle_pricing` (terp_oracle::OraclePricing) for raffle and svg-minter instantiate.
/// Default: `{enforce:false, alt_assets:[]}` (oracle queries answer, payment unchanged).
/// `--oracle-alt-assets <file>`: JSON array of AltAsset; every feed `oracle` equal to
/// "$AGGREGATOR" is replaced with the deployed aggregator. `--oracle-enforce` turns enforcement on.
/// Artifacts built before the field existed are left alone (TODO printed).
fn oracle_pricing(ctx: &Ctx, code: &str, msg: &mut Value) -> Result<()> {
    let bytes = read(&ctx.dir, file_of(code)?)?;
    if !bytes.windows(14).any(|w| w == b"oracle_pricing") {
        println!(
            "  TODO oracle_pricing: {code} artifact has no oracle_pricing field yet; instantiating without it"
        );
        return Ok(());
    }
    let agg = ctx.addr_or_predict("oracle-aggregator", "oracle-aggregator")?;
    let mut alts: Value = match &ctx.cli.oracle_alt_assets {
        Some(f) => serde_json::from_str(
            &std::fs::read_to_string(f).with_context(|| format!("{}", f.display()))?,
        )?,
        None => json!([]),
    };
    if let Some(list) = alts.as_array_mut() {
        for a in list {
            if let Some(feeds) = a["feeds"].as_array_mut() {
                for f in feeds {
                    if f["oracle"] == "$AGGREGATOR" {
                        f["oracle"] = json!(agg);
                    }
                }
            }
        }
    } else {
        bail!("--oracle-alt-assets must be a JSON array of AltAsset");
    }
    let n = alts.as_array().map(|a| a.len()).unwrap_or(0);
    if ctx.cli.oracle_enforce && n == 0 {
        bail!("--oracle-enforce needs --oracle-alt-assets");
    }
    msg["oracle_pricing"] =
        json!({"enforce": ctx.cli.oracle_enforce, "alt_assets": alts, "policy": {}});
    println!(
        "  {code}: oracle_pricing enforce={} alt_assets={n} (aggregator {agg})",
        ctx.cli.oracle_enforce
    );
    Ok(())
}

fn bps_decimal(bps: u64) -> String {
    format!("{}", cosmwasm_std::Decimal::from_ratio(bps, 10_000u64))
}

fn oracles(ctx: &mut Ctx) -> Result<()> {
    let admin = ctx.cli.admin.clone();
    let registry = ctx.addr_or_predict("oracle-registry", "oracle-registry")?;
    let steps = vec![
        Step::Inst {
            name: "oracle-registry".into(),
            code: "oracle-registry".into(),
            admin: Some(admin.clone()),
            msg: json!({"admin": admin, "open_submit": !ctx.cli.registry_closed, "bounty": null, "note": ctx.cli.registry_note}),
        },
        Step::Inst {
            name: "oracle-aggregator".into(),
            code: "oracle-aggregator".into(),
            admin: Some(admin.clone()),
            msg: json!({"registry": registry}),
        },
        Step::Inst {
            name: "oracle-twap-probe".into(),
            code: "oracle-twap-probe".into(),
            admin: Some(admin.clone()),
            msg: json!({"mid": ctx.cli.twap_mid}),
        },
    ];
    execute_steps(ctx, "oracles", steps)
}

fn nftfi(ctx: &mut Ctx) -> Result<()> {
    let a = ctx.cli.admin.clone();
    let d = ctx.cli.market_denom.clone();
    let fee = ctx.cli.nftfi_fee.clone();
    let inst = |name: &str, msg: Value| Step::Inst {
        name: name.into(),
        code: name.into(),
        admin: Some(a.clone()),
        msg,
    };
    let mut raffle_msg = json!({"name": "terp-raffle", "owner": a, "fee_addr": a, "minimum_raffle_duration": null,
        "max_tickets_per_raffle": null, "raffle_fee": fee, "creation_coins": null, "randomness": {"seed": {"provider": a}}});
    oracle_pricing(ctx, "raffle", &mut raffle_msg)?;
    let steps = vec![
        inst(
            "marketplace",
            json!({"fee_manager": a, "royalty_registry": null, "protocol_fee_bps": ctx.cli.fee_bps, "max_royalty_fee_bps": 1000,
            "maker_reward_bps": 0, "taker_reward_bps": 0, "burn_bps": 0, "default_denom": d}),
        ),
        inst(
            "reserve-auction",
            json!({"fee_manager": a, "trading_fee_percent": bps_decimal(ctx.cli.fee_bps), "min_bid_increment_percent": "0.05",
            "min_duration": 60, "max_duration": 3600, "extend_duration": 60, "create_auction_fee": {"denom": d, "amount": "0"},
            "max_auctions_to_settle_per_block": 10, "halt_duration_threshold": 3600, "halt_buffer_duration": 60,
            "halt_postpone_duration": 60, "burn_bps": 0, "min_reserve_prices": [{"denom": d, "amount": "1"}]}),
        ),
        inst(
            "svg-auction",
            json!({"protocol_fee_bps": ctx.cli.fee_bps, "min_increment_bps": 100, "min_reserve_price": "1", "duration": 3600,
            "extension": 60, "accepted_denom": [d], "protocol_addr": a}),
        ),
        inst("raffle", raffle_msg),
        inst(
            "nft-loan",
            json!({"name": "terp-nft-loan", "owner": a, "treasury_addr": a, "fee_rate": fee, "listing_fee_coins": null}),
        ),
    ];
    execute_steps(ctx, "nftfi", steps)
}

fn authenticators(ctx: &mut Ctx) -> Result<()> {
    // Codes come from upload. Per-account instances are created by each account when it
    // registers (MsgAddAuthenticator, CosmwasmAuthenticatorV1 {"contract": ...}).
    for c in ["passkey", "recovery"] {
        if ctx.state.codes.contains_key(c) && !ctx.code_on_chain(c)? {
            bail!("{c} code in state is not on chain");
        }
        println!("  {c} code {:?}", ctx.state.codes.get(c).map(|r| r.code_id));
    }
    let mut steps = Vec::new();
    if ctx.cli.smoke && ctx.cli.signer != SignerKind::LocalTest {
        println!("  --smoke is local-test only; skipped (upload-only phase)");
    } else if ctx.cli.smoke {
        steps.push(Step::Custom { id: "authenticators:smoke".into(), what: "throwaway account: instantiate2 terp-recovery (no admin), MsgAddAuthenticator CosmwasmAuthenticatorV1".into() });
    }
    execute_steps(ctx, "authenticators", steps)
}

fn smoke(ctx: &mut Ctx, phase: &str, d: &Daemon) -> Result<()> {
    if ctx.cli.signer != SignerKind::LocalTest {
        bail!(
            "the smoke test signs as the throwaway account itself; run it with --signer local-test on a rehearsal chain"
        );
    }
    let me = d.sender_addr().to_string();
    let name = "smoke-recovery";
    if !ctx.contract_on_chain(name) {
        let code_id = ctx.state.code_id("recovery")?;
        let msg = json!({"config": {"guardians": [{"address": me, "pubkey": null, "commitment": null}], "threshold": 1,
            "hash_alg": "sha256", "rehash_rounds": 1, "domain": "", "allow_address_only_approvals": true}});
        let r = d.instantiate2(
            code_id,
            &msg,
            Some(name),
            None,
            &[],
            Binary::from(salt(name)),
        )?;
        let address = r
            .instantiated_contract_address()
            .map_err(|e| anyhow!("{e}"))?
            .to_string();
        ctx.state.contracts.insert(
            name.into(),
            ContractRec {
                address,
                code: "recovery".into(),
                admin: None,
                tx: Some(r.txhash.clone()),
            },
        );
        ctx.state.tx(phase, "smoke instantiate2 recovery", &r)?;
    }
    let contract = ctx.state.addr(name).unwrap();
    // terp-recovery rejects OnAuthenticatorAdded without params, so build the
    // CosmwasmAuthenticatorV1 data with an (empty-object) params blob.
    let data = json!({"contract": contract, "params": Binary::from(b"{}".to_vec())}).to_string();
    let r = d.rt_handle.block_on(d.sender().commit_tx_any(
        vec![cosmrs::Any {
            type_url: "/terp.smartaccount.v1beta1.MsgAddAuthenticator".into(),
            value: msg_add_authenticator(&me, "CosmwasmAuthenticatorV1", data.as_bytes()),
        }],
        Some("smoke add authenticator"),
    ))?;
    if r.code != 0 {
        bail!("MsgAddAuthenticator failed: {}", r.raw_log);
    }
    ctx.state.done.insert("authenticators:smoke".into());
    ctx.state.tx(phase, "smoke MsgAddAuthenticator", &r)?;
    println!("  smoke: {me} registered CosmwasmAuthenticatorV1 -> {contract}");
    Ok(())
}

fn svg(ctx: &mut Ctx) -> Result<()> {
    if ctx.cli.skip_code.iter().any(|s| s == "svg-minter") {
        println!("  SKIPPED: svg-minter was excluded via --skip-code (rehearsal only)");
        return Ok(());
    }
    let admin = ctx.cli.admin.clone();
    // Owner/operators: the kms granter (rotatable cold key), never the ephemeral session key.
    let granter = operational_owner(ctx, "svg")?;
    let owner = ctx.cli.svg_owner.clone().unwrap_or_else(|| granter.clone());
    let factory = ctx.addr_or_predict("svg-minter", "svg-minter")?;
    let svg_code = ctx
        .state
        .codes
        .get("cw721-svg")
        .map(|c| c.code_id)
        .unwrap_or(0);
    let mut minter_msg = json!({"owner": granter, "svg_code_id": svg_code, "dregg_ibc_only": false, "runtime_minters": []});
    oracle_pricing(ctx, "svg-minter", &mut minter_msg)?;
    let mut steps = vec![Step::Inst {
        name: "svg-minter".into(),
        code: "svg-minter".into(),
        admin: Some(admin.clone()),
        msg: minter_msg,
    }];
    let creators = drop_creators(ctx)?;
    for (series, cfg) in [
        (
            MARAIN_SERIES,
            collection::marain_series(vec![owner.clone(), creators[0].1.clone()]),
        ),
        (
            BENDERS_SERIES,
            collection::benders_series(vec![owner.clone(), creators[1].1.clone()]),
        ),
    ] {
        steps.push(Step::ExecAs {
            id: format!("svg:series:{series}"),
            contract: factory.clone(),
            key: "set_series".into(),
            msg: json!({"set_series": {"series": series, "config": cfg}}),
        });
    }
    let cr = drops::Creators {
        withdraw: &admin,
        marain: &creators[0].1,
        benders: &creators[1].1,
        yijing: &creators[2].1,
    };
    for drop in drops::initial_collections_with(&cr, &ctx.cli.market_denom)? {
        let mut m = drop.msg.clone();
        let (key, body) = match drop.name {
            MARAIN_ORBS => (
                "create_series_collection",
                json!({"series": MARAIN_SERIES, "label": m.name, "instantiate_msg": m}),
            ),
            KARA_GIPPY => {
                m.collection_info_extension.mint_end_time = Some(Timestamp::from_seconds(86_400));
                (
                    "create_series_collection",
                    json!({"series": BENDERS_SERIES, "label": m.name, "instantiate_msg": m}),
                )
            }
            YIJING_HARMONY => (
                "create_svg_collection",
                json!({"label": m.name, "instantiate_msg": m}),
            ),
            other => bail!("no factory path for {other}"),
        };
        steps.push(Step::ExecAs {
            id: format!("svg:create:{}", drop.name),
            contract: factory.clone(),
            key: key.into(),
            msg: json!({key: body}),
        });
    }
    let handoff = owner != granter;
    if handoff {
        steps.push(Step::ExecAs { id: "svg:offer-ownership".into(), contract: factory.clone(), key: "update_ownership".into(),
            msg: json!({"update_ownership": {"transfer_ownership": {"new_owner": owner, "expiry": null}}}) });
    }
    execute_steps(ctx, "svg", steps)?;
    write_update_admin(ctx, &creators)?;
    let p = ctx.cli.out.join("svg_accept_ownership.json");
    if handoff && ctx.state.done.contains("svg:offer-ownership") {
        let accept = json!({"note": format!("{owner} sends this itself (MsgExecuteContract, or MsgExec via a grant)"),
            "contract": factory, "msg": {"update_ownership": "accept_ownership"}, "funds": []});
        std::fs::create_dir_all(&ctx.cli.out)?;
        std::fs::write(&p, serde_json::to_string_pretty(&accept)?)?;
        println!("[svg] --svg-owner accept-ownership msg -> {}", p.display());
    } else if p.exists() {
        // The granter owns the factory directly; no Foundation acceptance is needed.
        std::fs::remove_file(&p)?;
        println!(
            "[svg] removed stale {} (factory owned by the granter)",
            p.display()
        );
    }
    Ok(())
}

/// (drop name, creator address) for the three initial drops. Required when broadcasting.
fn drop_creators(ctx: &Ctx) -> Result<Vec<(&'static str, String)>> {
    let c = &ctx.cli;
    let mut out = Vec::new();
    for (name, v, flag) in [
        (MARAIN_ORBS, &c.creator_marain, "--creator-marain"),
        (KARA_GIPPY, &c.creator_benders, "--creator-benders"),
        (YIJING_HARMONY, &c.creator_yijing, "--creator-yijing"),
    ] {
        let a = match v {
            Some(a) if !a.is_empty() => a.clone(),
            _ if name == YIJING_HARMONY => c.admin.clone(),
            _ if c.broadcast => {
                bail!("{flag} (creator key address for {name}) is required with --broadcast")
            }
            _ => format!("<{}>", flag.trim_start_matches("--")),
        };
        if a == c.admin && name != YIJING_HARMONY {
            bail!("{flag}: the curator must be its own key, not the admin");
        }
        out.push((name, a));
    }
    Ok(out)
}

/// The factory sets wasm admin = creator. MsgUpdateAdmin is never granted to a script session
/// (cw-orch-kms policy), so each creator key sends it itself.
fn write_update_admin(ctx: &Ctx, creators: &[(&'static str, String)]) -> Result<()> {
    let mut msgs = Vec::new();
    for (name, creator) in creators {
        if creator == &ctx.cli.admin {
            continue;
        }
        if let Some(addr) = ctx.state.addr(&format!("drop:{name}")) {
            let key = if *name == MARAIN_ORBS {
                "marain"
            } else {
                "benders"
            };
            msgs.push(json!({"drop": name, "signer": creator, "msg": {"@type": "/cosmwasm.wasm.v1.MsgUpdateAdmin",
                "sender": creator, "new_admin": ctx.cli.admin, "contract": addr},
                "cw_orch": format!("TERP_SCRIPT_BROADCAST=1 cargo run --bin svg_curator -- handoff --name {key} --contract {addr} --admin {} --broadcast <same --network/--grpc/--rpc/--lcd>", ctx.cli.admin)}));
        }
    }
    if msgs.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(&ctx.cli.out)?;
    let p = ctx.cli.out.join("svg_update_admin.json");
    std::fs::write(
        &p,
        serde_json::to_string_pretty(
            &json!({"note": "each curator key signs its own MsgUpdateAdmin via svg_curator handoff (cw-orch); not a session step", "txs": msgs}),
        )?,
    )?;
    println!(
        "[svg] creator -> {} admin handoff msgs -> {}",
        ctx.cli.admin,
        p.display()
    );
    Ok(())
}

fn verify(ctx: &mut Ctx) -> Result<()> {
    let q = ctx.query()?;
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (name, rec) in &ctx.state.contracts {
        let info = q
            .wasm_querier()
            .contract_info(&Addr::unchecked(&rec.address));
        let (admin, code_id) = match &info {
            Ok(i) => (i.admin.as_ref().map(|a| a.to_string()), Some(i.code_id)),
            Err(_) => (None, None),
        };
        let expect = if ADMINLESS.contains(&name.as_str()) {
            "adminless"
        } else if name.starts_with("drop:") {
            "admin (after curator handoff)"
        } else {
            "admin"
        };
        let ok = match expect {
            "adminless" => admin.is_none(),
            "admin (after curator handoff)" => admin.as_deref() == Some(ctx.cli.admin.as_str()),
            _ => admin.as_deref() == Some(ctx.cli.admin.as_str()),
        };
        if !ok || info.is_err() {
            failures.push(format!("{name}: admin {admin:?}, expected {expect}"));
        }
        rows.push(json!({"name": name, "address": rec.address, "code": rec.code, "code_id": code_id, "admin": admin, "expected": expect, "ok": ok, "tx": rec.tx}));
    }
    if let Ok(creators) = drop_creators(ctx) {
        for (name, want) in creators {
            let Some(a) = ctx.state.addr(&format!("drop:{name}")) else {
                continue;
            };
            let a = Addr::unchecked(&a);
            let own = q
                .wasm_querier()
                .smart_query::<_, Value>(&a, &json!({"get_creator_ownership": {}}))
                .unwrap_or(Value::Null);
            let wd = q
                .wasm_querier()
                .smart_query::<_, Value>(&a, &json!({"get_withdraw_address": {}}))
                .unwrap_or(Value::Null);
            if own["owner"].as_str() != Some(want.as_str()) {
                failures.push(format!(
                    "drop:{name}: creator {}, expected {want}",
                    own["owner"]
                ));
            }
            if wd.as_str() != Some(ctx.cli.admin.as_str()) {
                failures.push(format!(
                    "drop:{name}: withdraw address {wd}, expected {}",
                    ctx.cli.admin
                ));
            }
            println!("  drop:{name} creator {} withdraw {wd}", own["owner"]);
        }
    }
    let mut owner = Value::Null;
    if let Some(f) = ctx.state.addr("svg-minter") {
        owner = q
            .wasm_querier()
            .smart_query::<_, Value>(&Addr::unchecked(&f), &json!({"ownership": {}}))
            .unwrap_or(Value::Null);
        let want = ctx
            .cli
            .svg_owner
            .clone()
            .or_else(|| (!ctx.cli.granter.is_empty()).then(|| ctx.cli.granter.clone()));
        if let Some(want) = want {
            let ok = owner["owner"].as_str() == Some(want.as_str())
                || owner["pending_owner"].as_str() == Some(want.as_str());
            if !ok {
                failures.push(format!("svg-minter: ownership {owner}, expected {want}"));
            }
            let cur = drop_creators(ctx).ok();
            for (i, series) in [MARAIN_SERIES, BENDERS_SERIES].into_iter().enumerate() {
                let mut wants = vec![want.clone()];
                if let Some(c) = &cur {
                    wants.push(c[i].1.clone());
                }
                let info = q
                    .wasm_querier()
                    .smart_query::<_, Value>(
                        &Addr::unchecked(&f),
                        &json!({"series": {"series": series}}),
                    )
                    .unwrap_or(Value::Null);
                let ops = info
                    .pointer("/config/operators")
                    .cloned()
                    .unwrap_or(Value::Null);
                if ops != json!(wants) {
                    failures.push(format!(
                        "svg-minter series {series}: operators {ops}, expected {wants:?}"
                    ));
                }
                println!("  series {series} operators {ops}");
            }
            println!("  svg-minter ownership {owner}");
        }
    }
    let gas: u64 = ctx.state.txs.iter().map(|t| t.gas_used).sum();
    let report = json!({"chain_id": ctx.info.chain_id, "admin": ctx.cli.admin, "codes": ctx.state.codes.iter().map(|(k, v)| (k.clone(), json!({"code_id": v.code_id, "checksum": v.checksum, "tx": v.tx}))).collect::<BTreeMap<_, _>>(),
        "external_codes": ctx.state.external_codes, "previous_codes_superseded": ctx.state.previous_codes, "contracts": rows, "svg_factory_ownership": owner,
        "tx_count": ctx.state.txs.len(), "gas_used_total": gas, "txs": ctx.state.txs, "failures": failures});
    std::fs::create_dir_all(&ctx.cli.out)?;
    let p = ctx.cli.out.join("deploy_report.json");
    std::fs::write(&p, serde_json::to_string_pretty(&report)?)?;
    println!(
        "[verify] report -> {} ({} contracts, {} failures)",
        p.display(),
        report["contracts"].as_array().map(|a| a.len()).unwrap_or(0),
        failures.len()
    );
    if !failures.is_empty() {
        bail!("verify failed:\n{}", failures.join("\n"));
    }
    Ok(())
}

/// Polytone v3 codes 78/79/80 are reused from state (no gov proposal, no store). Channel
/// creation needs the relayer, so this phase only checks the codes and points at polytone_osmosis.
fn polytone(ctx: &mut Ctx) -> Result<()> {
    let q = ctx.query()?;
    for (name, id) in ctx.state.external_codes.clone() {
        let c = q
            .wasm_querier()
            .code(id)
            .map_err(|e| anyhow!("{name} code {id}: {e}"))?;
        println!(
            "  {name} code {id} checksum {}",
            hex_lower(c.checksum.as_slice())
        );
    }
    println!(
        "[polytone] next: polytone_osmosis (dry-run) once hermes + the connection exist  [APPROVAL]"
    );
    Ok(())
}

// ---------------------------------------------------------------- main

fn local_guard(cli: &Cli) -> Result<()> {
    let g = cli.grpc.clone().unwrap_or_default();
    if !(g.contains("127.0.0.1") || g.contains("localhost")) {
        bail!("--signer local-test needs a 127.0.0.1 --grpc");
    }
    if ["morocco-1", "120u-1"].contains(&cli.network.as_str()) {
        bail!("--signer local-test refused on {}", cli.network);
    }
    Ok(())
}

fn fund_local(cli: &Cli, addr: &str) -> Result<()> {
    let Some(from) = &cli.fund_from else {
        return Ok(());
    };
    let mut args = vec![
        "tx",
        "bank",
        "send",
        from.as_str(),
        addr,
        "100000000000uthiol,100000000000uterp",
        "--keyring-backend",
        "test",
        "--chain-id",
        cli.network.as_str(),
        "--node",
        cli.terpd_node.as_str(),
        "--fees",
        "20000uthiol",
        "-y",
        "--output",
        "json",
    ];
    if !cli.terpd_home.is_empty() {
        args.extend(["--home", cli.terpd_home.as_str()]);
    }
    let out = std::process::Command::new(&cli.terpd)
        .args(&args)
        .output()?;
    if !out.status.success() {
        bail!("fund {addr}: {}", String::from_utf8_lossy(&out.stderr));
    }
    std::thread::sleep(std::time::Duration::from_secs(4));
    println!("funded local-test key {addr}");
    Ok(())
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    cw_orch_kms::validate_address(&cli.admin, "terp").map_err(|e| anyhow!("--admin: {e}"))?;
    let selected: Vec<&str> = match (cli.phase.as_str(), &cli.only, &cli.from) {
        ("all", Some(o), _) => vec![
            PHASES
                .iter()
                .copied()
                .find(|p| p == o)
                .ok_or_else(|| anyhow!("unknown phase {o}"))?,
        ],
        ("all", None, Some(f)) => {
            let i = PHASES
                .iter()
                .position(|p| p == f)
                .ok_or_else(|| anyhow!("unknown phase {f}"))?;
            PHASES[i..].to_vec()
        }
        ("all", None, None) => PHASES.to_vec(),
        (p, _, _) => vec![
            PHASES
                .iter()
                .chain(EXTRA_PHASES)
                .copied()
                .find(|x| *x == p)
                .ok_or_else(|| anyhow!("unknown phase {p}"))?,
        ],
    };
    let info = match &cli.grpc {
        Some(g) => kms_gate::chain_info(
            &cli.network,
            g,
            &cli.gas_denom,
            cli.gas_price,
            "terp",
            if cli.signer == SignerKind::LocalTest {
                ChainKind::Local
            } else {
                ChainKind::Mainnet
            },
        ),
        None => kms_gate::morocco_info(),
    };
    let info = kms_gate::with_http_fallback(info, cli.rpc.as_deref(), cli.lcd.as_deref());
    println!(
        "broadcast fallback: rpc {:?} lcd {:?}",
        info.rpc_urls, info.lcd_url
    );
    if cli.signer == SignerKind::LocalTest {
        local_guard(&cli)?;
    }
    if cli.broadcast && cli.chain_id != info.chain_id {
        bail!(
            "broadcast refused: --chain-id {:?} != {}",
            cli.chain_id,
            info.chain_id
        );
    }
    if cli.broadcast && std::env::var(kms_gate::BROADCAST_ENV).ok().as_deref() != Some("1") {
        bail!("broadcast refused: set {}=1", kms_gate::BROADCAST_ENV);
    }
    if cli.broadcast
        && info.chain_id != "morocco-1"
        && std::env::var(kms_gate::BROADCAST_CHAIN_ENV).ok().as_deref()
            != Some(info.chain_id.as_str())
    {
        bail!(
            "broadcast refused: also set {}={}",
            kms_gate::BROADCAST_CHAIN_ENV,
            info.chain_id
        );
    }
    let state = State::load(&cli.state, &info.chain_id, &cli.admin)?;
    let mut ctx = Ctx {
        dir: artifacts_dir(),
        info: info.clone(),
        state,
        creator: None,
        daemon_builder: None,
        local: None,
        gas_by_phase: BTreeMap::new(),
        cli,
    };

    // The signing key (ephemeral grantee / local-test key) is generated in-process and never printed.
    if ctx.cli.broadcast {
        cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|e| anyhow!(e))?;
        let mut b = Daemon::builder(info.clone());
        // Never merge ~/.cw-orchestrator/networks.toml: it pins morocco-1 to 127.0.0.1:9090.
        b.load_network(false);
        if let Some(g) = &ctx.cli.grpc {
            b.grpc_url(g);
        }
        if ctx.cli.signer == SignerKind::LocalTest {
            b.is_test(true);
        }
        if ctx.cli.signer == SignerKind::Kms {
            cw_orch_kms::set_approval_chain(kms_gate::kms_profile(&info))
                .map_err(|e| anyhow!(e))?;
        }
        let d = b.build()?;
        ctx.creator = Some(d.sender_addr().to_string());
        println!("signer {}", d.sender_addr());
        if ctx.cli.signer == SignerKind::LocalTest {
            if !selected.iter().all(|p| *p == "verify" || *p == "preflight") {
                fund_local(&ctx.cli, d.sender_addr().as_str())?;
            }
            ctx.local = Some(d);
        }
        ctx.daemon_builder = Some(b);
    }

    for phase in selected {
        println!("== phase {phase}");
        let before = (
            ctx.state.txs.len(),
            ctx.state.txs.iter().map(|t| t.gas_used).sum::<u64>(),
        );
        match phase {
            "preflight" => preflight(&mut ctx)?,
            "upload" => upload(&mut ctx)?,
            "dao-upload" => dao_upload(&mut ctx)?,
            "hs-upload" => hs_upload(&mut ctx)?,
            "bounty-upload" => bounty_upload(&mut ctx)?,
            "daos" => daos(&mut ctx)?,
            "vote" | "dao-vote" | "dao-execute" => vote(&mut ctx, phase)?,
            "dao-propose" => dao_propose(&mut ctx)?,
            "chunk-upload" => chunk_upload(&mut ctx)?,
            "circuit-propose" | "gov-propose" => gov_propose(&mut ctx, phase)?,
            "gov-vote" => gov_vote(&mut ctx)?,
            "oracles" => oracles(&mut ctx)?,
            "nftfi" => nftfi(&mut ctx)?,
            "authenticators" => authenticators(&mut ctx)?,
            "svg" => svg(&mut ctx)?,
            "verify" => verify(&mut ctx)?,
            "polytone" => polytone(&mut ctx)?,
            _ => unreachable!(),
        }
        let after = (
            ctx.state.txs.len(),
            ctx.state.txs.iter().map(|t| t.gas_used).sum::<u64>(),
        );
        ctx.gas_by_phase.insert(
            phase.into(),
            ((after.0 - before.0) as u64, after.1 - before.1),
        );
    }
    println!("== summary (this run)");
    for (p, (n, g)) in &ctx.gas_by_phase {
        println!("  {p}: {n} tx, {g} gas");
    }
    Ok(())
}

/// MsgAddAuthenticator { sender = 1, authenticator_type = 2, data = 3 } protobuf encoding.
fn msg_add_authenticator(sender: &str, kind: &str, data: &[u8]) -> Vec<u8> {
    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    let mut out = Vec::new();
    for (tag, bytes) in [
        (0x0au8, sender.as_bytes()),
        (0x12, kind.as_bytes()),
        (0x1a, data),
    ] {
        out.push(tag);
        varint(bytes.len() as u64, &mut out);
        out.extend_from_slice(bytes);
    }
    out
}

// ---------------------------------------------------------------- x/gov: gov-propose / circuit-propose / gov-vote

const MSG_SUBMIT_PROPOSAL: &str = "/cosmos.gov.v1.MsgSubmitProposal";
const MSG_DEPOSIT: &str = "/cosmos.gov.v1.MsgDeposit";
const MSG_GOV_VOTE: &str = "/cosmos.gov.v1.MsgVote";
const MSG_STORE_FULL_CIRCUIT: &str = "/cosmwasm.wasm.v1.MsgStoreFullCircuit";
const MSG_STORE_CIRCUIT: &str = "/cosmwasm.wasm.v1.MsgStoreCircuit";

/// Minimal protobuf wire writer/reader for messages with no Rust proto type in this tree
/// (the zk circuit msgs live only in the permissionlessweb wasmd fork, proto/cosmwasm/wasm/v1/tx.proto).
mod pb {
    use anyhow::{Result, bail};
    pub fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    fn key(field: u32, wt: u64, out: &mut Vec<u8>) {
        varint(((field as u64) << 3) | wt, out)
    }
    /// proto3 scalar: zero is omitted (same as gogoproto Marshal).
    pub fn uint(field: u32, v: u64, out: &mut Vec<u8>) {
        if v != 0 {
            key(field, 0, out);
            varint(v, out)
        }
    }
    pub fn bytes(field: u32, b: &[u8], out: &mut Vec<u8>) {
        if !b.is_empty() {
            key(field, 2, out);
            varint(b.len() as u64, out);
            out.extend_from_slice(b)
        }
    }
    /// embedded message: always emitted when present (non-nil pointer / non-nullable).
    pub fn msg(field: u32, b: &[u8], out: &mut Vec<u8>) {
        key(field, 2, out);
        varint(b.len() as u64, out);
        out.extend_from_slice(b)
    }
    pub enum Val<'a> {
        V(u64),
        B(&'a [u8]),
    }
    fn read_varint(buf: &[u8], i: &mut usize) -> Result<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let Some(&b) = buf.get(*i) else {
                bail!("truncated varint")
            };
            *i += 1;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift > 63 {
                bail!("varint overflow")
            }
        }
    }
    pub fn fields(buf: &[u8]) -> Result<Vec<(u32, Val<'_>)>> {
        let mut i = 0;
        let mut out = Vec::new();
        while i < buf.len() {
            let k = read_varint(buf, &mut i)?;
            let (f, wt) = ((k >> 3) as u32, k & 7);
            match wt {
                0 => out.push((f, Val::V(read_varint(buf, &mut i)?))),
                2 => {
                    let n = read_varint(buf, &mut i)? as usize;
                    if i + n > buf.len() {
                        bail!("truncated field {f}")
                    }
                    out.push((f, Val::B(&buf[i..i + n])));
                    i += n;
                }
                _ => bail!("field {f}: unsupported wire type {wt}"),
            }
        }
        Ok(out)
    }
}

fn json_u64(v: &Value, what: &str) -> Result<u64> {
    match v {
        Value::Null => Ok(0),
        Value::Number(n) => n.as_u64().ok_or_else(|| anyhow!("{what}: not a u64")),
        Value::String(s) => s.parse().map_err(|e| anyhow!("{what}: {s:?}: {e}")),
        _ => bail!("{what}: not a u64"),
    }
}
fn json_b64(v: &Value, what: &str) -> Result<Vec<u8>> {
    let s = v
        .as_str()
        .ok_or_else(|| anyhow!("{what}: base64 string required"))?;
    Ok(Binary::from_base64(s)
        .map_err(|e| anyhow!("{what}: {e}"))?
        .to_vec())
}
fn only_keys(m: &Value, allowed: &[&str], what: &str) -> Result<()> {
    for k in m
        .as_object()
        .ok_or_else(|| anyhow!("{what}: not an object"))?
        .keys()
    {
        if !allowed.contains(&k.as_str()) {
            bail!("{what}: field {k:?} has no encoder (refusing to drop it)");
        }
    }
    Ok(())
}

/// Canonical JSON form of a gov message (what encode() writes and decode() reads back).
fn gov_msg_canonical(m: &Value) -> Result<Value> {
    let t = m["@type"]
        .as_str()
        .ok_or_else(|| anyhow!("message without @type"))?;
    let b64 = |b: Vec<u8>| Binary::from(b).to_base64();
    Ok(match t {
        MSG_STORE_FULL_CIRCUIT => {
            only_keys(m, &["@type", "sender", "auth", "params_body", "vk_body"], t)?;
            only_keys(&m["auth"], &["circuit_type", "curve_type", "k"], "auth")?;
            json!({"@type": t, "sender": m["sender"].as_str().unwrap_or_default(),
                "auth": {"circuit_type": json_u64(&m["auth"]["circuit_type"], "auth.circuit_type")?.to_string(),
                         "curve_type": json_u64(&m["auth"]["curve_type"], "auth.curve_type")?.to_string(),
                         "k": json_u64(&m["auth"]["k"], "auth.k")?.to_string()},
                "params_body": b64(json_b64(&m["params_body"], "params_body")?), "vk_body": b64(json_b64(&m["vk_body"], "vk_body")?)})
        }
        MSG_STORE_CIRCUIT => {
            only_keys(
                m,
                &[
                    "@type",
                    "sender",
                    "param_key",
                    "vk_body",
                    "instantiate_permission",
                ],
                t,
            )?;
            if !m["instantiate_permission"].is_null() {
                bail!(
                    "{t}: instantiate_permission has no encoder; use {{\"@type\",\"@value_b64\"}}"
                );
            }
            json!({"@type": t, "sender": m["sender"].as_str().unwrap_or_default(),
                "param_key": json_u64(&m["param_key"], "param_key")?.to_string(), "vk_body": b64(json_b64(&m["vk_body"], "vk_body")?)})
        }
        _ => {
            only_keys(m, &["@type", "@value_b64"], t).map_err(|_| anyhow!("{t}: no encoder; pass it as {{\"@type\": \"{t}\", \"@value_b64\": <proto bytes>}}"))?;
            json!({"@type": t, "@value_b64": b64(json_b64(&m["@value_b64"], "@value_b64")?)})
        }
    })
}

fn gov_msg_encode(m: &Value) -> Result<cosmos_sdk_proto::Any> {
    let c = gov_msg_canonical(m)?;
    let t = c["@type"].as_str().unwrap().to_string();
    let mut out = Vec::new();
    match t.as_str() {
        MSG_STORE_FULL_CIRCUIT => {
            // MsgStoreFullCircuit { sender = 1, auth = 2 CircuitParamAuth { circuit_type = 1, curve_type = 2, k = 3 }, params_body = 3, vk_body = 4 }
            let mut auth = Vec::new();
            for (f, k) in [(1, "circuit_type"), (2, "curve_type"), (3, "k")] {
                pb::uint(f, json_u64(&c["auth"][k], k)?, &mut auth);
            }
            pb::bytes(1, c["sender"].as_str().unwrap().as_bytes(), &mut out);
            pb::msg(2, &auth, &mut out);
            pb::bytes(3, &json_b64(&c["params_body"], "params_body")?, &mut out);
            pb::bytes(4, &json_b64(&c["vk_body"], "vk_body")?, &mut out);
        }
        MSG_STORE_CIRCUIT => {
            // MsgStoreCircuit { sender = 1, param_key = 2, vk_body = 3, instantiate_permission = 4 }
            pb::bytes(1, c["sender"].as_str().unwrap().as_bytes(), &mut out);
            pb::uint(2, json_u64(&c["param_key"], "param_key")?, &mut out);
            pb::bytes(3, &json_b64(&c["vk_body"], "vk_body")?, &mut out);
        }
        _ => out = json_b64(&c["@value_b64"], "@value_b64")?,
    }
    Ok(cosmos_sdk_proto::Any {
        type_url: t,
        value: out,
    })
}

/// Independent wire decode back to canonical JSON (round-trip check).
fn gov_msg_decode(a: &cosmos_sdk_proto::Any) -> Result<Value> {
    use pb::Val;
    let s = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|e| anyhow!("{e}"));
    let b64 = |b: &[u8]| Binary::from(b.to_vec()).to_base64();
    Ok(match a.type_url.as_str() {
        MSG_STORE_FULL_CIRCUIT => {
            let (mut sender, mut auth, mut params, mut vk) =
                (String::new(), [0u64; 3], String::new(), String::new());
            for (f, v) in pb::fields(&a.value)? {
                match (f, v) {
                    (1, Val::B(b)) => sender = s(b)?,
                    (2, Val::B(b)) => {
                        for (g, w) in pb::fields(b)? {
                            match (g, w) {
                                (1..=3, Val::V(x)) => auth[g as usize - 1] = x,
                                (g, _) => bail!("auth: unexpected field {g}"),
                            }
                        }
                    }
                    (3, Val::B(b)) => params = b64(b),
                    (4, Val::B(b)) => vk = b64(b),
                    (f, _) => bail!("{}: unexpected field {f}", a.type_url),
                }
            }
            json!({"@type": a.type_url, "sender": sender, "auth": {"circuit_type": auth[0].to_string(), "curve_type": auth[1].to_string(), "k": auth[2].to_string()},
                "params_body": params, "vk_body": vk})
        }
        MSG_STORE_CIRCUIT => {
            let (mut sender, mut key, mut vk) = (String::new(), 0u64, String::new());
            for (f, v) in pb::fields(&a.value)? {
                match (f, v) {
                    (1, Val::B(b)) => sender = s(b)?,
                    (2, Val::V(x)) => key = x,
                    (3, Val::B(b)) => vk = b64(b),
                    (f, _) => bail!("{}: unexpected field {f}", a.type_url),
                }
            }
            json!({"@type": a.type_url, "sender": sender, "param_key": key.to_string(), "vk_body": vk})
        }
        t => {
            pb::fields(&a.value)
                .map_err(|e| anyhow!("{t}: @value_b64 is not valid protobuf: {e}"))?;
            json!({"@type": t, "@value_b64": b64(&a.value)})
        }
    })
}

fn gov_option(choice: &str) -> Result<i32> {
    Ok(match choice {
        "yes" => 1,
        "abstain" => 2,
        "no" => 3,
        "veto" | "no_with_veto" => 4,
        c => bail!("gov vote {c:?}: yes | no | abstain | veto"),
    })
}

fn gov_pid(ctx: &Ctx, id: &str, of: &Option<String>, pid: &Option<u64>) -> Result<u64> {
    match (pid, of) {
        (Some(p), _) => Ok(*p),
        (None, Some(o)) => ctx
            .state
            .proposals
            .get(o)
            .copied()
            .ok_or_else(|| anyhow!("{id}: proposal of {o} unknown")),
        _ => bail!("{id}: no proposal"),
    }
}

/// Generic x/gov proposal from a pinned MsgSubmitProposal JSON (`circuit-propose` is the same phase).
fn gov_propose(ctx: &mut Ctx, phase: &str) -> Result<()> {
    let f = ctx
        .cli
        .proposal_file
        .clone()
        .ok_or_else(|| anyhow!("{phase}: --proposal-file required"))?;
    let bytes = std::fs::read(&f).map_err(|e| anyhow!("{}: {e}", f.display()))?;
    let got = hex_lower(&Sha256::digest(&bytes));
    let want = ctx
        .cli
        .proposal_sha
        .clone()
        .ok_or_else(|| anyhow!("{phase}: --proposal-sha required"))?;
    if want.to_lowercase() != got {
        bail!(
            "refused before any grant: {} sha256 {got} != --proposal-sha {want} (nothing was broadcast)",
            f.display()
        );
    }
    let body: Value = serde_json::from_slice(&bytes)?;
    if body["@type"] != MSG_SUBMIT_PROPOSAL {
        bail!(
            "{phase}: {} is not a {MSG_SUBMIT_PROPOSAL} (got {})",
            f.display(),
            body["@type"]
        );
    }
    only_keys(
        &body,
        &[
            "@type",
            "messages",
            "initial_deposit",
            "proposer",
            "metadata",
            "title",
            "summary",
            "expedited",
        ],
        "MsgSubmitProposal",
    )?;
    let msgs = body["messages"]
        .as_array()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| anyhow!("{phase}: no messages"))?;
    let title = body["title"].as_str().unwrap_or_default().to_string();
    if title.is_empty() {
        bail!("{phase}: empty title");
    }
    println!(
        "  {} sha256 {got}: {} message(s), title {title:?}",
        f.display(),
        msgs.len()
    );

    let prefix = ctx.info.network_info.pub_address_prefix.clone();
    let q = ctx.query()?;
    let gov_addr = kms_gate::query_module_account(&q, "gov", &prefix)?;
    let mut anys = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        let a = gov_msg_encode(m)?;
        let back = gov_msg_decode(&a)?;
        if back != gov_msg_canonical(m)? {
            bail!("message {i} ({}): proto round-trip mismatch", a.type_url);
        }
        for k in ["sender", "authority"] {
            if let Some(s) = m[k].as_str() {
                if s != gov_addr {
                    bail!(
                        "message {i}: {k} {s} is not the gov module {gov_addr} (x/gov executes as itself)"
                    );
                }
            }
        }
        let extra = match a.type_url.as_str() {
            MSG_STORE_FULL_CIRCUIT => format!(
                " auth {} params {}B vk {}B",
                back["auth"],
                json_b64(&back["params_body"], "")?.len(),
                json_b64(&back["vk_body"], "")?.len()
            ),
            _ => String::new(),
        };
        println!(
            "  message {i}: {} {}B proto, round-trip ok, sender = gov {gov_addr}{extra}",
            a.type_url,
            a.value.len()
        );
        anys.push(a);
    }

    let coins = body["initial_deposit"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if coins.len() != 1 {
        bail!("{phase}: initial_deposit must be exactly one coin");
    }
    let denom = coins[0]["denom"].as_str().unwrap_or_default().to_string();
    let initial: u128 = coins[0]["amount"]
        .as_str()
        .unwrap_or_default()
        .parse()
        .map_err(|e| anyhow!("initial_deposit amount: {e}"))?;
    let params = gov_deposit_params!(q.channel(), &q.rt_handle)?;
    let expedited = body["expedited"].as_bool().unwrap_or(false);
    let mins = if expedited {
        &params.expedited_min_deposit
    } else {
        &params.min_deposit
    };
    let min: u128 = mins
        .iter()
        .find(|c| c.denom == denom)
        .ok_or_else(|| anyhow!("{phase}: {denom} is not a gov deposit denom ({mins:?})"))?
        .amount
        .parse()?;
    let ratio =
        <cosmwasm_std::Decimal as std::str::FromStr>::from_str(&params.min_initial_deposit_ratio)
            .map_err(|e| anyhow!("min_initial_deposit_ratio: {e}"))?;
    let need_init = cosmwasm_std::Uint128::new(min).mul_ceil(ratio).u128();
    println!(
        "  gov params: min_deposit {min}{denom}{}, min_initial_deposit_ratio {ratio} -> initial >= {need_init}{denom}; file initial_deposit {initial}{denom}",
        if expedited { " (expedited)" } else { "" }
    );
    if initial < need_init {
        bail!("refused before any grant: initial_deposit {initial}{denom} < {need_init}{denom}");
    }
    let top_up = if !ctx.cli.no_top_up && initial < min {
        min - initial
    } else {
        0
    };

    let signer = operational_owner(ctx, phase)?;
    let file_proposer = body["proposer"].as_str().unwrap_or_default().to_string();
    let proposer = if file_proposer == signer {
        signer.clone()
    } else if ctx.cli.set_proposer || signer == "<kms-granter>" {
        println!("  proposer {file_proposer} -> {signer} (--set-proposer)");
        signer.clone()
    } else {
        bail!(
            "refused before any grant: file proposer {file_proposer} != signer {signer}; pass --set-proposer to submit as {signer}"
        );
    };
    if signer != "<kms-granter>" {
        let have = bank_balance!(q.channel(), &q.rt_handle, &signer, &denom)?;
        if have < initial + top_up {
            bail!(
                "refused before any grant: {signer} has {have}{denom}, deposit needs {}{denom} (initial {initial} + top-up {top_up})",
                initial + top_up
            );
        }
        println!(
            "  {signer}: {have}{denom} >= deposit {}{denom}; it remains after: {}{denom}",
            initial + top_up,
            have - initial - top_up
        );
    }
    drop(q);

    let m = cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal {
        messages: anys,
        initial_deposit: vec![cosmos_sdk_proto::cosmos::base::v1beta1::Coin {
            denom: denom.clone(),
            amount: initial.to_string(),
        }],
        proposer,
        metadata: body["metadata"].as_str().unwrap_or_default().into(),
        title: title.clone(),
        summary: body["summary"].as_str().unwrap_or_default().into(),
        expedited,
    };
    let msg = cosmos_sdk_proto::prost::Message::encode_to_vec(&m);
    let id = format!("{phase}:{}", &got[..16]);
    let mut steps = vec![Step::GovSubmit {
        id: id.clone(),
        title,
        msg,
        deposit: format!("{initial}{denom}"),
    }];
    if top_up > 0 {
        steps.push(Step::GovDeposit {
            id: format!("{id}:top-up"),
            of: id.clone(),
            amount: top_up,
            denom,
        });
    }
    if let Some(c) = ctx.cli.vote_choice.clone() {
        gov_option(&c)?;
        steps.push(Step::GovVote {
            id: format!("{id}:vote"),
            of: Some(id.clone()),
            pid: None,
            choice: c,
        });
    }
    if ctx.cli.wait_pass {
        steps.push(Step::GovWait {
            id: format!("{id}:wait"),
            of: Some(id.clone()),
            pid: None,
            wait_secs: ctx.cli.wait_secs,
        });
    }
    execute_steps(ctx, phase, steps)
}

fn gov_vote(ctx: &mut Ctx) -> Result<()> {
    let c = ctx
        .cli
        .vote_choice
        .clone()
        .ok_or_else(|| anyhow!("gov-vote: --vote yes|no|abstain|veto required"))?;
    gov_option(&c)?;
    if ctx.cli.vote_proposals.is_empty() {
        bail!("gov-vote: --proposals required");
    }
    let q = ctx.query()?;
    let mut steps = Vec::new();
    for pid in ctx.cli.vote_proposals.clone() {
        let p = gov_proposal!(q.channel(), &q.rt_handle, pid)?;
        if p.status != 2 {
            bail!(
                "refused before any grant: gov proposal {pid} is not in its voting period (status {})",
                p.status
            );
        }
        println!("  gov proposal {pid}: {:?} in voting period", p.title);
        steps.push(Step::GovVote {
            id: format!("gov-vote:{pid}:{c}"),
            of: None,
            pid: Some(pid),
            choice: c.clone(),
        });
    }
    drop(q);
    if ctx.cli.wait_pass {
        for pid in ctx.cli.vote_proposals.clone() {
            steps.push(Step::GovWait {
                id: format!("gov-vote:{pid}:wait"),
                of: None,
                pid: Some(pid),
                wait_secs: ctx.cli.wait_secs,
            });
        }
    }
    execute_steps(ctx, "gov-vote", steps)
}

/// dao-propose templates: replace {{K}} inside wasm msgs (base64 JSON), title and description.
/// `"{{K}}"` (quoted) becomes a JSON number when V is an integer, else a JSON string.
fn substitute_template(
    title: &str,
    description: &str,
    msgs: Vec<Value>,
    params: &[(String, String)],
) -> Result<(String, String, Vec<Value>)> {
    let lit = |v: &str| {
        if v.parse::<u64>().is_ok() {
            v.to_string()
        } else {
            Value::String(v.into()).to_string()
        }
    };
    let mut used: BTreeSet<String> = BTreeSet::new();
    let sub = |s: &str, used: &mut BTreeSet<String>, quoted: bool| -> String {
        let mut s = s.to_string();
        for (k, v) in params {
            let ph = format!("{{{{{k}}}}}");
            if quoted {
                let q = format!("\"{ph}\"");
                if s.contains(&q) {
                    used.insert(k.clone());
                    s = s.replace(&q, &lit(v));
                }
            }
            if s.contains(&ph) {
                used.insert(k.clone());
                s = s.replace(&ph, v);
            }
        }
        s
    };
    let title = sub(title, &mut used, false);
    let description = sub(description, &mut used, false);
    let mut out = Vec::new();
    for mut m in msgs {
        for path in [
            ["wasm", "execute"],
            ["wasm", "instantiate"],
            ["wasm", "instantiate2"],
            ["wasm", "migrate"],
        ] {
            // non-inserting lookup: IndexMut would create {"msg":null} objects for absent kinds
            let b = match m
                .get(path[0])
                .and_then(|w| w.get(path[1]))
                .and_then(|k| k.get("msg"))
                .and_then(|v| v.as_str())
            {
                Some(b) => b.to_string(),
                None => continue,
            };
            {
                let b = b.as_str();
                let raw = String::from_utf8(
                    Binary::from_base64(b).map_err(|e| anyhow!("{e}"))?.to_vec(),
                )?;
                let new = sub(&raw, &mut used, true);
                if new.contains("{{") {
                    bail!(
                        "dao-propose: unresolved {{{{placeholder}}}} in a {} msg: {new} (pass --param / --zk-id)",
                        path[1]
                    );
                }
                if new != raw {
                    let _: Value = serde_json::from_str(&new).map_err(|e| {
                        anyhow!("template msg after substitution is not JSON: {e}: {new}")
                    })?;
                    println!(
                        "  template: {} {} -> {new}",
                        path[1],
                        m[path[0]][path[1]]["contract_addr"].as_str().unwrap_or("")
                    );
                    m[path[0]][path[1]]["msg"] =
                        Value::String(Binary::from(new.into_bytes()).to_base64());
                }
            }
        }
        if m.to_string().contains("{{") {
            bail!("dao-propose: unresolved {{{{placeholder}}}} in a msg (pass --param / --zk-id)");
        }
        out.push(m);
    }
    if title.contains("{{") || description.contains("{{") {
        bail!("dao-propose: unresolved {{{{placeholder}}}} in title/description");
    }
    for (k, _) in params {
        if !used.contains(k) {
            bail!("dao-propose: --param {k} matches no {{{{{k}}}}} in the proposal");
        }
    }
    Ok((title, description, out))
}

#[cfg(test)]
mod gov_tests {
    use super::*;
    #[test]
    fn store_full_circuit_round_trip() {
        let m = json!({"@type": MSG_STORE_FULL_CIRCUIT, "sender": "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq",
            "auth": {"circuit_type": "0", "curve_type": "0", "k": "11"}, "params_body": "AAEC", "vk_body": "/w=="});
        let a = gov_msg_encode(&m).unwrap();
        // 0a <len> sender | 12 02 18 0b | 1a 03 00 01 02 | 22 01 ff
        let mut want = vec![0x0a, 43];
        want.extend_from_slice(b"terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq");
        want.extend_from_slice(&[
            0x12, 0x02, 0x18, 0x0b, 0x1a, 0x03, 0, 1, 2, 0x22, 0x01, 0xff,
        ]);
        assert_eq!(a.value, want);
        assert_eq!(gov_msg_decode(&a).unwrap(), gov_msg_canonical(&m).unwrap());
    }
    #[test]
    fn template_zk_id() {
        let msg =
            Binary::from(br#"{"bind_note_circuit":{"zk_id":"{{ZK_ID}}"}}"#.to_vec()).to_base64();
        let (t, _, m) = substitute_template(
            "bind {{ZK_ID}}",
            "",
            vec![json!({"wasm": {"execute": {"contract_addr": "x", "msg": msg, "funds": []}}})],
            &[("ZK_ID".into(), "7".into())],
        )
        .unwrap();
        assert_eq!(t, "bind 7");
        let raw = Binary::from_base64(m[0]["wasm"]["execute"]["msg"].as_str().unwrap()).unwrap();
        assert_eq!(raw.as_slice(), br#"{"bind_note_circuit":{"zk_id":7}}"#);
        assert!(
            m[0]["wasm"].get("instantiate2").is_none(),
            "substitution must not insert absent msg kinds"
        );
        assert!(
            substitute_template(
                "t",
                "",
                vec![json!({"wasm": {"execute": {"contract_addr": "x", "msg": msg, "funds": []}}})],
                &[]
            )
            .is_err()
        );
    }
}
