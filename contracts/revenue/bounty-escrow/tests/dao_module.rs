//! DAO DAO proposal-module surface: Info, Dao, proposal list queries, creation gates
//! (anyone / min stake / counterparty TODO), pre-propose module modes and proposal hooks.
//! cw-multi-test only (local in-memory chain, MockApiBech32 test keys).
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, Binary, Decimal, Deps, DepsMut, Empty, Env, MessageInfo, Response, StakingMsg, StdError,
    StdResult, Uint256, Validator, WasmMsg, coin, coins, testing::mock_env, to_json_binary,
};
use cw_multi_test::{
    App, AppBuilder, BankKeeper, Contract, ContractWrapper, Executor, MockApiBech32, StakingInfo,
};
use cw_storage_plus::Item;
use dao_interface::state::ModuleInstantiateInfo;
use dao_interface::voting::InfoResponse;
use terp_bounty_escrow::contract::{
    CONTRACT_NAME, CONTRACT_VERSION, execute, instantiate, migrate, query, reply,
};
use terp_bounty_escrow::msg::{
    BountyProposalListResponse, BountyProposalResponse, ExecuteMsg, InstantiateMsg, PreProposeInfo,
    ProposalCreationPolicy, ProposerGateMsg, QueryMsg,
};
use terp_bounty_escrow::state::{Bounty, BountyStatus, CreationGate, DenomRule};

type TApp = App<BankKeeper, MockApiBech32>;
const UTERP: &str = "uterp";
const DAY: u64 = 86_400;
const HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

fn escrow_contract() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(execute, instantiate, query)
            .with_reply(reply)
            .with_migrate(|d, e, m: terp_bounty_escrow::msg::MigrateMsg, _i| migrate(d, e, m)),
    )
}

// ---- mock pre-propose module: forwards CreateBounty (with the deposit) to the escrow that
// ---- instantiated it, and accepts dao-pre-propose-base's ProposalCompletedHook.
const ESCROW: Item<Addr> = Item::new("escrow");
const COMPLETED: Item<u64> = Item::new("completed");

#[cw_serde]
enum PreMsg {
    Forward {
        msg: Binary,
    },
    ProposalCompletedHook {
        proposal_id: u64,
        new_status: dao_voting::status::Status,
    },
}
#[cw_serde]
enum PreQuery {
    Completed {},
}

fn pre_propose_contract() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(
        |deps: DepsMut, _e: Env, info: MessageInfo, msg: PreMsg| -> StdResult<Response> {
            let escrow = ESCROW.load(deps.storage)?;
            match msg {
                PreMsg::Forward { msg } => Ok(Response::new().add_message(WasmMsg::Execute {
                    contract_addr: escrow.to_string(),
                    msg,
                    funds: info.funds,
                })),
                PreMsg::ProposalCompletedHook { .. } => {
                    if info.sender != escrow {
                        return Err(StdError::msg("only the proposal module"));
                    }
                    let n = COMPLETED.may_load(deps.storage)?.unwrap_or(0);
                    COMPLETED.save(deps.storage, &(n + 1))?;
                    Ok(Response::new())
                }
            }
        },
        |deps: DepsMut, _e: Env, info: MessageInfo, _m: Empty| -> StdResult<Response> {
            ESCROW.save(deps.storage, &info.sender)?;
            Ok(Response::new())
        },
        |deps: Deps, _e: Env, _q: PreQuery| -> StdResult<Binary> {
            to_json_binary(&COMPLETED.may_load(deps.storage)?.unwrap_or(0))
        },
    ))
}

