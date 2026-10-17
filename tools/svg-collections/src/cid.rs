//! CIDv1 for a single-block file, matching `ipfs add --cid-version 1 --only-hash` (kubo
//! default chunker = 256 KiB, raw leaves on for CIDv1): a file of <= 262144 bytes is one raw
//! block, so CID = base32lower("b" + varint(1) + varint(0x55 raw) + 0x12 0x20 + sha256).
use sha2::{Digest, Sha256};

pub const MAX_SINGLE_BLOCK: usize = 262_144;

fn base32_lower(data: &[u8]) -> String {
    const A: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(A[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn cid_v1_raw(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_SINGLE_BLOCK {
        return Err(format!("{} bytes > {MAX_SINGLE_BLOCK}: would be chunked into a dag-pb root", bytes.len()));
    }
    let mut v = vec![0x01, 0x55, 0x12, 0x20];
    v.extend_from_slice(&Sha256::digest(bytes));
    Ok(format!("b{}", base32_lower(&v)))
}
