//! Store, instantiate, and set up the bounty and auction contracts.
//!
//! One cold signature. Instantiate2 fixes each contract address from the wasm
//! checksum, the ephemeral creator, and a salt, with an empty init message in
//! the derivation. The signature therefore grants store of those checksums,
//! `MsgInstantiateContract2`, and `update_config` once on the predicted bounty
//! address. The ephemeral key does that work, then close revokes the grant.
//!
//! Leave `--granter` empty. The key that signs `cw-orch-granter:v1` is the
//! granter. On groot-wan, pass `--spawn`.
//!
//! Ownership: with `--bounty-dao <terp1..>` the bounty DAO is the wasm admin
//! AND the contract owner/approver/protocol/fee manager of all three contracts
//! from instantiate. The ephemeral key never holds ownership. Because the DAO
//! owns the bounty, `update_config` is emitted as DAO proposal JSON, not executed.
//! After each instantiate the on-chain contract-info admin is asserted == DAO.
//!
//! ```sh
//! cargo run -p dao-scripts --bin app_setup -- --bounty-dao terp1... --dry-run
//! cargo run -p dao-scripts --bin app_setup -- --spawn --bounty-dao terp1...
//! ```

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::Parser;
use cosmwasm_std::{Addr, Binary};
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch_kms::{
    enable_session, instantiate2_bech32, planned_spend_limit, wasm_checksum, ApprovalSession,
    SigningRequest, SpawnedTerp, CW_ORCH_GRANTER_ENV,
};

/// Three stores, three instantiate2, and the closing revoke (update_config is a DAO proposal).
const TXS: u64 = 7;

struct Lab<'a> {
    name: &'a str,
    file: &'a str,
    init: fn(&str) -> serde_json::Value,
}

const LABS: &[Lab] = &[
    Lab {
        name: "bounty-escrow",
        file: "terp_bounty_escrow.wasm",
        init: bounty_init,
    },
    Lab {
        name: "svg-auction",
        file: "svg_auction.wasm",
        init: svg_init,
    },
    Lab {
        name: "reserve-auction",
        file: "revenue_reserve_auction.wasm",
        init: reserve_init,
    },
];

#[derive(Parser, Debug)]
#[clap(name = "app-setup")]
struct Cli {
    /// Optional. When empty, the airgapped signature chooses the granter.
    #[clap(long, env = CW_ORCH_GRANTER_ENV, default_value = "")]
    granter: String,
    /// `terp-mainnet` / `morocco-1` or `terp-testnet` / `120u-1`. Ignored with `--spawn`.
    #[clap(long)]
    chain: Option<String>,
    /// Boot the local Terp image on this machine.
    #[clap(long)]
    spawn: bool,
    /// Refused. A session names the Wasm and the contracts it may touch.
    #[clap(long)]
    wallet: bool,
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,
    #[clap(long, default_value_t = 0u32)]
    account_index: u32,
    #[clap(long)]
    retain: bool,
    /// Bounty DAO core address. Becomes wasm admin and owner of every contract.
    #[clap(long, env = "BOUNTY_DAO")]
    bounty_dao: Option<String>,
    /// Print the plan (init msgs, admin, session scope, DAO proposal) and exit.
    /// No granter page, no chain boot, no broadcast.
    #[clap(long)]
    dry_run: bool,
}

/// Who owns what. Every role is the DAO; nothing is the ephemeral key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ownership {
    wasm_admin: String,
    owner: String,
}

fn ownership_plan(dao: &str) -> Result<Ownership> {
    cw_orch_kms::validate_terp_address(dao).map_err(|err| anyhow!("--bounty-dao: {err}"))?;
    Ok(Ownership {
        wasm_admin: dao.to_string(),
        owner: dao.to_string(),
    })
}

/// Refuses any init message that names an owner role other than the DAO.
fn assert_dao_owned(own: &Ownership, init: &serde_json::Value) -> Result<()> {
    if own.wasm_admin != own.owner {
        return Err(anyhow!("wasm admin {} != owner {}", own.wasm_admin, own.owner));
    }
    for key in ["owner", "approver", "protocol_addr", "fee_manager"] {
        if let Some(v) = init.get(key) {
            if v.as_str() != Some(own.owner.as_str()) {
                return Err(anyhow!("init {key} is {v}, expected DAO {}", own.owner));
            }
        }
    }
    Ok(())
}

