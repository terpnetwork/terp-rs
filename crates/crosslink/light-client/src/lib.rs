//! Crosslink light client verification primitives.
//!
//! This crate provides the core types and verification logic for the
//! Crosslink Trailing Finality Layer (TFL) IBC light client. It is
//! IBC-agnostic and can be used independently of the CosmWasm contract.
//!
//! All types are self-contained — no dependency on the full zebra-crosslink
//! node crate. This makes the library suitable for CosmWasm contracts.

pub mod client_state;
pub mod consensus_state;
pub mod error;
pub mod header;
pub mod membership;
pub mod misbehaviour;
pub mod types;
pub mod update;
pub mod verify;

pub use client_state::{ClientState, FinalizerEntry};
pub use consensus_state::ConsensusState;
pub use error::CrosslinkIBCError;
pub use header::CrosslinkHeader;
pub use membership::{
    POOL_ROOT_HASH_ALG, POOL_ROOT_INNER_LEFT, POOL_ROOT_INNER_RIGHT, POOL_ROOT_LEAF_PREFIX,
    PoolRootExistenceProof, PoolRootProofSpec, SiblingStep, encode_merkle_proof,
    leaf_commitment_root, leaf_existence_proof, pool_root_proof_spec, pool_root_proof_specs,
    verify_membership, verify_non_membership,
};
pub use types::{
    BftBlock, Blake3Hash, FatPointerSignature2, FatPointerToBftBlock2, PROTOTYPE_PARAMETERS,
    PowHeader, SerializationError, ToIbcAny, ZcashCrosslinkParameters, ZcashDeserialize,
    ZcashSerialize, type_urls,
};
