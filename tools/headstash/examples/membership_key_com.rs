//! Read a 32-byte little-endian Pallas secret as hex on stdin and print the
//! hex of key_com = Poseidon(terp-hs-keycom-v1, secret), the same function the
//! browser worker exports as membership_key_com_hex. Prints only the commitment.

use ff::PrimeField;
use pasta_curves::pallas;
use std::io::Read;

fn fail() -> ! {
    eprintln!("secret");
    std::process::exit(2)
}

fn main() {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).expect("stdin");
    let bytes = hex::decode(text.trim()).unwrap_or_else(|_| fail());
    if bytes.len() != 32 {
        fail();
    }
    let mut repr = [0u8; 32];
    repr.copy_from_slice(&bytes);
    let secret = Option::<pallas::Base>::from(pallas::Base::from_repr(repr)).unwrap_or_else(|| fail());
    println!("{}", hex::encode(zk_headstash::no_ecdsa::key_commitment(secret).to_repr()));
}
