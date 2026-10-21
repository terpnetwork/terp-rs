#![cfg(feature = "orch-suite")]
use ::terp721_account::{ContractError, commands::transcode};
use cosmwasm_std::testing::mock_dependencies;
use cosmwasm_std::{Api, Binary, from_json};
use cw_orch::prelude::CallAs;
use cw_orch::{anyhow, mock::MockBech32, prelude::*};
use terp_orch::suites::accounts::{TerpAccountDeployData, TerpAccountExecuteFns, TerpAccountSuite};

use terp_account::verify_generic::{
    CosmosArbitrary, TestCosmosArb, preamble_msg_arb_036, pubkey_to_address,
};
use terp_account::{Metadata, NFT, TERP_PREFIX, TextRecord};
use terp721_account::msg::Terp721AccountsQueryMsgFns;
use terp721_account::state::REVERSE_MAP_KEY;

use ecdsa::signature::rand_core::OsRng;
use k256::ecdsa::{Signature, SigningKey, VerifyingKey};
// use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use sha2::digest::Update;

#[test]
fn init() -> anyhow::Result<()> {
    // new mock Bech32 chain environment
    let mock = MockBech32::new("mock");
    // simulate deploying the test suite to the mock chain env.
    TerpAccountSuite::deploy_on(mock.clone(), TerpAccountDeployData { admin: mock.sender })?;
    Ok(())
}
#[test]
fn mint_and_update() -> anyhow::Result<()> {
    let mock = MockBech32::new("mock");
    let mut suite = TerpAccountSuite::new(mock.clone());
    suite.default_setup(mock.clone(), None, None)?;

    let not_minter = mock.addr_make("not-minter");
    let minter = suite.manifold.address()?;

    // retrieve max record count
    let params = suite.nft.params()?;
    let max_record_count = params.max_record_count;

    // mint token
    let token_id = "Enterprise";

    let err = suite
        .nft
        .call_as(&not_minter)
        .mint(
            terp_account::Metadata::default(),
            not_minter.clone(),
            token_id,
            None,
        )
        .unwrap_err();

    assert!(
        err.to_string()
            .contains(&ContractError::UnauthorizedMinter {}.to_string()),
        "Unexpected error: {err}"
    );

    suite.nft.call_as(&minter).mint(
        terp_account::Metadata::default(),
        mock.sender,
        token_id,
        None,
    )?;

    // check token contains correct metadata
    let res = suite.nft.nft_info(token_id)?;

    assert_eq!(res.token_uri, None);
    assert_eq!(res.extension, terp_account::Metadata::default());

    // update image
    let new_nft = terp_account::NFT {
        collection: Addr::unchecked("contract"),
        token_id: "token_id".to_string(),
    };
    let nft_value = suite
        .nft
        .update_image_nft(token_id, Some(new_nft.clone()))?
        .event_attr_value("wasm-update_image_nft", "image_nft")
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .into_bytes();
    let nft: terp_account::NFT = from_json(nft_value).unwrap();
    assert_eq!(nft, new_nft);

    // add text record
    let new_record = terp_account::TextRecord {
        account: "test".to_string(),
        value: "test".to_string(),
        verified: None,
    };
    let record_value = suite
        .nft
        .update_text_record(token_id.to_string(), new_record.clone())?
        .event_attr_value("wasm-update-text-record", "record")
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .into_bytes();

    let record: terp_account::TextRecord =
        from_json(record_value).map_err(|e| anyhow::anyhow!("{e}"))?;
    assert_eq!(record, new_record);
    let records = suite.nft.text_records(token_id)?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].account, "test");
    assert_eq!(records[0].value, "test");

    // trigger too many records error
    for i in 1..=(max_record_count) {
        let new_record = terp_account::TextRecord {
            account: format!("key{:?}", i),
            value: "value".to_string(),
            verified: None,
        };
        if i == max_record_count {
            let res = suite.nft.update_text_record(token_id, new_record);
            assert!(
                res.unwrap_err().to_string().contains(
                    &ContractError::TooManyRecords {
                        max: max_record_count
                    }
                    .to_string()
                )
            );
            break;
        } else {
            suite.nft.update_text_record(token_id, new_record)?;
        }
    }

    // rm text records
    suite.nft.remove_text_record(token_id, "test")?;

    for i in 1..=(max_record_count) {
        let record_account = format!("key{:?}", i);
        suite.nft.remove_text_record(token_id, record_account)?;
    }
    // txt record count should be 0
    let res = suite.nft.nft_info(token_id)?;
    assert_eq!(res.extension.records.len(), 0);

    // unauthorized add txt record
    let err = suite
        .nft
        .call_as(&not_minter)
        .add_text_record(token_id, new_record.clone())
        .unwrap_err();
    assert!(
        err.to_string().contains(
            &ContractError::Cw721(cw721::error::Cw721ContractError::Ownership(
                cw_ownable::OwnershipError::NotOwner
            ))
            .to_string()
        )
    );
    // passes
    suite.nft.add_text_record(token_id, new_record)?;
    assert_eq!(suite.nft.nft_info(token_id)?.extension.records.len(), 1);

    // add another txt record
    let record = terp_account::TextRecord {
        account: "twitter".to_string(),
        value: "jackdorsey".to_string(),
        verified: None,
    };
    suite.nft.add_text_record(token_id, record)?;
    assert_eq!(suite.nft.nft_info(token_id)?.extension.records.len(), 2);

    // add duplicate record RecordAccountAlreadyExists
    let record = terp_account::TextRecord {
        account: "test".to_string(),
        value: "testtesttest".to_string(),
        verified: None,
    };
    assert!(
        suite
            .nft
            .add_text_record(token_id, record.clone())
            .unwrap_err()
            .to_string()
            .contains(&ContractError::RecordAccountAlreadyExists {}.to_string())
    );
    // update txt record
    suite.nft.update_text_record(token_id, record.clone())?;
    let res = suite.nft.nft_info(token_id)?;
    assert_eq!(res.extension.records.len(), 2);
    assert_eq!(res.extension.records[1].value, record.value);
    // rm txt record
    suite.nft.remove_text_record(token_id, record.account)?;
    let res = suite.nft.nft_info(token_id)?;
    assert_eq!(res.extension.records.len(), 1);

    Ok(())
}

#[test]
fn test_query_accounts() -> anyhow::Result<()> {
    let mock = MockBech32::new(TERP_PREFIX);
    let mut suite = TerpAccountSuite::new(mock.clone());
    suite.default_setup(mock.clone(), None, None)?;

    let addr = mock.addr_make("babber");

    assert!(
        suite
            .nft
            .account(addr.clone().to_string())
            .unwrap_err()
            .to_string()
            .contains(&format!("No account associated with address {}", addr))
    );
    Ok(())
}

