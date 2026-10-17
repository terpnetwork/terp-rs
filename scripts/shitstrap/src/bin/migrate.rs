use cw_orch::prelude::*;
use shit_scripts::CwShitstrapSuite;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    dotenv::dotenv().ok();
    let terp = Daemon::builder(networks::TERP_MAINNET).build()?;
    CwShitstrapSuite::new(terp.clone());
    Ok(())
}
