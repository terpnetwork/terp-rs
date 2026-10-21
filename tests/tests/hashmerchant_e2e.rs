// //! HashMerchant E2E — Unified workflow test for the Terp Network ecosystem.
// //!
// //! Exercises every custom capability end-to-end:
// //!
// //! Phase 1 — Spawn Environment:
// //!   - Preflight config generation for hash-market-server
// //!   - Server startup + health check + provider registration
// //!
// //! Phase 2 — Generate Merkle Tree Data:
// //!   - Mock dispensary loyalty DB state transitions (100 members)
// //!   - Merkle tree construction from member data
// //!   - Headstash + ecash distribution note generation
// //!   - Mock ETH light client state root transitions
// //!
// //! Phase 3 — Upload & Mirror:
// //!   - Tree data upload via POST /trees/{id}
// //!   - State root mirroring via POST /vote-extension
// //!   - BUD blob inclusion list retrieval (GET /blobs, GET /blobs/{hash})
// //!   - Headstash path-based download (GET /headstash/{id})
// //!
// //! Phase 4 — Validate:
// //!   - Merkle proof of inclusion verification
// //!   - File content integrity check (hash match)
// //!   - On-chain vote extension signature verification
// //!   - Full VE pipeline: /extend-vote → /verify-vote-extension

// use std::collections::HashMap;
// use std::path::PathBuf;

// use anyhow::{Context, Result};
// use sha2::{Digest, Sha256};
// use serde_json::json;
// use tokio::process::Command;

// const TEST_CHAIN_ID: &str = "terp-test-1";
// const PORT: u16 = 19090;

// // ---------------------------------------------------------------------------
// // Phase 1: Spawn Environment
// // ---------------------------------------------------------------------------

// /// Generate a unique test config for hash-market-server.
// fn generate_test_config() -> (tempfile::TempDir, String) {
//     let dir = tempfile::tempdir().expect("tempdir");
//     let config_path = dir.path().join("config.toml");

//     let content = format!(
//         r#"bind = "0.0.0.0:{PORT}"
// chain_id = "{TEST_CHAIN_ID}"
// signing_key = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"

// [[providers]]
// name = "test-provider-1"
// chain_uid = "mock-eth-1"
// algo = "keccak256"
// mode = "http"
// address = "http://127.0.0.1:8545"
// interval_secs = 30

// [[providers]]
// name = "test-provider-2"
// chain_uid = "loyalty-db"
// algo = "sha256"
// mode = "http"
// address = "http://127.0.0.1:9999"
// interval_secs = 30
// "#
//     );

//     std::fs::write(&config_path, &content).expect("write config");
//     (dir, config_path.to_string_lossy().to_string())
// }

// /// Find the hash-market-server binary.
// fn find_server_binary() -> PathBuf {
//     let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
//     let candidates = [
//         manifest.join("../../target/debug/hash-market-server"),
//         manifest.join("../../target/release/hash-market-server"),
//         manifest.join("../../tools/hash-market/target/debug/hash-market-server"),
//     ];
//     for c in &candidates {
//         if c.exists() {
//             return c.canonicalize().unwrap_or_else(|_| c.clone());
//         }
//     }
//     panic!(
//         "hash-market-server binary not found. Build it:\n  \
//          cd {} && cargo build --features server,ve,blossom,client,nostr",
//         manifest.join("../../tools/hash-market").display()
//     );
// }

// /// Start the hash-market-server as a subprocess. Returns the child handle.
// fn start_server(binary: &PathBuf, config_path: &str) -> std::io::Result<tokio::process::Child> {
//     Command::new(binary)
//         .args(["-c", config_path])
//         .env("RUST_LOG", "info")
//         .stdout(std::process::Stdio::piped())
//         .stderr(std::process::Stdio::piped())
//         .kill_on_drop(true)
//         .spawn()
// }

// /// Wait for the health endpoint to return OK.
// async fn wait_for_healthy(port: u16, timeout_secs: u64) -> Result<()> {
//     let url = format!("http://127.0.0.1:{port}/health");
//     let client = reqwest::Client::builder()
//         .timeout(std::time::Duration::from_secs(2))
//         .build()?;

