//! Groups of cw-orch contracts, plus one bag that deploys every published
//! artifact under `crates/terp-rs/artifacts/`.
//!
//! | Group | Contracts |
//! |-------|-----------|
//! | authenticators | `TerpAuthenticatorSuite` (passkey, recovery, ed25519, eth, irl, zk-jwt, vsck) |
//! | circuits | `cw-zk-verify-demo` |
//! | bridge | `cw-private-dex` |
//! | accounts | dao (ticketing), dregg authenticator, frost, bls, zktls, polytone-any-proxy |
//! | light clients | crosslink, dregg settlement, terp IAVL, zap1 |
//!
//! Enable with `--features stack`. Live ict-rs upload adds `--features ict`.

use cosmwasm_std::{Addr, Binary};
use cw_orch::prelude::*;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};

use crate::interfaces::accounts::{PolytoneAnyProxy, TerpBls, TerpDao, TerpDregg, TerpFrost, TerpZkTls};
use crate::interfaces::light::{
    CwIcs08WasmCrosslink, CwIcs08WasmDregg, CwIcs08WasmTerp, CwIcs08WasmZap1,
};
use crate::orch::{TerpOrchDeployData, TerpOrchSuite};

#[derive(Clone, Debug)]
pub struct TerpArtifactStackDeployData {
    pub admin: Addr,
}

impl From<Addr> for TerpArtifactStackDeployData {
    fn from(admin: Addr) -> Self {
        Self { admin }
    }
}

/// Ticketing, settlement authenticator, threshold accounts, and the polytone dispatcher.
pub struct TerpAccountSuite<Chain: CwEnv> {
    pub dao: TerpDao<Chain>,
    pub dregg: TerpDregg<Chain>,
    pub frost: TerpFrost<Chain>,
    pub bls: TerpBls<Chain>,
    pub zktls: TerpZkTls<Chain>,
    pub polytone: PolytoneAnyProxy<Chain>,
}

impl<Chain: CwEnv> TerpAccountSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            dao: TerpDao::new(chain.clone()),
            dregg: TerpDregg::new(chain.clone()),
            frost: TerpFrost::new(chain.clone()),
            bls: TerpBls::new(chain.clone()),
            zktls: TerpZkTls::new(chain.clone()),
            polytone: PolytoneAnyProxy::new(chain),
        }
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.dao.upload()?;
        self.dregg.upload()?;
        self.frost.upload()?;
        self.bls.upload()?;
        self.zktls.upload()?;
        self.polytone.upload()?;
        Ok(())
    }

    pub fn instantiate_lab(&self, admin: &Addr) -> Result<(), CwOrchError> {
        let admin_s = admin.to_string();
        self.dao.instantiate(
            &terp_dao::msg::InstantiateMsg {
                dao_addr: admin_s.clone(),
            },
            Some(admin),
            &[],
        )?;
        self.dregg.instantiate(
            &terp_dregg::InstantiateMsg {
                owner: Some(admin_s.clone()),
                mode: terp_dregg::AuthMode::EmbeddedHead,
                lc_contract: None,
                min_proven_height: Some(0),
                require_root_in_history: Some(false),
            },
            Some(admin),
            &[],
        )?;
        self.frost.instantiate(
            &terp_frost::InstantiateMsg {
                group_vk: "lab-group-vk".into(),
                t: 2,
                n: 3,
                seats: vec!["desk".into(), "steward".into(), "spare".into()],
            },
            Some(admin),
            &[],
        )?;
        self.bls.instantiate(
            &terp_bls::msg::InstantiateMsg {
                owner: None,
                operator_pubkeys: vec![Binary::from(cosmwasm_std::BLS12_381_G1_GENERATOR.to_vec())],
                threshold: 1,
                variant: terp_bls::BlsVariant::MinPk,
            },
            Some(admin),
            &[],
        )?;
        self.zktls.instantiate(
            &terp_zktls::InstantiateMsg {
                owner: admin_s,
            },
            Some(admin),
            &[],
        )?;
        self.polytone.instantiate(
            &polytone_any_proxy::InstantiateMsg {
                unrestricted: Some(true),
            },
            Some(admin),
            &[],
        )?;
        Ok(())
    }
}

/// 08-wasm clients. Instantiate messages are lab fixtures, not a live header.
pub struct TerpLightClientSuite<Chain: CwEnv> {
    pub crosslink: CwIcs08WasmCrosslink<Chain>,
    pub dregg: CwIcs08WasmDregg<Chain>,
    pub terp: CwIcs08WasmTerp<Chain>,
    pub zap1: CwIcs08WasmZap1<Chain>,
}

