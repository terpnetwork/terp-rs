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

use cosmwasm_std::{Addr, Binary, Coin, Decimal};
use cw_orch::prelude::*;
use hm_loyalty_whitelist::WhitelistConfig;
use hm_nft_oracle::PricePolicy;
use terp_authenticator_suite::{TerpAuthenticatorDeployData, TerpAuthenticatorSuite};

use crate::interfaces::accounts::{PolytoneAnyProxy, TerpBls, TerpDao, TerpDregg, TerpFrost, TerpZkTls};
use crate::interfaces::products::{
    ave_msg, headstash_msg, marketplace_msg, reserve_auction_msg, shitstrap_msg, svg_auction_msg, Cw420,
    CwAve, CwAveFactory, CwHeadstash, CwHeadstashManifold, CwNorick, CwNullifierAuth, CwNullifierGate,
    CwRegEligibility, CwShitstrap, CwShitstrapFactory, CwVoteAnchor, CwVoteCeremony, NftfiLoan, NftfiRaffle,
    RevenueMarketplace, RevenueReserveAuction, SvgAuction,
};
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

/// Product contracts merged into this workspace (shitstrap, nftfi, revenue, ave, headstash, svg auction).
pub struct TerpProductSuite<Chain: CwEnv> {
    pub shitstrap: CwShitstrap<Chain>,
    pub shitstrap_factory: CwShitstrapFactory<Chain>,
    pub raffle: NftfiRaffle<Chain>,
    pub loan: NftfiLoan<Chain>,
    pub marketplace: RevenueMarketplace<Chain>,
    pub reserve_auction: RevenueReserveAuction<Chain>,
    pub cw420: Cw420<Chain>,
    pub ave_factory: CwAveFactory<Chain>,
    pub ave: CwAve<Chain>,
    pub headstash: CwHeadstash<Chain>,
    pub manifold: CwHeadstashManifold<Chain>,
    pub norick: CwNorick<Chain>,
    pub nullifier_auth: CwNullifierAuth<Chain>,
    pub nullifier_gate: CwNullifierGate<Chain>,
    pub vote_anchor: CwVoteAnchor<Chain>,
    pub vote_ceremony: CwVoteCeremony<Chain>,
    pub reg_eligibility: CwRegEligibility<Chain>,
    pub svg_auction: SvgAuction<Chain>,
    pub oracle_aggregator: cw_oracle_aggregator::OracleAggregator<Chain>,
    pub oracle_twap_probe: cw_oracle_twap_probe::OracleTwapProbe<Chain>,
    pub marketplace_mirror: marketplace_mirror::MarketplaceMirror<Chain>,
    pub loyalty_verifier: loyalty_verifier::LoyaltyVerifier<Chain>,
    pub trusted_builder: trusted_builder_sbt::TrustedBuilderSbt<Chain>,
    pub cashu_registry: cw_cashu_registry::CwCashuRegistry<Chain>,
    pub hm_nft_minter: hm_nft_minter::HmNftMinter<Chain>,
}

