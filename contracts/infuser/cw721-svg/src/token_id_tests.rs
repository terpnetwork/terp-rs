//! Token ids come from a counter that only goes up. Regression for the
//! permissionless fractal (morocco-1): burning token 12 dropped the live count
//! to 17, so every later mint retried the taken id 17.
use crate::contract::{MigrateMsg, NEXT_TOKEN_ID};
use crate::entry::{execute, instantiate, migrate, query};
use crate::{
    ContractError, ExecuteMsg, InstantiateMsg, QueryMsg, SvgCollectionMetadata, SvgExecuteMsgExt,
};
use cosmwasm_std::testing::{
    MockApi, MockQuerier, MockStorage, message_info, mock_dependencies, mock_env,
};
use cosmwasm_std::{Addr, Binary, OwnedDeps, Storage, from_json};
use cw_svg::{VariableDef, VariableKind};

type Deps = OwnedDeps<MockStorage, MockApi, MockQuerier>;

fn setup(total: u64) -> (Deps, Addr) {
    let mut deps = mock_dependencies();
    let owner = deps.api.addr_make("owner");
    let v = vec![VariableDef {
        name: "a".into(),
        kind: VariableKind::Options(vec!["x".into(), "y".into()]),
    }];
    let t = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>${a}</text></svg>";
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&owner, &[]),
        InstantiateMsg {
            name: "fractal".into(),
            symbol: "FRACTAL".into(),
            minter: None,
            creator: None,
            withdraw_address: None,
            collection_info_extension: SvgCollectionMetadata {
                seed: Binary::from(b"seed".as_slice()),
                packed: crate::pack_svg(t, &v),
                price_tiers: vec![],
                total,
                mint_start_time: None,
                mint_end_time: None,
                whitelist: None,
                require_auth: false,
            },
        },
    )
    .unwrap();
    (deps, owner)
}

fn mint(deps: &mut Deps, who: &Addr, amnt: u64) -> Result<Vec<String>, ContractError> {
    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(who, &[]),
        ExecuteMsg::UpdateExtension {
            msg: SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
                alloc: 0,
                amnt,
                proof_hashes: vec![],
                ext: None,
            }),
        },
    )?;
    Ok(res
        .events
        .iter()
        .filter(|e| e.ty == "mint")
        .flat_map(|e| e.attributes.iter().filter(|a| a.key == "token_id"))
        .map(|a| a.value.clone())
        .collect())
}

fn burn(deps: &mut Deps, who: &Addr, id: u64) {
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(who, &[]),
        ExecuteMsg::Burn {
            token_id: id.to_string(),
        },
    )
    .unwrap();
}

fn count(deps: &Deps) -> u64 {
    from_json::<cw721::msg::NumTokensResponse>(
        query(deps.as_ref(), mock_env(), QueryMsg::NumTokens {}).unwrap(),
    )
    .unwrap()
    .count
}

fn counter(deps: &Deps) -> Option<u64> {
    NEXT_TOKEN_ID.may_load(&deps.storage).unwrap()
}

/// Mint ids 0..=17 one at a time, then burn 12 (the mainnet history).
fn fractal_history(total: u64) -> (Deps, Addr) {
    let (mut deps, owner) = setup(total);
    for i in 0..18u64 {
        assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec![i.to_string()]);
    }
    burn(&mut deps, &owner, 12);
    assert_eq!(count(&deps), 17);
    (deps, owner)
}

/// What code 98 left in storage: tokens, but no counter.
fn drop_counter(deps: &mut Deps) {
    deps.storage.remove(b"svg_next_token_id");
}

