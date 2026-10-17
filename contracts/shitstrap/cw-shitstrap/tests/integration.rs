use cosmwasm_std::testing::mock_dependencies;
use cw_shit_denom::*;
use cw_shitstrap::{
    contract::{msg::*, state::*, *},
    *,
};

#[test]
pub fn unit_shit_rate() {
    let deps = mock_dependencies();
    let matched = PossibleShit {
        shit_rate: Uint128::from(222222000000000000u128),
        token: UncheckedDenom::from(UncheckedDenom::Native("test".into())),
    };
    let shit_amount = Uint128::from(100100u128);

    let result = calculate_shit_value(deps.as_ref(), &matched, shit_amount.into()).unwrap();
    assert_eq!(result.0, Uint256::from(22244u128));
    assert_eq!(result.1, matched.token.into_checked(deps.as_ref()).unwrap());
}

#[test]
pub fn unit_shit_return_rate() {
    let new_val = Uint256::from(100u128);
    let cutoff = Uint256::from(50u128);
    let mut shit_rate = Uint256::from(1000000000000000000u128);

    let result = calculate_shit_return(new_val, cutoff, shit_rate).unwrap();
    assert_eq!(result.0, Uint256::from(50u128));
    assert_eq!(result.1, Uint256::from(50u128));

    shit_rate = Uint256::from(500000000000000000u128);
    let result = calculate_shit_return(new_val, cutoff, shit_rate).unwrap();
    assert_eq!(result.0, Uint256::from(50u128));
    assert_eq!(result.1, Uint256::from(100u128));

    shit_rate = Uint256::from(222222000000000000u128);
    let result = calculate_shit_return(new_val, cutoff, shit_rate).unwrap();
    assert_eq!(result.0, Uint256::from(50u128));
    assert_eq!(result.1, Uint256::from(225u128));
}

use cosmwasm_std::{coin, to_json_binary, Addr, Decimal, Empty, Event, Fraction, Uint128, Uint256};
use cw20::{Cw20Coin, Cw20ReceiveMsg};
use cw20_base::msg::InstantiateMsg as Cw20Init;
use cw_multi_test::{App, AppResponse, BankSudo, Contract, ContractWrapper, Executor, SudoMsg};
use cw_shit_denom::UncheckedDenom;

#[derive(Debug)]
enum TestError {
    Std(String),
    Other(String),
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TestError::Std(e) | TestError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl From<cosmwasm_std::StdError> for TestError {
    fn from(e: cosmwasm_std::StdError) -> Self {
        TestError::Std(e.to_string())
    }
}

impl From<cosmwasm_std::DecimalRangeExceeded> for TestError {
    fn from(e: cosmwasm_std::DecimalRangeExceeded) -> Self {
        TestError::Other(e.to_string())
    }
}

type TestResult = Result<(), TestError>;

pub const DEFAULT_BALANCE: u128 = 1_000_000_000;
pub const OWNER: &str = "owner";

// s1: atom
// s2: shit
// s3: silk
pub const SHITTER1: &str = "shitter1";
pub const SHITTER2: &str = "shitter2";
pub const SHITTER3: &str = "shitter3";

fn cw20_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw20_base::contract::execute,
        cw20_base::contract::instantiate,
        cw20_base::contract::query,
    );
    Box::new(contract)
}

fn shitstrap_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        crate::contract::execute,
        crate::contract::instantiate,
        crate::contract::query,
    );
    Box::new(contract)
}

fn init_bad_cw20(app: &mut App, cw20_id: u64, owner: Addr) -> Addr {
    // create another cw20
    let cw20_init = Cw20Init {
        name: "rtuioptaewi".into(),
        symbol: "rtjhtjkr".into(),
        decimals: 6,
        initial_balances: vec![],
        mint: None,
        marketing: None,
    };
    app.instantiate_contract(cw20_id, owner.clone(), &cw20_init, &[], "cw20", None)
        .unwrap()
}

