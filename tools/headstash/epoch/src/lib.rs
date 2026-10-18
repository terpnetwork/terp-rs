//! Tier-1 headstash claim lane for the hash-market sidecar.
//!
//! The snap note server is a different route. This crate checks a flock
//! proof, keeps the in-memory batch, and signs the pending or accepted root
//! that cw-headstash already hashes. A halo2 campaign checks `Proof::verify`
//! and does not accept a signature or a leaf key.

mod lane;
mod roots;

pub use headstash_claim::DEPTH;
pub use zk_headstash::circuit::{prove_epoch_claim, EpochClaimBundle};

pub use lane::{
    Backend, BatchReport, Campaign, CampaignParams, IngressError, PeerReceipt, SubmittedClaim,
};
pub use roots::{
    ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_HEIGHT, BATCH_ATTEST_TIMESTAMP, BATCH_SCOPE,
    BATCH_SOURCE_ID, CHAIN_UID, PayoutRow, SealedVote, accepted_root, attested_root,
    custody_message, pending_root, pending_root_unsorted, receipt_message, sha256, verify_custody,
};
