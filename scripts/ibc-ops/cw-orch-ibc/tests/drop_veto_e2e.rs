//! Local e2e for morocco-1 proposal 5: drop 7-day veto without bricking voting.
//!
//! Mirrors live proposal-single config (majority + 20% quorum, 115200s, 7-day
//! veto, pre-propose). Applies the exact UpdateConfig body (dao = this core),
//! then proposes / votes / executes again with veto off.

use cosmwasm_std::{to_json_binary, Addr, Empty, Uint256};
use cw4::Member;
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use cw_orch_ibc::{
    drop_veto_msg_for, drop_veto_update_config, execute_proposal_msg, vote_yes_msg, PARENT_CORE,
};
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_proposal_single::query::ProposalResponse;
use dao_voting::{
    pre_propose::{PreProposeInfo, PreProposeSubmissionPolicy, ProposalCreationPolicy},
    status::Status,
    threshold::{PercentageThreshold, Threshold},
    veto::VetoConfig,
};

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

struct Harness {
    app: App,
    dao: Addr,
    pmod: Addr,
    pre: Addr,
    a: Addr,
    b: Addr,
    c: Addr,
}

fn live_threshold() -> Threshold {
    Threshold::ThresholdQuorum {
        threshold: PercentageThreshold::Majority {},
        quorum: PercentageThreshold::Percent(cosmwasm_std::Decimal::percent(20)),
    }
}

/// Foundation-shaped DAO: 1000/1000/999, majority+20%, 32h vote, 7-day veto.
fn spawn() -> Harness {
    let mut app = App::default();
    let a = app.api().addr_make("m1000a");
    let b = app.api().addr_make("m1000b");
    let c = app.api().addr_make("m999");
    let vetoer = app.api().addr_make("govvetoer");

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
                initial_members: vec![
                    Member {
                        addr: a.to_string(),
                        weight: 1000,
                    },
                    Member {
                        addr: b.to_string(),
                        weight: 1000,
                    },
                    Member {
                        addr: c.to_string(),
                        weight: 999,
                    },
                ],
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
            threshold: live_threshold(),
            max_voting_period: cw_utils::Duration::Time(115200),
            min_voting_period: None,
            only_members_execute: false,
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
            veto: Some(VetoConfig {
                timelock_duration: cw_utils::Duration::Time(604800),
                vetoer: vetoer.to_string(),
                early_execute: true,
                veto_before_passed: false,
            }),
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
            a.clone(),
            &dao_interface::msg::InstantiateMsg {
                admin: None,
                name: "Terp Foundation".into(),
                description: "parent".into(),
                image_url: None,
                automatically_add_cw20s: true,
                automatically_add_cw721s: true,
                voting_module_instantiate_info: voting_info,
                proposal_modules_instantiate_info: vec![prop_info],
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
    let policy: ProposalCreationPolicy = app
        .wrap()
        .query_wasm_smart(
            &pmod,
            &dao_proposal_single::msg::QueryMsg::ProposalCreationPolicy {},
        )
        .unwrap();
    let pre = match policy {
        ProposalCreationPolicy::Module { addr } => addr,
        other => panic!("{other:?}"),
    };

    Harness {
        app,
        dao,
        pmod,
        pre,
        a,
        b,
        c,
    }
}

fn propose(h: &mut Harness, title: &str, msgs: Vec<cosmwasm_std::CosmosMsg>) {
    h.app
        .execute_contract(
            h.a.clone(),
            h.pre.clone(),
            &dao_pre_propose_single::ExecuteMsg::Propose {
                msg: dao_pre_propose_single::ProposeMessage::Propose {
                    title: title.into(),
                    description: title.into(),
                    msgs,
                    vote: None,
                },
            },
            &[],
        )
        .unwrap();
}

fn two_yes(h: &mut Harness, id: u64) {
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &vote_yes_msg(id), &[])
        .unwrap();
    h.app
        .execute_contract(h.b.clone(), h.pmod.clone(), &vote_yes_msg(id), &[])
        .unwrap();
}

fn proposal(h: &Harness, id: u64) -> ProposalResponse {
    h.app
        .wrap()
        .query_wasm_smart(
            &h.pmod,
            &dao_proposal_single::msg::QueryMsg::Proposal { proposal_id: id },
        )
        .unwrap()
}

fn cfg(h: &Harness) -> dao_proposal_single::state::Config {
    h.app
        .wrap()
        .query_wasm_smart(&h.pmod, &dao_proposal_single::msg::QueryMsg::Config {})
        .unwrap()
}

