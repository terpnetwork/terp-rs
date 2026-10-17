use cosmwasm_std::{Addr, Binary, Coin, Uint128, Uint256};
use cw_headstash::msg::InstantiateMsg as HeadstashInstantiateMsg;
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_headstash::wavs::WavsProofOfOwnership;
use cw_headstash_manifold::msg::{
    ExecuteMsg as FactoryExecuteMsg, HeadstashContract, InstantiateMsg as FactoryInitMsg,
    QueryMsg as FactoryQueryMsg,
};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use cw_ownable::Ownership;
use std::collections::HashMap;

// Mock tokenfactory module for testing
#[derive(Clone, Default)]
pub struct MockTokenFactory {
    pub denoms: HashMap<String, TokenInfo>,
    pub balances: HashMap<(Addr, String), Uint128>,
}

#[derive(Clone, Debug)]
pub struct TokenInfo {
    pub admin: Addr,
    pub metadata: Option<Metadata>,
}

impl MockTokenFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_denom(
        &mut self,
        creator: Addr,
        subdenom: String,
        metadata: Option<Metadata>,
    ) -> Result<String, String> {
        let denom = format!("factory/{}/{}", creator, subdenom);
        if self.denoms.contains_key(&denom) {
            return Err("denom already exists".to_string());
        }

        self.denoms.insert(
            denom.clone(),
            TokenInfo {
                admin: creator,
                metadata,
            },
        );
        Ok(denom)
    }

    pub fn mint_tokens(&mut self, denom: &str, amount: Uint128, to: Addr) -> Result<(), String> {
        if !self.denoms.contains_key(denom) {
            return Err("denom does not exist".to_string());
        }

        let key = (to, denom.to_string());
        let zero = Uint128::zero();
        let current = self.balances.get(&key).unwrap_or(&zero);
        self.balances.insert(key, current + amount);
        Ok(())
    }

    pub fn get_balance(&self, addr: &Addr, denom: &str) -> Uint128 {
        self.balances
            .get(&(addr.clone(), denom.to_string()))
            .unwrap_or(&Uint128::zero())
            .clone()
    }
}

// Test environment following DA0-DA0 pattern
pub struct TestEnv {
    pub app: App,
    pub owner: Addr,
    pub factory_code_id: u64,
    pub headstash_code_id: u64,
    pub factory_addr: Addr,
    pub tokenfactory: MockTokenFactory,
    pub test_accounts: Vec<Addr>,
}

impl TestEnv {
    pub fn new() -> Self {
        let mut app = App::default();

        // Create test accounts
        let owner = app.api().addr_make("owner");
        let test_accounts: Vec<Addr> = (0..5)
            .map(|i| app.api().addr_make(&format!("test_account_{}", i)))
            .collect();

        // Fund test accounts
        for account in &test_accounts {
            app.init_modules(|router, _, storage| {
                router
                    .bank
                    .init_balance(
                        storage,
                        account,
                        vec![Coin::new(Uint128::new(1_000_000), "uosmo")],
                    )
                    .unwrap();
            });
        }

        // Initialize mock tokenfactory
        let tokenfactory = MockTokenFactory::new();

        // Store headstash contract
        let headstash_contract = Box::new(ContractWrapper::new(
            cw_headstash::execute,
            cw_headstash::instantiate,
            cw_headstash::query,
        ));
        let headstash_code_id = app.store_code(headstash_contract);

        // Store factory contract
        let factory_contract = Box::new(
            ContractWrapper::new(
                cw_headstash_manifold::contract::execute,
                cw_headstash_manifold::contract::instantiate,
                cw_headstash_manifold::contract::query,
            )
            .with_reply(cw_headstash_manifold::contract::reply),
        );
        let factory_code_id = app.store_code(factory_contract);

        // Instantiate factory
        let factory_addr = app
            .instantiate_contract(
                factory_code_id,
                owner.clone(),
                &FactoryInitMsg {
                    owner: Some(owner.to_string()),
                    headstash_code_id,
                },
                &[],
                "headstash-manifold",
                Some(owner.to_string()),
            )
            .unwrap();

        Self {
            app,
            owner,
            factory_code_id,
            headstash_code_id,
            factory_addr,
            tokenfactory,
            test_accounts,
        }
    }
}

