use cw_orch::{anyhow, mock::MockBech32, prelude::*};
use terp_account::manifold::Config;

use crate::suite::TerpAccountDeployData;
use crate::{
    Bs721AccountMarketExecuteMsgTypes, Terp721AccountsQueryMsgFns, TerpAccountExecuteFns,
    TerpAccountMarketExecuteFns, TerpAccountMarketQueryFns, TerpAccountSuite,
};

use cosmwasm_std::{coins, to_json_binary, Decimal};
use cosmwasm_std::{Uint128, Uint256};
use cw_orch::mock::cw_multi_test::{SudoMsg, WasmSudo};

use cosmwasm_std::{coin, Attribute, Binary, Event};
use terp721_account_manifold::ContractError as MinterContractError;
use terp_account::{manifold::ExecuteMsg, Ask, Bid, PendingBid};
use terp_account::{DEPLOYMENT_DAO, TERP_PREFIX};

const BID_AMOUNT: u128 = 1_000_000_000;
#[test]
pub fn init() -> anyhow::Result<()> {
    // new mock Bech32 chain environment
    let mock = MockBech32::new("mock");
    // simulate deploying the test suite to the mock chain env.
    let suite =
        TerpAccountSuite::deploy_on(mock.clone(), TerpAccountDeployData { admin: mock.sender })?;

    assert_eq!(
        suite.manifold.config()?,
        Config {
            // minter: suite.manifold.address()?,
            // collection: suite.nft.addrwqess()?,
            public_mint_start_time: mock.app.borrow().block_info().time.plus_seconds(1)
        }
    );

    Ok(())
}

// mod hooks {
//     use abstract_interface::{AccountQueryFns, RegistryExecFns, RegistryQueryFns};
//     use abstract_std::objects::namespace::Namespace;
//     use abstract_std::objects::AccountId;
//     use abstract_std::REGISTRY;

//     use super::*;

//     #[test]
//     fn test_manage_sale_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("salehook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddSaleHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.sale_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveSaleHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.sale_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_manage_bid_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("bidhook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddBidHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.bid_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveBidHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.bid_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_manage_ask_hook() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let suite = TerpAccountSuite::deploy_on(mock.clone(), mock.sender.clone())?;
//         let hook_addr = mock.addr_make("askhook");

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::AddAskHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.ask_hooks()?;
//         assert_eq!(hooks.hooks.len(), 1);
//         assert_eq!(hooks.hooks[0], hook_addr.to_string());

//         suite
//             .manifold
//             .manage_hooks(ManageHooksAction::RemoveAskHook(hook_addr.to_string()))?;
//         let hooks = suite.manifold.ask_hooks()?;
//         assert_eq!(hooks.hooks.len(), 0);

//         Ok(())
//     }

//     #[test]
//     fn test_all_hooks_workflow() -> anyhow::Result<()> {
//         let mock = MockBech32::new("mock");
//         let mut suite = TerpAccountSuite::new(mock.clone());
//         suite.default_setup(mock.clone(), None, None)?;
//         mock.wait_seconds(200)?;
//         let mw_addr = suite.middleware.addr_str()?;
//         let account1 = "rick";

//         // // register hooks to middleware
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddAskHook(mw_addr.clone()))?;
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddBidHook(mw_addr.clone()))?;
//         // suite
//         //     .manifold
//         //     .manage_hooks(ManageHooksAction::AddSaleHook(mw_addr.clone()))?;

//         // // perform actions for hooks
//         // let res = suite.mint_and_list(mock.clone(), account1, &mock.sender_addr())?;
//         // let res = suite.account.instantiate(
//         //     &abstract_std::account::InstantiateMsg::<Empty> {
//         //         code_id: suite.account.code_id()?,
//         //         owner: Some(abstract_std::objects::gov_type::GovernanceDetails::NFT {
//         //             collection_addr: suite.nft.addr_str()?,
//         //             token_id: account1.to_string(),
//         //         }),
//         //         account_id: None,
//         //         authenticator: None,
//         //         namespace: Some(account1.to_string()),
//         //         install_modules: vec![], // TODO: install USB
//         //         name: Some(account1.to_string()),
//         //         description: Some("Powered By Terp Account Framework".into()),
//         //         link: None,
//         //     },
//         //     None,
//         //     &[],
//         // )?;

//         // let res = suite.registry.namespace_list(None, None)?.namespaces;
//         // println!("{:#?}", res);

//         // let res = suite.account.info()?;
//         // println!("{:#?}", res);
//         // let res = suite.registry.namespace_list(None, None)?;
//         // let res = suite
//         //     .registry
//         //     .namespace(Namespace::new(&account1.to_string())?)?;
//         // println!("{:#?}", res);

//         // res.assert_event(&Event::new("wasm-abstract").add_attributes(vec![
//         //         Attribute {
//         //             key: "_contract_address".into(),
//         //             value: suite.middleware.addr_str()?,
//         //         },
//         //         Attribute::new("contract", REGISTRY),
//         //         Attribute::new("action", "claim_namespace"),
//         //         Attribute::new(
//         //             "account_id",
//         //             &suite
//         //                 .registry
//         //                 .namespace(Namespace::new(account1)?)?
//         //                 .unwrap()
//         //                 .account_id
//         //                 .to_string(),
//         //         ),
//         //         Attribute::new("namespace", "rick"),
//         //     ]));

