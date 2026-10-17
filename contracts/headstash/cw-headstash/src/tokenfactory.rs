// src/tokenfactory.rs
use cosmwasm_std::{Addr, BankMsg, Binary, CosmosMsg, StdResult, Uint128, coins, to_json_binary};

// Hardcoded protobuf type URLs for tokenfactory messages
const MSG_MINT_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgMint";
const MSG_BURN_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgBurn";

// Simple metadata struct to avoid external dependencies
#[cosmwasm_schema::cw_serde]
pub struct Metadata {
    pub description: Option<String>,
    pub denom_units: Vec<DenomUnit>,
    pub base: Option<String>,
    pub display: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
}

#[cosmwasm_schema::cw_serde]
pub struct DenomUnit {
    pub denom: String,
    pub exponent: u32,
    pub aliases: Vec<String>,
}

// Helper functions for tokenfactory operations
fn create_msg_mint(sender: String, amount: Uint128, denom: String) -> StdResult<CosmosMsg> {
    #[derive(serde::Serialize)]
    struct Coin {
        denom: String,
        amount: String,
    }

    #[derive(serde::Serialize)]
    struct MsgMint {
        sender: String,
        amount: Coin,
        mint_to_address: String,
    }

    let coin = Coin {
        denom,
        amount: amount.to_string(),
    };

    let msg = MsgMint {
        sender: sender.clone(),
        amount: coin,
        mint_to_address: sender,
    };

    let value = to_json_binary(&msg)?;

    Ok(CosmosMsg::Stargate {
        type_url: MSG_MINT_TYPE_URL.to_string(),
        value,
    })
}

fn create_msg_burn(
    sender: String,
    amount: Uint128,
    denom: String,
    burn_from_address: String,
) -> StdResult<CosmosMsg> {
    #[derive(serde::Serialize)]
    struct Coin {
        denom: String,
        amount: String,
    }

    #[derive(serde::Serialize)]
    struct MsgBurn {
        sender: String,
        amount: Coin,
        burn_from_address: String,
    }

    let coin = Coin {
        denom,
        amount: amount.to_string(),
    };

    let msg = MsgBurn {
        sender,
        amount: coin,
        burn_from_address,
    };

    let value = to_json_binary(&msg)?;

    Ok(CosmosMsg::Stargate {
        type_url: MSG_BURN_TYPE_URL.to_string(),
        value,
    })
}

#[cosmwasm_schema::cw_serde]
pub enum TokenStrategy {
    /// Create new token via TokenFactory
    NewFungible(NewTokenConfig),
    /// Use existing denom (must pre-fund contract)
    ExistingFungible(HeadstashTokenObject),
}

#[cosmwasm_schema::cw_serde]
pub struct NewTokenConfig {
    pub subdenom: HeadstashTokenObject,
    pub metadata: Metadata,
    pub initial_mint: Option<Vec<InitialMint>>, // optional pre-mint
    pub manager: Option<String>,                // admin of denom
    pub minters: Vec<String>,                   // who can mint later
}

#[cosmwasm_schema::cw_serde]
pub struct HeadstashTokenObject {
    pub proof: Binary,
    pub raw: String,
}
#[cosmwasm_schema::cw_serde]
pub struct InitialMint {
    pub to_address: String,
    pub amount: Uint128,
}

pub fn derive_nd(raw: &str) -> Binary {
    // Same as `NoteDenom::new_for_proof`: blake3 then clear high bits of the
    // last byte so the 32-byte value is a canonical pallas::Base.
    let hash = blake3::hash(raw.as_bytes());
    let mut hash_bytes = *hash.as_bytes();
    hash_bytes[31] &= 0x1F;
    hash_bytes.into()
}
impl HeadstashTokenObject {
    pub fn new(raw: String) -> Self {
        Self {
            proof: derive_nd(&raw),
            raw,
        }
    }
}
impl TokenStrategy {
    /// ## TokenStrategy
    /// ### validates tokens proof representation is within spec defined for headstash proof inputs
    pub fn validate(&self) -> StdResult<()> {
        if !match self {
            TokenStrategy::NewFungible(cfg) => derive_nd(&cfg.subdenom.raw) == cfg.subdenom.proof,
            TokenStrategy::ExistingFungible(denom) => derive_nd(&denom.raw) == denom.proof,
        } {
            return Err(cosmwasm_std::StdError::msg("bad token params"));
        }
        Ok(())
    }

