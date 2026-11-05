//! Plane S crypto: FROST-Ed25519 only. No sockets, no WASI, no BLS.

mod frost;
mod scheme;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub use frost::{dkg_all, FrostEd25519};
pub use scheme::{
    identifier, CommitmentMap, ConsensusScheme, DkgR1Secret, DkgR2Secret, Error, ParticipantId,
    SigningNonces, SCHEME_ID,
};

pub const SCHEME_FROST_ED25519_V1: &str = SCHEME_ID;