#[test]
fn test_burn_function() -> anyhow::Result<()> {
    let mock = MockBech32::new(TERP_PREFIX);
    let mut suite = TerpAccountSuite::new(mock.clone());
    suite.default_setup(mock.clone(), None, None)?;
    let addr = mock.addr_make("babber23");
    let token_id = "enterprise";
    mock.wait_seconds(200u64)?;

    // mint token
    suite.mint_and_list(mock.clone(), token_id, &mock.sender.clone())?;

    // cannot burn token you dont own
    let err = suite.nft.call_as(&addr).burn(token_id).unwrap_err();
    assert!(
        err.to_string()
            .contains("Caller is not the contract's current owner"),
        "Unexpected error: {err}"
    );

    // token acutally gets burnt
    suite.nft.burn(token_id)?;
    let res = suite.nft.associated_address(token_id);
    assert!(res.is_err());
    let res = suite.nft.account(mock.sender.to_string());
    assert!(res.is_err());

    Ok(())
}

#[test]
fn test_reverse_map_key_limit() -> anyhow::Result<()> {
    let mock = MockBech32::new(TERP_PREFIX);
    let hrp = "cosmos";
    let mut suite = TerpAccountSuite::new(mock.clone());
    suite.default_setup(mock.clone(), None, None)?;
    let minter = suite.manifold.address()?;
    let notminter = mock.addr_make("not-minter");
    let sender = mock.sender.clone();

    // create non 'terp1...' addrs
    let mut carbs = vec![];
    for _ in 0..20 {
        // creeate new key
        let secret_key: ecdsa::SigningKey<k256::Secp256k1> = SigningKey::random(&mut OsRng); // Serialize with `::to_bytes()`
        let public_key: ecdsa::VerifyingKey<k256::Secp256k1> = VerifyingKey::from(&secret_key); // Serialize with `::to_encoded_point()`
        let base64terpaddr = &Binary::new(sender.as_bytes().to_vec()).to_base64();
        let hraddr = pubkey_to_address(public_key.to_encoded_point(false).as_bytes(), "cosmos")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let adr036msgtohash = preamble_msg_arb_036(&hraddr.to_string(), base64terpaddr);
        let msg_digest = Sha256::new().chain(&adr036msgtohash);
        let msg_hash = msg_digest.clone().finalize();

        let signature: Signature = secret_key.sign_prehash_recoverable(&msg_hash).unwrap().0;

        assert!(
            cosmwasm_crypto::secp256k1_verify(
                &msg_hash,
                signature.to_bytes().as_slice(),
                public_key.to_encoded_point(false).as_bytes()
            )
            .unwrap()
        );

        let cosmosarb = CosmosArbitrary {
            pubkey: Binary::from(public_key.to_encoded_point(false).as_bytes()),
            signature: Binary::from(signature.to_bytes().as_slice()),
            message: Binary::from(sender.as_bytes().to_vec()), // set the value to be base64 (verify_return_readable handles base64 automatically)
            hrp: Some(hrp.to_string()),
        };
        cosmosarb
            .verify_return_readable(&mock_dependencies().api)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // add object,including private key
        carbs.push(TestCosmosArb {
            carb: cosmosarb,
            pk: Binary::new(secret_key.to_bytes().to_vec()),
        });
        println!("hraddr: {:#?}", hraddr);
        println!("sender: {:#?}", sender);
        println!("base64terpaddr: {:#?}", base64terpaddr);
        println!("adr036msgtohash: {:#?}", adr036msgtohash.to_string());
        println!("msg_hash:  {:#?}", Binary::new(msg_hash.to_vec()));
        println!("signature:  {:#?}", Binary::new(signature.to_vec()));
    }

    let fifth_carb = carbs[5].clone();

    // mint tokens for mock.sender.clone()
    let token_id = "Enterprise";
    suite.nft.call_as(&minter).mint(
        terp_account::Metadata::default(),
        mock.sender.clone(),
        token_id,
        None,
    )?;

    // try to set more than limit of associated addresses in one go
    let err = suite
        .nft
        .call_as(&mock.sender.clone())
        .update_my_reverse_map_key(carbs.iter().map(|c| c.carb.clone()).collect(), vec![])
        .unwrap_err();

    assert!(
        err.to_string()
            .contains(&ContractError::TooManyReverseMaps { max: 10, have: 20 }.to_string()),
        "Unexpected error: {err}"
    );

    // not owner tries to update reverse map
    for i in 0..11 {
        let err = suite
            .nft
            .call_as(&minter)
            .update_my_reverse_map_key(vec![carbs[i as usize].clone().carb], vec![])
            .unwrap_err();
        assert!(
            err.to_string()
                .contains(&ContractError::AccountNotFound {}.to_string())
        )
    }
    // owner tries to set more than limit of associated addresses in recursion
    for i in 0..10 {
        let res = suite
            .nft
            .update_my_reverse_map_key(vec![carbs[i as usize].clone().carb], vec![]);
        if i == 10 {
            assert!(res.is_err());
            assert!(
                res.unwrap_err()
                    .to_string()
                    .contains(&ContractError::TooManyReverseMaps { max: 10, have: 11 }.to_string())
            )
        } else {
            println!("i: {:#?}", i);
            println!("res: {:#?}", res);
            assert!(res.is_ok())
        }
    }

    // let nfts = suite.nft.owner_of(token_id, None)?;

    // associate address with token owner
    suite
        .nft
        .associate_address(token_id, Some(mock.sender.to_string()))?;
    // confirm we can query the non cosmos addr token_id associated to it
    let res = suite
        .nft
        .reverse_map_account(
            &pubkey_to_address(
                &fifth_carb.carb.pubkey,
                fifth_carb.carb.hrp.as_ref().expect("hrp must be set"),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        )
        .unwrap();
    assert_eq!(token_id, res);

    // confirm we have maps set
    for _ in 0..10 {
        // query the terp address for a given external address
        let res = suite
            .nft
            .reverse_map_address(
                terp_account::verify_generic::pubkey_to_address(
                    &fifth_carb.carb.pubkey,
                    fifth_carb.carb.hrp.as_ref().expect("hrp must be set"),
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .to_string(),
            )
            .unwrap();
        assert_eq!(mock.sender, res);
    }

    //try to remove more than existing at once
    let err = suite
        .nft
        .update_my_reverse_map_key(
            vec![],
            carbs.iter().map(|c| c.carb.pubkey.to_string()).collect(),
        )
        .unwrap_err();

    assert!(
        err.to_string()
            .contains(&ContractError::CannotRemoveMoreThanWillExists {}.to_string())
    );

    // try to remove more than exists via recusion

    // confirm only owner of token can remove tokens from map
    let err = suite
        .nft
        .call_as(&notminter)
        .update_my_reverse_map_key(vec![], vec![fifth_carb.carb.pubkey.to_string()])
        .unwrap_err();
    assert!(
        err.to_string()
            .contains(&ContractError::AccountNotFound {}.to_string())
    );

    // owner tries to set more than limit of associated addresses in recursion
    for i in 0..10 {
        let res = suite
            .nft
            .update_my_reverse_map_key(vec![], vec![carbs[i as usize].carb.pubkey.to_string()]);
        if i == 10 {
            assert!(res.is_err());
            assert!(
                res.unwrap_err()
                    .to_string()
                    .contains(&ContractError::TooManyReverseMaps { max: 10, have: 0 }.to_string())
            )
        } else {
            println!("i: {:#?}", i);
            println!("res: {:#?}", res);
            assert!(res.is_ok())
        }
    }

    Ok(())
}

#[test]
fn test_transcode() -> anyhow::Result<()> {
    let mut deps = mock_dependencies();

    let cosmos1 = deps.api.addr_make("cosmos");
    let terp1 = deps.api.addr_make("terp");
    let res = transcode(
        deps.as_ref(),
        "cosmos1y54exmx84cqtasvjnskf9f63djuuj68p7hqf47",
    );
    assert!(
        res.unwrap_err().to_string().contains("no mappping set. Set a non `terp1...` addr mapped to your`terp1..` that owns this account token with UpdateMyReverseMapKey")
    );

    //
    let canon = deps.api.addr_canonicalize(terp1.as_ref()).unwrap();

    // save to store
    REVERSE_MAP_KEY
        .save(
            &mut deps.storage,
            &cosmos1.to_string(),
            &Binary::new(canon.to_vec()),
        )
        .unwrap();

    let res = transcode(deps.as_ref(), cosmos1.as_ref()).unwrap();
    assert_eq!(terp1.to_string(), res);
    Ok(())
}

#[test]
fn test_nft_new() {
    let deps = mock_dependencies();
    NFT::new(
        deps.as_ref(),
        "solana1y54exmx84cqtasvjnskf9f63djuuj68pcrqfm3".into(),
        "1".into(),
    )
    .unwrap_err();
    let nft = NFT::new(
        deps.as_ref(),
        "cosmwasm1y54exmx84cqtasvjnskf9f63djuuj68pv3cnl5".into(),
        "1".into(),
    )
    .unwrap();
    assert_eq!(
        nft.collection.as_str(),
        "cosmwasm1y54exmx84cqtasvjnskf9f63djuuj68pv3cnl5"
    );
    assert_eq!(nft.token_id, "1");
    assert!(NFT::new(deps.as_ref(), "".into(), "1".into()).is_err());
}

#[test]
fn test_metadata() {
    // Test 1: default() sets account_ownership to false
    let default_metadata = Metadata::default();
    assert!(!default_metadata.account_ownership);
    assert_eq!(default_metadata.image_nft, None);
    assert_eq!(default_metadata.records, vec![]);

    // Test 2: default_with_account() enables account_ownership
    let account_metadata = Metadata::default_with_account();
    assert!(account_metadata.account_ownership);
    assert_eq!(account_metadata.image_nft, None);
    assert_eq!(account_metadata.records, vec![]);

    // Test 3: construct custom metadata with values
    let custom_metadata = Metadata {
        account_ownership: true,
        image_nft: Some(NFT {
            token_id: "1".to_string(),
            collection: Addr::unchecked("contract123"),
        }),
        records: vec![TextRecord {
            account: "website".to_string(),
            value: "terp.io".to_string(),
            verified: None,
        }],
    };

    // Test 4: into_json_string produces valid JSON
    let json_str = custom_metadata.into_json_string().unwrap();
    let expected_json = r#"{"account_ownership":true,"image_nft":{"token_id":"1","collection":"contract123"},"records":[{"account":"website","value":"terp.io","verified": null}]}"#;

    // Parse both to ensure structural equality (avoid whitespace issues)
    let parsed_output: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let parsed_expected: serde_json::Value = serde_json::from_str(expected_json).unwrap();

    assert_eq!(parsed_output, parsed_expected);

    // Test 5: round-trip serde test (optional but strong)
    let deserialized: Metadata = cosmwasm_std::from_json(&json_str).unwrap();
    assert!(deserialized.account_ownership);
    assert_eq!(deserialized.image_nft.as_ref().unwrap().token_id, "1");
    assert_eq!(deserialized.records[0].account, "website");
}

use cw_orch::mock::cw_multi_test::{SudoMsg, WasmSudo};
use terp_account::manifold::Config;
use terp_orch::suites::accounts::{
    Bs721AccountMarketExecuteMsgTypes, TerpAccountMarketExecuteFns, TerpAccountMarketQueryFns,
};

use cosmwasm_std::{Attribute, Decimal, Event, Uint128, Uint256, coin, coins, to_json_binary};
use terp_account::{Ask, Bid, DEPLOYMENT_DAO, PendingBid, manifold::ExecuteMsg};
use terp721_account_manifold::ContractError as MinterContractError;

const BID_AMOUNT: u128 = 1_000_000_000;
#[test]
pub fn init_market() -> anyhow::Result<()> {
    // new mock Bech32 chain environment
    let mock = MockBech32::new("mock");
    // simulate deploying the test suite to the mock chain env.
    let suite =
        TerpAccountSuite::deploy_on(mock.clone(), TerpAccountDeployData { admin: mock.sender })?;

    assert_eq!(
        suite.manifold.config()?,
        Config {
            // minter: suite.manifold.address()?,
            // collection: suite.nft.addrwqess()?,
            public_mint_start_time: mock.app.borrow().block_info().time.plus_seconds(1)
        }
    );

    Ok(())
}

// mod hooks {
//     use abstract_interface::{AccountQueryFns, RegistryExecFns, RegistryQueryFns};
//     use abstract_std::objects::namespace::Namespace;
//     use abstract_std::objects::AccountId;
//     use abstract_std::REGISTRY;

//     use super::*;

//     #[test]
//     fn test_manage_sale_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("salehook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddSaleHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.sale_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveSaleHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.sale_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_manage_bid_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("bidhook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddBidHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.bid_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveBidHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.bid_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_manage_ask_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("askhook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddAskHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.ask_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveAskHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.ask_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_all_hooks_workflow() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let mut suite = TerpAccountSuite::new(mock.clone());
//         suite.default_setup(mock.clone(), None, None)?;
//         mock.wait_seconds(200)?;
//         let mw_addr = suite.middleware.addr_str()?;
//         let account1 = "rick";

//         // // register hooks to middleware
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddAskHook(mw_addr.clone()))?;
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddBidHook(mw_addr.clone()))?;
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddSaleHook(mw_addr.clone()))?;

