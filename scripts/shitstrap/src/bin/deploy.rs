use cw_orch::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let terp = Daemon::builder(networks::TERP_MAINNET).build()?;
    let dd = shit_scripts::new_default_ibc_set(terp.sender_addr());
    shit_scripts::CwShitstrapSuite::deploy_on(terp.clone(), Some(dd))?;
    Ok(())
}