fn migrate_with(deps: &mut Deps, next_token_id: Option<u64>) -> Result<u64, String> {
    migrate(
        deps.as_mut(),
        mock_env(),
        MigrateMsg {
            dregg_ibc_only: None,
            runtime_minters: None,
            next_token_id,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(counter(deps).unwrap())
}

#[test]
fn instantiate_starts_counter_at_zero() {
    let (deps, _) = setup(10);
    assert_eq!(counter(&deps), Some(0));
}

#[test]
fn burn_then_mint_takes_the_next_unused_id() {
    let (mut deps, owner) = fractal_history(10_000);
    assert_eq!(counter(&deps), Some(18));
    assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec!["18"]);
    assert_eq!(mint(&mut deps, &owner, 2).unwrap(), vec!["19", "20"]);
    assert_eq!(count(&deps), 20);
    assert_eq!(counter(&deps), Some(21));
}

#[test]
fn uncounted_collection_skips_taken_ids() {
    // Code-98 state with the new code but no migrate arg: start at the live
    // count (17), skip the taken 17, land on 18.
    let (mut deps, owner) = fractal_history(10_000);
    drop_counter(&mut deps);
    assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec!["18"]);
    assert_eq!(counter(&deps), Some(19));
}

#[test]
fn skip_runs_inside_a_multi_mint() {
    let (mut deps, owner) = setup(100);
    mint(&mut deps, &owner, 5).unwrap();
    NEXT_TOKEN_ID.save(&mut deps.storage, &2).unwrap();
    assert_eq!(mint(&mut deps, &owner, 2).unwrap(), vec!["5", "6"]);
    assert_eq!(counter(&deps), Some(7));
}

#[test]
fn supply_cap_uses_the_counter_not_the_live_count() {
    let (mut deps, owner) = setup(5);
    mint(&mut deps, &owner, 5).unwrap();
    burn(&mut deps, &owner, 1);
    assert_eq!(count(&deps), 4);
    assert!(matches!(
        mint(&mut deps, &owner, 1),
        Err(ContractError::CannotMintMoreThanTotal {})
    ));
    // a skip that would run past the cap is refused too
    let (mut deps, owner) = setup(6);
    mint(&mut deps, &owner, 6).unwrap();
    burn(&mut deps, &owner, 0);
    NEXT_TOKEN_ID.save(&mut deps.storage, &3).unwrap();
    assert!(matches!(
        mint(&mut deps, &owner, 1),
        Err(ContractError::CannotMintMoreThanTotal {})
    ));
}

#[test]
fn migrate_sets_the_fractal_counter() {
    let (mut deps, owner) = fractal_history(10_000);
    drop_counter(&mut deps);
    assert_eq!(migrate_with(&mut deps, Some(18)).unwrap(), 18);
    assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec!["18"]);
}

#[test]
fn migrate_default_is_highest_id_plus_one() {
    let (mut deps, owner) = fractal_history(10_000);
    drop_counter(&mut deps);
    assert_eq!(migrate_with(&mut deps, None).unwrap(), 18);
    assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec!["18"]);
    // empty collection defaults to 0
    let (mut deps, _) = setup(10);
    drop_counter(&mut deps);
    assert_eq!(migrate_with(&mut deps, None).unwrap(), 0);
}

#[test]
fn migrate_counter_only_goes_up() {
    let (mut deps, owner) = fractal_history(10_000);
    assert!(migrate_with(&mut deps, Some(5)).is_err());
    assert_eq!(counter(&deps), Some(18));
    assert_eq!(migrate_with(&mut deps, None).unwrap(), 18);
    assert_eq!(migrate_with(&mut deps, Some(18)).unwrap(), 18);
    assert_eq!(migrate_with(&mut deps, Some(25)).unwrap(), 25);
    assert_eq!(mint(&mut deps, &owner, 1).unwrap(), vec!["25"]);
}

#[test]
fn migrate_msg_without_the_field_still_parses() {
    let m: MigrateMsg = from_json(br#"{"dregg_ibc_only":null,"runtime_minters":null}"#).unwrap();
    assert_eq!(m.next_token_id, None);
    let m: MigrateMsg = from_json(br#"{"next_token_id":18}"#).unwrap();
    assert_eq!(m.next_token_id, Some(18));
}
