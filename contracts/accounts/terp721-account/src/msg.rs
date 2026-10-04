use crate::{state::SudoParams, Metadata};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Binary, CustomMsg, Empty};
use cw721::Expiration;
use terp_account::verify_generic::CosmosArbitrary;
use terp_account::{TextRecord, NFT};

use cw721::msg::Cw721InstantiateMsg;
use cw721::msg::{
    AllNftInfoResponse, ApprovalResponse, Cw721ExecuteMsg, Cw721QueryMsg, NftInfoResponse,
    NumTokensResponse, OperatorResponse, OwnerOfResponse, TokensResponse,
};
use cw721::{DefaultOptionalCollectionExtension, DefaultOptionalCollectionExtensionMsg};
use cw_ownable::Ownership;

pub type Terp721InstantiateMsg = Cw721InstantiateMsg<DefaultOptionalCollectionExtensionMsg>;

#[cw_serde]
pub struct InstantiateMsg {
    pub verifier: Option<String>,
    pub base_init_msg: Terp721InstantiateMsg,
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))] // cw-orch automatic
pub enum ExecuteMsg<T> {
    /// Set account marketplace contract address
    SetMarketplace { address: String },
    /// Set an address for account reverse lookup and updates token_uri
    /// Can be an EOA or a contract address.
    AssociateAddress {
        // namespace of the account token (token-id)
        account: String,
        // address to set to reverse map.  Set to None to remove
        address: Option<String>,
    },
    /// Update image NFT
    UpdateImageNft { account: String, nft: Option<NFT> },
    /// Add text record ex: abstract account, twitter handle, discord account, etc
    AddTextRecord { account: String, record: TextRecord },
    /// Remove text record ex: twitter handle, discord account, etc
    RemoveTextRecord {
        account: String,
        record_account: String,
    },
    /// Update text record ex: twitter handle, discord account, etc
    UpdateTextRecord { account: String, record: TextRecord },
    /// Verify a text record as true or false (via oracle)
    VerifyTextRecord {
        account: String,
        record_account: String,
        result: bool,
    },
    /// Update the reset the verification oracle
    UpdateVerifier { verifier: Option<String> },
    /// Transfer is a base message to move a token to another account without triggering actions
    TransferNft { recipient: String, token_id: String },
    /// Send is a base message to transfer a token to a contract and trigger an action
    /// on the receiving contract.
    SendNft {
        contract: String,
        token_id: String,
        msg: Binary,
    },
    /// Allows operator to transfer / send the token from the owner's account.
    /// If expiration is set, then this allowance has a time/height limit
    Approve {
        spender: String,
        token_id: String,
        expires: Option<Expiration>,
    },
    /// Remove previously granted Approval
    Revoke { spender: String, token_id: String },
    /// Marketplace makes use to manually approve transfers.
    /// Used if account owner has removed approval for marketplace as operator during cooldown period
    ApproveAllViaMarket {
        owner: String,
        expires: Option<Expiration>,
    },
    /// Allows operator to transfer / send any token from the owner's account.
    /// If expiration is set, then this allowance has a time/height limit
    ApproveAll {
        operator: String,
        expires: Option<Expiration>,
    },
    /// Allows user to define if token is being associated to an Abstract Account.
    #[cfg(feature = "abstract")]
    UpdateAbsAccSupport {
        token_id: String,
        r#abstract: Option<String>,
    },
    /// Remove previously granted ApproveAll permission
    RevokeAll { operator: String },
    /// Mint a new NFT, can only be called by the contract minter
    Mint {
        /// Unique ID of the NFT
        token_id: String,
        /// The owner of the newly minted NFT
        owner: String,
        /// This will be a smart contract address that is an EOA using this token as ownership authentication
        token_uri: Option<String>,
        /// Any custom extension used by this contract
        extension: T,
    },
    /// Burn an NFT the sender has access to
    Burn { token_id: String },
    /// Freeze collection info from further updates
    FreezeCollectionInfo {},
    /// Updates the mapping of wallet accounts to the sender.
    UpdateMyReverseMapKey {
        to_add: Vec<CosmosArbitrary>,
        to_remove: Vec<String>,
    },
}

impl<T> From<ExecuteMsg<T>> for Cw721ExecuteMsg<T, DefaultOptionalCollectionExtensionMsg, Empty> {
    fn from(
        msg: ExecuteMsg<T>,
    ) -> Cw721ExecuteMsg<T, DefaultOptionalCollectionExtensionMsg, Empty> {
        match msg {
            ExecuteMsg::TransferNft {
                recipient,
                token_id,
            } => Cw721ExecuteMsg::TransferNft {
                recipient,
                token_id,
            },
            ExecuteMsg::SendNft {
                contract,
                token_id,
                msg,
            } => Cw721ExecuteMsg::SendNft {
                contract,
                token_id,
                msg,
            },
            ExecuteMsg::Approve {
                spender,
                token_id,
                expires,
            } => Cw721ExecuteMsg::Approve {
                spender,
                token_id,
                expires,
            },
            ExecuteMsg::ApproveAll { operator, expires } => {
                Cw721ExecuteMsg::ApproveAll { operator, expires }
            }
            ExecuteMsg::Revoke { spender, token_id } => {
                Cw721ExecuteMsg::Revoke { spender, token_id }
            }
            ExecuteMsg::RevokeAll { operator } => Cw721ExecuteMsg::RevokeAll { operator },
            ExecuteMsg::Burn { token_id } => Cw721ExecuteMsg::Burn { token_id },
            ExecuteMsg::Mint {
                token_id,
                owner,
                token_uri,
                extension,
            } => Cw721ExecuteMsg::Mint {
                token_id,
                owner,
                token_uri,
                extension,
            },
            _ => unreachable!("Invalid ExecuteMsg"),
        }
    }
}

