//! One primitive init and use of each DAO module family.
//!
//! The same function runs on cw-orch `Mock` and on a `Daemon` connected to
//! terpd. The DAO never signs: hooks and child DAOs are installed by a
//! proposal the core executes. `tokenfactory` is the native-stake voting
//! leg; the mock has no tokenfactory module, so that caller passes `false`.
//! Bitsong's fantoken factory stays off in this crate.

use std::path::PathBuf;

use cosmwasm_std::{to_json_binary, Addr, Binary, Coin, CosmosMsg, Empty, Uint128, Uint256, WasmMsg};
use cw_orch::prelude::*;
use cw_utils::Duration;
use dao_interface::query::GetItemResponse;
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_testing::DaoDaoSuite;
use dao_voting::multiple_choice::{
    MultipleChoiceOption, MultipleChoiceOptions, MultipleChoiceVote, VotingStrategy,
};
use dao_voting::pre_propose::{
    PreProposeInfo, PreProposeSubmissionPolicy, ProposalCreationPolicy,
};
use dao_voting::proposal::SingleChoiceProposeMsg;
use dao_voting::status::Status;
use dao_voting::threshold::PercentageThreshold;
use dao_voting::voting::Vote;

use super::{
    assert_voting_child, instantiate_msg, voting_addr, voting_module, Codes, VotingKind, ROOT,
};

#[test]
fn dao_modules_exercise_primitive_actions() {
    let chain = Mock::new("dao-actions");
    let member = chain.sender_addr();
    chain
        .add_balance(&member, vec![Coin::new(10u128, "uterp")])
        .unwrap();
    let recipient = chain.addr_make("vestee");
    let counterparty = chain.addr_make("swap-two");
    run_primitive_actions(&chain, &recipient, &counterparty, false);
}

/// Init and one use of each integrated DAO family.
///
/// `recipient` receives the payroll vesting account. `counterparty` is the
/// second party on the token swap. Neither has to hold a key. When
/// `tokenfactory` is set, a child DAO creates a native denom through
/// `dao-voting-token-staked`.
pub fn run_primitive_actions<Chain: CwEnv>(
    chain: &Chain,
    recipient: &Addr,
    counterparty: &Addr,
    tokenfactory: bool,
) {
    let member = chain.sender_addr();
    let mut suite = DaoDaoSuite::new(chain.clone());
    let codes = upload_exercised(&suite, chain);
    let token_codes = tokenfactory.then(|| upload_tokenfactory(&suite, chain));

    let root = root_dao(&suite, chain, &codes, &member);
    suite.snapshot_current_dao(ROOT).unwrap();
    let single = proposal_single(chain, &root, &codes);
    exercise_pre_propose_and_single(chain, &root, &single);
    exercise_multiple(chain, &root, &codes);
    exercise_delegation(&suite, chain, &member, &root, &single);
    exercise_distributions(&suite, chain, &member, &root);
    exercise_gauges(&suite, chain, &member, &root);

    let cw20 = spawn(chain, &root, "cw20-subdao", &codes, VotingKind::Cw20Staked, &member);
    exercise_stake_and_rewards(&suite, chain, &member, &cw20);
    let cw721 = spawn(
        chain,
        &cw20,
        "cw721-subdao",
        &codes,
        VotingKind::Cw721Staked,
        &member,
    );
    exercise_nft_stake(chain, &member, &cw721);
    let roles = spawn(
        chain,
        &cw721,
        "cw721-roles-subdao",
        &codes,
        VotingKind::Cw721Roles,
        &member,
    );
    bump(chain);
    assert_power(chain, &roles, &member, 1);
    let balance = spawn(
        chain,
        &roles,
        "cw20-balance-subdao",
        &codes,
        VotingKind::Cw20Balance,
        &member,
    );
    bump(chain);
    assert_power(chain, &balance, &member, 1);

    if let Some((issuer, voting)) = token_codes {
        exercise_token_staked(chain, &balance, &codes, issuer, voting, &member);
    }

    exercise_externals(&suite, chain, &member, &codes, recipient, counterparty);
    exercise_polytone(&suite);
}

