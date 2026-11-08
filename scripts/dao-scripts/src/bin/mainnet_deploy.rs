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
const EXTRA_PHASES: &[&str] = &["polytone", "dao-upload", "bounty-upload", "vote"];

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
    /// vote phase: choice to cast (only `yes`).
    #[arg(long = "vote", default_value = "yes")]
    vote_choice: String,
    /// vote phase: Foundation proposal ids, e.g. `9,10`.
    #[arg(long = "proposals", value_delimiter = ',')]
    vote_proposals: Vec<u64>,
    /// vote phase: after voting, execute every listed proposal whose status is `passed`.
    #[arg(long = "execute")]
    vote_execute: bool,
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
            Step::ExecAs {
                id,
                contract,
                key,
                msg,
            } => json!({"execute_as_granter": id, "contract": contract, "key": key, "msg": msg}),
            Step::Vote { id, module, of } => {
                json!({"vote_as_granter": id, "module": module, "proposal_of": of, "vote": "yes"})
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
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .ok_or_else(|| anyhow!("unknown code {name}"))
}

fn crate_of(name: &str) -> Option<&'static str> {
    terp_scripts::ibc::CODE_SOURCE_TABLE
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, p)| *p)
}

fn git_stamp_at_store(repo: &Path) -> (Option<String>, Option<bool>) {
    let v = terp_scripts::ibc::git_build_stamp(repo);
    (
        v.get("commit").and_then(|c| c.as_str()).map(str::to_string),
        v.get("dirty").and_then(|d| d.as_bool()),
    )
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
                Step::ExecAs { id, .. } | Step::Vote { id, .. } | Step::Custom { id, .. } => {
                    self.state.done.contains(id)
                }
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
    if phase == "vote" && ctx.cli.vote_execute {
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
                e.0.insert("vote".into());
                e.1 += ctx.cli.vote_proposals.len() as u64;
                if ctx.cli.vote_execute {
                    e.0.insert("execute".into());
                    e.1 += ctx.cli.vote_proposals.len() as u64;
                }
            }
            Step::Inst { .. } | Step::Custom { .. } => {
                session = session.allow_instantiate2();
            }
            Step::ExecAs { contract, key, .. } => {
                let e = execs.entry(contract.clone()).or_default();
                e.0.insert(key.clone());
                e.1 += 1;
            }
            Step::Vote { module, .. } => {
                let e = execs.entry(module.clone()).or_default();
                e.0.insert("vote".into());
                e.1 += 1;
            }
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
    session.ttl_secs = ctx.cli.ttl_secs;
    session.account_index = ctx.cli.account_index;
    session.allowance_limit = planned_spend_limit(steps.len() as u64 + 1);
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
            Step::ExecAs { id, .. } | Step::Vote { id, .. } | Step::Custom { id, .. } => {
                ctx.cli.steps_filter.contains(id)
            }
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
            Step::ExecAs {
                id,
                contract,
                key,
                msg,
            } => {
                let granter = signer
                    .granter()
                    .ok_or_else(|| anyhow!("{id}: needs a kms session (granter)"))?;
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
                let granter = signer
                    .granter()
                    .ok_or_else(|| anyhow!("{id}: needs a kms session (granter)"))?;
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
fn vote(ctx: &mut Ctx) -> Result<()> {
    if ctx.cli.vote_choice != "yes" {
        bail!("--vote: only `yes` is supported");
    }
    if ctx.cli.vote_proposals.is_empty() {
        bail!("--proposals 9,10 required");
    }
    let pre = ctx.cli.foundation_pre_propose.clone();
    let q = ctx.query()?;
    let module: String = q
        .wasm_querier()
        .smart_query(&Addr::unchecked(&pre), &json!({"proposal_module": {}}))
        .map_err(|e| anyhow!("proposal_module: {e}"))?;
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
        "  proposal module {module}; execute={}",
        ctx.cli.vote_execute
    );
    drop(q);
    execute_steps(
        ctx,
        "vote",
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
    let granter = signer
        .granter()
        .ok_or_else(|| anyhow!("vote: needs a kms session (granter)"))?;
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
    for pid in ctx.cli.vote_proposals.clone() {
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
            &json!({"vote": {"proposal_id": pid, "vote": "yes", "rationale": null}}),
        )?;
        let r = d
            .rt_handle
            .block_on(d.sender().commit_tx_any(vec![any], Some(id.as_str())))
            .map_err(|e| anyhow!("{id}: {e}"))?;
        ctx.state.tx(phase, &id, &r)?;
        let p = status(pid)?;
        println!(
            "  {pid}: voted yes -> status {} votes {}",
            p["status"], p["votes"]
        );
        signer.note(&format!("ok {id}"));
    }
    if !ctx.cli.vote_execute {
        return Ok(());
    }
    let core_sum = d
        .wasm_querier()
        .code(CODE_CORE)?
        .checksum
        .as_slice()
        .to_vec();
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
            if c.parse::<u64>().ok() != Some(CODE_CORE) {
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
            "bounty-upload" => bounty_upload(&mut ctx)?,
            "daos" => daos(&mut ctx)?,
            "vote" => vote(&mut ctx)?,
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
