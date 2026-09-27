//! Ops CLI: live `notes_base` + blossom/snap auth + snap recover.
//!
//! ```bash
//! cargo run -p terp-seams --features cli --bin headstash-notes -- \
//!   put --base http://127.0.0.1:9090 --hs-id season-1 \
//!   --owner-key <64hex> --seam-json note.json --auth-bearer "$TOKEN"
//!
//! cargo run -p terp-seams --features cli --bin headstash-notes -- \
//!   recover --base http://127.0.0.1:9090 --hs-id season-1 \
//!   --addr cm.deadbeef --owner-key <64hex> --auth-bearer "$TOKEN"
//! ```
//!
//! Env: `NOTES_BASE`, `NOTES_BEARER_TOKEN`, `NOTES_SECP_SK`, `NOTES_OWNER_KEY`.

use clap::{Parser, Subcommand};
use terp_seams::dex::note::{
    auth_headers_from_env, bearer_auth_headers, decrypt_note_out, get_note_envelope,
    list_note_keys, note_addr_cm, persist_plan_from_seam_note_opts, pir_get_note_envelope,
    put_note_envelope, EncryptNoteOpts, SeamNoteOutV0,
};
#[cfg(feature = "auth")]
use terp_seams::dex::note::snap_secp_auth_headers;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "headstash-notes",
    about = "Headstash private notes ops: put / get / list / recover / pir-recover against notes_base"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Encrypt SEAM cleartext JSON and PUT to hash-market notes store.
    Put {
        #[arg(long, env = "NOTES_BASE")]
        base: String,
        #[arg(long)]
        hs_id: String,
        /// Path to SeamNoteOutV0 JSON (full field object) OR raw 382-byte hex file when --seam-hex.
        #[arg(long)]
        seam_json: Option<PathBuf>,
        /// Hex of 382B cleartext (alternative to --seam-json).
        #[arg(long)]
        seam_hex: Option<String>,
        /// Override addr (default: cm.<hex(cm_public)>).
        #[arg(long)]
        addr: Option<String>,
        #[arg(long, env = "NOTES_OWNER_KEY")]
        owner_key: String,
        #[arg(long, env = "NOTES_BEARER_TOKEN")]
        auth_bearer: Option<String>,
        #[arg(long, env = "NOTES_SECP_SK")]
        auth_secp_sk: Option<String>,
        /// Attach envelope.sha256 of ciphertext (distribution on).
        #[arg(long, default_value_t = false)]
        with_sha256: bool,
    },
    /// GET encrypted envelope (no decrypt).
    Get {
        #[arg(long, env = "NOTES_BASE")]
        base: String,
        #[arg(long)]
        hs_id: String,
        #[arg(long)]
        addr: String,
        #[arg(long, env = "NOTES_BEARER_TOKEN")]
        auth_bearer: Option<String>,
        #[arg(long, env = "NOTES_SECP_SK")]
        auth_secp_sk: Option<String>,
    },
    /// GET + decrypt with owner key (snap recover UX).
    Recover {
        #[arg(long, env = "NOTES_BASE")]
        base: String,
        #[arg(long)]
        hs_id: String,
        #[arg(long)]
        addr: String,
        #[arg(long, env = "NOTES_OWNER_KEY")]
        owner_key: String,
        #[arg(long, env = "NOTES_BEARER_TOKEN")]
        auth_bearer: Option<String>,
        #[arg(long, env = "NOTES_SECP_SK")]
        auth_secp_sk: Option<String>,
        /// Write cleartext hex to this path (default: stdout JSON-ish summary).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List note address keys for hs_id.
    List {
        #[arg(long, env = "NOTES_BASE")]
        base: String,
        #[arg(long)]
        hs_id: String,
        #[arg(long, env = "NOTES_BEARER_TOKEN")]
        auth_bearer: Option<String>,
        #[arg(long, env = "NOTES_SECP_SK")]
        auth_secp_sk: Option<String>,
    },
    /// PIR-fetch envelope for addr then decrypt (hides which note from server access logs pattern).
    PirRecover {
        #[arg(long, env = "NOTES_BASE")]
        base: String,
        #[arg(long)]
        hs_id: String,
        #[arg(long)]
        addr: String,
        #[arg(long, env = "NOTES_OWNER_KEY")]
        owner_key: String,
        #[arg(long, env = "NOTES_BEARER_TOKEN")]
        auth_bearer: Option<String>,
        #[arg(long, env = "NOTES_SECP_SK")]
        auth_secp_sk: Option<String>,
    },
}

fn parse_owner_key(hex_s: &str) -> Result<[u8; 32], String> {
    let b = hex::decode(hex_s.trim()).map_err(|e| e.to_string())?;
    if b.len() != 32 {
        return Err(format!("owner_key must be 32 bytes, got {}", b.len()));
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&b);
    Ok(k)
}

fn resolve_headers(
    auth_bearer: Option<&str>,
    auth_secp_sk: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();
    if let Some(t) = auth_bearer {
        if !t.is_empty() {
            headers.extend(bearer_auth_headers(t));
        }
    }
    #[cfg(feature = "auth")]
    if let Some(sk) = auth_secp_sk {
        if !sk.is_empty() {
            headers.extend(snap_secp_auth_headers(sk).map_err(|e| format!("{e:?}"))?);
        }
    }
    #[cfg(not(feature = "auth"))]
    let _ = auth_secp_sk;

    if headers.is_empty() {
        // Fall back to env (NOTES_BEARER_TOKEN / NOTES_SECP_SK)
        headers = auth_headers_from_env().map_err(|e| format!("{e:?}"))?;
    }
    Ok(headers)
}

