//! Wire types shared with nois-proxy and nois receivers.
//!
//! These mirror the JSON of the `nois` crate (`NoisCallback`,
//! `ReceiverExecuteMsg`, `ProxyExecuteMsg`, `MAX_JOB_ID_LEN`) so a contract
//! written against nois-proxy can point at this beacon unchanged. The `nois`
//! crate itself targets CosmWasm 1.x and is not a dependency.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{HexBinary, Timestamp};

/// Maximum job id length accepted by nois-proxy.
pub const MAX_JOB_ID_LEN: usize = 64;

/// Callback payload delivered to the requester.
#[cw_serde]
pub struct NoisCallback {
    pub job_id: String,
    /// Drand round publish time (not the chain block time).
    pub published: Timestamp,
    /// 32 bytes of randomness, `sha256(drand signature)`.
    pub randomness: HexBinary,
}

/// Message a receiver contract must accept.
#[cw_serde]
pub enum ReceiverExecuteMsg {
    NoisReceive { callback: NoisCallback },
}

/// The two nois-proxy request messages. Same JSON as `nois::ProxyExecuteMsg`.
#[cw_serde]
pub enum ProxyExecuteMsg {
    GetNextRandomness { job_id: String },
    GetRandomnessAfter { after: Timestamp, job_id: String },
}