//         // // perform actions for hooks
//         // let res = suite.mint_and_list(mock.clone(), account1, &mock.sender_addr())?;
//         // let res = suite.account.instantiate(
//         //     &abstract_std::account::InstantiateMsg::<Empty> {
//         //         code_id: suite.account.code_id()?,
//         //         owner: Some(abstract_std::objects::gov_type::GovernanceDetails::NFT {
//         //             collection_addr: suite.nft.addr_str()?,
//         //             token_id: account1.to_string(),
//         //         }),
//         //         account_id: None,
//         //         authenticator: None,
//         //         namespace: Some(account1.to_string()),
//         //         install_modules: vec![], // TODO: install USB
//         //         name: Some(account1.to_string()),
//         //         description: Some("Powered By Terp Account Framework".into()),
//         //         link: None,
//         //     },
//         //     None,
//         //     &[],
//         // )?;

//         // let res = suite.registry.namespace_list(None, None)?.namespaces;
//         // println!("{:#?}", res);

//         // let res = suite.account.info()?;
//         // println!("{:#?}", res);
//         // let res = suite.registry.namespace_list(None, None)?;
//         // let res = suite
//         //     .registry
//         //     .namespace(Namespace::new(&account1.to_string())?)?;
//         // println!("{:#?}", res);