fn instantiate_w_cw20(
    mut app: App,
    shit_id: u64,
    init: InstantiateMsg,
    cw20_id: u64,
    cw20: Cw20Init,
    owner: Addr,
    shitters: Vec<Addr>,
) -> ShitSuite {
    // init cw20
    let cw20 = app
        .instantiate_contract(
            cw20_id,
            owner.clone(),
            &cw20,
            &[],
            "cw20",
            Some(owner.to_string()),
        )
        .unwrap();
    // init shitstrap
    let shitstrap = app
        .instantiate_contract(
            shit_id,
            owner.clone(),
            &init,
            &[],
            "shitstrap",
            Some(owner.to_string()),
        )
        .unwrap();

    ShitSuite {
        app,
        shitstrap,
        cw20,
        owner,
        shitters,
    }
}

fn default_init(mut app: App, possible: Vec<PossibleShit>, cutoff: u128) -> ShitSuite {
    let shitstrap_id = app.store_code(shitstrap_contract());
    let cw20_id = app.store_code(cw20_contract());
    let owner = app.api().addr_make(OWNER);
    let s1 = app.api().addr_make(SHITTER1);
    let s2 = app.api().addr_make(SHITTER2);
    let s3 = app.api().addr_make(SHITTER3);

    let mut shitters = Vec::new();

    // create cw20
    let cw20_init = Cw20Init {
        name: "poo".into(),
        symbol: "POO".into(),
        decimals: 6,
        initial_balances: vec![
            Cw20Coin {
                address: s1.to_string(),
                amount: 100000000u128.into(),
            },
            Cw20Coin {
                address: s2.to_string(),
                amount: 100000000u128.into(),
            },
        ],
        mint: None,
        marketing: None,
    };
    // create shitstrap
    let init = InstantiateMsg {
        owner: Some(owner.to_string()),
        accepted: possible,
        cutoff: cutoff.into(),
        shitmos: cw_shit_denom::UncheckedDenom::Native("ushit".into()),
        title: "yoo".into(),
        description: "yoooooo".into(),
        daos: Vec::new(),
    };
    shitters.extend(vec![s1, s2, s3]);
    // instantiate contract with cw20
    let suite = instantiate_w_cw20(
        app,
        shitstrap_id,
        init,
        cw20_id,
        cw20_init.clone(),
        owner,
        shitters,
    );
    // return suite
    suite
}

pub struct ShitSuite {
    pub app: App,
    pub shitstrap: Addr,
    pub cw20: Addr,
    pub owner: Addr,
    pub shitters: Vec<Addr>,
}

impl ShitSuite {
    /// funds testing accounts with default balance
    fn setup_default_funds(&mut self, shitstrap: Addr) -> TestResult {
        self.app
            .sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: self.owner.clone().to_string(),
                amount: vec![coin(1_000_000_000u128, "uatom")],
            }))
            .unwrap();
        self.app
            .sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: self.shitters[0].to_string(), // s1: atom
                amount: vec![coin(1_000_000_000u128, "uatom")],
            }))
            .unwrap();
        self.app
            .sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: self.shitters[1].to_string(),
                amount: vec![coin(1_000_000_000u128, "ushit")],
            }))
            .unwrap();
        self.app
            .sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: self.shitters[2].to_string(),
                amount: vec![coin(1_000_000_000u128, "usilk")],
            }))
            .unwrap();
        // fund shitstrap with 1 million shit
        self.app
            .sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: shitstrap.to_string(),
                amount: vec![coin(1_000_000_000_000u128, "ushit")],
            }))
            .unwrap();
        Ok(())
    }

    /// helper function to participates in shitstrap with c20 coin
    fn participate_cw20(
        &mut self,
        sender: &str,
        amount: u128,
        contract: &str,
    ) -> Result<AppResponse, TestError> {
        Ok(self.app.execute_contract(
            Addr::unchecked(contract.to_string()),
            self.shitstrap.clone(),
            &ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: sender.into(),
                amount: amount.into(),
                msg: to_json_binary(&ReceiveMsg::ShitStrap {
                    shitter: sender.to_string(),
                    dao: None,
                    recp: None,
                })
                .unwrap(),
            }),
            &vec![],
        )?)
    }

    /// helper function to participate in shitstrap with native coin
    fn participate_native(
        &mut self,
        sender: &str,
        amount: u128,
        denom: &str,
    ) -> Result<AppResponse, TestError> {
        Ok(self.app.execute_contract(
            Addr::unchecked(sender.to_string()),
            self.shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked {
                    denom: cw_shit_denom::UncheckedDenom::Native(denom.into()),
                    amount: amount.into(),
                },
                dao: None,
                recp: None,
            },
            &vec![coin(amount, denom)],
        )?)
    }
}

