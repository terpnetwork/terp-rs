use abstract_cw_multi_test::Contract;
use anyhow::anyhow;
use cosmwasm_std::{coin, coins, Decimal, Event, Fraction, HexBinary, Uint128};
use cw721::EmptyOptionalCollectionExtension;
use cw_infuser::{
    msg::{ExecuteMsg, ExecuteMsgFns, InstantiateMsg, QueryMsgFns},
    state::Config,
    AnyOfErr, ContractError,
};
use cw_infusions::{
    bundles::{Bundle, BundleType},
    nfts::{InfusedCollection, NFT},
    state::{EligibleNFTCollection, Infusion, InfusionParamState},
    wavs::WavsBundle,
};
use std::{error::Error, str::FromStr};
// Use prelude to get all the necessary imports
use cw_infuser::interface::CwInfuser;
use cw_orch::{anyhow, prelude::*};

fn cw721_contract() -> Box<dyn Contract<Empty>> {
    let contract = ContractWrapper::new(
        cw721_base::entry::execute,
        cw721_base::entry::instantiate,
        cw721_base::entry::query,
    );
    Box::new(contract)
}

// fn v020_infusion() -> Box<dyn Contract<Empty>> {
//     let contract = ContractWrapper::new(
//         v020infuse::contract::execute,
//         v020infuse::contract::instantiate,
//         v020infuse::contract::query,
//     )
//     .with_reply(v020infuse::contract::reply)
//     .with_migrate(v020infuse::contract::migrate);
//     Box::new(contract)
// }

// minimal infuser
pub struct InfuserTestSuite<Chain> {
    pub chain: MockBech32,
    pub infuser: CwInfuser<Chain>,
    pub nfts: Vec<Addr>,
    pub infusion: Infusion,
    pub admin: Addr,
    pub wavs_service: Addr,
    pub payment_recipient: Addr,
}

impl<Chain: CwEnv> InfuserTestSuite<Chain> {
    fn default_infused_collection() -> anyhow::Result<InfusedCollection> {
        Ok(InfusedCollection {
            addr: None,
            admin: None,
            description: "test-description".to_string(),
            name: "test-name".to_string(),
            symbol: "TEST".to_string(),
            base_uri: "ipfs://bafybeidhcxcxolehykzlmmfxzcu5tr2bi4p5yaz7a2s6vsdyqkr25ykkku"
                .to_string(),
            num_tokens: 100,
            sg: false,
            royalty_info: None,
            start_trading_time: None,
            explicit_content: None,
            external_link: None,
            image: "ipfs://bafybeidhcxcxolehykzlmmfxzcu5tr2bi4p5yaz7a2s6vsdyqkr25ykkku".to_string(),
        })
    }

    fn default_good_nfts(&self, nfts: &Vec<Addr>) -> anyhow::Result<Vec<EligibleNFTCollection>> {
        let good_nfts = vec![
            EligibleNFTCollection {
                addr: nfts[0].clone(),
                min_req: 1,
                max_req: None,
                payment_substitute: None,
            },
            EligibleNFTCollection {
                addr: nfts[1].clone(),
                min_req: 1,
                max_req: None,
                payment_substitute: None,
            },
        ];
        Ok(good_nfts)
    }

    fn mint_and_approve_helper(
        chain: MockBech32,
        infuser: Addr,
        admin: Addr,
        nft_collection_addrs: Vec<Addr>,
        nft_count: u64,
    ) -> anyhow::Result<()> {
        for i in nft_collection_addrs.clone() {
            // mint 11 nfts?
            for n in 0..nft_count {
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Mint {
                        token_id: n.to_string(),
                        owner: chain.sender.to_string(),
                        token_uri: None,
                        extension: None,
                    };
                chain.execute(msg, &[], &i.clone())?;
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Approve {
                        spender: infuser.to_string(),
                        token_id: n.to_string(),
                        expires: None,
                    };
                // approve infuser for nft
                chain.execute(msg, &[], &i.clone())?;
            }
            for n in 11..21 {
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Mint {
                        token_id: n.to_string(),
                        owner: admin.to_string(),
                        token_uri: None,
                        extension: None,
                    };
                chain.execute(msg, &[], &i.clone())?;
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Approve {
                        spender: infuser.to_string(),
                        token_id: n.to_string(),
                        expires: None,
                    };
                // approve infuser for nft
                chain.call_as(&admin).execute(msg, &[], &i.clone())?;
            }
        }
        Ok(())
    }

    fn default_nft_instantiate(
        chain: MockBech32,
        cw721_code: u64,
        minter: Addr,
        withdraw: Addr,
    ) -> anyhow::Result<Vec<Addr>> {
        let mut nft_collection_addrs = vec![];

        for i in 0..4 {
            let msg_a = chain.instantiate(
                cw721_code,
                &cw721::msg::Cw721InstantiateMsg::<EmptyOptionalCollectionExtension> {
                    name: "good-chronic".to_string(),
                    symbol: "CHRONIC-".to_owned() + i.to_string().as_str(),
                    minter: Some(minter.to_string()),
                    collection_info_extension: None,
                    creator: Some(minter.to_string()),
                    withdraw_address: Some(withdraw.to_string()),
                },
                Some("cw721-base-good-chronic"),
                None,
                &[],
            )?;
            let cw721_a = msg_a
                .instantiated_contract_address()
                .map_err(|e| anyhow!(e.to_string()))?;
            println!("test nft collection: {:#?},{:#?}", i, cw721_a.to_string());
            nft_collection_addrs.push(cw721_a);
        }
        Ok(nft_collection_addrs)
    }

    fn default_nft_approvals(
        chain: MockBech32,
        nfts: Vec<Addr>,
        infuser: Addr,
        admin: Addr,
    ) -> anyhow::Result<()> {
        // approve nfts for new infusion
        for i in nfts.clone() {
            for n in 0..10 {
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Approve {
                        spender: infuser.to_string(),
                        token_id: n.to_string(),
                        expires: None,
                    };
                // approve infuser for nft
                chain.execute(msg, &[], &i.clone())?;
            }
            // mint nfts to contract owner
            for n in 11..21 {
                let msg: &cw721::msg::Cw721ExecuteMsg<Option<Empty>, Empty, Empty> =
                    &cw721::msg::Cw721ExecuteMsg::Approve {
                        spender: infuser.to_string(),
                        token_id: n.to_string(),
                        expires: None,
                    };
                // approve infuser for nft
                chain.call_as(&admin).execute(msg, &[], &i.clone())?;
            }
        }
        Ok(())
    }

    fn setup_fee_suite(
        bundle_type: BundleType,
        nft_owners: Vec<Addr>,
        wavs_public_key: bool,
    ) -> anyhow::Result<InfuserTestSuite<MockBech32>> {
        // setup infuser with admin fees
        let env = InfuserTestSuite::<MockBech32>::setup()?;
        let payment_recipient = env
            .chain
            .addr_make_with_balance("payment-recipient", vec![])?;

        // create new infusion contract
        env.infuser.instantiate(
            &InstantiateMsg {
                contract_owner: Some(env.admin.to_string()),
                owner_fee: Decimal::from_str("0.1").map_err(|e| anyhow!(e.to_string()))?,
                min_creation_fee: Some(coin(500u128, "uterp")),
                min_infusion_fee: Some(coin(100u128, "uterp")),
                min_per_bundle: None,
                max_per_bundle: None,
                max_bundles: None,
                max_infusions: None,
                cw721_code_id: 2u64,
                wavs_public_key: match wavs_public_key {
                    true => Some(env.wavs_service.to_string()),
                    false => None,
                },
            },
            Some(&env.admin.clone()),
            &[],
        )?;
        let nft1 = env.nfts[0].clone();
        let nft2 = env.nfts[1].clone();

        let good_nfts = env.default_good_nfts(&vec![nft1, nft2])?;
        InfuserTestSuite::<MockBech32>::default_nft_approvals(
            env.chain.clone(),
            env.nfts.clone(),
            env.infuser.address()?,
            env.admin.clone(),
        )?;

        let mut infusion_params = InfusionParamState {
            params: None,
            mint_fee: None,
            bundle_type,
            wavs_enabled: false,
        };

        let good_infused = InfuserTestSuite::<MockBech32>::default_infused_collection()?;
        // ensure fee set is within contract level bounds
        infusion_params.mint_fee = Some(coin(100, "uterp"));

        let infusion = Infusion {
            collections: good_nfts.clone(),
            infused_collection: good_infused.clone(),
            infusion_params,
            payment_recipient: Some(payment_recipient.clone()),
            owner: None,
            description: Some("testewates".to_string()),
        };
        Ok(InfuserTestSuite {
            chain: env.chain,
            infuser: env.infuser,
            nfts: env.nfts,
            infusion,
            admin: env.admin,
            payment_recipient,
            wavs_service: env.wavs_service,
        })
    }

