//! Shared runtime gate for dao-scripts binaries.
//!
//! Every script builds its messages, prints them, and stops (dry-run is the
//! default). `--broadcast` only proceeds when:
//! 1. `--chain-id` matches the configured chain info,
//! 2. the gRPC node reports that same chain-id (cw-orch `GrpcChannel::connect`
//!    refuses a mismatch, and the session re-checks it),
//! 3. `TERP_SCRIPT_BROADCAST=1` is set,
//! 4. a cw-orch-kms session is opened against a cold-key granter captured with
//!    the `cw-orch-granter:v2` challenge bound to that chain-id.
//! Messages are signed by the ephemeral cw-orch-kms key and wrapped in
//! `MsgExec` for the granter, through the cw-orch Daemon. No terpd CLI, no
//! offline JSON for another tool to sign.
//!
//! UpdateAdmin: no script here requests `/cosmwasm.wasm.v1.MsgUpdateAdmin`.
//! Wasm admins are set at instantiate (FIAB: Foundation; steward/custody: the x/gov
//! module account, queried from chain). Changing one is a DAO proposal, or a
//! separate session that names the contract with `allow_sdk` on purpose.

use anyhow::{anyhow, bail, Context, Result};
use cosmrs::Any;
use cw_orch::daemon::{Daemon, DaemonBuilder, TxSender};
use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
use cw_orch_kms::{enable_session, planned_spend_limit, ApprovalSession, SigningRequest};
use sha2::{Digest, Sha256};

pub const BROADCAST_ENV: &str = "TERP_SCRIPT_BROADCAST";
/// Chains other than morocco-1 need this set to the exact chain-id as well as
/// `TERP_SCRIPT_BROADCAST=1`. Without both, scripts stay dry-run.
pub const BROADCAST_CHAIN_ENV: &str = "TERP_SCRIPT_BROADCAST_CHAIN";
pub const GRANTER_PAGE: &str = "127.0.0.1:8741";

/// Chain info for a cosmos chain reached over gRPC.
pub fn chain_info(
    chain_id: &str,
    grpc: &str,
    gas_denom: &str,
    gas_price: f64,
    prefix: &str,
    kind: ChainKind,
) -> ChainInfoOwned {
    ChainInfoOwned {
        chain_id: chain_id.into(),
        gas_denom: gas_denom.into(),
        gas_price,
        grpc_urls: vec![grpc.into()],
        rpc_urls: vec![],
        lcd_url: None,
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: prefix.into(),
            pub_address_prefix: prefix.into(),
            coin_type: 118,
        },
        kind,
    }
}

/// morocco-1 defaults. `MOROCCO_GRPC` overrides the endpoint.
pub fn morocco_info() -> ChainInfoOwned {
    let grpc = std::env::var("MOROCCO_GRPC").unwrap_or_else(|_| "http://192.168.10.101:9090".into());
    chain_info("morocco-1", &grpc, "uthiol", 0.05, "terp", ChainKind::Mainnet)
}

/// Read-only daemon. Fails when the node's chain-id is not `info.chain_id`.
/// Read-only presets for chains a script may target besides Terp. Endpoints
/// come from env; there is no default public endpoint.
pub fn osmosis_info() -> ChainInfoOwned {
    let grpc = std::env::var("OSMOSIS_GRPC").unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    chain_info("osmosis-1", &grpc, "uosmo", 0.025, "osmo", ChainKind::Mainnet)
}

pub fn juno_info() -> ChainInfoOwned {
    let grpc = std::env::var("JUNO_GRPC").unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    chain_info("juno-1", &grpc, "ujuno", 0.075, "juno", ChainKind::Mainnet)
}

pub fn atomone_info() -> ChainInfoOwned {
    let grpc = std::env::var("ATOMONE_GRPC").unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    chain_info("atomone-1", &grpc, "uatone", 0.025, "atone", ChainKind::Mainnet)
}

/// Pick a chain by id, e.g. `osmosis-1` for an `osmo1…` granter.
pub fn info_for_chain(chain_id: &str) -> Result<ChainInfoOwned> {
    match chain_id {
        "morocco-1" => Ok(morocco_info()),
        "osmosis-1" => Ok(osmosis_info()),
        "juno-1" => Ok(juno_info()),
        "atomone-1" => Ok(atomone_info()),
        other => bail!("unknown chain {other:?}: build it with chain_info(..)"),
    }
}

