//! hash-market capability quorum check (hm-vext `x/hashmerchant`).
//!
//! A capability is `(chain_uid, algo)`. hashmerchant only stores a
//! `HashRoot` for it once the capability's quorum policy
//! (`CapabilityQuorum`: attesting voting power ≥ max(threshold, gov floor),
//! `min_attesters`) is met, so the latest `HashRoot` *is* the on-chain quorum
//! verdict. The registry cross-checks the operators' k-of-n tip against it:
//!
//! * same height, different hash → `Conflicts` (report rejected, tip withheld)
//! * `require = true` and no root reachable → tip withheld (fail closed)
//!
//! Wire: gRPC query `/terp.hashmerchant.v1.Query/HashRoot`, protobuf encoded
//! by hand (two strings in, a handful of scalar fields out) to avoid a prost
//! dependency in the contract.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Deps};

pub const HASH_ROOT_PATH: &str = "/terp.hashmerchant.v1.Query/HashRoot";

#[cw_serde]
pub struct HashMarketCapability {
    /// hashmerchant `chain_uid`, e.g. `zcash-regtest`.
    pub chain_uid: String,
    /// hashmerchant `algo`, e.g. `equihash-200-9` / `block-hash`.
    pub algo: String,
    /// Fail closed: withhold `ConsensusTip` when no quorum root is reachable.
    pub require: bool,
}

/// Decoded `HashRoot` (attestations are skipped).
#[cw_serde]
pub struct QuorumRoot {
    pub height: u64,
    /// Root bytes as hex. For PoW header capabilities this is the block hash
    /// in display order, the same encoding as `NodeState::tip_hash`.
    pub root: String,
    pub attestation_count: u32,
    pub block_time: i64,
}

#[cw_serde]
pub enum QuorumVerdict {
    /// Quorum root at the tip height matches.
    Agrees,
    /// Quorum root at the tip height differs: tip withheld.
    Conflicts,
    /// Quorum root is below the tip (quorum lags; tip not contradicted).
    QuorumBehind,
    /// Quorum root is above the tip (operators lag the quorum).
    QuorumAhead,
    /// No root reachable (module absent, query not accepted, or no quorum yet).
    Unavailable,
}

#[cw_serde]
pub struct QuorumCheck {
    pub capability: HashMarketCapability,
    pub root: Option<QuorumRoot>,
    pub verdict: QuorumVerdict,
}

impl QuorumCheck {
    /// True when the tip must not be served.
    pub fn blocks_tip(&self) -> bool {
        matches!(self.verdict, QuorumVerdict::Conflicts)
            || (self.capability.require && matches!(self.verdict, QuorumVerdict::Unavailable))
    }
}

pub fn verdict(root: Option<&QuorumRoot>, height: u64, hash: &str) -> QuorumVerdict {
    match root {
        None => QuorumVerdict::Unavailable,
        Some(r) if r.height == height => {
            if r.root.eq_ignore_ascii_case(hash) {
                QuorumVerdict::Agrees
            } else {
                QuorumVerdict::Conflicts
            }
        }
        Some(r) if r.height < height => QuorumVerdict::QuorumBehind,
        Some(_) => QuorumVerdict::QuorumAhead,
    }
}

/// Latest quorum root for the capability, `None` if unreachable.
pub fn query_root(deps: Deps, cap: &HashMarketCapability) -> Option<QuorumRoot> {
    let req = encode_request(&cap.chain_uid, &cap.algo);
    let resp = deps
        .querier
        .query_grpc(HASH_ROOT_PATH.to_string(), Binary::from(req))
        .ok()?;
    decode_response(resp.as_slice())
}

// --- minimal protobuf ---------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    put_varint(out, ((field as u64) << 3) | 2);
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// `QueryHashRootRequest { chain_uid = 1, algo = 2 }`.
pub fn encode_request(chain_uid: &str, algo: &str) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, 1, chain_uid.as_bytes());
    put_bytes(&mut out, 2, algo.as_bytes());
    out
}

fn get_varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Some(v);
        }
    }
    None
}

/// Iterate (field, wire_type, varint value | byte slice).
fn fields(b: &[u8]) -> Option<Vec<(u32, u64, &[u8])>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let key = get_varint(b, &mut i)?;
        let (field, wt) = ((key >> 3) as u32, key & 7);
        match wt {
            0 => out.push((field, get_varint(b, &mut i)?, &b[0..0])),
            2 => {
                let len = get_varint(b, &mut i)? as usize;
                let end = i.checked_add(len).filter(|e| *e <= b.len())?;
                out.push((field, 0, &b[i..end]));
                i = end;
            }
            1 => i = i.checked_add(8).filter(|e| *e <= b.len())?,
            5 => i = i.checked_add(4).filter(|e| *e <= b.len())?,
            _ => return None,
        }
    }
    Some(out)
}

/// `QueryHashRootResponse { HashRoot root = 1 }` → [`QuorumRoot`].
pub fn decode_response(b: &[u8]) -> Option<QuorumRoot> {
    let outer = fields(b)?;
    let (_, _, inner) = outer.into_iter().find(|(f, _, _)| *f == 1)?;
    let mut r = QuorumRoot {
        height: 0,
        root: String::new(),
        attestation_count: 0,
        block_time: 0,
    };
    for (f, v, bytes) in fields(inner)? {
        match f {
            3 => r.height = v,
            4 => r.root = hex::encode(bytes),
            5 => r.attestation_count = v as u32,
            6 => r.block_time = v as i64,
            _ => {}
        }
    }
    // An empty/default HashRoot means "no quorum yet".
    if r.root.is_empty() || r.attestation_count == 0 {
        return None;
    }
    Some(r)
}

/// Test/helper encoder for a `QueryHashRootResponse`.
pub fn encode_response(chain_uid: &str, algo: &str, root: &QuorumRoot) -> Vec<u8> {
    let mut inner = Vec::new();
    put_bytes(&mut inner, 1, chain_uid.as_bytes());
    put_bytes(&mut inner, 2, algo.as_bytes());
    put_varint(&mut inner, 3 << 3);
    put_varint(&mut inner, root.height);
    put_bytes(&mut inner, 4, &hex::decode(&root.root).unwrap_or_default());
    put_varint(&mut inner, 5 << 3);
    put_varint(&mut inner, root.attestation_count as u64);
    put_varint(&mut inner, 6 << 3);
    put_varint(&mut inner, root.block_time as u64);
    let mut out = Vec::new();
    put_bytes(&mut out, 1, &inner);
    out
}
