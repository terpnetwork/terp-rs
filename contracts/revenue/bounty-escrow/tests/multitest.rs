//! cw-multi-test suite. Local, in-memory chain only, test keys from MockApiBech32("terp").
use cosmwasm_std::{
    Addr, Coin, Decimal, Empty, StakingMsg, Timestamp, Uint256, Validator, coin, coins,
    testing::mock_env,
};
use cw_multi_test::{
    App, AppBuilder, AppResponse, BankKeeper, Contract, ContractWrapper, Executor, MockApiBech32,
    StakingInfo,
};
use terp_bounty_escrow::ContractError;
use terp_bounty_escrow::contract::SUMMARY_MAX_BYTES;
use terp_bounty_escrow::contract::{execute, instantiate, migrate, query};
use terp_bounty_escrow::msg::{
    AllowlistResponse, BountiesResponse, EligibilityResponse, ExecuteMsg, InstantiateMsg,
    ProposerGateMsg, QueryMsg, SubmissionsResponse, SubmitEligibilityResponse,
    UploadEligibilityResponse,
};
use terp_bounty_escrow::state::{Bounty, BountyStatus, DenomRule};

/// cw-multi-test 3 surfaces contract errors as `StdError::msg(<Display>)`, so the typed
/// `ContractError` cannot be downcast any more. Rebuild it from the rendered message (every
/// payload is a `String`), which keeps the typed `assert_eq!` / `matches!` checks below.
fn decode(e: &cosmwasm_std::StdError) -> ContractError {
    let full = e.to_string();
    let s = full.rsplit("error: ").next().unwrap_or(&full).trim();
    let after = |p: &str| s.strip_prefix(p).map(|r| r.to_string());
    let cut = |a: &str, b: &str| -> Option<String> {
        let i = s.find(a)? + a.len();
        let j = s[i..].find(b)? + i;
        Some(s[i..j].to_string())
    };
    let digits = |x: &str| {
        x.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
    };
    use ContractError as C;
    if let Some(r) = after("unauthorized: ") {
        return C::Unauthorized(r);
    }
    if let Some(r) = after("invalid input: ") {
        return C::InvalidInput(r);
    }
    if let Some(r) = after("invalid IPFS CID: ") {
        return C::InvalidCid(r);
    }
    match s {
        "this message does not accept funds" => return C::NonPayable,
        "bounty has expired" => return C::Expired,
        "bounty has not expired yet" => return C::NotExpired,
        "submission_hash must be 64 lowercase hex chars (sha256)" => return C::InvalidHash,
        "partial payouts are disabled" => return C::PartialNotAllowed,
        _ => {}
    }
    if s.starts_with("proposer ") && s.ends_with(" is not on the Terp staker whitelist") {
        return C::NotEligible(cut("proposer ", " is not on").unwrap());
    }
    if s.starts_with("deposit mismatch: ") {
        let i = s.find(", got [").unwrap();
        return C::FundsMismatch {
            expected: cut("expected exactly ", ", got [").unwrap(),
            got: s[i + 7..s.len() - 1].to_string(),
        };
    }
    if s.starts_with("denom ") && s.ends_with(" is not an allowed reward denom") {
        return C::DenomNotAllowed(cut("denom ", " is not").unwrap());
    }
    if s.starts_with("bounty_ref ") && s.ends_with(" already used") {
        return C::DuplicateRef(cut("bounty_ref ", " already used").unwrap());
    }
    if s.starts_with("action `") {
        return C::InvalidTransition {
            action: cut("action `", "`").unwrap(),
            from: cut("from status `", "`").unwrap(),
        };
    }
    if s.starts_with("payout amount ") {
        return C::AmountExceedsRemaining {
            amount: cut("payout amount ", " exceeds").unwrap(),
            remaining: after_word(s, "remaining escrow "),
        };
    }
    if s.starts_with("refusing to instantiate on mainnet chain id ") {
        return C::MainnetGuard(cut("chain id ", " (dev-chain-guard").unwrap());
    }
    if s.starts_with("submitter ") && s.contains(", below min_submit_stake ") {
        let who = cut("submitter ", " has ").unwrap();
        let staked_denom = cut(" has ", " bonded").unwrap();
        let staked = digits(&staked_denom);
        let denom = staked_denom[staked.len()..].to_string();
        let min = digits(&after_word(s, "below min_submit_stake "));
        return C::BelowSubmitStake {
            who,
            staked,
            min,
            denom,
        };
    }
    if s.starts_with("reward ") && s.contains(" outside allowed range [") {
        let ad = cut("reward ", " outside").unwrap();
        let amount = digits(&ad);
        let denom = ad[amount.len()..].to_string();
        let range = cut("range [", "]").unwrap();
        let (min, max) = range.split_once(", ").unwrap();
        return C::AmountOutOfRange {
            denom,
            amount,
            min: min.into(),
            max: max.into(),
        };
    }
    C::Std(cosmwasm_std::StdError::msg(s))
}

fn after_word(s: &str, a: &str) -> String {
    s[s.find(a).unwrap() + a.len()..].to_string()
}

/// cosmwasm-std 3+: `Coin::amount` is `Uint256`, so the contract keeps every amount in `Uint256`.
trait AsU128 {
    fn u128(&self) -> u128;
}
impl AsU128 for Uint256 {
    fn u128(&self) -> u128 {
        cosmwasm_std::Uint128::try_from(*self).unwrap().u128()
    }
}

type TApp = App<BankKeeper, MockApiBech32>;

const UTERP: &str = "uterp";
const UTHIOL: &str = "uthiol";
const DAY: u64 = 86_400; // only for moving the test clock; the window comes from params()
const HASH1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const HASH2: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
// real CIDs (ipfs docs examples): v1 dag-pb base32, v1 raw base32, v0 base58
const CID1: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
const CID_RAW: &str = "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku";
const CID0: &str = "QmY7Yh4UquoXHLPFo2XbhXkhBvFoPwmQUSa92pxnxjQuPU";

struct Suite {
    app: TApp,
    escrow: Addr,
    owner: Addr,
    approver: Addr,
    proposer: Addr,
    hunter: Addr,
    rando: Addr,
    fee_collector: Addr,
    validator: String,
}

fn escrow_contract() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(execute, instantiate, query)
            .with_reply(terp_bounty_escrow::contract::reply)
            .with_migrate(
                |deps, env, msg: terp_bounty_escrow::msg::MigrateMsg, _info| {
                    migrate(deps, env, msg)
                },
            ),
    )
}

fn cw4_contract() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(
        cw4_group::contract::execute,
        cw4_group::contract::instantiate,
        cw4_group::contract::query,
    ))
}