//         // assert hook logic is performed
//         Ok(())
//     }
// }

mod execute {

    use terp721_account_manifold::state::MAX_FEE_BPS;

    use super::*;

    #[test]
    fn test_check_approvals() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;

        let owner = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        mock.wait_seconds(6)?;

        let ops = suite.nft.operator(suite.manifold.address()?, owner, None)?;
        println!("{:#?}", ops);
        // check operators

        Ok(())
    }
    #[test]
    fn test_mint() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;

        //  must set approval for

        // check if account is listed in marketplace
        let res = suite.manifold.ask(token_id.to_string())?.unwrap();
        assert_eq!(res.token_id, token_id);

        // check if token minted
        let res = suite.nft.num_tokens()?;
        assert_eq!(res.count, 1);

        assert_eq!(suite.owner_of(token_id.into())?, owner.to_string());

        Ok(())
    }
    #[test]
    fn test_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let token_id = "bandura";
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        suite.bid_w_funds(mock, token_id, bidder, BID_AMOUNT)?;
        Ok(())
    }

    #[test]
    fn test_accept_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new("mock");
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let token_id = "bandura";

        mock.wait_seconds(200)?;

        // ensure operator approval is set
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        let balance = mock.query_balance(&bidder, "uthiol")?;
        // assert overwritten bid returns funds from first bid
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT * 2)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
        let balance2 = mock.query_balance(&bidder, "uthiol")?;

        assert_eq!(balance, Uint256::zero());
        assert_eq!(balance2, Uint256::from(BID_AMOUNT * 2));

        // bid smaller than minimum
        suite
            .bid_w_funds(mock.clone(), token_id, bidder.clone(), 99u128)
            .unwrap_err();

        // ask only removed by minter
        assert!(suite
            .manifold
            .remove_ask(token_id.to_string())
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::Unauthorized {}.to_string()));

        assert!(suite
            .manifold
            .call_as(&suite.nft.address()?)
            .remove_ask(token_id.to_string())
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::ExistingBids {}.to_string()));

        // user (owner) starts off with 0 internet funny money
        assert_eq!(
            mock.balance(&owner.clone(), Some("uthiol".into()))?[0].amount,
            Uint256::zero()
        );

        assert!(suite
            .manifold
            .update_ask(bidder.to_string(), token_id.to_string(),)
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::Unauthorized {}.to_string()));
        assert_eq!(
            suite.manifold.bids(token_id.to_string(), None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );
        assert_eq!(
            suite.manifold.bids_sorted_by_price(None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );

        assert_eq!(
            suite.manifold.reverse_bids_sorted_by_price(None, None)?,
            vec![Bid {
                token_id: token_id.to_string(),
                bidder: bidder.clone(),
                amount: BID_AMOUNT.into(),
                created_time: mock.block_info()?.time,
            }],
        );

        assert!(suite
            .manifold
            .call_as(&bidder)
            .accept_bid(bidder.clone(), token_id.into())
            .unwrap_err()
            .to_string()
            .contains("UnauthorizedOwner"));
        suite.manifold.accept_bid(bidder.clone(), token_id.into())?;

        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(token_id.to_string())?;

        // check if bid is removed
        assert!(suite
            .manifold
            .bid(bidder.to_string(), token_id.into())?
            .is_none());
        // verify that the bidder is the new owner
        assert_eq!(suite.owner_of(token_id.into())?, bidder.to_string());
        // check if user got the bid amount
        assert_eq!(
            mock.balance(&owner, Some("uthiol".into()))?[0].amount,
            Uint256::from(BID_AMOUNT)
        );
        // confirm that a new ask was created
        let res = suite.manifold.ask(token_id.to_string())?.unwrap();
        assert_eq!(res.seller, bidder);
        assert_eq!(res.token_id, token_id);

        // remove ask
        let res = suite.nft.call_as(&bidder).burn(token_id.to_string())?;
        println!("res: {:#?}", res);
        res.assert_event(&Event::new("wasm-burn-account").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.nft.addr_str()?,
            },
            Attribute::new("account", token_id.to_string()),
        ]));
        res.assert_event(&Event::new("wasm-remove-ask").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.manifold.addr_str()?,
            },
            Attribute::new("token_id", token_id.to_string()),
        ]));

        assert_eq!(suite.manifold.ask(token_id.to_string())?, None);

        Ok(())
    }
    #[test]

    fn test_two_sales_cycles() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let owner = mock.sender.clone();
        let bidder = mock.addr_make("bidder");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";
        let market = suite.manifold.address()?;
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &owner)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
        suite.manifold.accept_bid(bidder.clone(), token_id.into())?;
        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(token_id.to_string())?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT)?;
        suite.nft.call_as(&bidder).approve(market, token_id, None)?;
        suite
            .manifold
            .call_as(&bidder)
            .accept_bid(bidder2.clone(), token_id.into())?;
        mock.wait_seconds(60)?;
        suite
            .manifold
            .call_as(&bidder)
            .finalize_bid(token_id.to_string())?;

        Ok(())
    }
    #[test]
    fn test_reverse_map() -> anyhow::Result<()> {
        let token_id = "bandura";

        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        mock.wait_seconds(200)?;

        let admin2 = mock.addr_make("admin2");
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // when no associated address, query responds with token owner
        assert_eq!(
            suite.nft.call_as(&admin2).associated_address(token_id)?,
            admin2.clone()
        );

        // associate owner address with account account
        suite
            .nft
            .call_as(&admin2)
            .associate_address(token_id, Some(admin2.to_string()))?;

        // query associated address should return user
        assert_eq!(suite.nft.associated_address(token_id)?, admin2);

        // added to get around rate limiting
        mock.wait_seconds(60)?;
        // associate another
        let account2 = "exam";
        suite.mint_and_list(mock.clone(), account2, &admin2.clone())?;

        suite
            .nft
            .call_as(&admin2)
            .associate_address(account2, Some(admin2.to_string()))?;

        assert_eq!(suite.nft.account(admin2)?, account2.to_string());
        Ok(())
    }

    #[test]
    fn test_reverse_map_contract_address() -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn test_reverse_map_not_contract_address_admin() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;

        let not_admin = mock.addr_make_with_balance("not-admin", coins(1000000000, "uthiol"))?;
        mock.add_balance(&not_admin, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), not_admin.clone(), 10000000000u128)?;

        let minter = suite.manifold.address()?;
        let token_id = "bandura";
        println!("not-admin: {:#?}", not_admin);
        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &not_admin)?;

        suite
            .nft
            .associate_address(token_id, Some(minter.to_string()))
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_reverse_map_not_owner() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        suite
            .nft
            .associate_address(token_id, Some(admin2.to_string()))
            .unwrap_err();
        Ok(())
    }
    #[test]
    fn test_pause() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // pause minting
        suite.manifold.pause(true)?;

        // error trying to mint
        mock.wait_seconds(200)?;
        suite
            .mint_and_list(mock.clone(), token_id, &admin2)
            .unwrap_err();

        Ok(())
    }

    #[test]
    fn test_update_mkt_sudo() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, None)?;
        let token_id = "bandura";
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        // run sudo msg
        mock.app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.manifold.address()?,
                message: to_json_binary(&terp_account::manifold::SudoMsg::UpdateParams {
                    trading_fee_bps: Some(1000u64),
                    min_price: Some(Uint128::from(1000u128)),
                    ask_interval: Some(1000),
                    cooldown_duration: Some(69),
                    cooldown_cancel_fee: Some(coin(69u128, "jerets")),
                    min_account_length: None,
                    max_account_length: None,
                    base_price: None,
                    base_delegation: None,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let sudo_err = mock
            .app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.manifold.address()?,
                message: to_json_binary(&terp_account::manifold::SudoMsg::UpdateParams {
                    trading_fee_bps: Some(MAX_FEE_BPS * 2u64),
                    min_price: Some(Uint128::from(1000u128)),
                    ask_interval: Some(1000),
                    cooldown_duration: Some(69),
                    cooldown_cancel_fee: Some(coin(69u128, "jerets")),
                    min_account_length: None,
                    max_account_length: None,
                    base_price: None,
                    base_delegation: None,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .unwrap_err()
            .to_string();
        assert!(
            sudo_err.contains(
                &MinterContractError::InvalidTradingFeeBps(MAX_FEE_BPS * 2u64).to_string()
            ),
            "Expected error to contain InvalidTradingFeeBps, got: {sudo_err}"
        );

        // confirm updated params
        let res = suite.manifold.params()?;
        assert_eq!(res.trading_fee_percent, Decimal::percent(10));
        assert_eq!(res.min_price, Uint128::from(1000u128));
        assert_eq!(res.ask_interval, 1000);
        assert_eq!(res.cooldown_duration, 69);
        assert_eq!(res.cooldown_fee, coin(69u128, "jerets"));

        let _new = mock.addr_make("new-jawn");
        let _newnew = mock.addr_make("newer-jawn");

        Ok(())
    }

    #[test]
    fn test_cooldown_period_operator_approve() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;
        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        suite.nft.revoke_all(suite.manifold.address()?)?;
        mock.wait_seconds(60)?;
        let res = suite.manifold.finalize_bid(account.to_string())?;
        res.assert_event(&Event::new("wasm").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.nft.addr_str()?,
            },
            Attribute::new("action", "approve_all".to_string()),
            Attribute::new("operator", suite.manifold.addr_str()?),
        ]));
        assert_eq!(
            suite.manifold.ask(account.to_string())?.unwrap().seller,
            &bidder
        );
        Ok(())
    }

    #[test]
    fn test_cooldown_period() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;

        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        let owner_balance_a = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_a = mock.query_balance(&bidder, "uthiol")?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        // no funds are transfered before cooldown is over
        let owner_balance_b = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_b = mock.query_balance(&bidder, "uthiol")?;
        mock.wait_seconds(10)?;
        assert_eq!(owner_balance_a, owner_balance_b);
        assert_eq!(bidder_balance_a, bidder_balance_b);
        assert_eq!(
            suite.manifold.cooldown(account.to_string())?,
            Some(PendingBid {
                ask: Ask {
                    token_id: account.to_string(),
                    id: 1,
                    seller: owner.clone(),
                },
                new_owner: bidder.clone(),
                amount: BID_AMOUNT.into(),
                unlock_time: mock.block_info()?.time.plus_seconds(50)
            })
        );
        assert!(suite.nft.burn(account).unwrap_err().to_string().contains(
            &terp721_account::ContractError::AccountCannotBeTransfered {
                reason: "Account is in cooldown".to_string()
            }
            .to_string()
        ));
        assert!(suite
            .nft
            .transfer_nft(bidder.clone(), account)
            .unwrap_err()
            .to_string()
            .contains(
                &terp721_account::ContractError::AccountCannotBeTransfered {
                    reason: "Account is in cooldown".to_string()
                }
                .to_string()
            ));
        assert!(suite
            .nft
            .send_nft(suite.manifold.address()?, Binary::default(), account)
            .unwrap_err()
            .to_string()
            .contains(
                &terp721_account::ContractError::AccountCannotBeTransfered {
                    reason: "Account is in cooldown".to_string()
                }
                .to_string()
            ));

        // cannot finalize until cooldown period is over

        assert!(mock.block_info()?.time.seconds().lt(&suite
            .manifold
            .cooldown(account.to_string())?
            .unwrap()
            .unlock_time
            .seconds()));
        assert!(suite
            .manifold
            .finalize_bid(account.to_string())
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::InvalidDuration {}.to_string()));

        mock.wait_seconds(50)?;
        // cannot cancel after cooldown completes
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::CancelCooldown {
                    token_id: account.to_string(),
                },
                &[coin(500_000_000, "uthiol")],
            )
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::InvalidDuration {}.to_string()));
        // token id doesnt exists
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::CancelCooldown {
                    token_id: "babber".to_string(),
                },
                &[coin(500_000_000, "uthiol")],
            )
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::AskNotFound {}.to_string()));
        // token id doesnt exists
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::FinalizeBid {
                    token_id: "babber".to_string(),
                },
                &[coin(500_000_000, "uthiol")],
            )
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::AskNotFound {}.to_string()));

        suite
            .manifold
            .call_as(&owner)
            .finalize_bid(account.to_string())?;
        assert_eq!(suite.manifold.cooldown(account.to_string())?, None,);
        let owner_balance_c = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_c = mock.query_balance(&bidder, "uthiol")?;
        assert_eq!(owner_balance_c, owner_balance_b + Uint256::from(BID_AMOUNT));
        assert_eq!(bidder_balance_b, bidder_balance_c);
        Ok(())
    }

    #[test]
    fn test_cancel_cooldown_period() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        mock.wait_seconds(200)?;
        let owner = mock.sender.clone();
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let bidder = mock.addr_make("bidder");
        mock.add_balance(&owner, vec![coin(10000000000u128, "uthiol")])?;
        let account = "account";
        suite.mint_and_list(mock.clone(), account, &owner)?;
        suite.bid_w_funds(mock.clone(), account, bidder.clone(), BID_AMOUNT)?;
        let owner_balance_a = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_a = mock.query_balance(&bidder, "uthiol")?;
        suite.manifold.accept_bid(bidder.clone(), account.into())?;
        // no funds are transfered before cooldown is over
        let owner_balance_b = mock.query_balance(&owner, "uthiol")?;
        let bidder_balance_b = mock.query_balance(&bidder, "uthiol")?;
        mock.wait_seconds(10)?;
        assert_eq!(owner_balance_a, owner_balance_b);
        assert_eq!(bidder_balance_a, bidder_balance_b);
        assert_eq!(
            suite.manifold.cooldown(account.to_string())?,
            Some(PendingBid {
                ask: Ask {
                    token_id: account.to_string(),
                    id: 1,
                    seller: owner.clone(),
                },
                new_owner: bidder.clone(),
                amount: BID_AMOUNT.into(),
                unlock_time: mock.block_info()?.time.plus_seconds(50)
            })
        );
        mock.wait_seconds(49)?;
        assert!(suite
            .manifold
            .call_as(&bidder)
            .cancel_cooldown(account.to_string())
            .unwrap_err()
            .to_string()
            .contains(&MinterContractError::Unauthorized {}.to_string()));
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::CancelCooldown {
                    token_id: account.to_string()
                },
                &[]
            )
            .unwrap_err()
            .to_string()
            .contains("No funds sent"));
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::CancelCooldown {
                    token_id: account.to_string()
                },
                &[coin(499_000_000, "uthiol")]
            )
            .unwrap_err()
            .to_string()
            .contains(
                &MinterContractError::IncorrectPayment {
                    got: 499_000_000u128.into(),
                    expected: 500_000_000u128.into()
                }
                .to_string()
            ));
        assert!(suite
            .manifold
            .execute(
                &ExecuteMsg::CancelCooldown {
                    token_id: account.to_string()
                },
                &[coin(499_000_000, "uthiol")]
            )
            .unwrap_err()
            .to_string()
            .contains("Incorrect payment, got: 499000000, expected 500000000"));
        suite.manifold.execute(
            &ExecuteMsg::CancelCooldown {
                token_id: account.to_string(),
            },
            &[coin(500_000_000, "uthiol")],
        )?;
        let dd_balance = mock.query_balance(&Addr::unchecked(DEPLOYMENT_DAO), "uthiol")?;
        let bidder_balance_c = mock.query_balance(&bidder, "uthiol")?;
        assert_eq!(
            bidder_balance_c,
            Uint256::from(BID_AMOUNT + 250_000_000u128)
        );
        assert_eq!(dd_balance, Uint256::from(250_000_000u128));
        Ok(())
        // assert_eq!(suite.manifold.cooldown(account.to_string())?, None,);

        // assert_eq!(owner_balance_b, bidder_balance_c);
        // assert_ne!(bidder_balance_b, owner_balance_c);
    }

    #[test]
    fn test_mint_with_delegation_tiers() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;
        let user = mock.addr_make("user");
        mock.add_balance(&user, vec![coin(10000000000u128, "uthiol")])?;

        let id_3 = "rep";
        let id_4 = "repe";
        let id_5 = "repea";
        let id_6 = "repeat";
        let base_delegation = Uint128::new(2100_000_000); // assuming this is the base delegation amount

        // Test with account length 3
        let user3 = mock.addr_make("user3");
        mock.add_balance(&user3, vec![coin(10500000000u128, "uthiol")])?;
        suite.delegate_to_val(
            mock.clone(),
            user3.clone(),
            (base_delegation * Uint128::new(5u128)).into(),
        )?;
        suite.mint_and_list(mock.clone(), id_3, &user3)?;

        // Test with account length 4
        let user4 = mock.addr_make("user4abcd");
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(
            mock.clone(),
            user4.clone(),
            (base_delegation * Uint128::new(3u128)).into(),
        )?;
        suite.mint_and_list(mock.clone(), id_4, &user4)?;

        // Test with account length 5 or more
        let user5 = mock.addr_make("user5abcde");
        mock.add_balance(&user5, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user5.clone(), base_delegation.u128())?;
        suite.mint_and_list(mock.clone(), id_5, &user5)?;

        // Test with insufficient delegation for account length 3
        let user3_insufficient = mock.addr_make("user3_insufficient");
        mock.add_balance(&user3_insufficient, vec![coin(10000440u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user3_insufficient.clone(), 10000440u128)?;
        assert!(suite
            .mint_and_list(mock.clone(), id_6, &user3_insufficient)
            .unwrap_err()
            .to_string()
            .contains(
                &MinterContractError::IncorrectDelegation {
                    got: 10000440u128.into(),
                    expected: base_delegation.u128()
                }
                .to_string()
            ));

        Ok(())
    }
}
mod admin {
    use super::*;