fn upload_exercised<Chain: CwEnv>(suite: &DaoDaoSuite<Chain>, chain: &Chain) -> Codes {
    suite.dao_core.upload().expect("dao core");
    suite.proposal.prop_single.upload().expect("proposal single");
    suite.proposal.prop_multiple.upload().expect("proposal multiple");
    suite
        .proposal
        .pre_prop_suite
        .pre_prop_single
        .upload()
        .expect("pre-propose single");
    suite.voting.voting_cw4.upload().expect("voting cw4");
    suite.voting.cw4_group.upload().expect("cw4 group");
    suite
        .voting
        .voting_cw20_staked
        .upload()
        .expect("voting cw20 staked");
    suite
        .voting
        .voting_cw721_staked
        .upload()
        .expect("voting cw721 staked");
    suite
        .voting
        .voting_cw721_roles
        .upload()
        .expect("voting cw721 roles");
    suite.staking.cw20_stake.upload().expect("cw20 stake");
    suite.external.cw721_roles.upload().expect("cw721 roles");
    suite.delegation.vote.upload().expect("vote delegation");
    suite.distribution.fund_distr.upload().expect("fund distributor");
    suite
        .distribution
        .reward_distr
        .upload()
        .expect("rewards distributor");
    suite.gauges.adapter.upload().expect("gauge adapter");
    suite.gauges.orchestrator.upload().expect("gauge orchestrator");
    suite
        .staking
        .external_rewards
        .upload()
        .expect("external rewards");
    suite
        .staking
        .rewards_distributor
        .upload()
        .expect("reward distributor");
    suite.external.admin_factory.upload().expect("admin factory");
    suite.external.payroll_factory.upload().expect("payroll factory");
    suite.external.cw_tokenswap.upload().expect("token swap");
    let vesting = store(chain, &VestingNoStaking);
    suite.external.cw_vesting.set_code_id(vesting);

    Codes {
        core: suite.dao_core.code_id().unwrap(),
        proposal_single: suite.proposal.prop_single.code_id().unwrap(),
        cw4_group: suite.voting.cw4_group.code_id().unwrap(),
        voting_cw4: suite.voting.voting_cw4.code_id().unwrap(),
        cw20_base: store(chain, &Cw20BaseWasm),
        cw20_stake: suite.staking.cw20_stake.code_id().unwrap(),
        voting_cw20_staked: suite.voting.voting_cw20_staked.code_id().unwrap(),
        cw721_base: store(chain, &Cw721BaseWasm),
        voting_cw721_staked: suite.voting.voting_cw721_staked.code_id().unwrap(),
        cw721_roles: suite.external.cw721_roles.code_id().unwrap(),
        voting_cw721_roles: suite.voting.voting_cw721_roles.code_id().unwrap(),
        voting_cw20_balance: store(chain, &Cw20BalanceWasm),
    }
}

fn upload_tokenfactory<Chain: CwEnv>(suite: &DaoDaoSuite<Chain>, chain: &Chain) -> (u64, u64) {
    let issuer = store(chain, &TokenfactoryIssuerWasm);
    let voting = store(chain, &TokenStakedWasm);
    suite.external.cw_tokenfactory_issuer.set_code_id(issuer);
    suite.voting.voting_token_staked.set_code_id(voting);
    (issuer, voting)
}

fn root_dao<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    codes: &Codes,
    member: &Addr,
) -> Addr {
    let mut msg = instantiate_msg(
        ROOT,
        codes,
        voting_module(codes, VotingKind::Cw4, member, ROOT),
    );
    msg.proposal_modules_instantiate_info = vec![
        single_with_pre_propose(suite, codes),
        multiple_module(suite),
    ];
    let resp = suite
        .dao_core
        .instantiate(&msg, Some(member), &[])
        .unwrap_or_else(|err| panic!("root instantiate: {err}"));
    // A core instantiate also instantiates voting and proposal children.
    // The first instantiate event is not the core on wasmd.
    let root = dao_named(chain, &resp, ROOT);
    suite.dao_core.set_address(&root);
    assert_power(chain, &root, member, 1);
    // cw4 writes are visible at the next height. Proposal votes read power
    // at the proposal's start height, so that height must be after creation.
    bump(chain);
    root
}

