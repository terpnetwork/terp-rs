use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;
use hm_loyalty_whitelist::WhitelistConfig;
use hm_nft_oracle::{OracleMid, PricePolicy};

#[cw_serde]
pub struct Config {
    pub cw721_address: Option<Addr>,
    pub max_tokens: u32,
    pub next_token_id: u32,
    pub token_uri_prefix: Option<String>,
    pub whitelist: WhitelistConfig,
    pub price: PricePolicy,
    pub proceeds: Addr,
    pub paused: bool,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const ORACLE_MID: Item<OracleMid> = Item::new("oracle_mid");
/// Last confirmed hash root metadata (for queries; not used for supply).
pub const LAST_ROOT_META: Item<RootMeta> = Item::new("last_root_meta");

#[cw_serde]
pub struct RootMeta {
    pub chain_uid: String,
    pub algo: String,
    pub height: u64,
    pub attestation_count: u32,
    pub block_time: i64,
}
