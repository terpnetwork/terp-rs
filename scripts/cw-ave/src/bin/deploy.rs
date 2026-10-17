use av_event_helpers::get_license_fee;
use cosmwasm_std::{coin, Timestamp};
use cw4::Member;
use cw_ave::{
    msg::InstantiateMsg,
    state::{EventSegment, EventSegmentAccessType, GuestDetails},
};
use cw_ave_factory::msg::{ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInit};
use cw_orch::{
    daemon::networks::{JUNO_1, LOCAL_JUNO},
    prelude::{networks::ChainInfo, *},
};
use scripts::interfaces::CwAveSuite;

fn full_deploy(networks: Vec<ChainInfo>) -> cw_orch::anyhow::Result<()> {
    for network in networks {
        let chain = DaemonBuilder::new(network.clone()).build()?;

        let suite = CwAveSuite::deploy_on(chain.clone(), ())?;

        // instantiate factory
        suite.cw_ave_factory.instantiate(
            &FactoryInit {
                owner: None,
                cw_ave_id: suite.cw_ave.code_id()?,
            },
            Some(&chain.sender_addr()),
            &[get_license_fee(&chain.chain_info().chain_id)?],
        )?;
    }

    Ok(())
}

fn create_event(networks: Vec<ChainInfo>) -> cw_orch::anyhow::Result<()> {
    for network in networks {
        let chain = DaemonBuilder::new(network.clone()).build()?;

        let suite = CwAveSuite::new(chain.clone());
        // todo: grab existing contracts from state.json file & use to initialize new factory, ensuring everything is good
    }

    Ok(())
}

fn main() {
    dotenv().ok();
    env_logger::init();

    use dotenv::dotenv;

    let networks = vec![JUNO_1];
    full_deploy(networks).unwrap();
}
