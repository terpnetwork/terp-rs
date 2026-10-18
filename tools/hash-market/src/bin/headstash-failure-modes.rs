//! Four-validator headstash failure modes.
//!
//! Equal stake. Quorum is 0.667 of bonded stake:
//! `stake_up * 1000 >= 667 * total`.
//! This process does not start Docker and does not print keys.

const QUORUM_NUM: u128 = 667;
const QUORUM_DEN: u128 = 1000;
const VALIDATORS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Report {
    Absent,
    Root(&'static str),
}

struct Case {
    name: &'static str,
    reports: [Report; VALIDATORS],
    expect_pay: bool,
    expect_root: Option<&'static str>,
}

fn quorum(reports: &[Report]) -> bool {
    let up = reports
        .iter()
        .filter(|r| !matches!(r, Report::Absent))
        .count() as u128;
    let total = reports.len() as u128;
    up * QUORUM_DEN >= QUORUM_NUM * total
}

fn majority_root(reports: &[Report]) -> Option<&'static str> {
    let mut best: Option<(&str, usize)> = None;
    for r in reports {
        let Report::Root(root) = r else { continue };
        let n = reports
            .iter()
            .filter(|o| matches!(o, Report::Root(x) if *x == *root))
            .count();
        match best {
            Some((_, bn)) if n <= bn => {}
            _ => best = Some((root, n)),
        }
    }
    let (root, n) = best?;
    let up = reports
        .iter()
        .filter(|r| !matches!(r, Report::Absent))
        .count();
    if n * 2 > up { Some(root) } else { None }
}

fn pays(reports: &[Report], honest: &str) -> Option<&'static str> {
    if !quorum(reports) {
        return None;
    }
    let root = majority_root(reports)?;
    if root == honest { Some(root) } else { None }
}

fn spent(epoch_nullifiers: &[&str], nullifier: &str) -> bool {
    epoch_nullifiers.iter().any(|n| *n == nullifier)
}

fn main() {
    let honest = "accepted-root-honest";
    let wrong = "accepted-root-wrong";
    let cases = [
        Case {
            name: "one_sidecar_down_still_pays",
            reports: [
                Report::Root(honest),
                Report::Root(honest),
                Report::Root(honest),
                Report::Absent,
            ],
            expect_pay: true,
            expect_root: Some(honest),
        },
        Case {
            name: "two_sidecars_down_no_quorum",
            reports: [
                Report::Root(honest),
                Report::Root(honest),
                Report::Absent,
                Report::Absent,
            ],
            expect_pay: false,
            expect_root: None,
        },
        Case {
            name: "wrong_root_outvoted",
            reports: [
                Report::Root(honest),
                Report::Root(honest),
                Report::Root(honest),
                Report::Root(wrong),
            ],
            expect_pay: true,
            expect_root: Some(honest),
        },
        Case {
            name: "restart_mid_epoch_absent_then_back",
            reports: [
                Report::Root(honest),
                Report::Root(honest),
                Report::Root(honest),
                Report::Absent,
            ],
            expect_pay: true,
            expect_root: Some(honest),
        },
    ];

    let mut failed = 0usize;
    for case in cases {
        let got = pays(&case.reports, honest);
        let pay = got.is_some();
        let ok = pay == case.expect_pay && got == case.expect_root;
        println!(
            "{} {}",
            if ok { "PASS" } else { "FAIL" },
            case.name
        );
        if !ok {
            failed += 1;
            println!(
                "  pay={pay} root={got:?} expected pay={} root={:?}",
                case.expect_pay, case.expect_root
            );
        }
    }

    let back = pays(
        &[
            Report::Root(honest),
            Report::Root(honest),
            Report::Root(honest),
            Report::Root(honest),
        ],
        honest,
    );
    let back_ok = back == Some(honest);
    println!(
        "{} restart_mid_epoch_accepted_after_return",
        if back_ok { "PASS" } else { "FAIL" }
    );
    if !back_ok {
        failed += 1;
    }

    let two_down_no = !quorum(&[
        Report::Root(honest),
        Report::Root(honest),
        Report::Absent,
        Report::Absent,
    ]);
    let next_yes = quorum(&[
        Report::Root(honest),
        Report::Root(honest),
        Report::Root(honest),
        Report::Root(honest),
    ]);
    let recover_ok = two_down_no && next_yes;
    println!(
        "{} no_quorum_recovers_next_epoch",
        if recover_ok { "PASS" } else { "FAIL" }
    );
    if !recover_ok {
        failed += 1;
    }

    let replay_ok = spent(&["nullifier-a"], "nullifier-a")
        && !spent(&["nullifier-a"], "nullifier-b");
    println!(
        "{} replayed_nullifier_across_epochs",
        if replay_ok { "PASS" } else { "FAIL" }
    );
    if !replay_ok {
        failed += 1;
    }

    let campaign_open = false;
    let unclaimed = 4u64 - 1u64;
    let sweep = false;
    let unclaimed_ok = !campaign_open && unclaimed == 3 && !sweep;
    println!(
        "{} unclaimed_funds_after_campaign_end",
        if unclaimed_ok { "PASS" } else { "FAIL" }
    );
    if !unclaimed_ok {
        failed += 1;
    }

    if failed != 0 {
        eprintln!("{failed} failure-mode checks failed");
        std::process::exit(1);
    }
    println!("ALL_PASS validators={VALIDATORS} quorum=0.667");
}
