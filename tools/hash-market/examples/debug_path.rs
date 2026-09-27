//! Debug — test routes with path parameters.
use hash_market::custody::local::LocalSecp256k1;
use hash_market::server::{router as build_router, AppState};
use hash_market::client::ve::VoteExtensionHandler;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join("hashdp2");
    std::fs::create_dir_all(&dir).expect("create dir");
    let custody = LocalSecp256k1::generate();
    let ve_handler = VoteExtensionHandler::new(Box::new(custody));
    let state = Arc::new(AppState::new(Some(ve_handler), "debug".into(), dir, true).unwrap());
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9877").await.unwrap();
    println!("LISTENING ON 9877");
    tokio::spawn(async move { axum::serve(listener, app).await.ok(); });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let client = reqwest::Client::new();

    // Test GET /blobs/{hash} with valid 64-char hash
    let h = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
    let r = client.get(format!("http://127.0.0.1:9877/blobs/{h}")).send().await.unwrap();
    println!("GET /blobs/{} -> {} body={:?}", h, r.status(), r.text().await);

    // Test GET /blobs/xyz (short)
    let r = client.get("http://127.0.0.1:9877/blobs/xyz").send().await.unwrap();
    println!("GET /blobs/xyz -> {} body={:?}", r.status(), r.text().await);

    // Test DELETE /blobs/{hash}
    let r = client.delete(format!("http://127.0.0.1:9877/blobs/{h}")).send().await.unwrap();
    println!("DELETE /blobs/{} -> {} body={:?}", h, r.status(), r.text().await);

    // Test DELETE /blobs/xyz
    let r = client.delete("http://127.0.0.1:9877/blobs/xyz").send().await.unwrap();
    println!("DELETE /blobs/xyz -> {} body={:?}", r.status(), r.text().await);

    // Test POST /trees/{id} with explicit header
    let r = client.post("http://127.0.0.1:9877/trees/test-id")
        .header("content-type", "application/json")
        .body(r#"{"merkle_root":"0000000000000000000000000000000000000000000000000000000000000000","members":[]}"#)
        .send().await.unwrap();
    println!("POST /trees/test-id (explicit header) -> {} body={:?}", r.status(), r.text().await);

    // Test GET /trees after POST
    let r = client.get("http://127.0.0.1:9877/trees").send().await.unwrap();
    println!("GET /trees -> {} body={:?}", r.status(), r.text().await);

    // Test POST /headstash/{id} with explicit header
    let r = client.post("http://127.0.0.1:9877/headstash/test-hs")
        .header("content-type", "application/json")
        .body(r#"{"data":"test"}"#)
        .send().await.unwrap();
    println!("POST /headstash/test-hs (explicit header) -> {} body={:?}", r.status(), r.text().await);
}