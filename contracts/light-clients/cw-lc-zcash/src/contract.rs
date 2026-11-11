use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, to_json_binary,
};
use cw_ics08_wasm_zap1::{IbcV2Fields, VerifyZap1Ibcv2Params, verify_zap1_ibcv2};
use cw_lc_aggregator::msg::QueryMsg as AggQuery;
use cw_lc_aggregator::state::{PreconfRecord, PreconfStatus};
use cw_lc_registry::label::validate_label;
use cw_lc_registry::msg::QueryMsg as RegQuery;

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

const CONTRACT_NAME: &str = "crates.io:cw-lc-zcash";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ROLE: &str = "client";

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let target = validate_label(&msg.label, ROLE)?.to_string();
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            label: msg.label.clone(),
            target,
            registry: deps.api.addr_validate(&msg.registry)?,
            aggregator: deps.api.addr_validate(&msg.aggregator)?,
            registry_client_id: msg.registry_client_id,
            ibc_client_id: msg.ibc_client_id,
            accept_preconfirmed: msg.accept_preconfirmed,
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("label", msg.label))
}

fn hex32(field: &str, v: &str) -> Result<[u8; 32], ContractError> {
    hex::decode(v.trim().trim_start_matches("0x"))
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| ContractError::BadHex(field.into()))
}

fn header(deps: Deps, cfg: &Config, height: u64) -> StdResult<Option<PreconfRecord>> {
    deps.querier
        .query_wasm_smart(&cfg.aggregator, &AggQuery::Preconf { height })
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::VerifyPaymentInclusion(m) => {
            let m = *m;
            if m.dest_client != cfg.ibc_client_id {
                return Err(ContractError::WrongDest {
                    got: m.dest_client,
                    want: cfg.ibc_client_id,
                });
            }
            if PACKETS.has(deps.storage, (&m.source_client, m.sequence)) {
                return Err(ContractError::Replay(m.source_client, m.sequence));
            }
            let rec = header(deps.as_ref(), &cfg, m.height)?
                .ok_or(ContractError::NoHeader(m.height))?;
            if rec.status != PreconfStatus::Confirmed && !cfg.accept_preconfirmed {
                return Err(ContractError::NotFinal {
                    height: m.height,
                    status: format!("{:?}", rec.status),
                });
            }
            let root_hex = rec
                .header
                .payment_root
                .clone()
                .ok_or(ContractError::NoPaymentRoot(m.height))?;
            let root = hex32("payment_root", &root_hex)?;
            let note = hex32("note_hex", &m.note_hex)?;
            let frost = hex32("frost_group_hex", &m.frost_group_hex)?;
            let leaf = hex32("leaf_hex", &m.leaf_hex)?;
            let mut steps = Vec::with_capacity(m.proof.len());
            for s in &m.proof {
                steps.push((hex32("sibling_hex", &s.sibling_hex)?, s.sibling_is_left));
            }
            let h = verify_zap1_ibcv2(
                &IbcV2Fields {
                    source_client: m.source_client.clone(),
                    dest_client: m.dest_client.clone(),
                    sequence: m.sequence,
                    timeout_timestamp_ns: m.timeout_timestamp_ns,
                    app_data_hash_hex: m.app_data_hash_hex.clone(),
                },
                VerifyZap1Ibcv2Params {
                    event_kind: &m.event_kind,
                    wallet_or_serial: &m.wallet_or_serial,
                    note: &note,
                    amount_zat: m.amount_zat,
                    frost_group: &frost,
                    leaf: &leaf,
                    merkle_root: &root,
                    proof: &steps,
                },
            )
            .map_err(ContractError::Inclusion)?;
            let key = hex::encode(h);
            PACKETS.save(deps.storage, (&m.source_client, m.sequence), &key)?;
            INCLUSIONS.save(
                deps.storage,
                &key,
                &Inclusion {
                    app_data_hash: key.clone(),
                    height: m.height,
                    block_hash: rec.header.hash.to_ascii_lowercase(),
                    payment_root: root_hex.to_ascii_lowercase(),
                    leaf: hex::encode(leaf),
                    source_client: m.source_client,
                    sequence: m.sequence,
                    note: hex::encode(note),
                    amount_zat: m.amount_zat,
                    frost_group: hex::encode(frost),
                    recorded_at: env.block.height,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "verify_payment_inclusion")
                .add_attribute("height", m.height.to_string())
                .add_attribute("app_data_hash", key))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        QueryMsg::Config {} => to_json_binary(&cfg),
        QueryMsg::PaymentInclusion { app_data_hash_hex } => {
            let key = app_data_hash_hex
                .trim()
                .trim_start_matches("0x")
                .to_ascii_lowercase();
            let Some(inc) = INCLUSIONS.may_load(deps.storage, &key)? else {
                return to_json_binary(&None::<PaymentInclusionResponse>);
            };
            let status = match header(deps, &cfg, inc.height)? {
                Some(r) if r.header.hash.eq_ignore_ascii_case(&inc.block_hash) => match r.status {
                    PreconfStatus::Confirmed => InclusionStatus::Confirmed,
                    PreconfStatus::Preconfirmed => InclusionStatus::Preconfirmed,
                },
                _ => InclusionStatus::Reorged,
            };
            to_json_binary(&Some(PaymentInclusionResponse {
                inclusion: inc,
                status,
            }))
        }
        QueryMsg::LatestHeader {} => to_json_binary(
            &deps
                .querier
                .query_wasm_smart::<cw_lc_aggregator::msg::BestHeaderResponse>(
                    &cfg.aggregator,
                    &AggQuery::BestHeader {},
                )?,
        ),
        QueryMsg::ConsensusTip {} => to_json_binary(
            &deps
                .querier
                .query_wasm_smart::<cw_lc_registry::msg::ConsensusTipResponse>(
                    &cfg.registry,
                    &RegQuery::ConsensusTip {
                        client_id: cfg.registry_client_id.clone(),
                    },
                )?,
        ),
        QueryMsg::NodeStates {} => to_json_binary(
            &deps
                .querier
                .query_wasm_smart::<cw_lc_registry::msg::NodeStatesResponse>(
                    &cfg.registry,
                    &RegQuery::NodeStates {
                        client_id: cfg.registry_client_id.clone(),
                    },
                )?,
        ),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(
    deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: cosmwasm_std::MigrateInfo,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attribute("action", "migrate"))
}