//         // res.assert_event(&Event::new("wasm-abstract").add_attributes(vec![
//         //         Attribute {
//         //             key: "_contract_address".into(),
//         //             value: suite.middleware.addr_str()?,
//         //         },
//         //         Attribute::new("contract", REGISTRY),
//         //         Attribute::new("action", "claim_namespace"),
//         //         Attribute::new(
//         //             "account_id",
//         //             &suite
//         //                 .registry
//         //                 .namespace(Namespace::new(account1)?)?
//         //                 .unwrap()
//         //                 .account_id
//         //                 .to_string(),
//         //         ),
//         //         Attribute::new("namespace", "rick"),
//         //     ]));

//         // assert hook logic is performed
//         Ok(())
//     }
// }

mod execute {

    use terp721_account_manifold::state::MAX_FEE_BPS;

    use super::*;

    #[test]
    fn test_check_approvals() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;

        let owner = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        mock.wait_seconds(6)?;

        let ops = suite.nft.operator(suite.manifold.address()?, owner, None)?;
        println!("{:#?}", ops);
        // check operators

        Ok(())
    }
    #[test]
    fn test_mint() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;

        //  must set approval for

        // check if account is listed in marketplace
        let res = suite.manifold.ask(token_id.to_string())?.unwrap();
        assert_eq!(res.token_id, token_id);

        // check if token minted
        let res = suite.nft.num_tokens()?;
        assert_eq!(res.count, 1);

        assert_eq!(suite.owner_of(token_id.into())?, owner.to_string());

        Ok(())
    }
    #[test]
    fn test_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let token_id = "bandura";
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        suite.bid_w_funds(mock, token_id, bidder, BID_AMOUNT)?;
        Ok(())
    }

    #[test]
    fn test_accept_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let token_id = "bandura";

        mock.wait_seconds(200)?;

        // ensure operator approval is set
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        let balance = mock.query_balance(&bidder, "uthiol")?;
        // assert overwritten bid returns funds from first bid
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT * 2)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
        let balance2 = mock.query_balance(&bidder, "uthiol")?;

        assert_eq!(balance, Uint256::zero());
        assert_eq!(balance2, Uint256::from(BID_AMOUNT * 2));

        // bid smaller than minimum
        suite
            .bid_w_funds(mock.clone(), token_id, bidder.clone(), 99u128)
            .unwrap_err();

        // ask only removed by minter
        assert!(
            suite
                .manifold
                .remove_ask(token_id.to_string())
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::Unauthorized {}.to_string())
        );

        assert!(
            suite
                .manifold
                .call_as(&suite.nft.address()?)
                .remove_ask(token_id.to_string())
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::ExistingBids {}.to_string())
        );

        // user (owner) starts off with 0 internet funny money
        assert_eq!(
            mock.balance(&owner.clone(), Some("uthiol".into()))?[0].amount,
            Uint256::zero()
        );

        assert!(
            suite
                .manifold
                .update_ask(bidder.to_string(), token_id.to_string(),)
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::Unauthorized {}.to_string())
        );
        assert_eq!(
            suite.manifold.bids(token_id.to_string(), None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );
        assert_eq!(
            suite.manifold.bids_sorted_by_price(None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );

        assert_eq!(
            suite.manifold.reverse_bids_sorted_by_price(None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );

        assert!(
            suite
                .manifold
                .call_as(&bidder)
                .accept_bid(bidder.clone(), token_id.into())
                .unwrap_err()
                .to_string()
                .contains("UnauthorizedOwner")
        );
        suite.manifold.accept_bid(bidder.clone(), token_id.into())?;

        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(token_id.to_string())?;

        // check if bid is removed
        assert!(
            suite
                .manifold
                .bid(bidder.to_string(), token_id.into())?
                .is_none()
        );
        // verify that the bidder is the new owner
        assert_eq!(suite.owner_of(token_id.into())?, bidder.to_string());
        // check if user got the bid amount
        assert_eq!(
            mock.balance(&owner, Some("uthiol".into()))?[0].amount,
            Uint256::from(BID_AMOUNT)
        );
        // confirm that a new ask was created
        let res = suite.manifold.ask(token_id.to_string())?.unwrap();
        assert_eq!(res.seller, bidder);
        assert_eq!(res.token_id, token_id);

        // remove ask
        let res = suite.nft.call_as(&bidder).burn(token_id.to_string())?;
        println!("res: {:#?}", res);
        res.assert_event(&Event::new("wasm-burn-account").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.nft.addr_str()?,
            },
            Attribute::new("account", token_id.to_string()),
        ]));
        res.assert_event(&Event::new("wasm-remove-ask").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.manifold.addr_str()?,
            },
            Attribute::new("token_id", token_id.to_string()),
        ]));

        assert_eq!(suite.manifold.ask(token_id.to_string())?, None);

        Ok(())
    }
    #[test]

    fn test_two_sales_cycles() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";
        let market = suite.manifold.address()?;
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
        suite.manifold.accept_bid(bidder.clone(), token_id.into())?;
        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(token_id.to_string())?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT)?;
        suite.nft.call_as(&bidder).approve(market, token_id, None)?;
        suite
            .manifold
            .call_as(&bidder)
            .accept_bid(bidder2.clone(), token_id.into())?;
        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&bidder)
            .finalize_bid(token_id.to_string())?;

        Ok(())
    }
    #[test]
    fn test_reverse_map() -> anyhow::Result<()> {
        let token_id = "bandura";

        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        mock.wait_seconds(200)?;

        let admin2 = mock.addr_make("admin2");
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // when no associated address, query responds with token owner
        assert_eq!(
            suite.nft.call_as(&admin2).associated_address(token_id)?,
            admin2.clone()
        );

        // associate owner address with account account
        suite
            .nft
            .call_as(&admin2)
            .associate_address(token_id, Some(admin2.to_string()))?;

        // query associated address should return user
        assert_eq!(suite.nft.associated_address(token_id)?, admin2);

        // added to get around rate limiting
        mock.wait_seconds(60)?;
        // associate another
        let account2 = "exam";
        suite.mint_and_list(mock.clone(), account2, &admin2.clone())?;

        suite
            .nft
            .call_as(&admin2)
            .associate_address(account2, Some(admin2.to_string()))?;

        assert_eq!(suite.nft.account(admin2)?, account2.to_string());
        Ok(())
    }

    #[test]
    fn test_reverse_map_contract_address() -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn test_reverse_map_not_contract_address_admin() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;

        let not_admin = mock.addr_make_with_balance("not-admin", coins(1000000000, "uthiol"))?;
        mock.add_balance(&not_admin, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), not_admin.clone(), 10000000000u128)?;

        let minter = suite.manifold.address()?;
        let token_id = "bandura";
        println!("not-admin: {:#?}", not_admin);
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &not_admin)?;

        suite
            .nft
            .associate_address(token_id, Some(minter.to_string()))
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_reverse_map_not_owner() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        suite
            .nft
            .associate_address(token_id, Some(admin2.to_string()))
            .unwrap_err();
        Ok(())
    }
    #[test]
    fn test_pause() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // pause minting
        suite.manifold.pause(true)?;

        // error trying to mint
        mock.wait_seconds(200)?;
        suite
            .mint_and_list(mock.clone(), token_id, &admin2)
            .unwrap_err();

        Ok(())
    }

    #[test]
    fn test_update_mkt_sudo() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // run sudo msg
        mock.app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.manifold.address()?,
                message: to_json_binary(&terp_account::manifold::SudoMsg::UpdateParams {
                    trading_fee_bps: Some(1000u64),
                    min_price: Some(Uint128::from(1000u128)),
                    ask_interval: Some(1000),
                    cooldown_duration: Some(69),
                    cooldown_cancel_fee: Some(coin(69u128, "jerets")),
                    min_account_length: None,
                    max_account_length: None,
                    base_price: None,
                    base_delegation: None,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let sudo_err = mock
            .app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.manifold.address()?,
                message: to_json_binary(&terp_account::manifold::SudoMsg::UpdateParams {
                    trading_fee_bps: Some(MAX_FEE_BPS * 2u64),
                    min_price: Some(Uint128::from(1000u128)),
                    ask_interval: Some(1000),
                    cooldown_duration: Some(69),
                    cooldown_cancel_fee: Some(coin(69u128, "jerets")),
                    min_account_length: None,
                    max_account_length: None,
                    base_price: None,
                    base_delegation: None,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .unwrap_err()
            .to_string();
        assert!(
            sudo_err.contains(
                &MinterContractError::InvalidTradingFeeBps(MAX_FEE_BPS * 2u64).to_string()
            ),
            "Expected error to contain InvalidTradingFeeBps, got: {sudo_err}"
        );

        // confirm updated params
        let res = suite.manifold.params()?;
        assert_eq!(res.trading_fee_percent, Decimal::percent(10));
        assert_eq!(res.min_price, Uint128::from(1000u128));
        assert_eq!(res.ask_interval, 1000);
        assert_eq!(res.cooldown_duration, 69);
        assert_eq!(res.cooldown_fee, coin(69u128, "jerets"));

        let _new = mock.addr_make("new-jawn");
        let _newnew = mock.addr_make("newer-jawn");

        Ok(())
    }

    #[test]
    fn test_cooldown_period_operator_approve() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;
        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        suite.nft.revoke_all(suite.manifold.address()?)?;
        mock.wait_seconds(60)?;
        let res = suite.manifold.finalize_bid(account.to_string())?;
        res.assert_event(&Event::new("wasm").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.nft.addr_str()?,
            },
            Attribute::new("action", "approve_all".to_string()),
            Attribute::new("operator", suite.manifold.addr_str()?),
        ]));
        assert_eq!(
            suite.manifold.ask(account.to_string())?.unwrap().seller,
            &bidder
        );
        Ok(())
    }

    #[test]
    fn test_cooldown_period() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;

        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        let owner_balance_a = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_a = mock.query_balance(&bidder, "uthiol")?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        // no funds are transfered before cooldown is over
        let owner_balance_b = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_b = mock.query_balance(&bidder, "uthiol")?;
        mock.wait_seconds(10)?;
        assert_eq!(owner_balance_a, owner_balance_b);
        assert_eq!(bidder_balance_a, bidder_balance_b);
        assert_eq!(
            suite.manifold.cooldown(account.to_string())?,
            Some(PendingBid {
                ask: Ask {
                    token_id: account.to_string(),
                    id: 1,
                    seller: owner.clone(),
                },
                new_owner: bidder.clone(),
                amount: BID_AMOUNT.into(),
                unlock_time: mock.block_info()?.time.plus_seconds(50)
            })
        );
        assert!(
            suite.nft.burn(account).unwrap_err().to_string().contains(
                &terp721_account::ContractError::AccountCannotBeTransfered {
                    reason: "Account is in cooldown".to_string()
                }
                .to_string()
            )
        );
        assert!(
            suite
                .nft
                .transfer_nft(bidder.clone(), account)
                .unwrap_err()
                .to_string()
                .contains(
                    &terp721_account::ContractError::AccountCannotBeTransfered {
                        reason: "Account is in cooldown".to_string()
                    }
                    .to_string()
                )
        );
        assert!(
            suite
                .nft
                .send_nft(suite.manifold.address()?, Binary::default(), account)
                .unwrap_err()
                .to_string()
                .contains(
                    &terp721_account::ContractError::AccountCannotBeTransfered {
                        reason: "Account is in cooldown".to_string()
                    }
                    .to_string()
                )
        );

        // cannot finalize until cooldown period is over

        assert!(
            mock.block_info()?.time.seconds().lt(&suite
                .manifold
                .cooldown(account.to_string())?
                .unwrap()
                .unlock_time
                .seconds())
        );
        assert!(
            suite
                .manifold
                .finalize_bid(account.to_string())
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::InvalidDuration {}.to_string())
        );

        mock.wait_seconds(50)?;
        // cannot cancel after cooldown completes
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::CancelCooldown {
                        token_id: account.to_string(),
                    },
                    &[coin(500_000_000, "uthiol")],
                )
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::InvalidDuration {}.to_string())
        );
        // token id doesnt exists
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::CancelCooldown {
                        token_id: "babber".to_string(),
                    },
                    &[coin(500_000_000, "uthiol")],
                )
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::AskNotFound {}.to_string())
        );
        // token id doesnt exists
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::FinalizeBid {
                        token_id: "babber".to_string(),
                    },
                    &[coin(500_000_000, "uthiol")],
                )
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::AskNotFound {}.to_string())
        );

        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(account.to_string())?;
        assert_eq!(suite.manifold.cooldown(account.to_string())?, None,);
        let owner_balance_c = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_c = mock.query_balance(&bidder, "uthiol")?;
        assert_eq!(owner_balance_c, owner_balance_b + Uint256::from(BID_AMOUNT));
        assert_eq!(bidder_balance_b, bidder_balance_c);
        Ok(())
    }

    #[test]
    fn test_cancel_cooldown_period() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;
        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        let owner_balance_a = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_a = mock.query_balance(&bidder, "uthiol")?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        // no funds are transfered before cooldown is over
        let owner_balance_b = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_b = mock.query_balance(&bidder, "uthiol")?;
        mock.wait_seconds(10)?;
        assert_eq!(owner_balance_a, owner_balance_b);
        assert_eq!(bidder_balance_a, bidder_balance_b);
        assert_eq!(
            suite.manifold.cooldown(account.to_string())?,
            Some(PendingBid {
                ask: Ask {
                    token_id: account.to_string(),
                    id: 1,
                    seller: owner.clone(),
                },
                new_owner: bidder.clone(),
                amount: BID_AMOUNT.into(),
                unlock_time: mock.block_info()?.time.plus_seconds(50)
            })
        );
        mock.wait_seconds(49)?;
        assert!(
            suite
                .manifold
                .call_as(&bidder)
                .cancel_cooldown(account.to_string())
                .unwrap_err()
                .to_string()
                .contains(&MinterContractError::Unauthorized {}.to_string())
        );
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::CancelCooldown {
                        token_id: account.to_string()
                    },
                    &[]
                )
                .unwrap_err()
                .to_string()
                .contains("No funds sent")
        );
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::CancelCooldown {
                        token_id: account.to_string()
                    },
                    &[coin(499_000_000, "uthiol")]
                )
                .unwrap_err()
                .to_string()
                .contains(
                    &MinterContractError::IncorrectPayment {
                        got: 499_000_000u128.into(),
                        expected: 500_000_000u128.into()
                    }
                    .to_string()
                )
        );
        assert!(
            suite
                .manifold
                .execute(
                    &ExecuteMsg::CancelCooldown {
                        token_id: account.to_string()
                    },
                    &[coin(499_000_000, "uthiol")]
                )
                .unwrap_err()
                .to_string()
                .contains("Incorrect payment, got: 499000000, expected 500000000")
        );
        suite.manifold.execute(
            &ExecuteMsg::CancelCooldown {
                token_id: account.to_string(),
            },
            &[coin(500_000_000, "uthiol")],
        )?;
        let dd_balance = mock.query_balance(&Addr::unchecked(DEPLOYMENT_DAO), "uthiol")?;
        let bidder_balance_c = mock.query_balance(&bidder, "uthiol")?;
        assert_eq!(
            bidder_balance_c,
            Uint256::from(BID_AMOUNT + 250_000_000u128)
        );
        assert_eq!(dd_balance, Uint256::from(250_000_000u128));
        Ok(())
        // assert_eq!(suite.manifold.cooldown(account.to_string())?, None,);

        // assert_eq!(owner_balance_b, bidder_balance_c);
        // assert_ne!(bidder_balance_b, owner_balance_c);
    }

    #[test]
    fn test_mint_with_delegation_tiers() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;
        let user = mock.addr_make("user");
        mock.add_balance(&user, vec![coin(10000000000u128, "uthiol")])?;

        let id_3 = "rep";
        let id_4 = "repe";
        let id_5 = "repea";
        let id_6 = "repeat";
        let base_delegation = Uint128::new(2100_000_000); // assuming this is the base delegation amount

        // Test with account length 3
        let user3 = mock.addr_make("user3");
        mock.add_balance(&user3, vec![coin(10500000000u128, "uthiol")])?;
        suite.delegate_to_val(
            mock.clone(),
            user3.clone(),
            (base_delegation * Uint128::new(5u128)).into(),
        )?;
        suite.mint_and_list(mock.clone(), id_3, &user3)?;

        // Test with account length 4
        let user4 = mock.addr_make("user4abcd");
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(
            mock.clone(),
            user4.clone(),
            (base_delegation * Uint128::new(3u128)).into(),
        )?;
        suite.mint_and_list(mock.clone(), id_4, &user4)?;

        // Test with account length 5 or more
        let user5 = mock.addr_make("user5abcde");
        mock.add_balance(&user5, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user5.clone(), base_delegation.u128())?;
        suite.mint_and_list(mock.clone(), id_5, &user5)?;

        // Test with insufficient delegation for account length 3
        let user3_insufficient = mock.addr_make("user3_insufficient");
        mock.add_balance(&user3_insufficient, vec![coin(10000440u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user3_insufficient.clone(), 10000440u128)?;
        assert!(
            suite
                .mint_and_list(mock.clone(), id_6, &user3_insufficient)
                .unwrap_err()
                .to_string()
                .contains(
                    &MinterContractError::IncorrectDelegation {
                        got: 10000440u128.into(),
                        expected: base_delegation.u128()
                    }
                    .to_string()
                )
        );

        Ok(())
    }
}
mod admin {
    use super::*;