    // setsup the infuser suite by storing, instantiating, and configuring nft collections & the cw-infuser
    fn setup() -> anyhow::Result<InfuserTestSuite<MockBech32>> {
        let mock = MockBech32::new("mock");
        let sender = mock.sender_addr();
        let admin = mock.addr_make_with_balance("admin", coins(1000000000, "uterp"))?;
        mock.add_balance(&sender, coins(100000000, "ubtsg"))?;
        mock.add_balance(&sender, coins(100000000, "uterp"))?;
        let treasury = mock.addr_make("treasury");
        let wavs_service = mock.addr_make("wavs_service");

        // latest infusion version
        let infuser = CwInfuser::new(mock.clone());

        // store cw-infuser
        infuser.upload()?;

        // store cw721
        let cw721 = cw721_contract();
        let cw721_code_id = mock
            .upload_custom("cw721", cw721)?
            .uploaded_code_id()
            .map_err(|e| anyhow!(e.to_string()))?;
        let nft_collection_addrs = InfuserTestSuite::<MockBech32>::default_nft_instantiate(
            mock.clone(),
            cw721_code_id,
            sender.clone(),
            treasury.clone(),
        )?;

        let default_init = InstantiateMsg {
            contract_owner: Some(admin.to_string()),
            max_bundles: None,
            max_infusions: None,
            max_per_bundle: None,
            min_per_bundle: None,
            cw721_code_id,
            owner_fee: Decimal::zero(),
            min_creation_fee: None,
            min_infusion_fee: None,
            wavs_public_key: Some(wavs_service.to_string()),
        };

        // create cw-infsion app
        infuser.instantiate(&default_init, None, &[])?;

        // mint
        InfuserTestSuite::<MockBech32>::mint_and_approve_helper(
            mock.clone(),
            infuser.address()?.clone(),
            admin.clone(),
            nft_collection_addrs.clone(),
            10,
        )?;
        let mut infusion = Infusion {
            collections: vec![],
            infused_collection: InfuserTestSuite::<MockBech32>::default_infused_collection()?,
            infusion_params: InfusionParamState {
                params: None,
                mint_fee: None,
                bundle_type: BundleType::AllOf {},
                wavs_enabled: false,
            },
            payment_recipient: Some(treasury.clone()),
            owner: Some(admin.clone()),
            description: Some("testewates".to_string()),
        };

        for i in nft_collection_addrs.clone() {
            let nft_collection = EligibleNFTCollection {
                addr: i.clone(),
                min_req: 2,
                max_req: None,
                payment_substitute: None,
            };
            infusion.infused_collection.name =
                infusion.infused_collection.name + &i.to_string().to_owned();
            infusion.collections = [nft_collection].to_vec();
            // create infusion
            infuser.execute(
                &ExecuteMsg::CreateInfusion {
                    infusions: vec![infusion.clone()],
                },
                &[],
            )?;
            mock.next_block()?;
        }
        Ok(InfuserTestSuite {
            chain: mock,
            infuser,
            nfts: nft_collection_addrs,
            infusion,
            admin,
            payment_recipient: treasury,
            wavs_service,
        })
    }
}

#[test]
fn test_successful_install() -> anyhow::Result<()> {
    let env = InfuserTestSuite::<MockBech32>::setup()?;
    let app = env.infuser;

    let config = app.config()?;
    assert_eq!(
        config,
        Config {
            latest_infusion_id: 4,
            contract_owner: env.admin,
            max_infusions: 2u64,
            min_per_bundle: 1u64,
            max_per_bundle: 10u64,
            max_bundles: 5u64,
            code_id: 2,
            code_hash: HexBinary::from_hex(
                "7e961e9369f7a3619b102834beec5bc2463f9008b40de972c91c45e3b300a805"
            )
            .map_err(|e| anyhow!(e.to_string()))?,
            owner_fee: Decimal::zero(),
            min_creation_fee: None,
            min_infusion_fee: None,
        }
    );
    Ok(())
}

#[test]
fn test_successful_infusion() -> anyhow::Result<()> {
    let env = InfuserTestSuite::<MockBech32>::setup()?;
    let app = env.infuser;

    // infuse
    app.infuse(
        vec![Bundle {
            nfts: vec![
                NFT {
                    addr: env.nfts[0].clone(),
                    token_id: 1,
                },
                NFT {
                    addr: env.nfts[0].clone(),
                    token_id: 3,
                },
            ],
        }],
        1,
    )?;
    // println!("{:#?}", res);

    // confirm infused collection mint
    let res = app.infusion_by_id(1)?;
    assert!(
        res.infused_collection.addr.is_some(),
        "infusion collection not set!"
    );
    env.chain.wait_blocks(1)?;

    // error if too few nfts provided in bundle
    let err = app.infuse(vec![], 1).unwrap_err();
    assert!(err.to_string().contains(&"Bundle cannot be empty."));

    // error if too many nfts provided in bundle
    let err = app
        .infuse(
            vec![Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 4,
                    },
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 6,
                    },
                ],
            }],
            1,
        )
        .unwrap_err();
    println!("{:#?}", err);
    assert!(err.to_string().contains(
        &ContractError::BundleNotAccepted {
            have: 3,
            want: 2,
            addr: env.nfts[0].to_string(),
        }
        .to_string()
    ),);

    // assert queries
    let res = app.infusion_by_id(1)?;
    assert_eq!(res.collections.len(), 1);
    assert_eq!(res.collections[0].min_req, 2);

    // todo: finish asserting all queries work as expected
    Ok(())
}

// Multiple Collections In Bundle
#[test]
fn test_allof_infuse_multiple_collections_in_bundle() -> anyhow::Result<()> {
    let env = InfuserTestSuite::<MockBech32>::setup()?;
    let app = env.infuser.clone();

    let bad_nfts = vec![
        EligibleNFTCollection {
            addr: env.nfts[0].clone(),
            min_req: 2,
            max_req: None,
            payment_substitute: None,
        },
        EligibleNFTCollection {
            addr: env.nfts[0].clone(),
            min_req: 4,
            max_req: None,
            payment_substitute: None,
        },
    ];

    let good_nfts = vec![
        EligibleNFTCollection {
            addr: env.nfts[0].clone(),
            min_req: 2,
            max_req: None,
            payment_substitute: None,
        },
        EligibleNFTCollection {
            addr: env.nfts[1].clone(),
            min_req: 4,
            max_req: None,
            payment_substitute: None,
        },
    ];

    let good_infused = InfuserTestSuite::<MockBech32>::default_infused_collection()?;
    let good_infusion_params = InfusionParamState {
        mint_fee: None,
        params: None,
        bundle_type: BundleType::AllOf {},
        wavs_enabled: false,
    };

    let mut infusion = Infusion {
        collections: good_nfts.clone(),
        infused_collection: good_infused.clone(),
        infusion_params: good_infusion_params,
        payment_recipient: Some(env.chain.sender.clone()),
        owner: None,
        description: Some("testewates".to_string()),
    };

    // cannot provide same nft collection twice
    infusion.collections = bad_nfts;

    assert!(app
        .create_infusion(vec![infusion.clone()])
        .unwrap_err()
        .to_string()
        .contains(&ContractError::DuplicateCollectionInInfusion.to_string()));
    infusion.collections = good_nfts;

    // create infusion accepting nft collection 1 & 2
    let res = app.create_infusion(vec![infusion.clone()])?;
    let infusion_id = Uint128::from_str(
        &res.event_attr_value("wasm", "infusion-id")
            .map_err(|e| anyhow!(e.to_string()))?,
    )
    .unwrap()
    .u128() as u64;
    // assert bundle with collection 1 & 3 errors
    // infuse
    assert!(app
        .infuse(
            vec![Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 3,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 1,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 3,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 4,
                    },
                ],
            }],
            infusion_id,
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: env.nfts[1].to_string(),
                bun_type: 1,
                wavs: false,
                min_req: 4,
            }
            .to_string()
        ),);

    // assert bundle with collection 2 & 3 errors
    assert!(app
        .infuse(
            vec![Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 8,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 9,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 7,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 6,
                    },
                ],
            }],
            infusion_id,
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: env.nfts[0].to_string(),
                bun_type: 1,
                wavs: false,
                min_req: 2,
            }
            .to_string()
        ));
    // assert right number of collections for each bundle

    // assert bundle with collection 2 & 3 errors
    assert!(app
        .infuse(
            vec![Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 1,
                    },
                    NFT {
                        addr: env.nfts[2].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 1,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 3,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 4,
                    },
                ],
            }],
            infusion_id,
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: env.nfts[0].to_string(),
                bun_type: 1,
                wavs: false,
                min_req: 2,
            }
            .to_string()
        ),);
    env.chain.wait_blocks(1)?;

    // good infusion
    let res = app.infuse(
        vec![
            Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 1,
                    },
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 1,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 2,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 3,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 4,
                    },
                ],
            },
            Bundle {
                nfts: vec![
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 3,
                    },
                    NFT {
                        addr: env.nfts[0].clone(),
                        token_id: 4,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 5,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 6,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 7,
                    },
                    NFT {
                        addr: env.nfts[1].clone(),
                        token_id: 8,
                    },
                ],
            },
        ],
        infusion_id,
    )?;

    // good infusion
    // app.infuse(vec![], infusion_id.clone())?;

    //try to infuse again immediately
    Ok(())
}