fn header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

fn load_seam(
    seam_json: Option<&PathBuf>,
    seam_hex: Option<&str>,
) -> Result<SeamNoteOutV0, String> {
    if let Some(hex_s) = seam_hex {
        let b = hex::decode(hex_s.trim()).map_err(|e| e.to_string())?;
        return SeamNoteOutV0::from_bytes(&b).map_err(|e| format!("{e:?}"));
    }
    let path = seam_json.ok_or("--seam-json or --seam-hex required")?;
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Accept either full JSON object (serde via intermediate) or {"bytes_hex":"..."}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
        if let Some(h) = v.get("bytes_hex").and_then(|x| x.as_str()) {
            let b = hex::decode(h).map_err(|e| e.to_string())?;
            return SeamNoteOutV0::from_bytes(&b).map_err(|e| format!("{e:?}"));
        }
        // Field-wise: use from_bytes of re-encoded via manual? Prefer bytes_hex.
        // Also accept envelope already encrypted under wrong path — reject.
        if v.get("ciphertext").is_some() {
            return Err("seam_json looks like an envelope; use get/recover".into());
        }
    }
    // Raw hex file content
    let trimmed = data.trim();
    if trimmed.chars().all(|c| c.is_ascii_hexdigit()) && trimmed.len() == 382 * 2 {
        let b = hex::decode(trimmed).map_err(|e| e.to_string())?;
        return SeamNoteOutV0::from_bytes(&b).map_err(|e| format!("{e:?}"));
    }
    Err("seam_json: provide {\"bytes_hex\":\"…764 hex…\"} or a 764-char hex file".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Put {
            base,
            hs_id,
            seam_json,
            seam_hex,
            addr,
            owner_key,
            auth_bearer,
            auth_secp_sk,
            with_sha256,
        } => {
            let note = load_seam(seam_json.as_ref(), seam_hex.as_deref())?;
            let key = parse_owner_key(&owner_key)?;
            let addr = addr.unwrap_or_else(|| note_addr_cm(&note.cm_public));
            let plan = persist_plan_from_seam_note_opts(
                &note,
                &key,
                &hs_id,
                &addr,
                EncryptNoteOpts {
                    include_ciphertext_sha256: with_sha256,
                },
            )
            .map_err(|e| format!("{e:?}"))?;
            let headers = resolve_headers(auth_bearer.as_deref(), auth_secp_sk.as_deref())?;
            let hrefs = header_refs(&headers);
            put_note_envelope(&base, &plan.hs_id, &plan.addr, &plan.envelope, &hrefs)
                .map_err(|e| format!("{e:?}"))?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "hs_id": plan.hs_id,
                    "addr": plan.addr,
                    "store_path": plan.store_path,
                    "sha256": plan.envelope.sha256,
                })
            );
        }
        Cmd::Get {
            base,
            hs_id,
            addr,
            auth_bearer,
            auth_secp_sk,
        } => {
            let headers = resolve_headers(auth_bearer.as_deref(), auth_secp_sk.as_deref())?;
            let hrefs = header_refs(&headers);
            let env = get_note_envelope(&base, &hs_id, &addr, &hrefs).map_err(|e| format!("{e:?}"))?;
            println!("{}", serde_json::to_string_pretty(&env)?);
        }
        Cmd::Recover {
            base,
            hs_id,
            addr,
            owner_key,
            auth_bearer,
            auth_secp_sk,
            out,
        } => {
            let headers = resolve_headers(auth_bearer.as_deref(), auth_secp_sk.as_deref())?;
            let hrefs = header_refs(&headers);
            let key = parse_owner_key(&owner_key)?;
            let env = get_note_envelope(&base, &hs_id, &addr, &hrefs).map_err(|e| format!("{e:?}"))?;
            let note = decrypt_note_out(&env, &key).map_err(|e| format!("{e:?}"))?;
            let hex_out = hex::encode(note.to_bytes());
            if let Some(p) = out {
                std::fs::write(&p, &hex_out)?;
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "path": p, "cleartext_len": 382})
                );
            } else {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "hs_id": hs_id,
                        "addr": addr,
                        "value": note.value,
                        "cm_public": hex::encode(note.cm_public),
                        "bytes_hex": hex_out,
                    })
                );
            }
        }
        Cmd::List {
            base,
            hs_id,
            auth_bearer,
            auth_secp_sk,
        } => {
            let headers = resolve_headers(auth_bearer.as_deref(), auth_secp_sk.as_deref())?;
            let hrefs = header_refs(&headers);
            let keys = list_note_keys(&base, &hs_id, &hrefs).map_err(|e| format!("{e:?}"))?;
            println!("{}", serde_json::json!({ "keys": keys }));
        }
        Cmd::PirRecover {
            base,
            hs_id,
            addr,
            owner_key,
            auth_bearer,
            auth_secp_sk,
        } => {
            let headers = resolve_headers(auth_bearer.as_deref(), auth_secp_sk.as_deref())?;
            let hrefs = header_refs(&headers);
            let key = parse_owner_key(&owner_key)?;
            let env =
                pir_get_note_envelope(&base, &hs_id, &addr, &hrefs).map_err(|e| format!("{e:?}"))?;
            let note = decrypt_note_out(&env, &key).map_err(|e| format!("{e:?}"))?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "mode": "pir",
                    "hs_id": hs_id,
                    "addr": addr,
                    "value": note.value,
                    "cm_public": hex::encode(note.cm_public),
                    "bytes_hex": hex::encode(note.to_bytes()),
                })
            );
        }
    }
    Ok(())
}