    #[test]
    fn test_update_admin() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin2 = mock.addr_make("admin2");
        // non-admin tries to set admin to None
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::RenounceOwnership)
            .unwrap_err();
        // admin updates admin
        suite
            .manifold
            .update_ownership(cw_ownable::Action::TransferOwnership {
                new_owner: admin2.to_string(),
                expiry: None,
            })?;
        // new admin updates to have no admin
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::AcceptOwnership)?;
        suite
            .manifold
            .call_as(&admin2)
            .update_ownership(cw_ownable::Action::RenounceOwnership)?;
        // cannot update without admin
        suite
            .manifold
            .update_ownership(cw_ownable::Action::TransferOwnership {
                new_owner: admin2.to_string(),
                expiry: None,
            })
            .unwrap_err();
        Ok(())
    }
}
mod query {
    use cosmwasm_std::{coin, Attribute, Event};
    use terp_account::{manifold::BidOffset, Bid};

    use super::*;

    #[test]
    fn test_query_ask() -> anyhow::Result<()> {
        let token_id = "bandura";
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin2 = mock.addr_make("admin2");
        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin2)?;

        assert_eq!(
            suite.manifold.ask(token_id.into())?.unwrap().token_id,
            token_id
        );
        Ok(())
    }
    #[test]
    fn test_query_asks() -> anyhow::Result<()> {
        let token_id = "bandura";
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(suite.manifold.asks(None, None)?[0].id, 1);

        Ok(())
    }
    #[test]
    fn test_query_asks_by_seller() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(
            suite
                .manifold
                .asks_by_seller(admin.to_string(), None, None)?
                .len(),
            1
        );
        Ok(())
    }
    #[test]
    fn test_query_ask_count() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let admin2 = mock.addr_make("admin2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&admin2, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), admin2.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;
        suite.mint_and_list(mock.clone(), "hack", &admin2)?;

        assert_eq!(suite.manifold.ask_count()?, 2);
        Ok(())
    }
    #[test]
    fn test_query_top_bids() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        let res = suite.manifold.bids_for_seller(admin.clone(), None, None)?;
        assert_eq!(res.len(), 2);
        assert_eq!(res[1].amount.u128(), BID_AMOUNT);

        // test pagination
        let filter = BidOffset {
            price: Uint128::from(BID_AMOUNT),
            token_id: token_id.into(),
            bidder: bidder1.clone(),
        };
        let res: Vec<Bid> =
            suite
                .manifold
                .bids_for_seller(admin.clone(), None, Some(filter.clone()))?;

        // should be length 0 because there are no token_ids besides NAME.to_string()
        assert_eq!(res.len(), 0);

        // added to get around rate limiting
        mock.wait_seconds(60)?;

        // test pagination with multiple accounts and bids
        let account: &str = "jump";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), account, &admin)?;

        suite.bid_w_funds(mock.clone(), account, bidder1.clone(), BID_AMOUNT * 3)?;
        suite.bid_w_funds(mock.clone(), account, bidder2, BID_AMOUNT * 2)?;
        let res = suite
            .manifold
            .bids_for_seller(admin.clone(), None, Some(filter.clone()))?;
        // should be length 2 because there is token_id "jump" with 2 bids
        assert_eq!(res.len(), 2);

        suite
            .manifold
            .call_as(&bidder1)
            .execute(
                &Bs721AccountMarketExecuteMsgTypes::RemoveBid {
                    token_id: account.to_string(),
                },
                &[coin(1, "uthiol")],
            )
            .unwrap_err();

        let res = suite
            .manifold
            .call_as(&bidder1)
            .remove_bid(account.to_string())?;

        println!("res: {:#?}", res);
        res.assert_event(&Event::new("transfer").add_attributes(vec![
            Attribute::new("recipient", bidder1.to_string()),
            Attribute::new("sender", suite.manifold.addr_str()?),
            Attribute::new("amount", coin(BID_AMOUNT * 3, "uthiol").to_string()),
        ]));
        res.assert_event(&Event::new("wasm-remove-bid").add_attributes(vec![
            Attribute {
                key: "_contract_address".to_string(),
                value: suite.manifold.addr_str()?,
            },
            Attribute::new("token_id", account.to_string()),
            Attribute::new("bidder", bidder1.to_string()),
        ]));

        let res = suite.manifold.bids_for_seller(admin, None, Some(filter))?;
        assert_eq!(res.len(), 1);

        Ok(())
    }
    #[test]
    fn test_query_bids_by_seller() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        let res = suite.manifold.bids_for_seller(admin.clone(), None, None)?;
        assert_eq!(res.len(), 2);
        assert_eq!(res[1].amount.u128(), BID_AMOUNT);

        // test pagination
        let res = suite.manifold.bids_for_seller(
            admin.clone(),
            None,
            Some(BidOffset::new(
                Uint128::from(BID_AMOUNT),
                token_id.to_string(),
                bidder1.clone(),
            )),
        )?;
        assert_eq!(res.len(), 0);

        // added to get around rate limiting
        mock.wait_seconds(60)?;

        let account = "jump";
        suite.mint_and_list(mock.clone(), account, &admin)?;
        suite.bid_w_funds(mock.clone(), account, bidder1.clone(), BID_AMOUNT * 3)?;
        suite.bid_w_funds(mock.clone(), account, bidder2, BID_AMOUNT * 2)?;
        // should be length 2 because there is token_id "jump" with 2 bids
        let res = suite.manifold.bids_for_seller(
            admin.clone(),
            None,
            Some(BidOffset::new(
                Uint128::from(BID_AMOUNT),
                token_id.to_string(),
                bidder1.clone(),
            )),
        )?;
        assert_eq!(res.len(), 2);

        Ok(())
    }
    #[test]
    fn test_query_highest_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let bidder1 = mock.addr_make("bidder1");
        let bidder2 = mock.addr_make("bidder2");
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT)?;
        suite.bid_w_funds(mock.clone(), token_id, bidder2.clone(), BID_AMOUNT * 5)?;

        assert_eq!(
            suite
                .manifold
                .highest_bid(token_id.to_string())?
                .unwrap()
                .amount
                .u128(),
            BID_AMOUNT * 5
        );

        Ok(())
    }
    #[test]
    fn test_query_account() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let admin = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin)?;

        // fails with "user" string, has to be a bech32 address
        suite.nft.account(token_id).unwrap_err();

        suite.mint_and_list(mock.clone(), "yoyo", &admin)?;

        suite
            .nft
            .associate_address("yoyo", Some(admin.to_string()))?;

        assert_eq!(suite.nft.account(admin)?, "yoyo".to_string());

        Ok(())
    }
    // #[test]
    // fn test_query_trading_start_time() -> anyhow::Result<()> {
    //     let mock = MockBech32::new(TERP_PREFIX);
    //     let mut suite = TerpAccountSuite::new(mock.clone());
    //     suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

    //     Ok(())
    // }
}
mod collection {
    use cosmwasm_std::coin;
    use terp_account::TextRecord;

