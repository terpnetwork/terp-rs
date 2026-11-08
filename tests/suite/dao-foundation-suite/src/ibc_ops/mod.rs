//! DAO treasury scripting: native stake + bank send, composable with subDAO instantiate.

pub mod native_delegation;
pub mod quorum;
pub mod steward;

pub use native_delegation::{
    CODE_CORE, CODE_CW4_GROUP, CODE_PRE_PROPOSE_SINGLE, CODE_PROPOSAL_MULTIPLE,
    CODE_PROPOSAL_SINGLE, CODE_VOTING_CW4, FIAB_DESCRIPTION, FIAB_NAME, GAS_DENOM, MICRO,
    NEW_VALIDATOR_ACCOUNT, NEW_VALIDATOR_OPERATOR, NativeDelegationPlan, PARENT_CORE,
    PRE_PROPOSE_A, PROPOSAL_A, STAKE_DENOM, bank_send_msg, coin, delegate_any_msg, delegate_msg,
    encode_msg_delegate, execute_proposal_msg, instantiate_subdao_msg, million_tokens, terp_100k,
    terp_200k, thiol_7_10, to_valoper, vote_yes_msg,
};

use cosmwasm_std::{Coin, CosmosMsg, Decimal, Uint256, WasmMsg, to_json_binary};
use cw_utils::Duration;
use cw4::Member;
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_proposal_single::msg::ExecuteMsg as ProposalExecuteMsg;
use dao_voting::{
    pre_propose::PreProposeInfo,
    threshold::{PercentageThreshold, Threshold},
};

pub fn foundation_native_msgs(other_validators: Vec<String>) -> Vec<CosmosMsg> {
    NativeDelegationPlan::bonded_set(other_validators).cosmos_msgs()
}

/// Seed attached to the FIAB instantiate: 1,000,000 TERP and 1,000,000 THIOL.
pub fn fiab_seed() -> Vec<Coin> {
    vec![
        coin(million_tokens(), STAKE_DENOM),
        coin(million_tokens(), GAS_DENOM),
    ]
}

/// Sole FIAB member.
pub const FIAB_MEMBER: &str = "terp1fazvzt6pdpm4lg6t73h0j4dsyuc8tdxwrlwq9j";

pub fn fiab_members() -> Vec<Member> {
    vec![Member {
        addr: FIAB_MEMBER.into(),
        weight: 1,
    }]
}

