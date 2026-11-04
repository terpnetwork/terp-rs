//! Canonical public-instance encoding for `proof_instance_verify`.
//!
//! Host and prover must agree on this layout. Domain-separated with
//! `SWAP_INSTANCE_LABEL` so bridge mint / headstash claim instances cannot be
//! replayed as swap statements.

use cosmwasm_std::Binary;
use sha2::{Digest, Sha256};

use crate::msg::SwapStatementPublic;
use crate::seams::SWAP_INSTANCE_LABEL;

/// Build instance bytes for host verify.
///
/// Layout (v0):
/// 1. 32-byte domain tag = SHA256(SWAP_INSTANCE_LABEL)
/// 2. fixed-width public fields (LE integers, length-prefixed vectors of 32-byte chunks)
///
/// Opaque witness is **not** included — only public statement.
pub fn encode_swap_instances(statement: &SwapStatementPublic) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    let tag = Sha256::digest(SWAP_INSTANCE_LABEL);
    out.extend_from_slice(&tag);

    out.extend_from_slice(&statement.pool_id.to_le_bytes());
    push_bytes32(&mut out, &statement.asset_in);
    push_bytes32(&mut out, &statement.asset_out);
    push_bytes32(&mut out, &statement.root);

    push_vec32(&mut out, &statement.nullifiers);
    push_vec32(&mut out, &statement.cm_out);

    out.extend_from_slice(&statement.delta_r_in.u128().to_le_bytes());
    out.extend_from_slice(&statement.delta_r_out.u128().to_le_bytes());
    out.extend_from_slice(&statement.min_out.u128().to_le_bytes());
    out.extend_from_slice(&statement.gamma.to_le_bytes());
    out.extend_from_slice(&statement.gamma_den.to_le_bytes());
    out.extend_from_slice(&statement.r_in_before.u128().to_le_bytes());
    out.extend_from_slice(&statement.r_out_before.u128().to_le_bytes());

    // Oracle optional: flag + fields
    match &statement.oracle_mid {
        None => out.push(0),
        Some(m) => {
            out.push(1);
            let pk = m.pair_key.as_bytes();
            out.extend_from_slice(&(pk.len() as u32).to_le_bytes());
            out.extend_from_slice(pk);
            out.extend_from_slice(&m.mid.u128().to_le_bytes());
            out.extend_from_slice(&m.observed_height.to_le_bytes());
        }
    }
    match &statement.oracle_params {
        None => out.push(0),
        Some(p) => {
            out.push(1);
            out.extend_from_slice(&p.max_age_blocks.to_le_bytes());
            out.extend_from_slice(&p.max_slippage_bps.to_le_bytes());
            out.push(if p.require_oracle { 1 } else { 0 });
        }
    }

    out
}

fn push_bytes32(out: &mut Vec<u8>, b: &Binary) {
    let slice = b.as_slice();
    let mut buf = [0u8; 32];
    let n = slice.len().min(32);
    buf[..n].copy_from_slice(&slice[..n]);
    out.extend_from_slice(&buf);
}

fn push_vec32(out: &mut Vec<u8>, items: &[Binary]) {
    out.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for item in items {
        push_bytes32(out, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::Uint128;

    fn sample() -> SwapStatementPublic {
        SwapStatementPublic {
            pool_id: 1,
            asset_in: Binary::from(b"HUB".as_slice()),
            asset_out: Binary::from(b"B".as_slice()),
            root: Binary::from([1u8; 32].as_slice()),
            nullifiers: vec![Binary::from([2u8; 32].as_slice())],
            cm_out: vec![Binary::from([3u8; 32].as_slice())],
            delta_r_in: Uint128::new(1000),
            delta_r_out: Uint128::new(1994),
            min_out: Uint128::new(1),
            gamma: 997,
            gamma_den: 1000,
            r_in_before: Uint128::new(1_000_000),
            r_out_before: Uint128::new(2_000_000),
            oracle_mid: None,
            oracle_params: None,
        }
    }

    #[test]
    fn encoding_deterministic() {
        let a = encode_swap_instances(&sample());
        let b = encode_swap_instances(&sample());
        assert_eq!(a, b);
        assert!(a.len() > 32);
    }
}
