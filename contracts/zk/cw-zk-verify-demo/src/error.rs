use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("proof_instance_verify failed: {reason}")]
    VerifyFailed { reason: String },
    #[error("zk-host feature required for Path A verify")]
    ZkHostRequired,
}
