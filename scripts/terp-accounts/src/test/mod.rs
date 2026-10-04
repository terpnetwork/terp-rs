pub mod account;
pub mod market;
pub mod minter;
pub mod smart_accounts;

use cosmwasm_std::{
    coin, coins, instantiate2_address, CanonicalAddr, Decimal, Instantiate2AddressError,
    StakingMsg, Uint128, Uint256,
};
use cw_orch::{
    anyhow,
    mock::cw_multi_test::{AppResponse, Module, StakingInfo},
    prelude::*,
};
use terp_account::manifold::InstantiateMsg as AccountMinterInitMsg;

const BASE_PRICE: u128 = 100_000_000;
const BASE_DELEGATION: u128 = 2100000000;
const VALIDATOR_1: &str = "val-1";
use crate::{Terp721AccountsQueryMsgFns, TerpAccountExecuteFns, TerpAccountMarketQueryFns};
use serde::{Deserialize, Serialize};

use crate::TerpAccountSuite;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentStatus {
    pub chain_ids: Vec<String>,
    pub success: bool,
}

/// MockBech32 implementation for the Terp Account Suite.
impl TerpAccountSuite<MockBech32> {
    /// Creates intitial suite for testing
    pub fn default_setup(
        &mut self,
        mock: MockBech32,
        _creator: Option<Addr>,
        _admin: Option<Addr>,
    ) -> anyhow::Result<()> {
        self.upload()?;
        // self.test_owner.upload()?;
        // self.blob.upload()?;

        let _admin2 = mock.addr_make("admin2");
        mock.add_balance(&mock.sender, vec![coin(10500000000, "uthiol")])?;
        // let _blob_code_id = self.blob.code_id()?;
        let sender_addr = mock.sender_addr();
        let admin = sender_addr.to_string();
        let creator_account_id: cosmrs::AccountId = admin.as_str().parse().unwrap();
        let canon_creator = CanonicalAddr::from(creator_account_id.to_bytes());
        let _expected_addr = |salt: &[u8]| -> Result<CanonicalAddr, Instantiate2AddressError> {
            instantiate2_address(&cw_blob::CHECKSUM, &canon_creator, salt)
        };

        // a. uploads all contracts

        // b. instantiates marketplace
        let terp721_account = self
            .manifold
            .instantiate(
                &AccountMinterInitMsg {
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
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        // Account Minter
        // On instantitate, terp721-account contract is created by minter contract.
        // We grab this contract addr from response events, and set address in internal test suite state.

        self.nft
            .set_default_address(&Addr::unchecked(terp721_account));

        let block_info = mock.block_info()?;

        // create validator
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
                .map_err(|e| anyhow::anyhow!("{e}"))?;
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
                .map_err(|e| anyhow::anyhow!("{e}"))
        })?;

        // delgate some tokens to val one to satisfy delegation requirements
        // mock.wait_blocks(1)?;
        self.delegate_to_val(mock.clone(), mock.sender.clone(), 10500000000)?;

        // println!("TOKEN:   {:#?}", self.nft.addr_str()?);
        // println!("MARKET:  {:#?}", self.manifold.addr_str()?);
        // println!("MINTER:  {:#?}", self.manifold.addr_str()?);
        // println!("SENDER:  {:#?}", mock.sender_addr().to_string());
        // println!("ADMIN2:  {:#?}", admin2.to_string());
        // println!("ADMIN:   {:#?}", admin);
        // println!("CREATOR: {:#?}", creator);
        // // // // // // // // // // // // // // // // // //
        //  ABSTRACT ACCOUNTS
        // // // // // // // // // // // // // // // // // //

        // self.middleware.instantiate(
        //     &account_registry_middleware::InstantiateMsg {
        //         market: self.manifold.addr_str()?,
        //         collection: self.nft.addr_str()?,
        //         account_code_id: self.account.code_id()?,
        //     },
        //     Some(&Addr::unchecked(admin.clone())),
        //     &[],
        // )?;

        // self.ans_host.deterministic_instantiate(
        //     &abstract_std::ans_host::MigrateMsg::Instantiate(
        //         abstract_std::ans_host::InstantiateMsg {
        //             admin: admin.to_string(),
        //         },
        //     ),
        //     blob_code_id,
        //     expected_addr(native_addrs::ANS_HOST_SALT)?,
        //     Binary::from(native_addrs::ANS_HOST_SALT),
        // )?;

        // self.registry.deterministic_instantiate(
        //     &abstract_std::registry::MigrateMsg::Instantiate(
        //         abstract_std::registry::InstantiateMsg {
        //             admin: admin.to_string(),
        //             security_enabled: Some(false),
        //             namespace_registration_fee: None,
        //         },
        //     ),
        //     blob_code_id,
        //     expected_addr(native_addrs::REGISTRY_SALT)?,
        //     Binary::from(native_addrs::REGISTRY_SALT),
        // )?;

        // self.module_factory.deterministic_instantiate(
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
        // self.ibc.instantiate(&Addr::unchecked(admin.clone()))?;
        // self.registry
        //     .call_as(&Addr::unchecked(admin.clone()))
        //     .register_base(&self.account)
        //     .map_err(|e| CwOrchError::AnyError(anyhow!(e.to_string())))?;
        // self.registry
        //     .call_as(&Addr::unchecked(admin.clone()))
        //     .approve_any_abstract_modules()
        //     .map_err(|e| CwOrchError::AnyError(anyhow!(e.to_string())))?;

        // // self.middleware
        // //     .update_config(None, None, None, Some(self.registry.addr_str()?))?;

        // // Create the Abstract Account because it's needed for the fees for the dex module
        // self.account.instantiate(
        //     &abstract_std::account::InstantiateMsg::<Empty> {
        //         code_id: self.account.code_id()?,
        //         owner: Some(
        //             abstract_std::objects::gov_type::GovernanceDetails::Monarchy {
        //                 monarch: admin.to_string(),
        //             },
        //         ),
        //         account_id: None,
        //         authenticator: None,
        //         namespace: None,
        //         install_modules: vec![], // TODO: install USB
        //         name: Some("deployment-dao".into()),
        //         description: Some("Powered By Terp Account Framework".into()),
        //         link: None,
        //     },
        //     None,
        //     &[],
        // )?;

        println!("{:#?}", mock.state());
        Ok(())
    }
    /// mint and list an account token.
    pub fn delegate_to_val(
        &mut self,
        mock: MockBech32,
        delegator: Addr,
        amount: u128,
    ) -> anyhow::Result<()> {
        // delgate some tokens to val one to satisfy delegation requirements
        // mock.wait_blocks(1)?;
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
                .map_err(|e| anyhow::anyhow!("{e}"))
        })?;
        // mock.wait_blocks(1)?;
        Ok(())
    }

    pub fn mint_and_list(
        &mut self,
        mock: MockBech32,
        account: &str,
        user: &Addr,
    ) -> anyhow::Result<AppResponse> {
        // set approval for user, for all tokens
        // approve_all is needed because we don't know the token_id before-hand
        let market = self.manifold.address()?;
        self.nft.call_as(user).approve_all(market, None)?;

        let amount: Uint128 = (match account.to_string().as_str().len() {
            0..=2 => BASE_PRICE,
            3 => BASE_PRICE * 100,
            4 => BASE_PRICE * 10,
            _ => BASE_PRICE,
        })
        .into();
        let name_fee = coins(amount.u128(), "uthiol");
        // give user some funds
        if Uint128::from(BASE_PRICE) > Uint128::from(0u128) {
            mock.add_balance(&user.clone(), name_fee.clone())?;
        };
        // call as user to mint and list the account name, with account fees
        let res = self.manifold.call_as(user).execute(
            &terp_account::manifold::ExecuteMsg::MintAndList {
                account: account.to_string(),
            },
            &name_fee,
        )?;
        Ok(res)
    }

    pub fn owner_of(&self, id: String) -> anyhow::Result<String> {
        let res = self.nft.owner_of(id, None)?;
        Ok(res.owner)
    }

    pub fn bid_w_funds(
        &self,
        mock: MockBech32,
        account: &str,
        bidder: Addr,
        amount: u128,
    ) -> anyhow::Result<()> {
        // give bidder some funds
        let bid_amnt = coins(amount, "uthiol");
        mock.add_balance(&bidder, bid_amnt.clone())?;

        self.manifold.call_as(&bidder).execute(
            &terp_account::manifold::ExecuteMsg::SetBid {
                token_id: account.into(),
            },
            &bid_amnt,
        )?;

        // query if bid exists
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

// pub async fn assert_wallet_balance(mut chains: Vec<ChainInfoOwned>) -> Vec<ChainInfoOwned> {
//     if chains.is_empty() {
//         chains = SUPPORTED_CHAINS.iter().cloned().map(Into::into).collect();
//     }
//     // check that the wallet has enough gas on all the chains we want to support
//     for chain_info in &chains {
//         let chain = DaemonAsyncBuilder::new(chain_info.clone())
//             .build()
//             .await
//             .unwrap();

//         let gas_denom = chain.state().chain_data.gas_denom.clone();
//         let gas_price = chain.state().chain_data.gas_price;
//         let fee = (GAS_TO_DEPLOY as f64 * gas_price) as u128;
//         let bank = queriers::Bank::new_async(chain.channel());
//         let balance = bank
//             ._balance(&chain.sender_addr(), Some(gas_denom.clone()))
//             .await
//             .unwrap()
//             .clone()[0]
//             .clone();

//         log::debug!(
//             "Checking balance {} on chain {}, address {}. Expecting {}{}",
//             balance.amount,
//             chain_info.chain_id,
//             chain.sender_addr(),
//             fee,
//             gas_denom
//         );
//         if Uint256::from(fee) > balance.amount {
//             panic!("Not enough funds on chain {} to deploy the contract. Needed: {}{} but only have: {}{}", chain_info.chain_id, fee, gas_denom, balance.amount, gas_denom);
//         }
//         // check if we have enough funds
//     }

//     chains
// }
