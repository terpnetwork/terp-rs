use cosmwasm_std::{
    Addr, Binary, CosmosMsg, Deps, DepsMut, Env, Event, MessageInfo, Order, Reply, Response,
    StdResult, SubMsg, WasmMsg, entry_point, to_json_binary,
};
use cw_headstash::distro::DistroHashDomain;
use cw_ownable::initialize_owner;
use cw_storage_plus::Bound;

use crate::error::{ContractError, ContractResult};
use crate::msg::{
    EligibilityRootRecord, ExecuteMsg, FundingInfo, FundingToken, HeadstashContract,
    InstantiateMsg, QueryMsg,
};
use crate::state::{HEADSTASH_CODE_ID, MANIFOLD_ROOTS, contracts};
use cw_headstash::msg::{
    ExecuteMsg as HeadstashExecuteMsg, InstantiateMsg as HeadstashInstantiateMsg,
};

// Temporary storage for instantiation data during reply
pub const PENDING_INSTANTIATION: cw_storage_plus::Item<PendingInstantiation> =
    cw_storage_plus::Item::new("pending_instantiation");

#[cosmwasm_schema::cw_serde]
pub struct PendingInstantiation {
    pub instantiator: Addr,
    pub genesis_root: Binary,
    pub distro_hash_domain: DistroHashDomain,
    pub funding: Option<FundingInfo>,
}

const INSTANTIATE_HEADSTASH_REPLY_ID: u64 = 1;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> ContractResult<Response> {
    let owner = msg
        .owner
        .as_deref()
        .map_or(Ok(info.sender.clone()), |o| deps.api.addr_validate(o))?;

    initialize_owner(deps.storage, deps.api, Some(&owner.to_string()))?;
    HEADSTASH_CODE_ID.save(deps.storage, &msg.headstash_code_id)?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("owner", owner)
        .add_attribute("headstash_code_id", msg.headstash_code_id.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> ContractResult<Response> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => {
            cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new())
        }
        ExecuteMsg::CreateHeadstash {
            instantiate_msg,
            label,
            funding,
        } => execute_create_headstash(deps, env, info, instantiate_msg, label, funding),
        ExecuteMsg::RegisterEligibilityRoot {
            headstash,
            root,
            domain,
            label,
            forward_to_headstash,
            root_id,
        } => execute_register_eligibility_root(
            deps,
            info,
            headstash,
            root,
            domain,
            label,
            forward_to_headstash,
            root_id,
        ),
    }
}

fn execute_create_headstash(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    instantiate_msg: HeadstashInstantiateMsg,
    label: Option<String>,
    funding: Option<FundingInfo>,
) -> ContractResult<Response> {
    // Check that only the owner can create headstash contracts
    cw_ownable::assert_owner(deps.storage, &info.sender)?;

    // Validate funding if provided
    if let Some(ref funding_info) = funding {
        validate_funding(&deps, &info, funding_info)?;
    }

    // Reject empty genesis roots early (mirrors cw-headstash policy).
    if instantiate_msg.genesis_root.is_empty() {
        return Err(ContractError::Std(cosmwasm_std::StdError::msg(
            "empty eligibility root",
        )));
    }

    let code_id = HEADSTASH_CODE_ID.load(deps.storage)?;

    // Store pending instantiation data for the reply handler
    let pending = PendingInstantiation {
        instantiator: info.sender.clone(),
        genesis_root: instantiate_msg.genesis_root.clone(),
        distro_hash_domain: instantiate_msg.distro_hash_domain,
        funding: funding.clone(),
    };
    PENDING_INSTANTIATION.save(deps.storage, &pending)?;

    let label = label.unwrap_or_else(|| format!("headstash-{}", info.sender));

    let instantiate = WasmMsg::Instantiate {
        admin: Some(info.sender.to_string()),
        code_id,
        msg: to_json_binary(&instantiate_msg)?,
        funds: vec![], // Funding handled separately
        label,
    };

    let msg = SubMsg::reply_on_success(instantiate, INSTANTIATE_HEADSTASH_REPLY_ID);

    let event = Event::new("create_headstash")
        .add_attribute("instantiator", info.sender.to_string())
        .add_attribute("funding", funding.is_some().to_string())
        .add_attribute(
            "distro_hash_domain",
            instantiate_msg.distro_hash_domain.as_str(),
        );

    Ok(Response::new()
        .add_submessage(msg)
        .add_event(event)
        .add_attribute("action", "create_headstash"))
}

