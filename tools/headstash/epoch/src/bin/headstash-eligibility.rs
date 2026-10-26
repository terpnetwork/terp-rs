//! Build the combined EVM + Solana eligibility root from the community snapshots.
//!
//! Usage: headstash-eligibility <communities_dir> <out_dir>
//! Reads every `<project>/<project>.csv` (`addr,amount`). `0x` rows are EVM;
//! anything else is a Solana entry (base64 of base58, or base58). Duplicate
//! identities are summed. Writes `tree.json` (sidecar input), `paths.jsonl`,
//! and prints counts. Never reads keys.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use headstash_epoch::eligibility::{decode_evm, decode_sol, EligTree, Identity, SolDrop, SolForm};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("communities dir")?);
    let out = PathBuf::from(args.next().ok_or("out dir")?);
    std::fs::create_dir_all(&out)?;
    let mut rows = Vec::new();
    let mut report: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut projects: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).collect();
    projects.sort_by_key(|e| e.file_name());
    for project in projects {
        let name = project.file_name().to_string_lossy().to_string();
        let csv = project.path().join(format!("{name}.csv"));
        let Ok(text) = std::fs::read_to_string(&csv) else { continue };
        let (mut evm, mut sol, mut bad_evm, mut bad_amount) = (0u64, 0u64, 0u64, 0u64);
        let mut drops: BTreeMap<&str, u64> = BTreeMap::new();
        let mut forms: BTreeMap<&str, u64> = BTreeMap::new();
        for line in text.lines().skip(1) {
            let mut parts = line.split(',');
            let (Some(addr), Some(amount)) = (parts.next(), parts.next()) else { continue };
            let Ok(amount) = amount.trim().parse::<u64>() else { bad_amount += 1; continue };
            if addr.trim().starts_with("0x") || addr.trim().starts_with("0X") {
                match decode_evm(addr) {
                    Some(a) => { evm += 1; rows.push((Identity::Evm(a), amount)); }
                    None => bad_evm += 1,
                }
            } else {
                match decode_sol(addr) {
                    Ok((pk, form)) => {
                        sol += 1;
                        *forms.entry(match form { SolForm::Base64OfBase58 => "base64_of_base58", SolForm::Base58 => "base58" }).or_default() += 1;
                        rows.push((Identity::Sol(pk), amount));
                    }
                    Err(e) => *drops.entry(match e { SolDrop::Encoding => "encoding", SolDrop::Length => "length", SolDrop::OffCurve => "off_curve" }).or_default() += 1,
                }
            }
        }
        report.insert(name, serde_json::json!({"evm_valid": evm, "evm_invalid": bad_evm, "sol_valid": sol, "sol_dropped": drops, "sol_forms": forms, "bad_amount": bad_amount}));
    }
    let raw_rows = rows.len();
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
        "sol_identities": tree.entries.len() - evm_ids, "projects": report,
    });
    std::fs::write(out.join("summary.json"), serde_json::to_vec_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