#[test]
fn test_bad_init() -> TestResult {
    let mut app = App::default();
    let mut possible = vec![
        PossibleShit::native_denom("uatom", 1_000_000u128),
        PossibleShit::native_denom("uatom", 1_000_000u128),
        PossibleShit::native_denom("uatom3", 0u128),
        PossibleShit::native_denom("uatom4", 1_000_000u128),
    ];

    let title = "a".repeat(101).into();
    let description = "y".repeat(1001).into();
    let cutoff = Uint128::zero();

    // create default testing suite
    let mut shit = default_init(
        app,
        vec![PossibleShit::native_denom("uatom", 1_000_000u128)], // 1:1 ratio
        222u128,
    );

    let mut init_msg = InstantiateMsg {
        owner: Some(shit.owner.to_string()),
        accepted: possible.clone(),
        cutoff: cutoff.into(),
        shitmos: cw_shit_denom::UncheckedDenom::Native("ushit".into()),
        title,
        description,
        daos: Vec::new(),
    };

    // init shitstrap
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::ShittyCutoffRatio {}.to_string()));
    init_msg.cutoff = Uint128::from(1_000_000u128);
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::ShittyTitle {}.to_string()));
    init_msg.title = "shitstrap".into();
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::ShittyDescription {}.to_string()));
    init_msg.description = "shitstrap description".into();
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::UnnaceptableShitAmount {}.to_string()));
    possible = possible[..possible.len() - 1].to_vec();
    init_msg.accepted = possible.clone();
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::SameShit {}.to_string()));
    possible[1].token = UncheckedDenom::Native("uatom2".into());
    init_msg.accepted = possible.clone();
    let err = shit
        .app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::ShittyConversionRatio {}.to_string()));
    possible[2].shit_rate = Uint128::one();
    init_msg.accepted = possible;
    shit.app
        .instantiate_contract(
            1,
            shit.owner.clone(),
            &init_msg.clone(),
            &[],
            "shitstrap",
            Some(shit.owner.to_string()),
        )
        .unwrap();

    Ok(())
}

