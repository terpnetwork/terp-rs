// Shared cw-multi-test fixture: minimal osmosis tokenfactory Stargate handler.
// Included via include!() from each integration test file (works both as a
// standalone test target and as a submodule of tests/mod.rs).
// Handles MsgCreateDenom (returns {"new_token_denom"} as response data, which the
// headstash reply handler reads from msg_responses[0]), MsgMint (bank sudo mint)
// and MsgBurn (bank burn from sender). Values are JSON, as encoded by cw-headstash.
#[allow(dead_code)]
mod tf_stargate {
    use cosmwasm_std::{
        to_json_binary, Addr, Api, BankMsg, Binary, BlockInfo, Coin, CustomMsg, CustomQuery,
        StdError, StdResult, Storage, Uint128,
    };
    use cw_multi_test::{
        AppBuilder, AppResponse, BankSudo, CosmosRouter, Stargate, SudoMsg,
    };
    use serde::de::DeserializeOwned;

    pub struct TokenFactoryStargate;

    #[derive(serde::Deserialize)]
    struct JCoin {
        denom: String,
        amount: String,
    }
    #[derive(serde::Deserialize)]
    struct CreateDenom {
        sender: String,
        subdenom: String,
    }
    #[derive(serde::Deserialize)]
    struct Mint {
        amount: JCoin,
        mint_to_address: String,
    }
    #[derive(serde::Deserialize)]
    struct Burn {
        amount: JCoin,
    }
    #[derive(serde::Serialize)]
    struct CreateDenomResponse {
        new_token_denom: String,
    }

    fn coin(c: JCoin) -> StdResult<Coin> {
        let amt: u128 = c.amount.parse().map_err(|_| StdError::msg("bad amount"))?;
        Ok(Coin::new(Uint128::new(amt), c.denom))
    }

    impl Stargate for TokenFactoryStargate {
        fn execute_stargate<ExecC, QueryC>(
            &self,
            api: &dyn Api,
            storage: &mut dyn Storage,
            router: &dyn CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
            block: &BlockInfo,
            sender: Addr,
            type_url: String,
            value: Binary,
        ) -> StdResult<AppResponse>
        where
            ExecC: CustomMsg + DeserializeOwned + 'static,
            QueryC: CustomQuery + DeserializeOwned + 'static,
        {
            match type_url.as_str() {
                "/osmosis.tokenfactory.v1beta1.MsgCreateDenom" => {
                    let m: CreateDenom = cosmwasm_std::from_json(&value)?;
                    let denom = format!("factory/{}/{}", m.sender, m.subdenom);
                    Ok(AppResponse {
                        events: vec![],
                        data: Some(to_json_binary(&CreateDenomResponse {
                            new_token_denom: denom,
                        })?),
                    })
                }
                "/osmosis.tokenfactory.v1beta1.MsgMint" => {
                    let m: Mint = cosmwasm_std::from_json(&value)?;
                    router
                        .sudo(
                            api,
                            storage,
                            block,
                            SudoMsg::Bank(BankSudo::Mint {
                                to_address: m.mint_to_address,
                                amount: vec![coin(m.amount)?],
                            }),
                        )
                        .map_err(|e| StdError::msg(e.to_string()))
                }
                "/osmosis.tokenfactory.v1beta1.MsgBurn" => {
                    let m: Burn = cosmwasm_std::from_json(&value)?;
                    router
                        .execute(
                            api,
                            storage,
                            block,
                            sender,
                            BankMsg::Burn {
                                amount: vec![coin(m.amount)?],
                            }
                            .into(),
                        )
                        .map_err(|e| StdError::msg(e.to_string()))
                }
                other => Err(StdError::msg(format!("Unexpected stargate execute: {other}"))),
            }
        }
    }

    pub type TfApp = cw_multi_test::App<
        cw_multi_test::BankKeeper,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockStorage,
        cw_multi_test::FailingModule<cosmwasm_std::Empty, cosmwasm_std::Empty, cosmwasm_std::Empty>,
        cw_multi_test::WasmKeeper<cosmwasm_std::Empty, cosmwasm_std::Empty>,
        cw_multi_test::StakeKeeper,
        cw_multi_test::DistributionKeeper,
        cw_multi_test::IbcSimpleModule,
        cw_multi_test::GovFailingModule,
        TokenFactoryStargate,
    >;

    pub fn new_app() -> TfApp {
        AppBuilder::default()
            .with_stargate(TokenFactoryStargate)
            .build(|_, _, _| {})
    }
}