impl<Chain: CwEnv> TerpProductSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            shitstrap: CwShitstrap::new(chain.clone()),
            shitstrap_factory: CwShitstrapFactory::new(chain.clone()),
            raffle: NftfiRaffle::new(chain.clone()),
            loan: NftfiLoan::new(chain.clone()),
            marketplace: RevenueMarketplace::new(chain.clone()),
            reserve_auction: RevenueReserveAuction::new(chain.clone()),
            cw420: Cw420::new(chain.clone()),
            ave_factory: CwAveFactory::new(chain.clone()),
            ave: CwAve::new(chain.clone()),
            headstash: CwHeadstash::new(chain.clone()),
            manifold: CwHeadstashManifold::new(chain.clone()),
            norick: CwNorick::new(chain.clone()),
            nullifier_auth: CwNullifierAuth::new(chain.clone()),
            nullifier_gate: CwNullifierGate::new(chain.clone()),
            vote_anchor: CwVoteAnchor::new(chain.clone()),
            vote_ceremony: CwVoteCeremony::new(chain.clone()),
            reg_eligibility: CwRegEligibility::new(chain.clone()),
            svg_auction: SvgAuction::new(chain.clone()),
            oracle_aggregator: cw_oracle_aggregator::OracleAggregator::new(chain.clone()),
            oracle_twap_probe: cw_oracle_twap_probe::OracleTwapProbe::new(chain.clone()),
            marketplace_mirror: marketplace_mirror::MarketplaceMirror::new(chain.clone()),
            loyalty_verifier: loyalty_verifier::LoyaltyVerifier::new(chain.clone()),
            trusted_builder: trusted_builder_sbt::TrustedBuilderSbt::new(chain.clone()),
            cashu_registry: cw_cashu_registry::CwCashuRegistry::new(chain.clone()),
            hm_nft_minter: hm_nft_minter::HmNftMinter::new(chain),
        }
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.shitstrap.upload()?;
        self.shitstrap_factory.upload()?;
        self.raffle.upload()?;
        self.loan.upload()?;
        self.marketplace.upload()?;
        self.reserve_auction.upload()?;
        self.cw420.upload()?;
        self.ave_factory.upload()?;
        self.ave.upload()?;
        self.headstash.upload()?;
        self.manifold.upload()?;
        self.norick.upload()?;
        self.nullifier_auth.upload()?;
        self.nullifier_gate.upload()?;
        self.vote_anchor.upload()?;
        self.vote_ceremony.upload()?;
        self.reg_eligibility.upload()?;
        self.svg_auction.upload()?;
        self.oracle_aggregator.upload()?;
        self.oracle_twap_probe.upload()?;
        self.marketplace_mirror.upload()?;
        self.loyalty_verifier.upload()?;
        self.trusted_builder.upload()?;
        self.cashu_registry.upload()?;
        self.hm_nft_minter.upload()?;
        Ok(())
    }

    pub fn instantiate_lab(&self, admin: &Addr) -> Result<(), CwOrchError> {
        let admin_s = admin.to_string();
        self.shitstrap.instantiate(&shitstrap_msg(&admin_s), Some(admin), &[])?;
        self.shitstrap_factory.instantiate(
            &cw_shitstrap_factory::msg::InstantiateMsg {
                owner: Some(admin_s.clone()),
                shitstrap_id: self.shitstrap.code_id()?,
            },
            Some(admin),
            &[],
        )?;
        self.raffle.instantiate(
            &nftfi_raffle::InstantiateMsg {
                name: "lab".into(),
                owner: None,
                fee_addr: admin_s.clone(),
                minimum_raffle_duration: None,
                max_tickets_per_raffle: None,
                raffle_fee: Decimal::percent(1),
                creation_coins: None,
                randomness: nftfi_randomness::RandomnessSourceMsg::Seed { provider: None },
            },
            Some(admin),
            &[],
        )?;
        self.loan.instantiate(
            &nftfi_loan::InstantiateMsg {
                name: "lab".into(),
                owner: None,
                treasury_addr: admin_s.clone(),
                fee_rate: Decimal::percent(1),
                listing_fee_coins: None,
            },
            Some(admin),
            &[],
        )?;
        self.marketplace.instantiate(&marketplace_msg(&admin_s), Some(admin), &[])?;
        self.reserve_auction
            .instantiate(&reserve_auction_msg(&admin_s), Some(admin), &[])?;
        self.cw420.instantiate(
            &cw420::msg::InstantiateMsg {
                admin: Some(admin_s.clone()),
                members: vec![],
            },
            Some(admin),
            &[],
        )?;
        self.ave_factory.instantiate(
            &cw_ave_factory::msg::InstantiateMsg {
                owner: Some(admin_s.clone()),
                cw_ave_id: self.ave.code_id()?,
            },
            Some(admin),
            &[Coin::new(1_000_000u128, "uthiol")],
        )?;
        self.ave
            .instantiate(&ave_msg(&admin_s, self.cw420.code_id()?), Some(admin), &[])?;
        self.headstash.instantiate(
            &headstash_msg(),
            Some(admin),
            &[Coin::new(1000u128, "existing_token")],
        )?;
        self.manifold.instantiate(
            &cw_headstash_manifold::msg::InstantiateMsg {
                owner: Some(admin_s.clone()),
                headstash_code_id: self.headstash.code_id()?,
            },
            Some(admin),
            &[],
        )?;
        // cw-norick instantiate emits tokenfactory MsgCreateDenom. The cw-orch
        // mock router rejects Any/tokenfactory messages. Upload still stores
        // the code; a chain with tokenfactory instantiates it.
        self.nullifier_auth.instantiate(
            &cw_nullifier_auth::msg::InstantiateMsg { admin: Some(admin_s.clone()) },
            Some(admin),
            &[],
        )?;
        self.nullifier_gate
            .instantiate(&cw_nullifier_gate::msg::InstantiateMsg {}, Some(admin), &[])?;
        self.vote_anchor.instantiate(
            &cw_vote_anchor::InstantiateMsg { admin: Some(admin_s.clone()) },
            Some(admin),
            &[],
        )?;
        self.vote_ceremony.instantiate(
            &cw_vote_ceremony::InstantiateMsg {
                admin: Some(admin_s.clone()),
                max_nullifiers_per_mark: 8,
            },
            Some(admin),
            &[],
        )?;
        self.reg_eligibility.instantiate(
            &cw_reg_eligibility::msg::InstantiateMsg {
                accepted_proof: Binary::from(b"lab".as_slice()),
                dummy_account: None,
            },
            Some(admin),
            &[],
        )?;
        self.svg_auction
            .instantiate(&svg_auction_msg(&admin_s), Some(admin), &[])?;
        self.oracle_aggregator.instantiate(
            &cw_oracle_aggregator::InstantiateMsg {
                registry: admin_s.clone(),
            },
            Some(admin),
            &[],
        )?;
        self.oracle_twap_probe.instantiate(
            &cw_oracle_twap_probe::InstantiateMsg { mid: "1.0".into() },
            Some(admin),
            &[],
        )?;
        self.marketplace_mirror.instantiate(
            &marketplace_mirror::InstantiateMsg {
                admin: admin_s.clone(),
                min_buffer_blocks: 7,
                accepted_denom: "uterp".into(),
                protocol_fee_bps: None,
                auto_execute_ingress: false,
            },
            Some(admin),
            &[],
        )?;
        self.loyalty_verifier.instantiate(
            &loyalty_verifier::msg::InstantiateMsg {
                reward_denom: None,
                mint_enabled: false,
            },
            Some(admin),
            &[],
        )?;
        self.trusted_builder.instantiate(
            &trusted_builder_sbt::InstantiateMsg {
                admin: admin_s.clone(),
                manifesto_contract: "manifesto".into(),
                ibc2_source_client: None,
                ibc2_dest_port: None,
            },
            Some(admin),
            &[],
        )?;
        self.cashu_registry.instantiate(
            &cw_cashu_registry::InstantiateMsg {
                admin: Some(admin_s.clone()),
                max_metadata_bytes: None,
                allow_public_register: false,
            },
            Some(admin),
            &[],
        )?;
        self.hm_nft_minter.instantiate(
            &hm_nft_minter::InstantiateMsg {
                owner: Some(admin_s),
                cw721_address: None,
                max_tokens: 8,
                token_uri_prefix: None,
                whitelist: WhitelistConfig::default(),
                price: PricePolicy::default(),
                proceeds: None,
                paused: Some(true),
            },
            Some(admin),
            &[],
        )?;
        Ok(())
    }
}