impl<Chain: CwEnv> TerpLightClientSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            crosslink: CwIcs08WasmCrosslink::new(chain.clone()),
            dregg: CwIcs08WasmDregg::new(chain.clone()),
            terp: CwIcs08WasmTerp::new(chain.clone()),
            zap1: CwIcs08WasmZap1::new(chain),
        }
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.crosslink.upload()?;
        self.dregg.upload()?;
        self.terp.upload()?;
        self.zap1.upload()?;
        Ok(())
    }

    pub fn instantiate_lab(&self, admin: &Addr) -> Result<(), CwOrchError> {
        self.crosslink
            .instantiate(&crosslink_instantiate(), Some(admin), &[])?;
        self.dregg
            .instantiate(&dregg_instantiate(), Some(admin), &[])?;
        self.terp
            .instantiate(&terp_iavl_instantiate(), Some(admin), &[])?;
        self.zap1
            .instantiate(&crate::interfaces::light::Zap1Instantiate {}, Some(admin), &[])?;
        Ok(())
    }
}

/// Every optimized artifact: authenticators, verify demo, private dex, accounts, light clients.
pub struct TerpArtifactStack<Chain: CwEnv> {
    pub orch: TerpOrchSuite<Chain>,
    pub authenticators: TerpAuthenticatorSuite<Chain>,
    pub accounts: TerpAccountSuite<Chain>,
    pub light_clients: TerpLightClientSuite<Chain>,
}

impl<Chain: CwEnv> TerpArtifactStack<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            orch: TerpOrchSuite::new(chain.clone()),
            authenticators: TerpAuthenticatorSuite::new(chain.clone()),
            accounts: TerpAccountSuite::new(chain.clone()),
            light_clients: TerpLightClientSuite::new(chain),
        }
    }

    /// `(contract id, code id, address)` after a successful [`Self::deploy_on`].
    pub fn bound(&self) -> Result<Vec<(&'static str, u64, String)>, CwOrchError> {
        let mut rows = Vec::new();
        let mut push = |id: &'static str, code_id: Result<u64, CwOrchError>, addr: Result<String, CwOrchError>| -> Result<(), CwOrchError> {
            rows.push((id, code_id?, addr?));
            Ok(())
        };
        push("cw-zk-verify-demo", self.orch.circuits.verify.code_id(), self.orch.circuits.verify.addr_str())?;
        if let Some(bridge) = &self.orch.bridge {
            push("cw-private-dex", bridge.dex.code_id(), bridge.dex.addr_str())?;
        }
        let auth = &self.authenticators;
        push("terp-passkey", auth.passkey.code_id(), auth.passkey.addr_str())?;
        push("terp-recovery", auth.recovery.code_id(), auth.recovery.addr_str())?;
        push("terp-ed25519", auth.ed25519.code_id(), auth.ed25519.addr_str())?;
        push("terp-eth", auth.eth.code_id(), auth.eth.addr_str())?;
        push("terp-zkjwt", auth.zk_jwt.code_id(), auth.zk_jwt.addr_str())?;
        push("terp-vsck", auth.vsck.code_id(), auth.vsck.addr_str())?;
        let accounts = &self.accounts;
        push("terp-dao", accounts.dao.code_id(), accounts.dao.addr_str())?;
        push("terp-dregg", accounts.dregg.code_id(), accounts.dregg.addr_str())?;
        push("terp-frost", accounts.frost.code_id(), accounts.frost.addr_str())?;
        push("terp-bls", accounts.bls.code_id(), accounts.bls.addr_str())?;
        push("terp-zktls", accounts.zktls.code_id(), accounts.zktls.addr_str())?;
        push("polytone-any-proxy", accounts.polytone.code_id(), accounts.polytone.addr_str())?;
        let light = &self.light_clients;
        push("cw-ics08-wasm-crosslink", light.crosslink.code_id(), light.crosslink.addr_str())?;
        push("cw-ics08-wasm-dregg", light.dregg.code_id(), light.dregg.addr_str())?;
        push("cw-ics08-wasm-terp", light.terp.code_id(), light.terp.addr_str())?;
        push("cw-ics08-wasm-zap1", light.zap1.code_id(), light.zap1.addr_str())?;
        Ok(rows)
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for TerpArtifactStack<Chain> {
    type Error = CwOrchError;
    type DeployData = TerpArtifactStackDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let stack = Self::new(chain);
        stack.orch.circuits.verify.upload()?;
        if let Some(bridge) = &stack.orch.bridge {
            bridge.dex.upload()?;
        }
        stack.authenticators.upload_all()?;
        stack.accounts.upload()?;
        stack.light_clients.upload()?;
        Ok(stack)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        let TerpArtifactStack {
            orch,
            authenticators,
            accounts,
            light_clients,
        } = self;
        let mut v = orch.get_contracts_mut();
        v.extend(authenticators.get_contracts_mut());
        let rest: Vec<Box<&mut dyn ContractInstance<Chain>>> = vec![
            Box::new(&mut accounts.dao),
            Box::new(&mut accounts.dregg),
            Box::new(&mut accounts.frost),
            Box::new(&mut accounts.bls),
            Box::new(&mut accounts.zktls),
            Box::new(&mut accounts.polytone),
            Box::new(&mut light_clients.crosslink),
            Box::new(&mut light_clients.dregg),
            Box::new(&mut light_clients.terp),
            Box::new(&mut light_clients.zap1),
        ];
        v.extend(rest);
        v
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let admin = data.admin.clone();
        let orch = TerpOrchSuite::deploy_on(
            chain.clone(),
            TerpOrchDeployData {
                admin: admin.clone(),
                circuit_zkid: 7,
                attach_bridge: true,
                bridge_mock_verify: true,
                bridge_zkid: None,
            },
        )?;
        let authenticators = TerpAuthenticatorSuite::deploy_on(
            chain.clone(),
            TerpAuthenticatorDeployData::from(admin.clone()),
        )?;
        let accounts = TerpAccountSuite::new(chain.clone());
        accounts.upload()?;
        accounts.instantiate_lab(&admin)?;
        let light_clients = TerpLightClientSuite::new(chain);
        light_clients.upload()?;
        light_clients.instantiate_lab(&admin)?;
        Ok(Self {
            orch,
            authenticators,
            accounts,
            light_clients,
        })
    }
}

