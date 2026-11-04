/// Chain must advertise `hash_blake`. Note-denom proofs use `Api::blake3_256`.
#[unsafe(no_mangle)]
extern "C" fn requires_hash_blake() {}

pub mod bridge;
pub mod direct;
pub mod distro;
pub mod egress;
pub mod epoch;
pub mod forward;
pub mod notes;
pub mod reclaim;
pub mod pause;
pub mod registration;
pub mod headstash;
pub mod tf_proto;
pub mod msg;
pub mod tokenfactory;
pub mod wavs;

#[cfg(feature = "interface")]
pub mod interface;

#[cfg(all(test, feature = "interface"))]
mod epoch_orch;

// CosmWasm guest has no OS RNG — register unsupported custom getrandom so the
// dependency graph (via zk-headstash / rand) compiles for wasm32-unknown-unknown.
// Required for Daemon upload of BridgeMintNote (ict_local_funded).
#[cfg(target_arch = "wasm32")]
fn guest_getrandom(_dest: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
#[cfg(target_arch = "wasm32")]
getrandom::register_custom_getrandom!(guest_getrandom);

use crate::{
    distro::{DistroHashDomain, ELIGIBILITY_ROOTS, GENESIS_ROOT_ID, NEXT_ROOT_ID},
    headstash::*,
    tokenfactory::TokenStrategy,
    wavs::{WavsOperatorSet, WavsProofOfOwnership},
};

use cosmwasm_schema::{cw_serde, serde, QueryResponses};
use cosmwasm_std::{
    coins, to_json_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env,
    MessageInfo, Order, Reply, Response, StdError, StdResult, Storage, SubMsg, Uint128,
};
use cw_storage_plus::{Bound, Bounder, Item, KeyDeserialize, Map};
pub use msg::*;

// use cw2::{ContractVersion, get_contract_version, set_contract_version};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwHeadstashStructs {}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwHeadstash {}

// Version info for migration
const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Reply IDs
const CREATE_DENOM_REPLY_ID: u64 = 1;

pub const HEADSTASH_CFG: Item<HeadstashCfg> = Item::new("headstash_params");
pub(crate) const GENESIS_TREE_ROOT: Item<Binary> = Item::new("root_gen_tree");
pub(crate) const COMMITMENT_TREE_ROOT: Item<Binary> = Item::new("root_cm_tree");
pub(crate) const NULLIFIERS: Map<String, ()> = Map::new("nullifiers");
pub(crate) const DENOM: Item<String> = Item::new("denom");

// Allowance maps for minting and burning
pub const MINTER_ALLOWANCES: Map<Addr, Uint128> = Map::new("minter_allowances");
pub const BURNER_ALLOWANCES: Map<Addr, Uint128> = Map::new("burner_allowances");

// Hardcoded protobuf type URLs for tokenfactory messages
const MSG_CREATE_DENOM_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgCreateDenom";
const MSG_MINT_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgMint";
const MSG_BURN_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgBurn";

// Helper functions to create Stargate messages for tokenfactory operations.
// Values are protobuf (see tf_proto); wasmd rejects JSON with "Cannot unpack proto message".
fn msg_create_denom(sender: String, subdenom: String) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Stargate {
        type_url: MSG_CREATE_DENOM_TYPE_URL.to_string(),
        value: Binary::from(tf_proto::msg_create_denom(&sender, &subdenom)),
    })
}

pub(crate) fn msg_mint(sender: String, amount: Uint128, denom: String) -> StdResult<CosmosMsg> {
    // Mint to sender, then we'll send via BankMsg
    Ok(CosmosMsg::Stargate {
        type_url: MSG_MINT_TYPE_URL.to_string(),
        value: Binary::from(tf_proto::msg_coin_addr(&sender, &denom, &amount.to_string(), &sender)),
    })
}

fn msg_burn(
    sender: String,
    amount: Uint128,
    denom: String,
    burn_from_address: String,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Stargate {
        type_url: MSG_BURN_TYPE_URL.to_string(),
        value: Binary::from(tf_proto::msg_coin_addr(&sender, &denom, &amount.to_string(), &burn_from_address)),
    })
}

/// The only accessor for WAVS state. Fails closed when WAVS is disabled (`wavs: None`).
pub fn wavs_operator_set(storage: &dyn Storage) -> StdResult<WavsOperatorSet> {
    HEADSTASH_CFG
        .load(storage)?
        .w
        .ok_or_else(|| StdError::msg("wavs disabled"))
}