    #[test]
    fn test_update_admin() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin2 = mock.addr_make("admin2");
        // non-admin tries to set admin to None
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::RenounceOwnership)
            .unwrap_err();
        // admin updates admin
        suite
            .manifold
            .update_ownership(cw_ownable::Action::TransferOwnership {
                new_owner: admin2.to_string(),
                expiry: None,
            })?;
        // new admin updates to have no admin
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::AcceptOwnership)?;
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::RenounceOwnership)?;
        // cannot update without admin
        suite
            .manifold
            .update_ownership(cw_ownable::Action::TransferOwnership {
                new_owner: admin2.to_string(),
                expiry: None,
            })
            .unwrap_err();
        Ok(())
    }
}
mod query {
    use cosmwasm_std::{Attribute, Event, coin};
    use terp_account::{Bid, manifold::BidOffset};

    use super::*;

    #[test]
    fn test_query_ask() -> anyhow::Result<()> {
        let token_id = "bandura";
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        assert_eq!(
            suite.manifold.ask(token_id.into())?.unwrap().token_id,
            token_id
        );
        Ok(())
    }
    #[test]
    fn test_query_asks() -> anyhow::Result<()> {
        let token_id = "bandura";
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(suite.manifold.asks(None, None)?[0].id, 1);

        Ok(())
    }
    #[test]
    fn test_query_asks_by_seller() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(
            suite
                .manifold
                .asks_by_seller(admin.to_string(), None, None)?
                .len(),
            1
        );
        Ok(())
    }
    #[test]
    fn test_query_ask_count() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(suite.manifold.ask_count()?, 2);
        Ok(())
    }
    #[test]
    fn test_query_top_bids() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        let res = suite.manifold.bids_for_seller(admin.clone(), None, None)?;
        assert_eq!(res.len(), 2);
        assert_eq!(res[1].amount.u128(), BID_AMOUNT);

        // test pagination
        let filter = BidOffset {
            price: Uint128::from(BID_AMOUNT),
            token_id: token_id.into(),
            bidder: bidder1.clone(),
        };
        let res: Vec<Bid> =
            suite
                .manifold
                .bids_for_seller(admin.clone(), None, Some(filter.clone()))?;

        // should be length 0 because there are no token_ids besides NAME.to_string()
        assert_eq!(res.len(), 0);

        // added to get around rate limiting
        mock.wait_seconds(60)?;

        // test pagination with multiple accounts and bids
        let account: &str = "jump";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), account, &admin)?;

        suite.bid_w_funds(mock.clone(), account, bidder1.clone(), BID_AMOUNT * 3)?;
        suite.bid_w_funds(mock.clone(), account, bidder2, BID_AMOUNT * 2)?;
        let res = suite
            .manifold
            .bids_for_seller(admin.clone(), None, Some(filter.clone()))?;
        // should be length 2 because there is token_id "jump" with 2 bids
        assert_eq!(res.len(), 2);

        suite
            .manifold
            .call_as(&bidder1)
            .execute(
                &Bs721AccountMarketExecuteMsgTypes::RemoveBid {
                    token_id: account.to_string(),
                },
                &[coin(1, "uthiol")],
            )
            .unwrap_err();

        let res = suite
            .manifold
            .call_as(&bidder1)
            .remove_bid(account.to_string())?;

        println!("res: {:#?}", res);
        res.assert_event(&Event::new("transfer").add_attributes(vec![
            Attribute::new("recipient", bidder1.to_string()),
            Attribute::new("sender", suite.manifold.addr_str()?),
            Attribute::new("amount", coin(BID_AMOUNT * 3, "uthiol").to_string()),
        ]));
        res.assert_event(&Event::new("wasm-remove-bid").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.manifold.addr_str()?,
            },
            Attribute::new("token_id", account.to_string()),
            Attribute::new("bidder", bidder1.to_string()),
        ]));

        let res = suite.manifold.bids_for_seller(admin, None, Some(filter))?;
        assert_eq!(res.len(), 1);

        Ok(())
    }
    #[test]
    fn test_query_bids_by_seller() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        let res = suite.manifold.bids_for_seller(admin.clone(), None, None)?;
        assert_eq!(res.len(), 2);
        assert_eq!(res[1].amount.u128(), BID_AMOUNT);

        // test pagination
        let res = suite.manifold.bids_for_seller(
            admin.clone(),
            None,
            Some(BidOffset::new(
                Uint128::from(BID_AMOUNT),
                token_id.to_string(),
                bidder1.clone(),
            )),
        )?;
        assert_eq!(res.len(), 0);

        // added to get around rate limiting
        mock.wait_seconds(60)?;

        let account = "jump";
        suite.mint_and_list(mock.clone(), account, &admin)?;
        suite.bid_w_funds(mock.clone(), account, bidder1.clone(), BID_AMOUNT * 3)?;
        suite.bid_w_funds(mock.clone(), account, bidder2, BID_AMOUNT * 2)?;
        // should be length 2 because there is token_id "jump" with 2 bids
        let res = suite.manifold.bids_for_seller(
            admin.clone(),
            None,
            Some(BidOffset::new(
                Uint128::from(BID_AMOUNT),
                token_id.to_string(),
                bidder1.clone(),
            )),
        )?;
        assert_eq!(res.len(), 2);

        Ok(())
    }
    #[test]
    fn test_query_highest_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        assert_eq!(
            suite
                .manifold
                .highest_bid(token_id.to_string())?
                .unwrap()
                .amount
                .u128(),
            BID_AMOUNT * 5
        );

        Ok(())
    }
    #[test]
    fn test_query_account() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        // fails with "user" string, has to be a bech32 address
        suite.nft.account(token_id).unwrap_err();

        suite.mint_and_list(mock.clone(), "yoyo", &admin)?;

        suite
            .nft
            .associate_address("yoyo", Some(admin.to_string()))?;

        assert_eq!(suite.nft.account(admin)?, "yoyo".to_string());

        Ok(())
    }
    // #[test]
    // fn test_query_trading_start_time() -> anyhow::Result<()> {
    //     let mock = MockBech32::new(TERP_PREFIX);
    //     let mut suite = TerpAccountSuite::new(mock.clone());
    //     suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

    //     Ok(())
    // }
}
mod collection {
    use cosmwasm_std::coin;
    use terp_account::TextRecord;