/// cw-orch-kms profile for a script chain. Prefix, denom, gas and coin type
/// come from the chain info; Zigner's chain_name comes from the preset.
pub fn kms_profile(info: &ChainInfoOwned) -> cw_orch_kms::ChainProfile {
    let prefix = info.network_info.pub_address_prefix.clone();
    let mut p = cw_orch_kms::ChainProfile::for_prefix(&prefix).unwrap_or_else(|| {
        cw_orch_kms::ChainProfile::new(&info.chain_id, &prefix, &info.gas_denom, info.gas_price, &prefix, &prefix)
    });
    p.chain_id = info.chain_id.clone();
    p.fee_denom = info.gas_denom.clone();
    p.gas_price = info.gas_price;
    p.coin_type = info.network_info.coin_type;
    p
}

pub fn query_only(info: &ChainInfoOwned) -> Result<cw_orch::daemon::senders::QueryOnlyDaemon> {
    // load_network(false): ~/.cw-orchestrator/networks.toml must not override the
    // gRPC endpoint chosen here (it pinned morocco-1 to 127.0.0.1:9090).
    DaemonBuilder::new(info.clone())
        .load_network(false)
        .build_sender(())
        .map_err(|e| anyhow!("read-only connect to {} ({:?}) refused: {e}", info.chain_id, info.grpc_urls))
}

/// Module account address as the chain reports it (auth `ModuleAccountByName`),
/// cross-checked against the local `NewModuleAddress` derivation.
pub fn query_module_account(d: &cw_orch::daemon::senders::QueryOnlyDaemon, name: &str, prefix: &str) -> Result<String> {
    use cosmos_sdk_proto::prost::Message;
    use cosmrs::proto::cosmos::auth::v1beta1::{query_client::QueryClient, QueryModuleAccountByNameRequest};
    let resp = d
        .rt_handle
        .block_on(QueryClient::new(d.channel()).module_account_by_name(QueryModuleAccountByNameRequest { name: name.into() }))
        .map_err(|e| anyhow!("module account {name}: {e}"))?
        .into_inner();
    let any = resp.account.ok_or_else(|| anyhow!("module account {name}: empty response"))?;
    let acct = cosmos_sdk_proto::cosmos::auth::v1beta1::ModuleAccount::decode(any.value.as_slice())?;
    let addr = acct.base_account.map(|b| b.address).unwrap_or_default();
    let derived = module_address(name, prefix)?;
    if addr != derived {
        bail!("module account {name}: chain reports {addr}, derivation gives {derived}");
    }
    Ok(addr)
}

/// Address of a cosmos-sdk module account (`authtypes.NewModuleAddress`).
pub fn module_address(name: &str, prefix: &str) -> Result<String> {
    let hash = Sha256::digest(name.as_bytes());
    Ok(bech32::encode::<bech32::Bech32>(bech32::Hrp::parse(prefix)?, &hash[..20])?)
}

pub fn any<M: cosmos_sdk_proto::prost::Message>(type_url: &str, msg: &M) -> Any {
    Any { type_url: type_url.into(), value: msg.encode_to_vec() }
}

/// What a script asks the session to allow.
pub enum Scope {
    Sdk(Vec<String>),
    Execute(cw_orch_kms::ScopedCall),
    Instantiate2,
}

pub struct BroadcastRequest {
    pub info: ChainInfoOwned,
    /// `--chain-id` from the operator. Must equal `info.chain_id`.
    pub expected_chain_id: String,
    pub granter: String,
    pub purpose: String,
    pub account_index: u32,
    pub ttl_secs: u64,
    pub scope: Scope,
    pub summary: String,
}

/// Pre-flight checks that do not touch a key. Used by tests and dry-runs.
pub fn check_broadcast_preconditions(req: &BroadcastRequest, env_flag: Option<&str>) -> Result<()> {
    check_broadcast_preconditions_for(req, env_flag, std::env::var(BROADCAST_CHAIN_ENV).ok().as_deref())
}

pub fn check_broadcast_preconditions_for(
    req: &BroadcastRequest,
    env_flag: Option<&str>,
    chain_flag: Option<&str>,
) -> Result<()> {
    if req.expected_chain_id.is_empty() || req.expected_chain_id != req.info.chain_id {
        bail!(
            "broadcast refused: --chain-id {:?} does not match configured chain {:?}",
            req.expected_chain_id,
            req.info.chain_id
        );
    }
    if env_flag != Some("1") {
        bail!("broadcast refused: set {BROADCAST_ENV}=1 after reviewing the dry-run");
    }
    if req.info.chain_id != "morocco-1" && chain_flag != Some(req.info.chain_id.as_str()) {
        bail!(
            "broadcast refused: {} is not morocco-1; also set {BROADCAST_CHAIN_ENV}={} after reviewing the dry-run",
            req.info.chain_id,
            req.info.chain_id
        );
    }
    if !req.granter.is_empty() {
        cw_orch_kms::validate_address(&req.granter, &req.info.network_info.pub_address_prefix)
            .map_err(|e| anyhow!("broadcast refused: {e}"))?;
    }
    cw_orch_kms::validate_ttl(req.ttl_secs).map_err(|e| anyhow!(e))?;
    Ok(())
}

