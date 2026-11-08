//! Store and instantiate the market contracts, cw721-svg, and the minter factory.
//!
//! One cold signature. Instantiate2 fixes each market and the factory from the
//! wasm checksum, the ephemeral creator, and a salt. The same signature grants
//! execute on that factory. The ephemeral key then registers the Marain Orbs
//! and Latent Benders series and creates the three collections through the
//! factory. The factory, not this key, instantiates cw721-svg. Close revokes
//! the grant.
//!
//! The Foundation DAO (`--admin`, default `PARENT_CORE`) is the wasm admin of every
//! contract, the market protocol/fee address, and the creator, withdraw and royalty
//! address of the three drops. The ephemeral key owns the factory only long enough
//! to register the series, then proposes a cw-ownable transfer to the Foundation;
//! the DAO accepts with `update_ownership: accept_ownership` (written to
//! out/svg_markets_accept_ownership.json). Instantiate2 addresses depend on the
//! creator and salt, not the admin, so predictions are unchanged.
//! Admin rotation is planned: the Foundation DAO holds these roles only while we deploy and bootstrap.
//!
//! Leave `--granter` empty. The key that signs `cw-orch-granter:v1` is the
//! granter. On groot-wan, pass `--spawn`.
//!
//! ```sh
//! cargo run -p dao-scripts --bin svg_markets -- --spawn
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use clap::Parser;
use cosmwasm_std::{Addr, Binary, Timestamp};
use cw_infuser_scripts::collection::{self, SeriesConfig};
use cw_infuser_scripts::drops::{self, Drop, KARA_GIPPY, MARAIN_ORBS, YIJING_HARMONY};
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch_kms::{
    ApprovalSession, CW_ORCH_GRANTER_ENV, ScopedCall, SigningRequest, SpawnedTerp, enable_session,
    instantiate2_bech32, planned_spend_limit, wasm_checksum,
};

/// Five stores, four instantiate2, five factory executes, and the revoke.
const TXS: u64 = 16;
const FACTORY: &str = "cw_svg_minter.wasm";
const FACTORY_NAME: &str = "svg-minter";
const MARAIN_SERIES: &str = "marain-orbs";
const BENDERS_SERIES: &str = "latent-benders";

struct Market {
    name: &'static str,
    file: &'static str,
    init: fn(&str, &str) -> serde_json::Value,
}

const MARKETS: &[Market] = &[
    Market {
        name: "svg-auction",
        file: "svg_auction.wasm",
        init: svg_init,
    },
    Market {
        name: "reserve-auction",
        file: "revenue_reserve_auction.wasm",
        init: reserve_init,
    },
    Market {
        name: "marketplace",
        file: "revenue_marketplace.wasm",
        init: marketplace_init,
    },
];

const SVG: &str = "cw721_svg.wasm";

#[derive(Parser, Debug)]
#[clap(name = "svg-markets")]
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
    /// gRPC endpoint. Defaults to the endpoint in the chain config.
    #[clap(long)]
    grpc: Option<String>,
    /// Refused. A session names the Wasm and the contracts it instantiates.
    #[clap(long)]
    wallet: bool,
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,
    #[clap(long, default_value_t = 0u32)]
    account_index: u32,
    #[clap(long)]
    retain: bool,
    /// Wasm admin and owner of the deployed contracts. Defaults to the Foundation DAO.
    /// Admin rotation is planned: the Foundation DAO holds these roles only while we deploy and bootstrap.
    #[clap(long, default_value = dao_foundation_suite::ibc_ops::PARENT_CORE)]
    admin: String,
}

fn artifacts_dir() -> PathBuf {
    std::env::var("ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts"))
}

fn svg_init(admin: &str, denom: &str) -> serde_json::Value {
    serde_json::json!({
        "protocol_fee_bps": 100,
        "min_increment_bps": 100,
        "min_reserve_price": "1",
        "duration": 3600,
        "extension": 60,
        "accepted_denom": [denom],
        "protocol_addr": admin
    })
}

fn reserve_init(admin: &str, denom: &str) -> serde_json::Value {
    serde_json::json!({
        "fee_manager": admin,
        "trading_fee_percent": "0.02",
        "min_bid_increment_percent": "0.05",
        "min_duration": 60,
        "max_duration": 3600,
        "extend_duration": 60,
        "create_auction_fee": {"denom": denom, "amount": "0"},
        "max_auctions_to_settle_per_block": 10,
        "halt_duration_threshold": 3600,
        "halt_buffer_duration": 60,
        "halt_postpone_duration": 60,
        "burn_bps": 0,
        "min_reserve_prices": [{"denom": denom, "amount": "1"}]
    })
}

