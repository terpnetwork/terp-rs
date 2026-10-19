use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Addr, Coin, Decimal, Uint128};
use cw_controllers::HooksResponse;

use cw_ownable::cw_ownable_execute;

use cosmwasm_std::{Binary, StdResult, to_json_binary};

use crate::{Ask, Bid, Bidder, Id, PendingBid, Seller, TokenId};

#[cosmwasm_schema::cw_serde]
pub struct Config {
    pub public_mint_start_time: cosmwasm_std::Timestamp,
}

#[cosmwasm_schema::cw_serde]
pub struct InstantiateMsg {
    /// Temporary admin for managing whitelists
    pub admin: Option<String>,
    /// Oracle for verifying text records
    pub verifier: Option<String>,
    /// Code-id for BS721-Account. On Instantiate, minter will instantiate a new account collection.
    pub collection_code_id: u64,
    /// Minimum length an account id can be
    pub min_account_length: u32,
    /// Maximum length an account id can be
    pub max_account_length: u32,
    /// Base price for a account. Used to calculate premium for small account accounts
    pub base_price: Uint128,
    /// Base delegated tokens for an account. Used to calculate minimum required to mint a name
    pub base_delegation: Uint128,
    /// # of seconds to delay allowing minting to occur from contract creation. Defaults to 1 second
    pub mint_start_delay: Option<u64>,
    /// Min value for a bid
    pub min_price: Uint128,
    /// Interval to rate limit setting asks (in seconds)
    pub ask_interval: u64,
    /// Community pool fee for winning bids
    /// 0.25% = 25, 0.5% = 50, 1% = 100, 2.5% = 250
    pub trading_fee_bps: u64,
    /// The number of bids to query to when searching for the highest bid
    pub valid_bid_query_limit: u32,
    /// Minimum time accepted bids are in escrow until they can be finalized.
    /// Improves security of account tokens. (in seconds)
    pub cooldown_timeframe: u64,
    /// Fee required by token owner to cancel a bid they have accepted. Split betweeen Terp developers & biddee.
    pub cooldown_cancel_fee: Coin,
    /// Admin
    pub hooks_admin: Option<String>,
}

#[cosmwasm_schema::cw_serde]
pub enum SudoMsg {
    UpdateParams {
        min_account_length: Option<u32>,
        max_account_length: Option<u32>,
        base_price: Option<Uint128>,
        base_delegation: Option<Uint128>,
        trading_fee_bps: Option<u64>,
        min_price: Option<Uint128>,
        ask_interval: Option<u64>,
        cooldown_duration: Option<u64>,
        cooldown_cancel_fee: Option<Coin>,
    },
    UpdateAccountCollection {
        collection: String,
    },
}

#[cw_ownable_execute]
#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum ExecuteMsg {
    /// Mint a account and list on Terp Account Marketplace
    MintAndList {
        account: String,
    },
    /// Admin can pause minting during whitelist switching
    Pause {
        pause: bool,
    },
    /// Update config, only callable by admin
    UpdateConfig {
        config: Config,
    },

    /// Remove account on the marketplace.
    /// Only the account collection can call this (i.e: when burned).
    RemoveAsk {
        token_id: TokenId,
    },
    /// Update ask when an NFT is transferred
    /// Only the account collection can call this
    UpdateAsk {
        token_id: TokenId,
        seller: String,
    },
    /// Place a bid on an existing ask
    SetBid {
        token_id: TokenId,
    },
    /// Remove an existing bid from an ask.
    /// If bid is in cooldown period & current token_id owner is calling, this will revert
    RemoveBid {
        token_id: TokenId,
    },
    //
    RemoveBids {
        token_id: TokenId,
    },
    // Flush any pending bids to be removed from a token-id. Anyone can call this
    CheckedRemoveBids {
        token_id: TokenId,
    },
    /// Accept a bid on an existing ask
    AcceptBid {
        token_id: TokenId,
        bidder: String,
    },
    /// Finalize a bid for an account once the delay period is complete.
    ///  Bidder or Bidee can call this function.
    FinalizeBid {
        token_id: TokenId,
    },
    /// Cancel a bid that has been accepted an is in the cooldown period.
    CancelCooldown {
        token_id: TokenId,
    },
    /// Add a new hook to be informed of all asks
    ManageHooks(ManageHooksAction),
}

