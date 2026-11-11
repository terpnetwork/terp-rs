//! Deploy suites that used to live in `scripts/`.
//!
//! Each file is one cw-orch `Deploy` bag. `TerpArtifactStack` is the manifold
//! that holds every contract, including [`dao_testing::DaoDaoSuite`].

pub mod circuit;

#[cfg(feature = "bridge")]
pub mod private_dex;

#[cfg(feature = "stack")]
pub mod accounts;
#[cfg(feature = "stack")]
pub mod ave;
#[cfg(feature = "stack")]
pub mod bounties;
#[cfg(feature = "stack")]
pub mod nftfi;
#[cfg(feature = "stack")]
pub mod shitstrap;
#[cfg(feature = "stack")]
pub mod svg;

/// Privacy products: LC registry, seams, bridge, auctions, zk-voting slots.
#[cfg(feature = "privacy")]
pub mod privacy;
