use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

#[cw_serde]
pub struct AdminList {
    pub admins: Vec<Addr>,
    pub mutable: bool,
}

impl AdminList {
    pub fn is_admin(&self, addr: impl AsRef<str>) -> bool {
        let addr = addr.as_ref();
        self.admins.iter().any(|a| a.as_ref() == addr)
    }

    pub fn can_modify(&self, addr: &str) -> bool {
        self.mutable && self.is_admin(addr)
    }
}

pub const NATIVE_FEE_DENOM: &str = "uthiol";
pub const GENESIS_MINT_START_TIME: u64 = 17694490175915399;
pub const ADMIN_LIST: Item<AdminList> = Item::new("admin_list");
pub const MERKLE_ROOT: Item<String> = Item::new("merkle_root");
pub const MERKLE_TREE_URI: Item<String> = Item::new("merkle_tree_uri");