//     let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
//     loop {
//         if std::time::Instant::now() > deadline {
//             anyhow::bail!("server did not become healthy within {timeout_secs}s");
//         }
//         match client.get(&url).send().await {
//             Ok(resp) if resp.status().is_success() => {
//                 let body: serde_json::Value = resp.json().await?;
//                 if body["status"] == "ok" {
//                     println!("  ✓ health: {body}");
//                     return Ok(());
//                 }
//             }
//             _ => {}
//         }
//         tokio::time::sleep(std::time::Duration::from_millis(300)).await;
//     }
// }

// // ---------------------------------------------------------------------------
// // Phase 2: Generate Merkle Tree Data (Mock Dispensary Loyalty DB)
// // ---------------------------------------------------------------------------

// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// struct MemberRecord {
//     member_id: u64,
//     points: u64,
//     tier: String,
//     lifetime_visits: u64,
//     last_active_unix: u64,
//     bonus_multiplier: f64,
// }

// /// Generate 100 mock dispensary loyalty DB member records.
// fn generate_mock_members() -> Vec<MemberRecord> {
//     let now = std::time::SystemTime::now()
//         .duration_since(std::time::UNIX_EPOCH)
//         .unwrap()
//         .as_secs();

//     (0..100)
//         .map(|i| {
//             let tier = match i % 5 {
//                 0 => "platinum",
//                 1 => "gold",
//                 2 => "silver",
//                 3 => "bronze",
//                 _ => "unranked",
//             };
//             let multiplier = match tier {
//                 "platinum" => 3.0,
//                 "gold" => 2.0,
//                 "silver" => 1.5,
//                 "bronze" => 1.0,
//                 _ => 0.5,
//             };
//             MemberRecord {
//                 member_id: 1000 + i as u64,
//                 points: (1000 + i * 50) as u64,
//                 tier: tier.to_string(),
//                 lifetime_visits: (i * 3) as u64,
//                 last_active_unix: now - (i as u64 * 86400),
//                 bonus_multiplier: multiplier,
//             }
//         })
//         .collect()
// }

// /// Build a merkle tree from member records. Returns (root_hash, nodes_map).
// fn build_merkle_tree(members: &[MemberRecord]) -> ([u8; 32], HashMap<String, Vec<u8>>) {
//     // Each leaf = SHA256 of serialized member record
//     let leaves: Vec<Vec<u8>> = members
//         .iter()
//         .map(|m| {
//             let json = serde_json::to_vec(m).unwrap();
//             Sha256::digest(&json).to_vec()
//         })
//         .collect();

//     let mut nodes: HashMap<String, Vec<u8>> = HashMap::new();
//     let mut current = leaves.clone();

//     // Assign leaf hashes with their paths
//     for (i, leaf) in leaves.iter().enumerate() {
//         nodes.insert(format!("{:04x}", i), leaf.clone());
//     }

//     // Build tree bottom-up
//     let mut level = 0;
//     while current.len() > 1 {
//         let mut next: Vec<Vec<u8>> = Vec::new();
//         let mut pair_idx = 0;
//         for chunk in current.chunks(2) {
//             if chunk.len() == 2 {
//                 let combined = [chunk[0].as_slice(), chunk[1].as_slice()].concat();
//                 let hash = Sha256::digest(&combined).to_vec();
//                 nodes.insert(format!("l{level}p{pair_idx}"), hash.clone());
//                 next.push(hash);
//             } else {
//                 // Odd leaf — promote
//                 next.push(chunk[0].clone());
//             }
//             pair_idx += 1;
//         }
//         current = next;
//         level += 1;
//     }

//     let root = current[0].clone().try_into().expect("32 bytes");
//     (root, nodes)
// }

// /// Generate a mock headstash distribution note.
// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// struct HeadstashNote {
//     note_id: String,
//     distribution_round: u64,
//     member_id: u64,
//     allocated_amount: String,
//     merkle_proof: Vec<String>,
//     created_at: u64,
//     expires_at: u64,
//     memo: String,
// }

