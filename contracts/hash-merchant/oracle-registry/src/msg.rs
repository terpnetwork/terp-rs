use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Addr, Binary, Coin, Decimal, Uint64, Uint128};

use crate::state::SourceKind;

/// IBC v2 destination port for a posted-liquidity quote.
pub const DEX_QUOTE_PORT: &str = "dex-quote";
/// Payload version. Counterparties reject anything else.
pub const DEX_QUOTE_VERSION: &str = "dex-quote-1";
pub const DEX_QUOTE_ENCODING: &str = "application/json";
/// Matches Penumbra `MAX_DEX_QUOTES`. One packet carries at most this many quotes.
pub const DEX_QUOTE_BATCH_MAX: usize = 16;

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
    /// Optional IBC v2 client. `RegisterIbcV2` sets this after instantiate.
    #[serde(default)]
    pub ibc2_source_client: Option<String>,
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
    /// Admin: bind this chain's IBC v2 client. Destination port is `dex-quote`.
    RegisterIbcV2 { source_client: String },
    /// Ask the dex-quote counterparty for posted liquidity. The ack files the mid.
    RequestQuote {
        market_id: String,
        pair: DexPair,
        /// Empty selects the counterparty's best opened position on `pair`.
        #[serde(default)]
        position_id: String,
    },
    /// Several quotes in one IBC v2 packet. Each success files its own observation.
    RequestQuotes { quotes: Vec<DexQuoteRequestItem> },
    /// Polytone note ACK. Wire tag `{ "callback": { ... } }` matches
    /// `polytone::callbacks::CallbackMessage`.
    Callback(PolytoneCallbackMessage),
}

/// Trading pair inside a dex-quote request and ack. Asset ids are counterparty strings.
#[cw_serde]
pub struct DexPair {
    pub asset_1: String,
    pub asset_2: String,
}

/// Reserves sitting on the quoted position. Integers, as decimal strings.
#[cw_serde]
pub struct DexReserves {
    pub r1: String,
    pub r2: String,
}

/// One row of `RequestQuotes`. Height is the block that sends the packet.
#[cw_serde]
pub struct DexQuoteRequestItem {
    pub market_id: String,
    pub pair: DexPair,
    #[serde(default)]
    pub position_id: String,
}

/// Payload `value` on port `dex-quote`.
#[cw_serde]
pub struct DexQuoteRequest {
    pub market_id: String,
    /// Height this chain will file the observation under.
    pub height: u64,
    pub pair: DexPair,
    #[serde(default)]
    pub position_id: String,
}

/// Several [`DexQuoteRequest`]s in one payload.
#[cw_serde]
pub struct DexQuoteBatch {
    pub quotes: Vec<DexQuoteRequest>,
}

/// Success acknowledgement. `p` and `q` are base-unit coefficients of the
/// opened position, not a batch-swap fill and not a display price.
/// `Latest.mid` is `p/q`. For `5penumbra` against 100 base `uterp` that is
/// `p = 500000000`, `q = 100`, `mid = 5000000` (5 penumbra per 1 base uterp).
/// The ack is the counterparty chain's consensus result, not a reserve proof.
#[cw_serde]
pub struct DexQuoteAck {
    pub position_id: String,
    pub pair: DexPair,
    pub reserves: DexReserves,
    pub p: String,
    pub q: String,
}

/// One row of a batch acknowledgement. `error` set means that row is not a price.
#[cw_serde]
pub struct DexQuoteBatchItem {
    pub market_id: String,
    pub height: u64,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub position_id: String,
    #[serde(default)]
    pub pair: Option<DexPair>,
    #[serde(default)]
    pub reserves: Option<DexReserves>,
    #[serde(default)]
    pub p: String,
    #[serde(default)]
    pub q: String,
}

#[cw_serde]
pub struct DexQuoteBatchAck {
    pub quotes: Vec<DexQuoteBatchItem>,
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
    pub ibc2_source_client: Option<String>,
    pub ibc2_dest_port: Option<String>,
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
    #[serde(default)]
    pub position_id: Option<String>,
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

/// Migrate message. Intentionally empty: migrate is a plain capability entrypoint.
#[cw_serde]
pub struct MigrateMsg {}
