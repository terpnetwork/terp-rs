use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error(transparent)]
    Registry(#[from] cw_lc_registry::ContractError),
    #[error("no preconfirmed header at height {0}")]
    NoHeader(u64),
    #[error("header at {0} carries no payment root")]
    NoPaymentRoot(u64),
    #[error("header at {height} is {status}, client requires Confirmed")]
    NotFinal { height: u64, status: String },
    #[error("packet dest_client `{got}` is not this client `{want}`")]
    WrongDest { got: String, want: String },
    #[error("invalid hex field `{0}`")]
    BadHex(String),
    #[error("zap-1 inclusion failed: {0}")]
    Inclusion(String),
    #[error("packet ({0}, {1}) already used")]
    Replay(String, u64),
}
