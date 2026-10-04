/// Reads a tiered whitelist CSV (from parse_export) and generates a merkle tree
/// with per-address allocation proofs.
///
/// Each leaf is `blake3::hash(format!("{}{}", address, allocation))`, matching
/// the whitelist-merkletree contract's allocation-based verification.
///
/// Outputs a JSON file with:
///   - merkle_root: hex-encoded root hash
///   - total_addresses: count
///   - tier_summary: per-tier counts
///   - accounts: map of address → { tier, allocation, proof_hashes }
///
/// Usage:
///   cargo run -p cw-infuser-scripts --bin gen_merkle -- --csv data/whitelist.csv -o merkle.json
///
///   With instantiate msg:
///  cargo run -p cw-infuser-scripts --bin gen_merkle -- --csv data/whitelist.csv -o data/terp-warriors.json --instantiate-output data/mtree-init.json --merkle-tree-uri https://mtree-api.terp.network
use anyhow::{Context, Result};
use clap::Parser;
use rs_merkle::{Hasher, MerkleTree};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

#[derive(Parser, Debug)]
#[command(version, about = "Generate a merkle tree from a tiered whitelist CSV")]
struct Args {
    /// Path to the whitelist CSV (address,tier,allocation)
    #[arg(long)]
    csv: String,

    /// Output JSON file for the merkle tree (defaults to stdout)
    #[arg(long, short)]
    output: Option<String>,

    /// Output JSON file for the whitelist InstantiateMsg
    #[arg(long)]
    instantiate_output: Option<String>,

    /// Admin address (repeatable)
    #[arg(long = "admin")]
    admins: Vec<String>,

    /// Whether the admin list is mutable
    #[arg(long, default_value = "false")]
    admins_mutable: bool,

    /// Optional IPFS/HTTP URI pointing to the full merkle tree JSON
    #[arg(long)]
    merkle_tree_uri: Option<String>,
}

/// Blake3 hasher that sorts left/right before concatenation.
/// Must match the on-chain whitelist-merkletree contract's verification logic.
#[derive(Clone)]
struct SortingBlake3Hasher;

impl Hasher for SortingBlake3Hasher {
    type Hash = [u8; 32];

    fn concat_and_hash(left: &Self::Hash, right: Option<&Self::Hash>) -> Self::Hash {
        match right {
            Some(right_node) => {
                let mut both = [left, right_node];
                both.sort_unstable();

                let mut concatenated: Vec<u8> = both[0].to_vec();
                concatenated.extend_from_slice(both[1]);

                Self::hash(&concatenated)
            }
            None => *left,
        }
    }

    fn hash(data: &[u8]) -> Self::Hash {
        *blake3::hash(data).as_bytes()
    }

    fn hash_size() -> usize {
        blake3::OUT_LEN
    }
}

#[derive(Debug, Deserialize)]
struct CsvRow {
    address: String,
    #[serde(default)]
    operator_address: String,
    #[serde(default)]
    moniker: String,
    tier: u8,
    allocation: u32,
}

#[derive(Serialize)]
struct AccountProof {
    tier: u8,
    allocation: u32,
    proof_hashes: Vec<String>,
}

#[derive(Serialize)]
struct TierSummary {
    count: usize,
    allocation: u32,
}

#[derive(Serialize)]
struct MerkleOutput {
    merkle_root: String,
    total_addresses: usize,
    tier_summary: BTreeMap<String, TierSummary>,
    accounts: BTreeMap<String, AccountProof>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Read CSV
    let mut reader = csv::Reader::from_path(&args.csv)
        .with_context(|| format!("failed to open CSV: {}", args.csv))?;

    let mut rows: Vec<CsvRow> = Vec::new();
    for result in reader.deserialize() {
        let row: CsvRow = result.with_context(|| "failed to parse CSV row")?;
        rows.push(row);
    }

    if rows.is_empty() {
        anyhow::bail!("CSV is empty — no addresses to process");
    }

    eprintln!("Loaded {} addresses from CSV", rows.len());

    // 2. Build leaves: blake3::hash(format!("{}{}", address, allocation))
    let leaves: Vec<[u8; 32]> = rows
        .iter()
        .map(|row| {
            let leaf_input = format!("{}{}", row.address, row.allocation);
            *blake3::hash(leaf_input.as_bytes()).as_bytes()
        })
        .collect();

    // 3. Build merkle tree
    let tree = MerkleTree::<SortingBlake3Hasher>::from_leaves(&leaves);
    let root = tree.root_hex().context("empty tree — no root")?;

    eprintln!("Merkle root: {}", root);

    // 4. Generate proofs for each address
    let mut accounts: BTreeMap<String, AccountProof> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let proof = tree.proof(&[i]);
        let proof_hashes: Vec<String> = proof.proof_hashes_hex();

        accounts.insert(
            row.address.clone(),
            AccountProof {
                tier: row.tier,
                allocation: row.allocation,
                proof_hashes,
            },
        );
    }

    // 5. Build tier summary
    let mut tier_summary: BTreeMap<String, TierSummary> = BTreeMap::new();
    for row in &rows {
        let entry = tier_summary
            .entry(format!("tier_{}", row.tier))
            .or_insert(TierSummary {
                count: 0,
                allocation: row.allocation,
            });
        entry.count += 1;
    }

    // 6. Output
    let output = MerkleOutput {
        merkle_root: root.clone(),
        total_addresses: rows.len(),
        tier_summary,
        accounts,
    };

    let json = serde_json::to_string_pretty(&output)?;

    match args.output {
        Some(path) => {
            fs::write(&path, &json).with_context(|| format!("failed to write: {}", path))?;
            eprintln!("Merkle tree JSON written to: {}", path);
        }
        None => {
            println!("{}", json);
        }
    }

    // 7. Optionally write InstantiateMsg JSON
    if let Some(inst_path) = &args.instantiate_output {
        let init_msg = serde_json::json!({
            "merkle_root": root,
            "merkle_tree_uri": args.merkle_tree_uri,
            "admins": args.admins,
            "admins_mutable": args.admins_mutable
        });

        let init_json = serde_json::to_string_pretty(&init_msg)?;
        fs::write(inst_path, &init_json)
            .with_context(|| format!("failed to write instantiate msg to: {}", inst_path))?;
        eprintln!("Instantiate msg JSON written to: {}", inst_path);
    }

    eprintln!("\nSummary:");
    for (tier, summary) in &output.tier_summary {
        eprintln!(
            "  {}: {} addresses (allocation: {})",
            tier, summary.count, summary.allocation
        );
    }
    eprintln!("  Total: {} addresses", output.total_addresses);

    Ok(())
}
