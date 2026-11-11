//! Canonical report digest. BLAKE3 via the wasmvm `blake3_256` host import.
use cosmwasm_std::{Addr, Api};

use crate::msg::NodeState;

pub const DOMAIN_NODE_STATE_V1: &[u8] = b"terp/lc/registry/node-state/v1";

/// Length-prefixed (u32 BE) field, shared by the lc contract family.
pub fn put_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}

/// Preimage: domain ‖ chain_id ‖ contract ‖ client_id ‖ operator ‖ state fields,
/// each variable-length field length-prefixed (u32 BE).
pub fn report_preimage(
    chain_id: &str,
    contract: &Addr,
    client_id: &str,
    operator: &Addr,
    s: &NodeState,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    put_bytes(&mut buf, DOMAIN_NODE_STATE_V1);
    put_bytes(&mut buf, chain_id.as_bytes());
    put_bytes(&mut buf, contract.as_bytes());
    put_bytes(&mut buf, client_id.as_bytes());
    put_bytes(&mut buf, operator.as_bytes());
    buf.extend_from_slice(&s.tip_height.to_be_bytes());
    put_bytes(&mut buf, s.tip_hash.to_ascii_lowercase().as_bytes());
    put_bytes(&mut buf, s.prev_hash.to_ascii_lowercase().as_bytes());
    put_bytes(&mut buf, s.chain_work.to_ascii_lowercase().as_bytes());
    match s.finalized_height {
        Some(h) => {
            buf.push(1);
            buf.extend_from_slice(&h.to_be_bytes());
        }
        None => buf.push(0),
    }
    buf.extend_from_slice(&s.peer_count.to_be_bytes());
    buf.extend_from_slice(&s.observed_at_unix.to_be_bytes());
    buf
}

pub fn report_digest(
    api: &dyn Api,
    chain_id: &str,
    contract: &Addr,
    client_id: &str,
    operator: &Addr,
    s: &NodeState,
) -> [u8; 32] {
    api.blake3_256(&report_preimage(chain_id, contract, client_id, operator, s))
}