fn save_headstash_cfg(
    storage: &mut dyn Storage,
    genesis_root: Binary,
    distro_hash_domain: DistroHashDomain,
    genesis_label: Option<String>,
    ts: Vec<TokenStrategy>,
    w: Option<WavsOperatorSet>,
    circuit_id: u64,
) -> Result<(), StdError> {
    distro::validate_instantiate_domain(distro_hash_domain)?;
    distro::register_genesis_root(
        storage,
        genesis_root.clone(),
        distro_hash_domain,
        genesis_label,
    )?;
    GENESIS_TREE_ROOT.save(storage, &genesis_root)?;
    HEADSTASH_CFG.save(
        storage,
        &HeadstashCfg {
            gr: genesis_root,
            distro_hash_domain,
            ts,
            w,
            cid: circuit_id,
        },
    )?;
    Ok(())
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, StdError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(info.sender.as_str()))?;
    // WAVS is optional. None = disabled: nothing stored, no PoP/threshold checks.
    if let Some(w) = &msg.wavs {
        w.verify()?;
    }
    msg.token_strategy.validate(deps.api)?;
    let ts = msg.token_strategy;
    let c = env.contract.address.clone();
    let domain = msg.distro_hash_domain;
    let genesis_label = msg.genesis_label;
    let circuit_id = msg.circuit_id.unwrap_or(0);
    notes::set_registration_hook(deps.storage, deps.api, msg.registration_hook.clone(), msg.registration_hook_gas_limit)?;
    bridge::save_frontend_contract(deps.storage, deps.api, msg.frontend_contract.clone())?;
    pause::save_pauser(deps.storage, deps.api, msg.pauser.clone())?;
    match ts.clone() {
        tokenfactory::TokenStrategy::NewFungible(ref cfg) => {
            // Save config for reply handler to access initial mints
            let w = match &msg.wavs { Some(w) => Some(w.proof_of_ownership(deps.api, &c)?), None => None };
            save_headstash_cfg(
                deps.storage,
                msg.genesis_root.clone(),
                domain,
                genesis_label,
                vec![ts],
                w,
                circuit_id,
            )?;

            Ok(Response::new()
                .add_attribute("action", "instantiate")
                .add_attribute("owner", info.sender)
                .add_attribute("subdenom", cfg.subdenom.raw.clone())
                .add_attribute("distro_hash_domain", domain.as_str())
                .add_attribute("circuit_id", circuit_id.to_string())
                .add_attribute("root_id", GENESIS_ROOT_ID.to_string())
                .add_submessage(
                    // Create new denom, denom info is saved in the reply
                    SubMsg::reply_on_success(
                        msg_create_denom(
                            env.contract.address.to_string(),
                            cfg.subdenom.raw.clone(),
                        )?,
                        CREATE_DENOM_REPLY_ID,
                    ),
                ))
        }
        tokenfactory::TokenStrategy::ExistingFungible(ref denom) => {
            // check for prefunding of headstash
            if !info.funds.iter().any(|coin| coin.denom == denom.raw) {
                let balance = deps.querier.query_balance(&c, &denom.raw)?.amount;
                if balance.is_zero() {
                    return Err(StdError::msg(
                        "at least 1 token required for existing denom",
                    ));
                }
            }

            DENOM.save(deps.storage, &denom.raw)?;

            // Save config for existing tokens too
            let w = match &msg.wavs { Some(w) => Some(w.proof_of_ownership(deps.api, &c)?), None => None };
            save_headstash_cfg(
                deps.storage,
                msg.genesis_root.clone(),
                domain,
                genesis_label,
                vec![ts],
                w,
                circuit_id,
            )?;

            Ok(Response::new()
                .add_attribute("action", "instantiate")
                .add_attribute("owner", info.sender)
                .add_attribute("denom", denom.raw.clone())
                .add_attribute("distro_hash_domain", domain.as_str())
                .add_attribute("circuit_id", circuit_id.to_string())
                .add_attribute("root_id", GENESIS_ROOT_ID.to_string()))
        }
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, StdError> {
    // State-preserving: no storage is rewritten. Campaign creation binds the
    // eligibility root; a code upgrade must not copy one on afterwards. New v2
    // state (forward channels, pending forwards, HAL source) starts empty.
    let from = match cw2::get_contract_version(deps.storage) {
        Ok(v) => {
            if v.contract != CONTRACT_NAME {
                return Err(StdError::msg(format!("cannot migrate from {}", v.contract)));
            }
            if semver3(&v.version) > semver3(CONTRACT_VERSION) {
                return Err(StdError::msg(format!("cannot downgrade from {}", v.version)));
            }
            v.version
        }
        // v0.0.1 (mainnet code 105) never wrote cw2: require the headstash config instead.
        Err(_) => {
            if HEADSTASH_CFG.may_load(deps.storage)?.is_none() {
                return Err(StdError::msg("not a cw-headstash instance"));
            }
            "none".to_string()
        }
    };
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("from_version", from)
        .add_attribute("to_version", CONTRACT_VERSION))
}

fn semver3(v: &str) -> (u64, u64, u64) {
    let mut it = v.split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// ICS-20 source callbacks for `ClaimNoteForward` (refund on timeout / error ack).
#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn ibc_source_callback(
    deps: DepsMut,
    env: Env,
    msg: cosmwasm_std::IbcSourceCallbackMsg,
) -> StdResult<cosmwasm_std::IbcBasicResponse> {
    forward::source_callback(deps, env, msg)
}

/// Returns `f` through `black_box`, so the call becomes `call_indirect`. Without
/// this, wasm-opt -Os inlines every single-caller handler into `execute`, giving one
/// ~127 KB function that arm64 singlepass rejects (ImpossibleRelocation).
/// `#[inline(never)]` does not help: it only binds LLVM, not binaryen.
#[inline(always)]
fn opaque<F: Copy>(f: F) -> F {
    core::hint::black_box(f)
}


