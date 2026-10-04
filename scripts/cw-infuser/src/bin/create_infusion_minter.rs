// Integration deployment script for cw-svg + cw-infuser.
//
// Usage:
//   cargo run -p cw-infuser-scripts --bin integration -- --network local
//   cargo run -p cw-infuser-scripts --bin integration -- --network mainnet
// with single integration:
//   cargo run -p cw-infuser-scripts --bin integration -- --network mainnet --single

use cosmwasm_std::{coin, Decimal};
use cw_infuser::msg::InstantiateMsg;
use cw_infuser_scripts::suite::CwSvgSuite;
use cw_orch::{daemon::networks::TERP_MAINNET, prelude::*};

pub fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    let dao: Addr =
        Addr::unchecked("terp1lv2m4jkkrd87s5xt8j6kntpse3488qqjp7pcr5cmds7jk7vw0j4q38jj3e");

    let chain = DaemonBuilder::new(TERP_MAINNET).build()?;
    let suite = CwSvgSuite::new(chain.clone());

    suite.infuser.instantiate(
        &InstantiateMsg {
            contract_owner: Some(dao.to_string()),
            owner_fee: Decimal::from_ratio(2u128, 100u128),
            min_creation_fee: Some(coin(500u128, "uterp")),
            min_infusion_fee: Some(coin(100u128, "uterp")),
            min_per_bundle: None,
            max_per_bundle: None,
            max_bundles: None,
            max_infusions: None,
            cw721_code_id: 77u64,
            wavs_public_key: None,
        },
        Some(&dao.clone()),
        &[],
    )?;
    Ok(())
}
