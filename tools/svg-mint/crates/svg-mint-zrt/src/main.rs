//! Product ZEC wallet for the SVG mint desk.
//!
//! The spend seed lives in SVG_MINT_ZRT_DIR (default ~/.terp/svg-mint-zrt).
//! Commands other than `init` refuse to create a wallet. `send` checks the
//! donation UA and the note ledger, then stops: this build does not prove or
//! broadcast, and it does not mark notes spent.

use std::env;
use std::process::ExitCode;

use svg_mint_zrt::{self, dir_from_env, DONATION_UA};

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        eprintln!(
            "svg-mint-zrt — durable Ironwood wallet for permissionless SVG mint\n\
             \n\
             Directory: SVG_MINT_ZRT_DIR or ~/.terp/svg-mint-zrt\n\
             \n\
             Commands:\n\
               init\n\
               addresses\n\
               balance --json\n\
               notes\n\
               send --zat <n> --address <u1…>\n\
             \n\
             init is the only command that creates a spend seed.\n\
             send does not broadcast and does not mark notes spent in this build."
        );
        return ExitCode::SUCCESS;
    }
    let cmd = args.remove(0);
    let dir = dir_from_env();
    match cmd.as_str() {
        "init" => match svg_mint_zrt::init(&dir) {
            Ok(acct) => {
                println!("{}", acct.address);
                ExitCode::SUCCESS
            }
            Err(e) => die(&e),
        },
        "addresses" => match svg_mint_zrt::open(&dir) {
            Ok(acct) => {
                println!("Address: {}", acct.address);
                println!("diversifier_index: {}", acct.diversifier_index);
                ExitCode::SUCCESS
            }
            Err(e) => die(&e),
        },
        "balance" => match svg_mint_zrt::open(&dir).and_then(|_| svg_mint_zrt::balance(&dir)) {
            Ok(v) => {
                println!("{v}");
                ExitCode::SUCCESS
            }
            Err(e) => die(&e),
        },
        "notes" => match svg_mint_zrt::notes_json(&dir) {
            Ok(v) => {
                println!("{v}");
                ExitCode::SUCCESS
            }
            Err(e) => die(&e),
        },
        "sync" => die(
            "compact scan is not linked in this build; the note ledger was not modified",
        ),
        "send" => cmd_send(&dir, &args),
        "lab-credit" => cmd_lab_credit(&dir, &args),
        "lab-commit-spend" => cmd_lab_commit(&dir, &args),
        "lab-send" => cmd_lab_send(&dir, &args),
        other => die(&format!("unknown command {other}")),
    }
}

fn cmd_send(dir: &std::path::Path, args: &[String]) -> ExitCode {
    let (zat, addr) = parse_zat_addr(args);
    if zat == 0 {
        die("send --zat <n> required");
    }
    if !addr.starts_with("u1") {
        die("send --address must be a unified address");
    }
    match svg_mint_zrt::prepare_send(dir, zat, &addr) {
        Ok(plan) => {
            let _ = plan;
            die(
                "ironwood spend prover is not linked; notes were not marked spent and nothing was broadcast",
            );
        }
        Err(e) => die(&e),
    }
}

fn parse_zat_addr(args: &[String]) -> (u64, String) {
    let mut zat = 0u64;
    let mut addr = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--zat" if i + 1 < args.len() => {
                zat = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--address" if i + 1 < args.len() => {
                addr = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }
    let _ = DONATION_UA;
    (zat, addr)
}

#[cfg(feature = "lab")]
fn cmd_lab_credit(dir: &std::path::Path, args: &[String]) -> ExitCode {
    let mut nullifier = String::new();
    let mut txid = String::new();
    let mut zat = 0u64;
    let mut mined = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nullifier" if i + 1 < args.len() => {
                nullifier = args[i + 1].clone();
                i += 2;
            }
            "--txid" if i + 1 < args.len() => {
                txid = args[i + 1].clone();
                i += 2;
            }
            "--zat" if i + 1 < args.len() => {
                zat = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--mined" if i + 1 < args.len() => {
                mined = args[i + 1] != "false";
                i += 2;
            }
            _ => i += 1,
        }
    }
    match svg_mint_zrt::credit_ironwood(dir, &nullifier, &txid, zat, mined, "") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => die(&e),
    }
}

#[cfg(not(feature = "lab"))]
fn cmd_lab_credit(_dir: &std::path::Path, _args: &[String]) -> ExitCode {
    die("unknown command lab-credit");
}

#[cfg(feature = "lab")]
fn cmd_lab_commit(dir: &std::path::Path, args: &[String]) -> ExitCode {
    let mut nullifier = String::new();
    let mut txid = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nullifier" if i + 1 < args.len() => {
                nullifier = args[i + 1].clone();
                i += 2;
            }
            "--txid" if i + 1 < args.len() => {
                txid = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }
    match svg_mint_zrt::commit_spend(dir, &[nullifier], &txid) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => die(&e),
    }
}

#[cfg(not(feature = "lab"))]
fn cmd_lab_commit(_dir: &std::path::Path, _args: &[String]) -> ExitCode {
    die("unknown command lab-commit-spend");
}

fn arg_val(args: &[String], name: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default()
}

/// Lab only: mark the prepared notes spent after an id exists. Does not broadcast.
#[cfg(feature = "lab")]
fn cmd_lab_send(dir: &std::path::Path, args: &[String]) -> ExitCode {
    let (zat, addr) = parse_zat_addr(args);
    let txid = arg_val(args, "--txid");
    if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
        die("lab-send requires --txid of 64 hex; notes were not marked spent");
    }
    match svg_mint_zrt::prepare_send(dir, zat, &addr) {
        Ok(plan) => match svg_mint_zrt::commit_spend(dir, &plan.nullifiers, &txid) {
            Ok(()) => {
                println!(
                    "{{\"ok\":true,\"broadcast\":false,\"to\":\"{}\",\"zat\":{},\"txid\":\"{}\",\"spent\":{}}}",
                    addr, zat, txid, plan.nullifiers.len()
                );
                ExitCode::SUCCESS
            }
            Err(e) => die(&e),
        },
        Err(e) => die(&e),
    }
}

#[cfg(not(feature = "lab"))]
fn cmd_lab_send(_dir: &std::path::Path, _args: &[String]) -> ExitCode {
    die("unknown command lab-send");
}
