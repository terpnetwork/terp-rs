//! Zigner-sized authz session grant for a large governance message.
//!
//! The cold key (proposer) signs only this small amino tx on Zigner:
//! GenericAuthorization for each `--type` (default /cosmos.gov.v1.MsgSubmitProposal
//! plus /cosmos.authz.v1beta1.MsgRevoke so the session key can close) and a
//! BasicAllowance fee grant, both expiring after `--ttl-secs`. The on-device
//! ephemeral key then broadcasts MsgExec{MsgSubmitProposal} (any size).
//! Nothing is signed or broadcast here.
//!
//! cargo run --bin gov-session-grant -- --chain-id C --granter G --grantee E \
//!   --account-number N --sequence S [--ttl-secs 3600] [--gas 400000] \
//!   [--fee-amount 20000] [--fee-denom uthiol] [--allowance 25000000] \
//!   [--type URL]... [--out DIR]
use cw_orch_kms::{
    plan_session_setup, set_approval_chain, zigner_frames, zigner_qr_hex, ApprovalFactor, ChainProfile,
    GranteeState, RunAllowlist, SessionGrant, SessionSetup, SetupChain,
};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

const ZIGNER_MAX: usize = 65536;

fn main() -> Result<(), String> {
    let mut a = std::collections::HashMap::<String, String>::new();
    let mut types: Vec<String> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().ok_or(format!("{k} needs a value"))?;
        if k == "--type" { types.push(v) } else { a.insert(k.trim_start_matches("--").to_string(), v); }
    }
    let get = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let num = |k: &str, d: u64| a.get(k).map(|v| v.parse::<u64>().map_err(|e| format!("--{k}: {e}"))).unwrap_or(Ok(d));
    if types.is_empty() {
        types = vec!["/cosmos.gov.v1.MsgSubmitProposal".into(), "/cosmos.authz.v1beta1.MsgRevoke".into()];
    }
    let chain_id = get("chain-id")?;
    let granter = get("granter")?;
    let grantee = get("grantee")?;
    let ttl = num("ttl-secs", 3600)?;
    cw_orch_kms::validate_ttl(ttl)?;
    let fee_denom = a.get("fee-denom").cloned().unwrap_or("uthiol".into());
    let now = match a.get("now-unix") { Some(v) => v.parse().map_err(|e| format!("{e}"))?, None => SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() };
    set_approval_chain(ChainProfile::new(&chain_id, "terp", &fee_denom, 0.05, "terp", "Terp"))?;
    let mut allow = RunAllowlist::default();
    for t in &types { allow = allow.allow_type(t.clone()); }
    let grant = SessionGrant {
        grantee: grantee.clone(), expiration_unix: now + ttl, session_id: format!("gov-{grantee}"),
        allowlist: allow, factor: ApprovalFactor::Zafu, nonce: [0u8; 32],
    };
    let chain = SetupChain {
        chain_id: chain_id.clone(), account_number: num("account-number", u64::MAX)?, sequence: num("sequence", u64::MAX)?,
        gas: num("gas", 400_000)?, fee_denom: fee_denom.clone(), fee_amount: num("fee-amount", 20_000)? as u128,
    };
    if chain.account_number == u64::MAX || chain.sequence == u64::MAX { return Err("--account-number and --sequence are required".into()); }
    let empty = GranteeState { grantee: grantee.clone(), authz: vec![], feegrant: None };
    let tx = match plan_session_setup(&granter, &grant, &empty, &fee_denom, num("allowance", 25_000_000)? as u128, &chain)? {
        SessionSetup::Sign(tx) => tx,
        SessionSetup::Ready => return Err("nothing to sign".into()),
    };
    let qr = zigner_qr_hex(0, &tx.amino);
    let payload: Vec<u8> = (0..qr.len()).step_by(2).map(|i| u8::from_str_radix(&qr[i..i + 2], 16).unwrap()).collect();
    let frames = zigner_frames(&payload)?;
    let size = tx.amino.len();
    let meta = serde_json::json!({
        "chain_id": chain_id, "granter": granter, "grantee": grantee, "type_urls": types,
        "expiration_unix": now + ttl, "sign_doc_bytes": size, "zigner_max": ZIGNER_MAX,
        "fits_zigner": size <= ZIGNER_MAX, "qr_payload_bytes": payload.len(), "qr_frames": frames.len(),
        "sign_doc_sha256": Sha256::digest(&tx.amino).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "proto_msgs_hex": tx.proto.iter().map(|m| m.iter().map(|b| format!("{b:02x}")).collect::<String>()).collect::<Vec<_>>(),
        "signed": false,
    });
    if let Some(out) = a.get("out") {
        std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
        std::fs::write(format!("{out}/sign_doc.json"), &tx.amino).map_err(|e| e.to_string())?;
        std::fs::write(format!("{out}/zigner_qr.hex"), &qr).map_err(|e| e.to_string())?;
        std::fs::write(format!("{out}/meta.json"), serde_json::to_string_pretty(&meta).unwrap()).map_err(|e| e.to_string())?;
    }
    println!("{}", serde_json::to_string_pretty(&meta).unwrap());
    if size > ZIGNER_MAX { return Err(format!("sign doc {size} B exceeds Zigner max")); }
    Ok(())
}
