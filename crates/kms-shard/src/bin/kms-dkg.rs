//! File-exchange FROST DKG. Shares stay on this host; never printed.

use std::fs;
use std::path::PathBuf;

use kms_shard::{dkg_all, ConsensusScheme, FrostEd25519};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| {
        eprintln!("usage: kms-dkg init --n N --t T --out DIR");
        std::process::exit(2);
    });
    match cmd.as_str() {
        "init" => init(&mut args),
        "pubkey" => pubkey(&mut args),
        _ => {
            eprintln!("unknown command {cmd}");
            std::process::exit(2);
        }
    }
}

fn init(args: &mut impl Iterator<Item = String>) {
    let rest: Vec<String> = args.collect();
    let mut n = 3u16;
    let mut t = 2u16;
    let mut out = PathBuf::from(".");
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--n" => {
                n = rest[i + 1].parse().unwrap();
                i += 2;
            }
            "--t" => {
                t = rest[i + 1].parse().unwrap();
                i += 2;
            }
            "--out" => {
                out = PathBuf::from(&rest[i + 1]);
                i += 2;
            }
            _ => i += 1,
        }
    }
    fs::create_dir_all(&out).unwrap();
    let mut rng = rand_core::OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(n, t, &mut rng).expect("dkg");
    let gbytes = group.serialize().expect("group");
    fs::write(out.join("group.bin"), &gbytes).unwrap();
    let pk = FrostEd25519::group_pk_bytes(&group).unwrap();
    fs::write(out.join("group.hex"), hex::encode_upper_or(&pk)).unwrap();
    for (id, share) in shares {
        let raw = share.serialize().expect("share");
        fs::write(out.join(format!("share-{id}.bin")), raw).unwrap();
    }
    println!("dkg ok n={n} t={t} group_pk={}", hex::encode_upper_or(&pk));
}

fn pubkey(args: &mut impl Iterator<Item = String>) {
    let rest: Vec<String> = args.collect();
    let mut share = PathBuf::from("share-1.bin");
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--share" {
            share = PathBuf::from(&rest[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
    }
    let raw = fs::read(&share).expect("read share");
    let kp = frost_ed25519::keys::KeyPackage::deserialize(&raw).expect("keypackage");
    let v: Vec<u8> = kp.verifying_key().serialize().expect("pk");
    println!("{}", hex::encode_upper_or(&v));
}

mod hex {
    pub fn encode_upper_or(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
}
