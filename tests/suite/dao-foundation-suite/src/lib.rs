//! Nested subDAO creation on [`dao_testing::DaoDaoSuite`].
//!
//! Each DAO is created by its parent and uses a different voting module.
//! Those modules send `WasmMsg::Instantiate` (`salt` is `None`). The reply
//! reads the instantiate event for the child address: cw4 group, cw20 token
//! and stake, cw721, cw721-roles, and cw20-balance.
//! `dao-voting-token-staked`'s new-token path still predicts the issuer
//! address because the denom is derived from it, then the issuer asks the
//! tokenfactory module to create the denom. The cw-orch mock has no
//! tokenfactory module, so that leg is skipped unless the caller is a chain
//! that has the module.
//!
//! `actions` is the baseline a DAO actually runs: one primitive use of each
//! module family already on [`DaoDaoSuite`]. Bitsong's fantoken factory stays
//! out of this crate (`dao-testing` default features off).

pub mod ibc_ops;

pub mod actions;

use std::str::FromStr;

use cosmwasm_std::{to_json_binary, Addr, Binary, CosmosMsg, Empty, Uint256, WasmMsg};
use cw20::Cw20Coin;
use cw_multi_test::{AppResponse, Executor};
use cw_orch::mock::Mock;
use cw_orch::prelude::*;
use cw_utils::Duration;
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_testing::DaoDaoSuite;

const ROOT: &str = "foundation";

#[derive(Clone, Copy)]
pub enum VotingKind {
    Cw4,
    Cw20Staked,
    Cw721Staked,
    Cw721Roles,
    Cw20Balance,
}

pub struct Codes {
    pub core: u64,
    pub proposal_single: u64,
    pub cw4_group: u64,
    pub voting_cw4: u64,
    pub cw20_base: u64,
    pub cw20_stake: u64,
    pub voting_cw20_staked: u64,
    pub cw721_base: u64,
    pub voting_cw721_staked: u64,
    pub cw721_roles: u64,
    pub voting_cw721_roles: u64,
    pub voting_cw20_balance: u64,
}

fn upload(suite: &DaoDaoSuite<Mock>, chain: &Mock) -> Codes {
    suite.dao_core.upload().unwrap();
    suite.proposal.prop_single.upload().unwrap();
    suite.voting.voting_cw4.upload().unwrap();
    suite.voting.cw4_group.upload().unwrap();
    suite.voting.voting_cw20_staked.upload().unwrap();
    suite.voting.voting_cw721_staked.upload().unwrap();
    suite.voting.voting_cw721_roles.upload().unwrap();
    suite.staking.cw20_stake.upload().unwrap();
    suite.external.cw721_roles.upload().unwrap();

    let cw20_base = uploaded_code_id(
        &chain
            .upload_custom("cw20_base", dao_testing::contracts::cw20_base_contract())
            .unwrap(),
    );
    let cw721_base = uploaded_code_id(
        &chain
            .upload_custom("cw721_base", dao_testing::contracts::cw721_base_contract())
            .unwrap(),
    );
    let voting_cw20_balance = uploaded_code_id(
        &chain
            .upload_custom(
                "voting_cw20_balance",
                dao_testing::contracts::dao_voting_cw20_balance_contract(),
            )
            .unwrap(),
    );

    Codes {
        core: suite.dao_core.code_id().unwrap(),
        proposal_single: suite.proposal.prop_single.code_id().unwrap(),
        cw4_group: suite.voting.cw4_group.code_id().unwrap(),
        voting_cw4: suite.voting.voting_cw4.code_id().unwrap(),
        cw20_base,
        cw20_stake: suite.staking.cw20_stake.code_id().unwrap(),
        voting_cw20_staked: suite.voting.voting_cw20_staked.code_id().unwrap(),
        cw721_base,
        voting_cw721_staked: suite.voting.voting_cw721_staked.code_id().unwrap(),
        cw721_roles: suite.external.cw721_roles.code_id().unwrap(),
        voting_cw721_roles: suite.voting.voting_cw721_roles.code_id().unwrap(),
        voting_cw20_balance,
    }
}