fn crosslink_instantiate() -> cw_ics08_wasm_crosslink::msg::InstantiateMsg {
    use crosslink_light_client::{
        Blake3Hash, ClientState, ConsensusState, FinalizerEntry, ZcashSerialize, PROTOTYPE_PARAMETERS,
    };
    let client = ClientState::new(
        PROTOTYPE_PARAMETERS,
        Blake3Hash([1u8; 32]),
        1,
        1,
        [2u8; 32],
        vec![FinalizerEntry::new([3u8; 32], 1)],
    );
    let consensus = ConsensusState::v1(1, 1, [2u8; 32], 1, [4u8; 32]);
    cw_ics08_wasm_crosslink::msg::InstantiateMsg {
        client_state: Binary::from(client.zcash_serialize_to_vec().expect("client bytes")),
        consensus_state: Binary::from(consensus.zcash_serialize_to_vec().expect("consensus bytes")),
        checksum: Binary::from(vec![9u8; 32]),
    }
}

fn dregg_instantiate() -> cw_ics08_wasm_dregg::msg::InstantiateMsg {
    use cw_ics08_wasm_dregg::types::{ClaimCeiling, ClientState, ConsensusKind, ConsensusState, TrustLevel};
    let lanes = vec![1, 2, 3, 4, 5, 6, 7, 8];
    cw_ics08_wasm_dregg::msg::InstantiateMsg {
        client_state: ClientState {
            client_schema_version: 1,
            chain_id: "dregg-lab".into(),
            federation_id: None,
            trust_level: TrustLevel::SettlementWrapOnly,
            settlement_vk_hash: "11".repeat(32),
            genesis_lanes: lanes.clone(),
            latest_height: 0,
            frozen_height: None,
            allow_gate_off_finality: false,
            claim_ceiling: ClaimCeiling::SettlementOnly,
            auth_min_proven_height: None,
            auth_require_root_in_history: false,
        },
        consensus_state: ConsensusState {
            version: 1,
            kind: ConsensusKind::B,
            proven_lanes: lanes,
            proven_root_key: String::new(),
            proven_height: 0,
        },
        checksum: None,
    }
}

fn terp_iavl_instantiate() -> cw_ics08_wasm_terp::msg::InstantiateMsg {
    let client = br#"{"chain_id":"terp-local-1","latest_height":{"revision_number":0,"revision_height":1},"hasher_mode":"hybrid","frozen":false}"#;
    let consensus = br#"{"timestamp_ns":1,"root_hex":"aa"}"#;
    cw_ics08_wasm_terp::msg::InstantiateMsg {
        client_state: Binary::from(client.as_slice()),
        consensus_state: Binary::from(consensus.as_slice()),
        checksum: Binary::from(vec![0u8; 32]),
    }
}