#[test]
fn test_allof_eligible_nft_collections() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let env = InfuserTestSuite::<MockBech32>::setup()?;
    let app = env.infuser.clone();
    let not_nft = env.chain.addr_make("mock-nft");
    println!("{:#?}", not_nft);
    let bad_nfts = vec![
        EligibleNFTCollection {
            addr: env.nfts[0].clone(),
            min_req: 2,
            max_req: None,
            payment_substitute: None,
        },
        EligibleNFTCollection {
            addr: not_nft.clone(),
            min_req: 4,
            max_req: None,
            payment_substitute: None,
        },
    ];
    let good_infused = InfuserTestSuite::<MockBech32>::default_infused_collection()?;
    let good_infusion_params = InfusionParamState {
        mint_fee: None,
        params: None,
        bundle_type: BundleType::AllOf {},
        wavs_enabled: true,
    };

    let infusion = Infusion {
        collections: bad_nfts.clone(),
        infused_collection: good_infused.clone(),
        infusion_params: good_infusion_params,
        payment_recipient: Some(env.chain.sender),
        owner: None,
        description: Some("testewates".to_string()),
    };
    // we dont enorce until sg-72
    // assert_eq!(
    //     app.create_infusion(vec![infusion.clone()])
    //         .unwrap_err()
    //         .root()
    //         .to_string(),
    //     ContractError::AddrIsNotNFTCol {
    //         addr: not_nft.to_string()
    //     }
    //     .to_string()
    // );
    Ok(())
}

// Correct Fees & Destination
#[test]
fn test_correct_fees() -> anyhow::Result<()> {
    let env = InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;

    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();

    // err when sending less fees than required
    assert!(app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(499, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::RequirednfusionFeeError {
                fee: coin(500, "uterp")
            }
            .to_string()
        ));

    // err on more than required mint fee set
    assert!(app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(501, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::RequirednfusionFeeError {
                fee: coin(500, "uterp")
            }
            .to_string()
        ));

    // good infusion creation

    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .unwrap()
    .u128() as u64;

    let mut bundle = Bundle {
        nfts: vec![
            NFT {
                addr: nft1.clone(),
                token_id: 1,
            },
            NFT {
                addr: nft2.clone(),
                token_id: 1,
            },
        ],
    };

    // ensure fee is required when infusing
    // wrong amount
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(1, "uterp")]
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::FeeNotAccepted {
                have: coin(1, "uterp"),
                want: coin(1, "uterp")
            }
            .to_string()
        ),);

    // wrong token
    assert!(env
        .chain
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(1, "uterp")],
            &app.address()?,
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::FeeNotAccepted {
                have: coin(1, "uterp"),
                want: coin(1, "uterp")
            }
            .to_string()
        ),);

    // ensure fee is required when infusing
    bundle.nfts[0].token_id = 12;
    bundle.nfts[1].token_id = 12;
    let infuse = app.call_as(&env.admin).execute(
        &ExecuteMsg::Infuse {
            id: infusion_id,
            bundle: vec![bundle.clone()],
        },
        &[coin(100, "uterp")],
    );
    assert!(infuse.is_ok());

    bundle.nfts[0].token_id = 13;
    bundle.nfts[1].token_id = 13;
    env.chain.wait_blocks(1)?;

    // filter events to ensure fees goes to the correct place
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp")],
        )?
        .events;

    let fee_transfers = res
        .iter()
        .filter(|e| e.ty == "transfer")
        .collect::<Vec<&Event>>();
    println!("fee_transfers: {:#?}", fee_transfers);
    assert_eq!(fee_transfers.len(), 2);

    // for attribute with env.sender as recipient key value
    let admin_payment = fee_transfers
        .clone()
        .into_iter()
        .find(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == "recipient" && a.value == env.admin.to_string())
        })
        .expect("infusion creation fees were not sent to the correct destination. Should have gone to the contract owner")
        .attributes
        .iter()
        .find(|a| a.key == "amount")
        .expect("incorrect amount of tokens were sent to the contract owner for infusion creation fees.");
    assert_eq!(admin_payment.value, "10uterp".to_string());

    // for attribute with env.sender as recipient key value
    let infusion_owner_payment = fee_transfers
        .into_iter()
        .find(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == "recipient" && a.value == env.payment_recipient.to_string())
        })
        .expect("No admin event found")
        .attributes
        .iter()
        .find(|a| a.key == "amount")
        .expect("No amount attribute found");

    assert_eq!(infusion_owner_payment.value, "90uterp".to_string());
    env.chain.wait_blocks(1)?;

    // ensure admin is ommitted from creation fee validation
    app.call_as(&env.admin).execute(
        &ExecuteMsg::CreateInfusion {
            infusions: vec![env.infusion.clone()],
        },
        &[],
    )?;

    Ok(())
}

// Correct Trait Requirement Logic
#[test]
fn test_anyof_infuse_multiple_collections_in_bundle() -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn test_anyof_eligible_nft_collections() -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn test_allof_infusion_fee() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;
    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();
    let nft3 = env.nfts[2].clone();

    // update infusion bundle type

    env.infusion.collections[0].min_req = 2;
    // update substitute payment for infusion being created to 200uterp
    env.infusion.collections[1].payment_substitute = Some(coin(200u128, "uterp"));

    // good infusion creation
    let infusion_id = app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?;

    let infusion_id = Uint128::from_str(&infusion_id)
        .map_err(|e| anyhow!(e.to_string()))?
        .u128() as u64;

    // test bundle setups
    let mut bundle = Bundle { nfts: vec![] };

    // error on incorrect fee payment substitute for anyOf collection.
    bundle.nfts = vec![
        NFT {
            addr: nft1.clone(),
            token_id: 11,
        },
        NFT {
            addr: nft1.clone(),
            token_id: 12,
        },
    ];

    // error on incorrect nft collections
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: env.nfts[1].to_string(),
                have: coin(0, "uterp").into(),
                want: coin(200u128, "uterp")
            }
            .to_string()
        ),);

    // add ineligible nft collection to bundle
    bundle.nfts.push(NFT {
        addr: env.nfts[2].clone(),
        token_id: 11,
    });

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(500, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::NftIsNotEligible {
                col: env.nfts[2].to_string(),
            }
            .to_string()
        ));
    bundle.nfts.pop();

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("event attribute values for infusion: {:#?}", res);

    assert_eq!(res.len(), 3);
    assert_eq!(res[0], "burn");
    assert_eq!(res[1], "burn");
    assert_eq!(res[2], "mint");

    bundle.nfts = vec![
        NFT {
            addr: env.nfts[0].clone(),
            token_id: 13,
        },
        NFT {
            addr: env.nfts[0].clone(),
            token_id: 14,
        },
    ];

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(50, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::FeeNotAccepted {
                have: coin(1, "uterp"),
                want: coin(1, "uterp")
            }
            .to_string()
        ));

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("event attribute values for infusion: {:#?}", res);
    assert_eq!(res.len(), 3);
    assert_eq!(res[0], "burn");
    assert_eq!(res[1], "burn");
    assert_eq!(res[2], "mint");

    env.chain.wait_blocks(1)?;
    // good infusion creation
    env.infusion.collections[1].payment_substitute = None;

    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    let mut bundle = Bundle { nfts: vec![] };

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                bun_type: 1,
                col: env.nfts[0].to_string(),
                wavs: false,
                min_req: 2,
            }
            .to_string()
        ),);

    bundle.nfts = vec![
        NFT {
            addr: env.nfts[0].clone(),
            token_id: 15,
        },
        NFT {
            addr: env.nfts[0].clone(),
            token_id: 16,
        },
    ];

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                bun_type: 1,
                col: env.nfts[1].to_string(),
                wavs: false,
                min_req: 1,
            }
            .to_string()
        ),);
    bundle.nfts.extend(vec![
        NFT {
            addr: env.nfts[1].clone(),
            token_id: 15,
        },
        NFT {
            addr: env.nfts[2].clone(),
            token_id: 11,
        },
    ]);
    bundle.nfts.pop();

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("event attribute values for infusion: {:#?}", res);
    assert_eq!(res.len(), 4);
    assert_eq!(res[0], "burn");
    assert_eq!(res[1], "burn");
    assert_eq!(res[2], "burn");
    assert_eq!(res[3], "mint");

    // error on incorrect static mint fee
    Ok(())
}

