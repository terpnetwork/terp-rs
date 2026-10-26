//! Build the combined EVM + Solana eligibility root from the community snapshots.
//!
//! Usage: headstash-eligibility <communities_dir> <out_dir>
//! Reads every `<project>/<project>.csv` (`addr,amount`). `0x` rows are EVM;
//! anything else is a Solana entry (base64 of base58, or base58). Duplicate
//! identities are summed. Writes `tree.json` (sidecar input), `paths.jsonl`,
//! `allocations.jsonl` (one row per community x identity), and prints counts.
//! Never reads keys.
//!
//! Options (repeatable): `--decimals <project>=<n>` and `--budget <project>=<uterp>`.
//! Balances are parsed as exact decimals into base units (10^-n), never floats.
//! Without a budget the leaf amount is the base-unit balance (must fit u64).
//! With a budget, leaf = floor(budget * balance / community_total) in u128;
//! the floor remainder (dust) is reported per project and not distributed.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use headstash_epoch::eligibility::{allocate_floor, parse_base_units, decode_evm, decode_sol, EligTree, Identity, SolDrop, SolForm};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut pos, mut decimals, mut budgets) = (Vec::new(), BTreeMap::<String, u32>::new(), BTreeMap::<String, u64>::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--decimals" || a == "--budget" {
            let kv = args.next().ok_or("missing <project>=<n>")?;
            let (k, v) = kv.split_once('=').ok_or("expected <project>=<n>")?;
            if a == "--decimals" { decimals.insert(k.into(), v.parse()?); } else { budgets.insert(k.into(), v.parse()?); }
        } else { pos.push(a); }
    }
    let dir = PathBuf::from(pos.first().ok_or("communities dir")?);
    let out = PathBuf::from(pos.get(1).ok_or("out dir")?);
    let mut total_dust: u128 = 0;
    std::fs::create_dir_all(&out)?;
    let mut rows = Vec::new();
    // Per-(community, identity) leaf contributions, before cross-community summing.
    let mut allocs: Vec<(String, Identity, u64)> = Vec::new();
    let mut report: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut projects: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).collect();
    projects.sort_by_key(|e| e.file_name());
    for project in projects {
        let name = project.file_name().to_string_lossy().to_string();
        let csv = project.path().join(format!("{name}.csv"));
        let Ok(text) = std::fs::read_to_string(&csv) else { continue };
        let (mut evm, mut sol, mut bad_evm, mut bad_amount) = (0u64, 0u64, 0u64, 0u64);
        let dec = decimals.get(&name).copied().unwrap_or(0);
        let mut bad_samples: Vec<String> = Vec::new();
        let mut prow: Vec<(Identity, u128)> = Vec::new();
        let mut drops: BTreeMap<&str, u64> = BTreeMap::new();
        let mut forms: BTreeMap<&str, u64> = BTreeMap::new();
        for line in text.lines().skip(1) {
            let mut parts = line.split(',');
            let (Some(addr), Some(amount)) = (parts.next(), parts.next()) else { continue };
            let amount = match parse_base_units(amount, dec) {
                Ok(v) => v,
                Err(e) => { bad_amount += 1; if bad_samples.len() < 5 { bad_samples.push(e); } continue }
            };
            if addr.trim().starts_with("0x") || addr.trim().starts_with("0X") {
                match decode_evm(addr) {
                    Some(a) => { evm += 1; prow.push((Identity::Evm(a), amount)); }
                    None => bad_evm += 1,
                }
            } else {
                match decode_sol(addr) {
                    Ok((pk, form)) => {
                        sol += 1;
                        *forms.entry(match form { SolForm::Base64OfBase58 => "base64_of_base58", SolForm::Base58 => "base58" }).or_default() += 1;
                        prow.push((Identity::Sol(pk), amount));
                    }
                    Err(e) => *drops.entry(match e { SolDrop::Encoding => "encoding", SolDrop::Length => "length", SolDrop::OffCurve => "off_curve" }).or_default() += 1,
                }
            }
        }
        let total_units: u128 = prow.iter().map(|r| r.1).sum();
        let (mode, dust, budget) = match budgets.get(&name) {
            Some(&b) => {
                let bals: Vec<u128> = prow.iter().map(|r| r.1).collect();
                let (alloc, dust) = allocate_floor(&bals, b)?;
                for ((id, _), a) in prow.into_iter().zip(alloc) { allocs.push((name.clone(), id.clone(), a)); rows.push((id, a)); }
                ("budget_floor", dust, Some(b))
            }
            None => {
                for (id, v) in prow { let a = u64::try_from(v).map_err(|_| format!("{name}: base-unit balance exceeds u64; pass --budget"))?; allocs.push((name.clone(), id.clone(), a)); rows.push((id, a)); }
                ("base_units", 0, None)
            }
        };
        total_dust += dust as u128;
        report.insert(name, serde_json::json!({"evm_valid": evm, "evm_invalid": bad_evm, "sol_valid": sol, "sol_dropped": drops, "sol_forms": forms, "bad_amount": bad_amount, "bad_amount_samples": bad_samples, "decimals": dec, "total_base_units": total_units.to_string(), "mode": mode, "budget_uterp": budget, "dust_uterp": dust}));
    }
    let raw_rows = rows.len();
    let mut af = std::io::BufWriter::new(std::fs::File::create(out.join("allocations.jsonl"))?);
    for (project, id, amount) in &allocs {
        writeln!(af, "{}", serde_json::json!({"project": project, "identity": id.key(), "amount": amount}))?;
    }
    drop(af);
    let tree = EligTree::build(rows)?;
    let root = hex::encode(tree.root());
    let entries: Vec<_> = tree.entries.iter().map(|(id, amount)| match id {
        Identity::Evm(a) => serde_json::json!({"address_hex": hex::encode(a), "amount": amount}),
        Identity::Sol(p) => serde_json::json!({"sol_pubkey_hex": hex::encode(p), "amount": amount}),
    }).collect();
    let evm_ids = tree.entries.iter().filter(|(i, _)| matches!(i, Identity::Evm(_))).count();
    std::fs::write(out.join("tree.json"), serde_json::to_vec(&serde_json::json!({"depth": tree.depth, "root_hex": root, "entries": entries}))?)?;
    let mut paths = std::io::BufWriter::new(std::fs::File::create(out.join("paths.jsonl"))?);
    for (i, (id, amount)) in tree.entries.iter().enumerate() {
        let path: Vec<String> = tree.path(i).iter().map(hex::encode).collect();
        writeln!(paths, "{}", serde_json::json!({"identity": id.key(), "index": i, "amount": amount, "path": path}))?;
    }
    let summary = serde_json::json!({
        "root_hex": root, "depth": tree.depth, "rows": raw_rows,
        "identities": tree.entries.len(), "evm_identities": evm_ids,
        "sol_identities": tree.entries.len() - evm_ids, "total_dust_uterp": total_dust.to_string(), "projects": report,
    });
    std::fs::write(out.join("summary.json"), serde_json::to_vec_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