// Helper functions for creating test data
pub fn mock_metadata() -> Metadata {
    Metadata {
        description: Some("Headstash Token".to_string()),
        denom_units: vec![
            DenomUnit {
                denom: "uhead".to_string(),
                exponent: 0,
                aliases: vec![],
            },
            DenomUnit {
                denom: "HEAD".to_string(),
                exponent: 6,
                aliases: vec!["head".to_string()],
            },
        ],
        base: Some("uhead".to_string()),
        display: Some("HEAD".to_string()),
        name: Some("Headstash Token".to_string()),
        symbol: Some("HEAD".to_string()),
    }
}

pub fn mock_token_strategy_new() -> TokenStrategy {
    TokenStrategy::NewFungible(NewTokenConfig {
        subdenom: HeadstashTokenObject::new("headstash".to_string()),
        metadata: mock_metadata(),
        initial_mint: Some(vec![InitialMint {
            to_address: "test_account_0".to_string(),
            amount: Uint128::new(1000),
        }]),
        manager: None,
        minters: vec![],
    })
}

pub fn mock_token_strategy_existing() -> TokenStrategy {
    TokenStrategy::ExistingFungible(HeadstashTokenObject::new("uosmo".to_string()))
}

pub fn mock_wavs_proof() -> WavsProofOfOwnership {
    // Mock WAVS proof - in real tests we'd use proper cryptographic mocks
    WavsProofOfOwnership {
        poos: vec![], // Empty for basic tests
        msg: cw_headstash::wavs::WavsAuthMetadata {
            aggregate_key: "mock_key".to_string(),
            threshold: 1,
            total_operators: 1,
            nonce: 0,
        },
    }
}

pub fn mock_headstash_instantiate_msg() -> HeadstashInstantiateMsg {
    HeadstashInstantiateMsg {
        genesis_root: Binary::from(vec![1u8; 32]), // Mock Poseidon-v1 genesis root
        token_strategy: mock_token_strategy_new(),
        distro_hash_domain: Default::default(),
        genesis_label: None,
        wavs: mock_wavs_proof(),
        circuit_id: None,
    }
}

// Test insanity cases - error conditions and edge cases

// Test headstash instantiation with invalid WAVS parameters
#[test]
fn test_insanity_invalid_wavs_threshold_zero() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.wavs.msg.threshold = 0; // Invalid threshold

    let err = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "invalid-wavs",
            Some(env.owner.to_string()),
        )
        .unwrap_err();

    assert!(err.to_string().contains("invalid threshold"));
}

#[test]
fn test_insanity_invalid_wavs_threshold_too_high() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.wavs.msg.threshold = 10; // Higher than total_operators

    let err = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "invalid-wavs-high",
            Some(env.owner.to_string()),
        )
        .unwrap_err();

    assert!(err.to_string().contains("invalid threshold"));
}

#[test]
fn test_insanity_invalid_wavs_empty_operators() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.wavs.poos = vec![]; // No operators but total_operators = 1

    let err = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "empty-operators",
            Some(env.owner.to_string()),
        )
        .unwrap_err();

    assert!(err.to_string().contains("invalid amount of operators"));
}

// Test token strategy validation failures
#[test]
fn test_insanity_invalid_token_proof() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();

    // Tamper with the token proof to make it invalid
    if let cw_headstash::tokenfactory::TokenStrategy::NewFungible(ref mut cfg) = msg.token_strategy
    {
        cfg.subdenom.proof = Binary::from(vec![1, 2, 3]); // Wrong proof
    }

    let err = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "invalid-token-proof",
            Some(env.owner.to_string()),
        )
        .unwrap_err();

    assert!(err.to_string().contains("bad token params"));
}

// Test factory insanity cases
#[test]
fn test_insanity_factory_unauthorized_creation() {
    let mut env = TestEnv::new();

    let msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: mock_headstash_instantiate_msg(),
        label: None,
        funding: None,
    };

    // Try to create as non-owner
    let err = env
        .app
        .execute_contract(
            env.test_accounts[0].clone(),
            env.factory_addr.clone(),
            &msg,
            &[],
        )
        .unwrap_err();

    assert!(err.to_string().contains("Unauthorized"));
}

#[test]
fn test_insanity_factory_unauthorized_ownership_transfer() {
    let mut env = TestEnv::new();

    let transfer_msg = FactoryExecuteMsg::UpdateOwnership(cw_ownable::Action::TransferOwnership {
        new_owner: env.test_accounts[0].to_string(),
        expiry: None,
    });

    // Try ownership transfer as non-owner
    let err = env
        .app
        .execute_contract(
            env.test_accounts[0].clone(),
            env.factory_addr.clone(),
            &transfer_msg,
            &[],
        )
        .unwrap_err();

    assert!(err.to_string().contains("Unauthorized"));
}

