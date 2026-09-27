//! Curate DAO / wallet grant JSON (existing operator bin).
//!
//!   cargo run -p terp-onboard-authz --bin prepare-authz -- \
//!     --mode dao-proposal --preset register --chain-id morocco-1 \
//!     --runtime terp1RUNTIME --hashmerchant terp1HM --out ~/.terp/hashmerchant-authz
//!
//!   --mode wallet-proxy --from-addr terp1RUNTIME --hashmerchant terp1HM
//!   wallet-as-runtime is lab-only (runtime == HM); production HashMerchant forbids it.
//!   --state path/to/terp-state.json

use std::env;
use std::fs;
use std::path::PathBuf;
use terp_onboard_authz::curate::{curate, load_infra, preset, CurateMode, PRESETS};
use terp_onboard_authz::{
    authz_exec_feegrant, dao_authz_grants, registration_feegrant, terpd_hint, FeegrantRequest,
    OnboardRoles,
};

fn arg(flag: &str) -> Option<String> {
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn flag(name: &str) -> bool {
    env::args().any(|a| a == name)
}

fn main() {
    if flag("--list-presets") {
        for p in PRESETS {
            println!("{}  {:?}", p.id, p.purpose);
        }
        return;
    }

    let mode_s = arg("--mode").unwrap_or_else(|| "legacy".into());
    let out = PathBuf::from(arg("--out").unwrap_or_else(|| ".".into()));
    fs::create_dir_all(&out).expect("mkdir out");

    let from_addr = arg("--from-addr").unwrap_or_default();
    if !from_addr.is_empty() {
        env::set_var("ONBOARD_FROM_ADDR", &from_addr);
    }

    let roles = OnboardRoles {
        runtime: arg("--runtime")
            .or_else(|| arg("--dao"))
            .unwrap_or_else(|| {
                if from_addr.is_empty() {
                    "DAO_ADDR".into()
                } else {
                    from_addr.clone()
                }
            }),
        hashmerchant: arg("--hashmerchant")
            .or_else(|| arg("--hm"))
            .unwrap_or_else(|| "HASHMERCHANT_ADDR".into()),
        chain_id: arg("--chain-id").unwrap_or_else(|| "120u-1".into()),
    };

    if mode_s == "legacy" && arg("--preset").is_none() {
        let grants = dao_authz_grants(&roles);
        let sample = registration_feegrant(
            &roles,
            &FeegrantRequest {
                grantee: arg("--sample-user").unwrap_or_else(|| "terp1USER".into()),
                spend_limit: arg("--spend-limit").unwrap_or_else(|| "25000".into()),
                expiration: None,
            },
        );
        let exec = authz_exec_feegrant(&roles, sample.clone());
        write_json(out.join("dao-authz-grants.json"), &grants);
        write_json(out.join("feegrant-register.sample.json"), &sample);
        write_json(out.join("authz-exec-feegrant.sample.json"), &exec);
        fs::write(out.join("TERPD.txt"), terpd_hint(&roles)).ok();
        println!("wrote {} (legacy register-only)", out.display());
        return;
    }

    let mode = match mode_s.as_str() {
        "dao-proposal" | "dao" | "legacy" => CurateMode::DaoProposal,
        "wallet-proxy" | "proxy" => CurateMode::WalletProxy,
        "wallet-as-runtime" | "self" | "wallet" => CurateMode::WalletAsRuntime,
        other => {
            eprintln!("unknown --mode {other}");
            std::process::exit(2);
        }
    };
    let pre = preset(&arg("--preset").unwrap_or_else(|| "register".into())).unwrap_or_else(|| {
        eprintln!("unknown --preset; --list-presets");
        std::process::exit(2);
    });
    let infra = load_infra(arg("--state").map(PathBuf::from).as_deref());
    let bundle = curate(
        mode,
        &roles,
        pre,
        &infra,
        arg("--proposal-module").as_deref(),
        &arg("--sample-user").unwrap_or_else(|| "terp1USER".into()),
    );
    write_json(out.join("cosmos-msgs.json"), &bundle.cosmos_msgs);
    if let Some(p) = &bundle.dao_proposal {
        write_json(out.join("dao-proposal.json"), p);
        if let Some(ex) = p.get("execute") {
            write_json(out.join("dao-proposal.execute.json"), ex);
        }
    }
    write_json(
        out.join("preset.json"),
        &serde_json::to_value(&bundle.preset).unwrap(),
    );
    fs::write(out.join("TERPD.sh"), format!("#!/bin/sh\nset -eu\n{}\n", bundle.terpd)).ok();
    println!("wrote {}", out.display());
    println!(
        "  mode={} preset={} chain={} codes={:?}",
        bundle.mode, bundle.preset.id, roles.chain_id, bundle.preset.code_ids
    );
}

fn write_json(path: PathBuf, v: &serde_json::Value) {
    fs::write(&path, serde_json::to_string_pretty(v).unwrap() + "\n").expect("write");
    println!("  {}", path.display());
}
