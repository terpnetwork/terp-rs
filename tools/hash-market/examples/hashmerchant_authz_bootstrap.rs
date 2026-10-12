//! Prepare runtime → temp HashMerchant authz msgs (dry-run by default).
//! Runtime must not equal HashMerchant. Does not feegrant customers.
//! Does not print mnemonics. Does not broadcast unless AUTHZ_BROADCAST=1
//! (even then this binary only writes cosmos-msgs.json).
//!
//! ```sh
//! ONBOARD_RUNTIME=terp1… ONBOARD_HASHMERCHANT=terp1… \
//!   cargo run --example hashmerchant_authz_bootstrap
//! ```

use serde_json::json;
use std::path::PathBuf;
use hash_market::onboard_authz::{dao_authz_grants, OnboardRoles};

fn main() -> Result<(), String> {
    let runtime = std::env::var("ONBOARD_RUNTIME").map_err(|_| "ONBOARD_RUNTIME required")?;
    let hm = std::env::var("ONBOARD_HASHMERCHANT").map_err(|_| "ONBOARD_HASHMERCHANT required")?;
    if !runtime.starts_with("terp1") || !hm.starts_with("terp1") {
        return Err("both keys must be terp1".into());
    }
    if runtime == hm {
        return Err("collapsed keys: runtime must not equal hashmerchant".into());
    }
    if std::env::var("ONBOARD_ALLOW_COLLAPSE").ok().as_deref() == Some("1") {
        return Err("ONBOARD_ALLOW_COLLAPSE=1 is lab-only; refuse in bootstrap".into());
    }
    let roles = OnboardRoles {
        runtime: runtime.clone(),
        hashmerchant: hm.clone(),
        chain_id: std::env::var("ONBOARD_CHAIN").unwrap_or_else(|_| "morocco-1".into()),
    };
    if !roles.is_sound() {
        return Err("OnboardRoles::is_sound failed".into());
    }
    let bundle = dao_authz_grants(&roles);
    for m in bundle["msgs"].as_array().unwrap() {
        if m["granter"] != runtime || m["grantee"] != hm {
            return Err("msg is not runtime→hashmerchant".into());
        }
        if m["grant"]["authorization"]["msg"]
            .as_str()
            .unwrap_or("")
            .contains("MsgSend")
        {
            return Err("authz must not include bank send".into());
        }
    }
    let out = PathBuf::from(std::env::var("ONBOARD_AUTHZ_OUT").unwrap_or_else(|_| {
        format!(
            "{}/.terp/hashmerchant-authz",
            std::env::var("HOME").unwrap_or_default()
        )
    }));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let path = out.join("cosmos-msgs.json");
    std::fs::write(&path, serde_json::to_string_pretty(&bundle).unwrap() + "\n")
        .map_err(|e| e.to_string())?;
    println!("wrote {}", path.display());
    let broadcast = std::env::var("AUTHZ_BROADCAST").ok().as_deref() == Some("1");
    if broadcast {
        eprintln!("AUTHZ_BROADCAST=1 noted; this example does not sign or broadcast");
    }
    println!(
        "{}",
        json!({
            "runtime": runtime,
            "hashmerchant": hm,
            "fee_granter": hm,
            "authz_granter": runtime,
            "broadcast": broadcast,
            "next": "Daemon.commit_any MsgGrant GenericAuthorization feegrant grant/revoke. Signer = runtime key, never the customer feegrant.",
        })
    );
    Ok(())
}
