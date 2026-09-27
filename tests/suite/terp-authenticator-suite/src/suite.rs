//! Compose all priority authenticators under one `Deploy` suite.

use cosmwasm_std::{Addr, Binary};
use cw_orch::prelude::*;

use crate::interfaces::ed25519::TerpEd25519;
use crate::interfaces::eth::TerpEth;
use crate::interfaces::irl::TerpIrl;
use crate::interfaces::passkey::TerpPasskey;
use crate::interfaces::recovery::TerpRecovery;
use crate::interfaces::vsck::TerpVsck;
use crate::interfaces::zk_jwt::TerpZkJwt;
use crate::interfaces::zk_poseidon::TerpZkPoseidon;

#[derive(Clone, Debug)]
pub struct TerpAuthenticatorDeployData {
    pub admin: Addr,
    pub dao: Option<Addr>,
    /// Optional recovery break-glass hash algorithm (default: Sha256).
    /// Set to `Some(RecoveryHashAlg::PoseidonPallas)` to demo Poseidon digests.
    pub recovery_hash_alg: Option<terp_recovery::RecoveryHashAlg>,
    /// When true, zk-jwt Authenticate requires a prior RegisterClaim for the account.
    /// Default `false` keeps structural suite tests free of claim linking.
    pub require_registered_claim: bool,
    /// Ethereum address registered as eth authenticator signer (0x…, 40 hex).
    /// Defaults to the Hardhat/Anvil account #0 address used by eth personal_sign goldens.
    pub eth_signer: String,
}

impl From<Addr> for TerpAuthenticatorDeployData {
    fn from(admin: Addr) -> Self {
        Self {
            admin: admin.clone(),
            dao: Some(admin),
            recovery_hash_alg: None,
            require_registered_claim: false,
            eth_signer: ETH_GOLDEN_SIGNER.into(),
        }
    }
}

/// Hardhat/Anvil account #0 — pairs with suite eth personal_sign golden vectors.
pub const ETH_GOLDEN_SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

pub struct TerpAuthenticatorSuite<Chain: CwEnv> {
    pub passkey: TerpPasskey<Chain>,
    pub recovery: TerpRecovery<Chain>,
    pub ed25519: TerpEd25519<Chain>,
    pub eth: TerpEth<Chain>,
    pub irl: TerpIrl<Chain>,
    pub zk_jwt: TerpZkJwt<Chain>,
    pub zk_poseidon: TerpZkPoseidon<Chain>,
    pub vsck: TerpVsck<Chain>,
}

