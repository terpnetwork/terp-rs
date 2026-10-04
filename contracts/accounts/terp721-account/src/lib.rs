pub mod commands;
mod error;
pub mod helpers;
pub mod msg;
pub mod state;
pub use crate::error::ContractError;

#[cfg(not(target_arch = "wasm32"))]
pub mod interface;

use crate::msg::MigrateMsg;
use commands::{manifest::*, queries::*, sudo_update_params, transcode};
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response, StdError, StdResult,
};

use cw721::{
    traits::{Cw721Execute, Cw721Query},
    DefaultOptionalCollectionExtension, DefaultOptionalCollectionExtensionMsg,
};
use cw_utils::maybe_addr;
use msg::{InstantiateMsg, SudoMsg, Terp721AccountsQueryMsg};
use state::{SudoParams, ACCOUNT_MANIFOLD, SUDO_PARAMS, VERIFIER};
use terp_account::Metadata;

// version info for migration info
pub const ACCOUNT_CONTRACT: &str = "crates.io:terp721-account";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type Terp721AccountContract<'a> = cw721::extension::Cw721Extensions<
    'a,
    Metadata,
    Metadata,
    DefaultOptionalCollectionExtension,
    DefaultOptionalCollectionExtensionMsg,
    Empty,
    Terp721AccountsQueryMsg,
    Empty,
>;
pub type ExecuteMsg = crate::msg::ExecuteMsg<Metadata>;
pub type QueryMsg = Terp721AccountsQueryMsg;

pub mod entry {
    use super::*;

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn instantiate(
        mut deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: InstantiateMsg,
    ) -> StdResult<Response> {
        cw2::set_contract_version(deps.storage, ACCOUNT_CONTRACT, CONTRACT_VERSION)?;

        SUDO_PARAMS.save(
            deps.storage,
            &SudoParams {
                max_record_count: 10,
                max_reverse_map_key_limit: 10,
            },
        )?;
        let api = deps.api;
        // functions as admin for verification specific features.
        // This admin can never access token owner specific functions for terp-accounts.
        VERIFIER.set(deps.branch(), maybe_addr(api, msg.verifier)?)?;
        ACCOUNT_MANIFOLD.save(deps.storage, &info.sender)?;

        let res = Terp721AccountContract::default()
            .instantiate(deps.branch(), &env, &info, msg.base_init_msg)
            .map_err(|e: cw721::error::Cw721ContractError| {
                cosmwasm_std::StdError::msg(e.to_string())
            })?;
        Ok(res
            .add_attribute("action", "instantiate")
            .add_attribute("terp721_account_address", env.contract.address.to_string()))
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn execute(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: ExecuteMsg,
    ) -> Result<Response, ContractError> {
        let api = deps.api;
        match msg {
            // minter only function
            crate::msg::ExecuteMsg::SetMarketplace { address } => {
                set_profile_marketplace(deps, info, address)
            }
            // only account token owner authorized
            crate::msg::ExecuteMsg::AssociateAddress { account, address } => {
                associate_address(deps, info, env.contract.address, account, address)
            }
            // only account token owner authorized
            crate::msg::ExecuteMsg::UpdateImageNft { account, nft } => {
                update_image_nft(deps, info, account, nft)
            }
            // only account token owner authorized
            crate::msg::ExecuteMsg::AddTextRecord { account, record } => {
                execute_add_text_record(deps, info, account, record)
            }
            // only account token owner authorized
            crate::msg::ExecuteMsg::RemoveTextRecord {
                account,
                record_account,
            } => execute_remove_text_record(deps, info, account, record_account),
            // only account token owner authorized
            crate::msg::ExecuteMsg::UpdateTextRecord { account, record } => {
                execute_update_text_record(deps, info, account, record)
            }
            // only verified authorized
            crate::msg::ExecuteMsg::VerifyTextRecord {
                account,
                record_account,
                result,
            } => execute_verify_text_record(deps, info, account, record_account, result),
            // only verified authorized
            crate::msg::ExecuteMsg::UpdateVerifier { verifier } => {
                Ok(VERIFIER.execute_update_admin(deps, info, maybe_addr(api, verifier)?)?)
            }
            // only account token owner authorized
            crate::msg::ExecuteMsg::TransferNft {
                recipient,
                token_id,
            } => execute_transfer_nft(deps, env, info, recipient, token_id),
            // only account token owner authorized
            ExecuteMsg::SendNft {
                contract,
                token_id,
                msg,
            } => execute_send_nft(deps, env, info, contract, token_id, msg),
            // only collection minter authorized
            ExecuteMsg::Mint {
                token_id,
                owner,
                token_uri,
                extension,
            } => execute_mint(deps, info, token_id, owner, token_uri, extension),
            ExecuteMsg::Burn { token_id } => execute_burn(deps, env, info, token_id),
            ExecuteMsg::UpdateMyReverseMapKey { to_add, to_remove } => {
                execute_update_reverse_map_keys(deps, env, info, to_add, to_remove)
            }
            #[cfg(feature = "abstract")]
            ExecuteMsg::UpdateAbsAccSupport {
                token_id,
                r#abstract,
            } => execute_update_abstract_account_support(deps, env, info, &token_id, r#abstract),
            ExecuteMsg::ApproveAllViaMarket { owner, expires } => {
                execute_approve_all_via_market(deps, env, info, owner, expires)
            }
            _ => Terp721AccountContract::default()
                .execute(deps, &env, &info, msg.into())
                .map_err(|e: cw721::error::Cw721ContractError| e.into()),
        }
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
        match msg {
            QueryMsg::Params {} => to_json_binary(&query_params(deps)?),
            QueryMsg::Account { address } => to_json_binary(&query_account(deps, address)?),
            QueryMsg::Verifier {} => to_json_binary(&VERIFIER.query_admin(deps)?),
            QueryMsg::AssociatedAddress { account } => {
                to_json_binary(&query_associated_address(deps, &account)?)
            }
            QueryMsg::ImageNFT { account } => to_json_binary(&query_image_nft(deps, &account)?),
            QueryMsg::TextRecords { account } => {
                to_json_binary(&query_text_records(deps, &account)?)
            }
            QueryMsg::Minter {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
            QueryMsg::ReverseMapAccount { address } => {
                to_json_binary(&query_account(deps, address)?)
            }
            QueryMsg::ReverseMapAddress { address } => to_json_binary(&transcode(deps, &address)?),
            _ => Terp721AccountContract::default()
                .query(deps, &env, msg.into())
                .map_err(|e: cw721::error::Cw721ContractError| {
                    cosmwasm_std::StdError::msg(e.to_string())
                }),
        }
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
        match msg {
            SudoMsg::UpdateParams {
                max_record_count,
                max_rev_map_count,
            } => sudo_update_params(deps, max_record_count, max_rev_map_count),
        }
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
        let then = cw2::get_contract_version(deps.storage)?;
        if then.version >= CONTRACT_VERSION.to_owned()
            || then.contract != ACCOUNT_CONTRACT.to_owned()
        {
            return Err(ContractError::Std(StdError::msg(
                "unable to migrate terp721-account.",
            )));
        }
        cw2::set_contract_version(deps.storage, ACCOUNT_CONTRACT, CONTRACT_VERSION)?;
        Ok(Response::default())
    }
}
