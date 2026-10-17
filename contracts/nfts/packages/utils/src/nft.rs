use cosmwasm_std::{wasm_execute, CosmosMsg, Deps, Empty, Env, StdError, StdResult};
use cw721::msg::{Cw721ExecuteMsg, Cw721QueryMsg, OperatorResponse, OwnerOfResponse};

use crate::Cw721Token;

/// Why a cw721 pull (raffle escrow or loan settlement) was rejected.
#[derive(Debug, thiserror::Error)]
pub enum PullError {
    #[error("sender is not the cw721 owner")]
    NotOwner,
    #[error("cw721 is not approved for this contract")]
    NotApproved,
    #[error("{0}")]
    Query(#[from] StdError),
}

/// `operator` must be able to `TransferNft` this token out of `owner`.
///
/// Token approvals are read from `OwnerOf`. If that list does not include
/// `operator`, the cw721 `Operator` query is tried. Both messages are the
/// cw721 trait queries implemented by `cw721-svg`.
pub fn assert_can_pull(
    deps: Deps,
    env: &Env,
    owner: &str,
    token: &Cw721Token,
    operator: &str,
) -> Result<(), PullError> {
    let owned: OwnerOfResponse = deps.querier.query_wasm_smart(
        token.address.clone(),
        &Cw721QueryMsg::<Empty, Empty, Empty>::OwnerOf {
            token_id: token.token_id.clone(),
            include_expired: Some(true),
        },
    )?;
    if owned.owner != owner {
        return Err(PullError::NotOwner);
    }
    let token_ok = owned
        .approvals
        .iter()
        .any(|a| a.spender.as_str() == operator && !a.is_expired(&env.block));
    if token_ok {
        return Ok(());
    }
    let op: Result<OperatorResponse, StdError> = deps.querier.query_wasm_smart(
        token.address.clone(),
        &Cw721QueryMsg::<Empty, Empty, Empty>::Operator {
            owner: owner.to_string(),
            operator: operator.to_string(),
            include_expired: Some(true),
        },
    );
    match op {
        Ok(op) if !op.approval.is_expired(&env.block) => Ok(()),
        _ => Err(PullError::NotApproved),
    }
}

pub fn transfer_nft_msg(
    token: &Cw721Token,
    recipient: impl Into<String>,
) -> StdResult<CosmosMsg> {
    let msg = Cw721ExecuteMsg::<Empty, Empty, Empty>::TransferNft {
        recipient: recipient.into(),
        token_id: token.token_id.clone(),
    };
    Ok(wasm_execute(token.address.clone(), &msg, vec![])?.into())
}