fn single_with_pre_propose<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    codes: &Codes,
) -> ModuleInstantiateInfo {
    let pre_propose = ModuleInstantiateInfo {
        code_id: suite
            .proposal
            .pre_prop_suite
            .pre_prop_single
            .code_id()
            .unwrap(),
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
    };
    ModuleInstantiateInfo {
        code_id: codes.proposal_single,
        msg: to_json_binary(&dao_proposal_single::msg::InstantiateMsg {
            threshold: dao_voting::threshold::Threshold::AbsoluteCount {
                threshold: Uint256::new(1),
            },
            max_voting_period: Duration::Time(86_400),
            min_voting_period: None,
            only_members_execute: false,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::ModuleMayPropose { info: pre_propose },
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "proposal-single".into(),
        salt: None,
    }
}

fn multiple_module<Chain: CwEnv>(suite: &DaoDaoSuite<Chain>) -> ModuleInstantiateInfo {
    ModuleInstantiateInfo {
        code_id: suite.proposal.prop_multiple.code_id().unwrap(),
        msg: to_json_binary(&dao_proposal_multiple::msg::InstantiateMsg {
            voting_strategy: VotingStrategy::SingleChoice {
                quorum: PercentageThreshold::Majority {},
            },
            min_voting_period: None,
            max_voting_period: Duration::Time(86_400),
            only_members_execute: false,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::AnyoneMayPropose {},
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "proposal-multiple".into(),
        salt: None,
    }
}

fn proposal_single<Chain: CwEnv>(chain: &Chain, dao: &Addr, codes: &Codes) -> Addr {
    let modules: Vec<dao_interface::state::ProposalModule> = chain
        .query(
            &dao_interface::msg::QueryMsg::ProposalModules {
                start_after: None,
                limit: None,
            },
            dao,
        )
        .unwrap();
    modules
        .into_iter()
        .find(|module| super::contract_info(chain, &module.address).code_id == codes.proposal_single)
        .unwrap_or_else(|| panic!("{dao} has no proposal-single"))
        .address
}

fn pre_propose_of<Chain: CwEnv>(chain: &Chain, proposal: &Addr) -> Option<Addr> {
    let policy: ProposalCreationPolicy = chain
        .query(
            &dao_proposal_single::msg::QueryMsg::ProposalCreationPolicy {},
            proposal,
        )
        .unwrap();
    match policy {
        ProposalCreationPolicy::Module { addr } => Some(addr),
        ProposalCreationPolicy::Anyone {} => None,
    }
}

fn exercise_pre_propose_and_single<Chain: CwEnv>(chain: &Chain, dao: &Addr, single: &Addr) {
    let pre_propose = pre_propose_of(chain, single).expect("pre-propose module was not installed");
    let set_item = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: dao.to_string(),
        msg: to_json_binary(&dao_interface::msg::ExecuteMsg::SetItem {
            key: "baseline".into(),
            value: "exercised".into(),
        })
        .unwrap(),
        funds: vec![],
    });
    dao_exec(
        chain,
        single,
        Some(&pre_propose),
        vec![set_item],
        "record baseline",
    );
    let proposal: dao_proposal_single::query::ProposalResponse = chain
        .query(
            &dao_proposal_single::msg::QueryMsg::Proposal { proposal_id: 1 },
            single,
        )
        .unwrap();
    assert_eq!(proposal.proposal.status, Status::Executed);
    let item: GetItemResponse = chain
        .query(
            &dao_interface::msg::QueryMsg::GetItem {
                key: "baseline".into(),
            },
            dao,
        )
        .unwrap();
    assert_eq!(item.item.as_deref(), Some("exercised"));
}

fn exercise_multiple<Chain: CwEnv>(chain: &Chain, dao: &Addr, codes: &Codes) {
    let modules: Vec<dao_interface::state::ProposalModule> = chain
        .query(
            &dao_interface::msg::QueryMsg::ProposalModules {
                start_after: None,
                limit: None,
            },
            dao,
        )
        .unwrap();
    let single = proposal_single(chain, dao, codes);
    let multiple = modules
        .into_iter()
        .find(|module| module.address != single)
        .unwrap()
        .address;
    exec(
        chain,
        &multiple,
        &dao_proposal_multiple::msg::ExecuteMsg::Propose(
            dao_voting::proposal::MultipleChoiceProposeMsg {
                title: "pick a path".into(),
                description: "two choices".into(),
                choices: MultipleChoiceOptions {
                    options: vec![
                        MultipleChoiceOption {
                            title: "alpha".into(),
                            description: "first".into(),
                            msgs: vec![],
                        },
                        MultipleChoiceOption {
                            title: "beta".into(),
                            description: "second".into(),
                            msgs: vec![],
                        },
                    ],
                },
                proposer: None,
                vote: None,
            },
        ),
        &[],
    );
    bump(chain);
    exec(
        chain,
        &multiple,
        &dao_proposal_multiple::msg::ExecuteMsg::Vote {
            proposal_id: 1,
            vote: MultipleChoiceVote { option_id: 0 },
            rationale: None,
        },
        &[],
    );
    exec(
        chain,
        &multiple,
        &dao_proposal_multiple::msg::ExecuteMsg::Execute { proposal_id: 1 },
        &[],
    );
    let proposal: dao_proposal_multiple::query::ProposalResponse = chain
        .query(
            &dao_proposal_multiple::msg::QueryMsg::Proposal { proposal_id: 1 },
            &multiple,
        )
        .unwrap();
    assert_eq!(proposal.proposal.status, Status::Executed);
}

fn exercise_delegation<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    member: &Addr,
    dao: &Addr,
    single: &Addr,
) {
    let voting = voting_addr(chain, dao);
    let group: Addr = chain
        .query(&dao_voting_cw4::msg::QueryMsg::GroupContract {}, &voting)
        .unwrap();
    let addr = init(
        chain,
        suite.delegation.vote.code_id().unwrap(),
        &dao_vote_delegation::msg::InstantiateMsg {
            dao: Some(dao.to_string()),
            vp_hook_callers: Some(vec![group.to_string()]),
            no_sync_proposal_modules: None,
            vp_cap_percent: None,
            delegation_validity_blocks: None,
            max_delegations: Some(50),
        },
        "vote-delegation",
        None,
        &[],
    );
    let hook = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: group.to_string(),
        msg: to_json_binary(&serde_json::json!({
            "add_hook": { "addr": addr }
        }))
        .unwrap(),
        funds: vec![],
    });
    dao_exec(
        chain,
        single,
        pre_propose_of(chain, single).as_ref(),
        vec![hook],
        "register delegation hook",
    );
    exec(
        chain,
        &addr,
        &dao_vote_delegation::msg::ExecuteMsg::Register {},
        &[],
    );
    bump(chain);
    let registration: dao_voting::delegation::RegistrationResponse = chain
        .query(
            &dao_vote_delegation::msg::QueryMsg::Registration {
                delegate: member.to_string(),
                height: None,
            },
            &addr,
        )
        .unwrap();
    assert!(registration.registered);
}

