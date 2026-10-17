use cw_orch::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    dotenv::dotenv().ok();
    let terp = Daemon::builder(networks::TERP_MAINNET).build()?;
    let shit = shit_scripts::CwShitstrapSuite::new(terp.clone());
    shit.upload()?;
    println!("{:#?}", terp.state().get_all_code_ids()?);
    Ok(())
}
