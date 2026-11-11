//! FROST (FE01) vs TC01 timing baselines for the light-client committee.
//! `cargo test -p frost_escrow --release --test timing -- --ignored --nocapture`
//! Prints a markdown table (median of `BENCH_ITERS`, default 15). In-process:
//! no network latency; per-signer numbers are what one operator spends.
use std::time::{Duration, Instant};

use frost_ed25519 as frost;
use frost_escrow::FrostLabCommittee;
use rand_core06::OsRng;
use std::collections::BTreeMap;
use threshold_committee::LabCommittee;

fn median(mut v: Vec<Duration>) -> f64 {
    v.sort();
    v[v.len() / 2].as_secs_f64() * 1e6
}
fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed())
}

#[test]
#[ignore = "timing baseline"]
fn frost_and_tc01_timing() {
    let iters: usize = std::env::var("BENCH_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(15);
    let msg = [7u8; 32];
    let mut out = String::from(
        "| scheme | t-of-n | keygen/DKG total (µs) | round1 commit / signer (µs) | round2 sign / signer (µs) | aggregate / combine (µs) | verify (µs) | sig bytes |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
    );
    for &(t, n) in &[(2u16, 3u16), (3, 4), (4, 5), (7, 10), (11, 16), (22, 32)] {
        let (mut dkg, mut r1, mut r2, mut ag, mut vf) = (vec![], vec![], vec![], vec![], vec![]);
        let mut sig_len = 0;
        for _ in 0..iters {
            let (c, d) = time(|| FrostLabCommittee::dkg(t, n).unwrap());
            dkg.push(d);
            let mut rng = OsRng;
            let ids: Vec<frost::Identifier> = (1..=t).map(|i| i.try_into().unwrap()).collect();
            let (mut nonces, mut comms) = (BTreeMap::new(), BTreeMap::new());
            let (_, d) = time(|| {
                for id in &ids {
                    let (nn, cm) = frost::round1::commit(c.key_packages[id].signing_share(), &mut rng);
                    nonces.insert(*id, nn);
                    comms.insert(*id, cm);
                }
            });
            r1.push(d / t as u32);
            let pkg = frost::SigningPackage::new(comms, &msg);
            let mut shares = BTreeMap::new();
            let (_, d) = time(|| {
                for id in &ids {
                    shares.insert(*id, frost::round2::sign(&pkg, &nonces[id], &c.key_packages[id]).unwrap());
                }
            });
            r2.push(d / t as u32);
            let (sig, d) = time(|| frost::aggregate(&pkg, &shares, &c.pubkey_package).unwrap());
            ag.push(d);
            let bytes = sig.serialize().unwrap();
            sig_len = bytes.len();
            let (_, d) = time(|| frost_escrow::verify_encoded(&c.encoded_public, &msg, &bytes).unwrap());
            vf.push(d);
        }
        out += &format!(
            "| FE01 FROST-ed25519 | {t}-of-{n} | {:.0} | {:.1} | {:.1} | {:.1} | {:.1} | {sig_len} |\n",
            median(dkg), median(r1), median(r2), median(ag), median(vf)
        );
        let (mut kg, mut ps, mut cb, mut tv) = (vec![], vec![], vec![], vec![]);
        let mut tc_len = 0;
        for _ in 0..iters {
            let (c, d) = time(|| LabCommittee::generate(t, n).unwrap());
            kg.push(d);
            let (partials, d) = time(|| {
                (0..t)
                    .map(|i| c.share(i).unwrap().partial_sign_digest(&msg).unwrap())
                    .collect::<Vec<_>>()
            });
            ps.push(d / t as u32);
            let (enc, d) = time(|| threshold_committee::combine(&c.roster, &msg, &partials).unwrap().encode());
            cb.push(d);
            tc_len = enc.len();
            let (_, d) = time(|| threshold_committee::verify_encoded(&c.encoded_public, &msg, &enc).unwrap());
            tv.push(d);
        }
        out += &format!(
            "| TC01 k-of-n secp256k1 partials | {t}-of-{n} | {:.0} | – | {:.1} | {:.1} | {:.1} | {tc_len} |\n",
            median(kg), median(ps), median(cb), median(tv)
        );
    }
    println!("{out}");
    if let Ok(p) = std::env::var("BENCH_OUT") {
        std::fs::write(p, &out).unwrap();
    }
}
