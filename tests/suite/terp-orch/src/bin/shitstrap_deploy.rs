use cw_orch::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let terp = Daemon::builder(networks::TERP_MAINNET).build()?;
    let dd = terp_orch::new_default_ibc_set(terp.sender_addr());
    terp_orch::CwShitstrapSuite::deploy_on(terp.clone(), Some(dd))?;
    Ok(())
}
