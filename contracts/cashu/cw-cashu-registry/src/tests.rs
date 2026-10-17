use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi};
use cosmwasm_std::{from_json, Addr};

use crate::contract::{
    execute, instantiate, mint_id_from_url, normalize_url, query, validate_mint_id,
};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::MintStatus;

type TestDeps = cosmwasm_std::OwnedDeps<
    cosmwasm_std::testing::MockStorage,
    cosmwasm_std::testing::MockApi,
    cosmwasm_std::testing::MockQuerier,
>;

fn admin() -> Addr {
    MockApi::default().addr_make("admin")
}
fn alice() -> Addr {
    MockApi::default().addr_make("alice")
}
fn bob() -> Addr {
    MockApi::default().addr_make("bob")
}

fn setup(deps: &mut TestDeps, allow_public: bool) {
    instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        InstantiateMsg {
            admin: Some(admin().to_string()),
            max_metadata_bytes: Some(512),
            allow_public_register: allow_public,
        },
    )
    .unwrap();
}

fn register_admin(deps: &mut TestDeps, url: &str, mint_id: Option<&str>) {
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: mint_id.map(|s| s.to_string()),
            url: url.to_string(),
            name: Some("test mint".into()),
            units: vec!["sat".into()],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap();
}

// ── helpers ──────────────────────────────────────────────────────────────

#[test]
fn normalize_strips_slash_and_lowercases_host() {
    assert_eq!(
        normalize_url("HTTPS://Mint.Example/path/"),
        "https://mint.example/path"
    );
    assert_eq!(normalize_url("  https://x.com  "), "https://x.com");
}

#[test]
fn mint_id_charset() {
    assert!(validate_mint_id("abc-123._x").is_ok());
    assert!(validate_mint_id("a..b").is_err());
    assert!(validate_mint_id("").is_err());
    assert!(validate_mint_id(&"a".repeat(201)).is_err());
    assert!(validate_mint_id("bad/id").is_err());
}

#[test]
fn derived_mint_id_is_sha256_hex() {
    let n = normalize_url("https://mint.example/");
    let id = mint_id_from_url(&n);
    assert_eq!(id.len(), 64);
    assert!(validate_mint_id(&id).is_ok());
    // stable
    assert_eq!(id, mint_id_from_url("https://mint.example"));
}

// ── register → query ─────────────────────────────────────────────────────

#[test]
fn register_and_query_mint() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);

    register_admin(&mut deps, "https://mint.example/", Some("lab-mint-1"));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: "lab-mint-1".into(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    let m = m.expect("mint present");
    assert_eq!(m.mint_id, "lab-mint-1");
    assert_eq!(m.url, "https://mint.example");
    assert_eq!(m.status, MintStatus::Active);
    assert_eq!(m.name.as_deref(), Some("test mint"));
    assert_eq!(m.units, vec!["sat".to_string()]);
    assert_eq!(m.registrar, admin());

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::MintByUrl {
            url: "HTTPS://mint.example/".into(),
        },
    )
    .unwrap();
    let m2: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m2.unwrap().mint_id, "lab-mint-1");

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::IsRegistered {
            mint_id: Some("lab-mint-1".into()),
            url: None,
        },
    )
    .unwrap();
    let r: IsRegisteredResponse = from_json(bin).unwrap();
    assert!(r.registered);
}

#[test]
fn register_derives_mint_id_from_url() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);

    register_admin(&mut deps, "https://auto.example", None);
    let expected = mint_id_from_url(&normalize_url("https://auto.example"));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: expected.clone(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m.unwrap().mint_id, expected);
}

// ── list / status filter ─────────────────────────────────────────────────

#[test]
fn list_active_only_by_default() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);

    register_admin(&mut deps, "https://a.example", Some("mint-a"));
    register_admin(&mut deps, "https://b.example", Some("mint-b"));

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::SetStatus {
            mint_id: "mint-b".into(),
            status: MintStatus::Paused,
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ListMints {
            start_after: None,
            limit: None,
            status_filter: None,
        },
    )
    .unwrap();
    let list: ListMintsResponse = from_json(bin).unwrap();
    assert_eq!(list.mints.len(), 1);
    assert_eq!(list.mints[0].mint_id, "mint-a");

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ListMints {
            start_after: None,
            limit: None,
            status_filter: Some(MintStatus::Paused),
        },
    )
    .unwrap();
    let list: ListMintsResponse = from_json(bin).unwrap();
    assert_eq!(list.mints.len(), 1);
    assert_eq!(list.mints[0].mint_id, "mint-b");
}