    /// ## proof_representation
    /// ### provides the value that this token is represented as in circuit
    pub fn proof_representation(&self) -> Binary {
        match self {
            TokenStrategy::NewFungible(cfg) => cfg.subdenom.proof.clone(),
            TokenStrategy::ExistingFungible(denom) => denom.proof.clone(),
        }
    }

    /// ## denom
    /// ### raw token denomination, formatted if it has been created as a new token
    pub fn denom(&self, contract_addr: &Addr) -> String {
        match self {
            TokenStrategy::NewFungible(cfg) => {
                format!("factory/{}/{}", contract_addr, cfg.subdenom.raw)
            }
            TokenStrategy::ExistingFungible(denom) => denom.raw.clone(),
        }
    }

    pub fn initial_mint_msgs(&self, contract_addr: &Addr) -> StdResult<Vec<CosmosMsg>> {
        match self {
            TokenStrategy::NewFungible(cfg) => {
                // For new fungible tokens, initial minting happens after denom creation
                // via the reply handler. For now, return empty vec since denom creation is async.
                Ok(vec![])
            }
            TokenStrategy::ExistingFungible(_) => Ok(vec![]),
        }
    }

    /// Mint tokens to an address (used for initial minting and other operations)
    pub fn mint_tokens(
        &self,
        contract_addr: &Addr,
        amount: Uint128,
        to_address: &str,
    ) -> StdResult<Vec<CosmosMsg>> {
        let denom = self.denom(contract_addr);

        let mint_msg = create_msg_mint(contract_addr.to_string(), amount, denom.clone())?;

        let send_msg = BankMsg::Send {
            to_address: to_address.to_string(),
            amount: coins(amount.u128(), denom),
        };

        Ok(vec![mint_msg, send_msg.into()])
    }

    /// Burn tokens from the contract (requires tokens to be owned by contract)
    pub fn burn_tokens(
        &self,
        contract_addr: &Addr,
        amount: Uint128,
        from_address: &str,
    ) -> StdResult<CosmosMsg> {
        let denom = self.denom(contract_addr);

        create_msg_burn(
            contract_addr.to_string(),
            amount,
            denom,
            from_address.to_string(),
        )
    }

    /// Get initial mint configuration for processing after denom creation
    pub fn get_initial_mints(&self) -> Vec<InitialMint> {
        match self {
            TokenStrategy::NewFungible(cfg) => cfg.initial_mint.clone().unwrap_or_default(),
            TokenStrategy::ExistingFungible(_) => vec![],
        }
    }

