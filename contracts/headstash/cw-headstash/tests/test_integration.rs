use cosmwasm_std::{Addr, Binary, Coin, Uint128};
use cw_headstash::msg::InstantiateMsg as HeadstashInstantiateMsg;
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_headstash::wavs::WavsProofOfOwnership;

use cw_headstash_manifold::msg::{
    ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInitMsg, QueryMsg as FactoryQueryMsg,
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

// Test full end-to-end flow: Factory creates Headstash -> Headstash initializes
#[test]
fn test_integration_factory_to_headstash() {
    let mut env = TestEnv::new();

    // Create headstash through factory
    let instantiate_msg = mock_headstash_instantiate_msg();

    let create_msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: instantiate_msg.clone(),
        label: Some("integration-test-headstash".to_string()),
        funding: None,
    };

    // Execute creation through factory
    let res = env
        .app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &create_msg,
            &[],
        )
        .unwrap();

    // Verify submessage was created
    assert!(!res.events.is_empty());

    // In a full integration test, we would:
    // 1. Mock the tokenfactory module responses
    // 2. Verify headstash was created with correct parameters
    // 3. Verify factory registered the contract
    // 4. Verify token operations succeeded

    // For now, verify the execution succeeded and created events
    let wasm_events: Vec<_> = res.events.iter().filter(|e| e.ty == "wasm").collect();
    assert!(!wasm_events.is_empty());
}

// Test tokenfactory integration with headstash creation
#[test]
fn test_integration_tokenfactory_operations() {
    let mut env = TestEnv::new();

    // Test that headstash can create denoms through tokenfactory
    let instantiate_msg = mock_headstash_instantiate_msg();

    // Create headstash directly to test tokenfactory integration
    let headstash_addr = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &instantiate_msg,
            &[],
            "tokenfactory-test",
            Some(env.owner.to_string()),
        )
        .unwrap();

    // In a real test environment, we would mock tokenfactory responses
    // and verify that CreateDenom and MintTokens messages were sent

    // For now, verify the contract was created successfully
    assert!(!headstash_addr.as_str().is_empty());
}

// Test funding flow through factory
#[test]
fn test_integration_factory_funding() {
    let mut env = TestEnv::new();

    // Test creating headstash with funding through factory
    let instantiate_msg = mock_headstash_instantiate_msg();
    let funding_amount = Coin::new(1000u128, "uosmo");

    let create_msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg,
        label: Some("funded-headstash".to_string()),
        funding: Some(cw_headstash_manifold::msg::FundingInfo {
            token: cw_headstash_manifold::msg::FundingToken::Native {
                denom: "uosmo".to_string(),
            },
            amount: funding_amount.amount,
        }),
    };

    // Fund the factory call
    let res = env
        .app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &create_msg,
            &[funding_amount.clone()],
        )
        .unwrap();

    // Verify funding was processed
    assert!(!res.events.is_empty());
}

// Test error propagation from headstash to factory
#[test]
fn test_integration_error_propagation() {
    let mut env = TestEnv::new();

    // Create invalid headstash instantiate message
    let mut invalid_msg = mock_headstash_instantiate_msg();
    // Make WAVS invalid
    invalid_msg.wavs.msg.threshold = 0;

    let create_msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: invalid_msg,
        label: None,
        funding: None,
    };

    // This should fail during headstash instantiation
    let err = env
        .app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &create_msg,
            &[],
        )
        .unwrap_err();

    // Verify error was propagated
    assert!(err.to_string().contains("invalid threshold"));
}

// Test multiple headstash creations and registry
#[test]
fn test_integration_multiple_creations() {
    let mut env = TestEnv::new();

    // Create multiple headstashes
    for i in 0..3 {
        let mut instantiate_msg = mock_headstash_instantiate_msg();
        // Modify subdenom to make unique
        if let cw_headstash::tokenfactory::TokenStrategy::NewFungible(ref mut cfg) =
            instantiate_msg.token_strategy
        {
            cfg.subdenom =
                cw_headstash::tokenfactory::HeadstashTokenObject::new(format!("headstash{}", i));
        }

        let create_msg = FactoryExecuteMsg::CreateHeadstash {
            instantiate_msg,
            label: Some(format!("headstash-{}", i)),
            funding: None,
        };

        env.app
            .execute_contract(
                env.owner.clone(),
                env.factory_addr.clone(),
                &create_msg,
                &[],
            )
            .unwrap();
    }

    // Query contracts by instantiator
    let contracts: Vec<cw_headstash_manifold::msg::HeadstashContract> = env
        .app
        .wrap()
        .query_wasm_smart(
            &env.factory_addr,
            &FactoryQueryMsg::ContractsByInstantiator {
                instantiator: env.owner.to_string(),
                start_after: None,
                limit: Some(10),
            },
        )
        .unwrap();

    // Should have created 3 contracts
    assert_eq!(contracts.len(), 3);

    // Verify each contract has the owner as instantiator
    for contract in contracts {
        assert_eq!(contract.instantiator, env.owner);
    }
}

// Test cross-contract queries
#[test]
fn test_integration_cross_contract_queries() {
    let mut env = TestEnv::new();

    // Create a headstash
    let instantiate_msg = mock_headstash_instantiate_msg();

    let create_msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg: instantiate_msg.clone(),
        label: Some("query-test".to_string()),
        funding: None,
    };

    env.app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &create_msg,
            &[],
        )
        .unwrap();

    // Query factory for contracts
    let factory_contracts: Vec<cw_headstash_manifold::msg::HeadstashContract> = env
        .app
        .wrap()
        .query_wasm_smart(
            &env.factory_addr,
            &FactoryQueryMsg::ContractsByInstantiator {
                instantiator: env.owner.to_string(),
                start_after: None,
                limit: Some(1),
            },
        )
        .unwrap();

    assert_eq!(factory_contracts.len(), 1);

    // In a full integration, we would also query the headstash contract directly
    // to verify it was created with the correct parameters
}
