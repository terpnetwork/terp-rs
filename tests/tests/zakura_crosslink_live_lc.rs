//! Live Zakura → Crosslink 08-wasm light client.
//!
//! SoT: ssh groot-wan. Do not clone Zakura onto the Grok session laptop.
//!
//! The sibling `crosslink_light_client.rs` suite signs synthetic `BftBlock`s
//! with `ed25519_zebra::SigningKey::from([i as u8; 32])`. That is a
//! state-machine fixture, not node consensus.
//!
//! This file is the real path:
//!   ict-rs `spawn_zakura_local` → JSON-RPC TFL methods on that `zakurad`
//!   → `CrosslinkHeader` bytes → `cw-ics08-wasm-crosslink` `MsgUpdateClient`.
//!
//! Stock Zakura (CX0) has `getblockchaininfo` only. TFL methods appear when
//! the fork speaks zebra-crosslink RPC (CX3+):
//!   - `get_tfl_fat_pointer_to_bft_chain_tip`
//!   - `get_tfl_final_block_height_and_hash`
//!   - `get_tfl_roster_zats`
//!
//! `ZAKURA_CROSSLINK_LAB=1` fail-closes if those methods are missing or the
//! fat pointer has empty signatures.

#![cfg(all(feature = "crosslink-e2e", feature = "zakura-live"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use ict_rs::prelude::*;
use serde_json::{json, Value};

const TFL_FAT_POINTER: &str = "get_tfl_fat_pointer_to_bft_chain_tip";
const TFL_FINAL_HEIGHT_HASH: &str = "get_tfl_final_block_height_and_hash";
const TFL_ROSTER: &str = "get_tfl_roster_zats";

fn lab_fail_closed() -> bool {
    std::env::var("ZAKURA_CROSSLINK_LAB").ok().as_deref() == Some("1")
}

