//! Message surface.
//!
//! Transparent XYK wiring is patterned on `crates/dex` (astroport-core fork)
//! pair msgs (`Swap`, `Simulation`, `Pool`, pause). The product path is
//! **private settle**: public statement + proof, not bank/CW20 offer transfer.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Uint128};

/// 32-byte Terp asset id (SEAM-NOTE-OUT width).
pub type AssetId32 = [u8; 32];

#[cw_serde]
pub struct InstantiateMsg {
    /// When true, proof verify is mocked (lab / ict film). Production: false + zk-api.
    pub mock_verify: bool,
    /// Circuit id registered in x/wasm for `proof_instance_verify`. Optional until circuit lands.
    pub zkid: Option<u64>,
    /// Optional single allowed commitment-tree root (stub window).
    pub allowed_root: Option<Binary>,
}

#[cw_serde]
pub enum PoolStatus {
    Active,
    Paused,
}

/// Virtual pool — public reserves only (private notes stay off transparent balances).
///
/// Parallel to astroport `PairInfo` + reserve balances, but assets are 32-byte
/// domain ids and reserves are virtual (not queried from bank).
#[cw_serde]
pub struct Pool {
    pub pool_id: u64,
    pub asset_a: Binary,
    pub asset_b: Binary,
    pub r_a: Uint128,
    pub r_b: Uint128,
    /// Fee numerator (e.g. 997 for 30 bps with den 1000).
    pub gamma: u64,
    pub gamma_den: u64,
    pub status: PoolStatus,
}

#[cw_serde]
pub struct OracleMid {
    pub pair_key: String,
    /// Fixed-point mid (scale = 1e18).
    pub mid: Uint128,
    pub observed_height: u64,
}

#[cw_serde]
pub struct OracleBoundParams {
    pub max_age_blocks: u64,
    pub max_slippage_bps: u32,
    pub require_oracle: bool,
}

/// Public statement for a v1 single-leg private swap.
///
/// Mirrors pure `SwapActionPublic` / SPEC §1.2. Witness stays client-side;
/// host recomputes curve and updates reserves after proof accept.
#[cw_serde]
pub struct SwapStatementPublic {
    pub pool_id: u64,
    pub asset_in: Binary,
    pub asset_out: Binary,
    /// Commitment tree root used for membership.
    pub root: Binary,
    /// Pool-spend nullifiers (32 bytes each).
    pub nullifiers: Vec<Binary>,
    /// New leaf commitments (note_out, optional change).
    pub cm_out: Vec<Binary>,
    /// Public Δ applied to reserves.
    pub delta_r_in: Uint128,
    pub delta_r_out: Uint128,
    pub min_out: Uint128,
    pub gamma: u64,
    pub gamma_den: u64,
    pub r_in_before: Uint128,
    pub r_out_before: Uint128,
    pub oracle_mid: Option<OracleMid>,
    pub oracle_params: Option<OracleBoundParams>,
}

#[cw_serde]
pub enum ExecuteMsg {
    // --- Owner admin (astroport-style config / pause surface) ---
    /// Create a virtual pool (factory-lite; multi-pool map like pair registry).
    CreatePool {
        asset_a: Binary,
        asset_b: Binary,
        r_a: Uint128,
        r_b: Uint128,
        gamma: u64,
        gamma_den: u64,
    },
    /// Pause / unpause — mirrors `crates/dex` pair `PoolPaused` window spirit.
    SetPoolStatus { pool_id: u64, status: PoolStatus },
    /// Dual-path verify config.
    SetZkCfg {
        mock_verify: bool,
        zkid: Option<u64>,
    },
    /// Stub root window for membership narrative.
    SetAllowedRoot { root: Option<Binary> },

    // --- Product path ---
    /// Settle a private swap: verify proof → host recompute → nullifiers + reserves.
    ///
    /// Transparent analog: `crates/dex` `ExecuteMsg::Swap { offer_asset, max_spread, ... }`
    /// but offer is a **proven note spend**, not a bank push.
    SettleSwap {
        statement: Box<SwapStatementPublic>,
        proof: Binary,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(Pool)]
    Pool { pool_id: u64 },

    #[returns(bool)]
    IsNullifierSpent { nullifier: Binary },

    /// Host recompute quote — mirrors astroport `QueryMsg::Simulation`.
    #[returns(QuoteResponse)]
    QuoteExactIn {
        pool_id: u64,
        asset_in: Binary,
        delta_in: Uint128,
    },
}

#[cw_serde]
pub struct ConfigResponse {
    pub owner: Option<String>,
    pub mock_verify: bool,
    pub zkid: Option<u64>,
    pub allowed_root: Option<Binary>,
    pub next_pool_id: u64,
    /// Guest feature flag present at compile time.
    pub zk_api_compiled: bool,
}

#[cw_serde]
pub struct QuoteResponse {
    pub pool_id: u64,
    pub asset_in: Binary,
    pub asset_out: Binary,
    pub delta_in: Uint128,
    pub delta_out: Uint128,
    pub r_in_before: Uint128,
    pub r_out_before: Uint128,
}