    pub fn requires_prefund(&self) -> bool {
        matches!(self, TokenStrategy::ExistingFungible(_))
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use cosmwasm_std::testing::mock_dependencies;
//     use cosmwasm_std::{Addr, Uint128};
//     use token_bindings::{DenomUnit, Metadata};

//     const CONTRACT_ADDR: &str = "cosmos2contractaddr1234567890abcdef";

//     fn mock_metadata() -> Metadata {
//         Metadata {
//             description: Some("Headstash Token".to_string()),
//             denom_units: vec![
//                 DenomUnit {
//                     denom: "uhead".to_string(),
//                     exponent: 0,
//                     aliases: vec![],
//                 },
//                 DenomUnit {
//                     denom: "HEAD".to_string(),
//                     exponent: 6,
//                     aliases: vec!["head".to_string()],
//                 },
//             ],
//             base: Some("uhead".to_string()),
//             display: Some("HEAD".to_string()),
//             name: Some("Headstash Token".to_string()),
//             symbol: Some("HEAD".to_string()),
//         }
//     }

//     #[test]
//     fn test_denom_new_fungible() {
//         let deps = mock_dependencies();
//         let contract = Addr::unchecked(CONTRACT_ADDR);

//         let strategy = TokenStrategy::NewFungible(NewTokenConfig {
//             subdenom: "head".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: None,
//             manager: None,
//             minters: vec![],
//         });

//         assert_eq!(
//             strategy.denom(&contract),
//             format!("factory/{}/head", CONTRACT_ADDR)
//         );
//     }

//     #[test]
//     fn test_denom_existing_fungible() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);
//         let strategy = TokenStrategy::ExistingFungible("cosmos1abc...xyz".to_string());

//         assert_eq!(strategy.denom(&contract), "cosmos1abc...xyz");
//     }

//     #[test]
//     fn test_create_denom_msg_new_fungible() {
//         let deps = mock_dependencies();
//         let contract = Addr::unchecked(CONTRACT_ADDR);

//         let strategy = TokenStrategy::NewFungible(NewTokenConfig {
//             subdenom: "head".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: None,
//             manager: None,
//             minters: vec![],
//         });

//         let msg = strategy.create_denom_msg(&contract).unwrap();
//         match msg {
//             TokenFactoryMsg::CreateDenom { subdenom, metadata } => {
//                 assert_eq!(subdenom, "head");
//                 assert_eq!(metadata.unwrap().name.unwrap(), "Headstash Token");
//             }
//             _ => panic!("Expected CreateDenom"),
//         }
//     }

//     #[test]
//     fn test_create_denom_msg_existing_returns_none() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);
//         let strategy = TokenStrategy::ExistingFungible("existing_denom".to_string());

//         assert!(strategy.create_denom_msg(&contract).is_none());
//     }

//     #[test]
//     fn test_initial_mint_msgs_with_mints() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);

//         let strategy = TokenStrategy::NewFungible(NewTokenConfig {
//             subdenom: "head".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: Some(vec![
//                 InitialMint {
//                     to_address: "alice".to_string(),
//                     amount: Uint128::new(1000),
//                 },
//                 InitialMint {
//                     to_address: "bob".to_string(),
//                     amount: Uint128::new(500),
//                 },
//             ]),
//             manager: None,
//             minters: vec![],
//         });

//         let msgs = strategy.initial_mint_msgs(&contract).unwrap();
//         assert_eq!(msgs.len(), 2);

//         let expected_denom = format!("factory/{}/head", CONTRACT_ADDR);

//         match &msgs[0] {
//             TokenFactoryMsg::MintTokens {
//                 denom,
//                 amount,
//                 mint_to_address,
//             } => {
//                 assert_eq!(denom, &expected_denom);
//                 assert_eq!(*amount, Uint128::new(1000));
//                 assert_eq!(mint_to_address, "alice");
//             }
//             _ => panic!("Expected MintTokens"),
//         }

//         match &msgs[1] {
//             TokenFactoryMsg::MintTokens {
//                 denom,
//                 amount,
//                 mint_to_address,
//             } => {
//                 assert_eq!(denom, &expected_denom);
//                 assert_eq!(*amount, Uint128::new(500));
//                 assert_eq!(mint_to_address, "bob");
//             }
//             _ => panic!("Expected MintTokens"),
//         }
//     }

//     #[test]
//     fn test_initial_mint_msgs_no_initial_mint() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);

//         let strategy = TokenStrategy::NewFungible(NewTokenConfig {
//             subdenom: "head".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: None,
//             manager: None,
//             minters: vec![],
//         });

//         let msgs = strategy.initial_mint_msgs(&contract).unwrap();
//         assert!(msgs.is_empty());
//     }

//     #[test]
//     fn test_initial_mint_msgs_existing_fungible_returns_empty() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);
//         let strategy = TokenStrategy::ExistingFungible("existing".to_string());

//         let msgs = strategy.initial_mint_msgs(&contract).unwrap();
//         assert!(msgs.is_empty());
//     }

//     #[test]
//     fn test_requires_prefund() {
//         let contract = Addr::unchecked(CONTRACT_ADDR);

//         let new_strategy = TokenStrategy::NewFungible(NewTokenConfig {
//             subdenom: "head".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: None,
//             manager: None,
//             minters: vec![],
//         });

//         let existing_strategy = TokenStrategy::ExistingFungible("existing".to_string());

//         assert!(!new_strategy.requires_prefund());
//         assert!(existing_strategy.requires_prefund());
//     }

//     #[test]
//     fn test_full_denom_resolution_consistency() {
//         let contract = Addr::unchecked("cosmos1qwerty");

//         let cfg = NewTokenConfig {
//             subdenom: "meme".to_string(),
//             metadata: mock_metadata(),
//             initial_mint: None,
//             manager: None,
//             minters: vec![],
//         };

//         let strategy = TokenStrategy::NewFungible(cfg);

//         let from_denom = strategy.denom(&contract);
//         let from_create = strategy.create_denom_msg(&contract).unwrap();

//         if let TokenFactoryMsg::CreateDenom { subdenom, .. } = from_create {
//             let expected = format!("factory/{}/{}", contract, subdenom);
//             assert_eq!(from_denom, expected);
//         } else {
//             panic!("Wrong msg type");
//         }
//     }
// }
