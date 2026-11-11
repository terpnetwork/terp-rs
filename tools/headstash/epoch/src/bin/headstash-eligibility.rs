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
//!
//! `--exclude <csv>` (`address,community,reason,evidence`; community `*` = all):
//! excluded rows get no leaf. The split still uses the ORIGINAL community total,
//! so other holders are unchanged and the excluded share is reported as
//! `unallocated_uterp` (not redistributed).
//!
//! `--denominations` (rev 5): also build the per-note depth-17 tree
//! (`denominations.rs`) into `<out>/notes/`: `tree.json`, `notes.jsonl` (one row per
//! identity, landing lookup), `note_paths.jsonl` (one row per note) and
//! `summary.json` with the class table and the uterp invariant
//! notes + remainder + dust + unallocated == sum of budgets (`--total <uterp>` also checks it).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;

use headstash_epoch::eligibility::{
    EligTree, Identity, SolDrop, SolForm, allocate_floor, apply_exclusions, decode_evm, decode_sol,
    parse_base_units,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut pos, mut decimals, mut budgets) = (
        Vec::new(),
        BTreeMap::<String, u32>::new(),
        BTreeMap::<String, u64>::new(),
    );
    let mut exclude_file: Option<PathBuf> = None;
    let (mut denoms, mut expect_total) = (false, None::<u128>);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--denominations" {
            denoms = true;
        } else if a == "--total" {
            expect_total = Some(args.next().ok_or("missing --total <uterp>")?.parse()?);
        } else if a == "--exclude" {
            exclude_file = Some(PathBuf::from(args.next().ok_or("missing --exclude <csv>")?));
        } else if a == "--decimals" || a == "--budget" {
            let kv = args.next().ok_or("missing <project>=<n>")?;
            let (k, v) = kv.split_once('=').ok_or("expected <project>=<n>")?;
            if a == "--decimals" {
                decimals.insert(k.into(), v.parse()?);
            } else {
                budgets.insert(k.into(), v.parse()?);
            }
        } else {
            pos.push(a);
        }
    }
    let dir = PathBuf::from(pos.first().ok_or("communities dir")?);
    let out = PathBuf::from(pos.get(1).ok_or("out dir")?);
    let mut total_dust: u128 = 0;
    let mut total_unallocated: u128 = 0;
    // community ("*" = all) -> excluded identities
    let mut excl: BTreeMap<String, BTreeSet<Identity>> = BTreeMap::new();
    if let Some(f) = &exclude_file {
        for (n, line) in std::fs::read_to_string(f)?.lines().enumerate().skip(1) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut p = line.split(',');
            let (Some(addr), Some(comm)) = (p.next(), p.next()) else {
                return Err(format!("exclude line {}: need address,community", n + 1).into());
            };
            let addr = addr.trim();
            let id = if addr.starts_with("0x") || addr.starts_with("0X") {
                Identity::Evm(
                    decode_evm(addr)
                        .ok_or_else(|| format!("exclude line {}: bad evm {addr}", n + 1))?,
                )
            } else {
                match decode_sol(addr) {
                    Ok((pk, _)) => Identity::Sol(pk),
                    // Off-curve / undecodable Solana entries never get a leaf anyway.
                    Err(_) => continue,
                }
            };
            excl.entry(comm.trim().to_string()).or_default().insert(id);
        }
    }
    let all_excl = excl.get("*").cloned().unwrap_or_default();
    std::fs::create_dir_all(&out)?;
    let mut rows = Vec::new();
    // Per-(community, identity) leaf contributions, before cross-community summing.
    let mut allocs: Vec<(String, Identity, u64)> = Vec::new();
    let mut report: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut projects: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    projects.sort_by_key(|e| e.file_name());
    for project in projects {
        let name = project.file_name().to_string_lossy().to_string();
        let csv = project.path().join(format!("{name}.csv"));
        let Ok(text) = std::fs::read_to_string(&csv) else {
            continue;
        };
        let (mut evm, mut sol, mut bad_evm, mut bad_amount) = (0u64, 0u64, 0u64, 0u64);
        let dec = decimals.get(&name).copied().unwrap_or(0);
        let mut bad_samples: Vec<String> = Vec::new();
        let mut prow: Vec<(Identity, u128)> = Vec::new();
        let mut drops: BTreeMap<&str, u64> = BTreeMap::new();
        let mut forms: BTreeMap<&str, u64> = BTreeMap::new();
        for line in text.lines().skip(1) {
            let mut parts = line.split(',');
            let (Some(addr), Some(amount)) = (parts.next(), parts.next()) else {
                continue;
            };
            let amount = match parse_base_units(amount, dec) {
                Ok(v) => v,
                Err(e) => {
                    bad_amount += 1;
                    if bad_samples.len() < 5 {
                        bad_samples.push(e);
                    }
                    continue;
                }
            };
            if addr.trim().starts_with("0x") || addr.trim().starts_with("0X") {
                match decode_evm(addr) {
                    Some(a) => {
                        evm += 1;
                        prow.push((Identity::Evm(a), amount));
                    }
                    None => bad_evm += 1,
                }
            } else {
                match decode_sol(addr) {
                    Ok((pk, form)) => {
                        sol += 1;
                        *forms
                            .entry(match form {
                                SolForm::Base64OfBase58 => "base64_of_base58",
                                SolForm::Base58 => "base58",
                            })
                            .or_default() += 1;
                        prow.push((Identity::Sol(pk), amount));
                    }
                    Err(e) => {
                        *drops
                            .entry(match e {
                                SolDrop::Encoding => "encoding",
                                SolDrop::Length => "length",
                                SolDrop::OffCurve => "off_curve",
                            })
                            .or_default() += 1
                    }
                }
            }
        }
        let total_units: u128 = prow.iter().map(|r| r.1).sum();
        let (mut excluded_rows, mut unallocated) = (0u64, 0u128);
        let (mode, dust, budget) = match budgets.get(&name) {
            Some(&b) => {
                let bals: Vec<u128> = prow.iter().map(|r| r.1).collect();
                let (alloc, dust) = allocate_floor(&bals, b)?;
                let mut ex = all_excl.clone();
                if let Some(s) = excl.get(&name) {
                    ex.extend(s.iter().copied());
                }
                let split: Vec<(Identity, u64)> =
                    prow.into_iter().map(|r| r.0).zip(alloc).collect();
                let (kept, unalloc, n) = apply_exclusions(split, &ex);
                excluded_rows = n;
                unallocated = unalloc;
                for (id, a) in kept {
                    allocs.push((name.clone(), id, a));
                    rows.push((id, a));
                }
                ("budget_floor", dust, Some(b))
            }
            None => {
                for (id, v) in prow {
                    let a = u64::try_from(v).map_err(|_| {
                        format!("{name}: base-unit balance exceeds u64; pass --budget")
                    })?;
                    allocs.push((name.clone(), id.clone(), a));
                    rows.push((id, a));
                }
                ("base_units", 0, None)
            }
        };
        if exclude_file.is_some() && budget.is_none() && !excl.is_empty() {
            return Err(format!("{name}: --exclude requires --budget").into());
        }
        total_dust += dust as u128;
        total_unallocated += unallocated;
        report.insert(name, serde_json::json!({"evm_valid": evm, "evm_invalid": bad_evm, "sol_valid": sol, "sol_dropped": drops, "sol_forms": forms, "bad_amount": bad_amount, "bad_amount_samples": bad_samples, "decimals": dec, "total_base_units": total_units.to_string(), "mode": mode, "budget_uterp": budget, "dust_uterp": dust, "excluded_rows": excluded_rows, "unallocated_uterp": unallocated.to_string()}));
    }
    let raw_rows = rows.len();
    let mut af = std::io::BufWriter::new(std::fs::File::create(out.join("allocations.jsonl"))?);
    for (project, id, amount) in &allocs {
        writeln!(
            af,
            "{}",
            serde_json::json!({"project": project, "identity": id.key(), "amount": amount})
        )?;
    }
    drop(af);
    let tree = EligTree::build(rows)?;
    let root = hex::encode(tree.root());
    let entries: Vec<_> = tree
        .entries
        .iter()
        .map(|(id, amount)| match id {
            Identity::Evm(a) => {
                serde_json::json!({"address_hex": hex::encode(a), "amount": amount})
            }
            Identity::Sol(p) => {
                serde_json::json!({"sol_pubkey_hex": hex::encode(p), "amount": amount})
            }
        })
        .collect();
    let evm_ids = tree
        .entries
        .iter()
        .filter(|(i, _)| matches!(i, Identity::Evm(_)))
        .count();
    std::fs::write(
        out.join("tree.json"),
        serde_json::to_vec(
            &serde_json::json!({"depth": tree.depth, "root_hex": root, "entries": entries}),
        )?,
    )?;
    let mut paths = std::io::BufWriter::new(std::fs::File::create(out.join("paths.jsonl"))?);
    for (i, (id, amount)) in tree.entries.iter().enumerate() {
        let path: Vec<String> = tree.path(i).iter().map(hex::encode).collect();
        writeln!(
            paths,
            "{}",
            serde_json::json!({"identity": id.key(), "index": i, "amount": amount, "path": path})
        )?;
    }
    let summary = serde_json::json!({
        "root_hex": root, "depth": tree.depth, "rows": raw_rows,
        "identities": tree.entries.len(), "evm_identities": evm_ids,
        "sol_identities": tree.entries.len() - evm_ids, "total_dust_uterp": total_dust.to_string(), "total_unallocated_uterp": total_unallocated.to_string(), "projects": report,
    });
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if denoms {
        let budget_sum: u128 = budgets.values().map(|b| *b as u128).sum();
        write_notes(
            &out.join("notes"),
            &tree.entries,
            total_dust,
            total_unallocated,
            budget_sum,
            expect_total,
        )?;
    }
    Ok(())
}

