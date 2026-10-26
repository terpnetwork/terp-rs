use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, Map, MultiIndex};

/// The code ID of the cw721-svg contract to instantiate.
pub const SVG_CODE_ID: Item<u64> = Item::new("svg_code_id");

/// Global nonce incremented per collection to ensure unique instantiate2 salts.
pub const COLLECTION_NONCE: Item<u64> = Item::new("collection_nonce");

/// Bound IBC channel to the DREGG federation client (unordered `svgmint-dregg-v1`).
pub const DREGG_CHANNEL: Item<String> = Item::new("dregg_channel");
pub const DREGG_PORT: Item<String> = Item::new("dregg_port");
/// Terp accounts the DREGG runtime uses to mint. Everyone else is refused unless IBC.
pub const RUNTIME_MINTERS: Map<&Addr, ()> = Map::new("runtime_minters");
pub const DREGG_IBC_ONLY: Item<bool> = Item::new("dregg_ibc_only");
/// Default oracle pricing applied to every new collection (absent = none).
pub const ORACLE_PRICING: Item<terp_oracle::OraclePricing> = Item::new("oracle_pricing");
pub const SPENT_NULLIFIERS: Map<String, ()> = Map::new("spent_nullifiers");

/// Contract-enforced creation cadence for a named series (e.g. "latent-benders", "marain-orbs").
/// Every field is optional; an unset field is not enforced.
#[cw_serde]
#[derive(Default)]
pub struct SeriesConfig {
    /// Who may create in this series besides the factory owner. Empty = anyone.
    #[serde(default)]
    pub operators: Vec<String>,
    /// Minimum seconds between two collections of this series.
    pub min_interval_secs: Option<u64>,
    /// At most this many collections per rolling `window_secs`.
    pub max_per_window: Option<u32>,
    /// Rolling window for `max_per_window`; default 86400 (one day).
    pub window_secs: Option<u64>,
    /// If set, each collection must set mint_end_time (seconds after creation) and it must be
    /// at most this value.
    pub max_mint_window_secs: Option<u64>,
}

pub const DEFAULT_SERIES_WINDOW_SECS: u64 = 86_400;
/// Series configs, set by the factory owner.
pub const SERIES: Map<&str, SeriesConfig> = Map::new("series");
/// Creation times (unix secs, oldest first) of recent collections per series; pruned on write.
pub const SERIES_RECENT: Map<&str, Vec<u64>> = Map::new("series_recent");

#[cw_serde]
pub struct SvgCollection {
    /// The instantiated collection contract address (primary key).
    pub contract: String,
    /// Address that created this collection.
    pub creator: String,
    /// Collection name from InstantiateMsg.
    pub name: String,
    /// Collection symbol from InstantiateMsg.
    pub symbol: String,
    /// Series this collection was created under (CreateSeriesCollection), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
}

pub struct CollectionIndexes<'a> {
    pub creator: MultiIndex<'a, String, SvgCollection, String>,
}

impl IndexList<SvgCollection> for CollectionIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<SvgCollection>> + '_> {
        let v: Vec<&dyn Index<SvgCollection>> = vec![&self.creator];
        Box::new(v.into_iter())
    }
}

pub fn svg_collections<'a>() -> IndexedMap<&'a str, SvgCollection, CollectionIndexes<'a>> {
    let indexes = CollectionIndexes {
        creator: MultiIndex::new(
            |_pk: &[u8], d: &SvgCollection| d.creator.clone(),
            "svg_collections",
            "svg_collections__creator",
        ),
    };
    IndexedMap::new("svg_collections", indexes)
}