    use super::*;
    #[test]
    fn test_verify_twitter() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let verifier = mock.addr_make("verifier");
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;
        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        suite
            .nft
            .verify_text_record(token_id, account, true)
            .unwrap_err();

        suite
            .nft
            .call_as(&verifier)
            .verify_text_record(token_id, account, true)?;

        // query text record to see if verified is set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, Some(true));

        Ok(())
    }
    #[test]
    fn test_verify_false() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let verifier = mock.addr_make("verifier");
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        suite
            .nft
            .call_as(&verifier)
            .verify_text_record(token_id, account, false)?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, Some(false));

        Ok(())
    }
    #[test]
    fn test_verified_text_record() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        mock.wait_seconds(200)?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        suite.mint_and_list(mock, token_id, &admin_user)?;

        let account = "twitter";
        let value = "loaf0bred";

        suite
            .nft
            .add_text_record(token_id, TextRecord::new(account, value))?;

        // query text record to see if verified is not set
        let res = suite.nft.nft_info(token_id)?;
        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        // attempt update text record w verified value
        suite.nft.update_text_record(
            token_id,
            TextRecord {
                account: token_id.into(),
                value: "some new value".to_string(),
                verified: Some(true),
            },
        )?;

        // query text record to see if verified is set
        let res = suite.nft.nft_info(token_id)?;

        assert_eq!(res.extension.records[0].account, account.to_string());
        assert_eq!(res.extension.records[0].verified, None);

        // query image nft
        assert_eq!(suite.nft.image_nft(token_id)?, None);
        Ok(())
    }
    #[test]
    fn test_transfer_nft() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite
            .nft
            .transfer_nft(mock.addr_make("new-addr"), token_id)?;

        Ok(())
    }
    #[test]
    fn test_send_nft() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite.nft.send_nft(
            mock.addr_make("new-addr"),
            to_json_binary("ini").map_err(|e| anyhow::anyhow!("{e}"))?,
            token_id,
        )?;
        Ok(())
    }
    #[test]
    fn test_transfer_nft_and_bid() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;
        let bidder1 = mock.addr_make("bidder1");
        let market = suite.manifold.address()?;

        let user1 = mock.addr_make("user1");
        let admin_user = mock.sender.clone();
        let token_id = "bandura";

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        suite.nft.transfer_nft(user1.clone(), token_id)?;

        suite.bid_w_funds(mock.clone(), token_id, bidder1.clone(), BID_AMOUNT * 3)?;

        // user2 must approve the marketplace to transfer their account
        suite.nft.call_as(&user1).approve(market, token_id, None)?;
        // accept bid
        suite
            .manifold
            .call_as(&user1)
            .accept_bid(bidder1, token_id.into())?;

        Ok(())
    }
    #[test]
    fn test_transfer_nft_with_reverse_map() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let user = mock.addr_make("user");
        let user2 = mock.addr_make("user2");
        let token_id = "bandura";

        // delegate
        mock.add_balance(&user, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user.clone(), 10000000000u128)?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &user)?;

        suite
            .nft
            .call_as(&user)
            .associate_address(token_id, Some(user.to_string()))?;

        assert_eq!(suite.nft.account(user.clone())?, token_id);

        suite
            .nft
            .call_as(&user)
            .transfer_nft(user2.clone(), token_id)?;

        suite.nft.account(user).unwrap_err();
        suite.nft.account(user2).unwrap_err();

        Ok(())
    }
    // #[test]
    // fn test_burn_nft() -> anyhow::Result<()> {
    //     Ok(())
    // }
    // #[test]
    // fn test_burn_with_existing_bids() -> anyhow::Result<()> {
    //     Ok(())
    // }
    // #[test]
    // fn test_burn_nft_with_reverse_map() -> anyhow::Result<()> {
    //     Ok(())
    // }
    #[test]
    fn test_sudo_update() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let max_record_count = suite.nft.params()?.max_record_count;
        let max_rev_key_count = suite.nft.params()?.max_reverse_map_key_limit;

        // run sudo msg
        mock.app
            .borrow_mut()
            .sudo(SudoMsg::Wasm(WasmSudo {
                contract_addr: suite.nft.address()?,
                message: to_json_binary(&terp721_account::msg::SudoMsg::UpdateParams {
                    max_record_count: max_record_count + 1,
                    max_rev_map_count: max_rev_key_count + 4,
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            }))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        assert_eq!(suite.nft.params()?.max_record_count, max_record_count + 1);
        assert_eq!(
            suite.nft.params()?.max_reverse_map_key_limit,
            max_record_count + 4
        );

        Ok(())
    }
}
mod public_start_time {