fn execute_register_eligibility_root(
    deps: DepsMut,
    info: MessageInfo,
    headstash: String,
    root: Binary,
    domain: Option<DistroHashDomain>,
    label: Option<String>,
    forward_to_headstash: bool,
    root_id: Option<u64>,
) -> ContractResult<Response> {
    cw_ownable::assert_owner(deps.storage, &info.sender)?;

    if root.is_empty() {
        return Err(ContractError::Std(cosmwasm_std::StdError::msg(
            "empty eligibility root",
        )));
    }

    let headstash_addr = deps.api.addr_validate(&headstash)?;
    // Must be a known Headstash under this manifold.
    let _contract = contracts().load(deps.storage, &headstash_addr)?;

    let domain = domain.unwrap_or(DistroHashDomain::PoseidonV1);
    if !domain.allowed_for_new_registration() {
        return Err(ContractError::Std(cosmwasm_std::StdError::msg(format!(
            "additive roots require poseidon-v1; got {}",
            domain.as_str()
        ))));
    }

    let mut res = Response::new().add_attribute("action", "register_eligibility_root");

    if forward_to_headstash {
        let wasm = WasmMsg::Execute {
            contract_addr: headstash_addr.to_string(),
            msg: to_json_binary(&HeadstashExecuteMsg::RegisterEligibilityRoot {
                root: root.clone(),
                domain: Some(domain),
                label: label.clone(),
            })?,
            funds: vec![],
        };
        res = res.add_message(CosmosMsg::Wasm(wasm));
        // Headstash assigns root_id; if caller provided one, index under that.
        // Otherwise index under a provisional id only when root_id is given.
        if let Some(id) = root_id {
            let rec = EligibilityRootRecord {
                headstash: headstash_addr.clone(),
                root_id: id,
                root: root.clone(),
                domain,
                label: label.clone(),
            };
            MANIFOLD_ROOTS.save(deps.storage, (&headstash_addr, id), &rec)?;
            res = res.add_attribute("root_id", id.to_string());
        }
    } else {
        let id = root_id.ok_or_else(|| {
            ContractError::Std(cosmwasm_std::StdError::msg(
                "root_id required when forward_to_headstash is false",
            ))
        })?;
        let rec = EligibilityRootRecord {
            headstash: headstash_addr.clone(),
            root_id: id,
            root: root.clone(),
            domain,
            label: label.clone(),
        };
        MANIFOLD_ROOTS.save(deps.storage, (&headstash_addr, id), &rec)?;
        res = res.add_attribute("root_id", id.to_string());
    }

    Ok(res
        .add_attribute("headstash", headstash_addr)
        .add_attribute("distro_hash_domain", domain.as_str()))
}

