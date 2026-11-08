//! Tier-1 headstash claim lane for the hash-market sidecar.
//!
//! The snap note server is a different route. This crate checks a flock
//! proof, keeps the in-memory batch, and signs the pending or accepted root
//! that cw-headstash already hashes. A halo2 campaign selects a cached
//! verifying key by `vk_id` and does not accept a signature or a leaf key.

pub mod denominations;
pub mod eligibility;
mod host;
mod lane;
mod roots;

pub use headstash_claim::DEPTH;
pub use host::{Halo2Host, HostVerifyError, halo2_vk_hash, install_halo2_host, verify_with_host};
pub use zk_headstash::circuit::{
    EpochClaimBundle, EpochStatement, epoch_circuit_blob, epoch_vk_hash, host_public_inputs,
    prepare_halo2_verifier, prove_epoch_claim,
};
pub use zk_headstash::no_ecdsa::{
    MEMBERSHIP_VK_ID, membership_circuit_blob, membership_host_proof, membership_public_inputs,
    membership_vk_hash,
};
pub use zk_headstash::note_membership::{
    NOTE_VK_HASH, NOTE_VK_ID, NoteHostProof, note_circuit_blob, note_host_proof,
};
pub use zk_headstash::toy_host::{toy_vk_proof, toy_vk_proof_offset};

/// Campaign id of the epoch circuit blob. Not the Pasta curve id.
pub const EPOCH_VK_ID: u64 = 1;

pub use lane::{
    Backend, BatchReport, Campaign, CampaignParams, IngressError, NOTE_CLAIM_PREFIX, PeerReceipt,
    SubmittedClaim, note_claim_message,
};
pub use roots::{
    ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_HEIGHT, BATCH_ATTEST_TIMESTAMP, BATCH_SCOPE,
    BATCH_SOURCE_ID, CHAIN_UID, PayoutRow, SealedVote, accepted_root, accepted_root_anchored,
    aggregate_root, attestation_value, attested_root, custody_message, pending_root,
    pending_root_unsorted, receipt_message, sha256, verify_custody,
};
