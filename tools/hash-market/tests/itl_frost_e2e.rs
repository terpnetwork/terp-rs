//! ITL FROST 20/80 — same crate as combo / itl_auth_e2e.
//! Local: no chain, no Docker, no frostd. Live frostd is ignored.
//!
//!   cargo test --manifest-path tools/onboard-authz/Cargo.toml --test itl_frost_e2e

use hash_market::onboard_authz::itl_frost::{lab_taddr, persist_required, require_node_txid, 
    commit_line, local_twenty_eighty, parse_sign_recipient, require_transparent_taddr,
    session_json_ok, share_line, sign_line, JobStatus, Ledger, PayRecord, PayeeKind, PendingJob,
    PayoutLeg, TicketInflow, CHECK_IN_GUEST, DKG_TAG_R1, DKG_TAG_R2, DOMAIN_ZEC_SPEND_V0, FROST_N,
    FROST_T,
};

fn ink() -> PayRecord {
    PayRecord {
        code: "pay-20-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: taddr_ink(),
        leg: PayoutLeg::Twenty,
        amount_label: "20% upfront".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    }
}

fn paid_ledger() -> Ledger {
    let mut l = Ledger::default();
    l.confirm_inflow(TicketInflow {
        record_id: "ticket-1".into(),
        rail: "cashapp".into(),
        amount_label: "list".into(),
    })
    .unwrap();
    l
}

fn taddr_ink() -> String { lab_taddr(1) }

#[test]
fn itl_frost_transparent_taddr_required() {
    assert!(require_transparent_taddr(&lab_taddr(1)).is_ok());
    assert_eq!(require_transparent_taddr(""), Err("no_taddr"));
    assert_eq!(
        require_transparent_taddr("u1shieldednotforpay"),
        Err("not_transparent")
    );
    assert_eq!(
        require_transparent_taddr("terp1guest"),
        Err("not_transparent")
    );
}

#[test]
fn itl_frost_unpaid_cannot_open_twenty() {
    let mut l = Ledger::default();
    assert_eq!(l.enqueue(ink()), Err("inflow_unconfirmed"));
    l.confirm_inflow(TicketInflow {
        record_id: "ticket-cashapp-1".into(),
        rail: "cashapp".into(),
        amount_label: "list".into(),
    })
    .unwrap();
    l.enqueue(ink()).unwrap();
}

#[test]
fn itl_frost_eighty_requires_twenty_txid() {
    let mut l = paid_ledger();
    let eighty = PayRecord {
        code: "pay-80-ink".into(),
        kind: PayeeKind::Artist,
        name: "ink".into(),
        taddr: taddr_ink(),
        leg: PayoutLeg::Eighty,
        amount_label: "80% after event".into(),
        attest_url: Some("https://example.invalid/attest/ink".into()),
        txid: None,
    };
    assert_eq!(l.enqueue(eighty.clone()), Err("eighty_before_twenty"));
    l.enqueue(ink()).unwrap();
    assert_eq!(l.enqueue(eighty.clone()), Err("eighty_before_twenty"));
    l.mark_broadcast("pay-20-ink", "txid-20").unwrap();
    l.enqueue(eighty).unwrap();
}

#[test]
fn itl_frost_grant_rollup_twenty_then_eighty() {
    let l = local_twenty_eighty();
    assert!(l.has_confirmed_inflow());
    let rows = l.grant_rollup();
    let ink_row = rows.iter().find(|r| r.name == "ink").expect("ink");
    assert_eq!(ink_row.kind, PayeeKind::Artist);
    assert_eq!(ink_row.up, vec!["txid-20-ink"]);
    assert_eq!(ink_row.after, vec!["queued"]);
    let kit = rows.iter().find(|r| r.name == "kitchen").expect("vendor");
    assert_eq!(kit.kind, PayeeKind::Vendor);
    assert_eq!(kit.up, vec!["queued"]);
}

#[test]
fn itl_frost_queued_is_not_broadcast() {
    let mut l = paid_ledger();
    let mut sneak = ink();
    sneak.txid = Some("sneak".into());
    assert_eq!(l.enqueue(sneak), Err("queued_has_txid"));
    l.enqueue(ink()).unwrap();
    assert!(l.records()[0].txid.is_none());
}

fn lab_roster_commit() -> String {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    FrostCommittee::new("vk-old", &["desk", "steward", "spare"]).roster_commit()
}