#[test]
fn live_update_config_points_dao_at_foundation_core() {
    match drop_veto_update_config(PARENT_CORE) {
        dao_proposal_single::msg::ExecuteMsg::UpdateConfig { dao, veto, .. } => {
            assert_eq!(dao, PARENT_CORE);
            assert!(veto.is_none());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn drop_veto_then_dao_still_proposes_votes_and_executes() {
    let mut h = spawn();
    let before = cfg(&h);
    assert!(before.veto.is_some());
    assert_eq!(before.dao, h.dao);
    assert_eq!(before.max_voting_period, cw_utils::Duration::Time(115200));
    assert_eq!(before.threshold, live_threshold());
    assert!(!before.only_members_execute);
    assert!(!before.allow_revoting);
    assert!(before.close_proposal_on_execution_failure);

    let pmod = h.pmod.to_string();
    let dao = h.dao.to_string();
    propose(
        &mut h,
        "Drop the seven-day execution lock",
        vec![drop_veto_msg_for(&pmod, &dao)],
    );
    two_yes(&mut h, 1);
    let st = proposal(&h, 1).proposal.status;
    assert!(
        matches!(st, Status::VetoTimelock { .. }),
        "prop 5-style: still locked until timelock, got {st:?}"
    );
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &execute_proposal_msg(1), &[])
        .unwrap_err();

    // Timelock starts at vote-window end (115200s), then + 7 days.
    h.app.update_block(|b| {
        b.time = b.time.plus_seconds(115200 + 604800 + 1);
    });
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &execute_proposal_msg(1), &[])
        .unwrap();
    assert_eq!(proposal(&h, 1).proposal.status, Status::Executed);

    let after = cfg(&h);
    assert!(after.veto.is_none(), "veto must be off: {:?}", after.veto);
    assert_eq!(after.dao, h.dao, "dao must stay this core (live: Foundation)");
    assert_eq!(after.threshold, live_threshold());
    assert_eq!(after.max_voting_period, cw_utils::Duration::Time(115200));
    assert!(after.min_voting_period.is_none());
    assert!(!after.only_members_execute);
    assert!(!after.allow_revoting);
    assert!(after.close_proposal_on_execution_failure);

    propose(&mut h, "can we still govern", vec![]);
    two_yes(&mut h, 2);
    let st = proposal(&h, 2).proposal.status;
    assert_eq!(
        st,
        Status::Passed,
        "no timelock after veto None, got {st:?}"
    );
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &execute_proposal_msg(2), &[])
        .unwrap();
    assert_eq!(proposal(&h, 2).proposal.status, Status::Executed);
}

#[test]
fn wrong_dao_in_update_config_breaks_later_proposes() {
    let mut h = spawn();
    let stranger = h.app.api().addr_make("not-the-core");
    let pmod = h.pmod.to_string();
    propose(
        &mut h,
        "brick",
        vec![drop_veto_msg_for(&pmod, stranger.as_str())],
    );
    two_yes(&mut h, 1);
    h.app.update_block(|b| {
        b.time = b.time.plus_seconds(115200 + 604800 + 1);
    });
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &execute_proposal_msg(1), &[])
        .unwrap();
    assert_eq!(cfg(&h).dao, stranger);

    let err = h
        .app
        .execute_contract(
            h.a.clone(),
            h.pre.clone(),
            &dao_pre_propose_single::ExecuteMsg::Propose {
                msg: dao_pre_propose_single::ProposeMessage::Propose {
                    title: "after brick".into(),
                    description: "should fail".into(),
                    msgs: vec![],
                    vote: None,
                },
            },
            &[],
        )
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("query")
            || msg.contains("not found")
            || msg.contains("unknown")
            || msg.to_lowercase().contains("error"),
        "expected propose to fail after dao hijack, got {msg}"
    );
}

#[test]
fn two_of_three_thousand_weight_passes_without_the_999() {
    let mut h = spawn();
    propose(&mut h, "two is enough", vec![]);
    h.app
        .execute_contract(h.a.clone(), h.pmod.clone(), &vote_yes_msg(1), &[])
        .unwrap();
    h.app
        .execute_contract(h.b.clone(), h.pmod.clone(), &vote_yes_msg(1), &[])
        .unwrap();
    let p = proposal(&h, 1).proposal;
    assert_eq!(p.votes.yes, Uint256::from(2000u64));
    assert_eq!(p.total_power, Uint256::from(2999u64));
    assert!(matches!(
        p.status,
        Status::VetoTimelock { .. } | Status::Passed
    ));
    let _ = h.c;
}