fn update_config_proposal(dao: &str, bounty: &str) -> serde_json::Value {
    serde_json::json!({
        "dao": dao,
        "propose": {"title": "Bounty escrow: finalize params (lab-2)",
            "description": "Execute update_config on the bounty escrow owned by this DAO.",
            "msgs": [{"wasm": {"execute": {"contract_addr": bounty,
                "msg": base64_json(&serde_json::json!({"update_config": {"params_version": "lab-2", "params_placeholder": false}})),
                "funds": []}}}]}
    })
}

fn base64_json(v: &serde_json::Value) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(v).unwrap_or_default())
}

fn artifacts_dir() -> PathBuf {
    std::env::var("ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts"))
}

fn bounty_init(admin: &str) -> serde_json::Value {
    serde_json::json!({
        "owner": admin,
        "approver": admin,
        "gate": {"use_allowlist": true, "min_stake": null, "cw4_group": null},
        "denoms": [{"denom": "uterp", "min_amount": "1", "max_amount": "1000000000000"}],
        "allow_partial_payouts": false,
        "fee_bps": 0,
        "fee_collector": null,
        "min_duration_secs": 60,
        "max_duration_secs": 86400,
        "params_version": "lab-1",
        "params_placeholder": true,
        "min_submit_stake": "0",
        "min_upload_stake": "0"
    })
}

fn svg_init(admin: &str) -> serde_json::Value {
    serde_json::json!({
        "protocol_fee_bps": 100,
        "min_increment_bps": 100,
        "min_reserve_price": "1",
        "duration": 3600,
        "extension": 60,
        "accepted_denom": ["uterp"],
        "protocol_addr": admin
    })
}

fn reserve_init(admin: &str) -> serde_json::Value {
    serde_json::json!({
        "fee_manager": admin,
        "trading_fee_percent": "0.02",
        "min_bid_increment_percent": "0.05",
        "min_duration": 60,
        "max_duration": 3600,
        "extend_duration": 60,
        "create_auction_fee": {"denom": "uterp", "amount": "0"},
        "max_auctions_to_settle_per_block": 10,
        "halt_duration_threshold": 3600,
        "halt_buffer_duration": 60,
        "halt_postpone_duration": 60,
        "burn_bps": 0,
        "min_reserve_prices": [{"denom": "uterp", "amount": "1"}]
    })
}

fn resolve_chain(name: &str) -> Result<ChainInfo> {
    match name {
        "terp-mainnet" | "morocco-1" | "mainnet" => Ok(TERP_MAINNET),
        "terp-testnet" | "120u-1" | "testnet" => Ok(TERP_TESTNET),
        other => Err(anyhow!(
            "unknown chain {other:?}; use terp-mainnet / morocco-1 or terp-testnet / 120u-1, or --spawn"
        )),
    }
}

fn read_labs(dir: &Path) -> Result<Vec<(&Lab, Vec<u8>)>> {
    let mut out = Vec::with_capacity(LABS.len());
    for lab in LABS {
        let path = dir.join(lab.file);
        let bytes = std::fs::read(&path).map_err(|err| anyhow!("read {}: {err}", path.display()))?;
        if bytes.is_empty() {
            return Err(anyhow!("{} is empty", path.display()));
        }
        out.push((lab, bytes));
    }
    Ok(out)
}

fn store_once(chain: &Daemon, file: &str, path: &Path) -> Result<(u64, String)> {
    let wasm = WasmPath::new(path).map_err(|err| anyhow!("{file}: {err}"))?;
    let response = chain
        .rt_handle
        .block_on(cw_orch::daemon::upload_wasm_plain(
            chain.sender(),
            wasm,
            None,
        ))
        .map_err(|err| anyhow!("{file}: {err}"))?;
    let code_id = response
        .uploaded_code_id()
        .map_err(|err| anyhow!("{file}: {err}"))?;
    println!("stored {file} code {code_id} tx {}", response.txhash);
    Ok((code_id, response.txhash))
}

fn salt(name: &str) -> Vec<u8> {
    format!("app-setup/{name}").into_bytes()
}

fn canonical_account(addr: &str) -> Result<Vec<u8>> {
    let (hrp, data) = bech32::decode(addr).map_err(|err| anyhow!("creator {addr}: {err}"))?;
    if hrp.as_str() != "terp" {
        return Err(anyhow!("creator {addr} is not a terp account"));
    }
    Ok(data)
}

