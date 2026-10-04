pub mod bundles;
pub mod create;
pub mod nfts;
pub mod state;
pub mod wavs;

use cosmwasm_std::{Binary, HexBinary};
extern crate cosmwasm_std;
pub type TokenId = String;
pub const MAX_TEXT_LENGTH: u32 = 512;
pub const NATIVE_DENOM: &str = "ubtsg";
pub const SECONDS_PER_YEAR: u64 = 31536000;
pub const SALT_POSTFIX: &[u8] = b"infusion";

/// Generates the value used with instantiate2, via a hash of the infusers checksum.
pub fn generate_instantiate_salt2(checksum: &HexBinary, height: u64) -> Binary {
    let mut hash = Vec::new();
    hash.extend_from_slice(checksum.as_slice());
    hash.extend_from_slice(&height.to_be_bytes());
    let checksum_hash = <sha2::Sha256 as sha2::Digest>::digest(hash);
    let mut result = checksum_hash.to_vec();
    result.extend_from_slice(SALT_POSTFIX);
    Binary::new(result)
}

#[cosmwasm_schema::cw_serde]
pub struct BurnParams {
    pub compatible_traits: Vec<CompatibleTraits>,
}

#[cosmwasm_schema::cw_serde]
pub struct CompatibleTraits {
    pub a: String,
    pub b: String,
}