fn exercise_distributions<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    member: &Addr,
    dao: &Addr,
) {
    let voting = voting_addr(chain, dao);
    let fund = init(
        chain,
        suite.distribution.fund_distr.code_id().unwrap(),
        &cw_fund_distributor::msg::InstantiateMsg {
            voting_contract: voting.to_string(),
            funding_period: Duration::Time(3_600),
            distribution_height: 1,
        },
        "fund-distributor",
        None,
        &[],
    );
    let power: cw_fund_distributor::msg::TotalPowerResponse = chain
        .query(&cw_fund_distributor::msg::QueryMsg::TotalPower {}, &fund)
        .unwrap();
    assert_eq!(power.total_power, Uint128::new(1));

    let rewards = init(
        chain,
        suite.distribution.reward_distr.code_id().unwrap(),
        &dao_rewards_distributor::msg::InstantiateMsg {
            owner: Some(member.to_string()),
        },
        "rewards-distributor",
        None,
        &[],
    );
    exec(
        chain,
        &rewards,
        &dao_rewards_distributor::msg::ExecuteMsg::Create(dao_rewards_distributor::msg::CreateMsg {
            denom: cw20::UncheckedDenom::Native("utest".into()),
            emission_rate: dao_rewards_distributor::state::EmissionRate::Immediate {},
            vp_contract: voting.to_string(),
            hook_caller: voting.to_string(),
            open_funding: Some(true),
            withdraw_destination: None,
        }),
        &[],
    );
    let distribution: dao_rewards_distributor::state::DistributionState = chain
        .query(
            &dao_rewards_distributor::msg::QueryMsg::Distribution { id: 1 },
            &rewards,
        )
        .unwrap();
    assert_eq!(distribution.id, 1);
}

fn exercise_gauges<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    member: &Addr,
    dao: &Addr,
) {
    let voting = voting_addr(chain, dao);
    let adapter = init(
        chain,
        suite.gauges.adapter.code_id().unwrap(),
        &gauge_adapter::msg::InstantiateMsg {
            owner: member.to_string(),
            required_deposit: None,
            treasury: member.to_string(),
            reward: gauge_adapter::msg::AssetUnchecked {
                denom: cw_denom::UncheckedDenom::Native("utest".into()),
                amount: Uint256::new(1),
            },
            possible_msgs: vec![],
        },
        "gauge-adapter",
        None,
        &[],
    );
    let orchestrator = init(
        chain,
        suite.gauges.orchestrator.code_id().unwrap(),
        &gauge_orchestrator::msg::InstantiateMsg {
            voting_powers: voting.to_string(),
            hook_caller: voting.to_string(),
            owner: member.to_string(),
            gauges: Some(vec![gauge_orchestrator::msg::GaugeConfig {
                title: "baseline".into(),
                adapter: adapter.to_string(),
                epoch_size: 86_400,
                min_percent_selected: None,
                max_options_selected: 1,
                max_available_percentage: None,
                reset_epoch: None,
                total_epochs: None,
            }]),
        },
        "gauge-orchestrator",
        None,
        &[],
    );
    let listed: gauge_orchestrator::msg::ListGaugesResponse = chain
        .query(
            &gauge_orchestrator::msg::QueryMsg::ListGauges {
                start_after: None,
                limit: None,
            },
            &orchestrator,
        )
        .unwrap();
    assert_eq!(listed.gauges.len(), 1);
}