impl<Chain: CwEnv> TerpAuthenticatorSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            passkey: TerpPasskey::new(chain.clone()),
            recovery: TerpRecovery::new(chain.clone()),
            ed25519: TerpEd25519::new(chain.clone()),
            eth: TerpEth::new(chain.clone()),
            irl: TerpIrl::new(chain.clone()),
            zk_jwt: TerpZkJwt::new(chain.clone()),
            zk_poseidon: TerpZkPoseidon::new(chain.clone()),
            vsck: TerpVsck::new(chain),
        }
    }

    /// Store only the non-circuit product authenticators (passkey, recovery, ed25519, eth).
    pub fn upload_noncircuit_priority(&self) -> Result<(), CwOrchError> {
        self.passkey.upload()?;
        self.recovery.upload()?;
        self.ed25519.upload()?;
        self.eth.upload()?;
        Ok(())
    }

    pub fn upload_all(&self) -> Result<(), CwOrchError> {
        self.passkey.upload()?;
        self.recovery.upload()?;
        self.ed25519.upload()?;
        self.eth.upload()?;
        self.irl.upload()?;
        self.zk_jwt.upload()?;
        self.zk_poseidon.upload()?;
        self.vsck.upload()?;
        Ok(())
    }

    /// Store wasm only when this Daemon has no code_id for the contract id.
    /// This is the resume path when authenticators already exist on-chain.
    pub fn upload_if_needed(&self) -> Result<(), CwOrchError> {
        self.passkey.upload_if_needed()?;
        self.recovery.upload_if_needed()?;
        self.ed25519.upload_if_needed()?;
        self.eth.upload_if_needed()?;
        self.irl.upload_if_needed()?;
        self.zk_jwt.upload_if_needed()?;
        self.zk_poseidon.upload_if_needed()?;
        self.vsck.upload_if_needed()?;
        Ok(())
    }

    /// Code ids currently bound on this handle (state.json / Mock).
    pub fn bound_code_ids(&self) -> Vec<(&'static str, Option<u64>)> {
        vec![
            ("terp-passkey", self.passkey.code_id().ok()),
            ("terp-recovery", self.recovery.code_id().ok()),
            ("terp-ed25519", self.ed25519.code_id().ok()),
            ("terp-eth", self.eth.code_id().ok()),
            ("terp-irl", self.irl.code_id().ok()),
            ("terp-zkjwt", self.zk_jwt.code_id().ok()),
            ("terp-zkposiedon", self.zk_poseidon.code_id().ok()),
            ("terp-vsck", self.vsck.code_id().ok()),
        ]
    }

    pub fn missing_code_ids(&self) -> Vec<&'static str> {
        self.bound_code_ids()
            .into_iter()
            .filter_map(|(id, cid)| if cid.is_none() { Some(id) } else { None })
            .collect()
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for TerpAuthenticatorSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = TerpAuthenticatorDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![
            Box::new(&mut self.passkey),
            Box::new(&mut self.recovery),
            Box::new(&mut self.ed25519),
            Box::new(&mut self.eth),
            Box::new(&mut self.irl),
            Box::new(&mut self.zk_jwt),
            Box::new(&mut self.zk_poseidon),
            Box::new(&mut self.vsck),
        ]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        // Contract ids (terp-passkey, …) hydrate from Daemon state.json when present.
        // Missing ids stay unbound — caller uses missing_code_ids() / upload_if_needed().
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        let admin = &data.admin;
        let dao = data.dao.as_ref().unwrap_or(admin);

        suite.passkey.instantiate(
            &terp_passkey::InstantiateMsg {
                owner: Some(admin.to_string()),
                registration: terp_passkey::PasskeyRegistration {
                    origin: Some("https://app.terp.network".into()),
                    credential_id: Binary::from(b"cred-id-1"),
                },
            },
            Some(admin),
            &[],
        )?;

        suite.recovery.instantiate(
            &terp_recovery::InstantiateMsg {
                config: terp_recovery::RecoveryConfig {
                    guardians: vec![terp_recovery::GuardianConfig {
                        address: admin.to_string(),
                        pubkey: None,
                        commitment: None,
                    }],
                    threshold: 1,
                    hash_alg: data
                        .recovery_hash_alg
                        .unwrap_or(terp_recovery::RecoveryHashAlg::Sha256),
                    rehash_rounds: 1,
                    domain: terp_recovery::DEFAULT_DOMAIN.into(),
                    allow_address_only_approvals: true,
                },
            },
            Some(admin),
            &[],
        )?;

        suite.ed25519.instantiate(
            &terp_ed25519::InstantiateMsg {
                owner: Some(admin.to_string()),
                pubkey: Binary::from([1u8; 32]),
            },
            Some(admin),
            &[],
        )?;

        suite.eth.instantiate(
            &terp_eth::InstantiateMsg {
                owner: Some(admin.to_string()),
                signer: data.eth_signer.clone(),
            },
            Some(admin),
            &[],
        )?;

        suite.irl.instantiate(
            &terp_irl::InstantiateMsg {
                epoch_root: Binary::from(b"epoch-root-v1"),
            },
            Some(admin),
            &[],
        )?;

        suite.zk_jwt.instantiate(
            &terp_zkjwt::InstantiateMsg {
                admin: Some(admin.to_string()),
                issuers: vec![terp_zkjwt::IssuerConfig {
                    issuer: "https://accounts.example.com".into(),
                    verifying_key: Binary::from(b"test-issuer-vk"),
                    // zkid for host proof_instance_verify when feature zk-host is on
                    zkid: Some(1),
                    audience: Some("terp-app".into()),
                    // Headscale-style membership epoch root (demo constant)
                    inclusion_set_root: Some(Binary::from([0xABu8; 32])),
                }],
                require_registered_claim: data.require_registered_claim,
            },
            Some(admin),
            &[],
        )?;

        suite.zk_poseidon.instantiate(
            &terp_zkposiedon::InstantiateMsg {},
            Some(admin),
            &[],
        )?;

        suite.vsck.instantiate(
            &terp_vsck::InstantiateMsg {
                admin: Some(admin.to_string()),
                dao: dao.to_string(),
                circuit_vks: terp_vsck::CircuitVerifyingKeys {
                    delegation_vk: Binary::from(b"delegation-vk"),
                    vote_proof_vk: Binary::from(b"vote-proof-vk"),
                    share_reveal_vk: Binary::from(b"share-reveal-vk"),
                },
            },
            Some(admin),
            &[],
        )?;

        Ok(suite)
    }
}