// ---- mock proposal hook receiver: counts dao-hooks ProposalHook messages; can be told to fail.
const HOOKS_SEEN: Item<u64> = Item::new("seen");
#[cw_serde]
enum HookExec {
    ProposalHook(dao_hooks::proposal::ProposalHookMsg),
}
const FAIL: Item<bool> = Item::new("fail");
/// Instantiate msg = `true` for a receiver that always errors.
fn hook_contract() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(
        |deps: DepsMut, _e: Env, _i: MessageInfo, _m: HookExec| -> StdResult<Response> {
            if FAIL.load(deps.storage)? {
                return Err(StdError::msg("hook receiver is broken"));
            }
            let n = HOOKS_SEEN.may_load(deps.storage)?.unwrap_or(0);
            HOOKS_SEEN.save(deps.storage, &(n + 1))?;
            Ok(Response::new())
        },
        |deps: DepsMut, _e: Env, _i: MessageInfo, fail: bool| -> StdResult<Response> {
            FAIL.save(deps.storage, &fail)?;
            Ok(Response::new())
        },
        |deps: Deps, _e: Env, _q: Empty| -> StdResult<Binary> {
            to_json_binary(&HOOKS_SEEN.may_load(deps.storage)?.unwrap_or(0))
        },
    ))
}

struct S {
    app: TApp,
    code: u64,
    escrow: Addr,
    dao: Addr,
    approver: Addr,
    proposer: Addr,
    rando: Addr,
    validator: String,
}

fn base(s: &S) -> InstantiateMsg {
    InstantiateMsg {
        owner: s.dao.to_string(),
        approver: s.approver.to_string(),
        gate: ProposerGateMsg {
            use_allowlist: true,
            min_stake: None,
            cw4_group: None,
        },
        denoms: vec![DenomRule {
            denom: UTERP.into(),
            min_amount: Uint256::new(1_000_000),
            max_amount: Uint256::new(1_000_000_000),
        }],
        allow_partial_payouts: false,
        fee_bps: 0,
        fee_collector: None,
        min_duration_secs: DAY,
        max_duration_secs: 90 * DAY,
        params_version: "test-dao-module".into(),
        params_placeholder: true,
        min_submit_stake: Uint256::zero(),
        min_upload_stake: Uint256::zero(),
        initial_allowlist: vec![s.proposer.to_string()],
        dao: None,
        pre_propose_info: None,
        creation_gate: None,
    }
}

/// Instantiated by `dao` (the DAO core instantiates its proposal modules), unless the msg says otherwise.
fn setup(f: impl FnOnce(&S, InstantiateMsg) -> InstantiateMsg) -> S {
    let api = MockApiBech32::new("terp");
    let dao = api.addr_make("dao-core");
    let approver = api.addr_make("approver");
    let proposer = api.addr_make("proposer");
    let rando = api.addr_make("rando");
    let validator = api.addr_make("validator").to_string();
    let funded = vec![
        proposer.clone(),
        rando.clone(),
        approver.clone(),
        dao.clone(),
    ];
    let v = validator.clone();
    let mut app = AppBuilder::default()
        .with_api(api)
        .build(|router, api, storage| {
            for a in &funded {
                router
                    .bank
                    .init_balance(storage, a, vec![coin(1_000_000_000_000, UTERP)])
                    .unwrap();
            }
            router
                .staking
                .setup(
                    storage,
                    StakingInfo {
                        bonded_denom: UTERP.into(),
                        unbonding_time: 60,
                        apr: Decimal::percent(10).into(),
                    },
                )
                .unwrap();
            router
                .staking
                .add_validator(
                    api,
                    storage,
                    &mock_env().block,
                    Validator::create(
                        v,
                        Decimal::percent(5),
                        Decimal::percent(10),
                        Decimal::percent(1),
                    ),
                )
                .unwrap();
        });
    app.update_block(|b| b.chain_id = "terp-local-1".into());
    let code = app.store_code(escrow_contract());
    let mut s = S {
        app,
        code,
        escrow: Addr::unchecked(""),
        dao,
        approver,
        proposer,
        rando,
        validator,
    };
    let msg = f(&s, base(&s));
    s.escrow = s
        .app
        .instantiate_contract(
            code,
            s.dao.clone(),
            &msg,
            &[],
            "bounty-escrow",
            Some(s.dao.to_string()),
        )
        .unwrap();
    s
}