fn exercise_stake_and_rewards<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    member: &Addr,
    dao: &Addr,
) {
    let voting = voting_addr(chain, dao);
    let token: Addr = chain
        .query(
            &dao_voting_cw20_staked::msg::QueryMsg::TokenContract {},
            &voting,
        )
        .unwrap();
    let stake: Addr = chain
        .query(
            &dao_voting_cw20_staked::msg::QueryMsg::StakingContract {},
            &voting,
        )
        .unwrap();
    exec(
        chain,
        &token,
        &cw20::Cw20ExecuteMsg::Send {
            contract: stake.to_string(),
            amount: Uint256::new(1),
            msg: to_json_binary(&cw20_stake::msg::ReceiveMsg::Stake {}).unwrap(),
        },
        &[],
    );
    bump(chain);
    assert_power(chain, dao, member, 1);

    let external = init(
        chain,
        suite.staking.external_rewards.code_id().unwrap(),
        &cw20_stake_external_rewards::msg::InstantiateMsg {
            owner: Some(member.to_string()),
            staking_contract: stake.to_string(),
            reward_token: cw20::Denom::Cw20(token.clone()),
            reward_duration: 1,
        },
        "cw20-external-rewards",
        None,
        &[],
    );
    let info: cw20_stake_external_rewards::msg::InfoResponse = chain
        .query(
            &cw20_stake_external_rewards::msg::QueryMsg::Info {},
            &external,
        )
        .unwrap();
    assert_eq!(info.config.staking_contract, stake);

    let distributor = init(
        chain,
        suite.staking.rewards_distributor.code_id().unwrap(),
        &cw20_stake_reward_distributor::msg::InstantiateMsg {
            owner: member.to_string(),
            staking_addr: stake.to_string(),
            reward_rate: Uint128::new(1),
            reward_token: token.to_string(),
        },
        "cw20-reward-distributor",
        None,
        &[],
    );
    let owned: cw20_stake_reward_distributor::msg::InfoResponse = chain
        .query(
            &cw20_stake_reward_distributor::msg::QueryMsg::Info {},
            &distributor,
        )
        .unwrap();
    assert_eq!(owned.config.staking_addr, stake);
}

fn exercise_nft_stake<Chain: CwEnv>(chain: &Chain, member: &Addr, dao: &Addr) {
    let voting = voting_addr(chain, dao);
    let config: dao_voting_cw721_staked::state::Config = chain
        .query(&dao_voting_cw721_staked::msg::QueryMsg::Config {}, &voting)
        .unwrap();
    exec(
        chain,
        &config.nft_address,
        &cw721_base::msg::ExecuteMsg::SendNft {
            contract: voting.to_string(),
            token_id: "1".into(),
            msg: Binary::default(),
        },
        &[],
    );
    bump(chain);
    assert_power(chain, dao, member, 1);
}

fn exercise_token_staked<Chain: CwEnv>(
    chain: &Chain,
    parent: &Addr,
    codes: &Codes,
    issuer: u64,
    voting_code: u64,
    member: &Addr,
) {
    let name = "token-subdao";
    let voting = ModuleInstantiateInfo {
        code_id: voting_code,
        msg: to_json_binary(&serde_json::json!({
            "token_info": {
                "new": {
                    "token_issuer_code_id": issuer,
                    "token_issuer_salt": null,
                    "subdenom": "vote",
                    "metadata": null,
                    "initial_balances": [{ "amount": "1", "address": member }],
                    "initial_dao_balance": null
                }
            },
            "unstaking_duration": null,
            "active_threshold": null
        }))
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: format!("{name} voting"),
        salt: None,
    };
    let init = instantiate_msg(name, codes, voting);
    let msg = CosmosMsg::Wasm(WasmMsg::Instantiate {
        admin: Some(parent.to_string()),
        code_id: codes.core,
        msg: to_json_binary(&init).unwrap(),
        funds: vec![],
        label: name.to_string(),
    });
    let proposal = proposal_single(chain, parent, codes);
    let resp = dao_exec(
        chain,
        &proposal,
        pre_propose_of(chain, &proposal).as_ref(),
        vec![msg],
        name,
    );
    let child = dao_named(chain, &resp, name);
    let info = super::contract_info(chain, &child);
    assert_eq!(info.admin.as_ref(), Some(parent));
    assert_eq!(info.code_id, codes.core);
    let voting = voting_addr(chain, &child);
    assert_eq!(
        super::contract_info(chain, &voting).code_id,
        voting_code
    );
    let denom: dao_interface::voting::DenomResponse = chain
        .query(&serde_json::json!({ "denom": {} }), &voting)
        .unwrap();
    assert!(
        denom.denom.starts_with("factory/"),
        "tokenfactory denom {}",
        denom.denom
    );
    // Minted tokens have no voting power until they are staked.
    exec(
        chain,
        &voting,
        &serde_json::json!({ "stake": {} }),
        &[Coin::new(1u128, denom.denom)],
    );
    bump(chain);
    assert_power(chain, &child, member, 1);
}