fn validate_funding(
    deps: &DepsMut,
    info: &MessageInfo,
    funding: &FundingInfo,
) -> ContractResult<()> {
    match &funding.token {
        FundingToken::Native { denom } => {
            let coin = info
                .funds
                .iter()
                .find(|c| c.denom == *denom && c.amount >= funding.amount.into())
                .ok_or(ContractError::InvalidFundingToken {})?;
            if coin.amount < funding.amount.into() {
                return Err(ContractError::InvalidFundingToken {});
            }
        }
        FundingToken::Cw20 { contract_addr } => {
            let _contract = deps.api.addr_validate(contract_addr)?;
            // CW20 funding validation would be handled in the headstash contract
        }
    }
    Ok(())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(
    _deps: DepsMut,
    _env: Env,
    _msg: crate::msg::MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> ContractResult<Response> {
    Ok(Response::new().add_attribute("action", "migrate"))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Binary;
    use cw_headstash::distro::DistroHashDomain;

    use super::*;
    use crate::state::{MANIFOLD_ROOTS, contracts};

    #[test]
    fn test_instantiate() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let info = message_info(&creator, &[]);

        let msg = InstantiateMsg {
            owner: Some(creator.to_string()),
            headstash_code_id: 1,
        };

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(res.messages.len(), 0); // No messages on instantiate
        assert_eq!(res.attributes.len(), 3); // action, owner, code_id
    }

    #[test]
    fn register_root_rejects_empty() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let info = message_info(&creator, &[]);
        instantiate(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            InstantiateMsg {
                owner: Some(creator.to_string()),
                headstash_code_id: 1,
            },
        )
        .unwrap();

        // Seed a fake headstash contract record
        let hs = deps.api.addr_make("headstash1");
        contracts()
            .save(
                deps.as_mut().storage,
                &hs,
                &HeadstashContract {
                    address: hs.clone(),
                    instantiator: creator.clone(),
                    genesis_root: Binary::from(vec![1u8; 32]),
                    distro_hash_domain: DistroHashDomain::PoseidonV1,
                    funding: None,
                },
            )
            .unwrap();

        let err = execute(
            deps.as_mut(),
            env,
            info,
            ExecuteMsg::RegisterEligibilityRoot {
                headstash: hs.to_string(),
                root: Binary::default(),
                domain: None,
                label: None,
                forward_to_headstash: false,
                root_id: Some(1),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn register_second_root_additive_index() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");
        let info = message_info(&creator, &[]);
        instantiate(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            InstantiateMsg {
                owner: Some(creator.to_string()),
                headstash_code_id: 1,
            },
        )
        .unwrap();

        let hs = deps.api.addr_make("headstash1");
        contracts()
            .save(
                deps.as_mut().storage,
                &hs,
                &HeadstashContract {
                    address: hs.clone(),
                    instantiator: creator.clone(),
                    genesis_root: Binary::from(vec![1u8; 32]),
                    distro_hash_domain: DistroHashDomain::PoseidonV1,
                    funding: None,
                },
            )
            .unwrap();

        // Index genesis
        MANIFOLD_ROOTS
            .save(
                deps.as_mut().storage,
                (&hs, 0),
                &EligibilityRootRecord {
                    headstash: hs.clone(),
                    root_id: 0,
                    root: Binary::from(vec![1u8; 32]),
                    domain: DistroHashDomain::PoseidonV1,
                    label: Some("drop-0".into()),
                },
            )
            .unwrap();

        execute(
            deps.as_mut(),
            env,
            info,
            ExecuteMsg::RegisterEligibilityRoot {
                headstash: hs.to_string(),
                root: Binary::from(vec![2u8; 32]),
                domain: Some(DistroHashDomain::PoseidonV1),
                label: Some("drop-1".into()),
                forward_to_headstash: false,
                root_id: Some(1),
            },
        )
        .unwrap();

        let r1 = MANIFOLD_ROOTS.load(deps.as_ref().storage, (&hs, 1)).unwrap();
        assert_eq!(r1.root, Binary::from(vec![2u8; 32]));
        assert_eq!(r1.domain, DistroHashDomain::PoseidonV1);
    }
}

fn handle_instantiate_reply(deps: DepsMut, msg: Reply) -> ContractResult<Response> {
    // In CosmWasm v3, we extract the contract address from the reply result
    let response = msg
        .result
        .into_result()
        .map_err(|_| ContractError::InstantiationFailed {})?;
    let contract_addr_str = response
        .events
        .iter()
        .find(|e| e.ty == "instantiate")
        .and_then(|e| e.attributes.iter().find(|a| a.key == "_contract_address"))
        .map(|a| a.value.clone())
        .ok_or(ContractError::InstantiationFailed {})?;
    let contract_addr = deps.api.addr_validate(&contract_addr_str)?;

    // Load the pending instantiation data
    let pending = PENDING_INSTANTIATION.load(deps.storage)?;
    let domain = pending.distro_hash_domain;
    let genesis_root = pending.genesis_root.clone();

    let contract = HeadstashContract {
        address: contract_addr.clone(),
        instantiator: pending.instantiator,
        genesis_root: genesis_root.clone(),
        distro_hash_domain: domain,
        funding: pending.funding,
    };

    contracts().save(deps.storage, &contract_addr, &contract)?;

    // Index genesis root (root_id = 0) in the manifold registry.
    let genesis_rec = EligibilityRootRecord {
        headstash: contract_addr.clone(),
        root_id: 0,
        root: genesis_root,
        domain,
        label: None,
    };
    MANIFOLD_ROOTS.save(deps.storage, (&contract_addr, 0), &genesis_rec)?;

    // Clean up pending data
    PENDING_INSTANTIATION.remove(deps.storage);

    Ok(Response::new()
        .add_attribute("action", "register_headstash")
        .add_attribute("contract_address", contract_addr)
        .add_attribute("distro_hash_domain", domain.as_str())
        .add_attribute("root_id", "0"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::ContractsByInstantiator {
            instantiator,
            start_after,
            limit,
        } => query_contracts_by_instantiator(deps, instantiator, start_after, limit),

        QueryMsg::Contract { address } => query_contract(deps, address),
        QueryMsg::EligibilityRoots {
            headstash,
            start_after,
            limit,
        } => query_eligibility_roots(deps, headstash, start_after, limit),
        QueryMsg::EligibilityRoot {
            headstash,
            root_id,
        } => query_eligibility_root(deps, headstash, root_id),
    }
}

fn query_contracts_by_instantiator(
    deps: Deps,
    instantiator: String,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<Binary> {
    let instantiator_addr = deps.api.addr_validate(&instantiator)?;
    let start = start_after
        .map(|s| deps.api.addr_validate(&s))
        .transpose()?
        .map(Bound::exclusive);

    let contracts_list = contracts()
        .idx
        .instantiator
        .prefix(instantiator_addr)
        .range(deps.storage, start, None, cosmwasm_std::Order::Ascending)
        .take(limit.unwrap_or(30) as usize)
        .map(|item| item.map(|(_, contract)| contract))
        .collect::<StdResult<Vec<_>>>()?;

    to_json_binary(&contracts_list)
}

fn query_contract(deps: Deps, address: String) -> StdResult<Binary> {
    let addr = deps.api.addr_validate(&address)?;
    let contract = contracts().load(deps.storage, &addr)?;
    to_json_binary(&contract)
}

fn query_eligibility_roots(
    deps: Deps,
    headstash: String,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<Binary> {
    let addr = deps.api.addr_validate(&headstash)?;
    let start = start_after.map(Bound::exclusive);
    let limit = limit.unwrap_or(30).min(100) as usize;
    let roots: Vec<_> = MANIFOLD_ROOTS
        .prefix(&addr)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| item.map(|(_, rec)| rec))
        .collect::<StdResult<_>>()?;
    to_json_binary(&roots)
}

fn query_eligibility_root(deps: Deps, headstash: String, root_id: u64) -> StdResult<Binary> {
    let addr = deps.api.addr_validate(&headstash)?;
    let rec = MANIFOLD_ROOTS.load(deps.storage, (&addr, root_id))?;
    to_json_binary(&rec)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> ContractResult<Response> {
    match msg.id {
        INSTANTIATE_HEADSTASH_REPLY_ID => handle_instantiate_reply(deps, msg),
        _ => Err(ContractError::UnknownReplyId { id: msg.id }),
    }
}
