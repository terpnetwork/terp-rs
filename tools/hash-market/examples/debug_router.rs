//! Standalone debug binary to test the hash-market router in isolation.
//! Run with: cd tools/hash-market && cargo run --example debug --features "server,ve,blossom,client,nostr"
use hash_market::custody::local::LocalSecp256k1;
use hash_market::server::{router as build_router, AppState};
use hash_market::client::ve::VoteExtensionHandler;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join("hashdebut");
    std::fs::create_dir_all(&dir).expect("create dir");

    let custody = LocalSecp256k1::generate();
    let ve_handler = VoteExtensionHandler::new(Box::new(custody));
    let state = Arc::new(AppState::new(Some(ve_handler), "debug".into(), dir, true).unwrap());
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:9876").await.unwrap();
    println!("LISTENING: http://127.0.0.1:9876");
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let client = reqwest::Client::new();

    // Test health
    let resp = client.get("http://127.0.0.1:9876/health").send().await.unwrap();
    println!("GET /health -> {} body={:?}", resp.status(), resp.text().await);

    // Test POST /blobs
    let resp = client.post("http://127.0.0.1:9876/blobs").body(b"hello world".to_vec()).send().await.unwrap();
    println!("POST /blobs -> {} body={:?}", resp.status(), resp.text().await);

    // Test POST /trees/test-id
    let resp = client.post("http://127.0.0.1:9876/trees/test-id")
        .json(&serde_json::json!({
            "merkle_root": hex::encode([0u8; 32]),
            "members": [],
        }))
        .send().await.unwrap();
    println!("POST /trees/test-id -> {} body={:?}", resp.status(), resp.text().await);

    // Test GET /trees
    let resp = client.get("http://127.0.0.1:9876/trees").send().await.unwrap();
    println!("GET /trees -> {} body={:?}", resp.status(), resp.text().await);

    // Test POST /headstash/test-id
    let resp = client.post("http://127.0.0.1:9876/headstash/test-id")
        .json(&serde_json::json!({"data": "test"}))
        .send().await.unwrap();
    println!("POST /headstash/test-id -> {} body={:?}", resp.status(), resp.text().await);
}