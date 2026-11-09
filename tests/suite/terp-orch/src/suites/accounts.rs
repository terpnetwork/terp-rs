use abstract_interface::{AbstractIbc, AnsHost, ModuleFactory, Registry};
use abstract_std::{ANS_HOST, MODULE_FACTORY, REGISTRY};
use cosmwasm_std::{Decimal, StakingMsg, Uint128, coin, coins};
use cw_orch::mock::cw_multi_test::{AppResponse, Module, StakingInfo};
use cw_orch::prelude::*;
// use dao_testing::DaoDaoSuite;
use terp_account::{CURRENT_BASE_PRICE, CURRENT_COOLDOWN_FEE, CURRENT_MINIMUM_BID_PRICE, Metadata};
use terp721_account::{ACCOUNT_CONTRACT, interface::TerpAccountCollection};
use terp721_account_manifold::{contract::ACCOUNT_MANIFOLD_CONTRACT, interface::TerpAccountMinter};

// re-export contract cw-orch functions & entrypoint types
pub use terp_account::manifold::{
    AsyncQueryMsgFns as TerpAccountMinterAsyncQueryMsgFns,
    ExecuteMsg as Bs721AccountMinterExecuteMsgTypes, ExecuteMsgFns as TerpAccountMinterExecuteFns,
    QueryMsg as TerpAccountMinterQueryMsgTypes, QueryMsgFns as TerpAccountMinterQueryMsgFns,
};
pub use terp721_account::msg::{
    AsyncTerp721AccountsQueryMsgFns, ExecuteMsg as Bs721AccountExecuteMsgTypes,
    ExecuteMsgFns as TerpAccountExecuteFns, Terp721AccountsQueryMsg, Terp721AccountsQueryMsgFns,
};

pub use terp_account::manifold::{
    AsyncQueryMsgFns as TerpAccountMarketAsyncQueryMsgFns,
    ExecuteMsg as Bs721AccountMarketExecuteMsgTypes, ExecuteMsgFns as TerpAccountMarketExecuteFns,
    QueryMsg as TerpAccountMarketQueryMsgTypes, QueryMsgFns as TerpAccountMarketQueryFns,
};

pub use ownership_verifier::{
    ExecuteMsg as TestOwnershipExecuteMsg, ExecuteMsgFns as TestOwnershipExecuteMsgFns,
    InstantiateMsg as TestOwnershipInitMsg, QueryMsg as TestOwnershipQueryMsg,
    QueryMsgFns as TestOwnershipQueryMsgFns,
};

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

const BASE_PRICE: u128 = 100_000_000;
const BASE_DELEGATION: u128 = 2_100_000_000;
const VALIDATOR_1: &str = "val-1";

impl TerpAccountSuite<MockBech32> {
    /// Upload and instantiate the collection the way the account tests do.
    pub fn default_setup(
        &mut self,
        mock: MockBech32,
        _creator: Option<Addr>,
        _admin: Option<Addr>,
    ) -> anyhow::Result<()> {
        self.upload()?;
        mock.add_balance(&mock.sender, vec![coin(10_500_000_000, "uthiol")])?;
        let admin = mock.sender_addr().to_string();
        let terp721_account = self
            .manifold
            .instantiate(
                &terp_account::manifold::InstantiateMsg {
                    trading_fee_bps: 0u64,
                    min_price: 100u128.into(),
                    ask_interval: 30u64,
                    valid_bid_query_limit: 100u32,
                    cooldown_timeframe: 60u64,
                    cooldown_cancel_fee: coin(500_000_000, "uthiol"),
                    hooks_admin: None,
                    admin: Some(admin.clone()),
                    verifier: Some(mock.addr_make("verifier").to_string()),
                    collection_code_id: self.nft.code_id()?,
                    min_account_length: 3u32,
                    max_account_length: 128u32,
                    base_price: BASE_PRICE.into(),
                    base_delegation: BASE_DELEGATION.into(),
                    mint_start_delay: Some(200u64),
                },
                None,
                &[],
            )?
            .event_attr_value("wasm", "terp721_account_address")
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        self.nft
            .set_default_address(&Addr::unchecked(terp721_account));
        let block_info = mock.block_info()?;
        mock.app.borrow_mut().init_modules(|router, api, storage| {
            router
                .staking
                .setup(
                    storage,
                    StakingInfo {
                        bonded_denom: "uthiol".into(),
                        unbonding_time: 69u64,
                        apr: Decimal::from_ratio(69u128, 100u128).into(),
                    },
                )
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            router
                .staking
                .add_validator(
                    api,
                    storage,
                    &block_info,
                    cosmwasm_std::Validator::create(
                        VALIDATOR_1.to_string(),
                        Decimal::from_ratio(1u128, 2u128),
                        Decimal::one(),
                        Decimal::one(),
                    ),
                )
                .map_err(|err| anyhow::anyhow!("{err}"))
        })?;
        self.delegate_to_val(mock.clone(), mock.sender.clone(), 10_500_000_000)?;
        Ok(())
    }

    pub fn delegate_to_val(
        &mut self,
        mock: MockBech32,
        delegator: Addr,
        amount: u128,
    ) -> anyhow::Result<()> {
        let block_info = mock.block_info()?;
        mock.app.borrow_mut().init_modules(|router, api, storage| {
            router
                .staking
                .execute(
                    api,
                    storage,
                    router,
                    &block_info,
                    delegator,
                    StakingMsg::Delegate {
                        validator: VALIDATOR_1.into(),
                        amount: coin(amount, "uthiol"),
                    },
                )
                .map_err(|err| anyhow::anyhow!("{err}"))
        })?;
        Ok(())
    }

    pub fn mint_and_list(
        &mut self,
        mock: MockBech32,
        account: &str,
        user: &Addr,
    ) -> anyhow::Result<AppResponse> {
        let market = self.manifold.address()?;
        self.nft.call_as(user).approve_all(market, None)?;
        let amount: Uint128 = (match account.len() {
            0..=2 => BASE_PRICE,
            3 => BASE_PRICE * 100,
            4 => BASE_PRICE * 10,
            _ => BASE_PRICE,
        })
        .into();
        let name_fee = coins(amount.u128(), "uthiol");
        if Uint128::from(BASE_PRICE) > Uint128::zero() {
            mock.add_balance(user, name_fee.clone())?;
        }
        self.manifold
            .call_as(user)
            .execute(
                &terp_account::manifold::ExecuteMsg::MintAndList {
                    account: account.to_string(),
                },
                &name_fee,
            )
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    pub fn owner_of(&self, id: String) -> anyhow::Result<String> {
        Ok(self.nft.owner_of(id, None)?.owner)
    }

    pub fn bid_w_funds(
        &self,
        mock: MockBech32,
        account: &str,
        bidder: Addr,
        amount: u128,
    ) -> anyhow::Result<()> {
        let bid_amnt = coins(amount, "uthiol");
        mock.add_balance(&bidder, bid_amnt.clone())?;
        self.manifold.call_as(&bidder).execute(
            &terp_account::manifold::ExecuteMsg::SetBid {
                token_id: account.into(),
            },
            &bid_amnt,
        )?;
        let res = self
            .manifold
            .bid(bidder.to_string(), account.into())?
            .unwrap();
        assert_eq!(res.token_id, account.to_string());
        assert_eq!(res.bidder, bidder);
        assert_eq!(res.amount, Uint128::from(amount));
        Ok(())
    }
}