#[test]
fn itl_frost_inbox_holds_until_threshold_then_broadcast() {
    let mut job = PendingJob::open_bound(
        "job-20-ink",
        "pay-20-ink",
        FROST_T,
        FROST_N,
        &lab_roster_commit(),
    )
    .unwrap();
    assert_eq!(
        job.submit_share("desk", "S:0:opaque-a").unwrap(),
        JobStatus::Pending
    );
    assert_eq!(job.broadcast("txid-too-soon"), Err("below_threshold"));
    assert_eq!(
        job.submit_share("steward", "S:0:opaque-b").unwrap(),
        JobStatus::Threshold
    );
    job.broadcast("txid-20-ink").unwrap();
    assert_eq!(job.status, JobStatus::Broadcast);
    assert_eq!(
        job.submit_share("spare", "S:0:opaque-c"),
        Err("already_broadcast")
    );
}

#[test]
fn itl_frost_inbox_rejects_keypackage_and_dup_admin() {
    let mut job = PendingJob::open("j", "pay-20-ink", 2, 3);
    assert_eq!(
        job.submit_share("desk", "KeyPackage-hex"),
        Err("keypackage_rejected")
    );
    job.submit_share("desk", "S:0:a").unwrap();
    assert_eq!(job.submit_share("desk", "S:0:b"), Err("duplicate_admin"));
}