// fn generate_headstash_note(
//     round: u64,
//     member: &MemberRecord,
//     proof_path: Vec<String>,
// ) -> HeadstashNote {
//     let now = std::time::SystemTime::now()
//         .duration_since(std::time::UNIX_EPOCH)
//         .unwrap()
//         .as_secs();

//     let note_id = format!("note-{}-{}", round, member.member_id);

//     HeadstashNote {
//         note_id,
//         distribution_round: round,
//         member_id: member.member_id,
//         allocated_amount: format!("{}", member.points * 10_000),
//         merkle_proof: proof_path,
//         created_at: now,
//         expires_at: now + 86400 * 90,
//         memo: format!(
//             "Loyalty distribution round {round} for member {} ({})",
//             member.member_id, member.tier
//         ),
//     }
// }

// /// Generate mock ETH-like state root transitions.
// fn generate_mock_state_roots(count: usize) -> Vec<([u8; 32], u64)> {
//     (0..count)
//         .map(|i| {
//             let mut root = [0u8; 32];
//             // Simulate changing state roots
//             let data = format!("mock-block-{}", i * 2);
//             root.copy_from_slice(&Sha256::digest(data.as_bytes())[..32]);
//             let height = (i * 2 + 1) as u64;
//             (root, height)
//         })
//         .collect()
// }

// // ---------------------------------------------------------------------------
// // Phase 2b: ECash distribution note
// // ---------------------------------------------------------------------------

// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// struct ECashNote {
//     denom: String,
//     amount: String,
//     blinded_serial: String,
//     commitment: String,
//     created_at: u64,
//     expires_at: u64,
// }

// fn generate_ecash_notes(members: &[MemberRecord]) -> Vec<ECashNote> {
//     let now = std::time::SystemTime::now()
//         .duration_since(std::time::UNIX_EPOCH)
//         .unwrap()
//         .as_secs();

//     members
//         .iter()
//         .map(|m| {
//             let blinded = Sha256::digest(format!("ecash-blinding-{}", m.member_id).as_bytes());
//             let commitment = Sha256::digest(format!("ecash-commit-{}", m.member_id).as_bytes());
//             ECashNote {
//                 denom: "uhash".to_string(),
//                 amount: format!("{}", m.points * 1000),
//                 blinded_serial: hex::encode(blinded),
//                 commitment: hex::encode(commitment),
//                 created_at: now,
//                 expires_at: now + 86400 * 30,
//             }
//         })
//         .collect()
// }

// // ---------------------------------------------------------------------------
// // Phase 3: Upload & Mirror
// // ---------------------------------------------------------------------------

// /// Upload a merkle tree by ID to hash-market-server.
// async fn upload_tree(
//     client: &reqwest::Client,
//     base: &str,
//     tree_id: &str,
//     tree_data: &serde_json::Value,
// ) -> Result<()> {
//     let url = format!("{base}/trees/{tree_id}");
//     let resp = client
//         .post(&url)
//         .json(tree_data)
//         .send()
//         .await
//         .context("POST /trees/{id}")?;
//     let status = resp.status();
//     let body = resp.text().await?;
//     assert!(
//         status.is_success() || status.as_u16() == 409,
//         "upload_tree {tree_id} expected 200/409, got {status}: {body}"
//     );
//     println!("  ✓ tree '{tree_id}' uploaded (status {status})");
//     Ok(())
// }

// /// Upload a headstash note by ID.
// async fn upload_headstash(
//     client: &reqwest::Client,
//     base: &str,
//     note_id: &str,
//     note: &serde_json::Value,
// ) -> Result<()> {
//     let url = format!("{base}/headstash/{note_id}");
//     let resp = client
//         .post(&url)
//         .json(note)
//         .send()
//         .await
//         .context("POST /headstash/{id}")?;
//     let status = resp.status();
//     let body = resp.text().await?;
//     assert!(
//         status.is_success() || status.as_u16() == 409,
//         "upload_headstash {note_id} expected 200/409, got {status}: {body}"
//     );
//     println!("  ✓ headstash note '{note_id}' uploaded (status {status})");
//     Ok(())
// }