#[test]
fn test_anyof_infusion_fee() -> anyhow::Result<()> {
    let mut bundle_type = BundleType::AnyOf { addrs: vec![] };
    // setup infuser with admin fees
    let mut env = InfuserTestSuite::<MockBech32>::setup_fee_suite(bundle_type, vec![], false)?;
    let app = env.infuser;
    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();
    let nft3 = env.nfts[2].clone();

    // require 2 for first collection
    env.infusion.collections[0].min_req = 2;

    // cannot create w/ empty anyof
    assert!(app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::AnyOfConfigError {
                err: AnyOfErr::Empty
            }
            .to_string()
        ),);

    // cannot use not uneligible collection
    bundle_type = BundleType::AnyOf {
        addrs: vec![nft1.clone(), nft3.clone()],
    };
    env.infusion.infusion_params.bundle_type = bundle_type;

    assert!(app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::AnyOfConfigError {
                err: AnyOfErr::Uneligible
            }
            .to_string()
        ));

    // using correct one works
    bundle_type = BundleType::AnyOf {
        addrs: vec![nft1.clone(), nft2.clone()],
    };
    env.infusion.infusion_params.bundle_type = bundle_type;

    // set fee substitute for only one of them
    env.infusion.collections[1].payment_substitute = Some(coin(200u128, "uterp"));

    // good infusion creation
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;
    assert_eq!(infusion_id, 1);
    // test bundle setups
    let mut bundle = Bundle { nfts: vec![] };

    // let res = app.infusion_by_id(infusion_id)?;
    // println!("INFUSION PARAMS: {:#?}", res);

    // error on incorrect fee payment substitute for anyOf collection.
    bundle.nfts = vec![NFT {
        addr: nft1.clone(),
        token_id: 11,
    }];

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(&ContractError::InvalidAnyOfBundle {}.to_string()));
    bundle.nfts.extend(vec![NFT {
        addr: nft1.clone(),
        token_id: 14,
    }]);
    let b = env.chain.query_balance(&env.admin, "uterp")?;
    println!("{:#?}", b);
    let b = env.chain.query_balance(&app.address()?, "uterp")?;
    println!("{:#?}", b);
    // 1x correct amount nft1, good static fee, incorrect feesub nft2
    let mut infusion_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(150, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // 2 burn, one mint
    assert_eq!(infusion_res.len(), 3);
    assert_eq!(infusion_res[0], "burn");
    assert_eq!(infusion_res[1], "burn");
    assert_eq!(infusion_res[2], "mint");
    // .unwrap_err()
    //
    // .to_string(),
    // ContractError::PaymentSubstituteNotProvided {
    //     col: nft2.to_string(),
    //     have: coin(50u128, "uterp").into(),
    //     want: coin(200u128, "uterp")
    // }
    // .to_string()

    // ensure only 1 nft minted with anyOf satisfied by fee payment
    bundle.nfts = vec![];

    infusion_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("event attribute values for infusion: {:#?}", res);
    assert_eq!(infusion_res.len(), 1);
    assert_eq!(infusion_res[0], "mint");
    // ensure nft is not burnt
    env.chain.wait_blocks(1)?;
    //  ensure 2 nfts minted with anyOf satisfied twice (no-fee-substitute)
    bundle.nfts = vec![
        NFT {
            addr: nft1.clone(),
            token_id: 15,
        },
        NFT {
            addr: nft1.clone(),
            token_id: 16,
        },
    ];
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    assert_eq!(res.len(), 4);
    assert_eq!(res[2], "mint");
    assert_eq!(res[3], "mint");

    env.chain.wait_blocks(1)?;
    //  ensure 1 nft minted if fee subsitute provided for one in anyOf
    bundle.nfts = vec![];
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");

    assert_eq!(res.len(), 1);
    assert_eq!(res[0], "mint");

    // ensure 2 nfts are mintd if 2 fee payments are provided
    env.chain.wait_blocks(1)?;
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    assert_eq!(res.len(), 2);
    assert_eq!(res[0], "mint");
    assert_eq!(res[1], "mint");

    // ensure 3 nfts are mintd if 2 fee payments are provided
    env.chain.wait_blocks(1)?;
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(700, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    assert_eq!(res.len(), 3);
    assert_eq!(res[0], "mint");
    assert_eq!(res[1], "mint");
    assert_eq!(res[2], "mint");

    Ok(())
}

fn test_anyof_blend_infusion_fee() -> anyhow::Result<()> {
    Ok(())
}

// send mult

// 2 collections w/ identical min & fee sub, 2 collections w/out feesub & diff mins
#[test]
fn test_anyof_mix() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;
    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();
    let nft3 = env.nfts[2].clone();
    let nft4 = env.nfts[3].clone();

    env.chain.add_balance(
        &env.admin,
        vec![
            coin(1000000, "uterp"),
            coin(
                2000000u128,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;

    // test bundle setups
    let mut bundle = Bundle { nfts: vec![] };

    // set parameters as infusion #1 for
    env.infusion.collections[0].payment_substitute = Some(coin(100u128, "uterp"));
    env.infusion.collections[0].min_req = 1;
    env.infusion.collections[1].payment_substitute = Some(coin(
        100u128,
        "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
    ));
    env.infusion.infusion_params.mint_fee = Some(coin(2000, "uterp"));
    env.infusion.collections[1].min_req = 1;

    env.infusion.infusion_params.bundle_type = BundleType::AnyOf {
        addrs: vec![nft1.clone(), nft2.clone(), nft3.clone(), nft4.clone()],
    };
    env.infusion.collections.push(EligibleNFTCollection {
        addr: nft3.clone(),
        min_req: 2,
        max_req: None,
        payment_substitute: None,
    });
    env.infusion.collections.push(EligibleNFTCollection {
        addr: nft4.clone(),
        min_req: 4,
        max_req: None,
        payment_substitute: None,
    });

    // create infusion
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    // try to infuse with just nft 1 & the right fee payment
    bundle.nfts.push(NFT {
        addr: nft1.clone(),
        token_id: 11u64,
    });

    let mut infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(2000, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // one nft burnt, one minterd
    assert_eq!(infuse_res.len(), 2);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "mint");
    println!("{:#?}", infuse_res);

    bundle.nfts[0].addr = nft2.clone();

    // feesub nft1, single nft2 (both have same feesub denom, so check we are minting 2)
    infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(2100, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    println!("{:#?}", infuse_res);
    // one nft burnt, two minted
    assert_eq!(infuse_res.len(), 3);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "mint");
    assert_eq!(infuse_res[2], "mint");

    //  provide 2 counts of fee sub for nft2
    bundle = Bundle { nfts: vec![] };
    infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(2200, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // 0 nft burnt, two minted
    assert_eq!(infuse_res.len(), 2);
    assert_eq!(infuse_res[0], "mint");
    assert_eq!(infuse_res[1], "mint");

    //  provide 1.99 counts of feesub for nft2
    infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(2199, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // 0 nft burnt, two minted
    assert_eq!(infuse_res.len(), 1);
    assert_eq!(infuse_res[0], "mint");

    bundle.nfts.extend(vec![
        NFT {
            addr: nft1.clone(),
            token_id: 12u64,
        },
        NFT {
            addr: nft1.clone(),
            token_id: 13u64,
        },
    ]);
    //  provide 2 nft1, 1.99 counts of feesub for nft2, truth in feesub
    let infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[
                coin(2000, "uterp"),
                coin(
                    199,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
                ),
            ],
        )?
        .event_attr_values("wasm", "action");
    // 2 nft burnt, 3 minted
    println!("infuse_res: {:#?}", infuse_res);
    assert_eq!(infuse_res.len(), 5);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "burn");
    assert_eq!(infuse_res[2], "mint");
    assert_eq!(infuse_res[3], "mint");
    assert_eq!(infuse_res[4], "mint");
    Ok(())
}

#[test]
fn test_allof_payment_substitute() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;
    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();
    let nft3 = env.nfts[2].clone();

    env.chain.add_balance(
        &env.admin,
        vec![
            coin(2000, "ubtsg"),
            coin(
                200000,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;

    let spender = env.chain.addr_make_with_balance(
        "spain",
        vec![
            coin(1000u128, "uterp"),
            coin(
                20000u128,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;

    // update substitute payment for infusion being created to 200uterp
    env.infusion.collections[1].payment_substitute = Some(coin(
        200u128,
        "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
    ));

    // good infusion creation
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    let mut bundle = Bundle { nfts: vec![] };

    // 0 bundles
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[],
        )
        .unwrap_err()
        .to_string()
        .contains(&ContractError::EmptyBundle {}.to_string()),);

    // empty, single bundle with no fee payment, and no eligible collection
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: nft1.to_string(),
                bun_type: 1,
                wavs: false,
                min_req: 1,
            }
            .to_string()
        ),);

    //  bundle with correct # of non-subpayment col, incorrect fees for subpayment col
    bundle.nfts = vec![NFT {
        addr: nft1.clone(),
        token_id: 11,
    }];

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[
                coin(100, "uterp"),
                coin(
                    100,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
            ]
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft2.to_string(),
                have: coin(
                    100u128,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
                .into(),
                want: coin(
                    200u128,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
            }
            .to_string()
        ),);

    // ensure no static fee and no feesub errors
    let infuse = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp")],
        )
        .unwrap_err();

    assert!(infuse.to_string().contains(
        &ContractError::PaymentSubstituteNotProvided {
            col: nft2.to_string(),
            have: coin(
                0,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
            )
            .into(),
            want: coin(
                200u128,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
            )
        }
        .to_string()
    ),);

    // too many non paymentsub collection
    bundle.nfts.extend([NFT {
        addr: nft1.clone(),
        token_id: 12,
    }]);
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(200, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleNotAccepted {
                have: 2,
                want: 1,
                addr: nft1.to_string(),
            }
            .to_string()
        ),);

    // ensure correct payment substitute but incorrect bundle
    bundle = Bundle { nfts: vec![] };

    assert!(app
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(300, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: nft1.to_string(),
                bun_type: 1,
                wavs: false,
                min_req: 1,
            }
            .to_string()
        ));

    // ensure correct payment substitute but incorrect bundle
    bundle = Bundle {
        nfts: vec![NFT {
            addr: nft1.clone(),
            token_id: 11,
        }],
    };

    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp"), coin(200, "ubtsg")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft2.to_string(),
                have: coin(
                    0,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
                .into(),
                want: coin(
                    200u128,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
            }
            .to_string()
        ),);

    app.call_as(&env.admin).execute(
        &ExecuteMsg::Infuse {
            id: infusion_id,
            bundle: vec![bundle.clone()],
        },
        &[
            coin(100, "uterp"),
            coin(
                200u128,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;

    println!("{:#?}", app.infusion_by_id(infusion_id)?);

    // check with multiple collections
    env.infusion.infused_collection.addr = None;
    env.infusion.collections[0].payment_substitute = Some(coin(200u128, "uterp"));
    env.infusion.collections[0].max_req = Some(env.infusion.collections[0].min_req);

    env.infusion.collections.push(EligibleNFTCollection {
        addr: nft3.clone(),
        min_req: 1u64,
        max_req: Some(1u64),
        payment_substitute: Some(coin(200u128, "ubtsg")),
    });
    // env.infusion.collections[1].max_req = Some(env.infusion.collections[0].min_req);
    // env.infusion.collections[2].max_req = Some(env.infusion.collections[0].min_req);
    env.chain.wait_blocks(2)?;

    // good infusion creation
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    println!("{:#?}", app.infusion_by_id(infusion_id)?);

    // NO NFTS, NO FEESUB
    assert!(app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[]
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft1.to_string(),
                have: coin(0, "uterp").into(),
                want: coin(200u128, "uterp")
            }
            .to_string()
        ));
    // NO NFTS,0/3 FEESUB
    assert!(app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[coin(100, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft1.to_string(),
                have: coin(0, "uterp").into(),
                want: coin(200u128, "uterp")
            }
            .to_string()
        ));
    // NO NFTS,1/3 FEESUB
    assert!(app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[coin(300, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft2.to_string(),
                have: coin(
                    0,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
                .into(),
                want: coin(
                    200u128,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
            }
            .to_string()
        ),);

    // NO NFTS,2/3 FEESUB
    assert!(app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[
                coin(500, "uterp"),
                coin(
                    200u128,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591"
                )
            ],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::PaymentSubstituteNotProvided {
                col: nft3.to_string(),
                have: coin(0, "ubtsg").into(),
                want: coin(200u128, "ubtsg")
            }
            .to_string()
        ));

    Ok(())
}

#[test]
fn test_anyof_payment_substitute() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let nft2_feesub = coin(
        200u128,
        "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
    );

    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;

    let payment_recipient = env.chain.addr_make("payment_recipient");
    let spender = env.chain.addr_make_with_balance(
        "spain",
        vec![
            coin(1000u128, "uterp"),
            coin(
                1000u128,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;
    println!("payment_recipient: {:#?}", payment_recipient.to_string());
    println!("spender: {:#?}", spender.to_string());
    env.chain.add_balance(
        &env.admin,
        vec![
            coin(2000, "ubtsg"),
            coin(
                2000,
                "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
            ),
        ],
    )?;

    env.infusion.payment_recipient = Some(payment_recipient.clone());

    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();
    env.infusion.collections[1].payment_substitute = Some(nft2_feesub.clone());
    env.infusion.infusion_params.bundle_type = BundleType::AnyOf {
        addrs: vec![nft1.clone(), nft2.clone()],
    };

    // good infusion creation
    let mut infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    let mut bundle = Bundle { nfts: vec![] };

    // // 0 bundles
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[]
        )
        .unwrap_err()
        .to_string()
        .contains(&ContractError::EmptyBundle {}.to_string()),);

    // empty, single bundle with no fee payment, and no eligible collection
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[]
        )
        .unwrap_err()
        .to_string()
        .contains(&ContractError::InvalidAnyOfBundle {}.to_string()));

    //  bundle with correct # of non-feesub nfts, incorrect feesub payment
    bundle.nfts = vec![NFT {
        addr: nft1.clone(),
        token_id: 11,
    }];

    let mut infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(100, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // one nft burnt, one minted
    assert_eq!(infuse_res.len(), 2);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "mint");

    bundle.nfts[0].token_id += 1;

    // mint & correct non fee-sub, incorrect feesub payment
    infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // one nft burnt, one minterd
    assert_eq!(infuse_res.len(), 2);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "mint");

    // ensure correct payment substitute
    bundle.nfts[0].token_id += 1;
    infuse_res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![bundle.clone()],
            },
            &[
                coin(100, "uterp"),
                coin(
                    200,
                    "ibc/4A1C18CA7F50544760CF306189B810CE4C1CB156C7FC870143D401FE7280E591",
                ),
            ],
        )?
        .event_attr_values("wasm", "action");
    // one nft burnt, 2 minted
    assert_eq!(infuse_res.len(), 3);
    assert_eq!(infuse_res[0], "burn");
    assert_eq!(infuse_res[1], "mint");
    assert_eq!(infuse_res[1], "mint");

    // set owner
    // check with multiple collections
    env.infusion.infused_collection.addr = None;
    env.infusion.collections[0].payment_substitute = Some(coin(200u128, "uterp"));
    env.infusion.collections.push(EligibleNFTCollection {
        addr: env.nfts[2].clone(),
        min_req: 1u64,
        max_req: Some(1u64),
        payment_substitute: Some(coin(200u128, "ubtsg")),
    });
    env.chain.wait_blocks(2)?;

    // good infusion creation
    infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    // // NO NFTS, NO FEESUB
    assert!(app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[],
        )
        .unwrap_err()
        .to_string()
        .contains(&ContractError::InvalidAnyOfBundle {}.to_string()),);

    // NO NFTS,1/3 FEESUB
    infuse_res = app
        .call_as(&spender)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![Bundle { nfts: vec![] }],
            },
            &[coin(300, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    assert_eq!(infuse_res.len(), 1);
    assert_eq!(infuse_res[0], "mint");

    // assert pay sub goes to correct destination
    let inf = app.infusion_by_id(infusion_id)?;
    let cfg = app.config()?;
    println!("{:#?}", cfg);
    println!("{:#?}", inf);
    // println!("{:#?}", res);

    let bal_be4_owner = env.chain.query_balance(&cfg.contract_owner, "uterp")?;
    let bal_b4_pay_recipient = env.chain.query_balance(&payment_recipient, "uterp")?;

    // 270 should go to payment recipient, 30 should go to contract owner
    let res = app.call_as(&spender).execute(
        &ExecuteMsg::Infuse {
            id: infusion_id,
            bundle: vec![Bundle { nfts: vec![] }],
        },
        &[coin(300, "uterp")],
    )?;
    println!("{:#?}", res);
    env.chain.wait_blocks(2)?;

    let bal_aft_owner = env.chain.query_balance(&cfg.contract_owner, "uterp")?;
    let bal_aft_pay_recipient = env.chain.query_balance(&payment_recipient, "uterp")?;

    println!(
        "bal_be4_owner {:#?}: {:#?}",
        bal_be4_owner.to_string(),
        cfg.contract_owner.to_string()
    );
    println!(
        "bal_aft_owner {:#?}: {:#?}",
        bal_aft_owner.to_string(),
        cfg.contract_owner.to_string()
    );
    println!(
        "bal_b4_pay_recipient {:#?}: {:#?}",
        bal_b4_pay_recipient.to_string(),
        payment_recipient.to_string()
    );
    println!(
        "bal_aft_pay_recipient {:#?}: {:#?}",
        bal_aft_pay_recipient.to_string(),
        payment_recipient.to_string()
    );

    let pay = inf.collections[0]
        .payment_substitute
        .clone()
        .unwrap()
        .amount
        .checked_add(inf.infusion_params.mint_fee.unwrap().amount)?;
    let admin_cut = pay.multiply_ratio(cfg.owner_fee.numerator(), cfg.owner_fee.denominator());
    println!("pay: {:#?}", pay);
    println!("admin_cut: {:#?}", admin_cut);
    assert_eq!(bal_aft_owner, bal_be4_owner.checked_add(admin_cut)?);
    assert_eq!(
        bal_aft_pay_recipient,
        bal_b4_pay_recipient.checked_add(pay.checked_sub(admin_cut)?)?
    );
    // bal_b4_pay_recipient.checked_add();
    Ok(())
}

#[test]
fn test_anyofblend_payment_substitute() -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn test_updating_infusion_bundle_type() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let env = InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;

    // good infusion creation

    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    //  cannot update bundle type to anyOf with incorrect addr

    let mut bundle_type = BundleType::AnyOf {
        addrs: vec![env.nfts[2].clone(), env.nfts[1].clone()],
    };

    let err = app
        .update_infusions_eligible_collections_or_bundle_type(
            bundle_type,
            infusion_id,
            vec![],
            vec![],
        )
        .unwrap_err();

    assert!(err.to_string().contains(
        &ContractError::AnyOfConfigError {
            err: AnyOfErr::Uneligible,
        }
        .to_string()
    ),);

    bundle_type = BundleType::AnyOf {
        addrs: vec![env.nfts[1].clone()],
    };

    app.update_infusions_eligible_collections_or_bundle_type(
        bundle_type,
        infusion_id,
        vec![],
        vec![],
    )?;

    Ok(())
}

