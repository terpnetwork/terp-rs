use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Binary, Deps, DepsMut, Env, Ibc3ChannelOpenResponse,
    IbcBasicResponse, IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcOrder,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, MessageInfo,
    Response, StdError, StdResult,
};
use cw_storage_plus::{Item, Map};
use serde::{Deserialize, Serialize};

const CONTRACT_NAME: &str = "cw-dregg-svg-gate";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const IBC_VERSION: &str = "svgmint-dregg-v1";

const CHANNEL: Item<String> = Item::new("ch");
const PORT: Item<String> = Item::new("port");
const IBC_ONLY: Item<bool> = Item::new("ibc_only");
const RUNTIME: Map<&str, ()> = Map::new("runtime");
const SPENT: Map<&str, ()> = Map::new("spent");
const ORACLE_THRESHOLD: Item<u32> = Item::new("oth");
const ORACLE_KEYS: Item<Vec<Binary>> = Item::new("okeys");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct InstantiateMsg {
    pub dregg_ibc_only: bool,
    pub runtime_minters: Vec<String>,
    pub dregg_channel: Option<String>,
    pub dregg_port: Option<String>,
    pub oracle_threshold: u32,
    pub oracle_pubkeys: Vec<Binary>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct MigrateMsg {
    pub dregg_channel: Option<String>,
    pub dregg_port: Option<String>,
    pub dregg_ibc_only: Option<bool>,
    pub runtime_minters: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// IBC-equivalent: DREGG runtime submits a commitment packet (lab + runtime accounts).
    RecvDreggPacket {
        packet: MintPacket,
        signatures: Vec<OracleSig>,
    },
    SpendNullifier {
        n_deposit: [u8; 32],
        receipt: [u8; 32],
        signatures: Vec<OracleSig>,
    },
    BindDreggChannel { channel: String, port: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct OracleSig {
    pub pubkey: Binary,
    pub signature: Binary,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct MintPacket {
    pub c_deposit: [u8; 32],
    pub n_deposit: [u8; 32],
    pub c_action: [u8; 32],
    pub bind: [u8; 32],
    pub receipt: [u8; 32],
    #[serde(default)]
    pub token_recipient: String,
    #[serde(default)]
    pub collection: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    DreggIbc {},
    Spent { n_hex: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, schemars::JsonSchema)]
pub struct DreggIbcStatus {
    pub channel: Option<String>,
    pub port: Option<String>,
    pub ibc_only: bool,
}

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    IBC_ONLY.save(deps.storage, &msg.dregg_ibc_only)?;
    for a in msg.runtime_minters {
        let addr = deps.api.addr_validate(&a)?;
        RUNTIME.save(deps.storage, addr.as_str(), &())?;
    }
    if let Some(ch) = msg.dregg_channel {
        CHANNEL.save(deps.storage, &ch)?;
    }
    if let Some(p) = msg.dregg_port {
        PORT.save(deps.storage, &p)?;
    }
    if msg.oracle_threshold == 0 || msg.oracle_pubkeys.len() < msg.oracle_threshold as usize {
        return Err(StdError::generic_err("oracle threshold"));
    }
    ORACLE_THRESHOLD.save(deps.storage, &msg.oracle_threshold)?;
    ORACLE_KEYS.save(deps.storage, &msg.oracle_pubkeys)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> StdResult<Response> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    if let Some(ch) = msg.dregg_channel {
        CHANNEL.save(deps.storage, &ch)?;
    }
    if let Some(p) = msg.dregg_port {
        PORT.save(deps.storage, &p)?;
    }
    if let Some(only) = msg.dregg_ibc_only {
        IBC_ONLY.save(deps.storage, &only)?;
    }
    if let Some(addrs) = msg.runtime_minters {
        for a in addrs {
            let addr = deps.api.addr_validate(&a)?;
            RUNTIME.save(deps.storage, addr.as_str(), &())?;
        }
    }
    Ok(Response::new().add_attribute("action", "migrate"))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::BindDreggChannel { channel, port } => {
            if !RUNTIME.has(deps.storage, info.sender.as_str()) {
                return Err(StdError::generic_err("unauthorized"));
            }
            CHANNEL.save(deps.storage, &channel)?;
            PORT.save(deps.storage, &port)?;
            Ok(Response::new().add_attribute("action", "bind_channel"))
        }
        ExecuteMsg::RecvDreggPacket { packet, signatures } => {
            if !RUNTIME.has(deps.storage, info.sender.as_str()) {
                return Err(StdError::generic_err("unauthorized"));
            }
            accept_packet(deps, packet, signatures)?;
            Ok(Response::new().add_attribute("action", "recv_dregg_packet"))
        }
        ExecuteMsg::SpendNullifier {
            n_deposit,
            receipt,
            signatures,
        } => {
            if !RUNTIME.has(deps.storage, info.sender.as_str()) {
                return Err(StdError::generic_err("unauthorized"));
            }
            spend_with_qc(deps, n_deposit, receipt, signatures)?;
            Ok(Response::new().add_attribute("action", "spend_nullifier"))
        }
    }
}

fn nhex(n: &[u8; 32]) -> String {
    n.iter().map(|b| format!("{b:02x}")).collect()
}

fn dummy_receipt(r: &[u8; 32]) -> bool {
    r.iter().all(|b| *b == r[0])
}

fn verify_qc(deps: Deps, digest: &[u8], sigs: &[OracleSig]) -> StdResult<()> {
    let threshold = ORACLE_THRESHOLD.load(deps.storage)?;
    let keys = ORACLE_KEYS.load(deps.storage)?;
    let mut got = 0u32;
    let mut seen: Vec<Binary> = Vec::new();
    for s in sigs {
        if !keys.iter().any(|k| k == &s.pubkey) {
            continue;
        }
        if seen.iter().any(|k| k == &s.pubkey) {
            continue;
        }
        match deps
            .api
            .ed25519_verify(digest, s.signature.as_slice(), s.pubkey.as_slice())
        {
            Ok(true) => {
                seen.push(s.pubkey.clone());
                got += 1;
            }
            _ => continue,
        }
    }
    if got < threshold {
        return Err(StdError::generic_err("threshold not met"));
    }
    Ok(())
}

fn spend_with_qc(
    deps: DepsMut,
    n_deposit: [u8; 32],
    receipt: [u8; 32],
    sigs: Vec<OracleSig>,
) -> StdResult<()> {
    if dummy_receipt(&receipt) {
        return Err(StdError::generic_err("dummy receipt"));
    }
    verify_qc(deps.as_ref(), &receipt, &sigs)?;
    let id = nhex(&n_deposit);
    if SPENT.has(deps.storage, &id) {
        return Err(StdError::generic_err("nullifier spent"));
    }
    SPENT.save(deps.storage, &id, &())?;
    Ok(())
}

fn accept_packet(deps: DepsMut, pkt: MintPacket, sigs: Vec<OracleSig>) -> StdResult<()> {
    let blob = format!("{pkt:?}").to_ascii_lowercase();
    for k in ["txid", "rail", "ironwood", "ethereum", "bitcoin"] {
        if blob.contains(k) {
            return Err(StdError::generic_err("packet leaked payment source"));
        }
    }
    if dummy_receipt(&pkt.receipt) {
        return Err(StdError::generic_err("dummy receipt"));
    }
    verify_qc(deps.as_ref(), &pkt.receipt, &sigs)?;
    let id = nhex(&pkt.n_deposit);
    if SPENT.has(deps.storage, &id) {
        return Err(StdError::generic_err("nullifier spent"));
    }
    SPENT.save(deps.storage, &id, &())?;
    Ok(())
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::DreggIbc {} => to_json_binary(&DreggIbcStatus {
            channel: CHANNEL.may_load(deps.storage)?,
            port: PORT.may_load(deps.storage)?,
            ibc_only: IBC_ONLY.may_load(deps.storage)?.unwrap_or(false),
        }),
        QueryMsg::Spent { n_hex } => {
            to_json_binary(&SPENT.has(deps.storage, &n_hex.to_ascii_lowercase()))
        }
    }
}

#[entry_point]
pub fn ibc_channel_open(
    _deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> StdResult<Option<Ibc3ChannelOpenResponse>> {
    let chan = msg.channel();
    if chan.order != IbcOrder::Unordered {
        return Err(StdError::generic_err("unordered required"));
    }
    Ok(Some(Ibc3ChannelOpenResponse {
        version: IBC_VERSION.into(),
    }))
}

#[entry_point]
pub fn ibc_channel_connect(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelConnectMsg,
) -> StdResult<IbcBasicResponse> {
    let chan = msg.channel();
    CHANNEL.save(deps.storage, &chan.endpoint.channel_id)?;
    PORT.save(deps.storage, &chan.endpoint.port_id)?;
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_connect"))
}

#[entry_point]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_close"))
}

#[entry_point]
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> StdResult<IbcReceiveResponse> {
    let bound = CHANNEL.may_load(deps.storage)?;
    if bound.as_deref() != Some(msg.packet.dest.channel_id.as_str()) {
        return Ok(IbcReceiveResponse::new().set_ack(b"bad-channel".as_slice()));
    }
    #[derive(Deserialize)]
    struct Envelope {
        packet: MintPacket,
        signatures: Vec<OracleSig>,
    }
    let envl: Envelope = match from_json(&msg.packet.data) {
        Ok(e) => e,
        Err(_) => {
            return Ok(IbcReceiveResponse::new().set_ack(b"unsigned".as_slice()));
        }
    };
    if envl.signatures.is_empty() {
        return Ok(IbcReceiveResponse::new().set_ack(b"unsigned".as_slice()));
    }
    match accept_packet(deps, envl.packet, envl.signatures) {
        Ok(()) => Ok(IbcReceiveResponse::new().set_ack(b"ok".as_slice())),
        Err(e) => Ok(IbcReceiveResponse::new().set_ack(e.to_string().as_bytes())),
    }
}

#[entry_point]
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new())
}