fn marketplace_init(admin: &str, denom: &str) -> serde_json::Value {
    serde_json::json!({
        "fee_manager": admin,
        "royalty_registry": null,
        "protocol_fee_bps": 200,
        "max_royalty_fee_bps": 1000,
        "maker_reward_bps": 5000,
        "taker_reward_bps": 0,
        "burn_bps": 5000,
        "default_denom": denom
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

fn read_wasm(dir: &Path, file: &str) -> Result<Vec<u8>> {
    let path = dir.join(file);
    let bytes = std::fs::read(&path).map_err(|err| anyhow!("read {}: {err}", path.display()))?;
    if bytes.is_empty() {
        return Err(anyhow!("{} is empty", path.display()));
    }
    Ok(bytes)
}

fn salt(name: &str) -> Vec<u8> {
    format!("svg-markets/{name}").into_bytes()
}

fn canonical_account(addr: &str) -> Result<Vec<u8>> {
    let (hrp, data) = bech32::decode(addr).map_err(|err| anyhow!("creator {addr}: {err}"))?;
    if hrp.as_str() != "terp" {
        return Err(anyhow!("creator {addr} is not a terp account"));
    }
    Ok(data)
}

fn predict(bytes: &[u8], creator: &[u8], name: &str) -> Result<String> {
    instantiate2_bech32(
        "terp",
        &wasm_checksum(bytes).map_err(|err| anyhow!(err))?,
        creator,
        &salt(name),
    )
    .map_err(|err| anyhow!(err))
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

fn instantiate_at(
    chain: &Daemon,
    name: &str,
    code_id: u64,
    msg: &serde_json::Value,
    admin: &Addr,
    predicted: &str,
) -> Result<String> {
    let response = chain
        .instantiate2(
            code_id,
            msg,
            Some(name),
            Some(admin),
            &[],
            Binary::from(salt(name)),
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
    Ok(address.to_string())
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
            "--wallet is not a grant. This setup names the wasm checksums it stores"
        ));
    }
    if !cli.spawn && cli.chain.is_none() {
        return Err(anyhow!(
            "pass --spawn on groot-wan, or name a chain with --chain"
        ));
    }
    let dir = artifacts_dir();
    let mut wasm = Vec::new();
    for market in MARKETS {
        wasm.push((market.name, market.file, read_wasm(&dir, market.file)?));
    }
    let svg = read_wasm(&dir, SVG)?;
    let factory_wasm = read_wasm(&dir, FACTORY)?;

    let spawned = if cli.spawn {
        Some(SpawnedTerp::boot().map_err(|err| anyhow!(err))?)
    } else {
        None
    };
    let info = if let Some(spawned) = &spawned {
        spawned.chain_info()
    } else {
        resolve_chain(cli.chain.as_deref().unwrap_or(""))?.into()
    };
    let denom = info.gas_denom.clone();
    println!("fee denom {denom}");
    cw_orch_kms::ensure_ephemeral_mnemonic(info.kind.clone()).map_err(|err| anyhow!(err))?;
    let mut builder = Daemon::builder(info);
    if let Some(grpc) = &cli.grpc {
        builder.grpc_url(grpc);
    }
    if spawned.is_some() {
        builder.is_test(true).load_network(false);
    }
    let daemon = builder.build().map_err(|err| anyhow!(err))?;
    let creator = daemon.sender_addr();
    println!("ephemeral {creator}");
    if spawned.is_none() {
        wait_for_fee(&daemon, &creator, &denom)?;
    }
    let granter = if cli.granter.is_empty() {
        println!("sign once on the approval page below. that key is the granter");
        cw_orch_kms::capture_granter_sync(
            cw_orch_kms::page_addr(),
            cli.account_index,
            "svg-markets",
        )
        .map_err(|err| anyhow!(err))?
    } else {
        cli.granter.clone()
    };
    println!("granter {granter}");
    if let Some(spawned) = &spawned {
        spawned.fund(&granter).map_err(|err| anyhow!(err))?;
    }
    let creator_bytes = canonical_account(creator.as_str())?;
    let foundation = Addr::unchecked(cli.admin.clone());
    println!("admin {foundation}");
    let drops = drops::initial_collections(foundation.as_str(), &denom)?;

    let mut actions = Vec::new();
    let mut session = ApprovalSession::open(&granter).allow_instantiate2();
    for (name, file, bytes) in &wasm {
        let predicted = predict(bytes, &creator_bytes, name)?;
        println!("predict {name} {predicted}");
        actions.push(
            SigningRequest::new("session-key")
                .body(format!("store {file} and instantiate2 {predicted}")),
        );
        session = session.allow_store(bytes).map_err(|err| anyhow!(err))?;
    }
    session = session.allow_store(&svg).map_err(|err| anyhow!(err))?;
    session = session
        .allow_store(&factory_wasm)
        .map_err(|err| anyhow!(err))?;
    let factory_predicted = predict(&factory_wasm, &creator_bytes, FACTORY_NAME)?;
    println!("predict {FACTORY_NAME} {factory_predicted}");
    actions.push(SigningRequest::new("session-key").body(format!(
        "store {FACTORY} and instantiate2 {factory_predicted}"
    )));
    actions.push(SigningRequest::new("session-key").body(format!(
        "set series {MARAIN_SERIES} and {BENDERS_SERIES}, then create {MARAIN_ORBS}, {KARA_GIPPY}, and {YIJING_HARMONY} on {factory_predicted}, then offer factory ownership to {}", cli.admin
    )));
    session = session
        .allow_execute(ScopedCall {
            address: factory_predicted.clone(),
            keys: vec![
                "set_series".into(),
                "create_series_collection".into(),
                "create_svg_collection".into(),
                "update_ownership".into(),
            ],
            calls: 6,
            funds: vec![],
        })
        .map_err(|err| anyhow!(err))?;
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = planned_spend_limit(TXS);
    session.retain = cli.retain;
    let mut run = enable_session(&builder, session, actions).map_err(|err| anyhow!(err))?;
    let admin = run.daemon.sender_addr();
    if admin.as_str() != creator.as_str() {
        return Err(anyhow!(
            "ephemeral creator changed from {creator} to {admin}"
        ));
    }

    for market in MARKETS {
        let bytes = wasm
            .iter()
            .find(|(name, _, _)| *name == market.name)
            .map(|(_, _, bytes)| bytes.as_slice())
            .ok_or_else(|| anyhow!("missing {}", market.name))?;
        let predicted = predict(bytes, &creator_bytes, market.name)?;
        let (code_id, store_tx) = store_once(&run.daemon, market.file, &dir.join(market.file))?;
        run.note(&format!(
            "ok stored {} code {code_id} tx {store_tx}",
            market.file
        ));
        let address = instantiate_at(
            &run.daemon,
            market.name,
            code_id,
            &(market.init)(foundation.as_str(), &denom),
            &foundation,
            &predicted,
        )?;
        run.note(&format!("ok instantiated {} {address}", market.name));
    }

    let (svg_code, store_tx) = store_once(&run.daemon, SVG, &dir.join(SVG))?;
    run.note(&format!("ok stored {SVG} code {svg_code} tx {store_tx}"));
    let (factory_code, store_tx) = store_once(&run.daemon, FACTORY, &dir.join(FACTORY))?;
    run.note(&format!(
        "ok stored {FACTORY} code {factory_code} tx {store_tx}"
    ));
    let factory = instantiate_at(
        &run.daemon,
        FACTORY_NAME,
        factory_code,
        &serde_json::json!({
            "owner": admin.as_str(),
            "svg_code_id": svg_code,
            "dregg_ibc_only": false,
            "runtime_minters": []
        }),
        &foundation,
        &factory_predicted,
    )?;
    run.note(&format!("ok instantiated {FACTORY_NAME} {factory}"));
    let operators = vec![admin.to_string()];
    open_series(
        &run,
        &factory,
        MARAIN_SERIES,
        collection::marain_series(operators.clone()),
    )?;
    open_series(
        &run,
        &factory,
        BENDERS_SERIES,
        collection::benders_series(operators),
    )?;
    for drop in &drops {
        create_drop(&run, &factory, drop)?;
    }
    offer_factory(&run, &factory, foundation.as_str())?;

    let page = run.detach_page();
    let daemon = run.daemon.clone();
    if let Err(err) = run.close() {
        if let Some(page) = &page {
            page.note(&format!("err {err}"));
        }
        return Err(anyhow!(err));
    }
    println!("session grant closed");
    if let Some(page) = &page {
        page.note("ok session grant closed");
        daemon.rt_handle.block_on(async {
            tokio::task::yield_now().await;
        });
    }
    Ok(())
}

/// Bootstrap is done: offer factory ownership to the Foundation DAO (two-step
/// cw-ownable). Writes the accept message the DAO proposal executes.
fn offer_factory(run: &cw_orch_kms::AuthzSession, factory: &str, owner: &str) -> Result<()> {
    let msg = serde_json::json!({
        "update_ownership": { "transfer_ownership": { "new_owner": owner, "expiry": null } }
    });
    let response = run
        .daemon
        .execute(&msg, &[], &Addr::unchecked(factory))
        .map_err(|err| anyhow!("offer factory ownership: {err}"))?;
    println!(
        "factory ownership offered to {owner} tx {}",
        response.txhash
    );
    run.note(&format!(
        "ok factory ownership offered to {owner} tx {}",
        response.txhash
    ));
    std::fs::create_dir_all("out")?;
    let accept = serde_json::json!({
        "wasm": { "execute": {
            "contract_addr": factory,
            "msg": Binary::from(serde_json::to_vec(&serde_json::json!({"update_ownership": "accept_ownership"}))?),
            "funds": []
        }}
    });
    std::fs::write(
        "out/svg_markets_accept_ownership.json",
        serde_json::to_string_pretty(&accept)?,
    )?;
    Ok(())
}

fn wait_for_fee(daemon: &Daemon, address: &Addr, denom: &str) -> Result<()> {
    use cw_orch::daemon::queriers::Bank;
    let bank = Bank::new(daemon);
    println!(
        "send {denom} to {address} for the grant transaction. The feegrant pays the later transactions."
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20 * 60);
    loop {
        let coins = bank
            .balance(address, Some(denom.to_string()))
            .unwrap_or_default();
        let amount = coins
            .iter()
            .find(|coin| coin.denom == denom)
            .map(|coin| coin.amount.to_string())
            .unwrap_or_else(|| "0".into());
        if amount != "0" {
            println!("ephemeral balance {amount}{denom}");
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(anyhow!("ephemeral {address} still has no {denom}"));
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}

fn open_series(
    run: &cw_orch_kms::AuthzSession,
    factory: &str,
    series: &str,
    config: SeriesConfig,
) -> Result<()> {
    let msg = serde_json::json!({
        "set_series": { "series": series, "config": config }
    });
    let response = run
        .daemon
        .execute(&msg, &[], &Addr::unchecked(factory))
        .map_err(|err| anyhow!("set series {series}: {err}"))?;
    println!("series {series} tx {}", response.txhash);
    run.note(&format!("ok series {series} tx {}", response.txhash));
    Ok(())
}

fn create_drop(run: &cw_orch_kms::AuthzSession, factory: &str, drop: &Drop) -> Result<()> {
    let mut instantiate_msg = drop.msg.clone();
    let (key, body) = match drop.name {
        MARAIN_ORBS => (
            "create_series_collection",
            serde_json::json!({
                "series": MARAIN_SERIES,
                "label": instantiate_msg.name,
                "instantiate_msg": instantiate_msg
            }),
        ),
        KARA_GIPPY => {
            // Latent Benders caps the mint window at 24h, measured as seconds
            // after the factory creates the collection.
            instantiate_msg.collection_info_extension.mint_end_time =
                Some(Timestamp::from_seconds(86_400));
            (
                "create_series_collection",
                serde_json::json!({
                    "series": BENDERS_SERIES,
                    "label": instantiate_msg.name,
                    "instantiate_msg": instantiate_msg
                }),
            )
        }
        YIJING_HARMONY => (
            "create_svg_collection",
            serde_json::json!({
                "label": instantiate_msg.name,
                "instantiate_msg": instantiate_msg
            }),
        ),
        other => return Err(anyhow!("no factory path for {other}")),
    };
    let response = run
        .daemon
        .execute(
            &serde_json::json!({ key: body }),
            &[],
            &Addr::unchecked(factory),
        )
        .map_err(|err| anyhow!("create {}: {err}", drop.name))?;
    let address = response
        .event_attr_value("wasm", "contract")
        .map_err(|err| anyhow!("create {} address: {err}", drop.name))?;
    println!(
        "created {} {address} via {factory} tx {}",
        drop.name, response.txhash
    );
    run.note(&format!(
        "ok created {} {address} tx {}",
        drop.name, response.txhash
    ));
    Ok(())
}
