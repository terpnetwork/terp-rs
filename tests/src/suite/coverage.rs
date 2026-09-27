//! What terp-scripts can actually reach today.
//!
//! Product suite *logic* lives in the listed crates. This table is the honest
//! inclusion map for `SidecarFleet` / `TerpNetworkSuite` / feature flags.
//! Keep in sync with Hermes `t_d3c747da` (SUITE-CYCLE) and event-ops `t_7aa13966`.

/// How a product suite is wired into terp-scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inclusion {
    /// Re-exported and constructable from terp-scripts under this feature.
    Wired,
    /// Types re-exported only; no TerpNetworkSuite / SidecarFleet field.
    ReexportOnly,
    /// Exists in-tree; terp-scripts does not compile it in.
    External,
    /// Explicitly parked (do not deploy from this suite).
    Parked,
}

#[derive(Clone, Copy, Debug)]
pub struct SuiteCoverage {
    pub id: &'static str,
    pub crate_path: &'static str,
    pub feature: &'static str,
    pub inclusion: Inclusion,
    pub resumable: &'static str,
}

/// Operator-facing inventory. Not a deploy list.
pub const SUITE_COVERAGE: &[SuiteCoverage] = &[
    SuiteCoverage {
        id: "sidecars",
        crate_path: "terp-rs/tests/src/suite/{hashmerchant,indexer,nostr,relayer,minio}",
        feature: "default / docker / nostr",
        inclusion: Inclusion::Wired,
        resumable: "process fleet, not wasm code_ids",
    },
    SuiteCoverage {
        id: "auth-vsck",
        crate_path: "terp-rs/contracts/smart-accounts/terp-vsck",
        feature: "auth-suite",
        inclusion: Inclusion::ReexportOnly,
        resumable: "types only (verify_envelope)",
    },
    SuiteCoverage {
        id: "auth-core",
        crate_path: "terp-rs/tests/suite/terp-authenticator-suite",
        feature: "auth-suite-full",
        inclusion: Inclusion::Wired,
        resumable: "TerpNetworkSuite.auth via auth-suite-full; upload_if_needed",
    },
    SuiteCoverage {
        id: "auth-dao",
        crate_path: "terp-rs/contracts/smart-accounts/terp-dao",
        feature: "(none — parked)",
        inclusion: Inclusion::Parked,
        resumable: "stub todo!",
    },
    SuiteCoverage {
        id: "auth-wavs",
        crate_path: "terp-rs/contracts/smart-accounts/terp-wavs",
        feature: "(none — parked)",
        inclusion: Inclusion::Parked,
        resumable: "partial / parked",
    },
    SuiteCoverage {
        id: "auth-zktls",
        crate_path: "terp-rs/contracts/smart-accounts/terp-zktls",
        feature: "(none — parked)",
        inclusion: Inclusion::Parked,
        resumable: "partial / parked",
    },
    SuiteCoverage {
        id: "polytone",
        crate_path: "polytone/packages/cw-orch-polytone",
        feature: "(always — terp-scripts dep)",
        inclusion: Inclusion::External,
        resumable: "Polytone::load_from + store_if_needed; bin polytone_migrate",
    },
    SuiteCoverage {
        id: "dao-testing",
        crate_path: "dao-contracts/packages/dao-testing",
        feature: "(not a terp-scripts dep)",
        inclusion: Inclusion::External,
        resumable: "DaoDaoSuite store_on/upload_if_needed; load_from hydrates ids",
    },
    SuiteCoverage {
        id: "private-dex",
        crate_path: "headstash/test-press suites/private_dex + terp-rs/contracts/revenue/private-dex",
        feature: "zk (zk-test-press)",
        inclusion: Inclusion::Wired,
        resumable: "TerpNetworkSuite.pdex via zk; upload_if_needed",
    },
    SuiteCoverage {
        id: "private-bridge",
        crate_path: "headstash/test-press suites/private_bridge",
        feature: "zk — PARKED",
        inclusion: Inclusion::Parked,
        resumable: "load_from returns Err",
    },
    SuiteCoverage {
        id: "headstash",
        crate_path: "headstash/test-press suites/headstash",
        feature: "zk",
        inclusion: Inclusion::External,
        resumable: "load_from = new(); hydrate from Daemon",
    },
    SuiteCoverage {
        id: "billboards",
        crate_path: "terp-account-scripts TerpAccountSuite",
        feature: "account",
        inclusion: Inclusion::Wired,
        resumable: "only if TerpNetworkSuite::deploy_on + data.terp_billboards",
    },
    SuiteCoverage {
        id: "infuser / shitstrap / dao-in-terp-scripts",
        crate_path: "tests/src/suite/deploy_data.rs (commented out)",
        feature: "account (dead)",
        inclusion: Inclusion::External,
        resumable: "deploy_data is thin TerpNetworkDeployData; infuser/shitstrap still out of tree",
    },
];

pub fn print_coverage() {
    println!(
        "{:<22} {:<16} {:<14} {}",
        "ID", "FEATURE", "INCLUSION", "RESUME"
    );
    for s in SUITE_COVERAGE {
        println!(
            "{:<22} {:<16} {:<14} {}",
            s.id,
            s.feature,
            format!("{:?}", s.inclusion),
            s.resumable
        );
    }
}

pub fn wired_ids() -> impl Iterator<Item = &'static str> {
    SUITE_COVERAGE
        .iter()
        .filter(|s| s.inclusion == Inclusion::Wired)
        .map(|s| s.id)
}