/// Gate, open a cw-orch-kms session, broadcast `msgs` through the Daemon, revoke.
/// Returns the tx hash.
pub fn kms_broadcast(req: BroadcastRequest, msgs: Vec<Any>, memo: &str) -> Result<String> {
    check_broadcast_preconditions(&req, std::env::var(BROADCAST_ENV).ok().as_deref())?;
    // Node chain-id check before anything is signed.
    query_only(&req.info)?;
    cw_orch_kms::ensure_ephemeral_mnemonic(req.info.kind.clone()).map_err(|e| anyhow!(e))?;
    cw_orch_kms::set_approval_chain(kms_profile(&req.info)).map_err(|e| anyhow!(e))?;
    let granter = if req.granter.is_empty() {
        println!("sign the cw-orch-granter:v2 challenge at http://{GRANTER_PAGE}");
        cw_orch_kms::capture_granter_for_chain_sync(
            GRANTER_PAGE.parse()?,
            req.account_index,
            &req.purpose,
            &req.info.chain_id,
        )
        .map_err(|e| anyhow!(e))?
    } else {
        cw_orch_kms::set_approval_chain_id(&req.info.chain_id).map_err(|e| anyhow!(e))?;
        req.granter.clone()
    };
    let mut session = ApprovalSession::open(&granter);
    session = match req.scope {
        Scope::Sdk(urls) => {
            let mut s = session;
            for url in urls {
                s = s.allow_sdk(url).map_err(|e| anyhow!(e))?;
            }
            s
        }
        Scope::Execute(call) => session.allow_execute(call).map_err(|e| anyhow!(e))?,
        Scope::Instantiate2 => session.allow_instantiate2(),
    };
    session.ttl_secs = req.ttl_secs;
    session.account_index = req.account_index;
    session.allowance_limit = planned_spend_limit(1);
    let mut builder: DaemonBuilder = Daemon::builder(req.info.clone());
    builder.load_network(false);
    let run = enable_session(
        &builder,
        session,
        vec![SigningRequest::new("session-key").body(req.summary.clone())],
    )
    .map_err(|e| anyhow!(e))?;
    let result = run
        .daemon
        .rt_handle
        .block_on(run.daemon.sender().commit_tx_any(msgs, Some(memo)))
        .context("broadcast through cw-orch Daemon");
    let (daemon, grantee) = (run.daemon.clone(), run.grantee().to_string());
    let closed = run.close().map_err(|e| anyhow!(e));
    let resp = result?;
    closed?;
    // Nothing may outlive the flow: assert on-chain that the revoke landed.
    let left = cw_orch_kms::query_grantee_state(&daemon, &granter, &grantee).map_err(|e| anyhow!(e))?;
    if !left.authz.is_empty() || left.feegrant.is_some() {
        bail!(
            "session revoke incomplete: {} authz grant(s), feegrant {} remain for {grantee}",
            left.authz.len(),
            left.feegrant.is_some()
        );
    }
    Ok(resp.txhash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(expected: &str) -> BroadcastRequest {
        BroadcastRequest {
            info: morocco_info(),
            expected_chain_id: expected.into(),
            granter: String::new(),
            purpose: "test".into(),
            account_index: 0,
            ttl_secs: 600,
            scope: Scope::Sdk(vec![]),
            summary: String::new(),
        }
    }

    #[test]
    fn broadcast_refuses_chain_mismatch_and_missing_env() {
        assert!(check_broadcast_preconditions(&req("terp-1"), Some("1")).is_err());
        assert!(check_broadcast_preconditions(&req(""), Some("1")).is_err());
        assert!(check_broadcast_preconditions(&req("morocco-1"), None).is_err());
        assert!(check_broadcast_preconditions(&req("morocco-1"), Some("1")).is_ok());
    }

    fn req_on(info: ChainInfoOwned, granter: &str) -> BroadcastRequest {
        let mut r = req(&info.chain_id.clone());
        r.info = info;
        r.granter = granter.into();
        r
    }

    #[test]
    fn other_chains_need_their_own_flag() {
        let osmo = module_address("x", "osmo").unwrap();
        let r = req_on(osmosis_info(), &osmo);
        assert!(check_broadcast_preconditions_for(&r, Some("1"), None).is_err());
        assert!(check_broadcast_preconditions_for(&r, Some("1"), Some("morocco-1")).is_err());
        assert!(check_broadcast_preconditions_for(&r, None, Some("osmosis-1")).is_err());
        assert!(check_broadcast_preconditions_for(&r, Some("1"), Some("osmosis-1")).is_ok());
        // terp granter on osmosis is refused
        let bad = req_on(osmosis_info(), &module_address("x", "terp").unwrap());
        assert!(check_broadcast_preconditions_for(&bad, Some("1"), Some("osmosis-1")).is_err());
        // morocco-1 keeps the single flag
        assert!(check_broadcast_preconditions_for(&req("morocco-1"), Some("1"), None).is_ok());
    }

    #[test]
    fn chain_profiles_follow_info() {
        for (id, prefix, denom, zigner) in [
            ("morocco-1", "terp", "uthiol", "terp"),
            ("osmosis-1", "osmo", "uosmo", "osmosis"),
            ("juno-1", "juno", "ujuno", "juno"),
            ("atomone-1", "atone", "uatone", "atomone"),
        ] {
            let p = kms_profile(&info_for_chain(id).unwrap());
            assert_eq!((p.chain_id.as_str(), p.bech32_prefix.as_str(), p.fee_denom.as_str(), p.zigner_chain_name.as_str()), (id, prefix, denom, zigner));
            assert_eq!(p.coin_type, 118);
        }
        assert!(info_for_chain("cosmoshub-4").is_err());
    }

    #[test]
    fn gov_module_address_is_the_sdk_one() {
        // cosmos1 form of the gov module account is well known.
        assert_eq!(
            module_address("gov", "cosmos").unwrap(),
            "cosmos10d07y265gmmuvt4z0w9aw880jnsr700j6zn9kn"
        );
    }
}

/// `MsgExecuteContract` from `sender` (the granter under MsgExec).
pub fn execute_any(sender: &str, contract: &str, msg: &serde_json::Value) -> Result<Any> {
    use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
    Ok(any(
        "/cosmwasm.wasm.v1.MsgExecuteContract",
        &MsgExecuteContract {
            sender: sender.into(),
            contract: contract.into(),
            msg: serde_json::to_vec(msg)?,
            funds: vec![],
        },
    ))
}

/// Write a JSON plan next to the run and echo it.
pub fn write_plan(path: &str, plan: &serde_json::Value) -> Result<()> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let text = serde_json::to_string_pretty(plan)?;
    std::fs::write(path, &text)?;
    println!("{text}");
    Ok(())
}

