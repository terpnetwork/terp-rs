//! Test-only tokenfactory handler for `CosmosMsg::Any`.
//!
//! cw-headstash emits `/osmosis.tokenfactory.v1beta1.Msg{CreateDenom,Mint,Burn}`
//! as `AnyMsg`s and reads `MsgCreateDenomResponse` back from the reply. The
//! default multi-test `App` rejects every `Any`, so this module models the
//! tokenfactory semantics the contract relies on: denom creation
//! (`factory/{sender}/{subdenom}`, admin = creator, no duplicates), admin-only
//! mint into the bank module, and self-burn.

use cosmwasm_std::testing::{MockApi, MockStorage};
use cosmwasm_std::{
    Addr, AnyMsg, Api, BankMsg, BlockInfo, Coin, CustomMsg, CustomQuery, Empty, StdError,
    StdResult, Storage, Uint128, from_json, to_json_binary,
};
use cw_multi_test::{
    App, AppBuilder, AppResponse, BankKeeper, BankSudo, CosmosRouter, DistributionKeeper,
    FailingModule, GovFailingModule, IbcSimpleModule, StakeKeeper, Stargate, SudoMsg, WasmKeeper,
};
use serde::de::DeserializeOwned;

const CREATE_DENOM: &str = "/osmosis.tokenfactory.v1beta1.MsgCreateDenom";
const MINT: &str = "/osmosis.tokenfactory.v1beta1.MsgMint";
const BURN: &str = "/osmosis.tokenfactory.v1beta1.MsgBurn";
const ADMIN_PREFIX: &[u8] = b"tokenfactory/admin/";

pub type TfApp = App<
    BankKeeper,
    MockApi,
    MockStorage,
    FailingModule<Empty, Empty, Empty>,
    WasmKeeper<Empty, Empty>,
    StakeKeeper,
    DistributionKeeper,
    IbcSimpleModule,
    GovFailingModule,
    TokenFactoryStargate,
>;

/// Multi-test `App` with the tokenfactory `Any` handler installed.
pub fn tf_app() -> TfApp {
    AppBuilder::new()
        .with_stargate(TokenFactoryStargate)
        .build(|_, _, _| {})
}

#[derive(Default)]
pub struct TokenFactoryStargate;

#[derive(serde::Deserialize)]
struct MsgCreateDenom {
    sender: String,
    subdenom: String,
}

#[derive(serde::Deserialize)]
struct WireCoin {
    denom: String,
    amount: String,
}

#[derive(serde::Deserialize)]
struct MsgMint {
    sender: String,
    amount: WireCoin,
    mint_to_address: String,
}

#[derive(serde::Deserialize)]
struct MsgBurn {
    sender: String,
    amount: WireCoin,
    burn_from_address: String,
}

#[derive(serde::Serialize)]
struct MsgCreateDenomResponse {
    new_token_denom: String,
}

fn admin_key(denom: &str) -> Vec<u8> {
    [ADMIN_PREFIX, denom.as_bytes()].concat()
}

fn coin(c: WireCoin) -> StdResult<Coin> {
    let amount: Uint128 = c
        .amount
        .parse::<u128>()
        .map_err(|e| StdError::msg(format!("tokenfactory: bad amount: {e}")))?
        .into();
    Ok(Coin::new(amount, c.denom))
}

fn require_signer(sender: &Addr, claimed: &str) -> StdResult<()> {
    if sender.as_str() != claimed {
        return Err(StdError::msg(format!(
            "tokenfactory: signer {sender} != msg.sender {claimed}"
        )));
    }
    Ok(())
}

fn require_admin(storage: &dyn Storage, denom: &str, sender: &Addr) -> StdResult<()> {
    match storage.get(&admin_key(denom)) {
        Some(admin) if admin == sender.as_bytes() => Ok(()),
        Some(_) => Err(StdError::msg(format!(
            "tokenfactory: {sender} is not admin of {denom}"
        ))),
        None => Err(StdError::msg(format!(
            "tokenfactory: denom {denom} does not exist"
        ))),
    }
}

impl Stargate for TokenFactoryStargate {
    fn execute_any<ExecC, QueryC>(
        &self,
        api: &dyn Api,
        storage: &mut dyn Storage,
        router: &dyn CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
        block: &BlockInfo,
        sender: Addr,
        msg: AnyMsg,
    ) -> StdResult<AppResponse>
    where
        ExecC: CustomMsg + DeserializeOwned + 'static,
        QueryC: CustomQuery + DeserializeOwned + 'static,
    {
        match msg.type_url.as_str() {
            CREATE_DENOM => {
                let m: MsgCreateDenom = from_json(&msg.value)?;
                require_signer(&sender, &m.sender)?;
                let denom = format!("factory/{}/{}", m.sender, m.subdenom);
                let key = admin_key(&denom);
                if storage.get(&key).is_some() {
                    return Err(StdError::msg(format!(
                        "tokenfactory: denom {denom} already exists"
                    )));
                }
                storage.set(&key, sender.as_bytes());
                Ok(AppResponse {
                    events: vec![],
                    data: Some(to_json_binary(&MsgCreateDenomResponse {
                        new_token_denom: denom,
                    })?),
                })
            }
            MINT => {
                let m: MsgMint = from_json(&msg.value)?;
                require_signer(&sender, &m.sender)?;
                let amount = coin(m.amount)?;
                require_admin(storage, &amount.denom, &sender)?;
                router.sudo(
                    api,
                    storage,
                    block,
                    SudoMsg::Bank(BankSudo::Mint {
                        to_address: m.mint_to_address,
                        amount: vec![amount],
                    }),
                )
            }
            BURN => {
                let m: MsgBurn = from_json(&msg.value)?;
                require_signer(&sender, &m.sender)?;
                require_signer(&sender, &m.burn_from_address)?;
                let amount = coin(m.amount)?;
                require_admin(storage, &amount.denom, &sender)?;
                router.execute(
                    api,
                    storage,
                    block,
                    sender,
                    BankMsg::Burn {
                        amount: vec![amount],
                    }
                    .into(),
                )
            }
            other => Err(StdError::msg(format!("Unexpected any execute: {other}"))),
        }
    }
}
