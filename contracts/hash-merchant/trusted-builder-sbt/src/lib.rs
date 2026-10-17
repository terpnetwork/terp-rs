//! Soul-bound trusted-builder token. One mint per Ethereum address.
//! Transfer is not implemented — walkaway is a new mint on a new account, not a send.
//!
//! Two mint paths:
//! 1. **VE sidecar** — ETH `pledge()` first, hash-market records the claim, admin `Mint`.
//! 2. **IBC-v2 Australge** — user `CommitAustralge` on Terp; packet is sent over IBC-v2
//!    to ETH; after the manifesto is pledged, an attestation packet (or relayer
//!    `DeliverEthAttestation`) mints here with the ETH tx hash.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, from_json, to_json_binary, Binary, CosmosMsg, Deps, DepsMut, Env, Ibc2Msg,
    Ibc2PacketAckMsg, Ibc2PacketReceiveMsg, Ibc2PacketSendMsg, Ibc2PacketTimeoutMsg, Ibc2Payload,
    IbcBasicResponse, IbcReceiveResponse, MessageInfo, Response, StdAck, StdResult, Timestamp,
};
use cw_storage_plus::{Item, Map};
use thiserror::Error;

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::TrustedBuilderSbt;

pub const IBC2_VERSION: &str = "australge-1";
pub const IBC2_ENCODING: &str = "application/json";

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub manifesto_contract: String,
    #[serde(default)]
    pub ibc2_source_client: Option<String>,
    #[serde(default)]
    pub ibc2_dest_port: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Path 1: mint after hashmerchant VE / sidecar recorded the manifesto pledge.
    Mint {
        eth_address: String,
        sign_tx: String,
        ve_commitment_hex: String,
    },
    /// Admin registers the IBC-v2 counterparty (no v1 handshake). This is the
    /// app-level "connection" to ETH (Eureka client id + dest port).
    RegisterIbcV2 {
        source_client: String,
        dest_port: String,
    },
    /// Path 2: user commits to the Australge / Trustless Manifesto on Terp.
    /// Stores a pending commitment and, if a client is registered, emits
    /// `Ibc2Msg::SendPacket` so ETH can pledge and attest back.
    CommitAustralge {
        eth_address: String,
        commitment_hex: String,
    },
    /// Emit `Ibc2Msg::SendPacket` for a pending Australge commitment (requires
    /// `RegisterIbcV2`). Separate from commit so the on-chain commitment works
    /// even when the host has no IBC-v2 client yet.
    SendAustralgePacket { eth_address: String },
    /// Relayer/admin delivery of an ETH pledge attestation (same mint as
    /// `ibc2_packet_receive`). Used when a real Eureka relayer is not in the loop.
    DeliverEthAttestation {
        eth_address: String,
        pledge_tx: String,
        commitment_hex: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(Token)]
    Token { eth_address: String },
    #[returns(Commitment)]
    Commitment { eth_address: String },
}

#[cw_serde]
pub enum MintPath {
    VeSidecar,
    IbcV2Australge,
}

#[cw_serde]
pub enum CommitmentStatus {
    Pending,
    Completed,
    TimedOut,
}

#[cw_serde]
pub struct Config {
    pub admin: String,
    pub manifesto_contract: String,
    pub next_id: u64,
    pub ibc2_source_client: Option<String>,
    pub ibc2_dest_port: Option<String>,
}

#[cw_serde]
pub struct Token {
    pub token_id: u64,
    pub eth_address: String,
    pub owner: String,
    pub sign_tx: String,
    pub ve_commitment_hex: String,
    pub minted_height: u64,
    pub mint_path: MintPath,
}

#[cw_serde]
pub struct Commitment {
    pub eth_address: String,
    pub committer: String,
    pub commitment_hex: String,
    pub height: u64,
    pub status: CommitmentStatus,
}

/// Outbound IBC-v2 payload (Terp → ETH).
#[cw_serde]
pub enum Ibc2Packet {
    CommitAustralge {
        eth_address: String,
        commitment_hex: String,
        committer: String,
    },
    /// Inbound attestation (ETH → Terp).
    EthAttestation {
        eth_address: String,
        pledge_tx: String,
        commitment_hex: String,
    },
}

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] cosmwasm_std::StdError),
    #[error("unauthorized")]
    Unauthorized,
    #[error("already minted for this eth address")]
    AlreadyMinted,
    #[error("empty field")]
    EmptyField,
    #[error("no pending australge commitment")]
    NoCommitment,
    #[error("commitment already finished")]
    CommitmentFinished,
    #[error("commitment already pending")]
    CommitmentPending,
    #[error("ibc2 send not authorized for this signer")]
    Ibc2SendDenied,
}

