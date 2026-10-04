use crate::suite::TerpAccountDeployData;
use crate::TerpAccountExecuteFns;
use ::terp721_account::{commands::transcode, ContractError};
use cosmwasm_std::testing::mock_dependencies;
use cosmwasm_std::{from_json, Api, Binary};
use cw_orch::prelude::CallAs;
use cw_orch::{anyhow, mock::MockBech32, prelude::*};

use terp721_account::msg::Terp721AccountsQueryMsgFns;
use terp721_account::state::REVERSE_MAP_KEY;
use terp_account::verify_generic::{
    preamble_msg_arb_036, pubkey_to_address, CosmosArbitrary, TestCosmosArb,
};
use terp_account::{Metadata, TextRecord, NFT, TERP_PREFIX};

use crate::TerpAccountSuite;
use ecdsa::signature::rand_core::OsRng;
use k256::ecdsa::{Signature, SigningKey, VerifyingKey};
// use serde::Deserialize;
use sha2::digest::Update;
use sha2::Digest;
use sha2::Sha256;

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
            assert!(res.unwrap_err().to_string().contains(
                &ContractError::TooManyRecords {
                    max: max_record_count
                }
                .to_string()
            ));
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
    assert!(err.to_string().contains(
        &ContractError::Cw721(cw721::error::Cw721ContractError::Ownership(
            cw_ownable::OwnershipError::NotOwner
        ))
        .to_string()
    ));
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
    assert!(suite
        .nft
        .add_text_record(token_id, record.clone())
        .unwrap_err()
        .to_string()
        .contains(&ContractError::RecordAccountAlreadyExists {}.to_string()));
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

    assert!(suite
        .nft
        .account(addr.clone().to_string())
        .unwrap_err()
        .to_string()
        .contains(&format!("No account associated with address {}", addr)));
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

        assert!(cosmwasm_crypto::secp256k1_verify(
            &msg_hash,
            signature.to_bytes().as_slice(),
            public_key.to_encoded_point(false).as_bytes()
        )
        .unwrap());

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
        assert!(err
            .to_string()
            .contains(&ContractError::AccountNotFound {}.to_string()))
    }
    // owner tries to set more than limit of associated addresses in recursion
    for i in 0..10 {
        let res = suite
            .nft
            .update_my_reverse_map_key(vec![carbs[i as usize].clone().carb], vec![]);
        if i == 10 {
            assert!(res.is_err());
            assert!(res
                .unwrap_err()
                .to_string()
                .contains(&ContractError::TooManyReverseMaps { max: 10, have: 11 }.to_string()))
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

    assert!(err
        .to_string()
        .contains(&ContractError::CannotRemoveMoreThanWillExists {}.to_string()));

    // try to remove more than exists via recusion

    // confirm only owner of token can remove tokens from map
    let err = suite
        .nft
        .call_as(&notminter)
        .update_my_reverse_map_key(vec![], vec![fifth_carb.carb.pubkey.to_string()])
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::AccountNotFound {}.to_string()));

    // owner tries to set more than limit of associated addresses in recursion
    for i in 0..10 {
        let res = suite
            .nft
            .update_my_reverse_map_key(vec![], vec![carbs[i as usize].carb.pubkey.to_string()]);
        if i == 10 {
            assert!(res.is_err());
            assert!(res
                .unwrap_err()
                .to_string()
                .contains(&ContractError::TooManyReverseMaps { max: 10, have: 0 }.to_string()))
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