/// Governance parameters (PLACEHOLDERS) from escrow/params/escrow.params.json, generated by
/// derive_params.py from the catalog. Tests never hardcode a cap.
fn params() -> serde_json::Value {
    serde_json::from_str(include_str!("../params/escrow.params.json")).unwrap()
}
fn strip_labels(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(
            m.iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .map(|(k, v)| (k.clone(), strip_labels(v)))
                .collect(),
        ),
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(strip_labels).collect())
        }
        x => x.clone(),
    }
}
fn denoms() -> Vec<DenomRule> {
    serde_json::from_value(strip_labels(&params()["instantiate"]["denoms"])).unwrap()
}
fn cap(denom: &str) -> u128 {
    denoms()
        .into_iter()
        .find(|d| d.denom == denom)
        .unwrap()
        .max_amount
        .u128()
}
fn params_version() -> String {
    params()["params_version"].as_str().unwrap().to_string()
}
fn param_secs(k: &str) -> u64 {
    params()["instantiate"][k].as_u64().unwrap()
}
fn min_upload_stake() -> u128 {
    params()["instantiate"]["min_upload_stake"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}
fn min_submit_stake() -> u128 {
    params()["instantiate"]["min_submit_stake"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

fn base_msg(s: &Suite, gate: ProposerGateMsg, partial: bool, fee_bps: u16) -> InstantiateMsg {
    InstantiateMsg {
        owner: s.owner.to_string(),
        approver: s.approver.to_string(),
        gate,
        denoms: denoms(),
        allow_partial_payouts: partial,
        fee_bps,
        fee_collector: if fee_bps > 0 {
            Some(s.fee_collector.to_string())
        } else {
            None
        },
        min_duration_secs: param_secs("min_duration_secs"),
        max_duration_secs: param_secs("max_duration_secs"),
        params_version: params_version(),
        params_placeholder: true,
        min_submit_stake: Uint256::new(min_submit_stake()),
        min_upload_stake: Uint256::new(min_upload_stake()),
        initial_allowlist: vec![s.proposer.to_string()],
        dao: None,
        pre_propose_info: None,
        creation_gate: None,
    }
}

fn allowlist_gate() -> ProposerGateMsg {
    ProposerGateMsg {
        use_allowlist: true,
        min_stake: None,
        cw4_group: None,
    }
}

fn setup_with(f: impl FnOnce(&Suite) -> InstantiateMsg) -> Suite {
    let api = MockApiBech32::new("terp");
    let owner = api.addr_make("owner");
    let approver = api.addr_make("approver-dao");
    let proposer = api.addr_make("proposer");
    let hunter = api.addr_make("hunter");
    let rando = api.addr_make("rando");
    let fee_collector = api.addr_make("fee-collector");
    let validator = api.addr_make("validator").to_string();
    let funded = vec![
        proposer.clone(),
        rando.clone(),
        hunter.clone(),
        approver.clone(),
    ];
    let v = validator.clone();
    let mut app = AppBuilder::default()
        .with_api(api)
        .build(|router, api, storage| {
            for a in &funded {
                router
                    .bank
                    .init_balance(
                        storage,
                        a,
                        vec![
                            coin(1_000_000_000_000, UTERP),
                            coin(1_000_000_000_000, UTHIOL),
                            coin(1_000_000, "uatom"),
                        ],
                    )
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
    let mut s = Suite {
        app,
        escrow: Addr::unchecked(""),
        owner,
        approver,
        proposer,
        hunter,
        rando,
        fee_collector,
        validator,
    };
    let msg = f(&s);
    s.escrow = s
        .app
        .instantiate_contract(
            code,
            s.owner.clone(),
            &msg,
            &[],
            "terp-bounty-escrow",
            Some(s.owner.to_string()),
        )
        .unwrap();
    // the hunter bonds exactly the submission-stake threshold; rando bonds nothing
    let (h, v) = (s.hunter.clone(), s.validator.clone());
    s.app
        .execute(
            h,
            StakingMsg::Delegate {
                validator: v,
                amount: coin(min_submit_stake(), UTERP),
            }
            .into(),
        )
        .unwrap();
    s
}

fn setup() -> Suite {
    setup_with(|s| base_msg(s, allowlist_gate(), false, 0))
}

impl Suite {
    fn exec(
        &mut self,
        sender: &Addr,
        msg: &ExecuteMsg,
        funds: &[Coin],
    ) -> Result<AppResponse, ContractError> {
        self.app
            .execute_contract(sender.clone(), self.escrow.clone(), msg, funds)
            .map_err(|e| decode(&e))
    }
    fn now(&self) -> Timestamp {
        self.app.block_info().time
    }
    fn create(
        &mut self,
        sender: &Addr,
        r: &str,
        reward: Coin,
        funds: &[Coin],
    ) -> Result<AppResponse, ContractError> {
        let exp = self.now().plus_seconds(30 * DAY);
        let s = sender.clone();
        self.exec(
            &s,
            &ExecuteMsg::CreateBounty {
                bounty_ref: r.into(),
                title: format!("bounty {r}"),
                summary: format!("Summary of bounty {r}."),
                details_hash: HASH2.into(),
                details_cid: Some(CID_RAW.into()),
                reward,
                expires_at: exp,
                proposer: None,
            },
            funds,
        )
    }
    fn create_ok(&mut self, r: &str, amount: u128) -> u64 {
        let p = self.proposer.clone();
        let res = self
            .create(&p, r, coin(amount, UTERP), &coins(amount, UTERP))
            .unwrap();
        attr(&res, "bounty_id").parse().unwrap()
    }
    fn bounty(&self, id: u64) -> Bounty {
        self.app
            .wrap()
            .query_wasm_smart(&self.escrow, &QueryMsg::Bounty { id })
            .unwrap()
    }
    fn bal(&self, a: &Addr, denom: &str) -> u128 {
        self.app
            .wrap()
            .query_balance(a, denom)
            .unwrap()
            .amount
            .u128()
    }
    fn count(&self) -> usize {
        let r: BountiesResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.escrow,
                &QueryMsg::Bounties {
                    status: None,
                    proposer: None,
                    start_after: None,
                    limit: None,
                },
            )
            .unwrap();
        r.bounties.len()
    }
    fn advance(&mut self, secs: u64) {
        self.app.update_block(|b| {
            b.time = b.time.plus_seconds(secs);
            b.height += secs / 5;
        });
    }
    /// create -> approve -> assign -> submit -> accept
    fn run_to_accepted(&mut self, r: &str, amount: u128) -> u64 {
        let id = self.create_ok(r, amount);
        let (ap, h) = (self.approver.clone(), self.hunter.clone());
        self.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap();
        self.exec(
            &ap,
            &ExecuteMsg::Assign {
                id,
                assignee: h.to_string(),
            },
            &[],
        )
        .unwrap();
        self.exec(
            &h,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: CID1.into(),
            },
            &[],
        )
        .unwrap();
        self.exec(&ap, &ExecuteMsg::Accept { id }, &[]).unwrap();
        id
    }
}

fn attr(res: &AppResponse, key: &str) -> String {
    res.events
        .iter()
        .filter(|e| e.ty == "wasm-terp_bounty")
        .flat_map(|e| e.attributes.iter())
        .find(|a| a.key == key)
        .map(|a| a.value.clone())
        .unwrap_or_else(|| panic!("missing attr {key}"))
}

fn is_transition(e: &ContractError) -> bool {
    matches!(e, ContractError::InvalidTransition { .. })
}

// ------------------------------------------------------------------ whitelist

#[test]
fn allowlist_gate_admits_only_listed_proposers() {
    let mut s = setup();
    let r = s.rando.clone();
    let err = s
        .create(&r, "r1", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
        .unwrap_err();
    assert_eq!(err, ContractError::NotEligible(r.to_string()));

    // only owner manages the allowlist
    let err = s
        .exec(
            &r,
            &ExecuteMsg::UpdateAllowlist {
                add: vec![r.to_string()],
                remove: vec![],
            },
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, ContractError::Unauthorized(_)));
    let o = s.owner.clone();
    s.exec(
        &o,
        &ExecuteMsg::UpdateAllowlist {
            add: vec![r.to_string()],
            remove: vec![],
        },
        &[],
    )
    .unwrap();
    s.create(&r, "r1", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
        .unwrap();

    let list: AllowlistResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::Allowlist {
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(list.addresses.len(), 2);

    s.exec(
        &o,
        &ExecuteMsg::UpdateAllowlist {
            add: vec![],
            remove: vec![r.to_string()],
        },
        &[],
    )
    .unwrap();
    let err = s
        .create(&r, "r2", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
        .unwrap_err();
    assert_eq!(err, ContractError::NotEligible(r.to_string()));
}

#[test]
fn min_stake_gate_uses_all_delegations() {
    let mut s = setup_with(|s| {
        let mut m = base_msg(
            s,
            ProposerGateMsg {
                use_allowlist: false,
                min_stake: Some(Uint256::new(100_000_000)),
                cw4_group: None,
            },
            false,
            0,
        );
        m.initial_allowlist = vec![];
        m
    });
    let r = s.rando.clone();
    let val = s.validator.clone();
    // no stake
    assert_eq!(
        s.create(&r, "s1", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
            .unwrap_err(),
        ContractError::NotEligible(r.to_string())
    );
    // below min (two delegations summed = 99 TERP)
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: val.clone(),
                amount: coin(50_000_000, UTERP),
            }
            .into(),
        )
        .unwrap();
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: val.clone(),
                amount: coin(49_000_000, UTERP),
            }
            .into(),
        )
        .unwrap();
    let e: EligibilityResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::IsEligible {
                address: r.to_string(),
            },
        )
        .unwrap();
    assert!(!e.eligible);
    assert_eq!(e.staked, Some(Uint256::new(99_000_000)));
    assert_eq!(
        s.create(&r, "s1", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
            .unwrap_err(),
        ContractError::NotEligible(r.to_string())
    );
    // reach min
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: val,
                amount: coin(1_000_000, UTERP),
            }
            .into(),
        )
        .unwrap();
    let e: EligibilityResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::IsEligible {
                address: r.to_string(),
            },
        )
        .unwrap();
    assert!(e.eligible);
    assert_eq!(e.via.as_deref(), Some("min_stake"));
    let res = s
        .create(&r, "s1", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
        .unwrap();
    assert_eq!(attr(&res, "eligible_via"), "min_stake");
}

#[test]
fn cw4_gate_requires_positive_weight() {
    let api = MockApiBech32::new("terp");
    let (member, zero) = (api.addr_make("proposer"), api.addr_make("hunter"));
    let mut s = setup();
    let code = s.app.store_code(cw4_contract());
    let group = s
        .app
        .instantiate_contract(
            code,
            s.owner.clone(),
            &cw4_group::msg::InstantiateMsg {
                admin: None,
                members: vec![
                    cw4::Member {
                        addr: member.to_string(),
                        weight: 3,
                    },
                    cw4::Member {
                        addr: zero.to_string(),
                        weight: 0,
                    },
                ],
            },
            &[],
            "cw4",
            None,
        )
        .unwrap();
    let o = s.owner.clone();
    s.exec(
        &o,
        &ExecuteMsg::UpdateConfig {
            owner: None,
            approver: None,
            gate: Some(ProposerGateMsg {
                use_allowlist: false,
                min_stake: None,
                cw4_group: Some(group.to_string()),
            }),
            denoms: None,
            allow_partial_payouts: None,
            fee_bps: None,
            fee_collector: None,
            min_duration_secs: None,
            max_duration_secs: None,
            params_version: Some("test-cw4-gate".into()),
            params_placeholder: None,
            min_submit_stake: None,
            min_upload_stake: None,
            creation_gate: None,
        },
        &[],
    )
    .unwrap();
    let res = s
        .create(
            &member,
            "g1",
            coin(5_000_000, UTERP),
            &coins(5_000_000, UTERP),
        )
        .unwrap();
    assert_eq!(attr(&res, "eligible_via"), "cw4");
    assert_eq!(
        s.create(
            &zero,
            "g2",
            coin(5_000_000, UTERP),
            &coins(5_000_000, UTERP)
        )
        .unwrap_err(),
        ContractError::NotEligible(zero.to_string())
    );
    let r = s.rando.clone();
    assert_eq!(
        s.create(&r, "g3", coin(5_000_000, UTERP), &coins(5_000_000, UTERP))
            .unwrap_err(),
        ContractError::NotEligible(r.to_string())
    );
}

#[test]
fn instantiate_rejects_no_gate_and_bad_params() {
    let api = MockApiBech32::new("terp");
    let mut app: TApp = AppBuilder::default().with_api(api).build(|_, _, _| {});
    let code = app.store_code(escrow_contract());
    let owner = app.api().addr_make("owner");
    let mk = |gate: ProposerGateMsg, fee: u16| InstantiateMsg {
        owner: owner.to_string(),
        approver: owner.to_string(),
        gate,
        denoms: denoms(),
        allow_partial_payouts: false,
        fee_bps: fee,
        fee_collector: None,
        min_duration_secs: param_secs("min_duration_secs"),
        max_duration_secs: param_secs("max_duration_secs"),
        params_version: params_version(),
        params_placeholder: true,
        min_submit_stake: Uint256::new(min_submit_stake()),
        min_upload_stake: Uint256::new(min_upload_stake()),
        initial_allowlist: vec![],
        dao: None,
        pre_propose_info: None,
        creation_gate: None,
    };
    for m in [
        mk(
            ProposerGateMsg {
                use_allowlist: false,
                min_stake: None,
                cw4_group: None,
            },
            0,
        ),
        mk(
            ProposerGateMsg {
                use_allowlist: false,
                min_stake: Some(Uint256::zero()),
                cw4_group: None,
            },
            0,
        ),
        mk(allowlist_gate(), 50),   // fee without collector
        mk(allowlist_gate(), 5000), // above hard cap
    ] {
        let e = app
            .instantiate_contract(code, owner.clone(), &m, &[], "x", None)
            .unwrap_err();
        assert!(matches!(decode(&e), ContractError::InvalidInput(_)));
    }
}

/// morocco-1 instantiate: refused with the default `dev-chain-guard`, allowed by the
/// mainnet build (`--no-default-features`, owner-approved 2026-10-27).
#[test]
fn mainnet_chain_id_follows_dev_chain_guard() {
    let api = MockApiBech32::new("terp");
    let mut app: TApp = AppBuilder::default().with_api(api).build(|_, _, _| {});
    app.update_block(|b| b.chain_id = "morocco-1".into());
    let code = app.store_code(escrow_contract());
    let owner = app.api().addr_make("owner");
    let m = InstantiateMsg {
        owner: owner.to_string(),
        approver: owner.to_string(),
        gate: allowlist_gate(),
        denoms: denoms(),
        allow_partial_payouts: false,
        fee_bps: 0,
        fee_collector: None,
        min_duration_secs: param_secs("min_duration_secs"),
        max_duration_secs: param_secs("max_duration_secs"),
        params_version: params_version(),
        params_placeholder: true,
        min_submit_stake: Uint256::new(min_submit_stake()),
        min_upload_stake: Uint256::new(min_upload_stake()),
        initial_allowlist: vec![],
        dao: None,
        pre_propose_info: None,
        creation_gate: None,
    };
    let r = app.instantiate_contract(code, owner.clone(), &m, &[], "x", None);
    if cfg!(feature = "dev-chain-guard") {
        assert_eq!(
            decode(&r.unwrap_err()),
            ContractError::MainnetGuard("morocco-1".into())
        );
    } else {
        r.expect("mainnet build instantiates on morocco-1");
    }
}

// ------------------------------------------------------------------ deposit

#[test]
fn create_requires_exact_deposit() {
    let mut s = setup();
    let p = s.proposer.clone();
    let before = s.bal(&p, UTERP);
    let reward = coin(5_000_000, UTERP);
    let cases: Vec<Vec<Coin>> = vec![
        vec![],                                             // nothing
        coins(4_999_999, UTERP),                            // short
        coins(5_000_001, UTERP),                            // over
        coins(5_000_000, UTHIOL),                           // wrong denom
        vec![coin(1_000, "uatom"), coin(5_000_000, UTERP)], // extra coin
    ];
    for f in cases {
        let e = s.create(&p, "d1", reward.clone(), &f).unwrap_err();
        assert!(matches!(e, ContractError::FundsMismatch { .. }), "{e:?}");
    }
    // declared denom not allowed / amount outside range
    assert_eq!(
        s.create(&p, "d1", coin(1_000, "uatom"), &coins(1_000, "uatom"))
            .unwrap_err(),
        ContractError::DenomNotAllowed("uatom".into())
    );
    assert!(matches!(
        s.create(&p, "d1", coin(10, UTERP), &coins(10, UTERP))
            .unwrap_err(),
        ContractError::AmountOutOfRange { .. }
    ));
    // nothing recorded, nothing kept
    assert_eq!(s.count(), 0);
    assert_eq!(s.bal(&p, UTERP), before);
    assert_eq!(s.bal(&s.escrow, UTERP), 0);

    // exact deposit -> Pending, funds held by the contract
    let res = s
        .create(&p, "d1", reward.clone(), std::slice::from_ref(&reward))
        .unwrap();
    assert_eq!(attr(&res, "status"), "pending");
    let id: u64 = attr(&res, "bounty_id").parse().unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Pending);
    assert_eq!(s.bal(&s.escrow, UTERP), 5_000_000);
    // duplicate off-chain ref
    assert_eq!(
        s.create(&p, "d1", reward.clone(), &[reward]).unwrap_err(),
        ContractError::DuplicateRef("d1".into())
    );
}

#[test]
fn create_validates_expiry_window_and_non_payable_msgs() {
    let mut s = setup();
    let p = s.proposer.clone();
    let now = s.now();
    for exp in [
        now.plus_seconds(param_secs("min_duration_secs") - 1),
        now.plus_seconds(param_secs("max_duration_secs") + 1),
    ] {
        let e = s
            .exec(
                &p,
                &ExecuteMsg::CreateBounty {
                    bounty_ref: "e".into(),
                    title: "t".into(),
                    summary: "s".into(),
                    details_hash: HASH2.into(),
                    details_cid: None,
                    reward: coin(2_000_000, UTERP),
                    expires_at: exp,
                    proposer: None,
                },
                &coins(2_000_000, UTERP),
            )
            .unwrap_err();
        assert!(matches!(e, ContractError::InvalidInput(_)));
    }
    let id = s.create_ok("np", 2_000_000);
    let ap = s.approver.clone();
    assert_eq!(
        s.exec(&ap, &ExecuteMsg::Approve { id }, &coins(1, UTERP))
            .unwrap_err(),
        ContractError::NonPayable
    );
}

// ------------------------------------------------------------------ state machine

#[test]
fn full_lifecycle_with_fee_and_revision() {
    let mut s = setup_with(|s| base_msg(s, allowlist_gate(), false, 250)); // 2.5% placeholder fee
    let (ap, h, p) = (s.approver.clone(), s.hunter.clone(), s.proposer.clone());
    let hunter_before = s.bal(&h, UTERP);
    let id = s.create_ok("life", 100_000_000);

    let res = s.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap();
    assert_eq!(
        (
            attr(&res, "prev_status").as_str(),
            attr(&res, "status").as_str()
        ),
        ("pending", "open")
    );
    // can't submit before assignment
    assert!(matches!(
        s.exec(
            &h,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: CID1.into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    s.exec(
        &p,
        &ExecuteMsg::Assign {
            id,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Assigned);
    // only the assignee submits; hash + CID validated
    let r = s.rando.clone();
    assert!(matches!(
        s.exec(
            &r,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: CID1.into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    assert_eq!(
        s.exec(
            &h,
            &ExecuteMsg::Submit {
                id,
                submission_hash: "XYZ".into(),
                cid: CID1.into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::InvalidHash
    );
    assert!(matches!(
        s.exec(
            &h,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: "javascript:alert(1)".into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::InvalidCid(_)
    ));
    let res = s
        .exec(
            &h,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: CID1.into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(attr(&res, "submission_hash"), HASH1);
    assert_eq!(attr(&res, "submission_seq"), "1");
    // reviewer asks for changes, hunter resubmits
    s.exec(
        &p,
        &ExecuteMsg::RequestChanges {
            id,
            reason: Some("tests missing".into()),
        },
        &[],
    )
    .unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Assigned);
    s.exec(
        &h,
        &ExecuteMsg::Submit {
            id,
            submission_hash: HASH2.into(),
            cid: CID0.into(),
        },
        &[],
    )
    .unwrap();
    let subs: SubmissionsResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::Submissions {
                id,
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(subs.submissions.len(), 2);
    assert_eq!(subs.submissions[1].hash, HASH2);
    // proposer cannot accept; approver accepts
    assert!(matches!(
        s.exec(&p, &ExecuteMsg::Accept { id }, &[]).unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    s.exec(&ap, &ExecuteMsg::Accept { id }, &[]).unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Accepted);
    // accepting is not paying: nothing moved yet
    assert_eq!(s.bal(&h, UTERP), hunter_before);
    let res = s
        .exec(&ap, &ExecuteMsg::Payout { id, amount: None }, &[])
        .unwrap();
    assert_eq!(attr(&res, "status"), "paid");
    assert_eq!(attr(&res, "fee"), "2500000");
    assert_eq!(attr(&res, "net"), "97500000");
    assert_eq!(s.bal(&h, UTERP), hunter_before + 97_500_000);
    assert_eq!(s.bal(&s.fee_collector, UTERP), 2_500_000);
    assert_eq!(s.bal(&s.escrow, UTERP), 0);
    let b = s.bounty(id);
    assert_eq!((b.status, b.paid.u128()), (BountyStatus::Paid, 100_000_000));
    // terminal: no further moves
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Payout { id, amount: None }, &[])
            .unwrap_err()
    ));
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Cancel { id }, &[]).unwrap_err()
    ));
}

#[test]
fn invalid_transitions_are_rejected() {
    let mut s = setup();
    let (ap, h) = (s.approver.clone(), s.hunter.clone());
    let id = s.create_ok("t", 2_000_000);
    assert!(is_transition(
        &s.exec(
            &ap,
            &ExecuteMsg::Assign {
                id,
                assignee: h.to_string()
            },
            &[]
        )
        .unwrap_err()
    )); // Pending
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Accept { id }, &[]).unwrap_err()
    ));
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Payout { id, amount: None }, &[])
            .unwrap_err()
    ));
    s.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap();
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap_err()
    ));
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Reject { id, reason: None }, &[])
            .unwrap_err()
    ));
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    assert!(is_transition(
        &s.exec(&ap, &ExecuteMsg::Payout { id, amount: None }, &[])
            .unwrap_err()
    )); // Assigned
    s.exec(&ap, &ExecuteMsg::Unassign { id }, &[]).unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Open);
    assert_eq!(s.bounty(id).assignee, None);
    let p = s.proposer.clone();
    assert!(matches!(
        s.exec(
            &ap,
            &ExecuteMsg::Assign {
                id,
                assignee: p.to_string()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::InvalidInput(_)
    ));
}

// ------------------------------------------------------------------ refunds / timeouts

#[test]
fn reject_and_cancel_refund_the_proposer() {
    let mut s = setup();
    let (ap, p, h, r) = (
        s.approver.clone(),
        s.proposer.clone(),
        s.hunter.clone(),
        s.rando.clone(),
    );
    let start = s.bal(&p, UTERP);

    let a = s.create_ok("a", 3_000_000);
    let res = s
        .exec(
            &ap,
            &ExecuteMsg::Reject {
                id: a,
                reason: Some("out of scope".into()),
            },
            &[],
        )
        .unwrap();
    assert_eq!(attr(&res, "refund_amount"), "3000000");
    assert_eq!(s.bounty(a).status, BountyStatus::Refunded);

    let b = s.create_ok("b", 4_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: b }, &[]).unwrap();
    assert!(matches!(
        s.exec(&r, &ExecuteMsg::Cancel { id: b }, &[]).unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    s.exec(&p, &ExecuteMsg::Cancel { id: b }, &[]).unwrap();
    assert_eq!(s.bounty(b).status, BountyStatus::Cancelled);

    // proposer may not pull funds from under an assigned hunter; approver may
    let c = s.create_ok("c", 5_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: c }, &[]).unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id: c,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    assert!(is_transition(
        &s.exec(&p, &ExecuteMsg::Cancel { id: c }, &[]).unwrap_err()
    ));
    s.exec(&ap, &ExecuteMsg::Cancel { id: c }, &[]).unwrap();
    assert_eq!(s.bounty(c).refunded.u128(), 5_000_000);

    assert_eq!(s.bal(&p, UTERP), start);
    assert_eq!(s.bal(&s.escrow, UTERP), 0);
}

#[test]
fn expiry_refunds_but_never_after_submission() {
    let mut s = setup();
    let (ap, p, h, r) = (
        s.approver.clone(),
        s.proposer.clone(),
        s.hunter.clone(),
        s.rando.clone(),
    );
    let start = s.bal(&p, UTERP);
    let open = s.create_ok("open", 2_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: open }, &[]).unwrap();
    let assigned = s.create_ok("assigned", 2_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: assigned }, &[])
        .unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id: assigned,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    let submitted = s.create_ok("submitted", 2_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: submitted }, &[])
        .unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id: submitted,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    s.exec(
        &h,
        &ExecuteMsg::Submit {
            id: submitted,
            submission_hash: HASH1.into(),
            cid: CID1.into(),
        },
        &[],
    )
    .unwrap();
    let pending = s.create_ok("pending", 2_000_000);

    assert_eq!(
        s.exec(&r, &ExecuteMsg::Expire { id: open }, &[])
            .unwrap_err(),
        ContractError::NotExpired
    );
    s.advance(30 * DAY);
    // after expiry: no more approvals, assignments or submissions
    assert_eq!(
        s.exec(&ap, &ExecuteMsg::Approve { id: pending }, &[])
            .unwrap_err(),
        ContractError::Expired
    );
    assert_eq!(
        s.exec(
            &h,
            &ExecuteMsg::Submit {
                id: assigned,
                submission_hash: HASH1.into(),
                cid: CID1.into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::Expired
    );
    // anyone may trigger the refund
    for id in [open, assigned, pending] {
        let res = s.exec(&r, &ExecuteMsg::Expire { id }, &[]).unwrap();
        assert_eq!(attr(&res, "status"), "expired");
        assert_eq!(attr(&res, "refund_to"), p.to_string());
    }
    // submitted work is protected: only the approver resolves it
    assert!(is_transition(
        &s.exec(&r, &ExecuteMsg::Expire { id: submitted }, &[])
            .unwrap_err()
    ));
    s.exec(&ap, &ExecuteMsg::Accept { id: submitted }, &[])
        .unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Payout {
            id: submitted,
            amount: None,
        },
        &[],
    )
    .unwrap();
    assert_eq!(s.bal(&p, UTERP), start - 2_000_000);
    assert_eq!(s.bal(&s.escrow, UTERP), 0);
}