#[test]
fn test_updating_infusion_eligible_collections() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], false)?;
    let app = env.infuser;

    // update infusion collect params to undesired state
    env.infusion.collections[1].payment_substitute = Some(coin(1u128, "uterp"));

    // good infusion creation
    let infusion_id = app
        .execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?;
    let infusion_id = Uint128::from_str(&infusion_id)
        .map_err(|e| anyhow!(e.to_string()))?
        .u128() as u64;
    // update infusion accepted params
    env.infusion.collections[1].max_req = Some(3);
    env.infusion.collections[1].min_req = 2;
    env.infusion.collections[1].payment_substitute = Some(coin(100, "ubtsg"));
    app.update_infusions_eligible_collections_or_bundle_type(
        env.infusion.infusion_params.bundle_type,
        infusion_id,
        vec![env.infusion.collections[1].clone()],
        vec![env.infusion.collections[0].clone().addr],
    )?;

    let infusion = app.infusion_by_id(infusion_id)?;
    assert_eq!(infusion.collections.len(), 1);
    assert_eq!(infusion.collections[0].max_req, Some(3));
    assert_eq!(infusion.collections[0].min_req, 2);
    assert_eq!(
        infusion.collections[0].payment_substitute,
        Some(coin(100, "ubtsg"))
    );

    Ok(())
}

