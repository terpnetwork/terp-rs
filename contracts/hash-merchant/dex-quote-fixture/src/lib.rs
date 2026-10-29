//! Fixed posted-liquidity answer for port `dex-quote`.
//!
//! The registry sends `DexQuoteRequest`. This contract writes the stored
//! `DexQuoteAck` as raw JSON. A missing position or a different pair is an
//! ICS-20 JSON error ack, which the registry does not file as a price.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Binary, Deps, DepsMut, Env, Ibc2PacketAckMsg,
    Ibc2PacketReceiveMsg, Ibc2PacketSendMsg, Ibc2PacketTimeoutMsg, IbcBasicResponse,
    IbcReceiveResponse, MessageInfo, Response, StdAck, StdError, StdResult,
};
use cw_oracle_registry::msg::{
    DexQuoteAck, DexQuoteBatch, DexQuoteBatchAck, DexQuoteBatchItem, DexQuoteRequest,
    DEX_QUOTE_ENCODING, DEX_QUOTE_PORT, DEX_QUOTE_VERSION,
};
use cw_storage_plus::Item;

pub const ACK: Item<DexQuoteAck> = Item::new("ack");

#[cw_serde]
pub struct InstantiateMsg {
    pub ack: DexQuoteAck,
}

#[cw_serde]
pub enum ExecuteMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(DexQuoteAck)]
    Ack {},
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    if msg.ack.position_id.is_empty() || msg.ack.p.is_empty() || msg.ack.q.is_empty() {
        return Err(StdError::msg("ack missing position or price"));
    }
    ACK.save(deps.storage, &msg.ack)?;
    Ok(Response::new().add_attribute("action", "instantiate_dex_quote_fixture"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: ExecuteMsg,
) -> StdResult<Response> {
    Err(StdError::msg("no execute"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ack {} => to_json_binary(&ACK.load(deps.storage)?),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: Ibc2PacketReceiveMsg,
) -> StdResult<IbcReceiveResponse> {
    if msg.payload.destination_port != DEX_QUOTE_PORT
        || msg.payload.version != DEX_QUOTE_VERSION
        || msg.payload.encoding != DEX_QUOTE_ENCODING
    {
        return Ok(IbcReceiveResponse::new(
            StdAck::error("unexpected dex-quote payload").to_binary(),
        ));
    }
    let ack = ACK.load(deps.storage)?;
    if let Ok(batch) = from_json::<DexQuoteBatch>(&msg.payload.value) {
        let quotes = batch
            .quotes
            .into_iter()
            .map(|req| batch_item(&ack, req))
            .collect();
        return Ok(IbcReceiveResponse::new(to_json_binary(
            &DexQuoteBatchAck { quotes },
        )?));
    }
    let req: DexQuoteRequest = from_json(&msg.payload.value)?;
    if req.pair != ack.pair || (!req.position_id.is_empty() && req.position_id != ack.position_id) {
        return Ok(IbcReceiveResponse::new(
            StdAck::error("position").to_binary(),
        ));
    }
    Ok(IbcReceiveResponse::new(to_json_binary(&ack)?)
        .add_attribute("action", "dex_quote")
        .add_attribute("position_id", &ack.position_id))
}

fn batch_item(ack: &DexQuoteAck, req: DexQuoteRequest) -> DexQuoteBatchItem {
    if req.pair != ack.pair || (!req.position_id.is_empty() && req.position_id != ack.position_id) {
        return DexQuoteBatchItem {
            market_id: req.market_id,
            height: req.height,
            error: "position".into(),
            position_id: String::new(),
            pair: None,
            reserves: None,
            p: String::new(),
            q: String::new(),
        };
    }
    DexQuoteBatchItem {
        market_id: req.market_id,
        height: req.height,
        error: String::new(),
        position_id: ack.position_id.clone(),
        pair: Some(ack.pair.clone()),
        reserves: Some(ack.reserves.clone()),
        p: ack.p.clone(),
        q: ack.q.clone(),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: Ibc2PacketAckMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: Ibc2PacketTimeoutMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_send(
    _deps: DepsMut,
    _env: Env,
    _msg: Ibc2PacketSendMsg,
) -> StdResult<IbcBasicResponse> {
    Err(StdError::msg("fixture does not send"))
}

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub mod interface {
    use cosmwasm_std::Empty;
    use cw_orch::{interface, prelude::*};

    use super::{ExecuteMsg, InstantiateMsg, QueryMsg};

    #[interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty)]
    pub struct DexQuoteFixture;

    impl<Chain> Uploadable for DexQuoteFixture<Chain> {
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path("cw_dex_quote_fixture")
                .unwrap()
        }
        fn wrapper() -> Box<dyn MockContract<Empty>> {
            Box::new(ContractWrapper::new_with_empty(
                super::execute,
                super::instantiate,
                super::query,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{Addr, CosmosMsg, Ibc2Msg, Ibc2PacketAckMsg, Ibc2Payload};
    use cw_oracle_registry::contract::{
        execute as reg_execute, ibc2_packet_ack, ibc2_source_port, instantiate as reg_instantiate,
        query as reg_query,
    };
    use cw_oracle_registry::msg::{
        DexPair, DexQuoteRequest, DexReserves, ExecuteMsg as RegExec, InstantiateMsg as RegInit,
        ObservationResponse, QueryMsg as RegQuery,
    };
    use cw_oracle_registry::state::SourceKind;

    fn posted() -> DexQuoteAck {
        DexQuoteAck {
            position_id: "pos-9".into(),
            pair: DexPair {
                asset_1: "penumbra".into(),
                asset_2: "usdc".into(),
            },
            reserves: DexReserves {
                r1: "1000".into(),
                r2: "4000".into(),
            },
            p: "1".into(),
            q: "4".into(),
        }
    }

    #[test]
    fn round_trip_files_posted_mid() {
        let mut registry = mock_dependencies();
        let admin = registry.api.addr_make("admin");
        reg_instantiate(
            registry.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            RegInit {
                admin: admin.to_string(),
                open_submit: false,
                bounty: None,
                note: None,
                ibc2_source_client: None,
            },
        )
        .unwrap();
        reg_execute(
            registry.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            RegExec::RegisterIbcV2 {
                source_client: "07-tendermint-1".into(),
            },
        )
        .unwrap();
        let sent = reg_execute(
            registry.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            RegExec::RequestQuote {
                market_id: "PEN/USD".into(),
                pair: posted().pair.clone(),
                position_id: String::new(),
            },
        )
        .unwrap();
        let CosmosMsg::Ibc2(Ibc2Msg::SendPacket { payloads, .. }) = &sent.messages[0].msg else {
            panic!("send");
        };
        let req: DexQuoteRequest = from_json(&payloads[0].value).unwrap();

        let mut fixture = mock_dependencies();
        instantiate(
            fixture.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            InstantiateMsg { ack: posted() },
        )
        .unwrap();
        let recv = Ibc2PacketReceiveMsg::new(
            payloads[0].clone(),
            Addr::unchecked("relayer"),
            "07-tendermint-1".into(),
            "dest".into(),
            1,
        );
        let answered = ibc2_packet_receive(fixture.as_mut(), mock_env(), recv).unwrap();
        let raw = answered.acknowledgement.expect("ack bytes");

        ibc2_packet_ack(
            registry.as_mut(),
            mock_env(),
            Ibc2PacketAckMsg::new(
                "07-tendermint-1".into(),
                "dest".into(),
                Ibc2Payload::new(
                    ibc2_source_port(&mock_env().contract.address),
                    DEX_QUOTE_PORT.into(),
                    DEX_QUOTE_VERSION.into(),
                    DEX_QUOTE_ENCODING.into(),
                    to_json_binary(&req).unwrap(),
                ),
                raw,
                Addr::unchecked("relayer"),
            ),
        )
        .unwrap();

        let bin = reg_query(
            registry.as_ref(),
            mock_env(),
            RegQuery::Latest {
                market_id: "PEN/USD".into(),
            },
        )
        .unwrap();
        let latest: ObservationResponse = from_json(bin).unwrap();
        assert_eq!(latest.source, SourceKind::DexQuote);
        assert_eq!(latest.mid.to_string(), "0.25");
        assert_eq!(latest.position_id.as_deref(), Some("pos-9"));
        assert_eq!(latest.pool_id, None);
    }
}
