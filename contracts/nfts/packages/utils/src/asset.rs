use cosmwasm_schema::cw_serde;
use cosmwasm_std::{coin, Api, Coin, StdError, StdResult, Uint256};

/// One cw721 token. `address` is stored in the form `Api::addr_validate` returns.
#[cw_serde]
pub struct Cw721Token {
    pub address: String,
    pub token_id: String,
}

/// Prize, collateral, or ticket price.
///
/// Upstream also had an `Sg721Token` arm. This port keeps cw721 only, which is
/// the trait `cw721-svg` implements.
#[cw_serde]
pub enum AssetInfo {
    Coin(Coin),
    Cw721(Cw721Token),
}

impl AssetInfo {
    pub fn coin(amount: u128, denom: &str) -> Self {
        Self::Coin(coin(amount, denom))
    }

    pub fn cw721(address: &str, token_id: &str) -> Self {
        Self::Cw721(Cw721Token {
            address: address.to_string(),
            token_id: token_id.to_string(),
        })
    }

    pub fn canonicalize(self, api: &dyn Api) -> StdResult<Self> {
        match self {
            Self::Coin(c) => {
                if c.amount.is_zero() || c.denom.is_empty() {
                    return Err(StdError::msg("coin amount and denom are required"));
                }
                Ok(Self::Coin(c))
            }
            Self::Cw721(token) => {
                if token.token_id.is_empty() {
                    return Err(StdError::msg("cw721 token id is empty"));
                }
                Ok(Self::Cw721(Cw721Token {
                    address: api.addr_validate(&token.address)?.into_string(),
                    token_id: token.token_id,
                }))
            }
        }
    }

    pub fn overlaps(&self, other: &AssetInfo) -> bool {
        match (self, other) {
            (Self::Coin(a), Self::Coin(b)) => a.denom == b.denom,
            (Self::Cw721(a), Self::Cw721(b)) => {
                a.address == b.address && a.token_id == b.token_id
            }
            _ => false,
        }
    }

    pub fn as_coin(&self) -> Option<&Coin> {
        match self {
            Self::Coin(c) => Some(c),
            Self::Cw721(_) => None,
        }
    }

    pub fn coin_amount(&self, denom: &str) -> Option<Uint256> {
        self.as_coin()
            .filter(|c| c.denom == denom)
            .map(|c| c.amount)
    }
}

pub fn all_unique(assets: &[AssetInfo]) -> bool {
    for i in 0..assets.len() {
        for other in assets.iter().skip(i + 1) {
            if assets[i].overlaps(other) {
                return false;
            }
        }
    }
    true
}

pub fn is_valid_name(name: &str) -> bool {
    (3..=50).contains(&name.len())
}

pub fn is_valid_comment(comment: &str) -> bool {
    comment.len() <= 20_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_overlap() {
        assert!(!is_valid_name("ab"));
        assert!(is_valid_name("raffle"));
        assert!(!is_valid_name(&"x".repeat(51)));
        let a = AssetInfo::cw721("c", "1");
        let b = AssetInfo::cw721("c", "1");
        let c = AssetInfo::cw721("c", "2");
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
        assert!(all_unique(&[a.clone(), c]));
        assert!(!all_unique(&[a, b]));
        assert!(AssetInfo::coin(1, "uterp").overlaps(&AssetInfo::coin(2, "uterp")));
    }
}