#[test]
fn set_status_revoked_filtered_from_default_list() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://gone.example", Some("gone"));

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::SetStatus {
            mint_id: "gone".into(),
            status: MintStatus::Revoked,
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ListMints {
            start_after: None,
            limit: None,
            status_filter: None,
        },
    )
    .unwrap();
    let list: ListMintsResponse = from_json(bin).unwrap();
    assert!(list.mints.is_empty());

    // Still queryable by id (tombstone kept).
    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: "gone".into(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m.unwrap().status, MintStatus::Revoked);
}

#[test]
fn remove_mint_soft_revokes() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://rm.example", Some("rm-1"));

    let res = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::RemoveMint {
            mint_id: "rm-1".into(),
        },
    )
    .unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "action" && a.value == "remove"));
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "status" && a.value == "revoked"));

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: "rm-1".into(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m.unwrap().status, MintStatus::Revoked);
}

// ── auth ─────────────────────────────────────────────────────────────────

#[test]
fn unauthorized_register_when_public_disabled() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&alice(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: Some("x".into()),
            url: "https://x.example".into(),
            name: None,
            units: vec![],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn public_register_allowed_when_flag_set() {
    let mut deps = mock_dependencies();
    setup(&mut deps, true);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&alice(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: Some("pub-1".into()),
            url: "https://pub.example".into(),
            name: None,
            units: vec![],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: "pub-1".into(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m.unwrap().registrar, alice());
}

#[test]
fn non_admin_cannot_set_status() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://s.example", Some("s1"));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&bob(), &[]),
        ExecuteMsg::SetStatus {
            mint_id: "s1".into(),
            status: MintStatus::Paused,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

// ── duplicates ───────────────────────────────────────────────────────────

#[test]
fn duplicate_mint_id_fails() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://d1.example", Some("dup"));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: Some("dup".into()),
            url: "https://d2.example".into(),
            name: None,
            units: vec![],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::MintIdExists {
            mint_id: "dup".into()
        }
    );
}

#[test]
fn duplicate_normalized_url_fails() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://same.example/", Some("u1"));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&admin(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: Some("u2".into()),
            url: "HTTPS://Same.Example".into(),
            name: None,
            units: vec![],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::UrlExists {
            mint_id: "u1".into()
        }
    );
}

#[test]
fn registrar_can_update_mint() {
    let mut deps = mock_dependencies();
    setup(&mut deps, true);

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&alice(), &[]),
        ExecuteMsg::RegisterMint {
            mint_id: Some("alice-mint".into()),
            url: "https://alice.example".into(),
            name: Some("old".into()),
            units: vec![],
            keyset_ids: vec![],
            nuts: Default::default(),
            pubkey: None,
            status: None,
            content_sha256: None,
            metadata: Default::default(),
        },
    )
    .unwrap();

    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&alice(), &[]),
        ExecuteMsg::UpdateMint {
            mint_id: "alice-mint".into(),
            url: None,
            name: Some(Some("new".into())),
            units: None,
            keyset_ids: None,
            nuts: None,
            pubkey: None,
            content_sha256: None,
            metadata: None,
        },
    )
    .unwrap();

    let bin = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::Mint {
            mint_id: "alice-mint".into(),
        },
    )
    .unwrap();
    let m: Option<crate::state::MintDescriptor> = from_json(bin).unwrap();
    assert_eq!(m.unwrap().name.as_deref(), Some("new"));
}

#[test]
fn stranger_cannot_update_mint() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    register_admin(&mut deps, "https://own.example", Some("own"));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        message_info(&bob(), &[]),
        ExecuteMsg::UpdateMint {
            mint_id: "own".into(),
            url: None,
            name: Some(Some("hacked".into())),
            units: None,
            keyset_ids: None,
            nuts: None,
            pubkey: None,
            content_sha256: None,
            metadata: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn config_query() {
    let mut deps = mock_dependencies();
    setup(&mut deps, false);
    let bin = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap();
    let cfg: ConfigResponse = from_json(bin).unwrap();
    assert_eq!(cfg.admin, admin());
    assert!(!cfg.allow_public_register);
    assert_eq!(cfg.max_metadata_bytes, 512);
}