fn instantiate_at(
    chain: &Daemon,
    name: &str,
    code_id: u64,
    msg: &serde_json::Value,
    admin: &Addr,
    salt_bytes: &[u8],
    predicted: &str,
) -> Result<(String, String)> {
    let response = chain
        .instantiate2(
            code_id,
            msg,
            Some(name),
            Some(admin),
            &[],
            Binary::from(salt_bytes.to_vec()),
        )
        .map_err(|err| anyhow!("instantiate2 {name}: {err}"))?;
    let address = response
        .instantiated_contract_address()
        .map_err(|err| anyhow!("instantiate2 {name}: {err}"))?;
    if address.as_str() != predicted {
        return Err(anyhow!(
            "instantiate2 {name} landed on {address}, predicted {predicted}"
        ));
    }
    println!(
        "instantiated {name} {address} code {code_id} tx {}",
        response.txhash
    );
    Ok((address.to_string(), response.txhash))
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();
    let cli = Cli::parse();
    if cli.wallet {
        return Err(anyhow!(
            "--wallet is not a grant. This setup names the wasm checksums and the bounty it may configure"
        ));
    }
    if !cli.spawn && cli.chain.is_none() {
        return Err(anyhow!("pass --spawn on groot-wan, or name a chain with --chain"));
    }
    let Some(dao) = cli.bounty_dao.clone() else {
        return Err(anyhow!(
            "--bounty-dao is required: the bounty DAO must be wasm admin and owner from instantiate"
        ));
    };
    let own = ownership_plan(&dao)?;
    for lab in LABS {
        assert_dao_owned(&own, &(lab.init)(&own.owner))?;
    }
    if cli.dry_run {
        let plan = serde_json::json!({
            "dry_run": true,
            "broadcast": false,
            "wasm_admin": own.wasm_admin,
            "owner": own.owner,
            "contracts": LABS.iter().map(|l| serde_json::json!({
                "name": l.name, "file": l.file, "salt": l.name, "admin": own.wasm_admin,
                "init": (l.init)(&own.owner)})).collect::<Vec<_>>(),
            "session_scope": {"type_urls": ["/cosmwasm.wasm.v1.MsgStoreCode (checksum-pinned x3)",
                "/cosmwasm.wasm.v1.MsgInstantiateContract2"], "execute": [], "ttl_secs": cli.ttl_secs,
                "fee_allowance_txs": TXS},
            "dao_proposal_after_deploy": update_config_proposal(&own.owner, "<predicted bounty-escrow address>"),
        });
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    let dir = artifacts_dir();
    let labs = read_labs(&dir)?;
    let granter = if cli.granter.is_empty() {
        // v2 challenge: the signature binds the chain-id the run will target.
        let chain_id = if cli.spawn {
            cw_orch_kms::SPAWN_CHAIN_ID.to_string()
        } else {
            resolve_chain(cli.chain.as_deref().unwrap_or(""))?.chain_id.to_string()
        };
        println!("sign once on the approval page below (cw-orch-granter:v2, chain {chain_id}). that key is the granter");
        cw_orch_kms::capture_granter_for_chain_sync(cw_orch_kms::page_addr(), cli.account_index, "app-setup", &chain_id)
            .map_err(|err| anyhow!(err))?
    } else {
        cli.granter.clone()
    };
    println!("granter {granter}");
    let spawned = if cli.spawn {
        let chain = SpawnedTerp::boot().map_err(|err| anyhow!(err))?;
        chain.fund(&granter).map_err(|err| anyhow!(err))?;
        Some(chain)
    } else {
        None
    };
    let info = if let Some(spawned) = &spawned {
        spawned.chain_info()
    } else {
        resolve_chain(cli.chain.as_deref().unwrap_or(""))?.into()
    };

    cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|err| anyhow!(err))?;
    let mut builder = Daemon::builder(info);
    if spawned.is_some() {
        builder.is_test(true).load_network(false);
    }
    let creator = builder
        .build()
        .map_err(|err| anyhow!(err))?
        .sender_addr();
    let creator_bytes = canonical_account(creator.as_str())?;
    let mut bounty = String::new();
    let mut actions = Vec::new();
    for (lab, bytes) in &labs {
        let predicted = instantiate2_bech32(
            "terp",
            &wasm_checksum(bytes).map_err(|err| anyhow!(err))?,
            &creator_bytes,
            &salt(lab.name),
        )
        .map_err(|err| anyhow!(err))?;
        println!("predict {} {predicted}", lab.name);
        actions.push(
            SigningRequest::new("session-key")
                .body(format!("store {} and instantiate2 {predicted}", lab.file)),
        );
        if lab.name == "bounty-escrow" {
            bounty = predicted;
        }
    }
    let mut session = ApprovalSession::open(&granter).allow_instantiate2();
    for (_, bytes) in &labs {
        session = session.allow_store(bytes).map_err(|err| anyhow!(err))?;
    }
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = planned_spend_limit(TXS);
    session.retain = cli.retain;
    actions.push(SigningRequest::new("session-key").body(format!("admin and owner of all three: bounty DAO {dao}")));
    let mut run = enable_session(&builder, session, actions).map_err(|err| anyhow!(err))?;
    let sender = run.daemon.sender_addr();
    if sender.as_str() != creator.as_str() {
        return Err(anyhow!(
            "ephemeral creator changed from {creator} to {sender}"
        ));
    }
    let admin = Addr::unchecked(&own.wasm_admin);
    for (lab, bytes) in &labs {
        let predicted = instantiate2_bech32(
            "terp",
            &wasm_checksum(bytes).map_err(|err| anyhow!(err))?,
            &creator_bytes,
            &salt(lab.name),
        )
        .map_err(|err| anyhow!(err))?;
        let (code_id, store_tx) = match store_once(&run.daemon, lab.file, &dir.join(lab.file)) {
            Ok(stored) => stored,
            Err(err) => {
                run.note(&format!("err {err}"));
                return Err(err);
            }
        };
        run.note(&format!(
            "ok stored {} code {code_id} tx {store_tx}",
            lab.file
        ));
        let (address, init_tx) = match instantiate_at(
            &run.daemon,
            lab.name,
            code_id,
            &(lab.init)(own.owner.as_str()),
            &admin,
            &salt(lab.name),
            &predicted,
        ) {
            Ok(landed) => landed,
            Err(err) => {
                run.note(&format!("err {err}"));
                return Err(err);
            }
        };
        let info = run
            .daemon
            .wasm_querier()
            .contract_info(&Addr::unchecked(&address))
            .map_err(|err| anyhow!("contract info {address}: {err}"))?;
        if info.admin.as_ref().map(|a| a.to_string()) != Some(own.wasm_admin.clone()) {
            let err = anyhow!("{} admin is {:?}, expected bounty DAO {}", lab.name, info.admin, own.wasm_admin);
            run.note(&format!("err {err}"));
            return Err(err);
        }
        run.note(&format!(
            "ok instantiated {} {address} code {code_id} tx {init_tx} admin=owner=DAO",
            lab.name
        ));
    }
    let proposal = update_config_proposal(&own.owner, &bounty);
    println!("bounty DAO proposal (submit through the DAO, not executed here):\n{}",
        serde_json::to_string_pretty(&proposal)?);
    run.note(&format!("ok bounty {bounty} owned by DAO; update_config left to a DAO proposal"));
    let page = run.detach_page();
    let daemon = run.daemon.clone();
    if let Err(err) = run.close() {
        if let Some(page) = &page {
            page.note(&format!("err {err}"));
            let _ = daemon.rt_handle.block_on(async {
                tokio::task::yield_now().await;
            });
        }
        return Err(anyhow!(err));
    }
    println!("session grant closed");
    if let Some(page) = &page {
        page.note("ok session grant closed");
        let _ = daemon.rt_handle.block_on(async {
            tokio::task::yield_now().await;
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dao() -> String {
        bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("terp").unwrap(), &[9u8; 32]).unwrap()
    }

    #[test]
    fn bounty_dao_is_admin_and_owner_of_every_contract() {
        let own = ownership_plan(&dao()).unwrap();
        assert_eq!(own.wasm_admin, own.owner);
        for lab in LABS {
            let init = (lab.init)(&own.owner);
            assert_dao_owned(&own, &init).unwrap();
        }
        assert_eq!(bounty_init(&own.owner)["owner"], own.owner);
        assert_eq!(bounty_init(&own.owner)["approver"], own.owner);
    }

    #[test]
    fn a_foreign_owner_or_bad_dao_is_refused() {
        let own = ownership_plan(&dao()).unwrap();
        let mut init = bounty_init(&own.owner);
        init["approver"] = serde_json::json!("terp1ephemeral");
        assert!(assert_dao_owned(&own, &init).is_err());
        assert!(ownership_plan("cosmos1abc").is_err());
        assert!(ownership_plan("").is_err());
    }

    #[test]
    fn update_config_goes_through_the_dao() {
        let p = update_config_proposal(&dao(), "terp1bounty");
        assert_eq!(p["dao"], dao());
        assert!(p["propose"]["msgs"][0]["wasm"]["execute"]["msg"].as_str().unwrap().len() > 10);
    }
}
