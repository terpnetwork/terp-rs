//! HashMerchant feature planes — isolated sanity/insanity + concurrent load.
//!
//! Each test is one plane (health / onboard / blobs / trees) so they can run
//! **in parallel** (`cargo test --test hashmerchant_planes -- --test-threads=8`).
//! `concurrent_load_mixed_planes` hammers **one** host from many tasks so we
//! know the Axum state (tree store, blossom, onboard) survives concurrent use.
//!
//! In-process host: no docker, no `hash-market-server` binary, no port bind.
//! Live sidecar: `HashMerchantClient::from_url` / `SidecarFleet.hashmerchant`.
//!
//!   cargo test -p terp-scripts --test hashmerchant_planes --features hash-market

use axum::http::StatusCode;
use serde_json::json;
use terp_scripts::suite::HashMerchantClient;

const CHAIN: &str = "terp-test-1";
// NOTE: onboard readiness (is_live_terp1) requires real-length terp1
// addresses (>= 39 chars); shorter placeholders report ready=false.
const DAO: &str = "terp1dao0000000000000000000000000000000000000";
const RUNTIME: &str = "terp1rt00000000000000000000000000000000000000";
const USER: &str = "terp1user0000000000000000000000000000000000";

fn onboard_host() -> HashMerchantClient {
    HashMerchantClient::in_process_with_onboard(CHAIN, DAO, RUNTIME).expect("in-process host")
}

fn merkle_host() -> HashMerchantClient {
    HashMerchantClient::in_process_merkle(CHAIN).expect("merkle host")
}

fn tree_body() -> serde_json::Value {
    json!({
        "merkle_root": "ab".repeat(32),
        "accounts": {
            "terp1member000000000000000000000000000": {
                "tier": 1,
                "allocation": 1,
                "proof_hashes": []
            }
        }
    })
}

// ── health ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sanity_health_is_merchant_not_onboard() {
    let hm = merkle_host();
    let v = hm.health().await.unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["mode"], "merkle-bud-host");
    assert_eq!(v["onboard"]["enabled"], false);
}

#[tokio::test]
async fn insanity_unknown_path_404() {
    let hm = merkle_host();
    let st = hm.probe("GET", "/definitely-not-a-plane").await.unwrap();
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── onboard ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sanity_onboard_status_and_register() {
    let hm = onboard_host();
    let health = hm.health().await.unwrap();
    assert_eq!(health["onboard"]["prefix"], "/onboard");
    let st = hm.onboard_status().await.unwrap();
    assert_eq!(st["chain_id"], CHAIN);
    assert!(st["ready"].as_bool().unwrap());
    let (code, body) = hm.request_register_grant(USER).await.unwrap();
    assert_eq!(code, StatusCode::OK, "{body}");
    assert!(
        body.to_string().contains("AllowedMsgAllowance")
            || body.get("allowance").is_some()
            || body.get("@type").is_some()
            || body.to_string().contains("MsgGrantAllowance"),
        "grant json: {body}"
    );
}

#[tokio::test]
async fn insanity_onboard_rejects_non_terp() {
    let hm = onboard_host();
    let (code, body) = hm.request_register_grant("cosmos1nope").await.unwrap();
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("terp1"), "{body}");
}

