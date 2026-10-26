//! Yi Jing Harmony: write the cw721-svg InstantiateMsg (placeholders shown), and optionally deploy it
//! to a THROWAWAY LOCAL chain through the cw-orch suite (`CwSvgSuite`) to record gas.
//!
//!   cargo run -p cw-infuser-scripts --bin yijing_harmony -- emit --out out/yijing
//!   cargo run -p cw-infuser-scripts --bin yijing_harmony -- emit --denom ibc/ABC… --usdt-decimals 6 \
//!       --minter terp1… --creator terp1… --withdraw terp1… --royalty-recipient terp1…
//!   LOCAL_MNEMONIC=… STATE_FILE=/tmp/x/state.json \
//!   cargo run -p cw-infuser-scripts --bin yijing_harmony -- local --grpc http://127.0.0.1:9090 --chain-id test-1
//!
//! `local` refuses anything but a loopback gRPC URL and refuses the terp mainnet/testnet chain ids.
//! There is deliberately no mainnet/testnet mode: that deploy waits for the v2 per-token values
//! (TODO(yijing-v2-values)) and the Penumbra USDT denom (TODO(penumbra-usdt)).
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use cosmwasm_std::coin;
use cw_infuser_scripts::suite::CwSvgSuite;
use cw_infuser_scripts::yijing::{self, YiJingConfig};
use cw_orch::prelude::*;
use cw_svg_minter::msg::ExecuteMsgFns as _;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(about = "Yi Jing Harmony cw721-svg instantiate msg (cw-orch suite)")]
struct Args {
    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Debug, Clone)]
struct Cfg {
    /// Mint-price denom. Single swap point for the Penumbra USDT IBC denom.
    #[clap(long, env = "YIJING_DENOM", default_value = yijing::PENUMBRA_USDT_DENOM_PLACEHOLDER)]
    denom: String,
    /// Decimals of that denom (price = 0.36 * 10^decimals).
    #[clap(long, env = "YIJING_USDT_DECIMALS", default_value_t = yijing::USDT_DECIMALS_PLACEHOLDER)]
    usdt_decimals: u32,
    #[clap(long, env = "YIJING_MINTER")]
    minter: Option<String>,
    #[clap(long, env = "YIJING_CREATOR")]
    creator: Option<String>,
    #[clap(long, env = "YIJING_WITHDRAW")]
    withdraw: Option<String>,
    #[clap(long, env = "YIJING_ROYALTY_RECIPIENT")]
    royalty_recipient: Option<String>,
    /// Seconds after instantiate before minting opens.
    #[clap(long)]
    mint_start_offset_secs: Option<u64>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Write instantiate.json, royalty.json and report.json (placeholders where unset).
    Emit {
        #[clap(long, default_value = "out/yijing-harmony")]
        out: PathBuf,
        #[clap(flatten)]
        cfg: Cfg,
    },
    /// Deploy on a throwaway local chain (loopback only) and record gas.
    Local {
        #[clap(long)]
        grpc: String,
        #[clap(long)]
        chain_id: String,
        #[clap(long, default_value = "uthiol")]
        gas_denom: String,
        #[clap(long, default_value_t = 0.05)]
        gas_price: f64,
        /// Tokens to mint after instantiate (paid in --denom; on a local chain use the gas denom).
        #[clap(long, default_value_t = 1)]
        mint: u32,
        #[clap(long, default_value = "out/yijing-harmony-local")]
        out: PathBuf,
        #[clap(flatten)]
        cfg: Cfg,
    },
}

fn config(c: &Cfg, fallback: Option<&str>) -> YiJingConfig {
    let d = YiJingConfig::default();
    let pick = |v: &Option<String>, ph: Option<String>| {
        v.clone().or_else(|| fallback.map(str::to_string)).or(ph)
    };
    YiJingConfig {
        denom: c.denom.clone(),
        usdt_decimals: c.usdt_decimals,
        // Local runs default the minter to the contract itself (public mint), never to a guess.
        minter: c
            .minter
            .clone()
            .or(if fallback.is_some() { None } else { d.minter }),
        creator: pick(&c.creator, d.creator),
        withdraw_address: pick(&c.withdraw, d.withdraw_address),
        royalty_recipient: pick(&c.royalty_recipient, d.royalty_recipient),
        mint_start_offset_secs: c.mint_start_offset_secs,
    }
}