// /// Feed a vote extension data to the server (mocks gRPC transport).
// async fn feed_vote_extension(
//     client: &reqwest::Client,
//     base: &str,
//     chain_uid: &str,
//     algo: &str,
//     root: &[u8],
//     height: u64,
// ) -> Result<()> {
//     let url = format!("{base}/vote-extension");
//     let body = json!({
//         "runtime_id": "hashmerchant-e2e",
//         "chain_uid": chain_uid,
//         "algo": algo,
//         "root": hex::encode(root),
//         "foreign_height": height,
//         "foreign_block_time": std::time::SystemTime::now()
//             .duration_since(std::time::UNIX_EPOCH)
//             .unwrap()
//             .as_secs() as i64,
//     });

//     let resp = client
//         .post(&url)
//         .json(&body)
//         .send()
//         .await
//         .context("POST /vote-extension")?;
//     let status = resp.status();
//     let body_text = resp.text().await?;
//     assert!(
//         status.is_success(),
//         "feed_vote_extension expected 200, got {status}: {body_text}"
//     );
//     println!(
//         "  ✓ vote-extension fed: chain={chain_uid} algo={algo} height={height} root=0x{}",
//         hex::encode(root)
//     );
//     Ok(())
// }

// /// List all blobs.
// async fn list_blobs(client: &reqwest::Client, base: &str) -> Result<Vec<String>> {
//     let url = format!("{base}/blobs");
//     let resp = client.get(&url).send().await?;
//     let hashes: Vec<String> = resp.json().await?;
//     println!("  ✓ blob list returned {} entries", hashes.len());
//     Ok(hashes)
// }

// /// Upload a blob and return its hash.
// async fn upload_blob(client: &reqwest::Client, base: &str, data: &[u8]) -> Result<String> {
//     let url = format!("{base}/upload");
//     let resp = client
//         .post(&url)
//         .body(data.to_vec())
//         .send()
//         .await?;
//     let json: serde_json::Value = resp.json().await?;
//     let hash = json["hash"].as_str().unwrap_or("?").to_string();
//     println!("  ✓ blob uploaded: {hash} ({} bytes)", data.len());
//     Ok(hash)
// }

// /// Download a blob by hash and verify contents.
// async fn download_blob_and_verify(
//     client: &reqwest::Client,
//     base: &str,
//     hash: &str,
//     expected_hash: &[u8],
// ) -> Result<()> {
//     let url = format!("{base}/blobs/{hash}");
//     let resp = client.get(&url).send().await?;
//     assert!(resp.status().is_success(), "GET /blobs/{hash} failed: {}", resp.status());
//     let data = resp.bytes().await?;
//     let actual_hash = Sha256::digest(&data);
//     assert_eq!(
//         actual_hash[..],
//         *expected_hash,
//         "blob content hash mismatch for {hash}"
//     );
//     println!("  ✓ blob '{hash}' downloaded and content-verified ({} bytes)", data.len());
//     Ok(())
// }

// /// Get vote extension data.
// async fn get_vote_extension(client: &reqwest::Client, base: &str) -> Result<serde_json::Value> {
//     let url = format!("{base}/vote-extension");
//     let resp = client.get(&url).send().await?;
//     let json: serde_json::Value = resp.json().await?;
//     println!("  ✓ GET /vote-extension returned data");
//     Ok(json)
// }

// /// Test the extend-vote + verify-vote-extension pipeline.
// async fn test_ve_pipeline(client: &reqwest::Client, base: &str) -> Result<()> {
//     // POST /extend-vote — produce signed extension
//     let extend_body = json!({
//         "height": 42,
//         "chain_uid": "mock-eth-1",
//         "algo": "keccak256",
//     });
//     let resp = client
//         .post(format!("{base}/extend-vote"))
//         .json(&extend_body)
//         .send()
//         .await?;
//     let extend: serde_json::Value = resp.json().await?;
//     assert!(
//         extend.get("extension").is_some(),
//         "/extend-vote must return extension"
//     );
//     assert!(
//         extend.get("signature").is_some(),
//         "/extend-vote must return signature"
//     );
//     println!("  ✓ POST /extend-vote: extension + signature returned");

