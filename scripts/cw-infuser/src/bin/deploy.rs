// Integration deployment script for cw-svg + cw-infuser.
//
// Usage:
//   cargo run -p cw-infuser-scripts --bin integration -- --network local
//   cargo run -p cw-infuser-scripts --bin integration -- --network mainnet
// with single integration:
//   cargo run -p cw-infuser-scripts --bin integration -- --network mainnet --single

use anyhow::{anyhow, Result};
use clap::Parser;
use cw_infuser_scripts::{
    fractal::default_permissionless_fractal_init,
    suite::{
        svg::load_svg_init_msg, whitelist::load_terp_warrior_mtree, CwSvgSuite,
        CwSvgSuiteDeployData,
    },
};
use cw_orch::prelude::*;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Network: local, mainnet
    #[clap(short, long, default_value = "mainnet")]
    network: String,
}

pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    dotenv::dotenv().ok();
    env_logger::init();
    workflow(
        match args.network.as_str() {
            "local" => networks::TERP_LOCALNET.to_owned(),
            "mainnet" => networks::TERP_MAINNET.to_owned(),
            other => return Err(anyhow!("Unknown network: {}", other)),
        }
        .clone()
        .into(),
    )?;

    Ok(())
}


fn workflow(network: ChainInfoOwned) -> anyhow::Result<()> {
    let chain = DaemonBuilder::new(network.clone()).build()?;
    let tmp = CwSvgSuite::new(chain.clone());
    let mut suite = CwSvgSuite::deploy_on(
        chain.clone(),
        svg_deploy_data(
            chain.sender_addr(),
            &network.chain_id,
            &tmp.mtree.addr_str()?,
        )?,
    )?;
    suite.shit = shit_scripts::CwShitstrapSuite::new(chain.clone());
    Ok(())
}

fn svg_deploy_data(sender: Addr, _chain_id: &str, w: &str) -> Result<Option<CwSvgSuiteDeployData>> {
    // let mut atom = load_svg_init_msg("scripts/svgs/interchain/cosmos/init.json")?;
    // let mut bcna = load_svg_init_msg("scripts/svgs/interchain/bcna/init.json")?;
    // let mut btsg = load_svg_init_msg("scripts/svgs/interchain/bitsong/init.json")?;
    // let mut monero = load_svg_init_msg("scripts/svgs/interchain/monero/init.json")?;
    // let mut um = load_svg_init_msg("scripts/svgs/interchain/penumbra/init.json")?;
    // let mut osmo = load_svg_init_msg("scripts/svgs/interchain/osmosis/init.json")?;
    let mut akt = load_svg_init_msg("../scripts/svgs/interchain/akash/init.json")?;
    let mut btc = load_svg_init_msg("../scripts/svgs/interchain/bitcoin/init.json")?;
    let mut dao = load_svg_init_msg("../scripts/svgs/interchain/dao/init.json")?;
    let mut eth = load_svg_init_msg("../scripts/svgs/interchain/eth/init.json")?;
    let mut terp = load_svg_init_msg("../scripts/svgs/interchain/terp/init.json")?;
    let mut zec = load_svg_init_msg("../scripts/svgs/interchain/zec/init.json")?;
    let mut permissionless = default_permissionless_fractal_init(&7849u64, &sender.to_string());
    permissionless.collection_info_extension.whitelist = Some(w.to_string());

    // atom.creator = Some(sender.to_string());
    // um.creator = Some(sender.to_string());
    // btsg.creator = Some(sender.to_string());
    // bcna.creator = Some(sender.to_string());
    // monero.creator = Some(sender.to_string());
    // osmo.creator = Some(sender.to_string());
    terp.creator = Some(sender.to_string());
    terp.collection_info_extension.whitelist = Some(w.to_string());
    dao.creator = Some(sender.to_string());
    dao.collection_info_extension.whitelist = Some(w.to_string());
    btc.creator = Some(sender.to_string());
    btc.collection_info_extension.whitelist = Some(w.to_string());
    akt.creator = Some(sender.to_string());
    akt.collection_info_extension.whitelist = Some(w.to_string());
    eth.creator = Some(sender.to_string());
    eth.collection_info_extension.whitelist = Some(w.to_string());
    zec.creator = Some(sender.to_string());
    zec.collection_info_extension.whitelist = Some(w.to_string());
    //  SET CORRECT PRICE TIERS

    let mut svg = vec![(terp, None)];
    svg.extend(vec![
        (dao, None),
        (btc, None), // osmo,
        (akt, None),
        (eth, None),
        (zec, None),
        (permissionless, None),
        // atom,
        // btc,
        // akt,
        // um,
        // btsg,
        // bcna,
        // monero,
    ]);

    Ok(Some(CwSvgSuiteDeployData {
        svg,
        infuse: None,
        admin: Some(sender.clone()),
        infuse_coins: vec![],
        shit: Some(shit_scripts::new_default_ibc_set(sender)),
    }))
}
