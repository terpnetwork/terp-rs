//! Mock (cw-multi-test) admin invariants for the FIAB and liquidity-steward
//! deploy scripts. Real dao-dao contracts, morocco-1 code IDs (filler codes
//! pad the store so 23/24/31/56/61 land on the right contracts), terp bech32.

use cosmwasm_std::{Addr, Coin, CosmosMsg, Empty, Uint256};
use cw_multi_test::{
    App, AppBuilder, BankSudo, Contract, ContractWrapper, Executor, MockApiBech32, SudoMsg,
};
use dao_foundation_suite::ibc_ops::steward::{build_steward_plan, load_steward_accounts};
use dao_foundation_suite::ibc_ops::{
    CODE_CORE, CODE_CW4_GROUP, CODE_PRE_PROPOSE_SINGLE, CODE_PROPOSAL_SINGLE, CODE_VOTING_CW4,
    PARENT_CORE, fiab_seed, morocco_fiab_instantiate,
};
use serde_json::{Value, json};

type TerpApp = App<cw_multi_test::BankKeeper, MockApiBech32>;

fn migrate_info() -> cosmwasm_std::MigrateInfo {
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
        .with_migrate(|d, e, m, _| dao_dao_core::contract::migrate(d, e, m, migrate_info())),
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
    Box::new(ContractWrapper::new(
        dao_pre_propose_single::contract::execute,
        dao_pre_propose_single::contract::instantiate,
        dao_pre_propose_single::contract::query,
    ))
}

/// App whose code IDs match morocco-1.
fn morocco_app() -> TerpApp {
    let mut app = AppBuilder::default()
        .with_api(MockApiBech32::new("terp"))
        .build(|_, _, _| {});
    let mut next = 1u64;
    for (want, code) in [
        (CODE_CORE, dao_core as fn() -> Box<dyn Contract<Empty>>),
        (CODE_PROPOSAL_SINGLE, proposal_single),
        (CODE_PRE_PROPOSE_SINGLE, pre_propose_single),
        (CODE_VOTING_CW4, voting_cw4),
        (CODE_CW4_GROUP, cw4_group),
    ] {
        while next < want {
            app.store_code(cw4_group());
            next += 1;
        }
        assert_eq!(app.store_code(code()), want);
        next += 1;
    }
    app
}

fn smart(app: &TerpApp, addr: &str, q: Value) -> Value {
    app.wrap().query_wasm_smart(addr, &q).unwrap()
}

fn instantiated(res: &cw_multi_test::AppResponse, code_id: u64) -> Vec<String> {
    res.events
        .iter()
        .filter(|e| e.ty == "instantiate")
        .filter(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == "code_id" && a.value == code_id.to_string())
        })
        .filter_map(|e| {
            e.attributes
                .iter()
                .find(|a| a.key == "_contract_address")
                .map(|a| a.value.clone())
        })
        .collect()
}

fn mint(app: &mut TerpApp, to: &str, coins: Vec<Coin>) {
    app.sudo(SudoMsg::Bank(BankSudo::Mint {
        to_address: to.into(),
        amount: coins,
    }))
    .unwrap();
}

#[test]
fn fiab_parent_is_wasm_and_dao_admin_and_gets_the_seed() {
    let mut app = morocco_app();
    mint(&mut app, PARENT_CORE, fiab_seed());
    let res = app
        .execute(
            Addr::unchecked(PARENT_CORE),
            morocco_fiab_instantiate(PARENT_CORE),
        )
        .expect("FIAB instantiate from the Foundation");
    let cores = instantiated(&res, CODE_CORE);
    assert_eq!(cores.len(), 1);
    let fiab = &cores[0];
    let info = app.wrap().query_wasm_contract_info(fiab).unwrap();
    assert_eq!(info.admin.as_ref().map(Addr::as_str), Some(PARENT_CORE));
    assert_eq!(info.creator.as_str(), PARENT_CORE);
    assert_eq!(smart(&app, fiab, json!({"admin": {}})), json!(PARENT_CORE));
    for c in fiab_seed() {
        assert_eq!(
            app.wrap().query_balance(fiab, &c.denom).unwrap().amount,
            c.amount
        );
    }
    // One proposal module, one voting module wired to cw4 code 61.
    assert_eq!(instantiated(&res, CODE_VOTING_CW4).len(), 1);
    assert_eq!(instantiated(&res, CODE_CW4_GROUP).len(), 1);
    assert_eq!(instantiated(&res, CODE_PROPOSAL_SINGLE).len(), 1);
}