/// Opt-in for the old mnemonic/keyring signers (see `legacy_signer`).
pub const LEGACY_SIGNER_ENV: &str = "TERP_SCRIPT_LEGACY_SIGNER";

/// True for env names that hold signing material. They are never loaded from
/// `.env` by default.
pub fn is_secret_name(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    ["MNEMONIC", "PRIV", "SECRET", "PASSWORD", "PRIVATE_KEY", "SEED"].iter().any(|s| u.contains(s))
}

/// Load `./.env` for endpoints and paths only. Secret-named entries
/// (`MAIN_MNEMONIC`, `*_MNEMONIC`, ...) are skipped, and variables already set
/// are not overridden. cw-orch-kms supplies the signer.
pub fn load_dotenv_public() {
    let Ok(iter) = dotenv::dotenv_iter() else { return };
    for (k, v) in iter.flatten() {
        if !is_secret_name(&k) && std::env::var_os(&k).is_none() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var(&k, &v)
            };
        }
    }
}

/// Legacy fallback: a script that still signs with a mnemonic Daemon runs only
/// when `TERP_SCRIPT_LEGACY_SIGNER=1` is set; then the full `.env` is loaded.
/// Default is refusal, pointing at the cw-orch-kms replacement.
pub fn legacy_signer(bin: &str, replacement: &str) -> Result<()> {
    legacy_signer_with(bin, replacement, std::env::var(LEGACY_SIGNER_ENV).ok().as_deref())
}

pub fn legacy_signer_with(bin: &str, replacement: &str, flag: Option<&str>) -> Result<()> {
    if flag != Some("1") {
        bail!(
            "{bin} signs with a mnemonic from .env, which is no longer the default. \
Use {replacement}, or opt in with {LEGACY_SIGNER_ENV}=1."
        );
    }
    eprintln!("WARNING: {bin} legacy mnemonic signer ({LEGACY_SIGNER_ENV}=1)");
    dotenv::dotenv().ok();
    Ok(())
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    #[test]
    fn secret_names_are_filtered_and_legacy_needs_opt_in() {
        for k in ["MAIN_MNEMONIC", "DAO_MEMBER1_MNEMONIC", "TEST_MNEMONIC", "priv_key"] {
            assert!(is_secret_name(k), "{k}");
        }
        for k in ["MOROCCO_GRPC", "ARTIFACTS_DIR", "TERP_LCD"] {
            assert!(!is_secret_name(k), "{k}");
        }
        assert!(legacy_signer_with("x", "y", None).is_err());
        assert!(legacy_signer_with("x", "y", Some("true")).is_err());
    }
}