fn exercise_externals<Chain: CwEnv>(
    suite: &DaoDaoSuite<Chain>,
    chain: &Chain,
    member: &Addr,
    codes: &Codes,
    recipient: &Addr,
    counterparty: &Addr,
) {
    let factory = init(
        chain,
        suite.external.admin_factory.code_id().unwrap(),
        &cw_admin_factory::msg::InstantiateMsg { admin: None },
        "admin-factory",
        None,
        &[],
    );
    let child_msg = to_json_binary(&cw20_base::msg::InstantiateMsg {
        name: "Admin Child".into(),
        symbol: "ADM".into(),
        decimals: 6,
        initial_balances: vec![],
        mint: None,
        marketing: None,
    })
    .unwrap();
    let resp = chain
        .execute(
            &cw_admin_factory::msg::ExecuteMsg::InstantiateContractWithSelfAdmin {
                instantiate_msg: child_msg,
                code_id: codes.cw20_base,
                label: "admin-child".into(),
            },
            &[],
            &factory,
        )
        .expect("admin factory instantiates cw20");
    let child = newest_contract(chain, &resp, codes.cw20_base);
    assert_eq!(
        super::contract_info(chain, &child).admin.as_ref(),
        Some(&child)
    );

    let payroll = init(
        chain,
        suite.external.payroll_factory.code_id().unwrap(),
        &cw_payroll_factory::msg::InstantiateMsg {
            owner: Some(member.to_string()),
            vesting_code_id: suite.external.cw_vesting.code_id().unwrap(),
        },
        "payroll-factory",
        None,
        &[],
    );
    chain
        .execute(
            &cw_payroll_factory::msg::ExecuteMsg::InstantiateNativePayrollContract {
                instantiate_msg: cw_vesting::msg::InstantiateMsg {
                    owner: Some(member.to_string()),
                    recipient: recipient.to_string(),
                    title: "payroll".into(),
                    description: None,
                    total: Uint256::new(1),
                    denom: cw_denom::UncheckedDenom::Native("uterp".into()),
                    schedule: cw_vesting::vesting::Schedule::SaturatingLinear,
                    start_time: None,
                    vesting_duration_seconds: 3_600,
                    unbonding_duration_seconds: 1,
                },
                label: "native-payroll".into(),
            },
            &[Coin::new(1u128, "uterp")],
            &payroll,
        )
        .expect("native payroll vests the deposited uterp");
    let payrolls: Vec<cw_payroll_factory::state::VestingContract> = chain
        .query(
            &cw_payroll_factory::msg::QueryMsg::ListVestingContracts {
                start_after: None,
                limit: None,
            },
            &payroll,
        )
        .unwrap();
    assert_eq!(payrolls.len(), 1);

    let swap = init(
        chain,
        suite.external.cw_tokenswap.code_id().unwrap(),
        &cw_token_swap::msg::InstantiateMsg {
            counterparty_one: cw_token_swap::msg::Counterparty {
                address: member.to_string(),
                promise: cw_token_swap::msg::TokenInfo::Native {
                    denom: "uterp".into(),
                    amount: Uint256::new(1),
                },
            },
            counterparty_two: cw_token_swap::msg::Counterparty {
                address: counterparty.to_string(),
                promise: cw_token_swap::msg::TokenInfo::Native {
                    denom: "utest".into(),
                    amount: Uint256::new(1),
                },
            },
        },
        "token-swap",
        None,
        &[],
    );
    let status: cw_token_swap::msg::StatusResponse = chain
        .query(&cw_token_swap::msg::QueryMsg::Status {}, &swap)
        .unwrap();
    assert_eq!(status.counterparty_one.address, *member);
}

fn exercise_polytone<Chain: CwEnv>(suite: &DaoDaoSuite<Chain>) {
    suite.polytone.upload().expect("polytone wasm");
    suite
        .polytone
        .instantiate_immutable()
        .expect("note and voice");
    assert!(suite.polytone.inner.note.address().is_ok());
    assert!(suite.polytone.inner.voice.address().is_ok());
    assert!(suite.polytone.inner.proxy.code_id().is_ok());
}

