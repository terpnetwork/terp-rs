use cosmwasm_std::{Addr, Binary, Coin, Empty, Uint128};
use cw_headstash::msg::{
    ExecuteMsg as HeadstashExecuteMsg, InstantiateMsg as HeadstashInstantiateMsg,
};
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig,
};
use cw_headstash::wavs::WavsProofOfOwnership;
use cw_headstash::{QueryMsg as HeadstashQueryMsg, tokenfactory::TokenStrategy};
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
// Test headstash instantiation with new fungible token
#[test]
fn test_headstash_instantiate_new_fungible() {
    let mut env = TestEnv::new();

    let msg = mock_headstash_instantiate_msg();

    // Instantiate headstash directly (not through factory for unit testing)
    let headstash_addr = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[],
            "test-headstash",
            Some(env.owner.to_string()),
        )
        .unwrap();

    // Verify contract was created
    assert!(!headstash_addr.as_str().is_empty());

    // Test that we can query the contract
    let ownership: cw_ownable::Ownership<Addr> = env
        .app
        .wrap()
        .query_wasm_smart(&headstash_addr, &HeadstashQueryMsg::Ownership {})
        .unwrap();

    assert_eq!(ownership.owner, Some(env.owner));
}

// Test headstash instantiation with existing fungible token (requires pre-funding)
#[test]
fn test_headstash_instantiate_existing_fungible() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.token_strategy = mock_token_strategy_existing();

    // Pre-fund the contract with existing tokens
    let prefund_amount = Coin::new(1000u128, "uosmo");
    env.app.init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(storage, &env.owner, vec![prefund_amount.clone()])
            .unwrap();
    });

    let headstash_addr = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[prefund_amount],
            "test-headstash-existing",
            Some(env.owner.to_string()),
        )
        .unwrap();

    assert!(!headstash_addr.as_str().is_empty());
}

// Test instantiation failure with insufficient pre-funding for existing token
#[test]
fn test_headstash_instantiate_existing_insufficient_funds() {
    let mut env = TestEnv::new();

    let mut msg = mock_headstash_instantiate_msg();
    msg.token_strategy = mock_token_strategy_existing();

    // Try to instantiate without pre-funding
    let err = env
        .app
        .instantiate_contract(
            env.headstash_code_id,
            env.owner.clone(),
            &msg,
            &[], // No funds provided
            "test-headstash-existing",
            Some(env.owner.to_string()),
        )
        .unwrap_err();

    assert!(err.to_string().contains("at least 1 token required"));
}

// Test token strategy validation
#[test]
fn test_token_strategy_validation() {
    let strategy = mock_token_strategy_new();

    // Valid strategy should pass
    assert!(strategy.validate().is_ok());

    // Test invalid strategy (tamper with proof)
    let mut invalid_strategy = strategy.clone();
    if let TokenStrategy::NewFungible(ref mut cfg) = invalid_strategy {
        cfg.subdenom.proof = Binary::from(vec![0, 0, 0]); // Invalid proof
    }

    assert!(invalid_strategy.validate().is_err());
}

// Test WAVS proof validation
#[test]
fn test_wavs_proof_validation() {
    let proof = mock_wavs_proof();

    // Valid proof should pass
    assert!(proof.verify().is_ok());

    // Test invalid proof - wrong operator count
    let mut invalid_proof = proof.clone();
    invalid_proof.poos = vec![]; // Empty but total_operators = 1
    assert!(invalid_proof.verify().is_err());

    // Test invalid threshold
    let mut invalid_proof2 = proof.clone();
    invalid_proof2.msg.threshold = 0; // Invalid threshold
    assert!(invalid_proof2.verify().is_err());
}

// Test denom generation
#[test]
fn test_denom_generation() {
    let contract_addr = Addr::unchecked("cosmos1contract");
    let strategy = mock_token_strategy_new();

    let denom = strategy.denom(&contract_addr);
    assert!(denom.starts_with("factory/"));
    assert!(denom.contains("cosmos1contract"));
    assert!(denom.ends_with("headstash"));
}

// Test initial mint message generation
#[test]
fn test_initial_mint_messages() {
    let contract_addr = Addr::unchecked("cosmos1contract");
    let strategy = mock_token_strategy_new();

    let messages = strategy.initial_mint_msgs(&contract_addr).unwrap();

    // For new fungible tokens, initial minting is now handled in the reply
    // so initial_mint_msgs returns empty
    assert_eq!(messages.len(), 0);
}

// Test existing token strategy (no messages generated)
#[test]
fn test_existing_token_no_messages() {
    let contract_addr = Addr::unchecked("cosmos1contract");
    let strategy = mock_token_strategy_existing();

    let messages = strategy.initial_mint_msgs(&contract_addr).unwrap();
    assert_eq!(messages.len(), 0); // Existing tokens don't create any messages
}

// Test prefunding requirement check
#[test]
fn test_prefunding_requirement() {
    let new_strategy = mock_token_strategy_new();
    let existing_strategy = mock_token_strategy_existing();

    assert!(!new_strategy.requires_prefund());
    assert!(existing_strategy.requires_prefund());
}

// Test proof representation extraction
#[test]
fn test_proof_representation() {
    let strategy = mock_token_strategy_new();
    let proof = strategy.proof_representation();

    // Should be the blake3 hash of the subdenom
    assert_eq!(proof.len(), 32); // Blake3 hash length
}
