//! Poseidon-v1 Headstash notes + inclusion tree from enriched community CSVs.
//!
//! Only `pubkey_status=ok` rows. Unallocated/burn rows are skipped.
//! Leaf hash uses the recovered **compressed secp256k1 pubkey** (not an ETH address
//! as a fake spending key).
//!
//! ```bash
//! cargo run -p headstash-eth-pubkeys --release --bin gen_community_notes
//! just gen-community-notes
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ff::{Field, PrimeField};
use pasta_curves::pallas;
use rand::RngExt;
use rayon::prelude::*;
use serde_json::{json, Value};
use zk_headstash::circuit::gadget::secp256k1_chip::secp_coord_be_to_pallas_base;
use zk_headstash::distro_poseidon::{poseidon_distro_crh, poseidon_distro_leaf};
use zk_headstash::keys::EligiblePk;
use zk_headstash::value::NoteDenom;
use zk_headstash::FIXED_AMOUNTS;

fn communities_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tools/headstash/headstash/communities")
}

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tools/headstash/artifacts/community-notes")
}

struct Row {
    community: String,
    addr: String,
    pk_compressed: String,
    pk_uncompressed: String,
    token: String,
    value: u64,
}

fn load_ok_rows(dir: &Path) -> anyhow::Result<Vec<Row>> {
    let mut rows = Vec::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        if !ent.file_type()?.is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        let csv = ent.path().join(format!("{name}.enriched.csv"));
        if !csv.exists() {
            continue;
        }
        let mut rdr = csv::Reader::from_path(&csv)?;
        for rec in rdr.records() {
            let rec = rec?;
            let addr = rec.get(0).unwrap_or("").trim();
            let amount = rec.get(1).unwrap_or("0");
            let pk_c = rec.get(2).unwrap_or("").trim();
            let pk_u = rec.get(3).unwrap_or("").trim();
            let status = rec.get(5).unwrap_or("").trim();
            if status != "ok" || !pk_c.starts_with("0x") {
                continue;
            }
            let value = amount.parse::<f64>().unwrap_or(0.0).round() as u64;
            if value == 0 {
                continue;
            }
            rows.push(Row {
                community: name.clone(),
                addr: addr.to_string(),
                pk_compressed: pk_c.to_string(),
                pk_uncompressed: pk_u.to_string(),
                token: name.clone(),
                value,
            });
        }
    }
    Ok(rows)
}

fn split_denoms(v: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut rem = v;
    for &d in FIXED_AMOUNTS.iter() {
        let n = rem / d;
        if n == 0 {
            rem %= d;
            continue;
        }
        out.extend(std::iter::repeat(d).take(n as usize));
        rem %= d;
    }
    out
}

/// Circuit `epk = esk·G` natives: same `EligiblePk::xy()` the prover must witness.
fn epk_for_proof(compressed_hex: &str) -> anyhow::Result<([u8; 32], [u8; 32], pallas::Base, pallas::Base, String)> {
    let raw = hex::decode(compressed_hex.trim_start_matches("0x"))?;
    anyhow::ensure!(raw.len() == 33, "elig_pk compressed must be 33 bytes");
    let pk = EligiblePk::from(&raw);
    let (x, y) = pk.xy();
    let uncompressed = {
        let mut u = Vec::with_capacity(65);
        u.push(0x04);
        u.extend_from_slice(&x);
        u.extend_from_slice(&y);
        format!("0x{}", hex::encode(u))
    };
    Ok((
        x,
        y,
        secp_coord_be_to_pallas_base(&x),
        secp_coord_be_to_pallas_base(&y),
        uncompressed,
    ))
}

fn recp_from_eth(addr: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    if let Ok(b) = hex::decode(addr.trim_start_matches("0x")) {
        if b.len() == 20 {
            out[12..].copy_from_slice(&b);
        }
    }
    out
}

fn poseidon_root(mut c: Vec<pallas::Base>) -> pallas::Base {
    if c.is_empty() {
        return pallas::Base::ZERO;
    }
    let mut layer = 0u32;
    while c.len() > 1 {
        if c.len() % 2 != 0 {
            c.push(pallas::Base::ZERO);
        }
        c = c
            .par_chunks(2)
            .map(|pair| poseidon_distro_crh(layer, pair[0], pair[1]))
            .collect();
        layer += 1;
    }
    c[0]
}

