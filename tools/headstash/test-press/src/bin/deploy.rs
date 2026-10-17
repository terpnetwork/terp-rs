use cw_orch::{
    anyhow,
    prelude::{ZkCwEnv as _, *},
};
use cw_orch::{
    daemon::networks::{ATOMEONE_TESTNET, TERP_TESTNET},
    prelude::*,
};
use zk_test_press::suites::no_rick::interface::NoRickSuite;

fn main() -> anyhow::Result<()> {
    // rustls::crypto::aws_lc_rs::default_provider()
    //     .install_default()
    //     .unwrap();
    env_logger::init();
    let terp = Daemon::builder(TERP_TESTNET.clone()).mnemonic("tuna harbor jacket regular net athlete quarter very wave jewel observe voice evolve pass bamboo liberty write dentist just before pipe envelope bid matrix").build()?;
    // let dd = TestPressDeployData::local_default(terp.sender_addr(), genesis_root, keys_dir);
    let testpress = NoRickSuite::new(terp.clone());
    testpress.circuit.upload_circuit()?;
    // testpress.contract.upload()?;
    Ok(())
}
