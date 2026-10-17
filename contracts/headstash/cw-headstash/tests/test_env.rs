// Tests for cw-headstash and cw-headstash-manifold contracts using cw-multi-test
// Focus on auxiliary functions and tokenfactory interactions

use cosmwasm_std::{Addr, Binary, Coin, Empty, Uint128};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use cw_ownable::Ownership;
use hex;
use std::collections::HashMap;

// Import contract messages
use cw_headstash::msg::{
    ExecuteMsg as HeadstashExecuteMsg, InstantiateMsg as HeadstashInstantiateMsg,
    QueryMsg as HeadstashQueryMsg,
};
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_headstash::wavs::WavsProofOfOwnership;
use cw_headstash_manifold::msg::{
    ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInstantiateMsg,
    QueryMsg as FactoryQueryMsg,
};
// Import our local metadata struct

// Mock tokenfactory module for testing
#[derive(Clone, Default)]
pub struct MockTokenFactory {
    pub denoms: HashMap<String, TokenInfo>,
    pub balances: HashMap<(Addr, String), Uint128>,
}

const ZERO: &Uint128 = &Uint128::zero();

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
        let current = self.balances.get(&key).unwrap_or(ZERO);
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
                    .init_balance(storage, account, vec![Coin::new(1_000_000u128, "uosmo")])
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
                &FactoryInstantiateMsg {
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

    pub fn create_headstash(
        &mut self,
        instantiate_msg: HeadstashInstantiateMsg,
        label: Option<String>,
        funds: Vec<Coin>,
    ) -> Result<Addr, anyhow::Error> {
        let msg = FactoryExecuteMsg::CreateHeadstash {
            instantiate_msg,
            label,
            funding: None, // We'll test funding separately
        };

        self.app
            .execute_contract(self.owner.clone(), self.factory_addr.clone(), &msg, &funds)
            .map_err(|e| anyhow::Error::msg(e.to_string()))?;

        // Get the created headstash address from factory query
        // This would need to be implemented based on factory's contract registry
        // For now, return a mock address
        Ok(Addr::unchecked("headstash_addr"))
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
    // Mock WAVS proof - disabled for basic tests
    WavsProofOfOwnership {
        poos: vec![], // Empty for basic tests
        msg: cw_headstash::wavs::WavsAuthMetadata {
            aggregate_key: "mock_key".to_string(),
            threshold: 0,
            total_operators: 0,
            nonce: 0,
        },
    }
}

pub fn mock_wavs_proof_with_operators(count: usize) -> WavsProofOfOwnership {
    use cosmwasm_std::{BLS12_381_G1_GENERATOR as G1, BLS12_381_G2_GENERATOR as G2};

    // Generate mock BLS keypairs and proofs for testing
    let mut poos = vec![];
    for i in 0..count {
        // Use deterministic mock keys for reproducible tests
        // Convert BLS points to hex strings as expected by WavsOpAuth
        let mock_public_g1_hex = hex::encode(&G1);
        let mock_proof_hex = hex::encode(format!("mock_proof_{}", i).as_bytes());

        poos.push(cw_headstash::wavs::WavsOpAuth {
            key: mock_public_g1_hex, // G1 point as hex string
            poo: mock_proof_hex,     // Proof as hex string
        });
    }

    WavsProofOfOwnership {
        poos,
        msg: cw_headstash::wavs::WavsAuthMetadata {
            aggregate_key: "mock_aggregate_key".to_string(),
            threshold: count, // Require all operators
            total_operators: count,
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
