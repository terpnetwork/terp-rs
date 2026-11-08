//! IPFS CID validation without dependencies (submissions and bounty details point at JSON docs on IPFS).
//!
//! Accepted:
//! * CIDv0: `Qm…`, exactly 46 base58btc chars, decoding to the 34-byte multihash 0x12 0x20 ‖ sha2-256.
//! * CIDv1, multibase base32 lower (`b…`, RFC 4648 alphabet `a-z2-7`, no padding), 8..=128 chars,
//!   decoding to: version 0x01 ‖ codec varint ∈ {raw 0x55, dag-pb 0x70, dag-cbor 0x71, dag-json 0x0129,
//!   json 0x0200} ‖ multihash: code varint ∈ {sha2-256 0x12, blake3 0x1e} ‖ length 32 ‖ 32 digest bytes,
//!   with no trailing bytes.
//!
//! Other multibases (z/base58 v1, base36 `k…`, upper-case `B…`) are refused, so each CID has one stored form.

pub const CID_MAX_LEN: usize = 128;
const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

#[derive(Debug, PartialEq, Eq)]
pub struct CidInfo {
    pub version: u8,
    pub codec: u64,
    pub hash_code: u64,
    pub digest: Vec<u8>,
}

fn b58_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes() {
        let v = B58
            .iter()
            .position(|&x| x == c)
            .ok_or_else(|| format!("invalid base58 char {:?}", c as char))? as u32;
        let mut carry = v;
        for b in out.iter_mut().rev() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    let mut v = vec![0u8; zeros];
    v.extend(out);
    Ok(v)
}

fn b32_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => {
                return Err(format!(
                    "invalid base32 char {:?} (lower-case a-z2-7 only)",
                    c as char
                ));
            }
        } as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || (acc & ((1 << bits) - 1)) != 0 {
        return Err("non-canonical base32 padding bits".into());
    }
    Ok(out)
}

fn varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let mut v: u64 = 0;
    for shift in (0..63).step_by(7) {
        let x = *b.get(*i).ok_or("truncated varint")?;
        *i += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err("varint too long".into())
}

pub fn validate_cid(s: &str) -> Result<CidInfo, String> {
    if s.is_empty() || s.len() > CID_MAX_LEN {
        return Err(format!("length {} not in 1..={CID_MAX_LEN}", s.len()));
    }
    if let Some(rest) = s.strip_prefix("Qm") {
        let _ = rest;
        if s.len() != 46 {
            return Err(format!("CIDv0 must be 46 chars, got {}", s.len()));
        }
        let b = b58_decode(s)?;
        if b.len() != 34 || b[0] != 0x12 || b[1] != 0x20 {
            return Err("CIDv0 must decode to a sha2-256 multihash (0x12 0x20 + 32 bytes)".into());
        }
        return Ok(CidInfo {
            version: 0,
            codec: 0x70,
            hash_code: 0x12,
            digest: b[2..].to_vec(),
        });
    }
    let body = s
        .strip_prefix('b')
        .ok_or("CID must be CIDv0 (Qm…) or CIDv1 base32 (b…)")?;
    if s.len() < 8 {
        return Err("CIDv1 too short".into());
    }
    let b = b32_decode(body)?;
    let mut i = 0;
    if varint(&b, &mut i)? != 1 {
        return Err("CIDv1 version byte must be 0x01".into());
    }
    let codec = varint(&b, &mut i)?;
    if ![0x55, 0x70, 0x71, 0x0129, 0x0200].contains(&codec) {
        return Err(format!(
            "unsupported codec 0x{codec:x} (raw, dag-pb, dag-cbor, dag-json, json)"
        ));
    }
    let hash_code = varint(&b, &mut i)?;
    if ![0x12, 0x1e].contains(&hash_code) {
        return Err(format!(
            "unsupported multihash 0x{hash_code:x} (sha2-256, blake3)"
        ));
    }
    let len = varint(&b, &mut i)? as usize;
    if len != 32 || b.len() != i + 32 {
        return Err("multihash digest must be 32 bytes with nothing after it".into());
    }
    Ok(CidInfo {
        version: 1,
        codec,
        hash_code,
        digest: b[i..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_cids() {
        // ipfs docs examples
        let v0 = validate_cid("QmY7Yh4UquoXHLPFo2XbhXkhBvFoPwmQUSa92pxnxjQuPU").unwrap();
        assert_eq!((v0.version, v0.hash_code, v0.digest.len()), (0, 0x12, 32));
        let v1 =
            validate_cid("bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").unwrap();
        assert_eq!((v1.version, v1.codec), (1, 0x70));
        let raw =
            validate_cid("bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku").unwrap();
        assert_eq!(raw.codec, 0x55);
    }
    #[test]
    fn rejects() {
        for bad in [
            "",
            "Qm123",
            "QmY7Yh4UquoXHLPFo2XbhXkhBvFoPwmQUSa92pxnxjQuP0",
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzd",
            "BAFYBEIGDYRZT5SFP7UDM7HU76UH7Y26NF3EFUYLQABF3OCLGTQY55FBZDI",
            "zdj7WWeQ43G6JJvLWQWZpyHuAMq6uYWRjkBXFad11vE2LHhQ7",
            "https://x.y",
            "bafy beig",
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdiaa",
        ] {
            assert!(validate_cid(bad).is_err(), "{bad}");
        }
        assert!(validate_cid(&format!("b{}", "a".repeat(200))).is_err());
    }
}