const CONFIG: Item<Config> = Item::new("config");
const BY_ETH: Map<&str, Token> = Map::new("by_eth");
const COMMIT: Map<&str, Commitment> = Map::new("commit");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    mut deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.admin.is_empty() || msg.manifesto_contract.is_empty() {
        return Err(ContractError::EmptyField);
    }
    CONFIG.save(
        deps.storage,
        &Config {
            admin: deps.api.addr_validate(&msg.admin)?.to_string(),
            manifesto_contract: msg.manifesto_contract,
            next_id: 1,
            ibc2_source_client: empty_to_none(msg.ibc2_source_client),
            ibc2_dest_port: empty_to_none(msg.ibc2_dest_port),
        },
    )?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Mint {
            eth_address,
            sign_tx,
            ve_commitment_hex,
        } => exec_mint_ve(deps, env, info, eth_address, sign_tx, ve_commitment_hex),
        ExecuteMsg::RegisterIbcV2 {
            source_client,
            dest_port,
        } => exec_register_ibc2(deps, info, source_client, dest_port),
        ExecuteMsg::CommitAustralge {
            eth_address,
            commitment_hex,
        } => exec_commit(deps, env, info, eth_address, commitment_hex),
        ExecuteMsg::SendAustralgePacket { eth_address } => {
            exec_send_packet(deps, env, info, eth_address)
        }
        ExecuteMsg::DeliverEthAttestation {
            eth_address,
            pledge_tx,
            commitment_hex,
        } => {
            only_admin(deps.as_ref(), &info)?;
            mint_from_eth_attestation(deps, env, info.sender.to_string(), eth_address, pledge_tx, commitment_hex)
        }
    }
}

fn exec_register_ibc2(
    deps: DepsMut,
    info: MessageInfo,
    source_client: String,
    dest_port: String,
) -> Result<Response, ContractError> {
    only_admin(deps.as_ref(), &info)?;
    if source_client.is_empty() || dest_port.is_empty() {
        return Err(ContractError::EmptyField);
    }
    let mut cfg = CONFIG.load(deps.storage)?;
    cfg.ibc2_source_client = Some(source_client.clone());
    cfg.ibc2_dest_port = Some(dest_port.clone());
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "register_ibc_v2")
        .add_attribute("source_client", source_client)
        .add_attribute("dest_port", dest_port))
}

fn exec_commit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    eth_address: String,
    commitment_hex: String,
) -> Result<Response, ContractError> {
    let eth = normalize_eth(&eth_address)?;
    if commitment_hex.is_empty() {
        return Err(ContractError::EmptyField);
    }
    if BY_ETH.may_load(deps.storage, &eth)?.is_some() {
        return Err(ContractError::AlreadyMinted);
    }
    if let Some(existing) = COMMIT.may_load(deps.storage, &eth)? {
        if matches!(existing.status, CommitmentStatus::Pending) {
            return Err(ContractError::CommitmentPending);
        }
    }
    let rec = Commitment {
        eth_address: eth.clone(),
        committer: info.sender.to_string(),
        commitment_hex: commitment_hex.clone(),
        height: env.block.height,
        status: CommitmentStatus::Pending,
    };
    COMMIT.save(deps.storage, &eth, &rec)?;

    Ok(Response::new()
        .add_attribute("action", "commit_australge")
        .add_attribute("eth", eth)
        .add_attribute("committer", info.sender.to_string()))
}