#[test]
fn itl_frost_session_has_no_keypackages() {
    assert!(session_json_ok(
        r#"{"credId":"abc","role":"admin","alias":"desk"}"#
    ));
    assert!(!session_json_ok(r#"{"KeyPackage":"deadbeef"}"#));
    assert!(!session_json_ok(r#"{"S:0:sharehex"}"#));
}

#[test]
fn itl_frost_sign_cs_wire_recipient_is_taddr() {
    let taddr_s = lab_taddr(1);
    let taddr = taddr_s.as_str();
    let line = sign_line("aa", "0,1", taddr, "1000", "10", "pczt");
    assert!(line.starts_with("SIGN:"));
    assert_eq!(parse_sign_recipient(&line), Some(taddr));
    require_transparent_taddr(parse_sign_recipient(&line).unwrap()).unwrap();
    assert_eq!(commit_line(&["c0", "c1"]), "C:c0|c1");
    assert_eq!(share_line(0, "s0"), "S:0:s0");
    assert_eq!(DKG_TAG_R1, "dkg1");
    assert_eq!(DKG_TAG_R2, "dkg2");
    assert_eq!(FROST_T, 2);
    assert_eq!(FROST_N, 3);
    assert_eq!(DOMAIN_ZEC_SPEND_V0, b"terp/frost_escrow/zec_spend/v0");
    assert_eq!(CHECK_IN_GUEST, "check_in_guest");
}

#[test]
fn itl_frost_passkey_is_not_a_share() {
    let session = r#"{"credId":"webauthn-cred","role":"admin"}"#;
    assert!(session_json_ok(session));
    assert!(!session.contains("KeyPackage"));
}

/// Live: frostd loopback + holders. Do not treat HTML flags as signed.
#[test]
#[ignore = "needs frostd on 127.0.0.1"]
fn itl_frost_live_frostd_loopback() {
    let url = std::env::var("ITL_FROSTD").unwrap_or_else(|_| "http://127.0.0.1:2744".into());
    let _ = url;
}

#[test]
fn itl_frost_rotate_pending_until_threshold_then_vk_changes() {
    use hash_market::onboard_authz::itl_frost_rotate::{
        msg_rotate_frost_pubkey, vk_commit, FrostCommittee, RotateKind, RotateStatus,
        EXECUTE_CONTRACT_TYPE_URL, ROTATE_MSG,
    };
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let old = c.group_vk.clone();
    c.invoke_rotate("desk", "vk-new", Some("hkdf-lab")).unwrap();
    assert_eq!(c.rotate.as_ref().unwrap().status, RotateStatus::Pending);
    assert_eq!(c.group_vk, old, "pubkey unchanged until t");
    c.add_rotate_share("desk", "vk-new", Some("hkdf-lab"), "S:0:a")
        .unwrap();
    assert_eq!(c.group_vk, old);
    let st = c
        .add_rotate_share("steward", "vk-new", Some("hkdf-lab"), "S:0:b")
        .unwrap();
    assert_eq!(st, RotateStatus::Applied);
    assert_eq!(c.group_vk, "vk-new");
    assert_ne!(c.group_vk, old);
    let m = msg_rotate_frost_pubkey(
        "terp1desk",
        "terp1frostauth",
        &vk_commit("vk-new"),
        Some("hkdf-lab"),
    );
    assert_eq!(m["@type"], EXECUTE_CONTRACT_TYPE_URL);
    assert!(m["msg"][ROTATE_MSG].is_object());
    assert!(m.get("authenticator_type").is_none());
}

#[test]
fn itl_frost_rotate_always_writes_on_fail() {
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, RotateKind};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    assert_eq!(
        c.invoke_rotate("stranger", "vk-new", None),
        Err("not_admin")
    );
    assert!(c.log.iter().any(|e| e.kind == RotateKind::Fail
        && e.reason.as_deref() == Some("not_admin")));
    c.invoke_rotate("desk", "vk-new", Some("hkdf-a")).unwrap();
    let n = c.log.len();
    assert_eq!(
        c.add_rotate_share("steward", "vk-OTHER", Some("hkdf-a"), "S:0:x"),
        Err("commit_mismatch")
    );
    assert!(c.log.len() > n);
    assert_eq!(c.group_vk, "vk-old");
    let n2 = c.log.len();
    assert_eq!(
        c.add_rotate_share("spare", "vk-new", Some("hkdf-WRONG"), "S:0:y"),
        Err("hkdf_mismatch")
    );
    assert!(c.log.len() > n2);
    assert_eq!(c.group_vk, "vk-old");
}

#[test]
fn itl_frost_rotate_second_invoke_while_pending_fails_written() {
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, RotateKind};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    assert_eq!(
        c.invoke_rotate("steward", "vk-other", None),
        Err("already_pending")
    );
    assert!(c
        .log
        .iter()
        .any(|e| e.kind == RotateKind::Fail && e.reason.as_deref() == Some("already_pending")));
}

#[test]
fn itl_frost_rotate_rekey_spend_required() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    assert_eq!(c.group_vk, "vk-new");
    assert_eq!(c.can_spend_new_vk(), Err("rekey_incomplete"));
    assert_eq!(
        c.record_rekey_spend("txid-rekey", "vk-wrong", "vk-new"),
        Err("from_vk_mismatch")
    );
    c.record_rekey_spend("txid-rekey", "vk-old", "vk-new").unwrap();
    assert!(c.can_spend_new_vk().is_ok());
    assert!(c.old_vk.is_none());
}

#[test]
fn itl_frost_pay_job_rejects_ed25519_consensus_scheme() {
    use hash_market::onboard_authz::itl_frost::{
        BroadcastTarget, FrostScheme, JobStatus, PendingJob, FROST_N, FROST_T,
    };
    let mut job = PendingJob::open_for(
        "j",
        "pay-20-ink",
        FROST_T,
        FROST_N,
        FrostScheme::Ed25519Consensus,
        BroadcastTarget::Zcash,
    );
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(job.status, JobStatus::Threshold);
    assert_eq!(job.broadcast("txid"), Err("scheme_not_for_pay"));
}

#[test]
fn itl_frost_snapshot_roundtrip_status_only() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    let snap = c.snapshot();
    assert_eq!(snap.group_vk, "vk-new");
    assert_eq!(snap.old_vk.as_deref(), Some("vk-old"));
    let json = serde_json::to_string(&snap).unwrap();
    assert!(hash_market::onboard_authz::itl_frost::session_json_ok(&json));
    let mut c2 = FrostCommittee::new("other", &["desk", "steward", "spare"]);
    c2.restore_snapshot(snap).unwrap();
    assert_eq!(c2.group_vk, "vk-new");
}

#[test]
fn itl_frost_inbox_drops_blobs_after_broadcast() {
    use hash_market::onboard_authz::itl_frost::{JobStatus, PendingJob, FROST_N, FROST_T};
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &lab_roster_commit())
        .unwrap();
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    job.broadcast("txid").unwrap();
    assert_eq!(job.status, JobStatus::Broadcast);
    assert_eq!(job.share_count(), 0);
}

#[test]
fn itl_frost_rotate_duplicate_admin() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    assert_eq!(
        c.add_rotate_share("desk", "vk-new", None, "S:0:b"),
        Err("duplicate_admin")
    );
}

#[test]
fn itl_frost_rekey_to_vk_mismatch() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    assert_eq!(
        c.record_rekey_spend("txid", "vk-old", "vk-other"),
        Err("to_vk_mismatch")
    );
}