fn uploaded_code_id(resp: &AppResponse) -> u64 {
    resp.events
        .iter()
        .flat_map(|event| event.attributes.iter())
        .find(|attr| attr.key == "code_id")
        .map(|attr| u64::from_str(&attr.value).unwrap())
        .expect("store_code event")
}

fn proposal_module(codes: &Codes, label: &str) -> ModuleInstantiateInfo {
    salted_proposal(codes, label, None)
}

fn salted_proposal(codes: &Codes, label: &str, salt: Option<Binary>) -> ModuleInstantiateInfo {
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
            pre_propose_info: dao_voting::pre_propose::PreProposeInfo::AnyoneMayPropose {},
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .unwrap(),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: label.to_string(),
        salt,
    }
}

pub fn voting_module(
    codes: &Codes,
    kind: VotingKind,
    member: &Addr,
    label: &str,
) -> ModuleInstantiateInfo {
    let (code_id, msg) = match kind {
        VotingKind::Cw4 => (
            codes.voting_cw4,
            to_json_binary(&dao_voting_cw4::msg::InstantiateMsg {
                group_contract: dao_voting_cw4::msg::GroupContract::New {
                    cw4_group_code_id: codes.cw4_group,
                    cw4_group_salt: None,
                    initial_members: vec![cw4::Member {
                        addr: member.to_string(),
                        weight: 1,
                    }],
                },
            })
            .unwrap(),
        ),
        VotingKind::Cw20Staked => (
            codes.voting_cw20_staked,
            to_json_binary(&dao_voting_cw20_staked::msg::InstantiateMsg {
                token_info: dao_voting_cw20_staked::msg::TokenInfo::New {
                    code_id: codes.cw20_base,
                    salt: None,
                    label: format!("{label} token"),
                    name: label.to_string(),
                    symbol: "VOTE".to_string(),
                    decimals: 6,
                    initial_balances: vec![Cw20Coin {
                        address: member.to_string(),
                        amount: Uint256::new(1),
                    }],
                    marketing: None,
                    staking_code_id: codes.cw20_stake,
                    staking_salt: None,
                    unstaking_duration: None,
                    initial_dao_balance: None,
                },
                active_threshold: None,
            })
            .unwrap(),
        ),
        VotingKind::Cw721Staked => (
            codes.voting_cw721_staked,
            to_json_binary(&dao_voting_cw721_staked::msg::InstantiateMsg {
                nft_contract: dao_voting_cw721_staked::msg::NftContract::New {
                    code_id: codes.cw721_base,
                    salt: None,
                    label: format!("{label} nft"),
                    msg: to_json_binary(&cw721_base::msg::InstantiateMsg {
                        name: label.to_string(),
                        symbol: "VOTE".to_string(),
                        minter: Some(member.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    })
                    .unwrap(),
                    initial_nfts: vec![to_json_binary(&cw721_base::msg::ExecuteMsg::Mint {
                        token_id: "1".to_string(),
                        owner: member.to_string(),
                        token_uri: None,
                        extension: Some(Empty {}),
                    })
                    .unwrap()],
                },
                unstaking_duration: None,
                active_threshold: None,
            })
            .unwrap(),
        ),
        VotingKind::Cw721Roles => (
            codes.voting_cw721_roles,
            to_json_binary(&dao_voting_cw721_roles::msg::InstantiateMsg {
                nft_contract: dao_voting_cw721_roles::msg::NftContract::New {
                    code_id: codes.cw721_roles,
                    salt: None,
                    label: format!("{label} roles"),
                    name: label.to_string(),
                    symbol: "ROLE".to_string(),
                    initial_nfts: vec![dao_voting_cw721_roles::msg::NftMintMsg {
                        token_id: "1".to_string(),
                        owner: member.to_string(),
                        token_uri: None,
                        extension: dao_cw721_extensions::roles::MetadataExt {
                            role: None,
                            weight: 1,
                        },
                    }],
                },
            })
            .unwrap(),
        ),
        VotingKind::Cw20Balance => (
            codes.voting_cw20_balance,
            to_json_binary(&dao_voting_cw20_balance::msg::InstantiateMsg {
                token_info: dao_voting_cw20_balance::msg::TokenInfo::New {
                    code_id: codes.cw20_base,
                    salt: None,
                    label: format!("{label} balance"),
                    name: label.to_string(),
                    symbol: "BAL".to_string(),
                    decimals: 6,
                    initial_balances: vec![Cw20Coin {
                        address: member.to_string(),
                        amount: Uint256::new(1),
                    }],
                    marketing: None,
                },
            })
            .unwrap(),
        ),
    };
    ModuleInstantiateInfo {
        code_id,
        msg,
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: label.to_string(),
        salt: None,
    }
}

pub fn instantiate_msg(
    name: &str,
    codes: &Codes,
    voting: ModuleInstantiateInfo,
) -> dao_interface::msg::InstantiateMsg {
    dao_interface::msg::InstantiateMsg {
        admin: None,
        name: name.to_string(),
        description: name.to_string(),
        image_url: None,
        automatically_add_cw20s: false,
        automatically_add_cw721s: false,
        voting_module_instantiate_info: voting,
        proposal_modules_instantiate_info: vec![proposal_module(
            codes,
            &format!("{name}-proposal"),
        )],
        initial_items: None,
        initial_actions: None,
        dao_uri: None,
    }
}

fn dao_named(chain: &Mock, resp: &AppResponse, name: &str) -> Addr {
    resp.events
        .iter()
        .flat_map(|event| event.attributes.iter())
        .filter(|attr| attr.key == "_contract_address" || attr.key == "contract_address")
        .map(|attr| attr.value.clone())
        .find_map(|addr| {
            chain
                .query::<_, dao_interface::state::Config>(
                    &dao_interface::msg::QueryMsg::Config {},
                    &Addr::unchecked(&addr),
                )
                .ok()
                .filter(|config| config.name == name)
                .map(|_| Addr::unchecked(addr))
        })
        .unwrap_or_else(|| panic!("DAO {name} was not instantiated"))
}

fn contract_info<Chain: QueryHandler>(
    chain: &Chain,
    addr: &Addr,
) -> cosmwasm_std::ContractInfoResponse {
    chain
        .wasm_querier()
        .contract_info(addr)
        .unwrap_or_else(|err| panic!("contract {addr} missing: {err}"))
}

/// The voting module sent `WasmMsg::Instantiate` and stored the child from
/// the reply instantiate event. That child exists and names the voting
/// module as its creator.
fn assert_reply_child<Chain: QueryHandler>(
    chain: &Chain,
    voting: &Addr,
    child: &Addr,
    code_id: u64,
) {
    let info = contract_info(chain, child);
    assert_eq!(info.code_id, code_id, "child code id");
    assert_eq!(info.creator, *voting, "instantiate sender");
}

fn voting_addr<Chain: QueryHandler>(chain: &Chain, dao: &Addr) -> Addr {
    chain
        .query(&dao_interface::msg::QueryMsg::VotingModule {}, dao)
        .unwrap()
}

fn assert_voting_child<Chain: QueryHandler>(
    chain: &Chain,
    dao: &Addr,
    kind: VotingKind,
    codes: &Codes,
) {
    let voting = voting_addr(chain, dao);
    let info = contract_info(chain, &voting);
    match kind {
        VotingKind::Cw4 => {
            assert_eq!(info.code_id, codes.voting_cw4);
            let group: Addr = chain
                .query(&dao_voting_cw4::msg::QueryMsg::GroupContract {}, &voting)
                .unwrap();
            assert_reply_child(chain, &voting, &group, codes.cw4_group);
        }
        VotingKind::Cw20Staked => {
            assert_eq!(info.code_id, codes.voting_cw20_staked);
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
            assert_reply_child(chain, &voting, &token, codes.cw20_base);
            assert_reply_child(chain, &voting, &stake, codes.cw20_stake);
        }
        VotingKind::Cw721Staked => {
            assert_eq!(info.code_id, codes.voting_cw721_staked);
            let config: dao_voting_cw721_staked::state::Config = chain
                .query(&dao_voting_cw721_staked::msg::QueryMsg::Config {}, &voting)
                .unwrap();
            assert_reply_child(chain, &voting, &config.nft_address, codes.cw721_base);
        }
        VotingKind::Cw721Roles => {
            assert_eq!(info.code_id, codes.voting_cw721_roles);
            let config: dao_voting_cw721_roles::state::Config = chain
                .query(&dao_voting_cw721_roles::msg::QueryMsg::Config {}, &voting)
                .unwrap();
            assert_reply_child(chain, &voting, &config.nft_address, codes.cw721_roles);
        }
        VotingKind::Cw20Balance => {
            assert_eq!(info.code_id, codes.voting_cw20_balance);
            let token: Addr = chain
                .query(
                    &dao_voting_cw20_balance::msg::QueryMsg::TokenContract {},
                    &voting,
                )
                .unwrap();
            assert_reply_child(chain, &voting, &token, codes.cw20_base);
        }
    }
}

fn spawn(
    suite: &mut DaoDaoSuite<Mock>,
    chain: &Mock,
    parent_key: &str,
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
    let resp = suite
        .execute_as_dao_by_key(parent_key, vec![msg])
        .unwrap_or_else(|err| panic!("parent {parent_key} failed to create {name}: {err}"));
    let child = dao_named(chain, &resp, name);
    let info = contract_info(chain, &child);
    assert_eq!(info.admin.as_ref(), Some(parent), "{name} wasm admin");
    assert_eq!(info.code_id, codes.core);
    assert_voting_child(chain, &child, kind, codes);
    suite.save_dao(name, child.clone());
    child
}

#[test]
fn nested_subdaos_read_child_addresses_from_reply_events() {
    let chain = Mock::new("foundation");
    let member = chain.sender_addr();
    let mut suite = DaoDaoSuite::new(chain.clone());
    let codes = upload(&suite, &chain);

    let root_msg = instantiate_msg(
        ROOT,
        &codes,
        voting_module(&codes, VotingKind::Cw4, &member, ROOT),
    );
    suite
        .dao_core
        .instantiate(&root_msg, Some(&member), &[])
        .unwrap();
    let root = suite.dao_core.address().unwrap();
    suite.snapshot_current_dao(ROOT).unwrap();
    assert_voting_child(&chain, &root, VotingKind::Cw4, &codes);

    let cw20 = spawn(
        &mut suite,
        &chain,
        ROOT,
        &root,
        "cw20-subdao",
        &codes,
        VotingKind::Cw20Staked,
        &member,
    );
    let cw721 = spawn(
        &mut suite,
        &chain,
        "cw20-subdao",
        &cw20,
        "cw721-subdao",
        &codes,
        VotingKind::Cw721Staked,
        &member,
    );
    let roles = spawn(
        &mut suite,
        &chain,
        "cw721-subdao",
        &cw721,
        "cw721-roles-subdao",
        &codes,
        VotingKind::Cw721Roles,
        &member,
    );
    let _balance = spawn(
        &mut suite,
        &chain,
        "cw721-roles-subdao",
        &roles,
        "cw20-balance-subdao",
        &codes,
        VotingKind::Cw20Balance,
        &member,
    );
}

/// Two proposal modules in one instantiate, both with `salt: None`.
/// Ordinary instantiate does not derive either address. Both exist, they
/// differ, and the DAO is the creator because core sent the messages.
#[test]
fn two_proposal_modules_instantiate_without_salt() {
    let chain = Mock::new("reply-props");
    let member = chain.sender_addr();
    let suite = DaoDaoSuite::new(chain.clone());
    let codes = upload(&suite, &chain);
    let mut msg = instantiate_msg(
        "reply-dao",
        &codes,
        voting_module(&codes, VotingKind::Cw4, &member, "reply-dao"),
    );
    msg.proposal_modules_instantiate_info = vec![
        salted_proposal(&codes, "prop-a", None),
        salted_proposal(&codes, "prop-b", None),
    ];
    suite
        .dao_core
        .instantiate(&msg, Some(&member), &[])
        .unwrap();
    let dao = suite.dao_core.address().unwrap();
    let modules: Vec<dao_interface::state::ProposalModule> = chain
        .query(
            &dao_interface::msg::QueryMsg::ProposalModules {
                start_after: None,
                limit: None,
            },
            &dao,
        )
        .unwrap();
    assert_eq!(modules.len(), 2);
    assert_ne!(modules[0].address, modules[1].address);
    for module in &modules {
        let info = contract_info(&chain, &module.address);
        assert_eq!(info.creator, dao);
        assert_eq!(info.code_id, codes.proposal_single);
    }
}