#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
pub enum QueryMsg {
    #[returns(::cw_ownable::Ownership::<::cosmwasm_std::Addr>)]
    Ownership {},
    #[returns(Addr)]
    Collection {},
    #[returns(SudoParams)]
    Params {},
    #[returns(Config)]
    Config {},
    /// Get the current ask for specific name
    #[returns(Option<Ask>)]
    Ask { token_id: TokenId },
    /// Get all asks for a collection
    #[returns(Vec<Ask>)]
    Asks {
        start_after: Option<Id>,
        limit: Option<u32>,
    },
    /// Count of all asks
    #[returns(u64)]
    AskCount {},
    /// Get all asks by seller
    #[returns(Vec<Ask>)]
    AsksBySeller {
        seller: Seller,
        start_after: Option<TokenId>,
        limit: Option<u32>,
    },
    /// Get data for a specific bid
    #[returns(Option<Bid>)]
    Bid { token_id: TokenId, bidder: Bidder },
    /// Get all bids by a bidder
    #[returns(Vec<Bid>)]
    BidsByBidder {
        bidder: Bidder,
        start_after: Option<TokenId>,
        limit: Option<u32>,
    },
    /// Get all bids for a specific NFT
    #[returns(Vec<Bid>)]
    Bids {
        token_id: TokenId,
        start_after: Option<Bidder>,
        limit: Option<u32>,
    },
    /// Get all bids for a collection, sorted by price
    #[returns(Vec<Bid>)]
    BidsSortedByPrice {
        start_after: Option<BidOffset>,
        limit: Option<u32>,
    },
    /// Get all bids for a collection, sorted by price in reverse
    #[returns(Vec<Bid>)]
    ReverseBidsSortedByPrice {
        start_before: Option<BidOffset>,
        limit: Option<u32>,
    },
    /// Get all bids for a specific account
    #[returns(Vec<Bid>)]
    BidsForSeller {
        seller: String,
        start_after: Option<BidOffset>,
        limit: Option<u32>,
    },
    /// Get the highest bid for a name
    #[returns(Option<Bid>)]
    HighestBid { token_id: TokenId },
    /// Show all registered ask hooks
    #[returns(HooksResponse)]
    AskHooks {},
    /// Show all registered bid hooks
    #[returns(HooksResponse)]
    BidHooks {},
    /// Show all registered sale hooks
    #[returns(HooksResponse)]
    SaleHooks {},
    #[returns(Option<PendingBid>)]
    Cooldown { token_id: TokenId },
}

#[cosmwasm_schema::cw_serde]
pub struct MigrateMsg {}

#[cosmwasm_schema::cw_serde]
pub struct SudoParams {
    /// 3 (same as DNS)
    pub min_account_length: u32,
    /// 63 (same as DNS)
    pub max_account_length: u32,
    /// 100_000_000 (5+ ASCII char price)
    pub base_price: cosmwasm_std::Uint128,
    /// 100_000_000 (5+ ASCII char price)
    pub base_delegation: cosmwasm_std::Uint128,
    pub trading_fee_percent: Decimal,
    /// Min value for a bid
    pub min_price: Uint128,
    /// Interval to rate limit setting asks (in seconds)
    pub ask_interval: u64,
    /// The number of bids to query to when searching for the highest bid
    pub valid_bid_query_limit: u32,
    pub cooldown_duration: u64,
    pub cooldown_fee: Coin,
    pub hooks_admin: String,
}

#[cosmwasm_schema::cw_serde]
pub enum ManageHooksAction {
    RemoveAskHook(String),
    AddAskHook(String),
    AddBidHook(String),
    RemoveBidHook(String),
    AddSaleHook(String),
    RemoveSaleHook(String),
}

#[cosmwasm_schema::cw_serde]
pub struct ConfigResponse {
    pub minter: Addr,
    pub collection: Addr,
}

