use cosmwasm_schema::{cw_serde, QueryResponses};
use cw721_svg::InstantiateMsg as SvgInstantiateMsg;
use cw_ownable::cw_ownable_execute;

#[cw_serde]
pub struct InstantiateMsg {
    /// Optional owner. If set, only the owner may create collections.
    pub owner: Option<String>,
    /// Code ID of the cw721-svg contract to instantiate.
    pub svg_code_id: u64,
    /// If true, only DREGG runtime accounts / IBC packets may mint (no public native mint).
    #[serde(default)]
    pub dregg_ibc_only: bool,
    /// Terp addrs the DREGG runtime uses (OR with IBC packet receive).
    #[serde(default)]
    pub runtime_minters: Vec<String>,
}

#[cw_serde]
pub struct MigrateMsg {
    /// Bind an existing IBC channel to the DREGG federation (after handshake).
    pub dregg_channel: Option<String>,
    pub dregg_port: Option<String>,
    pub dregg_ibc_only: Option<bool>,
    pub runtime_minters: Option<Vec<String>>,
}

#[cw_ownable_execute]
#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Create a new cw721-svg collection.
    /// Uses instantiate2 for deterministic, anti-collision address prediction.
    CreateSvgCollection {
        instantiate_msg: SvgInstantiateMsg,
        label: String,
    },

    /// Owner-only: update the cw721-svg code ID used for new collections.
    UpdateCodeId { code_id: u64 },

    /// DREGG runtime submits a commitment packet (same bytes as IBC recv).
    RecvDreggPacket {
        packet: crate::ibc::MintPacket,
    },

    /// Bind IBC channel/port after handshake (also used by migrate).
    BindDreggChannel { channel: String, port: String },
}

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    /// Returns a paginated list of all svg collections.
    #[returns(Vec<crate::state::SvgCollection>)]
    ListSvgCollections {
        start_after: Option<String>,
        limit: Option<u32>,
    },

    /// Returns a paginated list of all svg collections in reverse.
    #[returns(Vec<crate::state::SvgCollection>)]
    ListSvgCollectionsReverse {
        start_before: Option<String>,
        limit: Option<u32>,
    },

    /// Returns a paginated list of svg collections created by a specific address.
    #[returns(Vec<crate::state::SvgCollection>)]
    ListSvgCollectionsByCreator {
        creator: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },

    /// Returns a paginated list of svg collections by creator in reverse.
    #[returns(Vec<crate::state::SvgCollection>)]
    ListSvgCollectionsByCreatorReverse {
        creator: String,
        start_before: Option<String>,
        limit: Option<u32>,
    },

    /// Returns ownership info.
    #[returns(::cw_ownable::Ownership<::cosmwasm_std::Addr>)]
    Ownership {},

    /// Returns the cw721-svg code ID currently used to instantiate collections.
    #[returns(::std::primitive::u64)]
    CodeId {},

    /// Bound DREGG IBC channel / runtime minters.
    #[returns(DreggIbcStatus)]
    DreggIbc {},
}

#[cw_serde]
pub struct DreggIbcStatus {
    pub channel: Option<String>,
    pub port: Option<String>,
    pub ibc_only: bool,
}