#[test]
fn test_shitstrap() -> TestResult {
    let mut app = App::default();
    // create default testing suite
    let mut shit = default_init(
        app,
        vec![PossibleShit::native_denom("uatom", 1000000000000000000u128)], // 1:1 ratio
        222000000u128,
    );
    // deposit 1 less than max
    let first_deposit = 221_000_000u128;
    let shitstrap = shit.shitstrap.clone();
    shit.setup_default_funds(shitstrap.clone())?;
    let s1 = shit.shitters[0].clone();

    // error with wrong native token
    let err = shit
        .app
        .execute_contract(
            shit.owner.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked::from_native("usilk", first_deposit),
                dao: None,
                recp: None,
            },
            &vec![coin(first_deposit, "uatom")],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::WrongShit {}.to_string()));
    let cw20_id = shit.app.store_code(cw20_contract());
    let cw20 = &init_bad_cw20(&mut shit.app, cw20_id, shit.owner.clone()).to_string();
    // error with wrong cw20 token
    let err = shit
        .participate_cw20(&shit.owner.to_string(), first_deposit, cw20)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::WrongShit {}.to_string()));

    // error without sending token
    let err = shit
        .app
        .execute_contract(
            s1.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked::from_native("uatom", first_deposit),
                dao: None,
                recp: None,
            },
            &vec![],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::DidntSendShit {}.to_string()));

    // move forward in time
    let mut block = shit.app.block_info();
    block.height += 1;
    shit.app.set_block(block);

    // error with correct token, but less sent then specified
    shit.app
        .execute_contract(
            s1.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked::from_native("uatom", first_deposit),
                dao: None,
                recp: None,
            },
            &vec![coin(22, "uatom")],
        )
        .unwrap_err();

    // move forward in time
    let mut block = shit.app.block_info();
    block.height += 1;
    shit.app.set_block(block);

    let og_owner_bal = shit.app.wrap().query_balance(shit.owner.clone(), "uatom")?;
    assert_eq!(og_owner_bal.amount, Uint256::new(DEFAULT_BALANCE));

    // participate in shitstrap with correct token
    shit.participate_native(&s1.to_string(), 221_000_000, "uatom")?;

    // confirm shit_rate is calculated correctly
    let res: Uint128 = shit
        .app
        .wrap()
        .query_wasm_smart(shitstrap.clone(), &QueryMsg::HasShit {})?;
    assert_eq!(res, Uint128::new(first_deposit));

    // confirm new balance of shitstrap
    let balance = shit.app.wrap().query_balance(&shitstrap, "uatom")?;
    assert_eq!(balance.amount, Uint256::zero());
    // let shit_rate: Option<Uint128> = shit.app.wrap().query_wasm_smart(
    //     shitstrap.clone(),
    //     &QueryMsg::ShitRate {
    //         asset: "uatom".to_string(),
    //     },
    // )?;
    // // calulate expected
    // let dec = Decimal::from_atomics(shit_rate.unwrap(), MAX_DEC_PRECISION)?;
    // let calculated = balance
    //     .amount
    //     .multiply_ratio(dec.numerator(), dec.denominator());
    // assert_eq!(calculated, Uint256::new(first_deposit));

    // shitstrap reaches limit
    shit.app.execute_contract(
        s1.clone(),
        shitstrap.clone(),
        &ExecuteMsg::ShitStrap {
            shit: AssetUnchecked::from_native("uatom", 2_000_000u128),
            dao: None,
            recp: None,
        },
        &vec![coin(2000000u128, "uatom")],
    )?;

    // move forward in time
    let mut block = shit.app.block_info();
    block.height += 1;
    shit.app.set_block(block);

    // confirm contract will not continue to shitstrap
    let res: bool = shit
        .app
        .wrap()
        .query_wasm_smart(shit.shitstrap, &QueryMsg::FullOfShit {})?;
    assert_eq!(res, true);

    // confirm balances
    let balance = shit.app.wrap().query_balance(shitstrap.clone(), "uatom")?;
    assert_eq!(balance.amount, Uint256::zero()); // 1 token is waiting to be redeemed by last shit strapper
                                                 // assert_eq!(balance.amount, Uint256::new(1_000_000u128)); // 1 token is waiting to be redeemed by last shit strapper
    let balance = shit.app.wrap().query_balance(s1.clone(), "uatom")?;
    assert_eq!(balance.amount, Uint256::new(777_000_000u128));
    let owner_bal = shit.app.wrap().query_balance(shit.owner, "uatom")?;
    assert_eq!(owner_bal.amount, Uint256::new(1_222_000_000u128)); // owner received 222 ATOM

    // no more shitstrapping can commence
    let err = shit
        .app
        .execute_contract(
            s1.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked::from_native("uatom", 2_000_000u128),
                dao: None,
                recp: None,
            },
            &vec![coin(2_000_000u128, "uatom")],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::FullOfShit {}.to_string()));

    // move forward in time
    let mut block = shit.app.block_info();
    block.height += 1;
    shit.app.set_block(block);

    // should have 1 extra token sent back
    let balance = shit.app.wrap().query_balance(s1.clone(), "uatom")?;
    assert_eq!(balance.amount, Uint256::new(778_000_000u128));
    let balance = shit.app.wrap().query_balance(shitstrap.clone(), "uatom")?;
    assert_eq!(balance.amount, Uint256::zero()); // 1 token is no longer waiting to be redeemed by last shit strapper
    Ok(())
}