/// Every optimized artifact: authenticators, verify demo, private dex, accounts, light clients, products.
pub struct TerpArtifactStack<Chain: CwEnv> {
    pub orch: TerpOrchSuite<Chain>,
    pub authenticators: TerpAuthenticatorSuite<Chain>,
    pub accounts: TerpAccountSuite<Chain>,
    pub light_clients: TerpLightClientSuite<Chain>,
    pub products: TerpProductSuite<Chain>,
}

impl<Chain: CwEnv> TerpArtifactStack<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            orch: TerpOrchSuite::new(chain.clone()),
            authenticators: TerpAuthenticatorSuite::new(chain.clone()),
            accounts: TerpAccountSuite::new(chain.clone()),
            light_clients: TerpLightClientSuite::new(chain.clone()),
            products: TerpProductSuite::new(chain),
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
        let products = &self.products;
        push("cw-shitstrap", products.shitstrap.code_id(), products.shitstrap.addr_str())?;
        push("cw-shitstrap-factory", products.shitstrap_factory.code_id(), products.shitstrap_factory.addr_str())?;
        push("nftfi-raffle", products.raffle.code_id(), products.raffle.addr_str())?;
        push("nftfi-loan", products.loan.code_id(), products.loan.addr_str())?;
        push("revenue-marketplace", products.marketplace.code_id(), products.marketplace.addr_str())?;
        push("revenue-reserve-auction", products.reserve_auction.code_id(), products.reserve_auction.addr_str())?;
        push("cw420", products.cw420.code_id(), products.cw420.addr_str())?;
        push("cw-ave-factory", products.ave_factory.code_id(), products.ave_factory.addr_str())?;
        push("cw-ave", products.ave.code_id(), products.ave.addr_str())?;
        push("cw-headstash", products.headstash.code_id(), products.headstash.addr_str())?;
        push("cw-headstash-manifold", products.manifold.code_id(), products.manifold.addr_str())?;
        push("cw-nullifier-auth", products.nullifier_auth.code_id(), products.nullifier_auth.addr_str())?;
        push("cw-nullifier-gate", products.nullifier_gate.code_id(), products.nullifier_gate.addr_str())?;
        push("cw-vote-anchor", products.vote_anchor.code_id(), products.vote_anchor.addr_str())?;
        push("cw-vote-ceremony", products.vote_ceremony.code_id(), products.vote_ceremony.addr_str())?;
        push("cw-reg-eligibility", products.reg_eligibility.code_id(), products.reg_eligibility.addr_str())?;
        push("svg-auction", products.svg_auction.code_id(), products.svg_auction.addr_str())?;
        push("cw-oracle-aggregator", products.oracle_aggregator.code_id(), products.oracle_aggregator.addr_str())?;
        push("cw-oracle-twap-probe", products.oracle_twap_probe.code_id(), products.oracle_twap_probe.addr_str())?;
        push("marketplace-mirror", products.marketplace_mirror.code_id(), products.marketplace_mirror.addr_str())?;
        push("loyalty-verifier", products.loyalty_verifier.code_id(), products.loyalty_verifier.addr_str())?;
        push("trusted-builder-sbt", products.trusted_builder.code_id(), products.trusted_builder.addr_str())?;
        push("cw-cashu-registry", products.cashu_registry.code_id(), products.cashu_registry.addr_str())?;
        push("hm-nft-minter", products.hm_nft_minter.code_id(), products.hm_nft_minter.addr_str())?;
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
        stack.products.upload()?;
        Ok(stack)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        let TerpArtifactStack {
            orch,
            authenticators,
            accounts,
            light_clients,
            products,
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
            Box::new(&mut products.shitstrap),
            Box::new(&mut products.shitstrap_factory),
            Box::new(&mut products.raffle),
            Box::new(&mut products.loan),
            Box::new(&mut products.marketplace),
            Box::new(&mut products.reserve_auction),
            Box::new(&mut products.cw420),
            Box::new(&mut products.ave_factory),
            Box::new(&mut products.ave),
            Box::new(&mut products.headstash),
            Box::new(&mut products.manifold),
            Box::new(&mut products.norick),
            Box::new(&mut products.nullifier_auth),
            Box::new(&mut products.nullifier_gate),
            Box::new(&mut products.vote_anchor),
            Box::new(&mut products.vote_ceremony),
            Box::new(&mut products.reg_eligibility),
            Box::new(&mut products.svg_auction),
            Box::new(&mut products.oracle_aggregator),
            Box::new(&mut products.oracle_twap_probe),
            Box::new(&mut products.marketplace_mirror),
            Box::new(&mut products.loyalty_verifier),
            Box::new(&mut products.trusted_builder),
            Box::new(&mut products.cashu_registry),
            Box::new(&mut products.hm_nft_minter),
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
        let light_clients = TerpLightClientSuite::new(chain.clone());
        light_clients.upload()?;
        light_clients.instantiate_lab(&admin)?;
        let products = TerpProductSuite::new(chain);
        products.upload()?;
        products.instantiate_lab(&admin)?;
        Ok(Self {
            orch,
            authenticators,
            accounts,
            light_clients,
            products,
        })
    }
}