// ------------------------------------------------------------------ payout auth / partials

#[test]
fn payout_requires_the_approver() {
    let mut s = setup();
    let id = s.run_to_accepted("pay", 10_000_000);
    for who in [
        s.owner.clone(),
        s.proposer.clone(),
        s.hunter.clone(),
        s.rando.clone(),
    ] {
        let e = s
            .exec(&who, &ExecuteMsg::Payout { id, amount: None }, &[])
            .unwrap_err();
        assert_eq!(e, ContractError::Unauthorized("approver".into()));
        let e = s
            .exec(&who, &ExecuteMsg::SettleRemainder { id }, &[])
            .unwrap_err();
        assert_eq!(e, ContractError::Unauthorized("approver".into()));
    }
    assert_eq!(s.bal(&s.escrow, UTERP), 10_000_000);
    // approver is switchable to a DAO core address only by the owner
    let (o, r) = (s.owner.clone(), s.rando.clone());
    let upd = |a: &Addr| ExecuteMsg::UpdateConfig {
        owner: None,
        approver: Some(a.to_string()),
        gate: None,
        denoms: None,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: Some(format!("test-approver-{a}")),
        params_placeholder: None,
        min_submit_stake: None,
        min_upload_stake: None,
        creation_gate: None,
    };
    assert!(matches!(
        s.exec(&r, &upd(&r), &[]).unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    let dao = s.app.api().addr_make("dao-core");
    s.exec(&o, &upd(&dao), &[]).unwrap();
    let old = s.approver.clone();
    assert!(matches!(
        s.exec(&old, &ExecuteMsg::Payout { id, amount: None }, &[])
            .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    s.exec(&dao, &ExecuteMsg::Payout { id, amount: None }, &[])
        .unwrap();
    assert_eq!(s.bounty(id).status, BountyStatus::Paid);
}

#[test]
fn partial_payouts_off_by_default() {
    let mut s = setup();
    let ap = s.approver.clone();
    let id = s.run_to_accepted("p0", 10_000_000);
    assert_eq!(
        s.exec(
            &ap,
            &ExecuteMsg::Payout {
                id,
                amount: Some(Uint256::new(1))
            },
            &[]
        )
        .unwrap_err(),
        ContractError::PartialNotAllowed
    );
}

#[test]
fn partial_payouts_then_settle_remainder() {
    let mut s = setup_with(|s| base_msg(s, allowlist_gate(), true, 0));
    let (ap, h, p) = (s.approver.clone(), s.hunter.clone(), s.proposer.clone());
    let h0 = s.bal(&h, UTERP);
    let p0 = s.bal(&p, UTERP);

    // milestone 1 of 2, then remaining paid
    let a = s.run_to_accepted("m", 10_000_000);
    let res = s
        .exec(
            &ap,
            &ExecuteMsg::Payout {
                id: a,
                amount: Some(Uint256::new(4_000_000)),
            },
            &[],
        )
        .unwrap();
    assert_eq!(
        (
            attr(&res, "partial").as_str(),
            attr(&res, "remaining").as_str()
        ),
        ("true", "6000000")
    );
    assert_eq!(s.bounty(a).status, BountyStatus::Accepted);
    assert!(matches!(
        s.exec(
            &ap,
            &ExecuteMsg::Payout {
                id: a,
                amount: Some(Uint256::new(6_000_001))
            },
            &[]
        )
        .unwrap_err(),
        ContractError::AmountExceedsRemaining { .. }
    ));
    assert!(matches!(
        s.exec(
            &ap,
            &ExecuteMsg::Payout {
                id: a,
                amount: Some(Uint256::zero())
            },
            &[]
        )
        .unwrap_err(),
        ContractError::InvalidInput(_)
    ));
    s.exec(
        &ap,
        &ExecuteMsg::Payout {
            id: a,
            amount: None,
        },
        &[],
    )
    .unwrap();
    assert_eq!(s.bounty(a).status, BountyStatus::Paid);

    // partial, then the rest goes back to the proposer
    let b = s.run_to_accepted("n", 10_000_000);
    assert!(matches!(
        s.exec(&ap, &ExecuteMsg::SettleRemainder { id: b }, &[])
            .unwrap_err(),
        ContractError::InvalidInput(_)
    ));
    s.exec(
        &ap,
        &ExecuteMsg::Payout {
            id: b,
            amount: Some(Uint256::new(3_000_000)),
        },
        &[],
    )
    .unwrap();
    let res = s
        .exec(&ap, &ExecuteMsg::SettleRemainder { id: b }, &[])
        .unwrap();
    assert_eq!(attr(&res, "refund_amount"), "7000000");
    let bb = s.bounty(b);
    assert_eq!(
        (bb.status, bb.paid.u128(), bb.refunded.u128()),
        (BountyStatus::Paid, 3_000_000, 7_000_000)
    );

    assert_eq!(s.bal(&h, UTERP), h0 + 13_000_000);
    assert_eq!(s.bal(&p, UTERP), p0 - 13_000_000);
    assert_eq!(s.bal(&s.escrow, UTERP), 0);
}

// ------------------------------------------------------------------ queries

#[test]
fn list_by_status_and_proposer() {
    let mut s = setup();
    let ap = s.approver.clone();
    let a = s.create_ok("q1", 2_000_000);
    s.create_ok("q2", 2_000_000);
    s.exec(&ap, &ExecuteMsg::Approve { id: a }, &[]).unwrap();
    let q = |s: &Suite, status: Option<BountyStatus>, proposer: Option<String>| -> Vec<u64> {
        let r: BountiesResponse = s
            .app
            .wrap()
            .query_wasm_smart(
                &s.escrow,
                &QueryMsg::Bounties {
                    status,
                    proposer,
                    start_after: None,
                    limit: None,
                },
            )
            .unwrap();
        r.bounties.into_iter().map(|b| b.id).collect()
    };
    assert_eq!(q(&s, Some(BountyStatus::Open), None), vec![1]);
    assert_eq!(q(&s, Some(BountyStatus::Pending), None), vec![2]);
    assert_eq!(q(&s, None, Some(s.proposer.to_string())), vec![1, 2]);
    assert_eq!(
        q(&s, Some(BountyStatus::Open), Some(s.rando.to_string())),
        Vec::<u64>::new()
    );
    let by_ref: Bounty = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::BountyByRef {
                bounty_ref: "q2".into(),
            },
        )
        .unwrap();
    assert_eq!(by_ref.id, 2);
}

// ------------------------------------------------------------------ governance params (task 5)

#[test]
fn params_caps_cover_every_catalog_reward() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/params/catalog-rewards.json");
    let cat: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("params/catalog-rewards.json"))
            .unwrap();
    let p = params();
    assert_eq!(
        p["params_placeholder"],
        serde_json::Value::Bool(true),
        "params must stay labelled as placeholders until voted"
    );
    let mut max = std::collections::HashMap::<String, u128>::new();
    for b in cat["bounties"].as_array().unwrap() {
        for (_, a) in b["reward"]["amounts"].as_object().unwrap() {
            let d = a["denom"].as_str().unwrap().to_string();
            let m: u128 = a["amount_micro"].as_str().unwrap().parse().unwrap();
            let e = max.entry(d).or_default();
            *e = (*e).max(m);
        }
    }
    for (d, m) in max {
        let rule = denoms()
            .into_iter()
            .find(|r| r.denom == d)
            .unwrap_or_else(|| panic!("no rule for {d}"));
        assert!(
            rule.max_amount.u128() >= m,
            "{d}: cap {} below catalog max {m}",
            rule.max_amount
        );
        assert!(rule.min_amount.u128() <= m);
    }
}

#[test]
fn catalog_max_reward_accepted_and_cap_enforced() {
    let mut s = setup();
    let p = s.proposer.clone();
    // TB-SEC-002, the largest catalog reward (uthiol)
    let cat_max: u128 = params()["instantiate"]["denoms"][1]["_derivation"]["catalog_max_micro"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = s
        .create(
            &p,
            "TB-SEC-002",
            coin(cat_max, UTHIOL),
            &coins(cat_max, UTHIOL),
        )
        .unwrap();
    assert_eq!(attr(&res, "params_version"), params_version());
    assert_eq!(attr(&res, "params_placeholder"), "true");
    assert_eq!(attr(&res, "cap"), cap(UTHIOL).to_string());
    let id: u64 = attr(&res, "bounty_id").parse().unwrap();
    assert_eq!(s.bounty(id).params_version, params_version());

    let over = cap(UTERP) + 1;
    assert!(matches!(
        s.create(&p, "over", coin(over, UTERP), &coins(over, UTERP))
            .unwrap_err(),
        ContractError::AmountOutOfRange { .. }
    ));

    // governance (owner) raises the cap under a new, still-placeholder params_version
    let o = s.owner.clone();
    let mut d = denoms();
    d[0].max_amount = Uint256::new(over);
    s.exec(
        &o,
        &ExecuteMsg::UpdateConfig {
            owner: None,
            approver: None,
            gate: None,
            denoms: Some(d),
            allow_partial_payouts: None,
            fee_bps: None,
            fee_collector: None,
            min_duration_secs: None,
            max_duration_secs: None,
            params_version: Some("gov-test-2".into()),
            params_placeholder: Some(true),
            min_submit_stake: None,
            min_upload_stake: None,
            creation_gate: None,
        },
        &[],
    )
    .unwrap();
    let res = s
        .create(&p, "over", coin(over, UTERP), &coins(over, UTERP))
        .unwrap();
    assert_eq!(attr(&res, "params_version"), "gov-test-2");
}

#[test]
fn governance_params_need_owner_and_a_new_version() {
    let mut s = setup();
    let (o, r) = (s.owner.clone(), s.rando.clone());
    let upd = |denoms: Option<Vec<DenomRule>>,
               min_stake: Option<Uint256>,
               approver: Option<String>,
               v: Option<String>| ExecuteMsg::UpdateConfig {
        owner: None,
        approver,
        gate: min_stake.map(|m| ProposerGateMsg {
            use_allowlist: true,
            min_stake: Some(m),
            cw4_group: None,
        }),
        denoms,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: v,
        params_placeholder: None,
        min_submit_stake: None,
        min_upload_stake: None,
        creation_gate: None,
    };
    // not the owner
    assert!(matches!(
        s.exec(&r, &upd(Some(denoms()), None, None, Some("x".into())), &[])
            .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    // cap / min_stake / approver changes without a new label are refused
    for m in [
        upd(Some(denoms()), None, None, None),
        upd(None, Some(Uint256::new(5)), None, None),
        upd(None, None, Some(r.to_string()), None),
        upd(Some(denoms()), None, None, Some(params_version())),
    ] {
        assert!(matches!(
            s.exec(&o, &m, &[]).unwrap_err(),
            ContractError::InvalidInput(_)
        ));
    }
    s.exec(
        &o,
        &upd(
            None,
            Some(Uint256::new(5)),
            None,
            Some("gov-min-stake".into()),
        ),
        &[],
    )
    .unwrap();
    let cfg: terp_bounty_escrow::state::Config = s
        .app
        .wrap()
        .query_wasm_smart(&s.escrow, &QueryMsg::Config {})
        .unwrap();
    assert_eq!(
        (
            cfg.gate.min_stake,
            cfg.params_version.as_str(),
            cfg.params_placeholder
        ),
        (Some(Uint256::new(5)), "gov-min-stake", true)
    );
    // empty label at instantiate is refused
    let code = s.app.store_code(escrow_contract());
    let mut m = base_msg(&s, allowlist_gate(), false, 0);
    m.params_version = " ".into();
    let e = s
        .app
        .instantiate_contract(code, o.clone(), &m, &[], "x", None)
        .unwrap_err();
    assert!(matches!(decode(&e), ContractError::InvalidInput(_)));
}

// ---------------------------------------------------------------- summary / details / CID / submit stake

fn create_with(
    s: &mut Suite,
    r: &str,
    summary: &str,
    details_hash: &str,
    details_cid: Option<&str>,
) -> Result<AppResponse, ContractError> {
    let (p, exp) = (s.proposer.clone(), s.now().plus_seconds(30 * DAY));
    s.exec(
        &p,
        &ExecuteMsg::CreateBounty {
            bounty_ref: r.into(),
            title: "t".into(),
            summary: summary.into(),
            details_hash: details_hash.into(),
            details_cid: details_cid.map(Into::into),
            reward: coin(2_000_000, UTERP),
            expires_at: exp,
            proposer: None,
        },
        &coins(2_000_000, UTERP),
    )
}

#[test]
fn summary_is_capped_at_280_utf8_bytes_and_details_are_hash_referenced() {
    let mut s = setup();
    assert_eq!(SUMMARY_MAX_BYTES, 280);
    // exactly 280 bytes passes, counting bytes not chars: 93 x "€" (3 bytes) + 1 = 280
    let euro = format!("{}x", "€".repeat(93));
    assert_eq!(euro.len(), 280);
    let res = create_with(&mut s, "s1", &euro, HASH2, Some(CID0)).unwrap();
    assert_eq!(attr(&res, "summary"), euro);
    assert_eq!(attr(&res, "details_hash"), HASH2);
    assert_eq!(attr(&res, "details_cid"), CID0);
    let b = s.bounty(attr(&res, "bounty_id").parse().unwrap());
    assert_eq!(
        (
            b.summary.len(),
            b.details_hash.as_str(),
            b.details_cid.as_deref()
        ),
        (280, HASH2, Some(CID0))
    );
    // 281 bytes (94 chars) is refused
    let long = format!("{}xy", "€".repeat(93));
    assert!(
        matches!(create_with(&mut s, "s2", &long, HASH2, None).unwrap_err(), ContractError::InvalidInput(m) if m.contains("280 bytes"))
    );
    assert!(matches!(
        create_with(&mut s, "s3", "   ", HASH2, None).unwrap_err(),
        ContractError::InvalidInput(_)
    ));
    assert!(
        matches!(create_with(&mut s, "s4", "line\nbreak", HASH2, None).unwrap_err(), ContractError::InvalidInput(m) if m.contains("control"))
    );
    // details_hash is required and must be sha256 hex; details_cid optional but validated
    assert_eq!(
        create_with(&mut s, "s5", "ok", "not-a-hash", None).unwrap_err(),
        ContractError::InvalidHash
    );
    assert!(matches!(
        create_with(&mut s, "s6", "ok", HASH2, Some("bafy-not-a-cid")).unwrap_err(),
        ContractError::InvalidCid(_)
    ));
    create_with(&mut s, "s7", "ok", HASH2, None).unwrap();
}

#[test]
fn submit_cid_v0_and_v1_validated_on_chain() {
    let mut s = setup();
    let id = s.create_ok("c1", 2_000_000);
    let (ap, h) = (s.approver.clone(), s.hunter.clone());
    s.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id,
            assignee: h.to_string(),
        },
        &[],
    )
    .unwrap();
    for bad in [
        "",
        "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
        "QmY7Yh4UquoXHLPFo2XbhXkhBvFoPwmQUSa92pxnxjQuP0", // '0' not base58
        "Qmshort",
        "BAFYBEIGDYRZT5SFP7UDM7HU76UH7Y26NF3EFUYLQABF3OCLGTQY55FBZDI",
        "zdj7WWeQ43G6JJvLWQWZpyHuAMq6uYWRjkBXFad11vE2LHhQ7",
        "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzd",
        "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbz1",
    ] {
        assert!(
            matches!(
                s.exec(
                    &h,
                    &ExecuteMsg::Submit {
                        id,
                        submission_hash: HASH1.into(),
                        cid: bad.into()
                    },
                    &[]
                )
                .unwrap_err(),
                ContractError::InvalidCid(_)
            ),
            "{bad}"
        );
    }
    for (i, good) in [CID1, CID_RAW, CID0].iter().enumerate() {
        let res = s
            .exec(
                &h,
                &ExecuteMsg::Submit {
                    id,
                    submission_hash: HASH1.into(),
                    cid: (*good).into(),
                },
                &[],
            )
            .unwrap();
        assert_eq!(attr(&res, "submission_cid"), *good);
        assert_eq!(attr(&res, "submission_seq"), (i + 1).to_string());
        let p = s.proposer.clone();
        s.exec(&p, &ExecuteMsg::RequestChanges { id, reason: None }, &[])
            .unwrap();
    }
    let subs: SubmissionsResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::Submissions {
                id,
                start_after: None,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(
        subs.submissions
            .iter()
            .map(|x| x.cid.as_str())
            .collect::<Vec<_>>(),
        vec![CID1, CID_RAW, CID0]
    );
}

#[test]
fn submit_stake_gate_uses_a_staking_query_and_is_a_governance_param() {
    let mut s = setup();
    let min = min_submit_stake();
    let id = s.create_ok("k1", 2_000_000);
    let (ap, r, h, o) = (
        s.approver.clone(),
        s.rando.clone(),
        s.hunter.clone(),
        s.owner.clone(),
    );
    s.exec(&ap, &ExecuteMsg::Approve { id }, &[]).unwrap();
    s.exec(
        &ap,
        &ExecuteMsg::Assign {
            id,
            assignee: r.to_string(),
        },
        &[],
    )
    .unwrap();
    let q = |s: &Suite, a: &Addr| -> SubmitEligibilityResponse {
        s.app
            .wrap()
            .query_wasm_smart(
                &s.escrow,
                &QueryMsg::CanSubmit {
                    address: a.to_string(),
                },
            )
            .unwrap()
    };
    let c = q(&s, &r);
    assert_eq!(
        (
            c.eligible,
            c.staked.u128(),
            c.min_submit_stake.u128(),
            c.bond_denom.as_str()
        ),
        (false, 0, min, UTERP)
    );
    // rando (no stake) is assigned but blocked on chain
    assert!(
        matches!(s.exec(&r, &ExecuteMsg::Submit { id, submission_hash: HASH1.into(), cid: CID1.into() }, &[]).unwrap_err(),
        ContractError::BelowSubmitStake { staked, min: m, .. } if staked == "0" && m == min.to_string())
    );
    // one micro below the threshold is still blocked; at the threshold it passes
    let v = s.validator.clone();
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: v,
                amount: coin(min - 1, UTERP),
            }
            .into(),
        )
        .unwrap();
    assert!(matches!(
        s.exec(
            &r,
            &ExecuteMsg::Submit {
                id,
                submission_hash: HASH1.into(),
                cid: CID1.into()
            },
            &[]
        )
        .unwrap_err(),
        ContractError::BelowSubmitStake { .. }
    ));
    let v = s.validator.clone();
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: v,
                amount: coin(1, UTERP),
            }
            .into(),
        )
        .unwrap();
    assert!(q(&s, &r).eligible);
    s.exec(
        &r,
        &ExecuteMsg::Submit {
            id,
            submission_hash: HASH1.into(),
            cid: CID1.into(),
        },
        &[],
    )
    .unwrap();
    // governance raises the gate: needs the owner and a new params_version
    let upd = |m: u128, v: Option<String>| ExecuteMsg::UpdateConfig {
        owner: None,
        approver: None,
        gate: None,
        denoms: None,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: v,
        params_placeholder: None,
        min_submit_stake: Some(Uint256::new(m)),
        min_upload_stake: None,
        creation_gate: None,
    };
    assert!(matches!(
        s.exec(&h, &upd(min * 10, Some("x".into())), &[])
            .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    assert!(matches!(
        s.exec(&o, &upd(min * 10, None), &[]).unwrap_err(),
        ContractError::InvalidInput(_)
    ));
    s.exec(&o, &upd(min * 10, Some("gov-submit-stake".into())), &[])
        .unwrap();
    assert!(
        !q(&s, &h).eligible,
        "the hunter (exactly the old threshold) is now below it"
    );
    // 0 disables the gate
    s.exec(&o, &upd(0, Some("gov-submit-stake-off".into())), &[])
        .unwrap();
    assert!(q(&s, &s.rando.clone()).eligible);
}

#[test]
fn upload_stake_gate_is_a_governance_param_with_a_staking_query() {
    let mut s = setup();
    let min = min_upload_stake();
    assert!(
        min > 0 && min <= min_submit_stake(),
        "a hunter who may submit must also be able to upload"
    );
    let q = |s: &Suite, a: &Addr| -> UploadEligibilityResponse {
        s.app
            .wrap()
            .query_wasm_smart(
                &s.escrow,
                &QueryMsg::CanUpload {
                    address: a.to_string(),
                },
            )
            .unwrap()
    };
    let (r, h) = (s.rando.clone(), s.hunter.clone());
    let c = q(&s, &r);
    assert_eq!(
        (
            c.eligible,
            c.staked,
            c.min_upload_stake,
            c.bond_denom.as_str()
        ),
        (false, Uint256::zero(), Uint256::new(min), UTERP)
    );
    // the hunter is bonded min_submit_stake (>= min_upload_stake) in setup()
    let hc = q(&s, &h);
    assert!(hc.eligible);
    assert_eq!(hc.staked, Uint256::new(min_submit_stake()));
    // one micro below: blocked; exactly min: eligible
    let v = s.validator.clone();
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: v,
                amount: coin(min - 1, UTERP),
            }
            .into(),
        )
        .unwrap();
    assert!(!q(&s, &r).eligible);
    let v = s.validator.clone();
    s.app
        .execute(
            r.clone(),
            StakingMsg::Delegate {
                validator: v,
                amount: coin(1, UTERP),
            }
            .into(),
        )
        .unwrap();
    assert!(q(&s, &r).eligible);
    // governance raises it (needs a new params_version); non-owner refused
    let (o, rr) = (s.owner.clone(), s.rando.clone());
    let upd = |m: u128, v: Option<String>| ExecuteMsg::UpdateConfig {
        owner: None,
        approver: None,
        gate: None,
        denoms: None,
        allow_partial_payouts: None,
        fee_bps: None,
        fee_collector: None,
        min_duration_secs: None,
        max_duration_secs: None,
        params_version: v,
        params_placeholder: None,
        min_submit_stake: None,
        min_upload_stake: Some(Uint256::new(m)),
        creation_gate: None,
    };
    assert!(matches!(
        s.exec(&rr, &upd(min * 2, Some("x".into())), &[])
            .unwrap_err(),
        ContractError::Unauthorized(_)
    ));
    assert!(matches!(
        s.exec(&o, &upd(min * 2, None), &[]).unwrap_err(),
        ContractError::InvalidInput(_)
    ));
    let res = s
        .exec(&o, &upd(min * 2, Some("gov-upload-2x".into())), &[])
        .unwrap();
    assert_eq!(attr(&res, "min_upload_stake"), (min * 2).to_string());
    assert!(!q(&s, &r).eligible);
    // the submission gate is independent of the upload gate
    let cs: SubmitEligibilityResponse = s
        .app
        .wrap()
        .query_wasm_smart(
            &s.escrow,
            &QueryMsg::CanSubmit {
                address: h.to_string(),
            },
        )
        .unwrap();
    assert!(cs.eligible);
    // 0 disables
    s.exec(&o, &upd(0, Some("gov-upload-off".into())), &[])
        .unwrap();
    let z = s.app.api().addr_make("nobody");
    assert!(q(&s, &z).eligible);
    // invalid address is a query error, not a silent false
    assert!(
        s.app
            .wrap()
            .query_wasm_smart::<UploadEligibilityResponse>(
                &s.escrow,
                &QueryMsg::CanUpload {
                    address: "not-an-address".into()
                }
            )
            .is_err()
    );
}
