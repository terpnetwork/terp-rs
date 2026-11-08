use cosmwasm_std::Addr;
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, Map, MultiIndex};

use crate::msg::{EligibilityRootRecord, HeadstashContract};

pub const HEADSTASH_CODE_ID: Item<u64> = Item::new("headstash_code_id");

/// Global manifold eligibility root registry: `(headstash_addr, root_id) → record`.
/// Supports additive multi-Headstash / multi-root composition (CLARITY §1).
pub const MANIFOLD_ROOTS: Map<(&Addr, u64), EligibilityRootRecord> = Map::new("manifold_roots");

// Indexes for tracking contracts
pub struct ContractIndexes<'a> {
    pub instantiator: MultiIndex<'a, Addr, HeadstashContract, Addr>,
}

impl<'a> IndexList<HeadstashContract> for ContractIndexes<'a> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<HeadstashContract>> + '_> {
        let v: Vec<&dyn Index<HeadstashContract>> = vec![&self.instantiator];
        Box::new(v.into_iter())
    }
}

pub fn contracts<'a>() -> IndexedMap<&'a Addr, HeadstashContract, ContractIndexes<'a>> {
    let indexes = ContractIndexes {
        instantiator: MultiIndex::new(
            |_pk: &[u8], d: &HeadstashContract| d.instantiator.clone(),
            "contracts",
            "contracts__instantiator",
        ),
    };
    IndexedMap::new("contracts", indexes)
}

/// Convenience helper for tests / queries: all roots under a headstash.
pub fn roots_for_headstash(
    storage: &dyn cosmwasm_std::Storage,
    headstash: &Addr,
) -> cosmwasm_std::StdResult<Vec<EligibilityRootRecord>> {
    MANIFOLD_ROOTS
        .prefix(headstash)
        .range(storage, None, None, cosmwasm_std::Order::Ascending)
        .map(|item| item.map(|(_, rec)| rec))
        .collect()
}