fn write_json(dir: &PathBuf, name: &str, v: &impl serde::Serialize) -> Result<usize> {
    std::fs::create_dir_all(dir)?;
    let s = serde_json::to_string_pretty(v)?;
    std::fs::write(dir.join(name), &s)?;
    Ok(s.len())
}

fn emit(out: PathBuf, cfg: YiJingConfig) -> Result<()> {
    let msg = yijing::yijing_harmony_init(&cfg)?;
    let compact = serde_json::to_vec(&msg)?;
    write_json(&out, "instantiate.json", &msg)?;
    write_json(&out, "royalty.json", &yijing::royalty_msg(&cfg, None))?;
    let p = &msg.collection_info_extension.packed;
    let report = serde_json::json!({
        "contract": "cw721-svg (packed SVP1 template, svg_packed key), terp-rs HEAD",
        "template_bytes": yijing::TEMPLATE.len(),
        "variables": yijing::variables().len(),
        "packed_bytes": p.len(),
        "packed_sha256": hex::encode(Sha256::digest(p.as_slice())),
        "instantiate_msg_json_bytes": compact.len(),
        "price": msg.collection_info_extension.price_tiers[0].price,
        "placeholders_remaining": cfg.placeholders(),
        "todo": [
            "TODO(yijing-v2-values): current contract seeds every variable; tournament-chosen per-token values need the v2 handoff",
            "TODO(penumbra-usdt): denom + decimals are placeholders until the Penumbra USDT IBC channel is open",
            "TODO(royalty): send royalty.json to the marketplace once the collection address and recipient are known",
        ],
    });
    write_json(&out, "report.json", &report)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn local(
    grpc: String,
    chain_id: String,
    gas_denom: String,
    gas_price: f64,
    mint: u32,
    out: PathBuf,
    c: Cfg,
) -> Result<()> {
    let host = grpc
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or_default();
    ensure!(
        host == "127.0.0.1" || host == "localhost",
        "refusing non-loopback gRPC {grpc}"
    );
    for net in [networks::TERP_MAINNET, networks::TERP_TESTNET] {
        ensure!(
            chain_id != net.chain_id,
            "refusing public chain id {chain_id}"
        );
    }
    let mut ci: ChainInfoOwned = networks::TERP_LOCALNET.into();
    ci.chain_id = chain_id.clone();
    ci.grpc_urls = vec![grpc.clone()];
    ci.gas_denom = gas_denom.clone();
    ci.gas_price = gas_price;
    // Mnemonic comes from LOCAL_MNEMONIC (cw-orch default for ChainKind::Local); never logged.
    let chain = DaemonBuilder::new(ci).build().context("daemon")?;
    let sender = chain.sender_addr();
    let mut c = c;
    if c.denom.contains('<') {
        c.denom = gas_denom.clone();
    }
    let cfg = config(&c, Some(sender.as_str()));
    ensure!(
        cfg.placeholders().is_empty(),
        "placeholders left: {:?}",
        cfg.placeholders()
    );
    let msg = yijing::yijing_harmony_init(&cfg)?;
    let suite = CwSvgSuite::new(chain.clone());
    let gas = |r: &cw_orch::daemon::CosmTxResponse| serde_json::json!({"gas_used": r.gas_used, "gas_wanted": r.gas_wanted, "height": r.height, "txhash": r.txhash});
    let mut rep = serde_json::Map::new();
    rep.insert("chain_id".into(), chain_id.clone().into());
    rep.insert(
        "instantiate_msg_json_bytes".into(),
        serde_json::to_vec(&msg)?.len().into(),
    );
    rep.insert(
        "packed_bytes".into(),
        msg.collection_info_extension.packed.len().into(),
    );
    if let Some(r) = suite.cwsvg.upload_if_needed()? {
        rep.insert("upload_cw721_svg".into(), gas(&r));
    }
    // (a) direct instantiate of cw721-svg (what MINT-COST.md measured)
    let r = suite.cwsvg.instantiate(&msg, Some(&sender), &[])?;
    let direct = r
        .instantiated_contract_address()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    rep.insert("instantiate_direct".into(), gas(&r));
    // mints on the direct collection, paying the tier price in the configured denom
    let price = cfg.price_amount();
    let mut mints = vec![];
    for i in 0..mint {
        let m = cw721_svg::ExecuteMsg::UpdateExtension {
            msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
                amnt: 1,
                proof_hashes: vec![],
                alloc: 0,
                ext: None,
            }),
        };
        let r = suite.cwsvg.execute(&m, &[coin(price, cfg.denom.clone())])?;
        // ids are sequential from mint_count (fresh collection: 0, 1, …)
        let tid = r
            .event_attr_values("wasm", "token_id")
            .into_iter()
            .find(|t| !t.is_empty())
            .unwrap_or_else(|| i.to_string());
        let uri: cw721_svg::contract::SvgTokenUriResponse =
            suite.cwsvg.query(&cw721_svg::QueryMsg::Extension {
                msg: cw721_svg::SvgQueryMsgExt::SvgTokenUri {
                    token_id: tid.clone(),
                },
            })?;
        mints.push(serde_json::json!({"token_id": tid, "tx": gas(&r), "token_uri_svg_bytes": uri.svg.len()}));
    }
    rep.insert("mints".into(), mints.into());
    // (b) the suite's own path (CwSvgSuite::deploy_on): minter factory -> CreateSvgCollection (instantiate2).
    // Recorded, not fatal: the factory artifact may not compile on every node build.
    let via: Result<serde_json::Value> = (|| {
        let mut out = serde_json::Map::new();
        if let Some(r) = suite.minter.upload_if_needed()? {
            out.insert("upload_cw_svg_minter".into(), gas(&r));
        }
        let r = suite.minter.instantiate(
            &cw_svg_minter::InstantiateMsg {
                owner: Some(sender.to_string()),
                svg_code_id: suite.cwsvg.code_id()?,
                dregg_ibc_only: false,
                oracle_pricing: None,
                runtime_minters: vec![],
            },
            Some(&sender),
            &[],
        )?;
        out.insert("instantiate_svg_minter".into(), gas(&r));
        let r = suite
            .minter
            .create_svg_collection(msg.clone(), "yijing-harmony")?;
        out.insert(
            "collection".into(),
            r.event_attr_value("wasm", "contract")
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .into(),
        );
        out.insert("create_svg_collection".into(), gas(&r));
        Ok(out.into())
    })();
    rep.insert(
        "via_minter_factory".into(),
        match via {
            Ok(v) => v,
            Err(e) => {
                serde_json::json!({"error": format!("{e:#}").chars().take(600).collect::<String>()})
            }
        },
    );
    rep.insert("collection_direct".into(), direct.to_string().into());
    rep.insert(
        "royalty_msg_for_marketplace".into(),
        yijing::royalty_msg(&cfg, Some(direct.as_str())),
    );
    write_json(&out, "local-run.json", &rep)?;
    println!("{}", serde_json::to_string_pretty(&rep)?);
    Ok(())
}

fn main() -> Result<()> {
    let _ = env_logger::try_init();
    match Args::parse().cmd {
        Cmd::Emit { out, cfg } => emit(out, config(&cfg, None)),
        Cmd::Local {
            grpc,
            chain_id,
            gas_denom,
            gas_price,
            mint,
            out,
            cfg,
        } => {
            if mint > 50 {
                bail!("--mint is capped at 50 for local runs")
            }
            local(grpc, chain_id, gas_denom, gas_price, mint, out, cfg)
        }
    }
}
