use abstract_interface::{AbstractIbc, AnsHost, ModuleFactory, Registry};
use abstract_std::{ANS_HOST, MODULE_FACTORY, REGISTRY};
use cosmwasm_std::{coin, Uint128};
use cw_orch::prelude::*;
// use dao_testing::DaoDaoSuite;
use terp721_account::{interface::TerpAccountCollection, ACCOUNT_CONTRACT};
use terp721_account_manifold::{contract::ACCOUNT_MANIFOLD_CONTRACT, interface::TerpAccountMinter};
use terp_account::{Metadata, CURRENT_BASE_PRICE, CURRENT_COOLDOWN_FEE, CURRENT_MINIMUM_BID_PRICE};

#[derive(Clone, Debug)]
pub struct TerpAccountDeployData {
    pub admin: Addr,
}

impl From<Addr> for TerpAccountDeployData {
    fn from(value: Addr) -> Self {
        Self { admin: value }
    }
}

pub struct TerpAccountSuite<Chain>
where
    Chain: cw_orch::prelude::CwEnv,
{
    // terp-account
    pub nft: TerpAccountCollection<Chain, Metadata>,
    pub manifold: TerpAccountMinter<Chain>,
    // // dao stuff
    // pub dao: Option<DaoDaoSuite<Chain>>,
    // abstract accounts
    // pub(crate) test_owner: TestingOwnershipVerifier<Chain>,
    // pub(crate) account: AccountI<Chain>,
    // pub(crate) blob: CwBlob<Chain>,
    pub ans_host: AnsHost<Chain>,
    pub registry: Registry<Chain>,
    pub module_factory: ModuleFactory<Chain>,
    pub ibc: AbstractIbc<Chain>,
}

pub const BLS_PUBKEY: &str = "";
pub const CW_BLOB: &str = "cw:blob";

impl<Chain: CwEnv> TerpAccountSuite<Chain> {
    pub fn new(chain: Chain) -> TerpAccountSuite<Chain> {
        TerpAccountSuite::<Chain> {
            nft: TerpAccountCollection::new(ACCOUNT_CONTRACT, chain.clone()),
            manifold: TerpAccountMinter::new(ACCOUNT_MANIFOLD_CONTRACT, chain.clone()),
            // test_owner: TestingOwnershipVerifier::new("ownership_verifier", chain.clone()),
            ans_host: AnsHost::new(ANS_HOST, chain.clone()),
            registry: Registry::new(REGISTRY, chain.clone()),
            module_factory: ModuleFactory::new(MODULE_FACTORY, chain.clone()),
            ibc: AbstractIbc::new(&chain),
            // account: AccountI::new(ACCOUNT, chain.clone()),
            // blob: CwBlob::new(CW_BLOB, chain.clone()),
        }
    }

    pub fn upload(&self) -> Result<(), CwOrchError> {
        let acc_code_id = self.nft.upload()?.uploaded_code_id()?;
        let minter_code_id = self.manifold.upload()?.uploaded_code_id()?;
        println!("{}: {}", self.nft.id(), acc_code_id);
        println!("{}: {}", self.manifold.id(), minter_code_id);
        Ok(())
    }
}