    use super::*;
    #[test]
    fn test_verify_twitter() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let verifier = mock.addr_make("verifier");
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;
        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        suite
            .nft
            .verify_text_record(token_id, account, true)
            .unwrap_err();

        suite
            .nft
            .call_as(&verifier)
            .verify_text_record(token_id, account, true)?;

        // query text record to see if verified is set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, Some(true));

        Ok(())
    }
    #[test]
    fn test_verify_false() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let verifier = mock.addr_make("verifier");
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        suite
            .nft
            .call_as(&verifier)
            .verify_text_record(token_id, account, false)?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, Some(false));

        Ok(())
    }
    #[test]
    fn test_verified_text_record() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;
        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        // attempt update text record w verified value
        suite.nft.update_text_record(
            token_id,
            TextRecord {
                account: token_id.into(),
                value: "some new value".to_string(),
                verified: Some(true),
            },
        )?;

        // query text record to see if verified is set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        // query image nft
        assert_eq!(suite.nft.image_nft(token_id)?, None);
        Ok(())
    }
    #[test]
    fn test_transfer_nft() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite
            .nft
            .transfer_nft(mock.addr_make("new-addr"), token_id)?;

        Ok(())
    }
    #[test]
    fn test_send_nft() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite.nft.send_nft(
            mock.addr_make("new-addr"),
            to_json_binary("ini").map_err(|e| anyhow::anyhow!("{e}"))?,
            token_id,
        )?;
        Ok(())
    }
    #[test]
    fn test_transfer_nft_and_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let bidder1 = mock.addr_make("bidder1");
        let market = suite.manifold.address()?;

        let user1 = mock.addr_make("user1");
        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite.nft.transfer_nft(user1.clone(), token_id)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT * 3)?;

        // user2 must approve the marketplace to transfer their account
        suite.nft.call_as(&user1).approve(market, token_id, None)?;
        // accept bid
        suite
            .manifold
            .call_as(&user1)
            .accept_bid(bidder1, token_id.into())?;

        Ok(())
    }
    #[test]
    fn test_transfer_nft_with_reverse_map() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let user = mock.addr_make("user");
        let user2 = mock.addr_make("user2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&user, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &user)?;

        suite
            .nft
            .call_as(&user)
            .associate_address(token_id, Some(user.to_string()))?;

        assert_eq!(suite.nft.account(user.clone())?, token_id);

        suite
            .nft
            .call_as(&user)
            .transfer_nft(user2.clone(), token_id)?;

        suite.nft.account(user).unwrap_err();
        suite.nft.account(user2).unwrap_err();

        Ok(())
    }
    // #[test]
    // fn test_burn_nft() -> anyhow::Result<()> {
    //     Ok(())
    // }
    // #[test]
    // fn test_burn_with_existing_bids() -> anyhow::Result<()> {
    //     Ok(())
    // }
    // #[test]
    // fn test_burn_nft_with_reverse_map() -> anyhow::Result<()> {
    //     Ok(())
    // }
    #[test]
    fn test_sudo_update() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let max_record_count = suite.nft.params()?.max_record_count;
        let max_rev_key_count = suite.nft.params()?.max_reverse_map_key_limit;

        // run sudo msg
        mock.app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.nft.address()?,
                message: to_json_binary(&terp721_account::msg::SudoMsg::UpdateParams {
                    max_record_count: max_record_count + 1,
                    max_rev_map_count: max_rev_key_count + 4,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        assert_eq!(suite.nft.params()?.max_record_count, max_record_count + 1);
        assert_eq!(
            suite.nft.params()?.max_reverse_map_key_limit,
            max_record_count + 4
        );

        Ok(())
    }
}
mod public_start_time {