#[test]
fn test_wavs_record_anyof() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut wavs_bundle = vec![];
    // let mut bundles = vec![];
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], true)?;
    let app = env.infuser;
    let not_wavs = env.chain.addr_make("not-wavs");

    let nft1 = env.nfts[0].clone();
    let nft2 = env.nfts[1].clone();

    // with/without payment substitute
    // with 0 nfts sent in bundle

    println!(" update infusion collect params to undesired state");
    env.infusion.infusion_params.wavs_enabled = true;
    env.infusion.collections[0].min_req = 2;
    env.infusion.collections[1].min_req = 1;
    env.infusion.collections[1].payment_substitute = Some(coin(100u128, "uterp"));
    env.infusion.infusion_params.bundle_type = BundleType::AnyOf {
        addrs: vec![nft1.clone()],
    };

    println!("good infusion creation");
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    println!("{:#?}", app.infusion_by_id(infusion_id)?);

    println!("assert only wavs operator can update");
    assert!(app
        .call_as(&not_wavs)
        .wavs_entry_point(wavs_bundle)
        .unwrap_err()
        .to_string()
        .contains(&"Caller is not admin"));
    println!("assert nft is recorded to state");
    wavs_bundle = vec![WavsBundle {
        infuser: env.admin.to_string(),
        nft_addr: env.nfts[0].to_string(),
        infused_ids: vec!["11".to_string()],
    }];
    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");

    println!("{:#?}", res);
    assert_eq!(res.len(), 1);
    assert_eq!(res[0], "mint");

    // println!("assert the stored record from wavs has been updated and cannot be reconsumed");
    assert!(
        app.call_as(&env.admin)
            .execute(
                &ExecuteMsg::Infuse {
                    id: infusion_id,
                    bundle: vec![],
                },
                &[coin(100, "uterp")],
            )
            .unwrap_err()
            .to_string()
            .contains(&ContractError::InvalidAnyOfBundle {}.to_string()) // ContractError::WavsBundleNotAccepted {
                                                                         //     have: 0u64,
                                                                         //     need: 1u64,
                                                                         // }
    );

    // println!(
    //     "assert that leftovers can be combined with nft in bundle to satisfy min requirements"
    // );
    // wavs_bundle = vec![WavsBundle {
    //     infuser: env.admin.to_string(),
    //     nft_addr: env.nfts[1].to_string(),
    //     infused_ids: vec!["12".to_string()],
    // }];

    // bundles = vec![Bundle {
    //     nfts: vec![NFT {
    //         addr: Addr::unchecked(env.nfts[0].to_string()),
    //         token_id: 11,
    //     }],
    // }];

    // app.call_as(&env.wavs_service.clone())
    //     .wavs_entry_point(wavs_bundle.clone())?;

    // let res = app
    //     .call_as(&env.admin)
    //     .execute(
    //         &ExecuteMsg::Infuse {
    //             id: infusion_id,
    //             bundle: bundles.clone(),
    //         },
    //         Some(&[coin(200, "uterp")]),
    //     )?
    //     .event_attr_values("wasm", "action");

    // // println!("{:#?}", res);
    // assert_eq!(res.len(), 2);
    // assert_eq!(res[0], "burn");
    // assert_eq!(res[1], "mint");

    Ok(())
}

