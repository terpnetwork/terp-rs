// /// Upload a merkle tree JSON to a running merkle-server instance.
// ///
// /// Signs the request using secp256k1 ECDSA-SHA256, matching the server's
// /// auth.rs verification exactly.
// ///
// /// Canonical message signed:
// ///   "POST\n/tree/{tree_id}\n{unix_timestamp}\n{sha256hex(body)}"
// ///
// /// The private key is read from an env file and never passed as an argument.
// ///
// /// Accepts both gen_merkle output format (merkle_root + accounts) and the
// /// server's native TreeInput format (root + members) — converts automatically.
// ///
// /// Usage:
// ///   cargo run -p cw-infuser-scripts --bin upload_merkle -- \
// ///       --tree-id season-1 \
// ///       --tree-file data/terp-warriors.json \
// ///       [--server https://merkle.example.com] \
// ///       [--env .env]
// ///
// /// .env file:
// ///   MERKLE_PRIVATE_KEY=<64-char hex secp256k1 private key>
// ///   MERKLE_SERVER_URL=https://merkle.example.com   (optional if --server is set)
// use anyhow::{Context, Result};
// use clap::Parser;
// use k256::ecdsa::{signature::Signer, Signature, SigningKey};
// use sha2::{Digest, Sha256};
// use serde_json::{json, Value};
// use std::{
//     fs,
//     path::PathBuf,
//     time::{SystemTime, UNIX_EPOCH},
// };

// #[derive(Parser, Debug)]
// #[command(
//     version,
//     about = "Upload a merkle tree JSON to a merkle-server instance (signs the request)"
// )]
// struct Args {
//     /// Tree identifier (e.g. "season-1" or a contract address)
//     #[arg(long)]
//     tree_id: String,

//     /// Path to the tree JSON (gen_merkle output or TreeInput format)
//     #[arg(long)]
//     tree_file: PathBuf,

//     /// Merkle server base URL.
//     /// Falls back to MERKLE_SERVER_URL env var, then http://127.0.0.1:8765.
//     #[arg(long, env = "MERKLE_SERVER_URL", default_value = "http://127.0.0.1:8765")]
//     server: String,

//     /// Path to the .env file containing MERKLE_PRIVATE_KEY
//     #[arg(long, default_value = ".env")]
//     env_file: String,
// }

fn main() {
    //     let args = Args::parse();

    //     // Load env file — ignore if file doesn't exist (env vars may already be set)
    //     dotenv::from_filename(&args.env_file).ok();

    //     // ── Load signing key ──────────────────────────────────────────────────────
    //     let sk_hex = std::env::var("MERKLE_PRIVATE_KEY")
    //         .context("MERKLE_PRIVATE_KEY not set — add it to your .env file or environment")?;
    //     let sk_bytes = hex::decode(sk_hex.trim())
    //         .context("MERKLE_PRIVATE_KEY is not valid hex")?;
    //     let signing_key = SigningKey::from_slice(&sk_bytes)
    //         .context("MERKLE_PRIVATE_KEY is not a valid secp256k1 private key")?;

    //     // ── Load and normalise tree JSON → TreeInput { root, members } ────────────
    //     let raw = fs::read_to_string(&args.tree_file)
    //         .with_context(|| format!("failed to read: {}", args.tree_file.display()))?;
    //     let value: Value = serde_json::from_str(&raw)
    //         .context("tree file is not valid JSON")?;

    //     let body: Vec<u8> = if value.get("merkle_root").is_some() {
    //         // gen_merkle output: { merkle_root, accounts: { addr: { proof_hashes, tier, alloc } } }
    //         let root = value["merkle_root"]
    //             .as_str()
    //             .context("missing merkle_root field")?;

    //         let members: serde_json::Map<String, Value> = value["accounts"]
    //             .as_object()
    //             .context("missing accounts field")?
    //             .iter()
    //             .map(|(addr, entry)| (addr.clone(), entry["proof_hashes"].clone()))
    //             .collect();

    //         serde_json::to_vec(&json!({ "root": root, "members": members }))?
    //     } else {
    //         // Already TreeInput format: { root, members }
    //         raw.into_bytes()
    //     };

    //     let member_count = serde_json::from_slice::<Value>(&body)?["members"]
    //         .as_object()
    //         .map(|m| m.len())
    //         .unwrap_or(0);

    //     // ── Build canonical message and sign ──────────────────────────────────────
    //     let timestamp = SystemTime::now()
    //         .duration_since(UNIX_EPOCH)?
    //         .as_secs()
    //         .to_string();

    //     let path = format!("/tree/{}", args.tree_id);
    //     let body_hash = hex::encode(Sha256::digest(&body));
    //     let canonical = format!("POST\n{path}\n{timestamp}\n{body_hash}");

    //     let sig: Signature = signing_key.sign(canonical.as_bytes());
    //     let sig_hex = hex::encode(sig.to_bytes());

    //     // ── POST to server ────────────────────────────────────────────────────────
    //     let url = format!("{}{}", args.server.trim_end_matches('/'), path);
    //     eprintln!("Uploading tree '{}' ({} members) → {}", args.tree_id, member_count, url);

    //     match ureq::post(&url)
    //         .set("Content-Type", "application/json")
    //         .set("X-Timestamp", &timestamp)
    //         .set("X-Signature", &sig_hex)
    //         .send_bytes(&body)
    //     {
    //         Ok(resp) => {
    //             eprintln!("OK: {}", resp.into_string()?);
    //         }
    //         Err(ureq::Error::Status(code, resp)) => {
    //             let body = resp.into_string().unwrap_or_default();
    //             anyhow::bail!("Server returned {}: {}", code, body);
    //         }
    //         Err(e) => return Err(e.into()),
    //     }
}