#[test]
fn steward_daos_are_self_owned_and_custody_is_vetoed_by_steward() {
    let mut app = morocco_app();
    let checksum = app.wrap().query_wasm_code_info(CODE_CORE).unwrap().checksum;
    let accounts = load_steward_accounts(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/steward-accounts.json"),
    )
    .expect("steward-accounts.json");
    let plan = build_steward_plan(&accounts, PARENT_CORE, checksum.as_slice(), None, None).unwrap();
    let msgs: Vec<CosmosMsg> = serde_json::from_value(
        plan.foundation_proposal["propose"]["msg"]["propose"]["msgs"].clone(),
    )
    .unwrap();
    assert_eq!(msgs.len(), 4);
    for m in msgs {
        app.execute(Addr::unchecked(PARENT_CORE), m)
            .expect("instantiate2 from the Foundation");
    }
    for (name, addr) in &plan.addresses {
        let info = app
            .wrap()
            .query_wasm_contract_info(addr)
            .unwrap_or_else(|e| panic!("{name} not at predicted {addr}: {e}"));
        assert_eq!(info.code_id, CODE_CORE, "{name}");
        assert_eq!(info.admin, None, "{name}: wasm admin must be unset");
        assert_eq!(
            smart(&app, addr, json!({"admin": {}})),
            json!(addr),
            "{name}: core must own itself"
        );
        assert_ne!(smart(&app, addr, json!({"admin": {}})), json!(PARENT_CORE));
    }
    for chain in ["akash", "juno"] {
        let custody = &plan.addresses[&format!("custody-{chain}")];
        let steward = &plan.addresses[&format!("steward-{chain}")];
        let modules = smart(&app, custody, json!({"proposal_modules": {}}));
        let prop = modules[0]["address"].as_str().unwrap();
        let cfg = smart(&app, prop, json!({"config": {}}));
        assert_eq!(
            cfg["veto"]["vetoer"],
            json!(steward),
            "{chain} custody vetoer"
        );
        assert_eq!(cfg["veto"]["timelock_duration"]["time"], json!(48 * 3600));
        assert_eq!(cfg["veto"]["veto_before_passed"], json!(true));
        assert_eq!(cfg["veto"]["early_execute"], json!(false));
    }
    // Custody-submitted TERP subDAO: custody is wasm admin, dao admin, vetoer.
    for sub in &plan.subdaos {
        let custody = sub.body["parent"].as_str().unwrap().to_string();
        let msgs: Vec<CosmosMsg> = serde_json::from_value(
            sub.body["terp_subdao"]["propose"]["propose"]["msg"]["propose"]["msgs"].clone(),
        )
        .unwrap();
        let res = app
            .execute(Addr::unchecked(&custody), msgs[0].clone())
            .expect("subDAO from custody");
        let core = &instantiated(&res, CODE_CORE)[0];
        let info = app.wrap().query_wasm_contract_info(core).unwrap();
        assert_eq!(
            info.admin.as_ref().map(Addr::as_str),
            Some(custody.as_str())
        );
        assert_eq!(smart(&app, core, json!({"admin": {}})), json!(custody));
        let prop = smart(&app, core, json!({"proposal_modules": {}}))[0]["address"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            smart(&app, &prop, json!({"config": {}}))["veto"]["vetoer"],
            json!(custody)
        );
    }
    let _ = Uint256::zero();
}