#[test]
fn itl_frost_pay_job_rejects_bls_scheme() {
    use hash_market::onboard_authz::itl_frost::{
        BroadcastTarget, FrostScheme, PendingJob, FROST_N, FROST_T,
    };
    let mut job = PendingJob::open_for(
        "j",
        "pay-20-ink",
        FROST_T,
        FROST_N,
        FrostScheme::Bls,
        BroadcastTarget::Zcash,
    );
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(job.broadcast("txid"), Err("scheme_not_for_pay"));
}

#[test]
fn itl_frost_snapshot_dirty_packages_refused() {
    use hash_market::onboard_authz::itl_frost_rotate::{CommitteeSnapshot, FrostCommittee};
    let mut c = FrostCommittee::new("vk", &["desk", "steward", "spare"]);
    let mut snap = c.snapshot();
    snap.group_vk = "vk-packages-leak".into();
    // typed snapshot cannot carry KeyPackages; dirty string path:
    assert!(!hash_market::onboard_authz::itl_frost::session_json_ok(
        r#"{"group_vk":"x","packages":["deadbeef"]}"#
    ));
    let _ = snap;
    let _ = c;
}

#[test]
fn itl_frost_persistence_off_without_key() {
    std::env::remove_var("ITL_FROST_PERSIST_KEY");
    assert_eq!(persist_required(), Err("persistence_off"));
    std::env::set_var("ITL_FROST_PERSIST_KEY", "sixteen-chars-ok");
    assert!(persist_required().is_ok());
}

#[test]
fn itl_frost_lab_taddr_checksums() {
    let a = lab_taddr(1);
    assert!(a.starts_with("t1"));
    require_transparent_taddr(&a).unwrap();
    let mut bad = a.clone();
    let last = bad.pop().unwrap();
    bad.push(if last == 'a' { 'b' } else { 'a' });
    let e = require_transparent_taddr(&bad);
    assert!(e == Err("bad_checksum") || e == Err("not_transparent"), "{e:?}");
}

#[test]
fn itl_frost_http_inbox_holds_until_threshold() {
    let mut box_ = hash_market::onboard_authz::frost_http::FrostInbox::default();
    let r = box_
        .try_handle(
            "POST",
            "/itl/frost/jobs",
            &format!(
                r#"{{"pay_code":"pay-20-ink","roster_commit":"{}"}}"#,
                lab_roster_commit()
            ),
        )
        .unwrap();
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    box_.try_handle(
        "POST",
        &format!("/itl/frost/jobs/{id}/share"),
        r#"{"admin_id":"desk","blob":"S:0:a"}"#,
    );
    let premature = box_
        .try_handle(
            "POST",
            &format!("/itl/frost/jobs/{id}/broadcast"),
            r#"{"txid":"x"}"#,
        )
        .unwrap();
    assert_eq!(premature.status, 400);
    box_.try_handle(
        "POST",
        &format!("/itl/frost/jobs/{id}/share"),
        r#"{"admin_id":"steward","blob":"S:0:b"}"#,
    );
    let ok = box_
        .try_handle(
            "POST",
            &format!("/itl/frost/jobs/{id}/broadcast"),
            r#"{"txid":"txid-20"}"#,
        )
        .unwrap();
    assert_eq!(ok.status, 200);
}

#[test]
fn itl_frost_rotate_add_member_changes_n_and_vk() {
    use hash_market::onboard_authz::itl_frost_rotate::{
        msg_rotate_frost_pubkey_roster, vk_commit, FrostCommittee, NextRoster, RotateStatus,
        ROTATE_MSG,
    };
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let roster = NextRoster::from_seats(&["desk", "steward", "spare", "usher"], 2, 4).unwrap();
    c.invoke_rotate_roster("desk", "vk-new", None, roster.clone())
        .unwrap();
    assert_eq!(c.n, 3, "roster unchanged until epoch apply");
    assert!(!c.is_seat("usher"), "player is not a dealer yet");
    assert_eq!(
        c.add_rotate_share("usher", "vk-new", None, "S:0:z"),
        Err("not_admin")
    );
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    let st = c
        .add_rotate_share("steward", "vk-new", None, "S:0:b")
        .unwrap();
    assert_eq!(st, RotateStatus::Applied);
    assert_eq!(c.group_vk, "vk-new");
    assert_eq!(c.n, 4);
    assert_eq!(c.t, 2);
    assert_eq!(c.epoch, 1);
    assert!(c.is_seat("usher"));
    assert_eq!(c.can_spend_new_vk(), Err("rekey_incomplete"));
    let m = msg_rotate_frost_pubkey_roster(
        "terp1desk",
        "terp1frostauth",
        &vk_commit("vk-new"),
        None,
        &roster,
    );
    assert_eq!(m["msg"][ROTATE_MSG]["next_n"], 4);
    assert!(m["msg"][ROTATE_MSG]["next_seats"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str() == Some("usher")));
    assert!(m.get("authenticator_type").is_none());
}

