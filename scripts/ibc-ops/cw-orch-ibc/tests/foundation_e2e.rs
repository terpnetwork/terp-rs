//! E2E against real dao-dao contracts on cw-multi-test (staking enabled).
//!
//! Mirrors dao-testing helpers: pre-propose-single → vote yes → execute.
//! One proposal: native delegations + FIAB-SUBDAO instantiate.

use cosmwasm_std::{
    testing::mock_env, to_json_binary, Addr, Coin, CosmosMsg, Decimal, Empty, Uint256, Validator,
    WasmMsg,
};
use cw4::Member;
use cw_multi_test::{App, BankSudo, Contract, ContractWrapper, Executor, SudoMsg};
use cw_orch_ibc::{
    execute_proposal_msg, instantiate_subdao_msg, vote_yes_msg, NativeDelegationPlan,
    FIAB_DESCRIPTION, FIAB_NAME, FOUNDATION_DESCRIPTION, FOUNDATION_TITLE, GAS_DENOM, terp_100k,
    DELEGATION_DESCRIPTION, DELEGATION_TITLE,
};
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_voting::{
    pre_propose::{PreProposeInfo, PreProposeSubmissionPolicy},
    threshold::Threshold,
};

const VAL2: &str = "valoper2";
const VAL3: &str = "valoper3";
const STAKE: &str = "TOKEN";

fn dummy_migrate_info() -> cosmwasm_std::MigrateInfo {
    cosmwasm_std::MigrateInfo {
        sender: Addr::unchecked(""),
        old_migrate_version: None,
    }
}

fn dao_core() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            dao_dao_core::contract::execute,
            dao_dao_core::contract::instantiate,
            dao_dao_core::contract::query,
        )
        .with_reply(dao_dao_core::contract::reply)
        .with_migrate(|d, e, m, _| dao_dao_core::contract::migrate(d, e, m, dummy_migrate_info())),
    )
}

fn proposal_single() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            dao_proposal_single::contract::execute,
            dao_proposal_single::contract::instantiate,
            dao_proposal_single::contract::query,
        )
        .with_reply(dao_proposal_single::contract::reply),
    )
}

fn voting_cw4() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            dao_voting_cw4::contract::execute,
            dao_voting_cw4::contract::instantiate,
            dao_voting_cw4::contract::query,
        )
        .with_reply(dao_voting_cw4::contract::reply),
    )
}

fn cw4_group() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(
        cw4_group::contract::execute,
        cw4_group::contract::instantiate,
        cw4_group::contract::query,
    ))
}

fn pre_propose_single() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            dao_pre_propose_single::contract::execute,
            dao_pre_propose_single::contract::instantiate,
            dao_pre_propose_single::contract::query,
        )
        .with_migrate(|d, e, m, _| {
            dao_pre_propose_single::contract::migrate(d, e, m, dummy_migrate_info())
        }),
    )
}

