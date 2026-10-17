//! Headstash circuit suite — Product A builders and fixtures.
//!
//! Spec: `docs/zk-headstash/suite.md`
//!
//! Default eligibility tree and claim pre-inputs: **Poseidon-v1** distro leaf/CRH
//! and Poseidon note `cmx`. Sinsemilla APIs are explicitly `*_sinsemilla_legacy`
//! (recovery only).

pub mod suite;
pub mod unit;

pub use suite::{build_headstash_keys_to, HeadstashCircuitSuite};
