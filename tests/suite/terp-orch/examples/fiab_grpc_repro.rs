//! Read-only repro of the daemon build inside cw_orch_kms::enable_session (no signing, no approval page).
//! Uses a throwaway in-process mnemonic exactly like ensure_ephemeral_mnemonic.
use cw_orch::daemon::DaemonBuilder;
use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
fn main() -> anyhow::Result<()> {
    let url = std::env::var("GRPC_URL")?;
    let chain = ChainInfoOwned {
        chain_id: "morocco-1".into(), gas_denom: "uthiol".into(), gas_price: 0.05,
        grpc_urls: vec![url], rpc_urls: vec![], lcd_url: Some("https://api.terp.network".into()), fcd_url: None,
        network_info: NetworkInfoOwned { chain_name: "terp".into(), pub_address_prefix: "terp".into(), coin_type: 118 },
        kind: ChainKind::Mainnet,
    };
    cw_orch_kms::ensure_ephemeral_mnemonic(ChainKind::Mainnet).map_err(|e| anyhow::anyhow!(e))?;
    let mut b = DaemonBuilder::new(chain);
    b.load_network(std::env::var("LOAD_NETWORK").ok().as_deref() == Some("1"));
    match b.build() { Ok(_) => println!("daemon build OK (connection fine)"), Err(e) => println!("daemon build FAIL: {e}") }
    Ok(())
}