    use cosmwasm_std::coin;
    use terp_account::manifold::Config;

    use super::*;

    #[test]
    fn test_mint_before_start() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();
        let token_id = "bandura";
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        suite
            .mint_and_list(mock.clone(), token_id, &admin_user)
            .unwrap_err();
        suite
            .mint_and_list(mock.clone(), token_id, &user4)
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_update_start_time() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let res = suite.manifold.config()?;
        assert_eq!(
            res.public_mint_start_time,
            mock.block_info()?.time.plus_seconds(200)
        );

        suite.manifold.update_config(Config {
            public_mint_start_time: mock.block_info()?.time.plus_seconds(2),
        })?;

        let res = suite.manifold.config()?;
        assert_eq!(
            res.public_mint_start_time,
            mock.block_info()?.time.plus_seconds(2)
        );

        Ok(())
    }
}

mod associate_address {

    use cosmwasm_std::coin;
    use terp721_account::msg::{InstantiateMsg, Terp721InstantiateMsg};

    use super::*;

    // #[test]
    // fn test_abstract_account_workflow() -> anyhow::Result<()> {
    //     let mock = MockBech32::new(TERP_PREFIX);
    //     let mut suite = TerpAccountSuite::new(mock.clone());
    //     suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

    //     let admin_user = mock.sender.clone();
    //     let token_id = "bandura";
    //     let bidder = mock.addr_make("bidder");

