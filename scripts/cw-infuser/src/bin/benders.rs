//! Latent Benders: A Study — instantiate one day collection from a finalized day record.
//!
//!   cargo run -p cw-infuser-scripts --bin benders -- plan --record <out>/day-record.json --creator terp1…
//!   LOCAL_MNEMONIC=… STATE_FILE=/tmp/benders/state.json \
//!   cargo run -p cw-infuser-scripts --bin benders -- local --record <out>/day-record.json \
//!       --grpc http://127.0.0.1:9090 --chain-id test-1
//!
//! Both modes re-verify record_sha256 (-> seed) and packed_sha256 (packed.svp beside the record).
//! `local` refuses non-loopback gRPC and the terp mainnet/testnet chain ids. There is no public-chain mode.
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use cw_infuser_scripts::benders::{self, DayMintParams};
use cw_infuser_scripts::collection::SvgCollectionSuite;
use cw_orch::prelude::*;

#[derive(Parser, Debug)]
#[clap(about = "Latent Benders day collection (cw-orch SvgCollectionSuite)")]
struct Args {
    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Debug, Clone)]
struct Day {
    /// Finalized day-record.json from tools/svg-collections.
    #[clap(long)]
    record: PathBuf,
    /// Packed SVP1 file; defaults to packed.svp next to the record.
    #[clap(long)]
    packed: Option<PathBuf>,
    #[clap(long, default_value_t = 10_000)]
    total: u64,
    /// Mint opens at instantiate and closes this long after (series caps it at 86400).
    #[clap(long, default_value_t = 86_400)]
    mint_window_secs: u64,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Verify hashes and write the instantiate msg; no chain.
    Plan {
        #[clap(flatten)]
        day: Day,
        /// Collection creator. Defaults to the Foundation DAO. Admin rotation is planned: the Foundation DAO holds these roles only while we deploy and bootstrap.
        #[clap(long, default_value = cw_infuser_scripts::FOUNDATION_DAO)]
        creator: String,
        #[clap(long, default_value = "out/benders")]
        out: PathBuf,
    },
    /// Instantiate on a loopback localnet (sender = creator).
    Local {
        #[clap(flatten)]
        day: Day,
        #[clap(long)]
        grpc: String,
        #[clap(long)]
        chain_id: String,
        #[clap(long, default_value = "uthiol")]
        gas_denom: String,
        #[clap(long, default_value_t = 0.05)]
        gas_price: f64,
        #[clap(long, default_value = "out/benders-local")]
        out: PathBuf,
        /// Name of the env var holding this collection creator mnemonic (never printed).
        #[clap(long, default_value = "BENDERS_CREATOR_MNEMONIC")]
        creator_mnemonic_env: String,
        /// Feeshare withdrawer for the factory and the collection (default: creator).
        #[clap(long)]
        withdrawer: Option<String>,
        /// Gas funding sent from the operator to the creator, in gas_denom.
        #[clap(long, default_value_t = 5_000_000)]
        fund_creator: u128,
    },
}

fn params(d: &Day, creator: Addr) -> DayMintParams {
    DayMintParams {
        creator,
        total: d.total,
        price_tiers: vec![],
        mint_window_secs: d.mint_window_secs,
    }
}

fn write(out: &PathBuf, name: &str, v: &impl serde::Serialize) -> Result<()> {
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join(name), serde_json::to_string_pretty(v)?)?;
    Ok(())
}

fn main() -> Result<()> {
    let _ = env_logger::try_init();
    match Args::parse().cmd {
        Cmd::Plan { day, creator, out } => {
            let v = benders::load(&day.record, day.packed.as_deref())?;
            let cfg = benders::day_config(&v, &params(&day, Addr::unchecked(creator)))?;
            // Offsets are relative to "now" (box clock); `local` recomputes from the chain.
            let now = cosmwasm_std::Timestamp::from_seconds(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
            );
            write(&out, "instantiate.json", &cfg.to_init_msg(now)?)?;
            let rep = serde_json::json!({
                "day": v.day, "day_index": v.day_index, "seed": v.record_sha256,
                "packed_sha256": v.packed_sha256, "packed_bytes": v.packed.len(),
                "mint_end_after_secs": cfg.mint_end_after_secs, "total": cfg.total,
                "series": benders::SERIES, "label": cfg.label,
            });
            write(&out, "plan.json", &rep)?;
            println!("{}", serde_json::to_string_pretty(&rep)?);
        }
        Cmd::Local { day, grpc, chain_id, gas_denom, gas_price, out, creator_mnemonic_env, withdrawer, fund_creator } => {
            let host = grpc.trim_start_matches("http://").split(':').next().unwrap_or_default();
            ensure!(host == "127.0.0.1" || host == "localhost", "refusing non-loopback gRPC {grpc}");
            for net in [networks::TERP_MAINNET, networks::TERP_TESTNET] {
                ensure!(chain_id != net.chain_id, "refusing public chain id {chain_id}");
            }
            let v = benders::load(&day.record, day.packed.as_deref())?;
            let mut ci: ChainInfoOwned = networks::TERP_LOCALNET.into();
            ci.chain_id = chain_id;
            ci.grpc_urls = vec![grpc];
            ci.gas_denom = gas_denom.clone();
            ci.gas_price = gas_price;
            // Operator (factory owner/admin): LOCAL_MNEMONIC, the cw-orch default for Local.
            let chain = DaemonBuilder::new(ci).build().context("daemon")?;
            // Fresh creator per collection: mnemonic from the named env var; never logged.
            let m = std::env::var(&creator_mnemonic_env)
                .with_context(|| format!("{creator_mnemonic_env} is not set"))?;
            let creator_chain = chain.rebuild().mnemonic(m).build().context("creator daemon")?;
            let creator = creator_chain.sender_addr();
            let withdrawer = withdrawer.unwrap_or_else(|| creator.to_string());
            chain.bank_send(&creator, &[cosmwasm_std::coin(fund_creator, &gas_denom)])?;

            let operator = chain.sender_addr();
            let cfg = benders::day_config(&v, &params(&day, creator.clone()))?;
            let mut suite = SvgCollectionSuite::new(chain.clone());
            // Factory: reuse the one in STATE_FILE if present, otherwise deploy + set the series.
            let factory = {
                let f = cw_infuser_scripts::collection::SvgFactory {
                    minter: cw_svg_minter::interface::Cw721SvgMinter::new(chain.clone()),
                };
                if f.minter.address().is_err() {
                    let f = suite.deploy_factory(Some(&operator), &operator)?;
                    suite.set_series(&f, benders::SERIES, Some(cw_infuser_scripts::collection::benders_series(vec![])))?;
                    let tx = cw_infuser_scripts::collection::feeshare::register(&chain, f.minter.address()?.as_str(), &withdrawer)?;
                    println!("feeshare factory tx {tx}");
                    f
                } else {
                    f
                }
            };
            let addr = suite.instantiate_in_series(&factory, benders::SERIES, &cfg)?;
            let tx = cw_infuser_scripts::collection::feeshare::register(&creator_chain, addr.as_str(), &withdrawer)?;
            println!("feeshare collection tx {tx}");
            let st = suite.query(&addr)?;
            ensure!(st.seed_hex == v.record_sha256, "on-chain seed mismatch");
            ensure!(st.packed_sha256 == v.packed_sha256, "on-chain packed mismatch");
            write(&out, "local-run.json", &st)?;
            println!("{}", serde_json::to_string_pretty(&st)?);
        }
    }
    Ok(())
}
