//! Slim ics23 host: only SHA-256 and BLAKE3 (IAVL + tendermint simple merkle).
//! Other HashOps panic — they are not on the RecvPacket path.

use sha2::{Digest, Sha256};

pub struct Ics23Host;

impl ics23::HostFunctionsProvider for Ics23Host {
    fn sha2_256(message: &[u8]) -> [u8; 32] {
        Sha256::digest(message).into()
    }

    fn blake3(message: &[u8]) -> [u8; 32] {
        *blake3::hash(message).as_bytes()
    }

    fn sha2_512(_: &[u8]) -> [u8; 64] {
        unused()
    }
    fn sha2_512_truncated(_: &[u8]) -> [u8; 32] {
        unused()
    }
    fn keccak_256(_: &[u8]) -> [u8; 32] {
        unused()
    }
    fn ripemd160(_: &[u8]) -> [u8; 20] {
        unused()
    }
    fn blake2b_512(_: &[u8]) -> [u8; 64] {
        unused()
    }
    fn blake2s_256(_: &[u8]) -> [u8; 32] {
        unused()
    }
    fn blake2b_256(_: &[u8]) -> [u8; 32] {
        unused()
    }
}

fn unused() -> ! {
    panic!("ics23 HashOp not compiled into production LC")
}