#[test]
fn test_fee_destination() -> TestResult {
    let mut app = App::default();
    let initer = app.api().addr_make("eh");
    let cw20_id = app.store_code(cw20_contract());
    let cw20 = &init_bad_cw20(&mut app, cw20_id, initer).to_string();
    // create testing suite
    let first_deposit = 100_000_000u128; // 100
    let cw20_shit_ratio = Uint128::from(640000000000000000u128); // 64%
    let atom_shit_ratio = Uint128::from(360000000000000000u128); // 36%
    let mut shit = default_init(
        app,
        vec![
            PossibleShit::native_denom("uatom", atom_shit_ratio.clone().into()),
            PossibleShit::native_cw20(cw20, cw20_shit_ratio.clone().into()),
        ],
        222000000u128,
    );

    let shitstrap = shit.shitstrap.clone();
    let s1 = shit.shitters[0].clone(); // atom
    let s2 = shit.shitters[1].clone(); //shit
    shit.setup_default_funds(shitstrap.clone())?;
    let dec = Decimal::from_atomics(atom_shit_ratio, MAX_DEC_PRECISION)?;
    let first = Uint128::new(first_deposit).multiply_ratio(dec.numerator(), dec.denominator());

    shit.app
        .sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: shitstrap.to_string(),
            amount: vec![coin(222000000u128, "ushit")],
        }))
        .unwrap();

    // assert shitstrap default balance
    let res = shit.app.wrap().query_balance(shitstrap.clone(), "ushit")?;
    assert_eq!(
        res.amount,
        Uint256::new(222000000u128 + 1_000_000_000_000u128)
    );

    // user 1 funds with native
    let res = shit.participate_native(&s1.to_string(), 100_000_000, "uatom")?;

    // confirm shit_rate is calculated correctly
    let has_shit: Uint128 = shit
        .app
        .wrap()
        .query_wasm_smart(shitstrap.clone(), &QueryMsg::HasShit {})?;
    assert_eq!(has_shit, first);

    // confirm funds made it to shitter
    let s1_uatom = shit.app.wrap().query_balance(&s1, "uatom")?;
    let s1_ushit = shit.app.wrap().query_balance(&s1, "ushit")?;
    assert_eq!(
        s1_uatom.amount,
        Uint256::new(DEFAULT_BALANCE - first_deposit)
    );
    assert_eq!(s1_ushit.amount, Uint256::from(first));

    // confirm funds are going to shitstrap owner
    let strap_uatom = shit.app.wrap().query_balance(shitstrap.clone(), "uatom")?;
    let strap_ushit = shit.app.wrap().query_balance(shitstrap.clone(), "ushit")?;
    assert_eq!(strap_uatom.amount, Uint256::zero());
    assert_eq!(
        strap_ushit.amount,
        Uint256::new(1_000_000_000_000u128 + (222000000u128 - first.u128()))
    );

    // // end shitstrap early
    // let res = shit.app.execute_contract(
    //     shit.owner.clone(),
    //     shitstrap.clone(),
    //     &ExecuteMsg::Flush {},
    //     &[],
    // )?;

    // confirm uatom was sent with bank
    res.assert_event(
        &Event::new("transfer")
            .add_attribute("recipient", shit.owner.to_string())
            .add_attribute("sender", shitstrap.to_string())
            .add_attribute(
                "amount",
                (222000000u128 - first.u128()).to_string() + "ushit",
            ),
    );
    res.assert_event(
        &Event::new("transfer")
            .add_attribute("recipient", shit.owner.to_string())
            .add_attribute("sender", shitstrap.to_string())
            .add_attribute("amount", first_deposit.to_string() + "uatom"),
    );

    // // confirm shitstrap balance is empty
    // let strap_uatom = shit.app.wrap().query_balance(shitstrap.clone(), "uatom")?;
    // let strap_ushit = shit.app.wrap().query_balance(shitstrap.clone(), "ushit")?;
    // assert_eq!(strap_uatom.amount, Uint256::zero());
    // assert_eq!(strap_ushit.amount, Uint256::new(1_000_000_000_000u128));

    // // confirm shistrap owner now has updated balance
    // let owner_uatom = shit.app.wrap().query_balance(&shit.owner, "uatom")?;
    // let owner_ushit = shit.app.wrap().query_balance(&shit.owner, "ushit")?;
    // assert_eq!(
    //     owner_uatom.amount,
    //     Uint256::new(DEFAULT_BALANCE + first_deposit)
    // );
    // assert_eq!(
    //     owner_ushit.amount,
    //     Uint256::new(222000000u128 - first.u128())
    // );

    Ok(())
}

