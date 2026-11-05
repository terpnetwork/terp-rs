use cosmwasm_std::{Addr, Binary, Coin, Uint128};
use cw_headstash::msg::InstantiateMsg as HeadstashInstantiateMsg;
use cw_headstash::tokenfactory::{
    DenomUnit, HeadstashTokenObject, InitialMint, Metadata, NewTokenConfig, TokenStrategy,
};
use cw_headstash::wavs::WavsProofOfOwnership;
use cw_headstash_manifold::msg::{
    ExecuteMsg as FactoryExecuteMsg, HeadstashContract, InstantiateMsg as FactoryInitMsg,
    QueryMsg as FactoryQueryMsg,
};
use cw_multi_test::{ContractWrapper, Executor};
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
        *self
            .balances
            .get(&(addr.clone(), denom.to_string()))
            .unwrap_or(&Uint128::zero())
    }
}

// Test environment following DA0-DA0 pattern
pub struct TestEnv {
    pub app: crate::tokenfactory_stargate::TfApp,
    pub owner: Addr,
    pub factory_code_id: u64,
    pub headstash_code_id: u64,
    pub factory_addr: Addr,
    pub tokenfactory: MockTokenFactory,
    pub test_accounts: Vec<Addr>,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnv {
    pub fn new() -> Self {
        let mut app = crate::tokenfactory_stargate::tf_app();

        // Create test accounts
        let owner = app.api().addr_make("owner");
        app.init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(
                    storage,
                    &owner,
                    vec![Coin::new(Uint128::new(1_000_000), "uosmo")],
                )
                .unwrap();
        });
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
        let headstash_contract = Box::new(
            ContractWrapper::new(
                cw_headstash::execute,
                cw_headstash::instantiate,
                cw_headstash::query,
            )
            .with_reply(cw_headstash::reply),
        );
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
    TokenStrategy::NewFungible(Box::new(NewTokenConfig {
        subdenom: HeadstashTokenObject::new(
            &cosmwasm_std::testing::MockApi::default(),
            "headstash".to_string(),
        ),
        metadata: mock_metadata(),
        initial_mint: Some(vec![InitialMint {
            to_address: "test_account_0".to_string(),
            amount: Uint128::new(1000),
        }]),
        manager: None,
        minters: vec![],
    }))
}

pub fn mock_token_strategy_existing() -> TokenStrategy {
    TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
        &cosmwasm_std::testing::MockApi::default(),
        "uosmo".to_string(),
    ))
}

pub fn mock_wavs_proof() -> WavsProofOfOwnership {
    // One real operator: instantiate verifies the PoP pairing.
    crate::wavs_fixture::valid_wavs_proof(1)
}

pub fn mock_headstash_instantiate_msg() -> HeadstashInstantiateMsg {
    HeadstashInstantiateMsg {
        genesis_root: Binary::from(vec![1u8; 32]), // Mock Poseidon-v1 genesis root
        token_strategy: mock_token_strategy_new(),
        distro_hash_domain: Default::default(),
        genesis_label: None,
        wavs: Some(mock_wavs_proof()),
        circuit_id: None,
        registration_hook: None,
        registration_hook_gas_limit: None,
        frontend_contract: None,
        pauser: None,
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

    // Exactly one instantiate reply on the factory, plus exactly one
    // create-denom reply on the new headstash (NewFungible strategy).
    let replies_on = |addr: &str| {
        res.events
            .iter()
            .filter(|e| {
                e.ty == "reply"
                    && e.attributes
                        .iter()
                        .any(|a| a.key == "_contract_address" && a.value == addr)
            })
            .count()
    };
    assert_eq!(replies_on(env.factory_addr.as_str()), 1);
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
    // Manifold gates owner-only actions with cw_ownable; assert that exact error.
    assert!(
        err.to_string()
            .contains(&cw_ownable::OwnershipError::NotOwner.to_string()),
        "expected NotOwner, got: {err}"
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

// Exact-bytes pins for the tokenfactory Stargate values: wasmd unpacks these as
// protobuf Any, so JSON (the pre-fix encoding) is rejected on chain.
#[test]
fn tf_proto_exact_bytes_round_trip() {
    use cw_headstash::tf_proto;
    let cd = tf_proto::msg_create_denom("terp1abc", "hs");
    let mut want = vec![0x0a, 0x08];
    want.extend_from_slice(b"terp1abc");
    want.extend_from_slice(&[0x12, 0x02]);
    want.extend_from_slice(b"hs");
    assert_eq!(cd, want);

    let mint = tf_proto::msg_coin_addr("s", "factory/s/hs", "1000", "r");
    let mut coin = vec![0x0a, 12];
    coin.extend_from_slice(b"factory/s/hs");
    coin.extend_from_slice(&[0x12, 0x04]);
    coin.extend_from_slice(b"1000");
    let mut want = vec![0x0a, 0x01, b's', 0x12, coin.len() as u8];
    want.extend_from_slice(&coin);
    want.extend_from_slice(&[0x1a, 0x01, b'r']);
    assert_eq!(mint, want);

    // Independent fixture parser reads the same bytes back.
    let f = tf_stargate::proto_fields(&mint).unwrap();
    assert_eq!(f.len(), 3);
    assert_eq!(f[0], (1, b"s".to_vec()));
    assert_eq!(f[1], (2, coin.clone()));
    assert_eq!(f[2], (3, b"r".to_vec()));

    // Long string: multi-byte varint length.
    let long = "x".repeat(300);
    let b = tf_proto::msg_create_denom(&long, "");
    assert_eq!(&b[..3], &[0x0a, 0xac, 0x02]);
    assert_eq!(b.len(), 3 + 300);
    assert_eq!(tf_proto::string_field(&b, 1).unwrap(), long);

    // MsgCreateDenomResponse decode (what the chain returns in msg_responses[0]).
    let mut resp = vec![0x0a, 12];
    resp.extend_from_slice(b"factory/s/hs");
    assert_eq!(tf_proto::create_denom_response_denom(&resp).unwrap(), "factory/s/hs");
    assert!(tf_proto::create_denom_response_denom(&[]).is_err());

    // JSON regression: neither the fixture nor the contract decoder accepts it.
    let json = br#"{"sender":"s","subdenom":"hs"}"#;
    assert!(tf_stargate::proto_fields(json).is_err());
    assert!(tf_proto::create_denom_response_denom(br#"{"new_token_denom":"x"}"#).is_err());
}
