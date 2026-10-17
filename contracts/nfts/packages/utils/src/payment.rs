use cosmwasm_std::{Coin, Coins, StdError, StdResult, Uint256};

/// Remove `wanted` from `sent`. `wanted` must be one of `accepted` and covered
/// by the balance. Used for raffle creation fees and loan listing fees.
pub fn take_one_of(sent: &mut Coins, accepted: &[Coin]) -> StdResult<Coin> {
    let fee = accepted
        .iter()
        .find(|c| !c.amount.is_zero() && sent.amount_of(&c.denom) >= c.amount)
        .cloned()
        .ok_or_else(|| {
            StdError::msg(format!("missing fee, accepted {accepted:?}, sent {sent}"))
        })?;
    sent.sub(fee.clone())?;
    Ok(fee)
}

/// True when `sent` contains at least `amount` of `denom`.
pub fn covers(sent: &Coins, denom: &str, amount: Uint256) -> bool {
    sent.amount_of(denom) >= amount
}
