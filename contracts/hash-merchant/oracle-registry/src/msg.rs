use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Binary, Coin, Decimal, Uint128, Uint64};

use crate::state::SourceKind;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    /// Anyone may Submit when true (relayer race). Admin still records VE sudo.
    #[serde(default = "default_true")]
    pub open_submit: bool,
    /// Optional first-writer bounty paid from contract balance.
    pub bounty: Option<Coin>,
    /// Polytone note on this chain. When set, only that address may `Callback`.
    #[serde(default)]
    pub note: Option<String>,
}

fn default_true() -> bool {
    true
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Relayer or polytone callback posts an Osmosis (or other) bound.
    /// First observation at `(market_id, height)` wins and may take `bounty`.
    Submit {
        market_id: String,
        source: SourceKind,
        source_chain: String,
        pool_id: Option<u64>,
        height: u64,
        /// Decimal string, e.g. `"1.234567"`.
        mid: String,
    },
    /// Admin: allow or revoke a named relayer when `open_submit` is false.
    SetRelayer { addr: String, allowed: bool },
    /// Admin: fund later bounties (also acceptable as a bank send).
    SetBounty { bounty: Option<Coin> },
    /// Admin: bind the polytone note that may deliver callbacks.
    SetNote { note: String },
    /// Polytone note ACK. Wire tag `{ "callback": { ... } }` matches
    /// `polytone::callbacks::CallbackMessage`.
    Callback(PolytoneCallbackMessage),
}

/// Same JSON as polytone note → receiver execute.
#[cw_serde]
pub struct PolytoneCallbackMessage {
    pub initiator: Addr,
    pub initiator_msg: Binary,
    pub result: PolytoneCallback,
}

#[cw_serde]
pub enum PolytoneCallback {
    Query(PolytoneQueryResult),
    Execute(PolytoneExecResult),
    FatalError(String),
}

#[cw_serde]
pub enum PolytoneQueryResult {
    Success(Vec<Binary>),
    Error(PolytoneError),
}

#[cw_serde]
pub enum PolytoneExecResult {
    Success(PolytoneExecOk),
    Error(String),
}

#[cw_serde]
pub struct PolytoneError {
    pub message_index: Uint64,
    pub error: String,
}

#[cw_serde]
pub struct PolytoneExecOk {
    pub executed_by: String,
}

/// Packed in `initiator_msg` (JSON, then Binary).
#[cw_serde]
pub struct CallbackHint {
    pub market_id: String,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
}

/// x/hashmerchant sudo. `algo == "price_median_v1"` is a bound, not a state root.
#[cw_serde]
pub enum SudoMsg {
    HashMerchant {
        chain_uid: String,
        algo: String,
        root: String,
        height: u64,
        #[serde(default)]
        attestation_count: u32,
        #[serde(default)]
        block_time: i64,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(ObservationResponse)]
    Latest { market_id: String },
    #[returns(HistoryResponse)]
    History {
        market_id: String,
        /// Max rows, newest first. Default 32.
        limit: Option<u32>,
    },
    /// Height-weighted mean of mids in `[from_height, to_height]`.
    #[returns(TwapResponse)]
    Twap {
        market_id: String,
        from_height: u64,
        to_height: u64,
    },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: Addr,
    pub open_submit: bool,
    pub bounty: Option<Coin>,
    pub note: Option<Addr>,
}

#[cw_serde]
pub struct ObservationResponse {
    pub market_id: String,
    pub source: SourceKind,
    pub source_chain: String,
    pub pool_id: Option<u64>,
    pub height: u64,
    pub mid: Decimal,
    pub recorder: Addr,
}

#[cw_serde]
pub struct HistoryResponse {
    pub observations: Vec<ObservationResponse>,
}

#[cw_serde]
pub struct TwapResponse {
    pub market_id: String,
    pub from_height: u64,
    pub to_height: u64,
    pub n: u32,
    pub twap: Decimal,
    pub volume_hint: Uint128,
}