#[inline(never)]
fn pause_gate(storage: &dyn cosmwasm_std::Storage, msg: &ExecuteMsg) -> Result<(), StdError> {
    #[allow(unused_mut)]
    let mut gated = matches!(
        msg,
        ExecuteMsg::ClaimNote { .. }
            | ExecuteMsg::ClaimNoteForward { .. }
            | ExecuteMsg::RegisterHeadstash { .. }
            | ExecuteMsg::RegisterHeadstashSol { .. }
            | ExecuteMsg::RegisterNotes { .. }
            | ExecuteMsg::AcceptAnchor { .. }
    );
    #[cfg(feature = "legacy")]
    {
        gated = gated
            || matches!(
                msg,
                ExecuteMsg::ProcessHeadstash { .. }
                    | ExecuteMsg::BridgeMintNote { .. }
                    | ExecuteMsg::ApplyAcceptedBatch { .. }
                    | ExecuteMsg::ApplyPublicBatch { .. }
            );
    }
    if gated {
        pause::ensure_not_paused(storage)?;
    }
    Ok(())
}
#[inline(never)]
fn pause_dispatch(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> Result<Response, StdError> {
    match msg {
        ExecuteMsg::Pause {} => pause::pause(deps, env, info),
        ExecuteMsg::Resume {} => pause::resume(deps, env, info),
        ExecuteMsg::UpdatePauser { pauser } => pause::update_pauser(deps, info, pauser),
        _ => Err(StdError::msg("unreachable")),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, StdError> {
    // Terminal state after an emergency reclaim: nothing executes (Appendix R).
    reclaim::ensure_live(deps.storage)?;
    // Pause gate (Appendix P): claims/payouts and registration only.
    pause_gate(deps.storage, &msg)?;
    // TODO: allowlist for writing to nullifier tree
    match msg {
        m @ (ExecuteMsg::Pause {} | ExecuteMsg::Resume {} | ExecuteMsg::UpdatePauser { .. }) => {
            opaque::<fn(_, _, _, _) -> _>(pause_dispatch)(deps, env, info, m)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::ProcessHeadstash { claims } => {
            opaque::<fn(_, _, _) -> _>(crate::headstash::process_headstash)(deps, env, claims)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::SetCircuitId { circuit_id } => {
            opaque::<fn(_, _, _) -> _>(crate::headstash::set_circuit_id)(deps, info, circuit_id)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::RegisterEligibilityRoot {
            root,
            domain,
            label,
        } => opaque::<fn(_, _, _, _, _) -> _>(crate::headstash::register_eligibility_root)(deps, info, root, domain, label),
        #[cfg(feature = "legacy")]
        ExecuteMsg::Mint { to_address, amount } => {
            opaque::<fn(_, _, _, _, _) -> _>(execute_mint)(deps, env, info, to_address, amount)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::Burn {
            from_address,
            amount,
        } => opaque::<fn(_, _, _, _, _) -> _>(execute_burn)(deps, env, info, from_address, amount),
        // Private-bridge mint router (CLARITY: cw-headstash is the mint)
        #[cfg(feature = "legacy")]
        ExecuteMsg::UpdateReflection {
            pool_root,
            spent_root,
            burn_root,
            source_height,
            tip_height,
        } => opaque::<fn(_, _, _, _, _, _, _) -> _>(bridge::execute_update_reflection)(
            deps,
            info,
            pool_root,
            spent_root,
            burn_root,
            source_height,
            tip_height,
        ),
        #[cfg(feature = "legacy")]
        ExecuteMsg::SetReflectionSnapshot { snapshot } => {
            opaque::<fn(_, _, _) -> _>(bridge::execute_set_reflection_snapshot)(deps, info, snapshot)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::RegisterAsset {
            asset_id,
            local_denom,
            origin,
            status,
        } => opaque::<fn(_, _, _, _, _, _) -> _>(bridge::execute_register_asset)(deps, info, asset_id, local_denom, origin, status),
        #[cfg(feature = "legacy")]
        ExecuteMsg::SetExternalAssetRegistry { addr } => {
            opaque::<fn(_, _, _) -> _>(bridge::execute_set_external_asset_registry)(deps, info, addr)
        }
        ExecuteMsg::SetFrontendContract { addr } => {
            opaque::<fn(_, _, _) -> _>(bridge::execute_set_frontend_contract)(deps, info, addr)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::SetBridgeCfg { cfg } => opaque::<fn(_, _, _) -> _>(bridge::execute_set_bridge_cfg)(deps, info, cfg),
        #[cfg(feature = "legacy")]
        ExecuteMsg::BridgeMintNote { claim, proof } => {
            opaque::<fn(_, _, _, _, _) -> _>(bridge::execute_bridge_mint_note)(deps, env, info, claim, proof)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::BridgeEgressBurn { statement, proof } => {
            opaque::<fn(_, _, _, _, _) -> _>(egress::execute_bridge_egress_burn)(deps, env, info, statement, proof)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::SetDenomTable { amounts } => opaque::<fn(_, _, _) -> _>(epoch::set_denom_table)(deps, info, amounts),
        #[cfg(feature = "legacy")]
        ExecuteMsg::AssignPublicNotes { nullifiers } => {
            opaque::<fn(_, _, _) -> _>(epoch::assign_public_notes)(deps, info, nullifiers)
        }
        ExecuteMsg::SetEpochCampaign { campaign } => opaque::<fn(_, _, _) -> _>(epoch::set_campaign)(deps, info, campaign),
        ExecuteMsg::SweepUnclaimed {} => opaque::<fn(_, _, _) -> _>(epoch::sweep_unclaimed)(deps, env, info),
        ExecuteMsg::EmergencyReclaim {} => opaque::<fn(_, _, _) -> _>(reclaim::emergency_reclaim)(deps, env, info),
        ExecuteMsg::UpdateOwnership(action) => {
            if matches!(action, cw_ownable::Action::RenounceOwnership) {
                return Err(StdError::msg("renounce_ownership is disabled"));
            }
            let o = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)
                .map_err(|e| StdError::msg(e.to_string()))?;
            Ok(Response::new().add_attributes(o.into_attributes()))
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::StageEpochBatch { digests, rows } => {
            opaque::<fn(_, _, _, _, _) -> _>(epoch::stage_batch)(deps, &env, info, digests, rows)
        }
        #[cfg(feature = "legacy")]
        ExecuteMsg::ApplyAcceptedBatch {
            allocation_root,
            root_id,
            rows,
            vk_hash,
        } => opaque::<fn(_, _, _, _, _, _) -> _>(epoch::apply_accepted_batch)(deps, env, allocation_root, root_id, rows, vk_hash),
        #[cfg(feature = "legacy")]
        ExecuteMsg::ApplyPublicBatch { rows } => opaque::<fn(_, _, _) -> _>(epoch::apply_public_batch)(deps, env, rows),
        ExecuteMsg::RegisterHeadstash {
            key_com,
            amount,
            signature,
            eligibility_index,
            eligibility_path,
        } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            registration::assert_registration_open(env.block.height, &campaign)?;
            opaque::<fn(_, _, _, _, _, _, _) -> _>(registration::register)(
                deps,
                &campaign,
                key_com,
                amount,
                signature,
                eligibility_index,
                eligibility_path,
            )
        }
        ExecuteMsg::RegisterHeadstashSol {
            key_com,
            amount,
            pubkey,
            signature,
            eligibility_index,
            eligibility_path,
        } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            registration::assert_registration_open(env.block.height, &campaign)?;
            opaque::<fn(_, _, _, _, _, _, _, _) -> _>(registration::register_sol)(
                deps,
                &campaign,
                key_com,
                amount,
                pubkey,
                signature,
                eligibility_index,
                eligibility_path,
            )
        }
        ExecuteMsg::RegisterNotes {
            signer,
            signature,
            list,
            start,
            eligibility,
            reward_addr,
        } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            registration::assert_registration_open(env.block.height, &campaign)?;
            opaque::<fn(_, _, _, _, _, _, _, _) -> _>(notes::register_notes)(
                deps,
                &campaign,
                signer,
                signature,
                list,
                start,
                eligibility,
                reward_addr,
            )
        }
        ExecuteMsg::BindNoteCircuit { zk_id } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            opaque::<fn(_, _, _, _, _) -> _>(direct::bind_circuit)(deps, &env, info, &campaign, zk_id)
        }
        ExecuteMsg::ClaimNote { root, nullifier, amount, proof } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            opaque::<fn(_, _, _, _, _, _, _, _) -> _>(direct::claim_note)(deps, &env, info, &campaign, root, nullifier, amount, proof)
        }
        ExecuteMsg::AcceptAnchor {} => {
            let campaign = epoch::query_campaign(deps.storage)?;
            opaque::<fn(_, _, _) -> _>(registration::accept_anchor)(deps, &env, &campaign)
        }
        ExecuteMsg::ClaimNoteForward { root, nullifier, amount, proof, forward } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            opaque::<fn(_, _, _, _, _, _, _, _) -> _>(forward::claim_note_forward)(deps, &env, &campaign, root, nullifier, amount, proof, forward)
        }
        ExecuteMsg::SetForwardChannel { channel_id, timeout_seconds, enabled } => {
            opaque::<fn(_, _, _, _, _) -> _>(forward::set_forward_channel)(deps, &info, channel_id, timeout_seconds, enabled)
        }
        ExecuteMsg::ResolveForward { channel_id, sequence } => {
            opaque::<fn(_, _, _, _, _) -> _>(forward::resolve_forward)(deps, &env, &info, channel_id, sequence)
        }
        ExecuteMsg::SetHalSource { source } => opaque::<fn(_, _, _) -> _>(forward::set_hal_source)(deps, &info, source),
        // Kept in the enum (manifold builds it); disabled in the v0.0.1 surface.
        #[cfg(not(feature = "legacy"))]
        ExecuteMsg::RegisterEligibilityRoot { .. } => Err(StdError::msg(DISABLED_IN_BUILD)),
        // v0.0.1 mainnet surface: legacy/bridge/epoch-batch variants are compiled
        // out of ExecuteMsg itself (they fail to parse), keeping the entry small.
    }
}

/// Error for every ExecuteMsg variant compiled out of the v0.0.1 build.
pub const DISABLED_IN_BUILD: &str = "disabled in this build (v0.0.1)";

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    msg: epoch::HashMerchantSudoMsg,
) -> Result<Response, StdError> {
    reclaim::ensure_live(deps.storage)?;
    pause::ensure_not_paused(deps.storage)?;
    #[cfg(feature = "legacy")]
    {
        epoch::sudo(deps, &env, msg)
    }
    #[cfg(not(feature = "legacy"))]
    {
        let _ = (deps, env, msg);
        Err(StdError::msg(DISABLED_IN_BUILD))
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::Nullifer { null } => Ok(to_json_binary(
            &NULLIFIERS.may_load(deps.storage, null)?.is_some(),
        )?),
        QueryMsg::Nullifiers { start_after, limit } => {
            headstash::query_nullifiers(deps, start_after, limit)
        }
        QueryMsg::DistroConfig {} => headstash::query_distro_config(deps),
        QueryMsg::WavsStatus {} => {
            let w = HEADSTASH_CFG.load(deps.storage)?.w;
            to_json_binary(&WavsStatusResponse { enabled: w.is_some(), operator_set: w })
        }
        QueryMsg::EligibilityRoot { root_id } => headstash::query_eligibility_root(deps, root_id),
        QueryMsg::EligibilityRoots { start_after, limit } => {
            headstash::query_eligibility_roots(deps, start_after, limit)
        }
        QueryMsg::ReflectionTip {} => bridge::query_reflection_tip(deps),
        QueryMsg::IsBridgeMinted { nullifier } => bridge::query_is_bridge_minted(deps, nullifier),
        QueryMsg::IsEgressSpent { nullifier } => egress::query_is_egress_spent(deps, nullifier),
        QueryMsg::BridgeAsset { asset_id } => bridge::query_asset(deps, asset_id),
        QueryMsg::BridgeConfig {} => bridge::query_bridge_cfg(deps),
        QueryMsg::ExternalAssetRegistry {} => bridge::query_external_asset_registry(deps),
        QueryMsg::FrontendContract {} => bridge::query_frontend_contract(deps),
        QueryMsg::EpochRoot { algo } => to_json_binary(&epoch::query_root(deps.storage, algo)?),
        QueryMsg::ForwardChannels {} => to_json_binary(&forward::query_channels(deps.storage)?),
        QueryMsg::HalSource {} => to_json_binary(&forward::HAL_SOURCE.may_load(deps.storage)?),
        QueryMsg::PendingForward { channel_id, sequence } => {
            to_json_binary(&forward::PENDING.may_load(deps.storage, (channel_id.as_str(), sequence))?)
        }
        QueryMsg::PendingForwards { start_after, limit } => to_json_binary(&forward::query_pending(deps.storage, start_after, limit)?),
        QueryMsg::ForwardDestination { forward } => {
            to_json_binary(&forward::forward_destination(&env.block.chain_id, env.contract.address.as_str(), &forward))
        }
        QueryMsg::CheckNoteForward { root, nullifier, amount, proof, forward } => {
            let campaign = epoch::query_campaign(deps.storage)?;
            let (dest, ..) = forward::check_target(deps, &env, &forward)?;
            direct::check_claim(deps, &env, &campaign, &dest, &root, &nullifier, amount, &proof)?;
            to_json_binary(&true)
        }
        QueryMsg::EpochCampaign {} => to_json_binary(&epoch::query_campaign(deps.storage)?),
        QueryMsg::RegistrationTree {} => {
            let campaign = epoch::query_campaign(deps.storage)?;
            let cfg = campaign
                .registration
                .ok_or_else(|| StdError::msg("registration"))?;
            to_json_binary(&registration::query_tree(deps.storage, &cfg)?)
        }
        QueryMsg::RegistrationAnchors { start_after, limit } => {
            to_json_binary(&registration::query_anchors(deps.storage, start_after, limit)?)
        }
        QueryMsg::RegistrationPath { key_com, anchor } => {
            to_json_binary(&registration::query_path(
                deps.storage,
                key_com.as_slice(),
                anchor.as_slice(),
            )?)
        }
        QueryMsg::CheckRegisterNotes { signer, signature, list, start, eligibility, reward_addr } => {
            reclaim::ensure_live(deps.storage)?;
            pause::ensure_not_paused(deps.storage)?;
            let campaign = epoch::query_campaign(deps.storage)?;
            registration::assert_registration_open(env.block.height, &campaign)?;
            notes::check_register_notes(deps, &campaign, &signer, signature.as_ref(), &list, start, &eligibility, reward_addr.as_deref())?;
            to_json_binary(&true)
        }
        QueryMsg::CheckNoteClaim { dest, root, nullifier, amount, proof } => {
            reclaim::ensure_live(deps.storage)?;
            pause::ensure_not_paused(deps.storage)?;
            let campaign = epoch::query_campaign(deps.storage)?;
            deps.api.addr_validate(&dest)?;
            direct::check_claim(deps, &env, &campaign, &dest, &root, &nullifier, amount, &proof)?;
            to_json_binary(&true)
        }
        QueryMsg::ReclaimStatus {} => to_json_binary(&reclaim::query_status(deps.storage)?),
        QueryMsg::PauseStatus {} => to_json_binary(&pause::query_status(deps.storage)?),
        QueryMsg::DirectClaim {} => to_json_binary(&direct::query_info(deps.storage, &env)?),
        QueryMsg::NoteList { identity } => {
            to_json_binary(&notes::query_note_list(deps.storage, &identity)?)
        }
        QueryMsg::Registered { identity } => {
            to_json_binary(&registration::is_registered(deps.storage, &identity)?)
        }
    }
}

pub fn execute_mint(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    to_address: String,
    amount: Uint128,
) -> Result<Response, StdError> {
    // Only allow current contract owner to mint
    cw_ownable::assert_owner(deps.storage, &info.sender)?;

    // Validate that to_address is a valid address
    deps.api.addr_validate(&to_address)?;

    // Don't allow minting of 0 coins
    if amount.is_zero() {
        return Err(StdError::msg("Zero amount"));
    }

    // Get token denom from contract
    let denom = DENOM.load(deps.storage)?;

    // Create tokenfactory MsgMint which mints coins to the contract address
    let mint_tokens_msg = msg_mint(env.contract.address.to_string(), amount, denom.clone())?;

    // Send newly minted coins from contract to designated recipient
    let send_tokens_msg = BankMsg::Send {
        to_address: to_address.clone(),
        amount: coins(amount.u128(), denom),
    };

    Ok(Response::new()
        .add_message(mint_tokens_msg)
        .add_message(send_tokens_msg)
        .add_attribute("action", "mint")
        .add_attribute("to", to_address)
        .add_attribute("amount", amount))
}

pub fn execute_burn(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    from_address: String,
    amount: Uint128,
) -> Result<Response, StdError> {
    // Only allow current contract owner to burn
    cw_ownable::assert_owner(deps.storage, &info.sender)?;

    // Don't allow burning of 0 coins
    if amount.is_zero() {
        return Err(StdError::msg("Zero amount"));
    }

    // Get token denom from contract config
    let denom = DENOM.load(deps.storage)?;

    // Create tokenfactory MsgBurn which burns coins from the contract address
    // NOTE: this requires the contract to own the tokens already
    let from_addr = deps.api.addr_validate(&from_address)?;
    let burn_tokens_msg = msg_burn(
        env.contract.address.to_string(),
        amount,
        denom,
        from_addr.to_string(),
    )?;

    Ok(Response::new()
        .add_message(burn_tokens_msg)
        .add_attribute("action", "burn")
        .add_attribute("burner", info.sender)
        .add_attribute("burn_from_address", from_addr.to_string())
        .add_attribute("amount", amount))
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> Result<Response, StdError> {
    match msg.id {
        forward::FORWARD_REPLY_ID => forward::reply(deps, msg),
        // reply_on_error only: a failing (or out-of-gas) hook never blocks registration.
        notes::HOOK_REPLY_ID => Ok(Response::new()
            .add_attribute("action", "registration_hook_failed")
            .add_attribute("hook_error", msg.result.unwrap_err())),
        CREATE_DENOM_REPLY_ID => {
            // Extract the new_token_denom from the reply data
            // In CosmWasm v3, the reply data contains the response from the submessage
            let sub = msg.result.into_result().map_err(|e| StdError::msg(e.to_string()))?;
            // msg_responses[0].value is the protobuf MsgCreateDenomResponse. A missing
            // response is an error, not an index panic.
            let response_data = &sub
                .msg_responses
                .first()
                .ok_or_else(|| StdError::msg("MsgCreateDenom: no msg response"))?
                .value;
            let new_token_denom = tf_proto::create_denom_response_denom(response_data.as_slice())?;

            // Claims mint and send `TokenStrategy::denom()` (factory/<contract>/<subdenom>),
            // so the chain-reported denom must be exactly that; otherwise fail instantiate
            // instead of saving a DENOM that payouts never use.
            let cfg = HEADSTASH_CFG.may_load(deps.storage)?;
            if let Some(expected) = cfg.as_ref().and_then(|c| c.ts.first()).map(|t| t.denom(&env.contract.address)) {
                if expected != new_token_denom {
                    return Err(StdError::msg("MsgCreateDenomResponse: unexpected denom"));
                }
            }

            DENOM.save(deps.storage, &new_token_denom)?;

            // Check if we need to do initial minting
            if let Some(config) = cfg {
                if let Some(token_strategy) = config.ts.first() {
                    let initial_mints = token_strategy.get_initial_mints();
                    if !initial_mints.is_empty() {
                        let mut messages = vec![];
                        let initial_mint_count = initial_mints.len();
                        for mint in initial_mints {
                            let mint_msgs = token_strategy.mint_tokens(
                                &env.contract.address,
                                mint.amount,
                                &mint.to_address,
                            )?;
                            messages.extend(mint_msgs);
                        }
                        return Ok(Response::new()
                            .add_attribute("denom", new_token_denom)
                            .add_attribute("initial_mints", initial_mint_count.to_string())
                            .add_messages(messages));
                    }
                }
            }

            Ok(Response::new().add_attribute("denom", new_token_denom))
        }
        _ => Err(StdError::msg("Unknown reply id")),
    }
}

// Smart-account authenticator impl (TerpAccountTrait: on_auth_added/removed/request/track/confirm)
// removed: it was unreachable (sudo decodes HashMerchantSudoMsg only) and on_auth_removed cleared
// config, nullifiers and eligibility roots. See spec Appendix W.

/// Map nonce -> indexList of wav operator bls12-381
/// we want to be able to query:
/// - the list of operators given an array of their positions in the index
/// - the list of operators at a given nonce
/// Stores: nonce -> list of operator indices (e.g., positions in validator set)

// -  merkle tree must be fixed length, meaning once buffer is full from specific tree,
// - we must create new one and be able to have users reference the head/where existing is to prevent expensive use

/// Generic function for paginating a list of (K, V) pairs in a
/// CosmWasm Map.
pub fn paginate_map<'a, 'b, K, V, R: 'static>(
    deps: Deps,
    map: &Map<K, V>,
    start_after: Option<K>,
    limit: Option<u32>,
    order: Order,
) -> StdResult<Vec<(R, V)>>
where
    K: Bounder<'a> + KeyDeserialize<Output = R> + 'b,
    V: serde::de::DeserializeOwned + serde::Serialize,
{
    let (range_min, range_max) = match order {
        Order::Ascending => (start_after.map(Bound::exclusive), None),
        Order::Descending => (None, start_after.map(Bound::exclusive)),
    };

    let items = map.range(deps.storage, range_min, range_max, order);
    match limit {
        Some(limit) => Ok(items
            .take(limit.try_into().unwrap())
            .collect::<StdResult<_>>()?),
        None => Ok(items.collect::<StdResult<_>>()?),
    }
}

#[cfg(test)]
mod instantiate_tests {
    use crate::tokenfactory::*;

    use super::*;
    use crate::tokenfactory::{DenomUnit, Metadata};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Uint128;

    #[test]
    fn minimal_test() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = message_info(&deps.api.addr_make("creator"), &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
                .unwrap(),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject {
                    proof: derive_nd(&cosmwasm_std::testing::MockApi::default(), "test"),
                    raw: "test".into(),
                },
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(3)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        let res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        // CreateDenom submessage only; initial mints (if any) land in reply.
        assert_eq!(res.messages.len(), 1);
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "distro_hash_domain")
                .map(|a| a.value.as_str()),
            Some("poseidon-v1")
        );
    }

    #[test]
    fn update_ownership_two_step_and_no_renounce() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let owner = deps.api.addr_make("owner");
        let dao = deps.api.addr_make("dao");
        let eve = deps.api.addr_make("eve");
        let api = deps.api;
        cw_ownable::initialize_owner(deps.as_mut().storage, &api, Some(owner.as_str())).unwrap();
        let propose = ExecuteMsg::UpdateOwnership(cw_ownable::Action::TransferOwnership {
            new_owner: dao.to_string(),
            expiry: None,
        });
        assert!(execute(deps.as_mut(), env.clone(), message_info(&eve, &[]), propose.clone()).is_err());
        execute(deps.as_mut(), env.clone(), message_info(&owner, &[]), propose).unwrap();
        let accept = ExecuteMsg::UpdateOwnership(cw_ownable::Action::AcceptOwnership);
        assert!(execute(deps.as_mut(), env.clone(), message_info(&eve, &[]), accept.clone()).is_err());
        execute(deps.as_mut(), env.clone(), message_info(&dao, &[]), accept).unwrap();
        let o = cw_ownable::get_ownership(deps.as_ref().storage).unwrap();
        assert_eq!(o.owner, Some(dao.clone()));
        assert!(cw_ownable::assert_owner(deps.as_ref().storage, &owner).is_err());
        let renounce = ExecuteMsg::UpdateOwnership(cw_ownable::Action::RenounceOwnership);
        let err = execute(deps.as_mut(), env, message_info(&dao, &[]), renounce).unwrap_err();
        assert!(err.to_string().contains("renounce_ownership is disabled"));
        assert_eq!(cw_ownable::get_ownership(deps.as_ref().storage).unwrap().owner, Some(dao));
    }

    // Helper: real BLS PoP for instantiate (see wavs::generate_test_wavs_proof).
    fn valid_wavs_proof(total_operators: usize) -> WavsProofOfOwnership {
        wavs::generate_test_wavs_proof(total_operators)
    }
    fn mock_metadata() -> Metadata {
        Metadata {
            description: Some("Test Token".into()),
            denom_units: vec![
                DenomUnit {
                    denom: "utest".into(),
                    exponent: 0,
                    aliases: vec![],
                },
                DenomUnit {
                    denom: "TEST".into(),
                    exponent: 6,
                    aliases: vec![],
                },
            ],
            base: Some("utest".into()),
            display: Some("TEST".into()),
            name: Some("Test Token".into()),
            symbol: Some("TEST".into()),
        }
    }

    #[test]
    fn instantiate_new_fungible_success() {
        let mut deps = mock_dependencies();
        let creator = deps.api.addr_make("creator");
        let alice = deps.api.addr_make("alice");
        let bob = deps.api.addr_make("bob");
        let env = mock_env();
        let info = message_info(&creator, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
                .unwrap(),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject {
                    proof: derive_nd(&cosmwasm_std::testing::MockApi::default(), &"test"),
                    raw: "test".into(),
                },
                metadata: mock_metadata(),
                initial_mint: Some(vec![
                    InitialMint {
                        to_address: alice.to_string(),
                        amount: Uint128::new(1000),
                    },
                    InitialMint {
                        to_address: bob.to_string(),
                        amount: Uint128::new(500),
                    },
                ]),
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(3)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        let res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // Should have 1 submessage for CreateDenom (initial minting happens in reply)
        assert_eq!(res.messages.len(), 1);

        // Check that it's a SubMsg with reply_on_success
        match &res.messages[0] {
            SubMsg { msg, reply_on, .. } => {
                match msg {
                    CosmosMsg::Stargate { type_url, value: _ } => {
                        // This should be the CreateDenom message
                        assert!(type_url.contains("tokenfactory"));
                    }
                    _ => panic!("Expected Stargate message for CreateDenom"),
                }
                assert_eq!(reply_on, &cosmwasm_std::ReplyOn::Success);
            }
            _ => panic!("Expected SubMsg"),
        }

        // Config should be saved for new fungible tokens
        let cfg = HEADSTASH_CFG.load(&deps.storage).unwrap();
        assert_eq!(cfg.ts.len(), 1);
        assert!(matches!(cfg.ts[0], TokenStrategy::NewFungible(_)));
    }

    #[test]
    fn instantiate_existing_fungible_with_funds_success() {
        let mut deps = mock_dependencies();
        let creator = deps.api.addr_make("creator");
        let _contract = deps.api.addr_make("contract");

        let env = mock_env();
        let info = message_info(&creator, &[Coin::new(Uint128::new(1000), "existing_token")]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from([0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                &cosmwasm_std::testing::MockApi::default(),
                "existing_token".to_string(),
            )),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        let res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // ExistingFungible does not enqueue CreateDenom.
        assert_eq!(res.messages.len(), 0);

        // Config should be saved for existing fungible tokens
        let cfg = HEADSTASH_CFG.load(&deps.storage).unwrap();
        assert_eq!(cfg.ts.len(), 1);
        assert!(matches!(cfg.ts[0], TokenStrategy::ExistingFungible(_)));
        assert_eq!(cfg.distro_hash_domain, DistroHashDomain::PoseidonV1);
    }

    #[test]
    fn instantiate_existing_fungible_no_funds_fails() {
        let mut deps = mock_dependencies();
        let creator = deps.api.addr_make("creator");

        // Balance = 0 for "existing_token"
        let env = mock_env();
        let info = message_info(&creator, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from([0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                &cosmwasm_std::testing::MockApi::default(),
                "test".to_string(),
            )),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            err.to_string(),
            "kind: Other, error: at least 1 token required for existing denom"
        );
    }

    #[test]
    fn instantiate_invalid_operator_count_fails() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");

        let info = message_info(&creator, &[]);

        let wavs = valid_wavs_proof(3);
        let mut invalid_wavs = wavs.clone();
        invalid_wavs.msg.total_operators = 5; // mismatch

        let msg = InstantiateMsg {
            genesis_root: Binary::from([0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(invalid_wavs),

            circuit_id: None,

            registration_hook: None,

            registration_hook_gas_limit: None,

            frontend_contract: None,

            pauser: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();

        assert!(err
            .to_string()
            .contains("invalid amount of operators defined"));
    }

    #[test]
    fn instantiate_invalid_proof_of_ownership_fails() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let creator = deps.api.addr_make("creator");

        let info = message_info(&creator, &[]);

        let mut wavs = valid_wavs_proof(1);
        wavs.poos[0].poo = "deadbeef".to_string(); // corrupt PoO

        let msg = InstantiateMsg {
            genesis_root: Binary::from([0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(wavs),

            circuit_id: None,

            registration_hook: None,

            registration_hook_gas_limit: None,

            frontend_contract: None,

            pauser: None,
        };

        let _err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
        // println!("{:#?}", err);
        // assert!(err.to_string().contains("proof of ownership failed"));
    }

    #[test]
    fn instantiate_stores_genesis_root_and_config() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = deps.api.addr_make("sender");

        let info = message_info(&sender, &[]);

        let genesis_root = Binary::from(vec![0xbb; 32]);

        let msg = InstantiateMsg {
            genesis_root: genesis_root.clone(),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        instantiate(deps.as_mut(), env, info, msg).unwrap();

        let stored_root = GENESIS_TREE_ROOT.load(&deps.storage).unwrap();
        assert_eq!(stored_root, genesis_root);

        let cfg = HEADSTASH_CFG.load(&deps.storage).unwrap();
        assert_eq!(cfg.gr, genesis_root);
        assert_eq!(cfg.distro_hash_domain, DistroHashDomain::PoseidonV1);
        assert_eq!(cfg.ts.len(), 1);
        assert!(cfg.w.as_ref().unwrap().msg.nonce == 0);

        let elig = distro::require_root(&deps.storage, GENESIS_ROOT_ID).unwrap();
        assert_eq!(elig.root, genesis_root);
        assert_eq!(elig.domain, DistroHashDomain::PoseidonV1);
    }

    fn wavs_msg(wavs: Option<WavsProofOfOwnership>) -> InstantiateMsg {
        InstantiateMsg {
            genesis_root: Binary::from(vec![0xbb; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(&cosmwasm_std::testing::MockApi::default(), "test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs,
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        }
    }

    #[test]
    fn instantiate_wavs_none_disables_wavs() {
        let mut deps = mock_dependencies();
        let sender = deps.api.addr_make("sender");
        instantiate(deps.as_mut(), mock_env(), message_info(&sender, &[]), wavs_msg(None)).unwrap();
        assert!(HEADSTASH_CFG.load(&deps.storage).unwrap().w.is_none());
        let st: WavsStatusResponse = cosmwasm_std::from_json(query(deps.as_ref(), mock_env(), QueryMsg::WavsStatus {}).unwrap()).unwrap();
        assert_eq!(st, WavsStatusResponse { enabled: false, operator_set: None });
        // fail closed
        assert!(wavs_operator_set(&deps.storage).unwrap_err().to_string().contains("wavs disabled"));
        // stored config JSON carries no operator set
        let raw = deps.storage.get(b"headstash_params").unwrap();
        assert!(String::from_utf8(raw).unwrap().contains("\"w\":null"));
    }

    #[test]
    fn instantiate_wavs_some_valid_is_enabled() {
        let mut deps = mock_dependencies();
        let sender = deps.api.addr_make("sender");
        instantiate(deps.as_mut(), mock_env(), message_info(&sender, &[]), wavs_msg(Some(valid_wavs_proof(2)))).unwrap();
        let st: WavsStatusResponse = cosmwasm_std::from_json(query(deps.as_ref(), mock_env(), QueryMsg::WavsStatus {}).unwrap()).unwrap();
        assert!(st.enabled);
        assert_eq!(st.operator_set.unwrap().keys.len(), 2);
        assert_eq!(wavs_operator_set(&deps.storage).unwrap().msg.total_operators, 2);
    }

    #[test]
    fn instantiate_wavs_some_invalid_still_rejected() {
        for bad in 0..3 {
            let mut deps = mock_dependencies();
            let sender = deps.api.addr_make("sender");
            let mut w = valid_wavs_proof(1);
            match bad {
                0 => w.poos[0].poo = hex::encode(cosmwasm_std::BLS12_381_G2_GENERATOR), // valid point, wrong PoP
                1 => w.msg.threshold = 0,
                _ => w.poos.clear(),
            }
            assert!(instantiate(deps.as_mut(), mock_env(), message_info(&sender, &[]), wavs_msg(Some(w))).is_err(), "case {bad}");
            assert!(HEADSTASH_CFG.may_load(&deps.storage).unwrap().is_none());
        }
    }

    #[test]
    fn wavs_field_absent_from_json_means_disabled() {
        let mut v = cosmwasm_std::to_json_vec(&wavs_msg(None)).unwrap();
        let s = String::from_utf8(v.clone()).unwrap().replace(",\"wavs\":null", "");
        assert!(!s.contains("wavs"));
        v = s.into_bytes();
        let m: InstantiateMsg = cosmwasm_std::from_json(&v).unwrap();
        assert!(m.wavs.is_none());
    }

    #[test]
    fn legacy_cfg_with_operator_set_still_decodes() {
        // configs stored by the previous code (non-optional w) decode as Some
        let w = valid_wavs_proof(1).proof_of_ownership(&cosmwasm_std::testing::MockApi::default(), &Addr::unchecked("c")).unwrap();
        let cfg = HeadstashCfg { cid: 0, gr: Binary::from(vec![1; 32]), distro_hash_domain: Default::default(), ts: vec![], w: Some(w.clone()) };
        let back: HeadstashCfg = cosmwasm_std::from_json(cosmwasm_std::to_json_vec(&cfg).unwrap()).unwrap();
        assert_eq!(back.w, Some(w));
    }

    #[test]
    fn sudo_refuses_smart_account_authenticator_messages() {
        // only HashMerchantSudoMsg decodes; authenticator hooks (incl. the removed state-wiping
        // on_authenticator_removed) cannot be dispatched
        for m in [
            r#"{"on_authenticator_added":{"account":"a","authenticator_id":"1","authenticator_params":null}}"#,
            r#"{"on_authenticator_removed":{"account":"a","authenticator_id":"1","authenticator_params":null}}"#,
            r#"{"authenticate":{}}"#, r#"{"track":{}}"#, r#"{"confirm_execution":{}}"#,
        ] {
            assert!(cosmwasm_std::from_json::<epoch::HashMerchantSudoMsg>(m.as_bytes()).is_err(), "{m}");
        }
    }

    #[test]
    fn instantiate_rejects_empty_root() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = deps.api.addr_make("sender");
        let info = message_info(&sender, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::default(),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[cfg(feature = "legacy")]
    #[test]
    fn register_second_root_additive_owner() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = deps.api.addr_make("sender");
        let info = message_info(&sender, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0x11; 32]),
            distro_hash_domain: DistroHashDomain::PoseidonV1,
            genesis_label: Some("drop-0".into()),
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

        let res = execute(
            deps.as_mut(),
            env.clone(),
            info,
            ExecuteMsg::RegisterEligibilityRoot {
                root: Binary::from(vec![0x22; 32]),
                domain: None,
                label: Some("drop-1".into()),
            },
        )
        .unwrap();
        assert_eq!(
            res.attributes
                .iter()
                .find(|a| a.key == "root_id")
                .map(|a| a.value.as_str()),
            Some("1")
        );

        let e1 = distro::require_root(&deps.storage, 1).unwrap();
        assert_eq!(e1.root, Binary::from(vec![0x22; 32]));
        assert_eq!(e1.domain, DistroHashDomain::PoseidonV1);
    }

    #[cfg(feature = "legacy")]
    #[test]
    fn set_circuit_id_and_claim_mock_verify_owner() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = deps.api.addr_make("sender");
        let info = message_info(&sender, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0xab; 32]),
            distro_hash_domain: DistroHashDomain::PoseidonV1,
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: Some(0),
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

        execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::SetCircuitId { circuit_id: 7 },
        )
        .unwrap();

        let cfg = HEADSTASH_CFG.load(&deps.storage).unwrap();
        assert_eq!(cfg.cid, 7);

        assert_eq!(cfg.distro_hash_domain, DistroHashDomain::PoseidonV1);
    }

    #[test]
    fn claim_unregistered_root_rejected_without_proof() {
        // Direct state-machine path used by process_headstash before ZK verify.
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = deps.api.addr_make("sender");
        let info = message_info(&sender, &[]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0x11; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject::new(
                    &cosmwasm_std::testing::MockApi::default(),
                    "test".to_string(),
                ),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: Some(valid_wavs_proof(1)),
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        let claim = HeadstashNote {
            i: HeadstashInstances {
                anchor: Binary::from(vec![0x11; 32]),
                nd: Binary::from(vec![0u8; 32]),
                v: 1,
                nf: Binary::from(vec![0xAB; 32]),
                recp: Binary::from(vec![0u8; 32]),
                cmx: Binary::from(vec![0u8; 32]),
                e: Binary::from(vec![0u8; 32]),
            },
            p: Binary::default(),
            rr: Binary::from(vec![0u8; 32]),
            root_id: 99,
        };

        let err = process_headstash(deps.as_mut(), env, vec![claim]).unwrap_err();
        assert!(err.to_string().contains("unregistered eligibility root_id"));
    }

    /// CreateDenom reply hardening (TF-DENOM-SECURITY-REVIEW F-1/F-2): an empty
    /// msg_responses or a chain-reported denom other than factory/<contract>/<subdenom>
    /// fails instantiate instead of panicking or saving a DENOM payouts never use.
    #[test]
    #[allow(deprecated)]
    fn create_denom_reply_rejects_missing_or_unexpected_denom() {
        use cosmwasm_std::{MsgResponse, SubMsgResponse, SubMsgResult};
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = message_info(&deps.api.addr_make("creator"), &[]);
        let msg = InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::NewFungible(NewTokenConfig {
                subdenom: HeadstashTokenObject {
                    proof: derive_nd(&cosmwasm_std::testing::MockApi::default(), "test"),
                    raw: "test".into(),
                },
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: None,
            circuit_id: None,
            registration_hook: None,
            registration_hook_gas_limit: None,
            frontend_contract: None,
            pauser: None,
        };
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let mk = |responses: Vec<MsgResponse>| Reply {
            id: CREATE_DENOM_REPLY_ID,
            payload: Binary::default(),
            gas_used: 0,
            result: SubMsgResult::Ok(SubMsgResponse { events: vec![], data: None, msg_responses: responses }),
        };
        let resp = |denom: &str| {
            let mut v = vec![0x0a, denom.len() as u8];
            v.extend_from_slice(denom.as_bytes());
            MsgResponse { type_url: "/osmosis.tokenfactory.v1beta1.MsgCreateDenomResponse".into(), value: Binary::from(v) }
        };
        let err = reply(deps.as_mut(), env.clone(), mk(vec![])).unwrap_err();
        assert!(err.to_string().contains("no msg response"), "{err}");
        let other = format!("factory/{}/other", env.contract.address);
        let err = reply(deps.as_mut(), env.clone(), mk(vec![resp(&other)])).unwrap_err();
        assert!(err.to_string().contains("unexpected denom"), "{err}");
        assert!(DENOM.may_load(&deps.storage).unwrap().is_none());
        let good = format!("factory/{}/test", env.contract.address);
        reply(deps.as_mut(), env.clone(), mk(vec![resp(&good)])).unwrap();
        assert_eq!(DENOM.load(&deps.storage).unwrap(), good);
    }
}