/// Compile each optimized artifact with cosmwasm-vm and instantiate it.
/// `mock_instance` runs `check_wasm` and the wasmer engine whose compiler
/// installs the Gatekeeper and metering middleware, then `instantiate` executes
/// inside that instance.
pub fn wasmer_upload_and_instantiate(admin_label: &str) {
    use cosmwasm_std::ContractResult;
    use cosmwasm_vm::testing::{
        instantiate, mock_env, mock_info, mock_instance_with_options, MockApi, MockInstanceOptions,
    };
    use cosmwasm_vm::CAP_BULK_MEMORY;

    let admin = MockApi::default().addr_make(admin_label);
    fn init<M: serde::Serialize>(admin: &str, name: &str, msg: &M) {
        let bytes = std::fs::read(crate::interfaces::wasm_path::wasm(name).path())
            .unwrap_or_else(|err| panic!("read {name}.wasm: {err}"));
        // Optimizer artifacts keep bulk-memory. The stock mock capability set does not.
        let mut options = MockInstanceOptions::default();
        options
            .available_capabilities
            .insert(CAP_BULK_MEMORY.to_string());
        let mut instance = mock_instance_with_options(&bytes, options);
        match instantiate::<_, _, _, _, cosmwasm_std::Empty>(
            &mut instance,
            mock_env(),
            mock_info(admin, &[]),
            msg,
        ) {
            ContractResult::Ok(_) => {}
            ContractResult::Err(err) => panic!("{name} wasmer instantiate: {err}"),
        }
    }
    init(
        &admin,
        "cw_oracle_aggregator",
        &cw_oracle_aggregator::InstantiateMsg {
            registry: admin.clone(),
        },
    );
    init(
        &admin,
        "cw_oracle_twap_probe",
        &cw_oracle_twap_probe::InstantiateMsg { mid: "1.0".into() },
    );
    init(
        &admin,
        "marketplace_mirror",
        &marketplace_mirror::InstantiateMsg {
            admin: admin.clone(),
            min_buffer_blocks: 7,
            accepted_denom: "uterp".into(),
            protocol_fee_bps: None,
            auto_execute_ingress: false,
        },
    );
    init(
        &admin,
        "loyalty_verifier",
        &loyalty_verifier::msg::InstantiateMsg {
            reward_denom: None,
            mint_enabled: false,
        },
    );
    init(
        &admin,
        "trusted_builder_sbt",
        &trusted_builder_sbt::InstantiateMsg {
            admin: admin.clone(),
            manifesto_contract: "manifesto".into(),
            ibc2_source_client: None,
            ibc2_dest_port: None,
        },
    );
    init(
        &admin,
        "cw_cashu_registry",
        &cw_cashu_registry::InstantiateMsg {
            admin: Some(admin.clone()),
            max_metadata_bytes: None,
            allow_public_register: false,
        },
    );
    init(
        &admin,
        "hm_nft_minter",
        &hm_nft_minter::InstantiateMsg {
            owner: Some(admin.clone()),
            cw721_address: None,
            max_tokens: 8,
            token_uri_prefix: None,
            whitelist: WhitelistConfig::default(),
            price: PricePolicy::default(),
            proceeds: None,
            paused: Some(true),
        },
    );
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