    use cosmwasm_std::coin;
    use terp_account::manifold::Config;

    use super::*;

    #[test]
    fn test_mint_before_start() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        suite
            .mint_and_list(mock.clone(), token_id, &admin_user)
            .unwrap_err();
        suite
            .mint_and_list(mock.clone(), token_id, &user4)
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_update_start_time() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let res = suite.manifold.config()?;
        assert_eq!(
            res.public_mint_start_time,
            mock.block_info()?.time.plus_seconds(200)
        );

        suite.manifold.update_config(Config {
            public_mint_start_time: mock.block_info()?.time.plus_seconds(2),
        })?;

        let res = suite.manifold.config()?;
        assert_eq!(
            res.public_mint_start_time,
            mock.block_info()?.time.plus_seconds(2)
        );

        Ok(())
    }
}

mod associate_address {

    use cosmwasm_std::coin;
    use terp721_account::msg::{InstantiateMsg, Terp721InstantiateMsg};

    use super::*;

    // #[test]
    // fn test_abstract_account_workflow() -> anyhow::Result<()> {
    //     let mock = MockBech32::new(TERP_PREFIX);
    //     let mut suite = TerpAccountSuite::new(mock.clone());
    //     suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

    //     let admin_user = mock.sender.clone();
    //     let token_id = "bandura";
    //     let bidder = mock.addr_make("bidder");

