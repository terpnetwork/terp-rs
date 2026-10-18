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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Escrow {
    owner: &'static str,
    campaign_set: bool,
    cutoff_height: u64,
    sweep_recipient: &'static str,
    staged: bool,
    paid: bool,
    balance: u64,
}

/// Same gates as cw-headstash `sweep_unclaimed`. The message carries no address.
fn sweep_unclaimed(escrow: &mut Escrow, caller: &str, height: u64) -> Result<u64, &'static str> {
    if caller != escrow.owner {
        return Err("Caller is not the contract's current owner");
    }
    if !escrow.campaign_set {
        return Err("epoch campaign is not set");
    }
    if escrow.sweep_recipient.is_empty() {
        return Err("sweep recipient is not set");
    }
    if height <= escrow.cutoff_height {
        return Err("campaign has not ended");
    }
    if escrow.staged && !escrow.paid {
        return Err("staged batch is unpaid");
    }
    if escrow.balance == 0 {
        return Err("nothing to sweep");
    }
    let moved = escrow.balance;
    escrow.balance = 0;
    Ok(moved)
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

    let mut escrow = Escrow {
        owner: "dao",
        campaign_set: true,
        cutoff_height: 80,
        sweep_recipient: "terp1treasury",
        staged: true,
        paid: true,
        balance: 999_985,
    };
    let moved = sweep_unclaimed(&mut escrow, "dao", 323);
    let second = sweep_unclaimed(&mut escrow, "dao", 324);
    let unclaimed_ok =
        moved == Ok(999_985) && escrow.balance == 0 && second == Err("nothing to sweep");
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

#[cfg(test)]
mod sweep_tests {
    use super::*;

    fn funded() -> Escrow {
        Escrow {
            owner: "dao",
            campaign_set: true,
            cutoff_height: 80,
            sweep_recipient: "terp1treasury",
            staged: false,
            paid: false,
            balance: 999_985,
        }
    }

    #[test]
    fn sweep_moves_the_balance_after_the_cutoff() {
        let mut escrow = funded();
        assert_eq!(sweep_unclaimed(&mut escrow, "dao", 81).unwrap(), 999_985);
        assert_eq!(escrow.balance, 0);
        assert_eq!(
            sweep_unclaimed(&mut escrow, "dao", 82),
            Err("nothing to sweep")
        );
    }

    #[test]
    fn sweep_refuses_the_gates_the_contract_refuses() {
        let mut missing = funded();
        missing.campaign_set = false;
        assert_eq!(
            sweep_unclaimed(&mut missing, "dao", 81),
            Err("epoch campaign is not set")
        );
        assert_eq!(missing.balance, 999_985);

        let mut stranger = funded();
        assert!(sweep_unclaimed(&mut stranger, "other", 81)
            .unwrap_err()
            .contains("owner"));
        assert_eq!(stranger.balance, 999_985);

        let mut empty = funded();
        empty.sweep_recipient = "";
        assert_eq!(
            sweep_unclaimed(&mut empty, "dao", 81),
            Err("sweep recipient is not set")
        );

        let mut early = funded();
        assert_eq!(
            sweep_unclaimed(&mut early, "dao", 80),
            Err("campaign has not ended")
        );
        assert_eq!(early.balance, 999_985);

        let mut unpaid = funded();
        unpaid.staged = true;
        unpaid.paid = false;
        assert_eq!(
            sweep_unclaimed(&mut unpaid, "dao", 81),
            Err("staged batch is unpaid")
        );
        assert_eq!(unpaid.balance, 999_985);

        let mut paid = funded();
        paid.staged = true;
        paid.paid = true;
        assert_eq!(sweep_unclaimed(&mut paid, "dao", 81).unwrap(), 999_985);
        assert_eq!(paid.balance, 0);
    }
}