fn write_notes(
    dir: &std::path::Path,
    entries: &[(Identity, u64)],
    dust: u128,
    unallocated: u128,
    budget_sum: u128,
    expect_total: Option<u128>,
) -> Result<(), Box<dyn std::error::Error>> {
    use headstash_epoch::denominations::{DENOMS_UTERP, NOTE_TREE_DEPTH, NoteTree};
    std::fs::create_dir_all(dir)?;
    let t = NoteTree::build(entries)?;
    let root = hex::encode(t.root());
    let (notes_u, rem_u) = (t.notes_uterp(), t.remainder_uterp());
    let total = notes_u + rem_u + dust + unallocated;
    if !budgets_match(total, budget_sum) {
        return Err(format!("invariant: {total} != budgets {budget_sum}").into());
    }
    if let Some(e) = expect_total {
        if total != e {
            return Err(format!("invariant: {total} != --total {e}").into());
        }
    }
    let mut nl = std::io::BufWriter::new(std::fs::File::create(dir.join("notes.jsonl"))?);
    let mut pl = std::io::BufWriter::new(std::fs::File::create(dir.join("note_paths.jsonl"))?);
    let mut counts: Vec<usize> = Vec::new();
    let mut zero_note_ids = 0u64;
    for i in &t.ids {
        let notes: Vec<_> = i.notes.iter().enumerate().map(|(k, n)| serde_json::json!({"class": n.class, "amount": n.amount(), "note_index": n.note_index, "leaf_index": i.first_leaf + k})).collect();
        writeln!(
            nl,
            "{}",
            serde_json::json!({"identity": i.identity.key(), "allocation": i.allocation, "remainder": i.remainder, "notes": notes})
        )?;
        for (k, n) in i.notes.iter().enumerate() {
            let idx = i.first_leaf + k;
            let path: Vec<String> = t.path(idx).iter().map(hex::encode).collect();
            writeln!(
                pl,
                "{}",
                serde_json::json!({"identity": i.identity.key(), "class": n.class, "note_index": n.note_index, "leaf_index": idx, "leaf": hex::encode(t.leaf(idx)), "path": path})
            )?;
        }
        counts.push(i.notes.len());
        if i.notes.is_empty() {
            zero_note_ids += 1;
        }
    }
    drop((nl, pl));
    counts.sort();
    let n = counts.len().max(1);
    let classes: Vec<_> = t.class_table().iter().enumerate().map(|(c, (notes, holders))| serde_json::json!({"class": c, "amount_uterp": DENOMS_UTERP[c], "notes": notes, "holders": holders})).collect();
    let summary = serde_json::json!({
        "root_hex": root, "depth": NOTE_TREE_DEPTH, "identities": t.ids.len(), "notes": t.note_count(),
        "identities_without_notes": zero_note_ids,
        "notes_per_identity": {"mean": t.note_count() as f64 / n as f64, "p50": counts[n / 2], "p99": counts[(n * 99) / 100], "max": counts[n - 1]},
        "classes": classes,
        "notes_uterp": notes_u.to_string(), "remainder_uterp": rem_u.to_string(),
        "dust_uterp": dust.to_string(), "unallocated_uterp": unallocated.to_string(),
        "total_uterp": total.to_string(), "invariant_ok": true,
    });
    std::fs::write(
        dir.join("tree.json"),
        serde_json::to_vec(
            &serde_json::json!({"depth": NOTE_TREE_DEPTH, "root_hex": root, "notes": t.note_count(), "denoms_uterp": DENOMS_UTERP}),
        )?,
    )?;
    std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

/// Without budgets (base-unit mode) there is nothing to check against.
fn budgets_match(total: u128, budget_sum: u128) -> bool {
    budget_sum == 0 || total == budget_sum
}