    //     mock.wait_seconds(200)?;
    //     suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

    //     // set nft as ownership
    //     suite.test_owner.instantiate(
    //         &TestOwnershipInitMsg {
    //             ownership: abstract_std::objects::gov_type::GovernanceDetails::NFT {
    //                 collection_addr: suite.nft.addr_str()?,
    //                 token_id: token_id.to_string(),
    //             },
    //         },
    //         None,
    //         &[],
    //     )?;

    //     // associate account to abstract account
    //     suite
    //         .nft
    //         .update_abs_acc_support(token_id, Some(suite.test_owner.addr_str()?))?;

    //     // query the associated address and ensure its the same as the abstract account
    //     assert_eq!(
    //         suite.nft.associated_address(token_id)?,
    //         suite.test_owner.address()?
    //     );

    //     // ensure if ownership is changed before cooldown
    //     suite
    //         .test_owner
    //         .update_ownership(abstract_std::objects::gov_type::GovernanceDetails::Renounced {})?;

    //     let owner_bal = mock.query_balance(&admin_user, "uthiol")?;
    //     let bidder_bal = mock.query_balance(&bidder, "uthiol")?;
    //     suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
    //     assert_eq!(bidder_bal, Uint128::zero());
    //     let _res = suite.manifold.accept_bid(bidder.clone(), token_id.into())?;
    //     // assert funds go back to bidder, along with tokens if owner changes ownership prior to finalizing bid
    //     mock.wait_seconds(60)?;
    //     let res = suite.manifold.finalize_bid(token_id.into())?;
    //     let owner_bal2 = mock.query_balance(&admin_user, "uthiol")?;
    //     let bidder_bal2 = mock.query_balance(&bidder, "uthiol")?;
    //     assert_eq!(BID_AMOUNT, bidder_bal2.u128());
    //     assert_eq!(owner_bal, owner_bal2);
    //     assert_eq!(
    //         suite.nft.owner_of(token_id, None)?.owner,
    //         bidder.to_string()
    //     );
    //     res.assert_event(&Event::new("transfer").add_attributes(vec![
    //         Attribute::new("recipient", bidder.to_string()),
    //         Attribute::new("sender", suite.manifold.addr_str()?),
    //         Attribute::new("amount", coin(BID_AMOUNT, "uthiol").to_string()),
    //     ]));

    //     Ok(())
    // }

    #[test]
    fn test_transfer_to_eoa() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();

        let cw721_id = suite.nft.code_id()?;
        let token_id = "bandura";

        let nft_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        collection_info_extension: None,
                        minter: Some(suite.manifold.address()?.to_string()),
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // mint and transfer to collection
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;
        suite.nft.transfer_nft(nft_addr.clone(), token_id)?;
        assert_eq!(
            suite.nft.owner_of(token_id, None)?.owner,
            nft_addr.to_string()
        );

        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_no_admin() -> anyhow::Result<()> {
        // For the purposes of this test, a collection contract with no admin needs to be instantiated (contract_with_no_admin)
        // This contract needs to have a creator that is itself a contract and this creator contract should have an admin (USER).
        // The admin (USER) of the creator contract will mint a account and associate the account with the collection contract that doesn't have an admin successfully.
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let creator_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        collection_info_extension: None,
                        minter: Some(suite.manifold.address()?.to_string()),
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // The creator contract instantiates the collection contract with no admin
        let collection_with_no_admin_addr = mock
            .call_as(&creator_addr)
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                None,
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // USER4 mints a account
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        // USER4 tries to associate the account with the collection contract that doesn't have an admin
        suite
            .nft
            .call_as(&admin_user)
            .associate_address(token_id, Some(collection_with_no_admin_addr.to_string()))?;

        mock.wait_seconds(200)?;
        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_no_admin_fail() -> anyhow::Result<()> {
        // For the purposes of this test, a collection contract with no admin needs to be instantiated (contract_with_no_admin)
        // This contract needs to have a creator that is itself a contract and this creator contract should have an admin (USER).
        // An address other than the admin (USER) of the creator contract will mint a account, try to associate the account with the collection contract that doesn't have an admin and fail.
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.addr_make("admin-user");
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let creator_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,

                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // The creator contract instantiates the collection contract with no admin
        let collection_with_no_admin_addr = mock
            .call_as(&creator_addr)
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),

                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                None,
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // USER4 mints a account
        suite.mint_and_list(mock.clone(), token_id, &user4)?;

        // USER4 tries to associate the account with the collection contract that doesn't have an admin
        let err = suite
            .nft
            .call_as(&user4)
            .associate_address(token_id, Some(collection_with_no_admin_addr.to_string()))
            .unwrap_err();

        assert!(
            err.to_string().contains(
                &terp721_account::ContractError::UnauthorizedCreatorOrAdmin {}.to_string()
            ),
            "Unexpected error: {err}"
        );
        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_an_admin_fail() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.addr_make("admin-user");
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let contract = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),

                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &user4)?;

        let err = suite
            .nft
            .call_as(&user4)
            .associate_address(token_id, Some(contract.to_string()))
            .unwrap_err();

        assert!(
            err.to_string().contains(
                &terp721_account::ContractError::UnauthorizedCreatorOrAdmin {}.to_string()
            ),
            "Unexpected error: {err}"
        );
        Ok(())
    }
}
