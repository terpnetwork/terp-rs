//! Shared types for the Atlas raffle and NFT-loan port.
//!
//! Stargaze `sg721` assets are not represented. Collateral and prizes are
//! native coins or cw721 tokens. `cw721-svg` uses the same `TransferNft` and
//! `OwnerOf` messages.

pub mod asset;
pub mod nft;
pub mod payment;

pub use asset::{AssetInfo, Cw721Token, all_unique, is_valid_comment, is_valid_name};
pub use nft::{PullError, assert_can_pull, transfer_nft_msg};
pub use payment::take_one_of;

#[cfg(feature = "testing")]
pub mod mock;

#[cfg(feature = "svg")]
pub mod svg;

use cosmwasm_schema::cw_serde;

/// Admin lock plus an optional governance lock. Either one blocks new listings.
#[cw_serde]
#[derive(Default)]
pub struct Locks {
    pub lock: bool,
    pub sudo_lock: bool,
}

impl Locks {
    pub fn is_locked(&self) -> bool {
        self.lock || self.sudo_lock
    }
}