//     // POST /verify-vote-extension — verify the signature
//     let verify_body = json!({
//         "height": 42,
//         "extension": extend["extension"],
//         "signature": extend["signature"],
//         "public_key": extend["public_key"],
//     });
//     let resp = client
//         .post(format!("{base}/verify-vote-extension"))
//         .json(&verify_body)
//         .send()
//         .await?;
//     let verify: serde_json::Value = resp.json().await?;
//     assert_eq!(verify["valid"].as_bool(), Some(true), "signature must be valid");
//     println!("  ✓ POST /verify-vote-extension: VALID signature");
//     Ok(())
// }

// // ---------------------------------------------------------------------------
// // Phase 4: Validate
// // ---------------------------------------------------------------------------

// /// Verify merkle proof of inclusion for a specific member.
// fn verify_merkle_inclusion(
//     root: &[u8; 32],
//     member: &MemberRecord,
//     proof_hashes: &[String],
// ) -> bool {
//     let leaf = Sha256::digest(&serde_json::to_vec(member).unwrap());
//     let mut computed = leaf.to_vec();

//     for proof_hash_hex in proof_hashes {
//         let proof_bytes = hex::decode(proof_hash_hex).unwrap();
//         let combined = [computed.as_slice(), proof_bytes.as_slice()].concat();
//         computed = Sha256::digest(&combined).to_vec();
//     }

//     computed.as_slice() == root
// }

// /// Generate a merkle proof path for a given leaf index.
// fn generate_merkle_proof(members: &[MemberRecord], leaf_idx: usize) -> Vec<String> {
//     let leaves: Vec<Vec<u8>> = members
//         .iter()
//         .map(|m| Sha256::digest(&serde_json::to_vec(m).unwrap()).to_vec())
//         .collect();

//     let mut proof = Vec::new();
//     let mut current = leaves;
//     let mut idx = leaf_idx;

//     while current.len() > 1 {
//         let mut next: Vec<Vec<u8>> = Vec::new();

//         let pair = if idx % 2 == 0 {
//             // Need sibling to the right
//             if idx + 1 < current.len() {
//                 Some(current[idx + 1].clone())
//             } else {
//                 None
//             }
//         } else {
//             // Sibling to the left
//             Some(current[idx - 1].clone())
//         };

//         if let Some(sibling) = pair {
//             proof.push(hex::encode(sibling));
//         }

//         for chunk in current.chunks(2) {
//             if chunk.len() == 2 {
//                 let combined = [chunk[0].as_slice(), chunk[1].as_slice()].concat();
//                 next.push(Sha256::digest(&combined).to_vec());
//             } else {
//                 next.push(chunk[0].clone());
//             }
//         }

//         idx = idx / 2;
//         current = next;
//     }

//     proof
// }

// // ---------------------------------------------------------------------------
// // Main
// // ---------------------------------------------------------------------------

// #[tokio::main]
// async fn main() -> Result<()> {
//     println!("═══ HashMerchant Unified E2E ═══");
//     println!();

//     // ── Phase 1: Spawn Environment ──────────────────────────────────────
//     println!("[Phase 1] Spawning hash-market-server...");
//     let binary = find_server_binary();
//     println!("  binary: {}", binary.display());

//     let (_dir, config_path) = generate_test_config();
//     println!("  config: {config_path}");

//     let mut child = start_server(&binary, &config_path)
//         .context("failed to start hash-market-server")?;
//     let pid = child.id().unwrap_or(0);
//     println!("  PID: {pid}");

//     let base = format!("http://127.0.0.1:{PORT}");

//     wait_for_healthy(PORT, 30).await
//         .context("server health check timed out")?;

//     let client = reqwest::Client::new();

//     println!();
//     println!("═══ Phase 1 complete: hash-market-server running at {base} ═══");
//     println!();