/// Parent DAO instantiates a child core. The proposal module of the parent
/// is the caller, so this works on a chain where the sender cannot be the DAO.
fn spawn<Chain: CwEnv>(
    chain: &Chain,
    parent: &Addr,
    name: &str,
    codes: &Codes,
    kind: VotingKind,
    member: &Addr,
) -> Addr {
    let init = instantiate_msg(name, codes, voting_module(codes, kind, member, name));
    let msg = CosmosMsg::Wasm(WasmMsg::Instantiate {
        admin: Some(parent.to_string()),
        code_id: codes.core,
        msg: to_json_binary(&init).unwrap(),
        funds: vec![],
        label: name.to_string(),
    });
    let proposal = proposal_single(chain, parent, codes);
    let resp = dao_exec(
        chain,
        &proposal,
        pre_propose_of(chain, &proposal).as_ref(),
        vec![msg],
        name,
    );
    let child = dao_named(chain, &resp, name);
    let info = super::contract_info(chain, &child);
    assert_eq!(info.admin.as_ref(), Some(parent), "{name} wasm admin");
    assert_eq!(info.code_id, codes.core);
    assert_voting_child(chain, &child, kind, codes);
    child
}

fn dao_exec<Chain: CwEnv>(
    chain: &Chain,
    proposal: &Addr,
    pre_propose: Option<&Addr>,
    msgs: Vec<CosmosMsg>,
    title: &str,
) -> Chain::Response {
    let before: u64 = chain
        .query(
            &dao_proposal_single::msg::QueryMsg::ProposalCount {},
            proposal,
        )
        .unwrap();
    if let Some(pre) = pre_propose {
        exec(
            chain,
            pre,
            &dao_pre_propose_single::ExecuteMsg::Propose {
                msg: dao_pre_propose_single::ProposeMessage::Propose {
                    title: title.into(),
                    description: title.into(),
                    msgs,
                    vote: None,
                },
            },
            &[],
        );
    } else {
        exec(
            chain,
            proposal,
            &dao_proposal_single::msg::ExecuteMsg::Propose(SingleChoiceProposeMsg {
                title: title.into(),
                description: title.into(),
                msgs,
                proposer: None,
                vote: None,
            }),
            &[],
        );
    }
    let id = before + 1;
    // Start height must be committed before the vote queries power there.
    bump(chain);
    exec(
        chain,
        proposal,
        &dao_proposal_single::msg::ExecuteMsg::Vote {
            proposal_id: id,
            vote: Vote::Yes,
            rationale: None,
        },
        &[],
    );
    let resp = chain
        .execute(
            &dao_proposal_single::msg::ExecuteMsg::Execute { proposal_id: id },
            &[],
            proposal,
        )
        .unwrap_or_else(|err| panic!("execute proposal {id} ({title}) on {proposal}: {err}"));
    // close_proposal_on_execution_failure keeps the tx successful and records
    // the failure on a wasm attribute. Surface that instead of looking for a
    // child that was never created.
    let failed = resp.events().iter().any(|event| {
        event
            .attributes
            .iter()
            .any(|attr| attr.key == "proposal_execution_failed")
    });
    if failed {
        let err = resp.events().iter().find_map(|event| {
            event
                .attributes
                .iter()
                .find(|attr| attr.key == "error")
                .map(|attr| attr.value.clone())
        });
        panic!("proposal {id} ({title}) execution failed: {err:?}");
    }
    resp
}

fn bump<Chain: QueryHandler>(chain: &Chain) {
    chain
        .next_block()
        .unwrap_or_else(|err| panic!("next block: {err}"));
}

fn assert_power<Chain: CwEnv>(chain: &Chain, dao: &Addr, member: &Addr, expected: u128) {
    let power: dao_interface::voting::VotingPowerAtHeightResponse = chain
        .query(
            &dao_interface::msg::QueryMsg::VotingPowerAtHeight {
                address: member.to_string(),
                height: None,
            },
            dao,
        )
        .unwrap();
    assert_eq!(power.power, Uint256::new(expected), "{dao} voting power");
}

fn init<Chain: CwEnv>(
    chain: &Chain,
    code_id: u64,
    msg: &(impl serde::Serialize + std::fmt::Debug),
    label: &str,
    admin: Option<&Addr>,
    funds: &[Coin],
) -> Addr {
    let resp = chain
        .instantiate(code_id, msg, Some(label), admin, funds)
        .unwrap_or_else(|err| panic!("instantiate {label}: {err}"));
    resp.instantiated_contract_address()
        .unwrap_or_else(|err| panic!("instantiate {label} address: {err}"))
}