#[test]
fn foundation_delegation_and_fiab_one_proposal_votes_yes_and_executes() {
    let mut app = App::new(|router, api, storage| {
        let block = mock_env().block;
        for addr in [VAL2, VAL3] {
            router
                .staking
                .add_validator(
                    api,
                    storage,
                    &block,
                    Validator::create(
                        addr.to_string(),
                        Decimal::percent(5),
                        Decimal::percent(20),
                        Decimal::percent(1),
                    ),
                )
                .unwrap();
        }
    });
    let owner = app.api().addr_make("owner");

    let core_id = app.store_code(dao_core());
    let prop_id = app.store_code(proposal_single());
    let vote_id = app.store_code(voting_cw4());
    let group_id = app.store_code(cw4_group());
    let pre_id = app.store_code(pre_propose_single());

    let voting_info = ModuleInstantiateInfo {
        code_id: vote_id,
        msg: to_json_binary(&dao_voting_cw4::msg::InstantiateMsg {
            group_contract: dao_voting_cw4::msg::GroupContract::New {
                cw4_group_code_id: group_id,
                cw4_group_salt: None,
                initial_members: vec![Member {
                    addr: owner.to_string(),
                    weight: 1,
                }],
            },
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "voting-cw4".into(),
        salt: None,
    };
    let prop_info = ModuleInstantiateInfo {
        code_id: prop_id,
        msg: to_json_binary(&dao_proposal_single::msg::InstantiateMsg {
            threshold: Threshold::AbsoluteCount {
                threshold: Uint256::from(1u64),
            },
            max_voting_period: cw_utils::Duration::Time(86400),
            min_voting_period: None,
            only_members_execute: true,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::ModuleMayPropose {
                info: ModuleInstantiateInfo {
                    code_id: pre_id,
                    msg: to_json_binary(&dao_pre_propose_single::InstantiateMsg {
                        deposit_info: None,
                        submission_policy: PreProposeSubmissionPolicy::Specific {
                            dao_members: true,
                            allowlist: vec![],
                            denylist: vec![],
                        },
                        extension: Empty {},
                    })
                    .unwrap(),
                    admin: Some(Admin::CoreModule {}),
                    funds: None,
                    label: "pre-propose-single".into(),
                    salt: None,
                },
            },
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "proposal-single".into(),
        salt: None,
    };

    let dao = app
        .instantiate_contract(
            core_id,
            owner.clone(),
            &dao_interface::msg::InstantiateMsg {
                admin: None,
                name: "Terp Foundation".into(),
                description: "parent".into(),
                image_url: None,
                automatically_add_cw20s: true,
                automatically_add_cw721s: true,
                voting_module_instantiate_info: voting_info.clone(),
                proposal_modules_instantiate_info: vec![prop_info.clone()],
                initial_items: None,
                initial_actions: None,
                dao_uri: None,
            },
            &[],
            "foundation",
            None,
        )
        .unwrap();

    app.update_block(|b| b.height += 2);

    let stake_needed = terp_100k() + terp_100k() + Uint256::from(1_000_000u128);
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: dao.to_string(),
        amount: vec![
            Coin::new(stake_needed, STAKE),
            Coin::new(1_000_000u128, GAS_DENOM),
        ],
    }))
    .unwrap();

    let mut plan = NativeDelegationPlan::bonded_set(vec![VAL2.into(), VAL3.into()]);
    plan.stake_denom = STAKE.into();
    plan.other_validators = vec![VAL2.into(), VAL3.into()];

    let fiab_prop = ModuleInstantiateInfo {
        code_id: prop_id,
        msg: to_json_binary(&dao_proposal_single::msg::InstantiateMsg {
            threshold: Threshold::AbsoluteCount {
                threshold: Uint256::from(1u64),
            },
            max_voting_period: cw_utils::Duration::Time(86400),
            min_voting_period: None,
            only_members_execute: true,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::AnyoneMayPropose {},
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "fiab-proposal".into(),
        salt: None,
    };
    let fiab = instantiate_subdao_msg(
        dao.to_string(),
        core_id,
        FIAB_NAME,
        to_json_binary(&dao_interface::msg::InstantiateMsg {
            admin: Some(dao.to_string()),
            name: FIAB_NAME.into(),
            description: FIAB_DESCRIPTION.into(),
            image_url: None,
            automatically_add_cw20s: true,
            automatically_add_cw721s: true,
            voting_module_instantiate_info: voting_info,
            proposal_modules_instantiate_info: vec![fiab_prop],
            initial_items: None,
            initial_actions: None,
            dao_uri: None,
        })
        .unwrap()
        .to_vec(),
        vec![
            Coin::new(1_000_000u128, STAKE),
            Coin::new(1_000_000u128, GAS_DENOM),
        ],
    );
    let msgs = plan.with_extra(vec![fiab]);
    assert!(msgs.iter().any(|m| matches!(m, CosmosMsg::Staking(_))));
    assert!(!msgs.iter().any(|m| matches!(m, CosmosMsg::Bank(_))));
    assert!(msgs.iter().any(|m| matches!(
        m,
        CosmosMsg::Wasm(WasmMsg::Instantiate { label, .. }) if label == FIAB_NAME
    )));

    let modules: Vec<dao_interface::state::ProposalModule> = app
        .wrap()
        .query_wasm_smart(
            &dao,
            &dao_interface::msg::QueryMsg::ProposalModules {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    let pmod = modules[0].address.clone();
    let policy: dao_voting::pre_propose::ProposalCreationPolicy = app
        .wrap()
        .query_wasm_smart(
            &pmod,
            &dao_proposal_single::msg::QueryMsg::ProposalCreationPolicy {},
        )
        .unwrap();
    let pre_addr = match policy {
        dao_voting::pre_propose::ProposalCreationPolicy::Module { addr } => addr,
        other => panic!("expected pre-propose module, got {other:?}"),
    };

    app.execute_contract(
        owner.clone(),
        pre_addr,
        &dao_pre_propose_single::ExecuteMsg::Propose {
            msg: dao_pre_propose_single::ProposeMessage::Propose {
                title: FOUNDATION_TITLE.into(),
                description: FOUNDATION_DESCRIPTION.into(),
                msgs,
                vote: None,
            },
        },
        &[],
    )
    .unwrap();

    app.execute_contract(owner.clone(), pmod.clone(), &vote_yes_msg(1), &[])
        .unwrap();
    let resp = app
        .execute_contract(owner, pmod, &execute_proposal_msg(1), &[])
        .unwrap();

    let dels = app.wrap().query_all_delegations(&dao).unwrap();
    let amt = |v: &str| {
        dels.iter()
            .find(|d| d.validator == v)
            .map(|d| d.amount.amount)
            .unwrap_or_default()
    };
    assert_eq!(amt(VAL2), terp_100k());
    assert_eq!(amt(VAL3), terp_100k());
    assert_eq!(dels.len(), 2);

    let cfg = resp
        .events
        .iter()
        .flat_map(|e| e.attributes.iter())
        .filter(|a| a.key == "_contract_address")
        .map(|a| a.value.clone())
        .find_map(|addr| {
            app.wrap()
                .query_wasm_smart::<dao_interface::state::Config>(
                    addr,
                    &dao_interface::msg::QueryMsg::Config {},
                )
                .ok()
                .filter(|c| c.name == FIAB_NAME)
        })
        .expect("FIAB-SUBDAO instantiated");
    assert_eq!(cfg.name, FIAB_NAME);
}

#[test]
fn dry_run_wrapper_is_the_pre_propose_execute_the_e2e_sends() {
    let msgs = NativeDelegationPlan::bonded_set(vec!["v1".into()]).cosmos_msgs();
    let v = cw_orch_ibc::foundation_proposal_wrapper(
        DELEGATION_TITLE,
        DELEGATION_DESCRIPTION,
        msgs,
    );
    let parsed: dao_pre_propose_single::ExecuteMsg =
        serde_json::from_value(v).expect("wrapper is pre-propose ExecuteMsg");
    match parsed {
        dao_pre_propose_single::ExecuteMsg::Propose {
            msg:
                dao_pre_propose_single::ProposeMessage::Propose {
                    title,
                    description,
                    msgs,
                    vote,
                },
        } => {
            assert_eq!(title, DELEGATION_TITLE);
            assert_eq!(description, DELEGATION_DESCRIPTION);
            assert!(vote.is_none());
            assert!(msgs.iter().any(|m| matches!(m, CosmosMsg::Staking(_))));
            assert!(!msgs.iter().any(|m| matches!(m, CosmosMsg::Bank(_))));
        }
        other => panic!("{other:?}"),
    }

    let fiab = cw_orch_ibc::foundation_proposal_wrapper(
        FOUNDATION_TITLE,
        FOUNDATION_DESCRIPTION,
        vec![cw_orch_ibc::morocco_fiab_instantiate(cw_orch_ibc::PARENT_CORE)],
    );
    let parsed: dao_pre_propose_single::ExecuteMsg =
        serde_json::from_value(fiab).expect("fiab wrapper");
    match parsed {
        dao_pre_propose_single::ExecuteMsg::Propose {
            msg: dao_pre_propose_single::ProposeMessage::Propose { title, msgs, .. },
        } => {
            assert_eq!(title, FOUNDATION_TITLE);
            assert_eq!(msgs.len(), 1);
            assert!(matches!(
                &msgs[0],
                CosmosMsg::Wasm(WasmMsg::Instantiate { label, .. }) if label == FIAB_NAME
            ));
        }
        other => panic!("{other:?}"),
    }
}
