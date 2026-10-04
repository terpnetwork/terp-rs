//! IBC receive from the DREGG federation client (sibling of Zakura/Anvil/bitcoind).
//!
//! Packet data is [`MintPacket`] commitments only. Channel/path must match the
//! bound dregg runtime. Inspired by PIR `cw-ics08-wasm-zap1` packet attest
//! (source/dest client + app data) — here the app data is the mint commitment.

use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Binary, DepsMut, Env, IbcBasicResponse,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse, IbcOrder,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, Never, StdError,
    StdResult, WasmMsg,
};

use crate::state::{DREGG_CHANNEL, DREGG_PORT, SPENT_NULLIFIERS};
use crate::ContractError;

const IBC_VERSION: &str = "svgmint-dregg-v1";

#[cosmwasm_schema::cw_serde]
pub struct MintPacket {
    pub c_deposit: Binary,
    pub n_deposit: Binary,
    pub c_action: Binary,
    pub bind: Binary,
    pub receipt: Binary,
    #[serde(default)]
    pub token_recipient: String,
    #[serde(default)]
    pub collection: String,
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_open(
    _deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> StdResult<IbcChannelOpenResponse> {
    let chan = msg.channel();
    if chan.order != IbcOrder::Unordered {
        return Err(StdError::msg("svgmint channel must be unordered"));
    }
    if chan.version != IBC_VERSION && !chan.version.is_empty() {
        return Err(StdError::msg("unknown ibc version"));
    }
    Ok(Some(cosmwasm_std::Ibc3ChannelOpenResponse {
        version: IBC_VERSION.into(),
    }))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_connect(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelConnectMsg,
) -> StdResult<IbcBasicResponse> {
    let chan = msg.channel();
    DREGG_CHANNEL.save(deps.storage, &chan.endpoint.channel_id)?;
    DREGG_PORT.save(deps.storage, &chan.endpoint.port_id)?;
    Ok(IbcBasicResponse::new()
        .add_attribute("action", "ibc_connect")
        .add_attribute("channel", chan.endpoint.channel_id.clone())
        .add_attribute("port", chan.endpoint.port_id.clone()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_close"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, Never> {
    match recv_inner(deps, msg) {
        Ok(resp) => Ok(resp),
        Err(e) => {
            let ack = to_json_binary(&format!("error:{e}")).unwrap_or_default();
            Ok(IbcReceiveResponse::new(ack).add_attribute("error", e.to_string()))
        }
    }
}

pub fn accept_execute(deps: DepsMut, pkt: MintPacket) -> Result<(), ContractError> {
    spend_nullifier(deps, &pkt)?;
    Ok(())
}

fn spend_nullifier(deps: DepsMut, pkt: &MintPacket) -> Result<(), ContractError> {
    let blob = format!("{pkt:?}").to_ascii_lowercase();
    for k in ["txid", "rail", "ironwood", "ethereum", "bitcoin"] {
        if blob.contains(k) {
            return Err(ContractError::Unauthorized {});
        }
    }
    let nid = pkt
        .n_deposit
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if SPENT_NULLIFIERS.has(deps.storage, nid.clone()) {
        return Err(ContractError::Unauthorized {});
    }
    SPENT_NULLIFIERS.save(deps.storage, nid, &())?;
    Ok(())
}

fn recv_inner(mut deps: DepsMut, msg: IbcPacketReceiveMsg) -> Result<IbcReceiveResponse, ContractError> {
    let bound = DREGG_CHANNEL
        .may_load(deps.storage)?
        .ok_or(ContractError::Unauthorized {})?;
    if msg.packet.dest.channel_id != bound {
        return Err(ContractError::Unauthorized {});
    }
    let pkt: MintPacket = from_json(&msg.packet.data)?;
    spend_nullifier(deps.branch(), &pkt)?;
    if pkt.collection.is_empty() || pkt.token_recipient.is_empty() {
        return Err(ContractError::Unauthorized {});
    }
    let mint = br#"{"update_extension":{"msg":{"mint":{"alloc":0,"amnt":1,"proof_hashes":[],"ext":null}}}}"#;
    let wasm = WasmMsg::Execute {
        contract_addr: pkt.collection,
        msg: mint.to_vec().into(),
        funds: vec![],
    };
    Ok(IbcReceiveResponse::new(b"ok")
        .add_message(wasm)
        .add_attribute("action", "ibc_mint")
        .add_attribute("recipient", pkt.token_recipient))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_ack"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_timeout"))
}
