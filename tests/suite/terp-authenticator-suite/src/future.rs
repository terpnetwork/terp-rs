//! Pre-curated extension sockets for post-suite work (W3).
//!
//! Intentionally empty implementations — vote-sdk role matrix and daemon e2e
//! hang off these types later without reshaping the suite.

use cw_orch::prelude::CwEnv;

use crate::suite::TerpAuthenticatorSuite;

/// Marker for the vote-sdk authentication matrix (W3a).
///
/// Future: map each voting role (proposer, voter, tallier, …) to an authenticator
/// contract address and exercise msg-auth options against this suite.
pub struct VoteSdkAuthMatrix;

/// Hook for role → authenticator routing (no-op until `vote-sdk` feature work).
pub trait FutureVoteAuthIntegration {
    fn authenticator_for_role(&self, role: &str) -> Option<String>;
}

impl<Chain: CwEnv> FutureVoteAuthIntegration for TerpAuthenticatorSuite<Chain> {
    fn authenticator_for_role(&self, _role: &str) -> Option<String> {
        None
    }
}
