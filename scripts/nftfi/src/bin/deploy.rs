//! Upload and instantiate the nftfi suite on a mock chain.
//!
//! A live chain uses the same call: `NftfiSuite::deploy_on(daemon, data)`.

use cw_orch::prelude::*;
use nftfi_scripts::{NftfiDeployData, NftfiSuite};
use nftfi_utils::svg::SvgDrop;

fn main() -> anyhow::Result<()> {
    let chain = MockBech32::new("terp");
    let admin = chain.sender_addr();
    let data = NftfiDeployData::lab(admin, SvgDrop::two_tone("NftFi", "NFI"));
    let suite = NftfiSuite::deploy_on(chain, data)?;
    println!("collection {}", suite.collection.address()?);
    println!("raffle {}", suite.raffle.address()?);
    println!("loan {}", suite.loan.address()?);
    Ok(())
}