async fn json_rpc(rpc_url: &str, method: &str, params: Value) -> Result<Value> {
    let url = rpc_url.trim_end_matches('/').to_string() + "/";
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let out = tokio::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "5",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body.to_string(),
        ])
        .output()
        .await
        .with_context(|| format!("curl {method} {url}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "curl {method} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .with_context(|| format!("parse {method} body"))?;
    if let Some(err) = v.get("error") {
        return Err(anyhow!("{method} JSON-RPC error: {err}"));
    }
    v.get("result")
        .cloned()
        .ok_or_else(|| anyhow!("{method} response missing result: {v}"))
}

/// Probe the TFL RPC surface a Crosslink-aware zakurad must expose.
async fn probe_tfl(rpc_url: &str) -> Result<TflProbe> {
    let fat = json_rpc(rpc_url, TFL_FAT_POINTER, json!([])).await;
    let tip = json_rpc(rpc_url, TFL_FINAL_HEIGHT_HASH, json!([])).await;
    let roster = json_rpc(rpc_url, TFL_ROSTER, json!([])).await;
    Ok(TflProbe {
        fat_pointer: fat,
        final_height_hash: tip,
        roster,
    })
}

struct TflProbe {
    fat_pointer: Result<Value>,
    final_height_hash: Result<Value>,
    roster: Result<Value>,
}

impl TflProbe {
    fn fat_pointer_has_sigs(&self) -> bool {
        let Ok(v) = &self.fat_pointer else {
            return false;
        };
        v.pointer("/signatures")
            .and_then(|s| s.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
            || v.get("signatures")
                .and_then(|s| s.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
    }

    fn summary(&self) -> String {
        format!(
            "fat_pointer={} final_height_hash={} roster={}",
            fmt_res(&self.fat_pointer),
            fmt_res(&self.final_height_hash),
            fmt_res(&self.roster),
        )
    }
}

fn fmt_res(r: &Result<Value>) -> String {
    match r {
        Ok(v) => v.to_string(),
        Err(e) => format!("ERR:{e}"),
    }
}

async fn spawn_or_reuse_zakura() -> Result<ZakuraNode> {
    let runtime: Arc<dyn RuntimeBackend> =
        Arc::new(DockerBackend::new(DockerConfig::default()).await?);
    let mut cfg = if std::env::var(ENV_ZAKURAD_BIN)
        .ok()
        .filter(|p| PathBuf::from(p.trim()).is_file())
        .is_some()
    {
        ZakuraNodeConfig::from_env()?
    } else {
        ZakuraNodeConfig::default()
    };
    cfg.test_name = "zakura-live-lc".into();
    // Do not steal PIR :18232. Live LC uses a distinct host port.
    if cfg.rpc_host_port == ZAKURA_RPC_PORT {
        cfg.rpc_host_port = 18252;
    }
    cfg.reuse_if_ready = std::env::var(ENV_ZAKURA_RPC).is_ok();
    spawn_zakura_local(runtime, cfg)
        .await
        .map_err(|e| anyhow!("spawn_zakura_local: {e}"))
}

/// CX7 step 1: a live zakurad answers chain info, and TFL RPCs are present
/// under lab fail-closed.
#[tokio::test]
async fn zakura_live_node_exposes_tfl_rpc() {
    let lab = lab_fail_closed();
    let mut node = match spawn_or_reuse_zakura().await {
        Ok(n) => n,
        Err(e) if lab => panic!("ZAKURA_CROSSLINK_LAB=1 but zakurad did not spawn: {e}"),
        Err(e) => {
            eprintln!("skip: cannot spawn zakurad ({e})");
            return;
        }
    };

    assert!(
        node.rpc_ready().await,
        "zakurad getblockchaininfo must return JSON-RPC result at {}",
        node.rpc_url()
    );

    let chain = json_rpc(node.rpc_url(), "getblockchaininfo", json!([]))
        .await
        .expect("getblockchaininfo");
    assert!(
        chain.get("chain").is_some(),
        "getblockchaininfo result must include chain: {chain}"
    );

    let probe = probe_tfl(node.rpc_url())
        .await
        .expect("probe TFL RPC");
    let summary = probe.summary();

    if lab {
        assert!(
            probe.fat_pointer.is_ok(),
            "ZAKURA_CROSSLINK_LAB=1: {TFL_FAT_POINTER} must work (CX3+ on the Zakura fork). {summary}"
        );
        assert!(
            probe.final_height_hash.is_ok(),
            "ZAKURA_CROSSLINK_LAB=1: {TFL_FINAL_HEIGHT_HASH} must work. {summary}"
        );
        assert!(
            probe.fat_pointer_has_sigs(),
            "ZAKURA_CROSSLINK_LAB=1: fat pointer must carry finalizer signatures \
             (not SigningKey::from seed, not FatPointerToBftBlock2::null). {summary}"
        );
    } else if probe.fat_pointer.is_err() {
        eprintln!(
            "honest residual (CX3+): TFL RPC not on this zakurad yet. {summary}"
        );
    }

    let _ = node.teardown().await;
}

/// CX7 step 2: build a `CrosslinkHeader` from the live fat pointer and submit
/// it as `MsgUpdateClient`. Needs a Terp chain + stored 08-wasm (same Docker
/// shape as `crosslink_light_client.rs`).
///
/// Gated until TFL RPC returns a signed fat pointer. Do not fill this with
/// synthetic keys.
#[tokio::test]
async fn zakura_live_header_updates_wasm_lc() {
    let lab = lab_fail_closed();
    let mut node = match spawn_or_reuse_zakura().await {
        Ok(n) => n,
        Err(e) if lab => panic!("ZAKURA_CROSSLINK_LAB=1 but zakurad did not spawn: {e}"),
        Err(e) => {
            eprintln!("skip: cannot spawn zakurad ({e})");
            return;
        }
    };

    let probe = probe_tfl(node.rpc_url())
        .await
        .expect("probe TFL RPC");
    if probe.fat_pointer.is_err() || !probe.fat_pointer_has_sigs() {
        let summary = probe.summary();
        let _ = node.teardown().await;
        if lab {
            panic!(
                "ZAKURA_CROSSLINK_LAB=1: cannot drive cw-ics08-wasm-crosslink without a \
                 node-signed fat pointer. Finish CX3/CX4 TFL RPC on zakurad. {summary}"
            );
        }
        eprintln!(
            "honest residual: live MsgUpdateClient waits on zakurad TFL RPC (CX3+). {summary}"
        );
        return;
    }

    let _ = node.teardown().await;
    // Header decode + wasm update is the next cut once CX3 returns typed bytes.
    // Do not invent a SigningKey roster here.
    panic!(
        "TFL RPC is live; wire CrosslinkHeader::zcash_deserialize from \
         get_tfl_fat_pointer_to_bft_chain_tip + BftBlock fetch, then reuse \
         prepare_wasm_light_clients / MsgUpdateClient from crosslink_light_client.rs. \
         Do not fall back to generate_test_finalizers."
    );
}