impl CustomMsg for Terp721AccountsQueryMsg {}
impl cw721::traits::Cw721CustomMsg for Terp721AccountsQueryMsg {}

#[cw_ownable::cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))] // cw-orch automatic
pub enum Terp721AccountsQueryMsg {
    /// Returns sudo params
    #[returns(SudoParams)]
    Params {},
    /// Query an address to return the account owned by this address
    #[returns(String)]
    Account { address: String },
    /// Query an account name to return the associated address.
    /// If being used as ownership token for EOA, will return the EOA contract, otherwise returns the owner of the token
    #[returns(Addr)]
    AssociatedAddress { account: String },
    /// Query a non `terp1...` address to retrieve the `terp1...` associated with it
    #[returns(Addr)]
    ReverseMapAddress { address: String },
    /// Query a non `terp1...` address to retrieve the account token associated with it. *Same as `QueryMsg::Account`*
    #[returns(String)]
    ReverseMapAccount { address: String },
    /// Returns the image NFT for a account
    #[returns(Option<NFT>)]
    ImageNFT { account: String },
    /// Returns the text records for a account
    #[returns(Vec<TextRecord>)]
    TextRecords { account: String },
    /// Returns the verification oracle address
    #[returns(Option<String>)]
    Verifier {},
    /// Everything below is inherited from sg721
    #[returns(OwnerOfResponse)]
    OwnerOf {
        token_id: String,
        include_expired: Option<bool>,
    },
    #[returns(ApprovalResponse)]
    Approval {
        token_id: String,
        spender: String,
        include_expired: Option<bool>,
    },
    #[returns(NumTokensResponse)]
    NumTokens {},
    #[returns(cw721::msg::CollectionInfoAndExtensionResponse<cw721::DefaultOptionalCollectionExtension>)]
    ContractInfo {},
    #[returns(NftInfoResponse<Metadata>)]
    NftInfo { token_id: String },
    #[returns(AllNftInfoResponse<Metadata>)]
    AllNftInfo {
        token_id: String,
        include_expired: Option<bool>,
    },
    #[returns(TokensResponse)]
    Tokens {
        owner: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(TokensResponse)]
    AllTokens {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(Ownership<Addr>)]
    Minter {},
    #[returns(OperatorResponse)]
    Operator {
        owner: String,
        operator: String,
        include_expired: Option<bool>,
    },
    // #[returns(CollectionInfoResponse)]
    // CollectionInfo {},
}

impl From<Terp721AccountsQueryMsg>
    for Cw721QueryMsg<Metadata, DefaultOptionalCollectionExtension, Terp721AccountsQueryMsg>
{
    fn from(
        msg: Terp721AccountsQueryMsg,
    ) -> Cw721QueryMsg<Metadata, DefaultOptionalCollectionExtension, Terp721AccountsQueryMsg> {
        match msg {
            Terp721AccountsQueryMsg::OwnerOf {
                token_id,
                include_expired,
            } => Cw721QueryMsg::OwnerOf {
                token_id,
                include_expired,
            },
            Terp721AccountsQueryMsg::Approval {
                token_id,
                spender,
                include_expired,
            } => Cw721QueryMsg::Approval {
                token_id,
                spender,
                include_expired,
            },
            Terp721AccountsQueryMsg::NumTokens {} => Cw721QueryMsg::NumTokens {},
            Terp721AccountsQueryMsg::ContractInfo {} => {
                Cw721QueryMsg::GetCollectionInfoAndExtension {}
            }
            Terp721AccountsQueryMsg::NftInfo { token_id } => Cw721QueryMsg::NftInfo { token_id },
            Terp721AccountsQueryMsg::AllNftInfo {
                token_id,
                include_expired,
            } => Cw721QueryMsg::AllNftInfo {
                token_id,
                include_expired,
            },
            Terp721AccountsQueryMsg::Tokens {
                owner,
                start_after,
                limit,
            } => Cw721QueryMsg::Tokens {
                owner,
                start_after,
                limit,
            },
            Terp721AccountsQueryMsg::AllTokens { start_after, limit } => {
                Cw721QueryMsg::AllTokens { start_after, limit }
            }
            Terp721AccountsQueryMsg::Minter {} => Cw721QueryMsg::GetMinterOwnership {},
            Terp721AccountsQueryMsg::Operator {
                owner,
                operator,
                include_expired,
            } => Cw721QueryMsg::Operator {
                owner,
                operator,
                include_expired,
            },
            _ => unreachable!("cannot convert {:?} to Cw721QueryMsg", msg),
        }
    }
}

#[cw_serde]
pub enum SudoMsg {
    UpdateParams {
        max_record_count: u32,
        max_rev_map_count: u32,
    },
}

#[cosmwasm_schema::cw_serde]
pub struct MigrateMsg {}