    //     mock.wait_seconds(200)?;
    //     suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

    //     // set nft as ownership
    //     suite.test_owner.instantiate(
    //         &TestOwnershipInitMsg {
    //             ownership: abstract_std::objects::gov_type::GovernanceDetails::NFT {
    //                 collection_addr: suite.nft.addr_str()?,
    //                 token_id: token_id.to_string(),
    //             },
    //         },
    //         None,
    //         &[],
    //     )?;

    //     // associate account to abstract account
    //     suite
    //         .nft
    //         .update_abs_acc_support(token_id, Some(suite.test_owner.addr_str()?))?;

    //     // query the associated address and ensure its the same as the abstract account
    //     assert_eq!(
    //         suite.nft.associated_address(token_id)?,
    //         suite.test_owner.address()?
    //     );

    //     // ensure if ownership is changed before cooldown
    //     suite
    //         .test_owner
    //         .update_ownership(abstract_std::objects::gov_type::GovernanceDetails::Renounced {})?;

    //     let owner_bal = mock.query_balance(&admin_user, "uthiol")?;
    //     let bidder_bal = mock.query_balance(&bidder, "uthiol")?;
    //     suite.bid_w_funds(mock.clone(), token_id, bidder.clone(), BID_AMOUNT)?;
    //     assert_eq!(bidder_bal, Uint128::zero());
    //     let _res = suite.manifold.accept_bid(bidder.clone(), token_id.into())?;
    //     // assert funds go back to bidder, along with tokens if owner changes ownership prior to finalizing bid
    //     mock.wait_seconds(60)?;
    //     let res = suite.manifold.finalize_bid(token_id.into())?;
    //     let owner_bal2 = mock.query_balance(&admin_user, "uthiol")?;
    //     let bidder_bal2 = mock.query_balance(&bidder, "uthiol")?;
    //     assert_eq!(BID_AMOUNT, bidder_bal2.u128());
    //     assert_eq!(owner_bal, owner_bal2);
    //     assert_eq!(
    //         suite.nft.owner_of(token_id, None)?.owner,
    //         bidder.to_string()
    //     );
    //     res.assert_event(&Event::new("transfer").add_attributes(vec![
    //         Attribute::new("recipient", bidder.to_string()),
    //         Attribute::new("sender", suite.manifold.addr_str()?),
    //         Attribute::new("amount", coin(BID_AMOUNT, "uthiol").to_string()),
    //     ]));