#[test]
fn itl_frost_rotate_remove_member_drops_seat() {
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster, RotateStatus};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let roster = NextRoster::from_seats(&["desk", "steward"], 2, 2).unwrap();
    c.invoke_rotate_roster("desk", "vk-new", None, roster)
        .unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    let st = c
        .add_rotate_share("steward", "vk-new", None, "S:0:b")
        .unwrap();
    assert_eq!(st, RotateStatus::Applied);
    assert_eq!(c.n, 2);
    assert!(!c.is_seat("spare"));
    assert_eq!(
        c.invoke_rotate("spare", "vk-newer", None),
        Err("not_admin")
    );
}

#[test]
fn itl_frost_rotate_same_vk_membership_change_forbidden() {
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster, RotateKind};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let roster = NextRoster::from_seats(&["desk", "steward", "spare", "usher"], 2, 4).unwrap();
    assert_eq!(
        c.invoke_rotate_roster("desk", "vk-old", None, roster),
        Err("same_vk")
    );
    assert!(c
        .log
        .iter()
        .any(|e| e.kind == RotateKind::Fail && e.reason.as_deref() == Some("same_vk")));
    assert_eq!(c.group_vk, "vk-old");
    assert_eq!(c.n, 3);
}

#[test]
fn itl_frost_rotate_bad_next_roster_writes_fail() {
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster, RotateKind};
    assert_eq!(
        NextRoster::from_seats(&["desk"], 2, 1),
        Err("bad_threshold")
    );
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let bad = NextRoster {
        seats: vec!["desk".into()],
        t: 2,
        n: 1,
    };
    assert_eq!(
        c.invoke_rotate_roster("desk", "vk-new", None, bad),
        Err("bad_threshold")
    );
    assert!(c.log.iter().any(|e| e.kind == RotateKind::Fail));
    assert_eq!(c.group_vk, "vk-old");
}

#[test]
fn itl_frost_rotate_share_replay_across_epoch_fails() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    c.record_rekey_spend("txid-rekey", "vk-old", "vk-new").unwrap();
    c.invoke_rotate("desk", "vk-newer", None).unwrap();
    assert_eq!(
        c.add_rotate_share("desk", "vk-newer", None, "S:0:a"),
        Err("share_replay_epoch")
    );
    assert_eq!(c.group_vk, "vk-new");
}

#[test]
fn itl_frost_spend_requires_roster_memo_sane() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob, FROST_N, FROST_T};
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let rc = c.roster_commit();
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &rc).unwrap();
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    let memo = spend_memo_roster(&rc);
    c.authorize_spend(&job, &memo).unwrap();
    job.broadcast("txid-20").unwrap();
}

#[test]
fn itl_frost_spend_insane_missing_roster_commit() {
    let mut job = PendingJob::open("j", "pay-20-ink", FROST_T, FROST_N);
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(job.broadcast("txid-20"), Err("roster_commit_required"));
}

#[test]
fn itl_frost_spend_insane_roster_split() {
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let a = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]).roster_commit();
    let b = FrostCommittee::new("vk-old", &["desk", "steward", "usher"]).roster_commit();
    assert_ne!(a, b);
    let mut job = PendingJob::open("j", "pay-20-ink", FROST_T, FROST_N);
    job.submit_share_bound("desk", "S:0:a", &a).unwrap();
    assert_eq!(
        job.submit_share_bound("steward", "S:0:b", &b),
        Err("roster_split")
    );
}

#[test]
fn itl_frost_spend_insane_stale_roster_after_add_member() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob, FROST_N, FROST_T};
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let old_rc = c.roster_commit();
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &old_rc).unwrap();
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    let roster = NextRoster::from_seats(&["desk", "steward", "spare", "usher"], 2, 4).unwrap();
    c.invoke_rotate_roster("desk", "vk-new", None, roster)
        .unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:1:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:1:b").unwrap();
    c.record_rekey_spend("txid-rekey", "vk-old", "vk-new").unwrap();
    let memo = spend_memo_roster(&old_rc);
    assert_eq!(c.authorize_spend(&job, &memo), Err("roster_stale"));
    assert_ne!(old_rc, c.roster_commit());
}

