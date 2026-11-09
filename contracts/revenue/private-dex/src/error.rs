use cosmwasm_std::StdError;
use thiserror::Error;

/// Contract errors — names align with pure `terp_seams::dex` where useful.
#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error(transparent)]
    Ownable(#[from] cw_ownable::OwnershipError),

    #[error("unauthorized")]
    Unauthorized {},

    #[error("pool paused or missing")]
    PoolPaused {},

    #[error("pool already exists")]
    PoolExists {},

    #[error("wrong asset orientation for pool")]
    WrongAsset {},

    #[error("bad amount / fee / schema")]
    BadAmount {},

    #[error("min_out not met")]
    MinOut {},

    #[error("insufficient reserve")]
    InsufficientReserve {},

    #[error("nullifier already spent")]
    NullifierExists {},

    #[error("nullifier domain: pool-spend ν must not equal ingress lineage")]
    NullifierDomain {},

    #[error("curve mismatch: host Δ_out ≠ statement")]
    CurveMismatch {},

    #[error("bad commitment root")]
    BadRoot {},

    #[error("oracle mid missing while required")]
    OracleMissing {},

    #[error("oracle mid stale")]
    OracleStale {},

    #[error("oracle slippage exceeded")]
    OracleSlippage {},

    #[error("oracle cannot mint balances or bump reserves alone")]
    OracleDisabledMint {},

    #[error("proof verify rejected: {0}")]
    ProofRejected(String),

    #[error("proof_instance_verify requires zk-api feature + zk wasmvm (set mock_verify for lab)")]
    ZkApiUnavailable {},

    #[error("zkid not configured")]
    ZkidMissing {},
}