    //     Ok(())
    // }

    #[test]
    fn test_transfer_to_eoa() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();

        let cw721_id = suite.nft.code_id()?;
        let token_id = "bandura";

        let nft_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        collection_info_extension: None,
                        minter: Some(suite.manifold.address()?.to_string()),
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // mint and transfer to collection
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;
        suite.nft.transfer_nft(nft_addr.clone(), token_id)?;
        assert_eq!(
            suite.nft.owner_of(token_id, None)?.owner,
            nft_addr.to_string()
        );

        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_no_admin() -> anyhow::Result<()> {
        // For the purposes of this test, a collection contract with no admin needs to be instantiated (contract_with_no_admin)
        // This contract needs to have a creator that is itself a contract and this creator contract should have an admin (USER).
        // The admin (USER) of the creator contract will mint a account and associate the account with the collection contract that doesn't have an admin successfully.
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.sender.clone();

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let creator_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        collection_info_extension: None,
                        minter: Some(suite.manifold.address()?.to_string()),
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // The creator contract instantiates the collection contract with no admin
        let collection_with_no_admin_addr = mock
            .call_as(&creator_addr)
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                None,
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // USER4 mints a account
        suite.mint_and_list(mock.clone(), token_id, &admin_user)?;

        // USER4 tries to associate the account with the collection contract that doesn't have an admin
        suite
            .nft
            .call_as(&admin_user)
            .associate_address(token_id, Some(collection_with_no_admin_addr.to_string()))?;

        mock.wait_seconds(200)?;
        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_no_admin_fail() -> anyhow::Result<()> {
        // For the purposes of this test, a collection contract with no admin needs to be instantiated (contract_with_no_admin)
        // This contract needs to have a creator that is itself a contract and this creator contract should have an admin (USER).
        // An address other than the admin (USER) of the creator contract will mint a account, try to associate the account with the collection contract that doesn't have an admin and fail.
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.addr_make("admin-user");
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let creator_addr = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,

                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),
                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // The creator contract instantiates the collection contract with no admin
        let collection_with_no_admin_addr = mock
            .call_as(&creator_addr)
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),

                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                None,
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        // USER4 mints a account
        suite.mint_and_list(mock.clone(), token_id, &user4)?;