//     // ── Phase 2: Generate Merkle Tree Data ──────────────────────────────
//     println!("[Phase 2] Generating merkle tree data...");

//     // 2a. Generate mock dispensary loyalty DB members
//     println!("  generating 100 mock dispensary members...");
//     let members = generate_mock_members();
//     println!(
//         "  members generated: {} ({}..{})",
//         members.len(),
//         members.first().unwrap().member_id,
//         members.last().unwrap().member_id
//     );

//     // 2b. Build merkle trees from member data
//     println!("  building merkle trees...");
//     let (root, nodes) = build_merkle_tree(&members);
//     println!("  merkle root: 0x{}", hex::encode(root));

//     // Upload the full tree
//     let tree_data = json!({
//         "root": hex::encode(root),
//         "leaf_count": members.len(),
//         "nodes": nodes.iter().map(|(k, v)| (k.clone(), hex::encode(v))).collect::<HashMap<_, _>>(),
//         "created_at": std::time::SystemTime::now()
//             .duration_since(std::time::UNIX_EPOCH)
//             .unwrap()
//             .as_secs(),
//     });
//     upload_tree(&client, &base, "loyalty-db-v1", &tree_data).await?;

//     // 2c. Generate headstash + ecash distribution notes
//     println!("  generating headstash + ecash notes...");
//     let round = 1u64;
//     for (i, member) in members.iter().enumerate().take(5) {
//         // Generate merkle proof for this member
//         let proof = generate_merkle_proof(&members, i);
//         let note = generate_headstash_note(round, member, proof);
//         let note_id = format!("{}-{}", round, member.member_id);

//         upload_headstash(&client, &base, &note_id, &json!(note)).await?;
//     }

//     // Upload ecash notes as headstash sub-records
//     let ecash_notes = generate_ecash_notes(&members);
//     for (i, ecash) in ecash_notes.iter().enumerate().take(5) {
//         upload_headstash(&client, &base, &format!("ecash-{}", members[i].member_id), &json!(ecash)).await?;
//     }

//     // 2d. Generate mock ETH state root transitions
//     println!("  generating mock ETH state root transitions...");
//     let state_roots = generate_mock_state_roots(3);
//     for (sr, height) in &state_roots {
//         let _ = feed_vote_extension(
//             &client,
//             &base,
//             "mock-eth-1",
//             "keccak256",
//             sr,
//             *height,
//         ).await;
//     }
//     // Also feed a loyalty-db root
//     let _ = feed_vote_extension(
//         &client,
//         &base,
//         "loyalty-db",
//         "sha256",
//         &root,
//         1,
//     ).await;

//     println!();
//     println!("═══ Phase 2 complete: data generated and uploaded ═══");
//     println!();

//     // ── Phase 3: Upload & Mirror ────────────────────────────────────────
//     println!("[Phase 3] Verifying data mirroring and retrieval...");

//     // 3a. Verify vote extension data is served
//     println!("  checking vote extension endpoint...");
//     let ve_data = get_vote_extension(&client, &base).await?;
//     println!("    chains in VE data: {:?}", ve_data.get("chain_uid"));

//     // 3b. Upload some BUD blobs and verify listing
//     println!("  uploading BUD blobs...");
//     let blob_contents = [
//         b"{\"event\":\"member_joined\",\"member_id\":1000,\"tier\":\"platinum\"}",
//         b"{\"event\":\"points_redeemed\",\"member_id\":1001,\"amount\":5000}",
//         b"{\"event\":\"distribution_round_1\",\"total\":100,\"allocated\":100}",
//         b"{\"event\":\"merkle_root_mirrored\",\"root\":\"",
//     ];
//     let mut uploaded_hashes = Vec::new();
//     for content in &blob_contents {
//         let hash = upload_blob(&client, &base, content).await?;
//         uploaded_hashes.push(hash);
//     }

//     // 3c. List blobs and verify count
//     println!("  listing blobs...");
//     let blob_list = list_blobs(&client, &base).await?;
//     assert_eq!(blob_list.len(), blob_contents.len(), "blob count mismatch");

