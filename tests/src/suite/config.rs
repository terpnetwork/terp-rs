//! Named, composable profiles for [`crate::suite::SidecarFleet`].
//!
//! Profiles are the reuse point: every integration test picks a profile
//! instead of assembling sidecars ad-hoc. Auth contracts live behind
//! `auth-suite` and are **not** expanded here until the auth impl freeze lifts.

use super::SidecarFleet;

/// What a test actually needs from the unified suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteProfile {
    /// Docker runtime + local chain Daemon only.
    ChainOnly,
    /// Chain + NIP-01 relay (calendar / DAO records / marketplace notes).
    NostrCalendar,
    /// Chain + hashmerchant + merkle server (vote-extension ingress).
    HashmerchantVe,
    /// Nostr + hashmerchant VE (DAO records + calendar + VE ingress).
    NostrVe,
    /// IBC relayer + two-chain intent (relayer sidecar only; caller supplies chains).
    Relayer,
    /// Crosslink light-client e2e (chain only; headers submitted via cw-orch).
    Crosslink,
    /// norick WASM / Halo2 guest proofs (no extra sidecars).
    Norick,
    /// FROST privval sidecar e2e (chain only; FrostPrivvalSuite composes manually).
    FrostPrivval,
    /// Cashu mint e2e (sidecar reserved; mint image chosen after research).
    CashuMint,
}

impl SuiteProfile {
    /// Build an unstarted fleet. Caller still supplies mnemonic via
    /// [`SidecarFleet::with_local_chain`] / [`SidecarFleet::with_local_chain_from_env`]
    /// when a Daemon is needed (never a hardcoded seed).
    pub fn fleet(self, name: &str) -> SidecarFleet {
        let fleet = SidecarFleet::new(name);
        match self {
            Self::ChainOnly | Self::Crosslink | Self::Norick | Self::CashuMint | Self::FrostPrivval => fleet,
            Self::NostrCalendar => {
                #[cfg(feature = "nostr")]
                {
                    return fleet.with_nostr_relay("nostr-relay");
                }
                #[cfg(not(feature = "nostr"))]
                {
                    fleet
                }
            }
            Self::HashmerchantVe => fleet.with_hashmerchant_defaults("terp-test-1").with_merkle_server_defaults(),
            Self::NostrVe => {
                let fleet = fleet
                    .with_hashmerchant_defaults("terp-test-1")
                    .with_merkle_server_defaults();
                #[cfg(feature = "nostr")]
                {
                    return fleet.with_nostr_relay("nostr-relay");
                }
                #[cfg(not(feature = "nostr"))]
                {
                    fleet
                }
            }
            Self::Relayer => fleet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SuiteProfile;

    #[test]
    fn chain_only_profile_builds_unstarted_fleet() {
        let fleet = SuiteProfile::ChainOnly.fleet("suite-profile-unit");
        assert!(fleet.endpoint("hashmerchant", "http").is_none());
    }

    #[test]
    fn hashmerchant_profile_wires_ve_sidecars() {
        let _ = SuiteProfile::HashmerchantVe.fleet("suite-profile-ve");
        let _ = SuiteProfile::NostrVe.fleet("suite-profile-nostr-ve");
        let _ = SuiteProfile::NostrCalendar.fleet("suite-profile-nostr");
    }
}
