//! Existing in-process test bin: keeper-sim + grant curation for every preset/mode.
//!
//!   cargo run -p terp-onboard-authz --bin local-workflow
//!   ONBOARD_PRESET=norick ONBOARD_MODE=wallet-as-runtime cargo run -p terp-onboard-authz --bin local-workflow

use terp_onboard_authz::curate::{curate, load_infra, preset, CurateMode, PRESETS};
use terp_onboard_authz::workflow::{run_happy_path, SimKeeper};
use terp_onboard_authz::{dao_authz_grants, fee_denom, OnboardRoles, ALLOWED_MSG_TYPES};

fn main() {
    let chain = std::env::var("ONBOARD_CHAIN").unwrap_or_else(|_| "120u-1".into());
    let roles = OnboardRoles {
        runtime: std::env::var("ONBOARD_RUNTIME")
            .or_else(|_| std::env::var("ONBOARD_DAO"))
            .unwrap_or_else(|_| "terp1dao".into()),
        hashmerchant: std::env::var("ONBOARD_HASHMERCHANT")
            .unwrap_or_else(|_| "terp1runtime".into()),
        chain_id: chain.clone(),
    };

    let mut k = SimKeeper::default();
    run_happy_path(&mut k, &roles, "terp1user").expect("happy path");
    let grants = dao_authz_grants(&roles);
    println!("PASS local-workflow keeper");
    println!("  chain={} denom={}", roles.chain_id, fee_denom());
    println!(
        "  dao_authz_msgs={}",
        grants["msgs"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    for t in ALLOWED_MSG_TYPES {
        println!("    {t}");
    }

    let infra = load_infra(None);
    let only = std::env::var("ONBOARD_PRESET").ok();
    let only_mode = std::env::var("ONBOARD_MODE").ok();
    let modes = [
        ("dao-proposal", CurateMode::DaoProposal),
        ("wallet-proxy", CurateMode::WalletProxy),
        ("wallet-as-runtime", CurateMode::WalletAsRuntime),
    ];

    println!("PASS local-workflow curate");
    for p in PRESETS {
        if only.as_deref().is_some_and(|id| id != p.id) {
            continue;
        }
        for (name, mode) in modes {
            if only_mode.as_deref().is_some_and(|m| m != name) {
                continue;
            }
            let b = curate(mode, &roles, p, &infra, None, "terp1user");
            assert!(
                !b.preset.allowance_msg_types.is_empty(),
                "preset {} empty allowance",
                p.id
            );
            println!(
                "  {}/{} codes={:?} missing={:?}",
                p.id, name, b.preset.code_ids, b.preset.missing_code_ids
            );
        }
    }

    let norick = preset("norick").unwrap().resolve(&chain, &infra);
    if chain == "120u-1" {
        assert_eq!(
            norick.code_ids.get("headstash:cw_norick"),
            Some(&72),
            "120u-1 norick code id"
        );
    }
    println!("PASS local-workflow presets");

    let acc = terp_onboard_authz::itl_auth::local_anyof_rotate();
    assert_eq!(acc.devices.len(), 2);
    println!(
        "PASS local-workflow itl-auth anyOf rotate devices={}",
        acc.devices
            .iter()
            .map(|d| d.label.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );

    let frost = terp_onboard_authz::itl_frost::local_twenty_eighty();
    let rows = frost.grant_rollup();
    assert!(rows.iter().any(|r| r.name == "ink" && r.up == ["txid-20-ink"]));
    println!("PASS local-workflow itl-frost 20/80 t-addr rollup rows={}", rows.len());

    let mut cash = terp_onboard_authz::itl_cashapp::Engine::default();
    let o = cash.create_order("cred-guest");
    let html = terp_onboard_authz::itl_cashapp::synthetic_received_html(
        &o.pay_ref,
        terp_onboard_authz::itl_cashapp::LIST_CENTS,
        "D-HARNESS01",
    );
    cash.ingest_text(&html, "Payment received")
        .expect("cashapp ingest");
    assert_eq!(cash.paid_count(), 1);
    println!("PASS local-workflow itl-cashapp payRef+nullifier paid=1");

    let mut ing = terp_onboard_authz::itl_ingress::IngressEngine::default();
    let o2 = ing.create_cash_order("cred-guest");
    let eml = terp_onboard_authz::itl_cashapp::synthetic_received_eml(
        &o2.pay_ref,
        terp_onboard_authz::itl_cashapp::LIST_CENTS,
        "D-INGRESS01",
    );
    let issued = ing.ingest_cashapp_eml(&eml).expect("ingress cashapp");
    assert_eq!(issued.paid_n, 1);
    assert!(terp_onboard_authz::itl_ingress::public_json_is_clean(
        &ing.public_sale_json()
    ));
    println!(
        "PASS local-workflow itl-ingress poseidon guest_root paid={}",
        issued.paid_n
    );

    let mut door = terp_onboard_authz::itl_consume::ConsumeEngine::default();
    door.add_usher("terp1usher");
    assert!(door.is_usher("terp1usher"));
    assert!(door.guest_list_members().is_none());
    assert!(!terp_onboard_authz::itl_consume::consume_success_allowed(
        "MockBech32",
        None
    ));
    assert_eq!(
        terp_onboard_authz::itl_consume::msg_store_code_with_circuit("terp1", b"w", b"v")["@type"],
        terp_onboard_authz::itl_consume::MSG_STORE_CODE_WITH_CIRCUIT
    );
    println!("PASS local-workflow itl-consume usher-only nullifiers no ListMembers");
}