//     // 3d. Download and verify each blob
//     println!("  downloading and verifying blobs...");
//     for (content, hash) in blob_contents.iter().zip(blob_list.iter()) {
//         let expected_hash = Sha256::digest(content);
//         download_blob_and_verify(&client, &base, hash, &expected_hash).await?;
//     }

//     // 3e. Download headstash notes and verify contents
//     println!("  verifying headstash notes...");
//     for i in 0..5 {
//         let member = &members[i];
//         let url = format!("{base}/headstash/{round}-{}", member.member_id);
//         let resp = client.get(&url).send().await?;
//         assert!(resp.status().is_success(), "GET /headstash/{i} failed");
//         let note: serde_json::Value = resp.json().await?;
//         assert_eq!(note["member_id"].as_u64(), Some(member.member_id));
//         println!(
//             "    note '{}': member {} allocated {}",
//             note["note_id"].as_str().unwrap_or("?"),
//             member.member_id,
//             note["allocated_amount"].as_str().unwrap_or("?")
//         );
//     }

//     println!();
//     println!("═══ Phase 3 complete: mirroring and retrieval verified ═══");
//     println!();

//     // ── Phase 4: Validate ───────────────────────────────────────────────
//     println!("[Phase 4] Validating proofs and signatures...");

//     // 4a. Verify merkle proof of inclusion for the first member
//     println!("  verifying merkle proof of inclusion...");
//     let first_member = &members[0];
//     let proof = generate_merkle_proof(&members, 0);
//     let included = verify_merkle_inclusion(&root, first_member, &proof);
//     assert!(included, "member {} must be provably included in merkle tree", first_member.member_id);
//     println!("    ✓ member {} included in loyalty-db merkle tree", first_member.member_id);

//     // Verify a non-present hash would NOT be provable (negative case)
//     // We use a modified record to test exclusion
//     let mut tampered = first_member.clone();
//     tampered.points += 1;
//     let excluded = verify_merkle_inclusion(&root, &tampered, &proof);
//     assert!(!excluded, "tampered record must NOT be provably included");
//     println!("    ✓ tampered record correctly excluded from proof");

//     // 4b. Test the full VE signature pipeline
//     println!("  testing vote extension signature pipeline...");
//     test_ve_pipeline(&client, &base).await?;

//     // 4c. Verify blob content integrity (already done in Phase 3)
//     // re-download and verify cross-check
//     println!("  cross-checking blob integrity...");
//     for hash in &uploaded_hashes {
//         let url = format!("{base}/blobs/{hash}");
//         let resp = client.get(&url).send().await?;
//         assert!(resp.status().is_success());
//         let data = resp.bytes().await?;
//         let actual_hash = hex::encode(Sha256::digest(&data));
//         assert_eq!(
//             &actual_hash, hash,
//             "blob {hash} integrity check failed"
//         );
//         println!("    ✓ blob {hash} SHA256 integrity verified");
//     }

//     println!();
//     println!("═══ Phase 4 complete: all validations passed ═══");
//     println!();

//     // ── Cleanup ──────────────────────────────────────────────────────────
//     println!("[Cleanup] Stopping hash-market-server...");
//     child.kill().await.ok();
//     child.wait().await.ok();
//     println!("  PID {pid} stopped");

//     println!();
//     println!("═══ ALL PHASES PASSED ═══");
//     println!();
//     println!("Workflows exercised:");
//     println!("  1. Server startup with provider registration");
//     println!("  2. Mock dispensary loyalty DB data generation (100 members)");
//     println!("  3. Merkle tree construction and upload (POST /trees/{{id}})");
//     println!("  4. Headstash + ecash distribution note generation/upload");
//     println!("  5. Mock ETH state root mirroring via vote extensions");
//     println!("  6. BUD blob upload, listing, download, integrity verification");
//     println!("  7. Merkle proof of inclusion (positive + negative cases)");
//     println!("  8. Vote extension signature pipeline (/extend-vote → /verify)");
//     println!("  9. Headstash path-based retrieval and content verification");
//     println!("  10. Blob SHA256 integrity cross-check");

//     Ok(())
// }
