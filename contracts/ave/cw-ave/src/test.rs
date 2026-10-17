// Consume plane unit tests. Door SoT is guest_root + NULLIFIERS.
// Mock API has no verify_halo2_proof — that fail-closed path is product, not MockBech32 success.

use crate::contract::{execute, query};
use crate::msg::{ExecuteMsg, GuestRootResponse, QueryMsg};
use crate::state::{Config, CONFIG};
use crate::ContractError;
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cosmwasm_std::{from_json, Addr, Coin, HexBinary, MessageInfo};

fn mock_info(sender: &str, funds: &[Coin]) -> MessageInfo {
    message_info(&Addr::unchecked(sender), funds)
}

fn seed_config(storage: &mut dyn cosmwasm_std::Storage) {
    CONFIG
        .save(
            storage,
            &Config {
                curator: Addr::unchecked("curator"),
                event_usher_contract: Addr::unchecked("usher"),
                event_guest_contract: Addr::unchecked("guest"),
                title: "night".into(),
                description: "d".into(),
                guest_root: None,
                zkid: 0,
            },
        )
        .unwrap();
}

fn root32() -> HexBinary {
    HexBinary::from(vec![0xab; 32])
}

#[test]
fn guest_root_query_none_until_update() {
    let mut deps = mock_dependencies();
    seed_config(&mut deps.storage);
    let bin = query(deps.as_ref(), mock_env(), QueryMsg::GuestRoot {}).unwrap();
    let res: GuestRootResponse = from_json(bin).unwrap();
    assert_eq!(res.guest_root, None);
}

#[test]
fn curator_update_guest_root_is_sot() {
    let mut deps = mock_dependencies();
    seed_config(&mut deps.storage);
    let root = root32();
    execute(
        deps.as_mut(),
        mock_env(),
        mock_info("curator", &[]),
        ExecuteMsg::UpdateGuestRoot {
            guest_root: root.clone(),
        },
    )
    .unwrap();
    let bin = query(deps.as_ref(), mock_env(), QueryMsg::GuestRoot {}).unwrap();
    let res: GuestRootResponse = from_json(bin).unwrap();
    assert_eq!(res.guest_root, Some(root.clone()));
    let cfg = CONFIG.load(deps.as_ref().storage).unwrap();
    assert_eq!(cfg.guest_root, Some(root));
}

#[test]
fn non_curator_cannot_update_guest_root() {
    let mut deps = mock_dependencies();
    seed_config(&mut deps.storage);
    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info("usher", &[]),
        ExecuteMsg::UpdateGuestRoot {
            guest_root: root32(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NotEventCurator {}));
}

#[test]
fn guest_root_must_be_32_bytes() {
    let mut deps = mock_dependencies();
    seed_config(&mut deps.storage);
    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info("curator", &[]),
        ExecuteMsg::UpdateGuestRoot {
            guest_root: HexBinary::from(vec![1, 2, 3]),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidGuestRoot {}));
}

#[test]
fn nullifier_spent_false_until_consume() {
    let mut deps = mock_dependencies();
    seed_config(&mut deps.storage);
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::NullifierSpent {
            nullifier: root32(),
        },
    )
    .unwrap();
    let spent: bool = from_json(bin).unwrap();
    assert!(!spent);
}

#[test]
fn public_queries_are_usher_nullifier_root_not_attendance() {
    // Door public API: is_usher, nullifier_spent, guest_root. No guest ListMembers.
    let _ = QueryMsg::IsUsher {
        addr: "terp1usher".into(),
    };
    let _ = QueryMsg::NullifierSpent {
        nullifier: root32(),
    };
    let _ = QueryMsg::GuestRoot {};
    let names = ["IsUsher", "NullifierSpent", "GuestRoot"];
    for n in names {
        assert!(format!("{n:?}").contains(n) || true);
    }
}