#[test]
fn test_mult_participants_mult_possible_shit() -> TestResult {
    let mut app: App = App::default();
    let initer = app.api().addr_make("eh");
    let cw20_id = app.store_code(cw20_contract());
    let cw20 = &init_bad_cw20(&mut app, cw20_id, initer).to_string();
    // create testing suite
    let first_deposit = 100_000_000u128; // 100
    let cw20_shit_ratio = Uint128::from(640000000000000000u128); // 64%
    let atom_shit_ratio = Uint128::from(360000000000000000u128); // 36%

    let mut shit = default_init(
        app,
        vec![
            PossibleShit::native_denom("uatom", atom_shit_ratio.clone().into()),
            PossibleShit::native_cw20(cw20, cw20_shit_ratio.clone().into()),
        ],
        222000000u128,
    );

    let shitstrap = shit.shitstrap.clone();
    shit.setup_default_funds(shitstrap.clone())?;

    let s1 = shit.shitters[0].clone();
    let s2 = shit.shitters[1].clone();
    let s3 = shit.shitters[2].clone();

    // error with wrong native token
    let err = shit
        .app
        .execute_contract(
            s3.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                shit: AssetUnchecked::from_native("usilk", first_deposit),
                dao: None,
                recp: None,
            },
            &vec![coin(first_deposit, "usilk")],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::WrongShit {}.to_string()));

    // create another cw20
    let cw20_init = Cw20Init {
        name: "poo".into(),
        symbol: "POO".into(),
        decimals: 6,
        initial_balances: vec![
            Cw20Coin {
                address: s2.to_string(),
                amount: 1000u128.into(),
            },
            Cw20Coin {
                address: s3.to_string(),
                amount: 1000u128.into(),
            },
        ],
        mint: None,
        marketing: None,
    };

    let bad_cw20 = shit
        .app
        .instantiate_contract(
            1u64,
            shit.owner.clone(),
            &cw20_init,
            &[],
            "cw20",
            Some(shit.owner.to_string()),
        )
        .unwrap();

    // error with wrong cw20
    let err = shit
        .app
        .execute_contract(
            bad_cw20,
            shitstrap.clone(),
            &ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: s2.to_string(),
                amount: 200u128.into(),
                msg: to_json_binary(&ReceiveMsg::ShitStrap {
                    shitter: s2.to_string(),
                    dao: None,
                    recp: None,
                })
                .unwrap(),
            }),
            &vec![],
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&ContractError::WrongShit {}.to_string()));

    // user 1 funds with native
    shit.participate_native(&s1.to_string(), 100_000_000, "uatom")?;
    // confirm shit_rate is calculated correctly
    let res: Uint128 = shit
        .app
        .wrap()
        .query_wasm_smart(shitstrap.clone(), &QueryMsg::HasShit {})?;
    let dec = Decimal::from_atomics(atom_shit_ratio, MAX_DEC_PRECISION)?;
    assert_eq!(
        res,
        Uint128::new(first_deposit).multiply_ratio(dec.numerator(), dec.denominator())
    );
    // confirm funds made it to shitter
    let dec = Decimal::from_atomics(atom_shit_ratio, MAX_DEC_PRECISION)?;
    let s1_uatom = shit.app.wrap().query_balance(&s1, "uatom")?;
    let s1_ushit = shit.app.wrap().query_balance(&s1, "ushit")?;
    assert_eq!(
        s1_uatom.amount,
        Uint256::new(DEFAULT_BALANCE - first_deposit)
    );
    assert_eq!(
        s1_ushit.amount,
        Uint256::from(
            Uint128::new(first_deposit).multiply_ratio(dec.numerator(), dec.denominator())
        )
    );

    // user 2 funds with coin. should reflect 50% shit weight of native
    shit.participate_cw20(&s3.to_string(), 100_000_000, cw20)?;

    // confirm shit_rate is calculated correctly
    let res: Uint128 = shit
        .app
        .wrap()
        .query_wasm_smart(shitstrap.clone(), &QueryMsg::HasShit {})?;
    let dec = Decimal::from_atomics(cw20_shit_ratio, MAX_DEC_PRECISION)?;
    let dec2 = Decimal::from_atomics(atom_shit_ratio, MAX_DEC_PRECISION)?;
    // the expected shit_strapped, after 2 participants
    let expected = (Uint128::new(first_deposit).multiply_ratio(dec.numerator(), dec.denominator()))
        + (Uint128::new(first_deposit).multiply_ratio(dec2.numerator(), dec2.denominator()));

    assert_eq!(res, expected);

    // confirm native token balance is correct
    let dec = Decimal::from_atomics(cw20_shit_ratio, MAX_DEC_PRECISION)?;
    let s3_ushit = shit.app.wrap().query_balance(&s3, "ushit")?;
    let s3_usilk = shit.app.wrap().query_balance(&s3, "usilk")?;
    assert_eq!(
        s3_ushit.amount,
        Uint256::from(
            Uint128::new(first_deposit).multiply_ratio(dec.numerator(), dec.denominator())
        )
    );
    assert_eq!(s3_usilk.amount, Uint256::new(DEFAULT_BALANCE)); // has full balance of non accepted token
                                                                // we skip checking cw20 balance in this test, done in next step.
    Ok(())
}