#[tokio::test]
async fn insanity_root_status_is_not_onboard() {
    let hm = onboard_host();
    // Merchant has no GET /status — that belongs under /onboard/status.
    let st = hm.probe("GET", "/status").await.unwrap();
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── blobs ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sanity_blob_roundtrip() {
    let hm = merkle_host();
    let payload = b"hashmerchant-plane-blob";
    let up = hm.upload_blob(payload).await.unwrap();
    let sha = up["sha256"].as_str().expect("sha256");
    let (st, got) = hm.get_blob(sha).await.unwrap();
    assert!(st.is_success(), "{st}");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn insanity_missing_blob_404() {
    let hm = merkle_host();
    let (st, _) = hm.get_blob(&"00".repeat(32)).await.unwrap();
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── trees ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sanity_tree_roundtrip() {
    let hm = merkle_host();
    let st = hm.put_tree("plane-a", tree_body()).await.unwrap();
    assert!(
        st == StatusCode::CREATED || st.is_success(),
        "put tree {st}"
    );
    let (gst, tree) = hm.get_tree("plane-a").await.unwrap();
    assert!(gst.is_success(), "{gst} {tree}");
}

#[tokio::test]
async fn insanity_tree_bad_root() {
    let hm = merkle_host();
    let st = hm
        .put_tree(
            "bad",
            json!({ "merkle_root": "nope", "accounts": {} }),
        )
        .await
        .unwrap();
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

// ── concurrent load on one host ─────────────────────────────────────────────

#[tokio::test]
async fn concurrent_load_mixed_planes() {
    let hm = onboard_host();
    // Warm the host so first-request setup is not the only thing we measure.
    hm.health().await.unwrap();

    let n = 24usize;
    let mut tasks = Vec::with_capacity(n);
    for i in 0..n {
        let c = hm.clone();
        tasks.push(tokio::spawn(async move {
            match i % 4 {
                0 => {
                    let v = c.health().await?;
                    anyhow::ensure!(v["status"] == "ok");
                }
                1 => {
                    let addr = format!("terp1load{i:026}");
                    let (st, _) = c.request_register_grant(&addr).await?;
                    anyhow::ensure!(st == StatusCode::OK, "grant {st}");
                }
                2 => {
                    let bytes = format!("blob-{i}").into_bytes();
                    let up = c.upload_blob(&bytes).await?;
                    let sha = up["sha256"].as_str().unwrap();
                    let (st, got) = c.get_blob(sha).await?;
                    anyhow::ensure!(st.is_success() && got == bytes);
                }
                _ => {
                    let id = format!("tree-{i}");
                    let st = c.put_tree(&id, tree_body()).await?;
                    anyhow::ensure!(st.is_success() || st == StatusCode::CREATED, "tree {st}");
                }
            }
            Ok::<_, anyhow::Error>(())
        }));
    }
    for t in tasks {
        t.await.expect("join").expect("plane");
    }
}

/// Same as mixed load but the four plane families run as sibling tasks (true overlap).
#[tokio::test]
async fn concurrent_families_in_parallel() {
    let hm = onboard_host();
    let health = {
        let c = hm.clone();
        tokio::spawn(async move {
            for _ in 0..8 {
                c.health().await.unwrap();
            }
        })
    };
    let grants = {
        let c = hm.clone();
        tokio::spawn(async move {
            for i in 0..8 {
                let addr = format!("terp1fam{i:027}");
                let (st, _) = c.request_register_grant(&addr).await.unwrap();
                assert_eq!(st, StatusCode::OK);
            }
        })
    };
    let blobs = {
        let c = hm.clone();
        tokio::spawn(async move {
            for i in 0..8 {
                c.upload_blob(format!("fam-blob-{i}").as_bytes())
                    .await
                    .unwrap();
            }
        })
    };
    let trees = {
        let c = hm.clone();
        tokio::spawn(async move {
            for i in 0..8 {
                let st = c.put_tree(&format!("fam-{i}"), tree_body()).await.unwrap();
                assert!(st.is_success() || st == StatusCode::CREATED);
            }
        })
    };
    let (a, b, c, d) = tokio::join!(health, grants, blobs, trees);
    a.unwrap();
    b.unwrap();
    c.unwrap();
    d.unwrap();
}


const MEMBER: &str = "terp1member000000000000000000000000000";

#[tokio::test]
async fn sanity_mint_grant() {
    let hm = onboard_host();
    let (code, body) = hm.request_mint_grant(USER).await.unwrap();
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["purpose"], "mint");
    let msgs = body["msg"]["allowance"]["allowed_messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        msgs.iter().any(|v| v.as_str() == Some("/cosmwasm.wasm.v1.MsgExecuteContract")),
        "{body}"
    );
}

#[tokio::test]
async fn sanity_tree_member_proof() {
    let hm = merkle_host();
    let st = hm.put_tree("plane-a", tree_body()).await.unwrap();
    assert!(st.is_success() || st == StatusCode::CREATED, "{st}");
    let (gst, proof) = hm.member_proof("plane-a", MEMBER).await.unwrap();
    assert!(gst.is_success(), "{gst} {proof}");
    assert_eq!(proof["eligible"], true);
    let (mst, miss) = hm
        .member_proof("plane-a", "terp1notamember000000000000000000000")
        .await
        .unwrap();
    assert!(mst.is_success());
    assert_eq!(miss["eligible"], false);
}

#[tokio::test]
async fn sanity_ve_health_alias() {
    let hm = merkle_host();
    let (st, v) = hm.ve_health().await.unwrap();
    assert!(st.is_success(), "{st}");
    assert_eq!(v["status"], "ok");
}

#[tokio::test]
async fn sanity_cashu_mint_list() {
    let hm = merkle_host();
    let (st, v) = hm.cashu_mints().await.unwrap();
    assert!(st.is_success(), "{st} {v}");
}

#[tokio::test]
async fn sanity_headstash_roundtrip() {
    let hm = merkle_host();
    let st = hm
        .put_headstash("hs1", serde_json::json!({"note": "plane"}))
        .await
        .unwrap();
    assert!(st.is_success() || st == StatusCode::CREATED, "{st}");
    let (gst, v) = hm.get_headstash("hs1").await.unwrap();
    assert!(gst.is_success(), "{gst} {v}");
}

#[tokio::test]
async fn sanity_blob_delete() {
    let hm = merkle_host();
    let up = hm.upload_blob(b"delete-me").await.unwrap();
    let sha = up["sha256"].as_str().unwrap();
    let st = hm.delete_blob(sha).await.unwrap();
    assert!(st.is_success() || st == StatusCode::NO_CONTENT, "{st}");
    let (gst, _) = hm.get_blob(sha).await.unwrap();
    assert_eq!(gst, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn insanity_oracle_off_is_loud() {
    let hm = merkle_host();
    let (st, _) = hm.oracle_bounds().await.unwrap();
    assert!(
        st == StatusCode::SERVICE_UNAVAILABLE
            || st == StatusCode::NOT_FOUND
            || st.is_client_error()
            || st.is_server_error(),
        "oracle should fail loud when unset, got {st}"
    );
}
