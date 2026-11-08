//! Opt-in daemon gate for svgc-preview-1.
//!
//! `NFTFI_BENCH_LOCAL=1` selects the path. cw-orch `ChainKind::Local` reads
//! `LOCAL_MNEMONIC` for the key named by `NFTFI_BENCH_KEY` (default `user`).
//! This module never reads the keyring, never prints a mnemonic, and never
//! calls `DaemonBuilder::build`, so it does not broadcast.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonGate {
    /// `NFTFI_BENCH_LOCAL` is not `1`.
    Skipped,
    /// The env is set and this crate was built without `--features local`.
    NeedsFeature,
    /// The env is set and the `local` feature is on. Check wasm, then refuse
    /// to broadcast the harness.
    CheckWasm,
}

pub fn daemon_gate() -> DaemonGate {
    let armed = std::env::var("NFTFI_BENCH_LOCAL").ok().as_deref() == Some("1");
    if !armed {
        return DaemonGate::Skipped;
    }
    #[cfg(feature = "local")]
    {
        DaemonGate::CheckWasm
    }
    #[cfg(not(feature = "local"))]
    {
        DaemonGate::NeedsFeature
    }
}

pub fn wasm_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NFTFI_WASM_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../artifacts/nftfi")
}

const WASM: &[&str] = &[
    "marketplace.wasm",
    "reserve-auction.wasm",
    "raffle.wasm",
    "nft-loan.wasm",
];

pub fn missing_wasm(dir: &Path) -> Vec<&'static str> {
    WASM.iter()
        .copied()
        .filter(|name| !dir.join(name).is_file())
        .collect()
}

/// Describe the daemon column without opening a grpc connection.
pub fn daemon_note() -> String {
    match daemon_gate() {
        DaemonGate::Skipped => {
            "daemon: skipped. Set NFTFI_BENCH_LOCAL=1 and build with --features local. No transaction sent.".into()
        }
        DaemonGate::NeedsFeature => {
            "daemon: NFTFI_BENCH_LOCAL=1 requires cargo test -p nftfi-bench --features local. No transaction sent.".into()
        }
        DaemonGate::CheckWasm => {
            #[cfg(feature = "local")]
            {
                match run_daemon() {
                    Ok(()) => "daemon: unexpected success".into(),
                    Err(err) => format!("daemon: {err}"),
                }
            }
            #[cfg(not(feature = "local"))]
            {
                unreachable!("CheckWasm is only returned with feature local");
            }
        }
    }
}

#[cfg(feature = "local")]
pub fn run_daemon() -> anyhow::Result<()> {
    use cw_orch::environment::{ChainKind, NetworkInfoOwned};
    use cw_orch::prelude::*;

    let dir = wasm_dir();
    let missing = missing_wasm(&dir);
    if !missing.is_empty() {
        anyhow::bail!(
            "not broadcast: missing wasm in {}: {}. No transaction sent.",
            dir.display(),
            missing.join(", ")
        );
    }

    let key = std::env::var("NFTFI_BENCH_KEY").unwrap_or_else(|_| "user".into());
    if std::env::var("LOCAL_MNEMONIC").is_err() {
        anyhow::bail!(
            "not broadcast: LOCAL_MNEMONIC is unset. ChainKind::Local loads it for key `{key}`. Keyring was not read."
        );
    }

    let gas_denom = std::env::var("NFTFI_BENCH_GAS_DENOM").unwrap_or_else(|_| "uterp".into());
    let chain = ChainInfoOwned {
        kind: ChainKind::Local,
        chain_id: "svgc-preview-1".into(),
        gas_denom,
        gas_price: 0.0,
        grpc_urls: vec!["http://127.0.0.1:39090".into()],
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        lcd_url: Some("http://127.0.0.1:31317".into()),
        fcd_url: None,
    };
    // Building the daemon would open grpc. Stop before `build`.
    let mut builder = DaemonBuilder::new(chain);
    let _ = builder.is_test(true).load_network(false);
    anyhow::bail!(
        "not broadcast: sibling wasm is present for key `{key}`, and this harness is not the port. Refusing to upload onto svgc-preview-1."
    );
}