#[test]
fn test_wavs_record_anyof_blend() -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn test_wavs_record_allof() -> anyhow::Result<()> {
    // setup infuser with admin fees
    let mut wavs_bundle = vec![];
    let mut bundles = vec![];
    let mut env =
        InfuserTestSuite::<MockBech32>::setup_fee_suite(BundleType::AllOf {}, vec![], true)?;
    let app = env.infuser;
    let not_wavs = env.chain.addr_make("not-wavs");

    // with/without payment substitute
    // with 0 nfts sent in bundle

    println!(" update infusion collect params to undesired state");
    env.infusion.infusion_params.wavs_enabled = true;
    env.infusion.collections[0].min_req = 2;
    env.infusion.collections[1].payment_substitute = Some(coin(100u128, "uterp"));

    println!("good infusion creation");
    let infusion_id = Uint128::from_str(
        &app.execute(
            &ExecuteMsg::CreateInfusion {
                infusions: vec![env.infusion.clone()],
            },
            &[coin(500, "uterp")],
        )?
        .event_attr_value("wasm", "infusion-id")
        .map_err(|e| anyhow!(e.to_string()))?,
    )
    .map_err(|e| anyhow!(e.to_string()))?
    .u128() as u64;

    println!("{:#?}", app.infusion_by_id(infusion_id)?);

    println!("assert only wavs operator can update");
    assert!(app
        .call_as(&not_wavs)
        .wavs_entry_point(wavs_bundle)
        .unwrap_err()
        .to_string()
        .contains(&"Caller is not admin"));

    println!("assert nft is recorded to state");
    wavs_bundle = vec![WavsBundle {
        infuser: env.admin.to_string(),
        nft_addr: env.nfts[0].to_string(),
        infused_ids: vec!["11".to_string()],
    }];
    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    assert!(app
        .call_as(&env.admin)
        .infuse(vec![], infusion_id)
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::WavsBundleNotAccepted {
                have: 1u64,
                need: 2u64,
            }
            .to_string()
        ));

    println!("use record from WAVS expect to mint 1 NFT");
    wavs_bundle[0].infused_ids = vec![12.to_string()];
    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("{:#?}", res);
    assert_eq!(res.len(), 1);
    assert_eq!(res[0], "mint");

    println!("assert the stored record from wavs has been updated and cannot be reconsumed");
    assert!(app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[coin(200, "uterp")],
        )
        .unwrap_err()
        .to_string()
        .contains(
            &ContractError::BundleCollectionNotEligilbe {
                col: env.nfts[0].to_string(),
                bun_type: 1,
                wavs: true,
                min_req: 2,
            }
            .to_string()
        ));

    println!(
        "assert that leftovers can be combined with nft in bundle to satisfy min requirements"
    );
    wavs_bundle = vec![WavsBundle {
        infuser: env.admin.to_string(),
        nft_addr: env.nfts[0].to_string(),
        infused_ids: vec!["15".to_string()],
    }];
    bundles = vec![Bundle {
        nfts: vec![NFT {
            addr: Addr::unchecked(env.nfts[0].to_string()),
            token_id: 11,
        }],
    }];

    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: bundles.clone(),
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");

    // println!("{:#?}", res);
    assert_eq!(res.len(), 2);
    assert_eq!(res[0], "burn");
    assert_eq!(res[1], "mint");

    println!(
        "ensure we accurately retain any excess mint count if min burnt is recorded in store
     (1st tx mints one), add one more"
    );
    wavs_bundle[0].infused_ids.push("16".into());
    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    bundles[0].nfts[0].token_id = 12u64;

    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: bundles.clone(),
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    // println!("{:#?}", res);
    assert_eq!(res.len(), 2);
    assert_eq!(res[0], "burn");
    assert_eq!(res[1], "mint");

    wavs_bundle[0].infused_ids = vec![16u64.to_string()];
    app.call_as(&env.wavs_service.clone())
        .wavs_entry_point(wavs_bundle.clone())?;

    let res = app.wavs_record(vec![env.nfts[0].to_string()], Some(env.admin.clone()))?;
    println!("{:#?}", res);
    assert_eq!(res[0].count, Some(2u64));

    println!("ensure we can now mint with the remaining balance in the wavs record store");
    let res = app
        .call_as(&env.admin)
        .execute(
            &ExecuteMsg::Infuse {
                id: infusion_id,
                bundle: vec![],
            },
            &[coin(200, "uterp")],
        )?
        .event_attr_values("wasm", "action");
    assert_eq!(res.len(), 1);
    assert_eq!(res[0], "mint");

    let res = app.wavs_record(vec![env.nfts[0].to_string()], Some(env.admin.clone()))?;
    println!("{:#?}", res);
    assert_eq!(res[0].count, Some(0));
    Ok(())
}

#[test]
fn test_mintout_collection() -> anyhow::Result<()> {
    Ok(())
}

#[test]
fn test_anyof_feesub_payment_destination() -> anyhow::Result<()> {
    // 2 collections + static fee. both feesub, one different denom than static fee

    Ok(())
}

#[test]
fn test_allof_feesub_payment_destination() -> anyhow::Result<()> {
    Ok(())
}

// #[test]
// fn test_migration_v041() -> anyhow::Result<()> {
//     let inf1_token_id = vec!["187", "332", "477", "594", "88"];
//     let inf2_token_id = vec!["487"];

//     // setup nft & infusion simulating current infusions
//     let env = InfuserTestSuite::<MockBech32>::setup_fee_suite()?;

//     let v020_infusion = v020_infusion();
//     let v020_infusion_code_id = env
//         .chain
//         .upload_custom("v020infusion", v020_infusion)?
//         .uploaded_code_id()?;

//     // instantiate 2 infusion contracts from same code-id
//     let mut infuse_addrs = vec![];
//     for _ in 0..2 {
//         let ia = env
//             .chain
//             .instantiate(
//                 v020_infusion_code_id,
//                 &v020infuse::msg::InstantiateMsg {
//                     contract_owner: Some(env.admin.to_string()),
//                     owner_fee: 10,
//                     min_creation_fee: Some(coin(1, "uterp")),
//                     min_infusion_fee: Some(coin(2, "uterp")),
//                     min_per_bundle: None,
//                     max_per_bundle: None,
//                     max_bundles: None,
//                     max_infusions: None,
//                     cw721_code_id: 2u64,
//                 },
//                 Some("infusion being migrated"),
//                 Some(&env.admin),
//                 &[],
//             )?
//             .instantiated_contract_address()?;

//         infuse_addrs.push(ia);
//     }

//     // configure cw721

//     let good_nfts = env.default_good_nfts(&vec![env.nfts[0].clone(), env.nfts[1].clone()])?;
//     let infused_col = InfuserTestSuite::<MockBech32>::default_infused_collection()?;
//     // create infusion for both
//     for ia in 0..infuse_addrs.len() {
//         let mut total_supply = 666u64;
//         if ia == 2 {
//             total_supply = 100;
//         }
//         env.chain.execute(
//             &v020infuse::msg::ExecuteMsg::CreateInfusion {
//                 infusions: vec![v020infuse::state::Infusion {
//                     collections: good_nfts
//                         .clone()
//                         .into_iter()
//                         .map(|n| v020infuse::state::EligibleNFTCollection {
//                             addr: n.addr,
//                             min_req: n.min_req,
//                             max_req: n.max_req,
//                             payment_substitute: n.payment_substitute,
//                         })
//                         .collect(),
//                     infused_collection: v020infuse::state::InfusedCollection {
//                         sg: infused_col.sg,
//                         addr: None,
//                         admin: infused_col.admin.clone(),
//                         name: infused_col.name.clone(),
//                         symbol: infused_col.symbol.clone(),
//                         base_uri: infused_col.base_uri.clone(),
//                         num_tokens: infused_col.num_tokens.clone(),
//                         royalty_info: infused_col.royalty_info.clone(),
//                         description: infused_col.description.clone(),
//                         image: infused_col.image.clone(),
//                         start_trading_time: infused_col.start_trading_time.clone(),
//                         explicit_content: infused_col.explicit_content.clone(),
//                         external_link: infused_col.external_link.clone(),
//                     },
//                     infusion_params: v020infuse::state::InfusionParams {
//                         mint_fee: Some(coin(100, "uterp")),
//                         params: None,
//                     },
//                     payment_recipient: env.infusion.payment_recipient.clone(),
//                     description: env.infusion.description.clone(),
//                     owner: env.infusion.owner.clone(),
//                 }],
//             },
//             &[coin(1, "uterp".to_string())],
//             &infuse_addrs[ia],
//         )?;