fn exec<Chain: CwEnv>(
    chain: &Chain,
    contract: &Addr,
    msg: &(impl serde::Serialize + std::fmt::Debug),
    funds: &[Coin],
) {
    chain
        .execute(msg, funds, contract)
        .unwrap_or_else(|err| panic!("execute {contract}: {err}"));
}

fn store<Chain: CwEnv>(chain: &Chain, source: &impl Uploadable) -> u64 {
    chain
        .upload(source)
        .unwrap_or_else(|err| panic!("upload: {err}"))
        .uploaded_code_id()
        .expect("store_code event")
}

fn dao_named<Chain: CwEnv>(chain: &Chain, resp: &impl IndexResponse, name: &str) -> Addr {
    resp.events()
        .into_iter()
        .flat_map(|event| event.attributes)
        .filter(|attr| attr.key == "_contract_address" || attr.key == "contract_address")
        .map(|attr| attr.value)
        .find_map(|addr| {
            let addr = Addr::unchecked(addr);
            let config: Result<dao_interface::state::Config, _> = chain.query(
                &dao_interface::msg::QueryMsg::Config {},
                &addr,
            );
            config.ok().filter(|config| config.name == name).map(|_| addr)
        })
        .unwrap_or_else(|| panic!("DAO {name} was not instantiated"))
}

fn newest_contract<Chain: CwEnv>(chain: &Chain, resp: &impl IndexResponse, code_id: u64) -> Addr {
    let mut found = None;
    for event in resp.events() {
        for attr in &event.attributes {
            if attr.key != "_contract_address" && attr.key != "contract_address" {
                continue;
            }
            let addr = Addr::unchecked(&attr.value);
            let matches = chain
                .wasm_querier()
                .contract_info(&addr)
                .ok()
                .filter(|info| info.code_id == code_id)
                .is_some();
            if matches {
                found = Some(addr);
            }
        }
    }
    found.unwrap_or_else(|| panic!("no contract with code {code_id}"))
}

fn artifact(rel: &str) -> WasmPath {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../dao-contracts/artifacts")
        .join(rel);
    WasmPath::new(path).unwrap_or_else(|err| panic!("wasm {rel}: {err}"))
}

struct Cw20BaseWasm;
impl Uploadable for Cw20BaseWasm {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        // The default artifact directory has no unsuffixed cw20_base.wasm.
        // This file is only the token payload. Voting modules load from
        // artifacts/ and read the child address from the instantiate event.
        artifact("init2/cw20_base.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        Box::new(
            ContractWrapper::new(
                cw20_base::contract::execute,
                cw20_base::contract::instantiate,
                cw20_base::contract::query,
            )
            .with_migrate(cw20_base::contract::migrate),
        )
    }
}

struct Cw721BaseWasm;
impl Uploadable for Cw721BaseWasm {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        // Same as cw20-base: payload only, not the voting instantiate path.
        artifact("init2/cw721_base.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        Box::new(
            ContractWrapper::new(
                cw721_base::entry::execute,
                cw721_base::entry::instantiate,
                cw721_base::entry::query,
            )
            .with_migrate(cw721_base::entry::migrate),
        )
    }
}

struct Cw20BalanceWasm;
impl Uploadable for Cw20BalanceWasm {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifact("dao_voting_cw20_balance.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        Box::new(
            ContractWrapper::new(
                dao_voting_cw20_balance::contract::execute,
                dao_voting_cw20_balance::contract::instantiate,
                dao_voting_cw20_balance::contract::query,
            )
            .with_reply(dao_voting_cw20_balance::contract::reply),
        )
    }
}

struct VestingNoStaking;
impl Uploadable for VestingNoStaking {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifact("cw_vesting-no_staking.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        Box::new(ContractWrapper::new_with_empty(
            cw_vesting::contract::execute,
            cw_vesting::contract::instantiate,
            cw_vesting::contract::query,
        ))
    }
}

struct TokenfactoryIssuerWasm;
impl Uploadable for TokenfactoryIssuerWasm {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        // terpd registers osmosis.tokenfactory.v1beta1, not the CosmWasm
        // tokenfactory type URL. The cosmwasm build is rejected as an
        // invalid CosmosMsg (wasm code 10).
        artifact("cw_tokenfactory_issuer-osmosis.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        unimplemented!("tokenfactory issuer is uploaded from wasm on terpd")
    }
}

struct TokenStakedWasm;
impl Uploadable for TokenStakedWasm {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        artifact("dao_voting_token_staked-default.wasm")
    }
    fn wrapper() -> Box<dyn MockContract<Empty, Empty>> {
        unimplemented!("token-staked is uploaded from wasm on terpd")
    }
}
