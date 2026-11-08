// Shared cw-multi-test fixture: minimal osmosis tokenfactory Stargate handler.
// Included via include!() from each integration test file (works both as a
// standalone test target and as a submodule of tests/mod.rs).
// Handles MsgCreateDenom (returns {"new_token_denom"} as response data, which the
// headstash reply handler reads from msg_responses[0]), MsgMint (bank sudo mint)
// and MsgBurn (bank burn from sender). Values are protobuf (as wasmd requires); this
// fixture decodes them with its own independent parser, so a JSON regression fails here.
#[allow(dead_code)]
mod tf_stargate {
    use cosmwasm_std::{
        Addr, Api, BankMsg, Binary, BlockInfo, Coin, CustomMsg, CustomQuery,
        StdError, StdResult, Storage, Uint128,
    };
    use cw_multi_test::{
        AppBuilder, AppResponse, BankSudo, CosmosRouter, Stargate, SudoMsg,
    };
    use serde::de::DeserializeOwned;

    pub struct TokenFactoryStargate;

    // Independent proto3 reader: length-delimited fields only (all these messages are
    // strings / nested messages). Anything else, including JSON, is an error.
    pub fn proto_fields(b: &[u8]) -> StdResult<Vec<(u64, Vec<u8>)>> {
        fn varint(b: &[u8], i: &mut usize) -> StdResult<u64> {
            let mut v = 0u64;
            let mut sh = 0;
            loop {
                let x = *b.get(*i).ok_or_else(|| StdError::msg("tf mock: truncated"))?;
                *i += 1;
                v |= ((x & 0x7f) as u64) << sh;
                if x & 0x80 == 0 { return Ok(v); }
                sh += 7;
                if sh >= 64 { return Err(StdError::msg("tf mock: varint")); }
            }
        }
        let (mut i, mut out) = (0usize, vec![]);
        while i < b.len() {
            let k = varint(b, &mut i)?;
            if k & 7 != 2 { return Err(StdError::msg(format!("tf mock: not proto (key {k:#x})"))); }
            let n = varint(b, &mut i)? as usize;
            let v = b.get(i..i + n).ok_or_else(|| StdError::msg("tf mock: truncated field"))?;
            out.push((k >> 3, v.to_vec()));
            i += n;
        }
        Ok(out)
    }
    fn fstr(b: &[u8], f: u64) -> StdResult<String> {
        let v = proto_fields(b)?.into_iter().filter(|(n, _)| *n == f).last().map(|x| x.1).unwrap_or_default();
        String::from_utf8(v).map_err(|_| StdError::msg("tf mock: utf8"))
    }
    fn fmsg(b: &[u8], f: u64) -> StdResult<Vec<u8>> {
        Ok(proto_fields(b)?.into_iter().filter(|(n, _)| *n == f).last().map(|x| x.1).unwrap_or_default())
    }
    fn coin(b: &[u8]) -> StdResult<Coin> {
        let amt: u128 = fstr(b, 2)?.parse().map_err(|_| StdError::msg("bad amount"))?;
        Ok(Coin::new(Uint128::new(amt), fstr(b, 1)?))
    }
    fn create_denom_response(denom: &str) -> Binary {
        let mut o = vec![0x0a];
        let mut n = denom.len();
        while n >= 0x80 { o.push((n as u8) | 0x80); n >>= 7; }
        o.push(n as u8);
        o.extend_from_slice(denom.as_bytes());
        Binary::from(o)
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
                    let denom = format!("factory/{}/{}", fstr(&value, 1)?, fstr(&value, 2)?);
                    Ok(AppResponse {
                        events: vec![],
                        data: Some(create_denom_response(&denom)),
                    })
                }
                "/osmosis.tokenfactory.v1beta1.MsgMint" => {
                    let (amount, to) = (coin(&fmsg(&value, 2)?)?, fstr(&value, 3)?);
                    router
                        .sudo(
                            api,
                            storage,
                            block,
                            SudoMsg::Bank(BankSudo::Mint {
                                to_address: to,
                                amount: vec![amount],
                            }),
                        )
                        .map_err(|e| StdError::msg(e.to_string()))
                }
                "/osmosis.tokenfactory.v1beta1.MsgBurn" => {
                    let amount = coin(&fmsg(&value, 2)?)?;
                    router
                        .execute(
                            api,
                            storage,
                            block,
                            sender,
                            BankMsg::Burn {
                                amount: vec![amount],
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
