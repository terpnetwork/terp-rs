//! Terp Warriors whitelist file vs the on-chain merkle root.
//!
//! Contract `terp1uv54d987cq5c83wcs9phal9drgrgf3jdxzp33phgjrfm27djvqdq7xy0cr`
//! (code 53, no admin) stores this root and `merkle_tree_uri = null`.
//! Leaf is `blake3(address || allocation)`. Each proof step sorts the two
//! 32-byte hashes and blake3-concatenates them, matching `query_has_member`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

const CHAIN_ROOT: &str = "f5c68fb4b1e98358963cdc43e83b653875f3f4794ed46d2927e65884451253b1";

#[derive(Deserialize)]
struct WarriorsFile {
    merkle_root: String,
    total_addresses: usize,
    accounts: BTreeMap<String, AccountProof>,
}

#[derive(Deserialize)]
struct AccountProof {
    allocation: u32,
    proof_hashes: Vec<String>,
}

fn warriors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../crates/cw-infuser/data/terp-warriors.json")
}

fn load_warriors() -> WarriorsFile {
    let path = warriors_path();
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&raw).expect("terp-warriors.json")
}

/// Same fold as whitelist-merkletree `query_has_member`.
fn has_member(address: &str, allocation: u32, proof_hashes: &[String], root_hex: &str) -> bool {
    let composite = format!("{address}{allocation}");
    let mut current = *blake3::hash(composite.as_bytes()).as_bytes();
    for sibling_hex in proof_hashes {
        let sibling = hex::decode(sibling_hex).expect("proof hex");
        let sibling: [u8; 32] = sibling.try_into().expect("proof is 32 bytes");
        let mut pair = [current, sibling];
        pair.sort_unstable();
        let mut cat = [0u8; 64];
        cat[..32].copy_from_slice(&pair[0]);
        cat[32..].copy_from_slice(&pair[1]);
        current = *blake3::hash(&cat).as_bytes();
    }
    hex::encode(current) == root_hex
}

#[test]
fn terp_warriors_proofs_rebuild_the_chain_root() {
    let file = load_warriors();
    assert_eq!(file.accounts.len(), 277);
    assert_eq!(file.total_addresses, 277);
    assert_eq!(file.merkle_root, CHAIN_ROOT);
    for (addr, acc) in &file.accounts {
        assert!(
            has_member(addr, acc.allocation, &acc.proof_hashes, CHAIN_ROOT),
            "has_member failed for {addr}"
        );
        assert!(
            crate::hash::verify(addr, acc.allocation, &acc.proof_hashes, CHAIN_ROOT)
                .unwrap(),
            "hash::tree::verify failed for {addr}"
        );
    }
}

#[cfg(feature = "server")]
mod served {
    use super::*;
    use crate::server::{router, AppState};
    use crate::tree_operator::{ephemeral_operator, sign_tree_write};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn host(operator: &str) -> (axum::Router, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hash-market-warriors-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AppState::new_merkle_host("morocco-1".into(), dir.clone()).unwrap();
        state.set_tree_operator(operator);
        (router(Arc::new(state)), dir)
    }

    async fn call(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: &[u8],
        pubkey: Option<&str>,
        sig: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder().method(method).uri(path);
        if !body.is_empty() {
            req = req.header("content-type", "application/json");
        }
        if let Some(pk) = pubkey {
            req = req.header("x-hash-market-pubkey", pk);
        }
        if let Some(sig) = sig {
            req = req.header("x-hash-market-signature", sig);
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::from(body.to_vec())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn served_member_proof_passes_has_member() {
        let file = load_warriors();
        let (addr, acc) = file.accounts.iter().next().expect("one account");
        let raw = std::fs::read(warriors_path()).unwrap();
        let id = CHAIN_ROOT;
        let path = format!("/trees/{id}");

        let (operator, op_addr) = ephemeral_operator();
        let (app, dir) = host(&op_addr);

        let (status, _) = call(&app, "POST", &path, &raw, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (other, _) = ephemeral_operator();
        let (pk, sig) = sign_tree_write(&other, "POST", &path, &raw);
        let (status, _) = call(&app, "POST", &path, &raw, Some(&pk), Some(&sig)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (pk, sig) = sign_tree_write(&operator, "POST", &path, &raw);
        let (status, _) = call(&app, "POST", &path, &raw, Some(&pk), Some(&sig)).await;
        assert_eq!(status, StatusCode::CREATED);

        let (pk, sig) = sign_tree_write(&operator, "PUT", &path, &raw);
        let (status, _) = call(&app, "PUT", &path, &raw, Some(&pk), Some(&sig)).await;
        assert_eq!(status, StatusCode::CREATED);

        let member = format!("/trees/{id}/members/{addr}");
        let (status, body) = call(&app, "GET", &member, &[], None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["eligible"], true);
        assert_eq!(body["merkle_root"], CHAIN_ROOT);
        assert_eq!(body["allocation"], acc.allocation);
        let proofs: Vec<String> =
            serde_json::from_value(body["proof_hashes"].clone()).expect("proof_hashes");
        assert!(has_member(addr, acc.allocation, &proofs, CHAIN_ROOT));

        let (status, _) = call(&app, "DELETE", &path, &[], None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(dir);
    }
}
