use crate::{
    claims::{fetch_witness_for_claim, Proof},
    state::{Config, Epoch, Witness, CONFIG, EPOCHS},
};
use terp_account::traits::default::TerpAccountTrait;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{from_json, Addr, Event, Response, Uint128};
use serde::{Deserialize, Serialize};

pub use crate::error::ContractError;
pub mod claims;
pub mod digest;
mod error;
mod state;

use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, StdError, StdResult,
};

// version info for migration info
use cw2::set_contract_version;
const CONTRACT_NAME: &str = "crates.io:terp-zktls";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerpAccountZkTls {}
pub type TerpAccountZkTslAuthStuct = Proof;
pub type SudoMsg = <TerpAccountZkTls as TerpAccountTrait>::SudoMsg;

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    AddEpoch {
        witness: Vec<Witness>,
        minimum_witness: Uint128,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(GetAllEpochResponse)]
    GetAllEpoch {},
    #[returns(GetEpochResponse)]
    GetEpoch { id: u128 },
}

#[cw_serde]
pub struct GetAllEpochResponse {
    pub ids: Vec<u128>,
}

#[cw_serde]
pub struct GetEpochResponse {
    pub epoch: Epoch,
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            owner: msg.owner.to_string(),
            current_epoch: Uint128::zero(),
        },
    )?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetEpoch { id } => to_json_binary(&query_epoch_id(deps, id)?),
        QueryMsg::GetAllEpoch {} => to_json_binary(&query_all_epoch_ids(deps)?),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::AddEpoch {
            witness,
            minimum_witness,
        } => add_epoch(deps, env, witness, minimum_witness, info.sender.clone()),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    TerpAccountZkTls::process_sudo_auth(deps, env, &msg)
}

impl terp_account::traits::default::TerpAccountTrait for TerpAccountZkTls {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = terp_auth::AuthSudoMsg;

    type ContractError = ContractError;

    type AuthMethodStructs = TerpAccountZkTslAuthStuct;
    type AuthProcessResult = Result<Response, ContractError>;

    fn extended_authenticate(
        deps: cosmwasm_std::DepsMut,
        auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        todo!()
    }

    fn process_sudo_auth(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &Self::SudoMsg,
    ) -> Self::AuthProcessResult {
        todo!()
    }

    fn on_auth_added(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        // small storage writes, for example global contract entropy or count of registered accounts
        match req.authenticator_params {
            Some(_) => Ok(Response::new().add_attribute("action", "auth_added_req")),
            None => Err(ContractError::MissingAuthenticatorMetadata {}),
        }
    }

    fn on_auth_removed(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::new().add_attribute("action", "auth_removed_req"))
    }

    fn on_auth_request(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let mut resp = Response::new().add_attribute("action", "auth_req");
        let Proof {
            claimInfo,
            signedClaim,
        }: Proof = from_json(&req.signature)?;
        match EPOCHS.may_load(deps.storage, signedClaim.claim.epoch.into())? {
            Some(epoch) => {
                // Hash the claims, and verify with identifier hash
                let hashed = claimInfo.hash();
                if signedClaim.claim.identifier != hashed {
                    return Err(ContractError::HashMismatchErr {});
                }

                // Fetch witness for claim
                let expected_witness = fetch_witness_for_claim(
                    epoch,
                    signedClaim.claim.identifier.clone(),
                    env.block.time,
                );

                let expected_witness_addresses = Witness::get_addresses(expected_witness);

                // recover witness address from SignedClaims Object
                let signed_witness = signedClaim.recover_signers_of_signed_claim(deps)?;

                // make sure the minimum requirement for witness is satisfied
                if expected_witness_addresses.len() != signed_witness.len() {
                    return Err(ContractError::WitnessMismatchErr {});
                }
                // Ensure for every signature in the sign, a expected witness exists from the database
                for signed in signed_witness {
                    let signed_event = Event::new("signer").add_attribute("sig", signed.clone());
                    resp = resp.add_event(signed_event);
                    if !expected_witness_addresses.contains(&signed) {
                        return Err(ContractError::SignatureErr {});
                    }
                }
                Ok(resp)
            }
            None => return Err(ContractError::NotFoundErr {}),
        }
    }

    fn on_auth_track(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        // this is where we handle any processes after authentication, regarding message contents, prep to track balances prior to msg execution, etc..
        Ok(Response::new().add_attribute("action", "track_req"))
    }

    fn on_auth_confirm(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        // here is were we compare balances post event execution, based on data saved from sudo_track_request,etc..
        Ok(Response::new().add_attribute("action", "conf_exec_req"))
    }

    fn on_hooks(deps: cosmwasm_std::DepsMut, env: cosmwasm_std::Env) -> Self::AuthProcessResult {
        todo!()
    }
}

//NOTE: Unimplemented as secret doesn't allow to iterate via keys
fn query_all_epoch_ids(_deps: Deps) -> StdResult<GetAllEpochResponse> {
    Ok(GetAllEpochResponse { ids: vec![] })
}

fn query_epoch_id(deps: Deps, id: u128) -> StdResult<GetEpochResponse> {
    match EPOCHS.may_load(deps.storage, id)? {
        Some(epoch) => Ok(GetEpochResponse { epoch }),
        None => Err(StdError::msg("No such epoch")),
    }
}

// @dev - add epoch
pub fn add_epoch(
    deps: DepsMut,
    env: Env,
    witness: Vec<Witness>,
    minimum_witness: Uint128,
    sender: Addr,
) -> Result<Response, ContractError> {
    // load configs
    let mut config = CONFIG.load(deps.storage)?;

    if config.owner != sender.to_string() {
        return Err(ContractError::Unauthorized {});
    }

    // Increment Epoch number
    let new_epoch = config.current_epoch + Uint128::one();
    // Create the new epoch
    let epoch = Epoch {
        id: new_epoch,
        witness,
        timestamp_start: env.block.time.nanos(),
        timestamp_end: env.block.time.plus_seconds(86400).nanos(),
        minimum_witness_for_claim_creation: minimum_witness,
    };

    // Upsert the new epoch into memory
    EPOCHS.save(deps.storage, new_epoch.into(), &epoch)?;

    // Save the new epoch
    config.current_epoch = new_epoch;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::default())
}
