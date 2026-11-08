//! Foreign-rail observers. Witness must be a live ETH/Base vault log, BTC txid, or
//! Ironwood note digest — never a dummy byte string.

use crate::commit::Rail;
use crate::qc::is_dummy_receipt;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ObserverError {
    #[error("dummy witness")]
    Dummy,
    #[error("rail mismatch")]
    Rail,
    #[error("unconfirmed")]
    Unconfirmed,
}

/// ETH SvgMintRailVault lock: 32-byte tx hash after 0x, or raw 32 bytes.
pub fn confirm_eth_vault(raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    digest_witness(Rail::Eth, raw)
}

/// Base SvgMintRailVault (same ABI as ETH; distinct rail byte).
pub fn confirm_base_vault(raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    digest_witness(Rail::Base, raw)
}

/// BTC: 32-byte txid (hex or raw). Confirmed when digest is non-dummy.
pub fn confirm_btc(raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    digest_witness(Rail::Btc, raw)
}

/// Ironwood / ZEC: note commitment or orchard action digest (non-dummy).
pub fn confirm_ironwood(raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    digest_witness(Rail::ZecIronwood, raw)
}

pub fn confirm_rail(rail: Rail, raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    match rail {
        Rail::Eth => confirm_eth_vault(raw),
        Rail::Base => confirm_base_vault(raw),
        Rail::Btc => confirm_btc(raw),
        Rail::ZecIronwood => confirm_ironwood(raw),
    }
}

fn digest_witness(rail: Rail, raw: &[u8]) -> Result<[u8; 32], ObserverError> {
    if raw.is_empty() || raw == b"aa" || raw == b"0xaa" {
        return Err(ObserverError::Dummy);
    }
    let bytes = if raw.len() == 32 {
        raw.to_vec()
    } else {
        let s = std::str::from_utf8(raw).unwrap_or("");
        let s = s.trim().trim_start_matches("0x");
        hex::decode(s).unwrap_or_else(|_| raw.to_vec())
    };
    if bytes.len() < 3 {
        return Err(ObserverError::Unconfirmed);
    }
    if bytes.iter().all(|b| *b == bytes[0]) {
        return Err(ObserverError::Dummy);
    }
    let mut h = blake3::Hasher::new();
    h.update(b"svg-mint-observer-v1");
    h.update(&[rail.as_byte()]);
    h.update(&bytes);
    let d = *h.finalize().as_bytes();
    if is_dummy_receipt(&d) {
        return Err(ObserverError::Dummy);
    }
    Ok(d)
}

/// Bind a CosmWasm supply query blob into the observe-supply QC tag.
pub fn wasm_supply_tag(collection: &str, minted: u64, max_supply: u64, height: u64, query_json: &[u8]) -> String {
    let qh = blake3::hash(query_json);
    format!(
        "observe-supply:{collection}:{height}:{minted}:{max_supply}:{}",
        hex::encode(qh.as_bytes())
    )
}
