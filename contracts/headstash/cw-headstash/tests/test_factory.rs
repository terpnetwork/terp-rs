use cosmwasm_std::{Addr, Binary, Coin, Empty, Uint128};
use cw_headstash::msg::{
    ExecuteMsg as HeadstashExecuteMsg, InstantiateMsg as HeadstashInstantiateMsg,
    QueryMsg as HeadstashQueryMsg,
};
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_headstash::wavs::WavsProofOfOwnership;
use cw_headstash_manifold::msg::{
    ExecuteMsg as FactoryExecuteMsg, HeadstashContract, InstantiateMsg as FactoryInitMsg,
    QueryMsg as FactoryQueryMsg,
};
use cw_multi_test::{Contract, ContractWrapper, Executor};
include!("common/tf_stargate.rs");
use tf_stargate::{new_app, TfApp as App};
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
        let mut app = new_app();

        // Create test accounts
        let owner = app.api().addr_make("owner");
        let test_accounts: Vec<Addr> = (0..5)
            .map(|i| app.api().addr_make(&format!("test_account_{}", i)))
            .collect();

        // Fund owner (factory-funding tests send from owner) and test accounts
        for account in std::iter::once(&owner).chain(test_accounts.iter()) {
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
        )
        .with_reply(cw_headstash::reply));
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
    // Deterministic valid 1-operator BLS12-381 proof of possession, so instantiate's
    // operator-count, threshold and pairing checks pass (same as test_bridge_e2e).
    use ark_bls12_381::{Fr, G1Affine, G2Affine};
    use ark_ec::AffineRepr;
    use ark_ff::PrimeField;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use cosmwasm_std::{testing::MockApi, Api, HashFunction, BLS12_381_G2_GENERATOR as G2};

    let api = MockApi::default();
    let sk = Fr::from_be_bytes_mod_order(&[0x42; 32]);
    let pk: G1Affine = (G1Affine::generator() * sk).into();
    let mut pk_bytes = vec![];
    pk.serialize_compressed(&mut pk_bytes).unwrap();
    let pop_hash = api
        .bls12_381_hash_to_g2(HashFunction::Sha256, &pk_bytes, &G2)
        .unwrap();
    let h_point = G2Affine::deserialize_compressed(&pop_hash[..]).unwrap();
    let pop_sig: G2Affine = (h_point * sk).into();
    let mut sig_bytes = vec![];
    pop_sig.serialize_compressed(&mut sig_bytes).unwrap();

    WavsProofOfOwnership {
        poos: vec![cw_headstash::wavs::WavsOpAuth {
            key: hex::encode(&pk_bytes),
            poo: hex::encode(&sig_bytes),
        }],
        msg: cw_headstash::wavs::WavsAuthMetadata {
            aggregate_key: hex::encode(&pk_bytes),
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

// Test factory instantiation
#[test]
fn test_factory_instantiate() {
    let env = TestEnv::new();

    // Verify factory was created
    assert!(!env.factory_addr.as_str().is_empty());

    // Test ownership query
    let ownership: cw_ownable::Ownership<Addr> = env
        .app
        .wrap()
        .query_wasm_smart(&env.factory_addr, &FactoryQueryMsg::Ownership {})
        .unwrap();

    assert_eq!(ownership.owner, Some(env.owner));
}

// Test headstash creation through factory
#[test]
fn test_factory_create_headstash() {
    let mut env = TestEnv::new();

    let instantiate_msg = mock_headstash_instantiate_msg();

    let msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg,
        label: Some("test-headstash".to_string()),
        funding: None,
    };

    let res = env
        .app
        .execute_contract(env.owner.clone(), env.factory_addr.clone(), &msg, &[])
        .unwrap();

    // Should have created a submessage for instantiation
    assert!(!res.events.is_empty());

    // Check for instantiate reply event
    // factory instantiate reply + headstash MsgCreateDenom reply
    let reply_events: Vec<_> = res.events.iter().filter(|e| e.ty == "reply").collect();
    assert_eq!(reply_events.len(), 2);
}

// Test factory query functions
#[test]
fn test_factory_queries() {
    let env = TestEnv::new();

    // Test empty contracts query
    let contracts: Vec<HeadstashContract> = env
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

    // Should be empty initially
    assert_eq!(contracts.len(), 0);
}

// Test unauthorized factory operations
#[test]
fn test_factory_unauthorized() {
    let mut env = TestEnv::new();

    // Try to create headstash as non-owner
    let instantiate_msg = mock_headstash_instantiate_msg();
    let msg = FactoryExecuteMsg::CreateHeadstash {
        instantiate_msg,
        label: None,
        funding: None,
    };

    let err = env
        .app
        .execute_contract(
            env.test_accounts[0].clone(), // Non-owner
            env.factory_addr.clone(),
            &msg,
            &[],
        )
        .unwrap_err();

    // Should fail with unauthorized error
    // manifold surfaces cw_ownable::OwnershipError via ContractError::Ownership("{0}")
    let e = err.to_string();
    assert!(
        e.contains("Unauthorized") || e.contains("not the contract's current owner"),
        "unexpected error: {e}"
    );
}

// Test ownership transfer
#[test]
fn test_factory_ownership_transfer() {
    let mut env = TestEnv::new();

    let new_owner = env.test_accounts[0].clone();

    // Initiate ownership transfer
    let transfer_msg = FactoryExecuteMsg::UpdateOwnership(cw_ownable::Action::TransferOwnership {
        new_owner: new_owner.to_string(),
        expiry: None,
    });

    env.app
        .execute_contract(
            env.owner.clone(),
            env.factory_addr.clone(),
            &transfer_msg,
            &[],
        )
        .unwrap();

    // Accept ownership transfer
    let accept_msg = FactoryExecuteMsg::UpdateOwnership(cw_ownable::Action::AcceptOwnership {});

    env.app
        .execute_contract(
            new_owner.clone(),
            env.factory_addr.clone(),
            &accept_msg,
            &[],
        )
        .unwrap();

    // Verify ownership changed
    let ownership: cw_ownable::Ownership<Addr> = env
        .app
        .wrap()
        .query_wasm_smart(&env.factory_addr, &FactoryQueryMsg::Ownership {})
        .unwrap();

    assert_eq!(ownership.owner, Some(new_owner));
}

// Factory doesn't support code ID updates after instantiation