fn create_msg(r: &str, exp: cosmwasm_std::Timestamp, proposer: Option<String>) -> ExecuteMsg {
    ExecuteMsg::CreateBounty {
        bounty_ref: r.into(),
        title: format!("bounty {r}"),
        summary: "A short summary.".into(),
        details_hash: HASH.into(),
        details_cid: None,
        reward: coin(2_000_000, UTERP),
        expires_at: exp,
        proposer,
    }
}

impl S {
    fn exp(&self) -> cosmwasm_std::Timestamp {
        self.app.block_info().time.plus_seconds(30 * DAY)
    }
    fn create(&mut self, who: &Addr, r: &str) -> StdResult<cw_multi_test::AppResponse> {
        let m = create_msg(r, self.exp(), None);
        self.app.execute_contract(
            who.clone(),
            self.escrow.clone(),
            &m,
            &coins(2_000_000, UTERP),
        )
    }
    fn q<T: serde::de::DeserializeOwned>(&self, m: &QueryMsg) -> T {
        self.app.wrap().query_wasm_smart(&self.escrow, m).unwrap()
    }
    fn delegate(&mut self, who: &Addr, amount: u128) {
        let v = self.validator.clone();
        self.app
            .execute(
                who.clone(),
                StakingMsg::Delegate {
                    validator: v,
                    amount: coin(amount, UTERP),
                }
                .into(),
            )
            .unwrap();
    }
}

fn err_str(e: StdError) -> String {
    format!("{e:?}")
}

#[test]
fn info_returns_cw2_name_and_version() {
    let s = setup(|_, m| m);
    let r: InfoResponse = s.q(&QueryMsg::Info {});
    assert_eq!(r.info.contract, CONTRACT_NAME);
    assert_eq!(r.info.version, CONTRACT_VERSION);
}

#[test]
fn dao_defaults_to_instantiator_or_explicit_address() {
    let s = setup(|_, m| m);
    let d: Addr = s.q(&QueryMsg::Dao {});
    assert_eq!(d, s.dao);
    let other = MockApiBech32::new("terp").addr_make("other-dao");
    let o = other.to_string();
    let s = setup(move |_, mut m| {
        m.dao = Some(o);
        m
    });
    let d: Addr = s.q(&QueryMsg::Dao {});
    assert_eq!(d, other);
}