fn exec_send_packet(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    eth_address: String,
) -> Result<Response, ContractError> {
    let eth = normalize_eth(&eth_address)?;
    let pending = COMMIT
        .may_load(deps.storage, &eth)?
        .ok_or(ContractError::NoCommitment)?;
    if !matches!(pending.status, CommitmentStatus::Pending) {
        return Err(ContractError::CommitmentFinished);
    }
    if pending.committer != info.sender.as_str() {
        only_admin(deps.as_ref(), &info)?;
    }
    let cfg = CONFIG.load(deps.storage)?;
    let source_client = cfg
        .ibc2_source_client
        .ok_or(ContractError::EmptyField)?;
    let dest_port = cfg.ibc2_dest_port.ok_or(ContractError::EmptyField)?;
    let packet = Ibc2Packet::CommitAustralge {
        eth_address: eth.clone(),
        commitment_hex: pending.commitment_hex,
        committer: pending.committer,
    };
    let payload = Ibc2Payload::new(
        format!("wasm.{}", env.contract.address),
        dest_port,
        IBC2_VERSION.into(),
        IBC2_ENCODING.into(),
        to_json_binary(&packet)?,
    );
    let timeout: Timestamp = env.block.time.plus_seconds(600);
    let msg = Ibc2Msg::SendPacket {
        source_client,
        timeout,
        payloads: vec![payload],
    };
    Ok(Response::new()
        .add_message(CosmosMsg::from(msg))
        .add_attribute("action", "send_australge_packet")
        .add_attribute("eth", eth))
}

fn exec_mint_ve(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    eth_address: String,
    sign_tx: String,
    ve_commitment_hex: String,
) -> Result<Response, ContractError> {
    only_admin(deps.as_ref(), &info)?;
    let eth = normalize_eth(&eth_address)?;
    if sign_tx.is_empty() || ve_commitment_hex.is_empty() {
        return Err(ContractError::EmptyField);
    }
    write_token(
        deps,
        env,
        eth,
        info.sender.to_string(),
        sign_tx,
        ve_commitment_hex,
        MintPath::VeSidecar,
    )
}

fn mint_from_eth_attestation(
    mut deps: DepsMut,
    env: Env,
    owner: String,
    eth_address: String,
    pledge_tx: String,
    commitment_hex: String,
) -> Result<Response, ContractError> {
    let eth = normalize_eth(&eth_address)?;
    if pledge_tx.is_empty() || commitment_hex.is_empty() {
        return Err(ContractError::EmptyField);
    }
    let mut pending = COMMIT
        .may_load(deps.storage, &eth)?
        .ok_or(ContractError::NoCommitment)?;
    if !matches!(pending.status, CommitmentStatus::Pending) {
        return Err(ContractError::CommitmentFinished);
    }
    if pending.commitment_hex != commitment_hex {
        return Err(ContractError::EmptyField);
    }
    let res = write_token(
        deps.branch(),
        env,
        eth.clone(),
        owner,
        pledge_tx,
        commitment_hex,
        MintPath::IbcV2Australge,
    )?;
    pending.status = CommitmentStatus::Completed;
    COMMIT.save(deps.storage, &eth, &pending)?;
    Ok(res.add_attribute("path", "ibc_v2_australge"))
}

fn write_token(
    deps: DepsMut,
    env: Env,
    eth: String,
    owner: String,
    sign_tx: String,
    ve_commitment_hex: String,
    mint_path: MintPath,
) -> Result<Response, ContractError> {
    if BY_ETH.may_load(deps.storage, &eth)?.is_some() {
        return Err(ContractError::AlreadyMinted);
    }
    let mut cfg = CONFIG.load(deps.storage)?;
    let token = Token {
        token_id: cfg.next_id,
        eth_address: eth.clone(),
        owner,
        sign_tx,
        ve_commitment_hex,
        minted_height: env.block.height,
        mint_path,
    };
    BY_ETH.save(deps.storage, &eth, &token)?;
    cfg.next_id += 1;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "mint_trusted_builder")
        .add_attribute("token_id", token.token_id.to_string())
        .add_attribute("eth", eth))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::Token { eth_address } => {
            let t = BY_ETH.load(deps.storage, &eth_address.to_ascii_lowercase())?;
            to_json_binary(&t)
        }
        QueryMsg::Commitment { eth_address } => {
            let c = COMMIT.load(deps.storage, &eth_address.to_ascii_lowercase())?;
            to_json_binary(&c)
        }
    }
}

