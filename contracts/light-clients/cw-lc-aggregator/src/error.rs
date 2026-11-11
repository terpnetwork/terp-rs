use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error(transparent)]
    Registry(#[from] cw_lc_registry::ContractError),
    #[error("epoch {0} is not the active committee epoch")]
    EpochNotActive(u64),
    #[error("invalid hex field `{0}`")]
    BadHex(String),
    #[error("committee signature does not verify")]
    BadSignature {},
    #[error("prev_hash does not link to the preconfirmed parent at {0}")]
    BrokenLink(u64),
    #[error("height {0} is already confirmed with a different hash")]
    ConflictsConfirmed(u64),
    #[error("no registry consensus tip")]
    NoTip {},
    #[error("registry tip {tip} is not {depth} blocks past {height}")]
    NotDeepEnough { height: u64, tip: u64, depth: u64 },
    #[error("preconfirmed chain does not reach the registry tip from {0}")]
    ChainMismatch(u64),
    #[error("confirm walk longer than {0}")]
    TooDeep(u32),
}