#[test]
fn proposal_list_reverse_count_and_single_queries() {
    let mut s = setup(|_, m| m);
    let p = s.proposer.clone();
    assert_eq!(s.q::<u64>(&QueryMsg::ProposalCount {}), 0);
    assert_eq!(s.q::<u64>(&QueryMsg::NextProposalId {}), 1);
    for r in ["a", "b", "c"] {
        s.create(&p, r).unwrap();
    }
    assert_eq!(s.q::<u64>(&QueryMsg::ProposalCount {}), 3);
    assert_eq!(s.q::<u64>(&QueryMsg::NextProposalId {}), 4);
    let l: BountyProposalListResponse = s.q(&QueryMsg::ListProposals {
        start_after: None,
        limit: None,
    });
    assert_eq!(
        l.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let l: BountyProposalListResponse = s.q(&QueryMsg::ListProposals {
        start_after: Some(1),
        limit: Some(1),
    });
    assert_eq!(
        l.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![2]
    );
    let r: BountyProposalListResponse = s.q(&QueryMsg::ReverseProposals {
        start_before: None,
        limit: None,
    });
    assert_eq!(
        r.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    let r: BountyProposalListResponse = s.q(&QueryMsg::ReverseProposals {
        start_before: Some(3),
        limit: Some(1),
    });
    assert_eq!(
        r.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![2]
    );
    let one: BountyProposalResponse = s.q(&QueryMsg::Proposal { proposal_id: 2 });
    assert_eq!(one.id, 2);
    assert_eq!(one.proposal.bounty_ref, "b");
    assert_eq!(one.proposal.status, BountyStatus::Pending);
}

#[test]
fn creation_gate_anyone_lets_any_funded_sender_create() {
    // default gate (allowlist) refuses rando
    let mut s = setup(|_, m| m);
    let r = s.rando.clone();
    assert!(err_str(s.create(&r, "x").unwrap_err()).contains("not on the Terp staker whitelist"));
    // (a) Anyone: no allowlist entry, no stake needed; the ProposerGate may be all-disabled
    let mut s = setup(|_, mut m| {
        m.creation_gate = Some(CreationGate::Anyone {});
        m.gate = ProposerGateMsg {
            use_allowlist: false,
            min_stake: None,
            cw4_group: None,
        };
        m
    });
    let r = s.rando.clone();
    let res = s.create(&r, "x").unwrap();
    assert!(
        res.events
            .iter()
            .flat_map(|e| &e.attributes)
            .any(|a| a.key == "eligible_via" && a.value == "anyone")
    );
    let b: Bounty = s.q(&QueryMsg::Bounty { id: 1 });
    assert_eq!(b.proposer, s.rando);
    assert_eq!(
        b.status,
        BountyStatus::Pending,
        "listing still needs the approver"
    );
}

#[test]
fn creation_gate_min_stake_uses_bonded_stake() {
    let mut s = setup(|_, mut m| {
        m.creation_gate = Some(CreationGate::MinStake {
            amount: Uint256::new(50_000_000),
        });
        m
    });
    let r = s.rando.clone();
    assert!(err_str(s.create(&r, "x").unwrap_err()).contains("not on the Terp staker whitelist"));
    s.delegate(&r, 49_999_999);
    assert!(s.create(&r, "x").is_err());
    s.delegate(&r, 1);
    let res = s.create(&r, "x").unwrap();
    assert!(
        res.events
            .iter()
            .flat_map(|e| &e.attributes)
            .any(|a| a.key == "eligible_via" && a.value == "creation_min_stake")
    );
    // the allowlisted proposer is NOT admitted by the allowlist under MinStake
    let p = s.proposer.clone();
    assert!(s.create(&p, "y").is_err());
}

#[test]
fn creation_gate_validation_and_counterparty_stub_is_unimplemented() {
    let mut s = setup(|_, m| m);
    let mut m = base(&s);
    m.creation_gate = Some(CreationGate::MinStake {
        amount: Uint256::zero(),
    });
    let e = s
        .app
        .instantiate_contract(s.code, s.dao.clone(), &m, &[], "x", None)
        .unwrap_err();
    assert!(format!("{e:?}").contains("must be > 0"), "{e:?}");
    m.creation_gate = Some(CreationGate::CounterpartyEscrowProof {
        chain: "zcash-mainnet".into(),
    });
    let e = s
        .app
        .instantiate_contract(s.code, s.dao.clone(), &m, &[], "x", None)
        .unwrap_err();
    assert!(format!("{e:?}").contains("not implemented"), "{e:?}");
    // also refused through update_config
    let upd = ExecuteMsg::UpdateConfig {
        owner: None,
        approver: None,
        gate: None,
        denoms: None,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: Some("v2".into()),
        params_placeholder: None,
        min_submit_stake: None,
        min_upload_stake: None,
        creation_gate: Some(CreationGate::CounterpartyEscrowProof {
            chain: "zcash-mainnet".into(),
        }),
    };
    let e = s
        .app
        .execute_contract(s.dao.clone(), s.escrow.clone(), &upd, &[])
        .unwrap_err();
    assert!(format!("{e:?}").contains("not implemented"), "{e:?}");
    // switching to Anyone and disabling the ProposerGate works; switching back with an empty gate does not
    let to = |cg: CreationGate, v: &str| ExecuteMsg::UpdateConfig {
        owner: None,
        approver: None,
        gate: Some(ProposerGateMsg {
            use_allowlist: false,
            min_stake: None,
            cw4_group: None,
        }),
        denoms: None,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: Some(v.into()),
        params_placeholder: None,
        min_submit_stake: None,
        min_upload_stake: None,
        creation_gate: Some(cg),
    };
    s.app
        .execute_contract(
            s.dao.clone(),
            s.escrow.clone(),
            &to(CreationGate::Anyone {}, "v3"),
            &[],
        )
        .unwrap();
    assert!(
        s.app
            .execute_contract(
                s.dao.clone(),
                s.escrow.clone(),
                &to(CreationGate::ProposerGate {}, "v4"),
                &[]
            )
            .is_err()
    );
}

#[test]
fn pre_propose_anyone_mode_is_the_default() {
    let s = setup(|_, m| m);
    assert_eq!(
        s.q::<ProposalCreationPolicy>(&QueryMsg::ProposalCreationPolicy {}),
        ProposalCreationPolicy::Anyone {}
    );
    let s = setup(|_, mut m| {
        m.pre_propose_info = Some(PreProposeInfo::AnyoneMayPropose {});
        m
    });
    assert_eq!(
        s.q::<ProposalCreationPolicy>(&QueryMsg::ProposalCreationPolicy {}),
        ProposalCreationPolicy::Anyone {}
    );
}

fn module_info(code_id: u64) -> PreProposeInfo {
    PreProposeInfo::ModuleMayPropose {
        info: ModuleInstantiateInfo {
            code_id,
            msg: to_json_binary(&Empty {}).unwrap(),
            admin: None,
            funds: None,
            label: "bounty-pre-propose".into(),
            salt: None,
        },
    }
}

#[test]
fn pre_propose_module_mode_gates_creation() {
    // store the mock module first so its code id is known when the escrow is instantiated
    let mut s = setup(|_, m| m);
    let pre_code = s.app.store_code(pre_propose_contract());
    let mut m = base(&s);
    m.pre_propose_info = Some(module_info(pre_code));
    let escrow = s
        .app
        .instantiate_contract(s.code, s.dao.clone(), &m, &[], "bounty-escrow-2", None)
        .unwrap();
    s.escrow = escrow;
    let module = match s.q::<ProposalCreationPolicy>(&QueryMsg::ProposalCreationPolicy {}) {
        ProposalCreationPolicy::Module { addr } => addr,
        p => panic!("expected module policy, got {p:?}"),
    };
    // direct creation, even by the allowlisted proposer, is refused
    let p = s.proposer.clone();
    assert!(err_str(s.create(&p, "x").unwrap_err()).contains("pre-propose module"));
    // the module creates on behalf of a proposer who is NOT allowlisted (the module is the gate)
    let r = s.rando.clone();
    let inner = to_json_binary(&create_msg("x", s.exp(), Some(r.to_string()))).unwrap();
    s.app
        .execute_contract(
            r.clone(),
            module.clone(),
            &PreMsg::Forward { msg: inner },
            &coins(2_000_000, UTERP),
        )
        .unwrap();
    let b: Bounty = s.q(&QueryMsg::Bounty { id: 1 });
    assert_eq!(b.proposer, r);
    // only the module may set `proposer`
    let m2 = create_msg("y", s.exp(), Some(r.to_string()));
    assert!(
        s.app
            .execute_contract(r.clone(), s.escrow.clone(), &m2, &coins(2_000_000, UTERP))
            .is_err()
    );
    // terminal status -> ProposalCompletedHook to the module; refund goes to the real proposer
    let before = s.app.wrap().query_balance(&r, UTERP).unwrap().amount;
    let ap = s.approver.clone();
    s.app
        .execute_contract(
            ap,
            s.escrow.clone(),
            &ExecuteMsg::Reject {
                id: 1,
                reason: None,
            },
            &[],
        )
        .unwrap();
    let after = s.app.wrap().query_balance(&r, UTERP).unwrap().amount;
    assert_eq!(after - before, Uint256::new(2_000_000));
    let completed: u64 = s
        .app
        .wrap()
        .query_wasm_smart(&module, &PreQuery::Completed {})
        .unwrap();
    assert_eq!(completed, 1);
    assert!(matches!(
        s.q::<ProposalCreationPolicy>(&QueryMsg::ProposalCreationPolicy {}),
        ProposalCreationPolicy::Module { .. }
    ));
    // only the DAO may change the pre-propose config
    let upd = ExecuteMsg::UpdatePreProposeInfo {
        info: PreProposeInfo::AnyoneMayPropose {},
    };
    assert!(
        s.app
            .execute_contract(p.clone(), s.escrow.clone(), &upd, &[])
            .is_err()
    );
    s.app
        .execute_contract(s.dao.clone(), s.escrow.clone(), &upd, &[])
        .unwrap();
    assert_eq!(
        s.q::<ProposalCreationPolicy>(&QueryMsg::ProposalCreationPolicy {}),
        ProposalCreationPolicy::Anyone {}
    );
    s.create(&p, "z").unwrap();
}

#[test]
fn proposal_hooks_fire_on_create_and_status_change_and_broken_hooks_are_removed() {
    let mut s = setup(|_, m| m);
    let hook_code = s.app.store_code(hook_contract());
    let good = s
        .app
        .instantiate_contract(hook_code, s.dao.clone(), &false, &[], "hook", None)
        .unwrap();
    let bad = s
        .app
        .instantiate_contract(hook_code, s.dao.clone(), &true, &[], "bad-hook", None)
        .unwrap();
    // only the DAO may add hooks
    let add = |a: &Addr| ExecuteMsg::AddProposalHook {
        address: a.to_string(),
    };
    let p = s.proposer.clone();
    assert!(
        s.app
            .execute_contract(p.clone(), s.escrow.clone(), &add(&good), &[])
            .is_err()
    );
    s.app
        .execute_contract(s.dao.clone(), s.escrow.clone(), &add(&good), &[])
        .unwrap();
    s.app
        .execute_contract(s.dao.clone(), s.escrow.clone(), &add(&bad), &[])
        .unwrap();
    let hooks: cw_hooks::HooksResponse = s.q(&QueryMsg::ProposalHooks {});
    assert_eq!(hooks.hooks.len(), 2);
    // create (NewProposal) -> the broken receiver errors, is removed, the tx still succeeds
    s.create(&p, "h1").unwrap();
    let hooks: cw_hooks::HooksResponse = s.q(&QueryMsg::ProposalHooks {});
    assert_eq!(hooks.hooks, vec![good.to_string()]);
    let ap = s.approver.clone();
    s.app
        .execute_contract(
            ap.clone(),
            s.escrow.clone(),
            &ExecuteMsg::Approve { id: 1 },
            &[],
        )
        .unwrap();
    // a rejected action does not change status and fires nothing
    assert!(
        s.app
            .execute_contract(ap, s.escrow.clone(), &ExecuteMsg::Approve { id: 1 }, &[])
            .is_err()
    );
    let seen: u64 = s.app.wrap().query_wasm_smart(&good, &Empty {}).unwrap();
    assert_eq!(
        seen, 2,
        "NewProposal + ProposalStatusChanged(pending->open)"
    );
    s.app
        .execute_contract(
            s.dao.clone(),
            s.escrow.clone(),
            &ExecuteMsg::RemoveProposalHook {
                address: good.to_string(),
            },
            &[],
        )
        .unwrap();
    let hooks: cw_hooks::HooksResponse = s.q(&QueryMsg::ProposalHooks {});
    assert!(hooks.hooks.is_empty());
}