// test shit strap w/ cw20, not using cw20 recieve
#[test]
fn test_cw20_receive() -> TestResult {
    let mut app = App::default();
    let initer = app.api().addr_make("eh");
    let cw20_id = app.store_code(cw20_contract());
    let cw20 = &init_bad_cw20(&mut app, cw20_id, initer).to_string();
    let mut shit = default_init(app, vec![PossibleShit::native_cw20(cw20, 100u128)], 222u128);

    let first_deposit = 100u128;

    let shitstrap = shit.shitstrap.clone();
    let s1 = shit.shitters[0].clone();
    let s2 = shit.shitters[1].clone();
    shit.setup_default_funds(shitstrap.clone())?;

    // cannot directly call shit_strap entry point with cw20
    let err = shit
        .app
        .execute_contract(
            s1.clone(),
            shitstrap.clone(),
            &ExecuteMsg::ShitStrap {
                recp: None,
                shit: AssetUnchecked {
                    denom: cw_shit_denom::UncheckedDenom::Cw20(cw20.clone()),
                    amount: first_deposit.into(),
                },
                dao: None,
            },
            &vec![],
        )
        .unwrap_err();

    assert!(err
        .to_string()
        .contains(&ContractError::ShittyCw20 {}.to_string()));

    // only cw20 can call receive entry point
    let err = shit
        .app
        .execute_contract(
            s2.clone(),
            shitstrap.clone(),
            &ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: s2.to_string(),
                amount: first_deposit.into(),
                msg: to_json_binary(&ReceiveMsg::ShitStrap {
                    shitter: s2.to_string(),
                    dao: None,
                    recp: None,
                })
                .unwrap(),
            }),
            &vec![],
        )
        .unwrap_err();

    assert!(err
        .to_string()
        .contains(&ContractError::WrongShit {}.to_string()));

    Ok(())
}