//         InfuserTestSuite::<MockBech32>::default_nft_approvals(
//             env.chain.clone(),
//             env.nfts.clone(),
//             infuse_addrs[ia ].clone(),
//             env.admin.clone(),
//         )?;
//         // infuse once for both
//         let mut bundle = v020infuse::state::Bundle {
//             nfts: vec![
//                 v020infuse::state::NFT {
//                     addr: env.nfts[0].clone(),
//                     token_id: 8,
//                 },
//                 v020infuse::state::NFT {
//                     addr: env.nfts[1].clone(),
//                     token_id: 9,
//                 },
//             ],
//         };

//         //mint 5 tokens for infusion 1 and 1 token for infusion 2
//         for i in 0..5 {
//             env.chain.execute(
//                 &v020infuse::msg::ExecuteMsg::Infuse {
//                     infusion_id: 1,
//                     bundle: vec![bundle.clone()],
//                 },
//                 &[coin(100u128, "uterp")],
//                 &infuse_addrs[0],
//             )?;
//         }
//         for i in 0..2 {}
//     }
//     Ok(())
// }

// #[test]
// fn test_migration_v030() -> anyhow::Result<()> {
//     // setup infuser with admin fees
//     let env = InfuserTestSuite::<MockBech32>::setup_fee_suite()?;

//     // store cw721
//     let v020_infusion = v020_infusion();
//     let v020_infusion_code_id = env
//         .chain
//         .upload_custom("v020infusion", v020_infusion)?
//         .uploaded_code_id()?;

//     // instantiate 2 infusion contracts from same code-id
//     let mut infuse_addrs = vec![];
//     for _ in 0..2 {
//         let ia = env
//             .chain
//             .instantiate(
//                 v020_infusion_code_id,
//                 &v020infuse::msg::InstantiateMsg {
//                     contract_owner: Some(env.admin.to_string()),
//                     owner_fee: 10,
//                     min_creation_fee: Some(coin(1, "uterp")),
//                     min_infusion_fee: Some(coin(2, "uterp")),
//                     min_per_bundle: None,
//                     max_per_bundle: None,
//                     max_bundles: None,
//                     max_infusions: None,
//                     cw721_code_id: 2u64,
//                 },
//                 Some("infusion being migrated"),
//                 Some(&env.admin),
//                 &[],
//             )?
//             .instantiated_contract_address()?;

//         infuse_addrs.push(ia);
//     }

//     // configure cw721

//     let good_nfts = env.default_good_nfts(&vec![env.nfts[0].clone(), env.nfts[1].clone()])?;

//     let infused_col = InfuserTestSuite::<MockBech32>::default_infused_collection()?;
//     // create infusion for both
//     for ia in &infuse_addrs {
//         env.chain.execute(
//             &v020infuse::msg::ExecuteMsg::CreateInfusion {
//                 infusions: vec![v020infuse::state::Infusion {
//                     collections: good_nfts
//                         .clone()
//                         .into_iter()
//                         .map(|n| v020infuse::state::EligibleNFTCollection {
//                             addr: n.addr,
//                             min_req: n.min_req,
//                             max_req: n.max_req,
//                             payment_substitute: n.payment_substitute,
//                         })
//                         .collect(),
//                     infused_collection: v020infuse::state::InfusedCollection {
//                         sg: infused_col.sg,
//                         addr: None,
//                         admin: infused_col.admin.clone(),
//                         name: infused_col.name.clone(),
//                         symbol: infused_col.symbol.clone(),
//                         base_uri: infused_col.base_uri.clone(),
//                         num_tokens: infused_col.num_tokens.clone(),
//                         royalty_info: infused_col.royalty_info.clone(),
//                         description: infused_col.description.clone(),
//                         image: infused_col.image.clone(),
//                         start_trading_time: infused_col.start_trading_time.clone(),
//                         explicit_content: infused_col.explicit_content.clone(),
//                         external_link: infused_col.external_link.clone(),
//                     },
//                     infusion_params: v020infuse::state::InfusionParams {
//                         mint_fee: Some(coin(100, "uterp")),
//                         params: None,
//                     },
//                     payment_recipient: env.infusion.payment_recipient.clone(),
//                     description: env.infusion.description.clone(),
//                     owner: env.infusion.owner.clone(),
//                 }],
//             },
//             &[coin(1, "uterp".to_string())],
//             &ia,
//         )?;

//         InfuserTestSuite::<MockBech32>::default_nft_approvals(
//             env.chain.clone(),
//             env.nfts.clone(),
//             ia.clone(),
//             env.admin.clone(),
//         )?;
//     }
//     // infuse once for both
//     let mut bundle = v020infuse::state::Bundle {
//         nfts: vec![
//             v020infuse::state::NFT {
//                 addr: env.nfts[0].clone(),
//                 token_id: 8,
//             },
//             v020infuse::state::NFT {
//                 addr: env.nfts[1].clone(),
//                 token_id: 9,
//             },
//         ],
//     };

//     env.chain.execute(
//         &v020infuse::msg::ExecuteMsg::Infuse {
//             infusion_id: 1,
//             bundle: vec![bundle.clone()],
//         },
//         &[coin(100, "uterp")],
//         &infuse_addrs[0].clone(),
//     )?;
//     bundle.nfts[0].token_id = 7;
//     bundle.nfts[1].token_id = 6;

//     env.chain.wait_blocks(1)?;
//     env.chain.execute(
//         &v020infuse::msg::ExecuteMsg::Infuse {
//             infusion_id: 1,
//             bundle: vec![bundle.clone()],
//         },
//         &[coin(100, "uterp")],
//         &infuse_addrs[0].clone(),
//     )?;

//     env.chain.call_as(&env.admin).migrate(
//         &cw_infuser::msg::MigrateMsg {},
//         env.infuser.code_id()?,
//         &infuse_addrs[0],
//     )?;
//     env.chain.call_as(&env.admin).migrate(
//         &cw_infuser::msg::MigrateMsg {},
//         env.infuser.code_id()?,
//         &infuse_addrs[1],
//     )?;

//     // migrate 1st contract, expect new v030 global config to be saved
//     let cfg1: Config = env.chain.query(&QueryMsg::Config {}, &infuse_addrs[0])?;
//     assert_eq!(cfg1.owner_fee, Decimal::percent(10));
//     // confirm first migration worked
//     let inf: InfusionState = env
//         .chain
//         .query(&QueryMsg::InfusionById { id: 1u64 }, &infuse_addrs[0])?;

//     assert_eq!(inf.infusion_params.bundle_type, BundleType::AllOf {});

//     // migrate 2nd contract,
//     // confirm second migration worked
//     let cfg1: Config = env.chain.query(&QueryMsg::Config {}, &infuse_addrs[1])?;
//     assert_eq!(cfg1.owner_fee, Decimal::percent(10));
//     // confirm first migration worked
//     let inf: InfusionState = env
//         .chain
//         .query(&QueryMsg::InfusionById { id: 1u64 }, &infuse_addrs[1])?;

//     assert_eq!(inf.infusion_params.bundle_type, BundleType::AllOf {});

//     // check can infuse
//     env.chain.wait_blocks(1)?;
//     bundle.nfts[0].token_id = 5;
//     bundle.nfts[1].token_id = 4;

//     env.chain.execute(
//         &v020infuse::msg::ExecuteMsg::Infuse {
//             infusion_id: 1,
//             bundle: vec![bundle.clone()],
//         },
//         &[coin(100, "uterp")],
//         &infuse_addrs[0].clone(),
//     )?;

//     Ok(())
// }