#[entry_point]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};

    fn pkt() -> MintPacket {
        MintPacket {
            c_deposit: [1u8; 32],
            n_deposit: [2u8; 32],
            c_action: [3u8; 32],
            bind: [4u8; 32],
            receipt: [5u8; 32],
            token_recipient: "terp1runtime".into(),
            collection: "terp1coll".into(),
        }
    }

    #[test]
    fn stranger_refused_runtime_accepted() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("owner", &[]),
            InstantiateMsg {
                dregg_ibc_only: true,
                runtime_minters: vec!["terp1runtime".into()],
                dregg_channel: Some("channel-dregg".into()),
                dregg_port: Some("svgmint".into()),
                oracle_threshold: 1,
                oracle_pubkeys: vec![Binary::from(vec![1u8; 32])],
            },
        )
        .unwrap();
        let msg = ExecuteMsg::RecvDreggPacket {
            packet: pkt(),
            signatures: vec![],
        };
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("stranger", &[]),
            msg.clone(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unauthorized"));
        let err2 = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("terp1runtime", &[]),
            msg,
        )
        .unwrap_err();
        let es = err2.to_string();
        assert!(
            es.contains("dummy receipt") || es.contains("threshold"),
            "{es}"
        );
    }

    #[test]
    fn migrate_binds_channel() {
        let mut deps = mock_dependencies();
        instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("owner", &[]),
            InstantiateMsg {
                dregg_ibc_only: false,
                runtime_minters: vec![],
                dregg_channel: None,
                dregg_port: None,
                oracle_threshold: 1,
                oracle_pubkeys: vec![Binary::from(vec![1u8; 32])],
            },
        )
        .unwrap();
        migrate(
            deps.as_mut(),
            mock_env(),
            MigrateMsg {
                dregg_channel: Some("channel-7".into()),
                dregg_port: Some("svgmint".into()),
                dregg_ibc_only: Some(true),
                runtime_minters: Some(vec!["terp1rt".into()]),
            },
        )
        .unwrap();
        let st: DreggIbcStatus = from_json(
            query(deps.as_ref(), mock_env(), QueryMsg::DreggIbc {}).unwrap(),
        )
        .unwrap();
        assert_eq!(st.channel.as_deref(), Some("channel-7"));
        assert!(st.ibc_only);
    }
}