// #[test]
// fn test_insanity_factory_unauthorized_code_update() {
//     let mut env = TestEnv::new();

//     let update_msg = FactoryExecuteMsg::UpdateCodeId {
//         headstash_code_id: 999,
//     };

//     // Try code update as non-owner
//     let err = env
//         .app
//         .execute_contract(
//             env.test_accounts[0].clone(),
//             env.factory_addr.clone(),
//             &update_msg,
//             &[],
//         )
//         .unwrap_err();

//     assert!(err.to_string().contains("Unauthorized"));
// }

// Test funding validation failures
#[test]
fn test_insanity_factory_insufficient_funding() {
    let mut env = TestEnv::new();

    let msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: mock_headstash_instantiate_msg(),
        label: None,
        funding: Some(cw_headstash_manifold::msg::FundingInfo {
            token: cw_headstash_manifold::msg::FundingToken::Native {
                denom: "uosmo".to_string(),
            },
            amount: Uint256::new(1000),
        }),
    };

    // Try with insufficient funds
    let err = env
        .app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &msg,
            &[Coin::new(500u128, "uosmo")], // Less than required
        )
        .unwrap_err();

    // Should fail with funding validation error
    assert!(err.to_string().contains("insufficient"));
}

#[test]
fn test_insanity_factory_wrong_funding_denom() {
    let mut env = TestEnv::new();

    let msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: mock_headstash_instantiate_msg(),
        label: None,
        funding: Some(cw_headstash_manifold::msg::FundingInfo {
            token: cw_headstash_manifold::msg::FundingToken::Native {
                denom: "uosmo".to_string(),
            },
            amount: Uint256::new(1000),
        }),
    };

    // Try with wrong denom
    let err = env
        .app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &msg,
            &[Coin::new(1000u128, "uatom")], // Wrong denom
        )
        .unwrap_err();

    assert!(err.to_string().contains("funding"));
}

// Test query edge cases
#[test]
fn test_insanity_factory_query_nonexistent_contract() {
    let env = TestEnv::new();

    // Query for non-existent contract
    let err: Result<cw_headstash_manifold::msg::HeadstashContract, _> =
        env.app.wrap().query_wasm_smart(
            &env.factory_addr,
            &FactoryQueryMsg::Contract {
                address: "nonexistent".to_string(),
            },
        );

    assert!(err.is_err());
}

#[test]
fn test_insanity_factory_query_invalid_address() {
    let env = TestEnv::new();

    // Query with invalid address
    let err: Result<Vec<cw_headstash_manifold::msg::HeadstashContract>, _> =
        env.app.wrap().query_wasm_smart(
            &env.factory_addr,
            &FactoryQueryMsg::ContractsByInstantiator {
                instantiator: "invalid_address".to_string(),
                start_after: None,
                limit: Some(10),
            },
        );

    assert!(err.is_err());
}

// Test instantiation edge cases
#[test]
fn test_insanity_empty_genesis_root() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.genesis_root = Binary::default(); // Empty genesis root

    // This might succeed or fail depending on implementation
    // The test ensures we handle empty genesis root appropriately
    let result = env.app.instantiate_contract(
        env.headstash_code_id,
        env.owner.clone(),
        &msg,
        &[],
        "empty-genesis",
        Some(env.owner.to_string()),
    );

    // Document the expected behavior - should either succeed or give clear error
    match result {
        Ok(_) => println!("Empty genesis root allowed"),
        Err(e) => assert!(e.to_string().contains("genesis") || e.to_string().contains("empty")),
    }
}

// Test duplicate instantiation attempts
#[test]
fn test_insanity_duplicate_instantiation() {
    let mut env = TestEnv::new();

    let msg = mock_headstash_instantiate_msg();

    // First instantiation should succeed
    let addr1 = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "duplicate-test-1",
            Some(env.owner.to_string()),
        )
        .unwrap();

    // Second instantiation with same label should fail (if label collision checking exists)
    // Note: This depends on CosmWasm's label uniqueness - may not actually fail
    let result = env.app.instantiate_contract(
        env.headstash_code_id,
        env.owner.clone(),
        &msg,
        &[],
        "duplicate-test-1", // Same label
        Some(env.owner.to_string()),
    );

    // Document behavior - may succeed if labels don't enforce uniqueness
    match result {
        Ok(addr2) => assert_ne!(addr1, addr2), // Different addresses
        Err(e) => assert!(e.to_string().contains("label") || e.to_string().contains("duplicate")),
    }
}
