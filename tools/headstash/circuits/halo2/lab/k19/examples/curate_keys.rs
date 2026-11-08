//! Offline Product A store-circuit blob for wasmvm / cw-orch upload.

fn main() {
    let path = std::env::var("HEADSTASH_VK_PATH").unwrap_or_else(|_| {
        "artifacts/headstash_vk.bin".to_string()
    });
    zk_headstash::suite::build_headstash_keys_to(&path).unwrap_or_else(|e| {
        panic!("keygen: {e}");
    });
    eprintln!("offline keygen complete: {path}");
    eprintln!("next: store-circuit / upload_circuit with this blob");
}