fn main() -> anyhow::Result<()> {
    let dir = communities_dir();
    let rows = load_ok_rows(&dir)?;
    eprintln!("[notes] keyed holders={}", rows.len());

    struct Piece {
        community: String,
        addr: String,
        pk_compressed: String,
        token: String,
        value: u64,
        fdi: u64,
    }
    let mut pieces: Vec<Piece> = rows
        .into_iter()
        .flat_map(|r| {
            split_denoms(r.value)
                .into_iter()
                .map(move |value| Piece {
                    community: r.community.clone(),
                    addr: r.addr.clone(),
                    pk_compressed: r.pk_compressed.clone(),
                    token: r.token.clone(),
                    value,
                    fdi: 0,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    pieces.sort_by(|a, b| {
        (&a.community, &a.addr, a.value, &a.pk_compressed).cmp(&(
            &b.community,
            &b.addr,
            b.value,
            &b.pk_compressed,
        ))
    });
    for (i, p) in pieces.iter_mut().enumerate() {
        p.fdi = i as u64;
    }
    eprintln!(
        "[notes] denomination pieces={} threads={}",
        pieces.len(),
        rayon::current_num_threads()
    );

    let notes: Vec<Value> = pieces
        .par_iter()
        .map(|p| {
            let (x, y, epk_x, epk_y, uncompressed) =
                epk_for_proof(&p.pk_compressed).expect("elig_pk");
            let nd = NoteDenom::new_for_proof(&p.token);
            let mut rseed = [0u8; 32];
            rand::rng().fill(&mut rseed);
            let (lo, hi) = zk_headstash::claim_auth::rseed_halves(&rseed);
            let leaf = poseidon_distro_leaf(
                epk_x,
                epk_y,
                nd.to_fp(),
                pallas::Base::from(p.value),
                pallas::Base::from(p.fdi),
                zk_headstash::claim_auth::claim_rseed_com(lo, hi),
            );
            json!({
                "community": p.community,
                "addr": p.addr,
                "elig_pk_compressed": p.pk_compressed,
                "elig_pk_uncompressed": uncompressed,
                "epk_x": format!("0x{}", hex::encode(x)),
                "epk_y": format!("0x{}", hex::encode(y)),
                "esk_hint": "rseed is the one-time witness for this leaf. It is not public. The current circuit also proves epk = esk·G with the ETH key for addr. Do not publish rseed or esk.",
                "token": p.token,
                "nd": format!("0x{}", hex::encode(nd.as_bytes())),
                "v": p.value,
                "fdi": p.fdi,
                "rseed": format!("0x{}", hex::encode(rseed)),
                "recp": format!("0x{}", hex::encode(recp_from_eth(&p.addr))),
                "leaf": format!("0x{}", hex::encode(leaf.to_repr())),
            })
        })
        .collect();

    let leaves: Vec<pallas::Base> = notes
        .par_iter()
        .map(|n| {
            let h = n["leaf"].as_str().unwrap().trim_start_matches("0x");
            let b = hex::decode(h).unwrap();
            let mut r = [0u8; 32];
            r.copy_from_slice(&b);
            pallas::Base::from_repr(r).unwrap()
        })
        .collect();

    let root = poseidon_root(leaves);
    let root_hex = format!("0x{}", hex::encode(root.to_repr()));

    let out = out_dir();
    fs::create_dir_all(&out)?;
    let merkle = json!({
        "root": root_hex,
        "count": notes.len(),
        "holders_keyed": notes.iter().filter_map(|n| n["addr"].as_str()).collect::<HashSet<_>>().len(),
        "distro_hash_domain": "poseidon-v1",
        "leaf_personalization": "terp-hs-distro-leaf-v1",
        "leaf_arity": 7,
        "leaf_fields": ["tag", "epk_x", "epk_y", "nd", "v", "fdi", "rseed_com"],
        "rseed_com": "Poseidon(terp-hs-rseed-v1, lo, hi). The 32-byte rseed is stored on the note only.",
        "supersedes_root": "0x6bb155b7a04eb34410bb32f5790e6c6735940dcb08964397bfb91caff8b3272b",
        "threads": rayon::current_num_threads(),
    });
    fs::write(out.join("merkle_output.json"), serde_json::to_vec_pretty(&merkle)?)?;
    fs::write(out.join("notes.json"), serde_json::to_vec_pretty(&json!({ "notes": notes }))?)?;
    eprintln!("✅ genesis notes {}  root={root_hex}", notes.len());
    eprintln!("📁 {}", out.display());
    Ok(())
}
