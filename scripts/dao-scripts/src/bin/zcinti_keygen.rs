//! Write a zcinti consensus key. Prints the public key and the file path.

use std::path::PathBuf;

use clap::Parser;
use dao_scripts::zcinti;

#[derive(Parser)]
struct Args {
    /// CometBFT key file. The private key is written here and is not printed.
    #[arg(long, default_value = "/tmp/zcinti-val/priv_validator_key.json")]
    out: PathBuf,
}

fn main() {
    let args = Args::parse();
    match zcinti::generate_consensus_key(&args.out) {
        Ok(key) => {
            println!("pubkey {}", key.pubkey_b64);
            println!("address {}", key.address_hex);
            println!("file {}", key.path);
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