/// Permissionless send check: only the pending committer (or admin) may emit
/// an Australge packet from this contract's IBC-v2 port.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_send(
    deps: DepsMut,
    _env: Env,
    msg: Ibc2PacketSendMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if msg.signer.as_str() == cfg.admin {
        return Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_send_ok_admin"));
    }
    let packet: Ibc2Packet = from_json(&msg.payload.value)?;
    match packet {
        Ibc2Packet::CommitAustralge { eth_address, .. } => {
            let eth = normalize_eth(&eth_address)?;
            let pending = COMMIT
                .may_load(deps.storage, &eth)?
                .ok_or(ContractError::Ibc2SendDenied)?;
            if pending.committer != msg.signer.as_str() {
                return Err(ContractError::Ibc2SendDenied);
            }
            Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_send_ok"))
        }
        Ibc2Packet::EthAttestation { .. } => {
            if msg.signer.as_str() == cfg.admin {
                Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_send_ok_attestation"))
            } else {
                Err(ContractError::Ibc2SendDenied)
            }
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_receive(
    deps: DepsMut,
    env: Env,
    msg: Ibc2PacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    let packet: Ibc2Packet = from_json(&msg.payload.value)?;
    match packet {
        Ibc2Packet::EthAttestation {
            eth_address,
            pledge_tx,
            commitment_hex,
        } => {
            mint_from_eth_attestation(
                deps,
                env,
                msg.relayer.to_string(),
                eth_address,
                pledge_tx,
                commitment_hex,
            )?;
            Ok(IbcReceiveResponse::new(StdAck::success(b"ok").to_binary())
                .add_attribute("action", "ibc2_recv_eth_attestation"))
        }
        Ibc2Packet::CommitAustralge { .. } => {
            // Inbound commit is unexpected on Terp (we are the source). Ack ok, no mint.
            Ok(IbcReceiveResponse::new(StdAck::success(b"ignored").to_binary())
                .add_attribute("action", "ibc2_recv_commit_ignored"))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: Ibc2PacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_ack"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc2_packet_timeout(
    deps: DepsMut,
    _env: Env,
    msg: Ibc2PacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    if let Ok(Ibc2Packet::CommitAustralge { eth_address, .. }) = from_json(&msg.payload.value) {
        if let Ok(eth) = normalize_eth(&eth_address) {
            if let Some(mut c) = COMMIT.may_load(deps.storage, &eth)? {
                if matches!(c.status, CommitmentStatus::Pending) {
                    c.status = CommitmentStatus::TimedOut;
                    COMMIT.save(deps.storage, &eth, &c)?;
                }
            }
        }
    }
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc2_timeout"))
}

fn only_admin(deps: Deps, info: &MessageInfo) -> Result<(), ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender.as_str() != cfg.admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn normalize_eth(eth: &str) -> Result<String, ContractError> {
    if eth.is_empty() {
        return Err(ContractError::EmptyField);
    }
    Ok(eth.to_ascii_lowercase())
}

fn empty_to_none(s: Option<String>) -> Option<String> {
    s.and_then(|v| if v.is_empty() { None } else { Some(v) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{
        message_info, mock_dependencies, mock_env, mock_ibc2_packet_recv, mock_ibc2_packet_send,
        mock_ibc2_packet_timeout,
    };
    use cosmwasm_std::{from_json, Addr};

    fn setup() -> (cosmwasm_std::OwnedDeps<cosmwasm_std::testing::MockStorage, cosmwasm_std::testing::MockApi, cosmwasm_std::testing::MockQuerier>, Addr) {
        let mut deps = mock_dependencies();
        let admin = deps.api.addr_make("admin");
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            InstantiateMsg {
                admin: admin.to_string(),
                manifesto_contract: "0x32aa964746ba2be65c71fe4a5cb3c4a023ca3e20".into(),
                ibc2_source_client: None,
                ibc2_dest_port: None,
            },
        )
        .unwrap();
        (deps, admin)
    }

    #[test]
    fn sane_mint_then_query() {
        let (mut deps, admin) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            ExecuteMsg::Mint {
                eth_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
                sign_tx: "0xabc".into(),
                ve_commitment_hex: "aa".into(),
            },
        )
        .unwrap();
        let bin = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Token {
                eth_address: "0xF39FD6E51AAD88F6F4CE6AB8827279CFFFB92266".into(),
            },
        )
        .unwrap();
        let tok: Token = from_json(bin).unwrap();
        assert_eq!(tok.token_id, 1);
        assert_eq!(tok.eth_address, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        assert!(matches!(tok.mint_path, MintPath::VeSidecar));
    }

    #[test]
    fn sane_commit_then_ibc2_receive_mints() {
        let (mut deps, admin) = setup();
        let user = deps.api.addr_make("user");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::CommitAustralge {
                eth_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into(),
                commitment_hex: "cafebabe".into(),
            },
        )
        .unwrap();
        let pkt = Ibc2Packet::EthAttestation {
            eth_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into(),
            pledge_tx: "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
            commitment_hex: "cafebabe".into(),
        };
        let recv = mock_ibc2_packet_recv(&pkt).unwrap();
        ibc2_packet_receive(deps.as_mut(), mock_env(), recv).unwrap();
        let tok: Token = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::Token {
                    eth_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(tok.mint_path, MintPath::IbcV2Australge));
        assert!(tok.sign_tx.starts_with("0xdd"));
        let c: Commitment = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::Commitment {
                    eth_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(c.status, CommitmentStatus::Completed));
    }

    #[test]
    fn sane_register_ibc2_then_commit_emits_packet() {
        let (mut deps, admin) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            ExecuteMsg::RegisterIbcV2 {
                source_client: "07-tendermint-eth".into(),
                dest_port: "ics26-router".into(),
            },
        )
        .unwrap();
        let user = deps.api.addr_make("user");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::CommitAustralge {
                eth_address: "0xabc".into(),
                commitment_hex: "11".into(),
            },
        )
        .unwrap();
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::SendAustralgePacket {
                eth_address: "0xabc".into(),
            },
        )
        .unwrap();
        assert_eq!(res.messages.len(), 1);
    }

    #[test]
    fn insane_non_admin_cannot_mint() {
        let (mut deps, _admin) = setup();
        let other = deps.api.addr_make("other");
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&other, &[]),
            ExecuteMsg::Mint {
                eth_address: "0x1".into(),
                sign_tx: "0x2".into(),
                ve_commitment_hex: "bb".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Unauthorized));
    }

    #[test]
    fn insane_double_mint_same_eth() {
        let (mut deps, admin) = setup();
        let msg = ExecuteMsg::Mint {
            eth_address: "0xabc".into(),
            sign_tx: "0x1".into(),
            ve_commitment_hex: "cc".into(),
        };
        execute(deps.as_mut(), mock_env(), message_info(&admin, &[]), msg.clone()).unwrap();
        let err = execute(deps.as_mut(), mock_env(), message_info(&admin, &[]), msg).unwrap_err();
        assert!(matches!(err, ContractError::AlreadyMinted));
    }

    #[test]
    fn insane_empty_instantiate() {
        let mut deps = mock_dependencies();
        let admin = deps.api.addr_make("admin");
        let err = instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            InstantiateMsg {
                admin: String::new(),
                manifesto_contract: String::new(),
                ibc2_source_client: None,
                ibc2_dest_port: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::EmptyField));
    }

    #[test]
    fn insane_ibc2_send_denied_without_commitment() {
        let (mut deps, _admin) = setup();
        let pkt = Ibc2Packet::CommitAustralge {
            eth_address: "0xabc".into(),
            commitment_hex: "11".into(),
            committer: "nobody".into(),
        };
        let send = mock_ibc2_packet_send(&pkt).unwrap();
        let err = ibc2_packet_send(deps.as_mut(), mock_env(), send).unwrap_err();
        assert!(matches!(err, ContractError::Ibc2SendDenied));
    }

    #[test]
    fn insane_timeout_marks_pending() {
        let (mut deps, _admin) = setup();
        let user = deps.api.addr_make("user");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::CommitAustralge {
                eth_address: "0xabc".into(),
                commitment_hex: "11".into(),
            },
        )
        .unwrap();
        let pkt = Ibc2Packet::CommitAustralge {
            eth_address: "0xabc".into(),
            commitment_hex: "11".into(),
            committer: user.to_string(),
        };
        let to = mock_ibc2_packet_timeout(&pkt).unwrap();
        ibc2_packet_timeout(deps.as_mut(), mock_env(), to).unwrap();
        let c: Commitment = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::Commitment {
                    eth_address: "0xabc".into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(c.status, CommitmentStatus::TimedOut));
    }

    #[test]
    fn insane_cross_path_same_eth() {
        let (mut deps, admin) = setup();
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&admin, &[]),
            ExecuteMsg::Mint {
                eth_address: "0xabc".into(),
                sign_tx: "0x1".into(),
                ve_commitment_hex: "cc".into(),
            },
        )
        .unwrap();
        let user = deps.api.addr_make("user");
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::CommitAustralge {
                eth_address: "0xabc".into(),
                commitment_hex: "11".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::AlreadyMinted));
    }
}