/// FIAB-SUBDAO instantiate under `parent` using morocco-1 code IDs.
pub fn morocco_fiab_instantiate(parent: &str) -> CosmosMsg {
    let voting = ModuleInstantiateInfo {
        code_id: CODE_VOTING_CW4,
        msg: to_json_binary(&dao_voting_cw4::msg::InstantiateMsg {
            group_contract: dao_voting_cw4::msg::GroupContract::New {
                cw4_group_code_id: CODE_CW4_GROUP,
                cw4_group_salt: None,
                initial_members: fiab_members(),
            },
        })
        .expect("fiab voting msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "fiab-voting-cw4".into(),
        salt: None,
    };
    let proposal = ModuleInstantiateInfo {
        code_id: CODE_PROPOSAL_SINGLE,
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
        .expect("fiab proposal msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "fiab-proposal-single".into(),
        salt: None,
    };
    let init = dao_interface::msg::InstantiateMsg {
        admin: Some(parent.to_string()),
        name: FIAB_NAME.into(),
        description: FIAB_DESCRIPTION.into(),
        image_url: None,
        automatically_add_cw20s: true,
        automatically_add_cw721s: true,
        voting_module_instantiate_info: voting,
        proposal_modules_instantiate_info: vec![proposal],
        initial_items: None,
        initial_actions: None,
        dao_uri: None,
    };
    instantiate_subdao_msg(
        parent,
        CODE_CORE,
        FIAB_NAME,
        to_json_binary(&init).expect("fiab core msg").to_vec(),
        fiab_seed(),
    )
}

pub const BOUNTY_NAME: &str = "BOUNTY-SUBDAO";
pub const BOUNTY_PROPOSAL_TITLE: &str = "Create BOUNTY-SUBDAO with 1,000,000 TERP";

/// Seed attached to the bounty subDAO: 1,000,000 TERP.
pub fn bounty_seed() -> Vec<Coin> {
    vec![coin(million_tokens(), STAKE_DENOM)]
}

pub fn bounty_proposal_description(bounty_code_id: u64) -> String {
    format!(
        "Spawn BOUNTY-SUBDAO to steward bounties. This DAO stays admin. \
One cw4 member. Proposal module A is dao-proposal-single (code {CODE_PROPOSAL_SINGLE}): \
one yes passes, only members execute, voting lasts 24 hours, and anyone in the group may propose. \
Proposal module B is the bounties contract (code {bounty_code_id}). \
The instantiate attaches 1000000000000uterp (1,000,000 TERP)."
    )
}

/// Bounty subDAO under `parent`. Module A is single-choice. Module B is the
/// bounties proposal contract, whose instantiate message is supplied by the caller.
pub fn morocco_bounty_instantiate(
    parent: &str,
    member: &str,
    bounty_module: ModuleInstantiateInfo,
) -> CosmosMsg {
    let voting = ModuleInstantiateInfo {
        code_id: CODE_VOTING_CW4,
        msg: to_json_binary(&dao_voting_cw4::msg::InstantiateMsg {
            group_contract: dao_voting_cw4::msg::GroupContract::New {
                cw4_group_code_id: CODE_CW4_GROUP,
                cw4_group_salt: None,
                initial_members: vec![Member {
                    addr: member.to_string(),
                    weight: 1,
                }],
            },
        })
        .expect("bounty voting msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "bounty-voting-cw4".into(),
        salt: None,
    };
    let single = ModuleInstantiateInfo {
        code_id: CODE_PROPOSAL_SINGLE,
        msg: to_json_binary(&dao_proposal_single::msg::InstantiateMsg {
            threshold: Threshold::AbsoluteCount {
                threshold: Uint256::from(1u64),
            },
            max_voting_period: Duration::Time(86400),
            min_voting_period: None,
            only_members_execute: true,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::AnyoneMayPropose {},
            close_proposal_on_execution_failure: true,
            veto: None,
            delegation_module: None,
        })
        .expect("bounty single-choice msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: "bounty-proposal-single".into(),
        salt: None,
    };
    let init = dao_interface::msg::InstantiateMsg {
        admin: Some(parent.to_string()),
        name: BOUNTY_NAME.into(),
        description: bounty_proposal_description(bounty_module.code_id),
        image_url: None,
        automatically_add_cw20s: true,
        automatically_add_cw721s: true,
        voting_module_instantiate_info: voting,
        proposal_modules_instantiate_info: vec![single, bounty_module],
        initial_items: None,
        initial_actions: None,
        dao_uri: None,
    };
    instantiate_subdao_msg(
        parent,
        CODE_CORE,
        BOUNTY_NAME,
        to_json_binary(&init).expect("bounty core msg").to_vec(),
        bounty_seed(),
    )
}

pub fn foundation_proposal_wrapper(
    title: &str,
    description: &str,
    msgs: Vec<CosmosMsg>,
) -> serde_json::Value {
    dao_testing::pre_propose_single_body(title, description, msgs)
}

pub const FOUNDATION_TITLE: &str = "Say fuck it and begin";
/// Reproposal of proposal 4 (execution_failed). Title, DAO name and description,
/// codes, member, thresholds and periods match it. Only the seed differs: proposal 4
/// sent 1,000,000 micro-units (1 TERP, 1 THIOL); this sends 1,000,000 whole tokens.
pub const FIAB_PROPOSAL_TITLE: &str = FOUNDATION_TITLE;
pub const FOUNDATION_DESCRIPTION: &str = "\
'Say Fuck It And Begin'. A forward deployment subdao for Terp Network Foundation, \
deploying and interacting with Terp Network's ecosystem in a minimally constrained \
structure. Allows us to curate data points, and reflect for improving the stewardship \
and collaborative support of growth.

This DAO stays admin. Seed 1,000,000 TERP and 1,000,000 THIOL. One member.";

pub const DELEGATION_TITLE: &str = "Delegate 100,000 TERP to every bonded validator";
pub const DELEGATION_DESCRIPTION: &str =
    "One hundred thousand TERP from this treasury onto each bonded validator.";

pub const DROP_VETO_TITLE: &str = "Drop the seven-day execution lock";
pub const DROP_VETO_DESCRIPTION: &str = "\
After a vote passes, this module waits seven days so chain governance can veto. \
We do not need that. Voting already runs thirty-two hours. Turn the veto off.

Passing rules stay the same: majority, 20% quorum, 32-hour window.

This does not change proposals 3 and 4. They already carry the lock.";

pub const BLOCKPRO_DELEGATE_TITLE: &str = "Delegate 100,000 TERP to BlockPro";
pub const BLOCKPRO_DELEGATE_DESCRIPTION: &str = "\
Stake 100,000 TERP with BlockPro \
(terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk).";

pub const FIAB_TOPUP_TITLE: &str = "Seed FIAB with 1,000,000 TERP and 1,000,000 THIOL";
pub const FIAB_TOPUP_DESCRIPTION: &str = "\
Proposal 4 only sent 1 TERP and 1 THIOL with the instantiate. Send one million \
TERP and one million THIOL (1000000000000 uterp and 1000000000000 uthiol) to the \
FIAB core once it exists.";

/// Exact UpdateConfig body of morocco-1 proposal 5. Only `veto` changes.
pub fn drop_veto_update_config(dao: impl Into<String>) -> ProposalExecuteMsg {
    ProposalExecuteMsg::UpdateConfig {
        threshold: Threshold::ThresholdQuorum {
            threshold: PercentageThreshold::Majority {},
            quorum: PercentageThreshold::Percent(Decimal::percent(20)),
        },
        max_voting_period: Duration::Time(115200),
        min_voting_period: None,
        only_members_execute: false,
        allow_revoting: false,
        dao: dao.into(),
        close_proposal_on_execution_failure: true,
        veto: None,
    }
}

pub fn drop_veto_msg_for(proposal_module: impl Into<String>, dao: impl Into<String>) -> CosmosMsg {
    CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: proposal_module.into(),
        msg: to_json_binary(&drop_veto_update_config(dao)).expect("update config"),
        funds: vec![],
    })
}

/// Keep live threshold/period; only `veto` goes to none. Targets proposal A.
pub fn drop_veto_msg() -> CosmosMsg {
    drop_veto_msg_for(PROPOSAL_A, PARENT_CORE)
}

pub fn blockpro_delegate_msg() -> CosmosMsg {
    delegate_any_msg(
        PARENT_CORE,
        NEW_VALIDATOR_OPERATOR,
        terp_100k(),
        STAKE_DENOM,
    )
}

pub fn fiab_topup_msg(fiab_core: &str) -> CosmosMsg {
    CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: fiab_core.into(),
        amount: vec![
            Coin::new(million_tokens(), STAKE_DENOM),
            Coin::new(million_tokens(), GAS_DENOM),
        ],
    })
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    #[test]
    fn fiab_is_sole_named_member() {
        let m = fiab_members();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].addr, FIAB_MEMBER);
        assert_eq!(m[0].weight, 1);
    }

    #[test]
    fn pre_propose_body_nests_the_proposal() {
        let body = foundation_proposal_wrapper("title", "desc", vec![]);
        assert_eq!(body["propose"]["msg"]["propose"]["title"], "title");
        assert_eq!(body["propose"]["msg"]["propose"]["description"], "desc");
        assert!(
            body["propose"]["msg"]["propose"]["msgs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(body["propose"]["msg"]["propose"].get("vote").is_none());
    }

    #[test]
    fn bounty_subdao_has_single_choice_then_bounties_module_and_one_million_terp() {
        let bounty_module = ModuleInstantiateInfo {
            code_id: 99,
            msg: to_json_binary(&serde_json::json!({"bounties": true})).unwrap(),
            admin: Some(Admin::CoreModule {}),
            funds: None,
            label: "bounty-bounties".into(),
            salt: None,
        };
        let msg = morocco_bounty_instantiate(PARENT_CORE, "terp1member", bounty_module);
        let CosmosMsg::Wasm(WasmMsg::Instantiate {
            admin,
            code_id,
            msg,
            funds,
            label,
        }) = msg
        else {
            panic!("bounty spawn is a wasm instantiate");
        };
        assert_eq!(admin.as_deref(), Some(PARENT_CORE));
        assert_eq!(code_id, CODE_CORE);
        assert_eq!(label, BOUNTY_NAME);
        assert_eq!(funds, bounty_seed());
        assert_eq!(funds.len(), 1);
        assert_eq!(funds[0].denom, "uterp");
        assert_eq!(funds[0].amount, Uint256::from(1_000_000_000_000u128));
        let init: dao_interface::msg::InstantiateMsg = cosmwasm_std::from_json(msg).unwrap();
        assert_eq!(init.admin.as_deref(), Some(PARENT_CORE));
        assert_eq!(init.name, BOUNTY_NAME);
        assert_eq!(init.proposal_modules_instantiate_info.len(), 2);
        assert_eq!(
            init.proposal_modules_instantiate_info[0].code_id,
            CODE_PROPOSAL_SINGLE
        );
        assert_eq!(
            init.proposal_modules_instantiate_info[0].label,
            "bounty-proposal-single"
        );
        assert_eq!(init.proposal_modules_instantiate_info[1].code_id, 99);
        assert_eq!(
            init.proposal_modules_instantiate_info[1].label,
            "bounty-bounties"
        );
        let voting: dao_voting_cw4::msg::InstantiateMsg =
            cosmwasm_std::from_json(&init.voting_module_instantiate_info.msg).unwrap();
        let dao_voting_cw4::msg::GroupContract::New {
            initial_members, ..
        } = voting.group_contract
        else {
            panic!("bounty group is a new cw4");
        };
        assert_eq!(initial_members.len(), 1);
        assert_eq!(initial_members[0].addr, "terp1member");
        assert_eq!(initial_members[0].weight, 1);
    }

    #[test]
    fn fiab_seed_is_one_million_of_each_denom() {
        let seed = fiab_seed();
        assert_eq!(seed.len(), 2);
        assert_eq!(seed[0].denom, "uterp");
        assert_eq!(seed[0].amount, Uint256::from(1_000_000_000_000u128));
        assert_eq!(seed[1].denom, "uthiol");
        assert_eq!(seed[1].amount, Uint256::from(1_000_000_000_000u128));
    }

    #[test]
    fn drop_veto_payload_matches_on_chain_prop5() {
        let v: serde_json::Value = serde_json::from_slice(
            &cosmwasm_std::to_json_binary(&drop_veto_update_config(PARENT_CORE)).unwrap(),
        )
        .unwrap();
        let u = &v["update_config"];
        assert!(u["veto"].is_null(), "veto must be JSON null, got {u}");
        assert_eq!(u["dao"], PARENT_CORE);
        assert_eq!(u["max_voting_period"]["time"], 115200);
        assert!(u["min_voting_period"].is_null());
        assert_eq!(u["only_members_execute"], false);
        assert_eq!(u["allow_revoting"], false);
        assert_eq!(u["close_proposal_on_execution_failure"], true);
        assert_eq!(
            u["threshold"]["threshold_quorum"]["threshold"]["majority"],
            serde_json::json!({})
        );
        assert_eq!(
            u["threshold"]["threshold_quorum"]["quorum"]["percent"],
            "0.2"
        );
    }
}