// Terp Accounts `Deploy` Suite
impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for TerpAccountSuite<Chain> {
    // We don't have a custom error type
    type Error = CwOrchError;
    type DeployData = TerpAccountDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = TerpAccountSuite::new(chain.clone());
        suite.upload()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.nft), Box::new(&mut self.manifold)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain.clone());
        Ok(suite)
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        // ########### Upload ##############
        let suite: TerpAccountSuite<Chain> = TerpAccountSuite::store_on(chain.clone())?;

        // // // // // // // // // // // // // // // // // //
        //  THIOL ACCOUNT TOKENS
        // // // // // // // // // // // // // // // // // //
        let terp721_account = suite
            .manifold
            .instantiate(
                &terp_account::manifold::InstantiateMsg {
                    trading_fee_bps: 200,
                    min_price: Uint128::from(CURRENT_MINIMUM_BID_PRICE),
                    ask_interval: 60,
                    valid_bid_query_limit: 30,
                    cooldown_timeframe: 60 * 60 * 24 * 7u64, // 14 days
                    cooldown_cancel_fee: coin(CURRENT_COOLDOWN_FEE.into(), "uthiol"),
                    hooks_admin: None,
                    admin: Some(data.admin.to_string()),
                    verifier: None,
                    collection_code_id: suite.nft.code_id()?,
                    min_account_length: 3u32,
                    max_account_length: 128u32,
                    base_price: CURRENT_BASE_PRICE.into(),
                    base_delegation: 0u128.into(),
                    mint_start_delay: None,
                },
                Some(&Addr::unchecked(data.admin.to_string())),
                &[],
            )?
            .event_attr_value("wasm", "terp721_account_address")?;

        println!("terp721_account: {:#?}", terp721_account);
        println!("minter contract: {}", suite.manifold.addr_str()?);
        suite.nft.set_address(&Addr::unchecked(terp721_account));

        // println!("collection contract: {}", terp721_account);
        // let account = &Addr::unchecked(terp721_account);
        // suite.nft.set_default_address(&account);
        // suite.nft.set_address(&account);

        // suite.middleware.instantiate(
        //     &account_registry_middleware::InstantiateMsg {
        //         market: suite.manifold.addr_str()?,
        //         collection: suite.manifold.addr_str()?,
        //         account_code_id: suite.account.code_id()?,
        //     },
        //     Some(&Addr::unchecked(data.clone())),
        //     &[],
        // )?;

        // // // // // // // // // // // // // // // // // //
        //  ABSTRACT ACCOUNTS
        // // // // // // // // // // // // // // // // // //
        // let admin = chain.sender_addr().to_string();
        // let creator_account_id: cosmrs::AccountId = admin.as_str().parse().unwrap();
        // let canon_creator = CanonicalAddr::from(creator_account_id.to_bytes());
        // let blob_code_id = suite.blob.code_id()?;
        // let expected_addr = |salt: &[u8]| -> Result<CanonicalAddr, Instantiate2AddressError> {
        //     instantiate2_address(&cw_blob::CHECKSUM, &canon_creator, salt)
        // };
        // suite.ans_host.deterministic_instantiate(
        //     &abstract_std::ans_host::MigrateMsg::Instantiate(
        //         abstract_std::ans_host::InstantiateMsg {
        //             admin: admin.to_string(),
        //         },
        //     ),
        //     blob_code_id,
        //     expected_addr(native_addrs::ANS_HOST_SALT)?,
        //     Binary::from(native_addrs::ANS_HOST_SALT),
        // )?;

        // suite.registry.deterministic_instantiate(
        //     &abstract_std::registry::MigrateMsg::Instantiate(
        //         abstract_std::registry::InstantiateMsg {
        //             admin: suite.middleware.addr_str()?,
        //             security_enabled: Some(true),
        //             namespace_registration_fee: None,
        //         },
        //     ),
        //     blob_code_id,
        //     expected_addr(native_addrs::REGISTRY_SALT)?,
        //     Binary::from(native_addrs::REGISTRY_SALT),
        // )?;

        // suite.module_factory.deterministic_instantiate(
        //     &abstract_std::module_factory::MigrateMsg::Instantiate(
        //         abstract_std::module_factory::InstantiateMsg {
        //             admin: admin.to_string(),
        //         },
        //     ),
        //     blob_code_id,
        //     expected_addr(native_addrs::MODULE_FACTORY_SALT)?,
        //     Binary::from(native_addrs::MODULE_FACTORY_SALT),
        // )?;
        // // We also instantiate ibc contracts
        // suite.ibc.instantiate(&Addr::unchecked(admin.clone()))?;
        // suite
        //     .registry
        //     .register_base(&suite.account)
        //     .map_err(|e| CwOrchError::AnyError(anyhow!(e.to_string())))?;
        // suite
        //     .registry
        //     .approve_any_abstract_modules()
        //     .map_err(|e| CwOrchError::AnyError(anyhow!(e.to_string())))?;

        // suite
        //     .middleware
        //     .update_config(None, None, None, Some(suite.registry.addr_str()?))?;

        Ok(suite)
    }
}
