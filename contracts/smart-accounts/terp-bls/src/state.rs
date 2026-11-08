use cosmwasm_std::Binary;
use cw_storage_plus::Item;

use crate::msg::BlsVariant;

pub const BLS_CONFIG: Item<BlsConfig> = Item::new("bls");

#[cosmwasm_schema::cw_serde]
pub struct BlsConfig {
    pub operator_keys: Vec<Binary>,
    pub threshold: u32,
    pub variant: BlsVariant,
}
