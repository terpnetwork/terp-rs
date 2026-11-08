//! F1 (soundness review 05-findings): vk_id 2 is refused end to end through the
//! contract entry points on a multi-test chain.
use cosmwasm_std::{Addr, Binary};
use cw_headstash::epoch::{AmountCap, EpochCampaignParams};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg};
use cw_headstash::notes::{NOTE_VK_HASH, RETIRED_VK_ERR, RETIRED_VK_ID};
use cw_headstash::tokenfactory::{
    derive_nd, DenomUnit, HeadstashTokenObject, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_multi_test::{ContractWrapper, Executor};
include!("common/tf_stargate.rs");
use tf_stargate::{new_app, TfApp as App};

fn deploy() -> (App, Addr, Addr) {
    let mut app = new_app();
    let owner = app.api().addr_make("owner");
    let code = app.store_code(Box::new(
        ContractWrapper::new(cw_headstash::execute, cw_headstash::instantiate, cw_headstash::query)
            .with_reply(cw_headstash::reply),
    ));
    let msg = InstantiateMsg {
        genesis_root: Binary::from(vec![0u8; 32]),
        distro_hash_domain: Default::default(),
        genesis_label: None,
        token_strategy: TokenStrategy::NewFungible(Box::new(NewTokenConfig {
            subdenom: HeadstashTokenObject::new(app.api(), "cht".into()),
            metadata: Metadata {
                description: None,
                denom_units: vec![DenomUnit { denom: "ucht".into(), exponent: 0, aliases: vec![] }],
                base: Some("ucht".into()),
                display: Some("CHT".into()),
                name: Some("CHT".into()),
                symbol: Some("CHT".into()),
            },
            initial_mint: None,
            manager: None,
            minters: vec![],
        })),
        wavs: None,
        circuit_id: None,
        registration_hook: None,
        registration_hook_gas_limit: None,
        frontend_contract: None,
        pauser: None,
    };
    let addr = app
        .instantiate_contract(code, owner.clone(), &msg, &[], "headstash", None)
        .unwrap();
    (app, owner, addr)
}

fn campaign(vk_id: u64) -> EpochCampaignParams {
    EpochCampaignParams {
        ctx: Binary::from(vec![5u8; 32]),
        backend: "halo2".into(),
        cutoff_height: 1_000_000,
        bech32_hrp: "cosmwasm".into(),
        denom: derive_nd(&cosmwasm_std::testing::MockApi::default(), "cht"),
        amounts: vec![AmountCap { amount: 15, cap: 1 }],
        reserved: vec![],
        trusted: vec![Binary::from(vec![8u8; 32])],
        trusted_verify_threshold: 1,
        trusted_sign_threshold: 1,
        max_rows: 64,
        allocation_root: Binary::from(vec![0u8; 32]),
        root_id: 0,
        vk_id,
        vk_hash: Binary::from([9u8; 32].to_vec()),
        sweep_recipient: String::new(),
        burn_unclaimed: None,
        registration: None,
    }
}

fn err_of(app: &mut App, owner: &Addr, addr: &Addr, msg: &ExecuteMsg) -> String {
    let e = app.execute_contract(owner.clone(), addr.clone(), msg, &[]).unwrap_err();
    format!("{e:?}")
}

#[test]
fn f1_vk2_campaign_cannot_be_set_or_rebound() {
    let (mut app, owner, addr) = deploy();
    // A new vk_id 2 campaign is refused, with the registration lane or without it.
    let e = err_of(&mut app, &owner, &addr, &ExecuteMsg::SetEpochCampaign { campaign: campaign(RETIRED_VK_ID) });
    assert!(e.contains(RETIRED_VK_ERR), "{e}");
    let mut vk2_note = campaign(RETIRED_VK_ID);
    vk2_note.vk_hash = Binary::from(NOTE_VK_HASH.to_vec());
    let e = err_of(&mut app, &owner, &addr, &ExecuteMsg::SetEpochCampaign { campaign: vk2_note });
    assert!(e.contains(RETIRED_VK_ERR), "{e}");

    // A live campaign cannot be switched to vk_id 2.
    app.execute_contract(owner.clone(), addr.clone(), &ExecuteMsg::SetEpochCampaign { campaign: campaign(1) }, &[])
        .unwrap();
    let e = err_of(&mut app, &owner, &addr, &ExecuteMsg::SetEpochCampaign { campaign: campaign(RETIRED_VK_ID) });
    assert!(e.contains(RETIRED_VK_ERR), "{e}");

    // The direct note path never binds or claims outside vk_id 3.
    let e = err_of(&mut app, &owner, &addr, &ExecuteMsg::BindNoteCircuit { zk_id: 1 });
    assert!(e.contains("not a note campaign"), "{e}");
    let e = err_of(
        &mut app,
        &owner,
        &addr,
        &ExecuteMsg::ClaimNote {
            root: Binary::from(vec![1u8; 32]),
            nullifier: Binary::from(vec![2u8; 32]),
            amount: 1_000_000,
            proof: Binary::from(vec![0u8; 64]),
        },
    );
    assert!(e.contains("not a note campaign") || e.contains("circuit"), "{e}");
}