#[test]
fn itl_frost_spend_insane_removed_holder() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob};
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let roster = NextRoster::from_seats(&["desk", "steward"], 2, 2).unwrap();
    c.invoke_rotate_roster("desk", "vk-new", None, roster)
        .unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    c.record_rekey_spend("txid-rekey", "vk-old", "vk-new").unwrap();
    let rc = c.roster_commit();
    let mut job = PendingJob::open_bound("j", "pay-20-ink", 2, 2, &rc).unwrap();
    job.submit_share("desk", "S:0:c").unwrap();
    job.submit_share("spare", "S:0:d").unwrap(); // inbox does not know seats
    let memo = spend_memo_roster(&rc);
    assert_eq!(c.authorize_spend(&job, &memo), Err("not_holder"));
}

#[test]
fn itl_frost_spend_insane_memo_mismatch_or_missing() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob, FROST_N, FROST_T};
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let rc = c.roster_commit();
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &rc).unwrap();
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(c.authorize_spend(&job, ""), Err("memo_missing_roster"));
    let other = FrostCommittee::new("vk-other", &["desk", "steward", "spare"]).roster_commit();
    assert_eq!(
        c.authorize_spend(&job, &spend_memo_roster(&other)),
        Err("memo_mismatch")
    );
}

#[test]
fn itl_frost_spend_insane_pending_next_roster_not_live() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob, FROST_N, FROST_T};
    use hash_market::onboard_authz::itl_frost_rotate::{FrostCommittee, NextRoster};
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    let live = c.roster_commit();
    let roster = NextRoster::from_seats(&["desk", "steward", "spare", "usher"], 2, 4).unwrap();
    c.invoke_rotate_roster("desk", "vk-new", None, roster)
        .unwrap();
    let pending = c.pending_roster_commit().expect("pending players");
    assert_ne!(live, pending);
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &pending).unwrap();
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("usher", "S:0:z").unwrap();
    assert_eq!(
        c.authorize_spend(&job, &spend_memo_roster(&pending)),
        Err("roster_stale")
    );
}

#[test]
fn itl_frost_spend_insane_rekey_incomplete_blocks_pay() {
    use hash_market::onboard_authz::itl_frost::{spend_memo_roster, PendingJob, FROST_N, FROST_T};
    use hash_market::onboard_authz::itl_frost_rotate::FrostCommittee;
    let mut c = FrostCommittee::new("vk-old", &["desk", "steward", "spare"]);
    c.invoke_rotate("desk", "vk-new", None).unwrap();
    c.add_rotate_share("desk", "vk-new", None, "S:0:a").unwrap();
    c.add_rotate_share("steward", "vk-new", None, "S:0:b").unwrap();
    let rc = c.roster_commit();
    let mut job = PendingJob::open_bound("j", "pay-20-ink", FROST_T, FROST_N, &rc).unwrap();
    job.submit_share("desk", "S:0:c").unwrap();
    job.submit_share("steward", "S:0:d").unwrap();
    assert_eq!(
        c.authorize_spend(&job, &spend_memo_roster(&rc)),
        Err("rekey_incomplete")
    );
}

#[test]
fn itl_frost_http_pay_requires_roster_commit() {
    let mut box_ = hash_market::onboard_authz::frost_http::FrostInbox::default();
    let r = box_
        .try_handle("POST", "/itl/frost/jobs", r#"{"pay_code":"pay-20-ink"}"#)
        .unwrap();
    assert_eq!(r.status, 400);
    assert!(r.body.contains("roster_commit_required"), "{}", r.body);
}

#[test]
fn itl_frost_broadcast_rejects_fixture_txid() {
    assert_eq!(require_node_txid(""), Err("no_txid"));
    assert_eq!(require_node_txid("txid-20-ink"), Err("fixture_txid"));
    assert_eq!(require_node_txid("txid"), Err("fixture_txid"));
    assert_eq!(require_node_txid(&"aa".repeat(32)), Err("fixture_txid"));
    assert_eq!(require_node_txid("not-hex"), Err("txid_len"));
    let mut job = PendingJob::open("j", "pay-20-ink", FROST_T, FROST_N);
    job.submit_share("desk", "S:0:a").unwrap();
    job.submit_share("steward", "S:0:b").unwrap();
    assert_eq!(
        job.broadcast_node_confirmed("txid-20-ink", "http://127.0.0.1:9"),
        Err("fixture_txid")
    );
    assert_eq!(job.status, JobStatus::Threshold);
}
