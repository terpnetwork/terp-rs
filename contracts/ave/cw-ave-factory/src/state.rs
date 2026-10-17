use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, MultiIndex};

/// Temporarily holds the address of the instantiator for use in submessages
pub const TMP_INSTANTIATOR_INFO: Item<Addr> = Item::new("tmp_instantiator_info");
pub const AVEVENT_CODE_ID: Item<u64> = Item::new("pci");

#[cw_serde]
pub struct AvEventContract {
    pub contract: String,
    pub instantiator: String,
}

pub struct TokenIndexes<'a> {
    pub instantiator: MultiIndex<'a, String, AvEventContract, String>,
}

impl IndexList<AvEventContract> for TokenIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<AvEventContract>> + '_> {
        let v: Vec<&dyn Index<AvEventContract>> = vec![&self.instantiator];
        Box::new(v.into_iter())
    }
}

pub fn avevent_contracts<'a>() -> IndexedMap<&'a str, AvEventContract, TokenIndexes<'a>> {
    let indexes = TokenIndexes {
        instantiator: MultiIndex::new(
            |_pk: &[u8], d: &AvEventContract| d.instantiator.clone(),
            "avevent_contracts",
            "avevent_contracts__instantiator",
        ),
    };
    IndexedMap::new("avevent_contracts", indexes)
}