#[cosmwasm_schema::cw_serde]
pub struct AskRenewPriceResponse {
    pub token_id: TokenId,
    pub price: Coin,
    pub bid: Option<Bid>,
}

/// Offset for bid pagination
#[cosmwasm_schema::cw_serde]
pub struct BidOffset {
    pub price: Uint128,
    pub token_id: TokenId,
    pub bidder: Addr,
}

impl BidOffset {
    pub fn new(price: Uint128, token_id: TokenId, bidder: Addr) -> Self {
        BidOffset {
            price,
            token_id,
            bidder,
        }
    }
}

/// Offset for ask pagination
#[cosmwasm_schema::cw_serde]
pub struct AskOffset {
    pub price: Uint128,
    pub token_id: TokenId,
}

impl AskOffset {
    pub fn new(price: Uint128, token_id: TokenId) -> Self {
        AskOffset { price, token_id }
    }
}

#[cfg(feature = "market-hooks")]
pub mod hooks {
    use super::*;

    #[cosmwasm_schema::cw_serde]
    pub struct SaleHookMsg {
        pub token_id: String,
        pub ask_id: u32,
        pub seller: String,
        pub buyer: String,
    }

    impl SaleHookMsg {
        pub fn new(token_id: &str, ask_id: u32, seller: String, buyer: String) -> Self {
            SaleHookMsg {
                token_id: token_id.to_string(),
                ask_id,
                seller,
                buyer,
            }
        }

        /// serializes the message
        pub fn into_json_binary(self) -> StdResult<Binary> {
            let msg = SaleExecuteMsg::SaleHook(self);
            to_json_binary(&msg)
        }
    }

    // This is just a helper to properly serialize the above message
    #[cosmwasm_schema::cw_serde]
    pub enum SaleExecuteMsg {
        SaleHook(SaleHookMsg),
    }

    #[cosmwasm_schema::cw_serde]
    pub enum HookAction {
        Create,
        Update,
        Delete,
    }

    #[cosmwasm_schema::cw_serde]
    pub struct AskHookMsg {
        pub ask: Ask,
    }

    impl AskHookMsg {
        pub fn new(ask: Ask) -> Self {
            AskHookMsg { ask }
        }

        /// serializes the message
        pub fn into_json_binary(self, action: HookAction) -> StdResult<Binary> {
            let msg = match action {
                HookAction::Create => AskHookExecuteMsg::AskCreatedHook(self),
                HookAction::Update => AskHookExecuteMsg::AskUpdatedHook(self),
                HookAction::Delete => AskHookExecuteMsg::AskDeletedHook(self),
            };
            to_json_binary(&msg)
        }
    }

    // This is just a helper to properly serialize the above message
    #[cosmwasm_schema::cw_serde]
    pub enum AskHookExecuteMsg {
        AskCreatedHook(AskHookMsg),
        AskUpdatedHook(AskHookMsg),
        AskDeletedHook(AskHookMsg),
    }

    #[cosmwasm_schema::cw_serde]
    pub struct BidHookMsg {
        pub bid: Bid,
    }

    #[cosmwasm_schema::cw_serde]
    pub struct BidHookReply {
        pub token_id: String,
        pub account: String,
    }

    impl BidHookMsg {
        pub fn new(bid: Bid) -> Self {
            BidHookMsg { bid }
        }

        /// serializes the message
        pub fn into_json_binary(self, action: HookAction) -> cosmwasm_std::StdResult<Binary> {
            let msg = match action {
                HookAction::Create => BidHookExecuteMsg::BidCreatedHook(self),
                HookAction::Update => BidHookExecuteMsg::BidUpdatedHook(self),
                HookAction::Delete => BidHookExecuteMsg::BidDeletedHook(self),
            };
            to_json_binary(&msg)
        }
    }

    // This is just a helper to properly serialize the above message
    #[cosmwasm_schema::cw_serde]
    pub enum BidHookExecuteMsg {
        BidCreatedHook(BidHookMsg),
        BidUpdatedHook(BidHookMsg),
        BidDeletedHook(BidHookMsg),
    }
}
