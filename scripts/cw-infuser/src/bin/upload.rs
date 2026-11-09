use cw_infuser_scripts::prelude::*;
use cw_orch::{anyhow, prelude::*};

pub fn main() -> anyhow::Result<()> {
    // rustls::crypto::aws_lc_rs::default_provider()
    //     .install_default()
    //     .unwrap();
    env_logger::init();
    dotenv::dotenv().ok();
    let terp = Daemon::builder(networks::TERP_MAINNET).build()?;

    let infuse = CwSvgSuite::new(terp.clone());
    infuse.upload()?;

    let id = terp.state().get_all_code_ids()?;
    println!("{:#?}", id);

    Ok(())
}
