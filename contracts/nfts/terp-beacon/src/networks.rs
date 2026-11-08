//! Public drand network parameters.
//!
//! Values come from the `/info` endpoint of each network. They are public
//! data, used by deploy scripts and the suite to default a beacon to quicknet.

use crate::msg::{DrandScheme, InstantiateMsg};
use cosmwasm_std::{Coin, HexBinary, Timestamp};

pub struct DrandNetwork {
    pub info_url: &'static str,
    pub chain_hash_hex: &'static str,
    pub pubkey_hex: &'static str,
    pub genesis_time: u64,
    pub period_seconds: u64,
    pub scheme: DrandScheme,
}

impl DrandNetwork {
    pub fn chain_hash(&self) -> HexBinary {
        HexBinary::from_hex(self.chain_hash_hex).expect("network chain hash is hex")
    }

    pub fn pubkey(&self) -> HexBinary {
        HexBinary::from_hex(self.pubkey_hex).expect("network pubkey is hex")
    }

    pub fn instantiate_msg(
        &self,
        admin: &str,
        prices: Vec<Coin>,
        required_attestations: u32,
    ) -> InstantiateMsg {
        InstantiateMsg {
            admin: admin.to_string(),
            chain_hash: self.chain_hash(),
            pubkey: self.pubkey(),
            genesis_time: Timestamp::from_seconds(self.genesis_time),
            period_seconds: self.period_seconds,
            scheme: self.scheme.clone(),
            required_attestations: Some(required_attestations),
            prices: Some(prices),
            callback_gas_limit: Some(crate::state::DEFAULT_CALLBACK_GAS_LIMIT),
        }
    }
}

/// League of Entropy mainnet "default", scheme pedersen-bls-chained.
pub const DEFAULT: DrandNetwork = DrandNetwork {
    info_url: "https://api.drand.sh/info",
    chain_hash_hex: "8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce",
    pubkey_hex: "868f005eb8e6e4ca0a47c8a77ceaa5309a47978a7c71bc5cce96366b5d7a569937c529eeda66c7293784a9402801af31",
    genesis_time: 1595431050,
    period_seconds: 30,
    scheme: DrandScheme::PedersenBlsChained,
};

/// Quicknet, scheme bls-unchained-g1-rfc9380.
pub const QUICKNET: DrandNetwork = DrandNetwork {
    info_url: "https://api.drand.sh/52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971/info",
    chain_hash_hex: "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971",
    pubkey_hex: "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a",
    genesis_time: 1692803367,
    period_seconds: 3,
    scheme: DrandScheme::BlsUnchainedG1Rfc9380,
};