        // USER4 tries to associate the account with the collection contract that doesn't have an admin
        let err = suite
            .nft
            .call_as(&user4)
            .associate_address(token_id, Some(collection_with_no_admin_addr.to_string()))
            .unwrap_err();

        assert!(
            err.to_string().contains(
                &terp721_account::ContractError::UnauthorizedCreatorOrAdmin {}.to_string()
            ),
            "Unexpected error: {err}"
        );
        Ok(())
    }
    #[test]
    fn test_associate_with_a_contract_with_an_admin_fail() -> anyhow::Result<()> {
        let mock = MockBech32::new(TERP_PREFIX);
        let mut suite = TerpAccountSuite::new(mock.clone());
        suite.default_setup(mock.clone(), None, Some(mock.sender.clone()))?;

        let admin_user = mock.addr_make("admin-user");
        let user4 = mock.addr_make("user4");

        // delegate
        mock.add_balance(&user4, vec![coin(10000000000u128, "uthiol")])?;
        suite.delegate_to_val(mock.clone(), user4.clone(), 10000000000u128)?;

        let cw721_id = suite.nft.code_id()?;

        let token_id = "bandura";
        // Instantiating the creator contract with an admin (USER)
        let contract = mock
            .instantiate(
                cw721_id,
                &InstantiateMsg {
                    verifier: None,
                    base_init_msg: Terp721InstantiateMsg {
                        name: "test2".into(),
                        symbol: "TEST2".into(),

                        minter: Some(suite.manifold.address()?.to_string()),
                        collection_info_extension: None,
                        creator: None,
                        withdraw_address: None,
                    },
                },
                "test".into(),
                Some(&admin_user),
                &[],
            )?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        mock.wait_seconds(200)?;
        suite.mint_and_list(mock.clone(), token_id, &user4)?;

        let err = suite
            .nft
            .call_as(&user4)
            .associate_address(token_id, Some(contract.to_string()))
            .unwrap_err();

        assert!(
            err.to_string().contains(
                &terp721_account::ContractError::UnauthorizedCreatorOrAdmin {}.to_string()
            ),
            "Unexpected error: {err}"
        );
        Ok(())
    }
}
