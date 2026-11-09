//! Upload ave, its factory, and cw420 from the network suite.
use cw_orch::prelude::*;
use terp_orch::TerpArtifactStack;

pub fn upload_ave<Chain: CwEnv>(chain: Chain) -> Result<TerpArtifactStack<Chain>, CwOrchError> {
    let suite = TerpArtifactStack::new(chain);
    suite.products.cw420.upload()?;
    suite.products.ave_factory.upload()?;
    suite.products.ave.upload()?;
    Ok(suite)
}

/// `av_event_helpers::get_license_fee` returns `StdResult`. This fork's `StdError`
/// does not implement `std::error::Error`, so tests cannot `?` it into `anyhow`.
pub fn license_fee(chain_id: &str) -> anyhow::Result<cosmwasm_std::Coin> {
    av_event_helpers::get_license_fee(chain_id).map_err(|err| anyhow::anyhow!("{err}"))
}
