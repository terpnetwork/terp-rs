pub mod bridge;
pub mod distro;
pub mod egress;
pub mod epoch;
pub mod headstash;
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

use ark_ff::Zero;
use cosmwasm_schema::{QueryResponses, cw_serde, serde};
use cosmwasm_std::{
    Addr, BLS12_381_G1_GENERATOR as G1, BLS12_381_G2_GENERATOR as G2, BankMsg, Binary, Coin,
    CosmosMsg, Deps, DepsMut, Env, MessageInfo, Order, Reply, Response, StdError, StdResult,
    Storage, SubMsg, Uint128, coins, from_json, to_json_binary,
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

// a historical set of wavs operator keys after rotated. once rotated we list of old keys iwith block height changed as key.
pub const WAVS_OPERATORS: Map<String, Vec<String>> = Map::new("wavs_operators");
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

// Helper functions to create Stargate messages for tokenfactory operations
fn msg_create_denom(sender: String, subdenom: String) -> StdResult<CosmosMsg> {
    #[derive(serde::Serialize)]
    struct MsgCreateDenom {
        sender: String,
        subdenom: String,
    }

    let msg = MsgCreateDenom { sender, subdenom };
    let value = to_json_binary(&msg)?;

    Ok(CosmosMsg::Stargate {
        type_url: MSG_CREATE_DENOM_TYPE_URL.to_string(),
        value,
    })
}

pub(crate) fn msg_mint(sender: String, amount: Uint128, denom: String) -> StdResult<CosmosMsg> {
    #[derive(serde::Serialize)]
    struct Coin {
        denom: String,
        amount: String,
    }

    #[derive(serde::Serialize)]
    struct MsgMint {
        sender: String,
        amount: Coin,
        mint_to_address: String,
    }

    let coin = Coin {
        denom,
        amount: amount.to_string(),
    };

    let msg = MsgMint {
        sender: sender.clone(),
        amount: coin,
        mint_to_address: sender.clone(), // Mint to sender, then we'll send via BankMsg
    };

    let value = to_json_binary(&msg)?;

    Ok(CosmosMsg::Stargate {
        type_url: MSG_MINT_TYPE_URL.to_string(),
        value,
    })
}

fn msg_burn(
    sender: String,
    amount: Uint128,
    denom: String,
    burn_from_address: String,
) -> StdResult<CosmosMsg> {
    #[derive(serde::Serialize)]
    struct Coin {
        denom: String,
        amount: String,
    }

    #[derive(serde::Serialize)]
    struct MsgBurn {
        sender: String,
        amount: Coin,
        burn_from_address: String,
    }

    let coin = Coin {
        denom,
        amount: amount.to_string(),
    };

    let msg = MsgBurn {
        sender,
        amount: coin,
        burn_from_address,
    };

    let value = to_json_binary(&msg)?;

    Ok(CosmosMsg::Stargate {
        type_url: MSG_BURN_TYPE_URL.to_string(),
        value,
    })
}

fn save_headstash_cfg(
    storage: &mut dyn Storage,
    genesis_root: Binary,
    distro_hash_domain: DistroHashDomain,
    genesis_label: Option<String>,
    ts: Vec<TokenStrategy>,
    w: WavsOperatorSet,
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
    // set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(info.sender.as_str()))?;
    msg.wavs.verify()?;
    msg.token_strategy.validate()?;
    let ts = msg.token_strategy;
    let c = env.contract.address.clone();
    let domain = msg.distro_hash_domain;
    let genesis_label = msg.genesis_label;
    let circuit_id = msg.circuit_id.unwrap_or(0);
    match ts.clone() {
        tokenfactory::TokenStrategy::NewFungible(ref cfg) => {
            // Save config for reply handler to access initial mints
            let w = msg.wavs.proof_of_ownership(deps.api, &c)?;
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
            let w = msg.wavs.proof_of_ownership(deps.api, &c)?;
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
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, StdError> {
    // TODO: allowlist for writing to nullifier tree
    match msg {
        ExecuteMsg::ProcessHeadstash { claims } => {
            crate::headstash::process_headstash(deps, env, claims)
        }
        ExecuteMsg::SetCircuitId { circuit_id } => {
            crate::headstash::set_circuit_id(deps, info, circuit_id)
        }
        ExecuteMsg::RegisterEligibilityRoot {
            root,
            domain,
            label,
        } => crate::headstash::register_eligibility_root(deps, info, root, domain, label),
        ExecuteMsg::Mint { to_address, amount } => {
            execute_mint(deps, env, info, to_address, amount)
        }
        ExecuteMsg::Burn {
            from_address,
            amount,
        } => execute_burn(deps, env, info, from_address, amount),
        // Private-bridge mint router (CLARITY: cw-headstash is the mint)
        ExecuteMsg::UpdateReflection {
            pool_root,
            spent_root,
            burn_root,
            source_height,
            tip_height,
        } => bridge::execute_update_reflection(
            deps,
            info,
            pool_root,
            spent_root,
            burn_root,
            source_height,
            tip_height,
        ),
        ExecuteMsg::SetReflectionSnapshot { snapshot } => {
            bridge::execute_set_reflection_snapshot(deps, info, snapshot)
        }
        ExecuteMsg::RegisterAsset {
            asset_id,
            local_denom,
            origin,
            status,
        } => bridge::execute_register_asset(deps, info, asset_id, local_denom, origin, status),
        ExecuteMsg::SetExternalAssetRegistry { addr } => {
            bridge::execute_set_external_asset_registry(deps, info, addr)
        }
        ExecuteMsg::SetBridgeCfg { cfg } => bridge::execute_set_bridge_cfg(deps, info, cfg),
        ExecuteMsg::BridgeMintNote { claim, proof } => {
            bridge::execute_bridge_mint_note(deps, env, info, claim, proof)
        }
        ExecuteMsg::BridgeEgressBurn { statement, proof } => {
            egress::execute_bridge_egress_burn(deps, env, info, statement, proof)
        }
        ExecuteMsg::SetDenomTable { amounts } => epoch::set_denom_table(deps, info, amounts),
        ExecuteMsg::AssignPublicNotes { nullifiers } => {
            epoch::assign_public_notes(deps, info, nullifiers)
        }
        ExecuteMsg::ApplyAcceptedBatch {
            allocation_root,
            root_id,
            rows,
        } => epoch::apply_accepted_batch(deps, env, allocation_root, root_id, rows),
        ExecuteMsg::ApplyPublicBatch { rows } => epoch::apply_public_batch(deps, env, rows),
    }
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn sudo(
    deps: DepsMut,
    _env: Env,
    msg: epoch::HashMerchantSudoMsg,
) -> Result<Response, StdError> {
    epoch::sudo(deps, msg)
}

#[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::Nullifer { null } => Ok(to_json_binary(
            &NULLIFIERS.may_load(deps.storage, null)?.is_some(),
        )?),
        QueryMsg::Nullifiers { start_after, limit } => {
            headstash::query_nullifiers(deps, start_after, limit)
        }
        QueryMsg::DistroConfig {} => headstash::query_distro_config(deps),
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
        QueryMsg::EpochRoot { algo } => to_json_binary(&epoch::query_root(deps.storage, algo)?),
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
        CREATE_DENOM_REPLY_ID => {
            // Extract the new_token_denom from the reply data
            // In CosmWasm v3, the reply data contains the response from the submessage
            let response_data = &msg
                .result
                .into_result()
                .map_err(|e| StdError::msg(e.to_string()))?
                .msg_responses[0]
                .value;

            #[derive(serde::Deserialize)]
            struct MsgCreateDenomResponse {
                new_token_denom: String,
            }

            let response: MsgCreateDenomResponse = cosmwasm_std::from_json(&response_data)?;
            let new_token_denom = response.new_token_denom;

            DENOM.save(deps.storage, &new_token_denom)?;

            // Check if we need to do initial minting
            let cfg = HEADSTASH_CFG.may_load(deps.storage)?;
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

impl terp_auth::TerpAccountTrait for CwHeadstash {
    type InstantiateMsg = InstantiateMsg;
    type ExecuteMsg = ExecuteMsg;
    type QueryMsg = QueryMsg;
    type SudoMsg = terp_auth::AuthSudoMsg;
    type ContractError = StdError;
    type AuthMethodStructs = Binary;
    type AuthProcessResult = Result<Response, Self::ContractError>;

    fn process_sudo_auth(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &Self::SudoMsg,
    ) -> Self::AuthProcessResult {
        match req {
            terp_auth::AuthSudoMsg::OnAuthAdded(req) => Self::on_auth_added(deps, env, req),
            terp_auth::AuthSudoMsg::OnAuthRemoved(req) => Self::on_auth_removed(deps, env, req),
            terp_auth::AuthSudoMsg::Authenticate(req) => Self::on_auth_request(deps, env, req),
            terp_auth::AuthSudoMsg::Track(req) => Self::on_auth_track(deps, env, req),
            terp_auth::AuthSudoMsg::ConfirmExecution(req) => Self::on_auth_confirm(deps, env, req),
        }
    }

    fn extended_authenticate(
        _deps: cosmwasm_std::DepsMut,
        _auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn on_auth_added(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn on_auth_removed(
        deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult {
        // prune state
        WAVS_OPERATORS.clear(deps.storage);
        HEADSTASH_CFG.remove(deps.storage);
        NULLIFIERS.clear(deps.storage);
        ELIGIBILITY_ROOTS.clear(deps.storage);
        NEXT_ROOT_ID.remove(deps.storage);
        Ok(Response::default())
    }

    // - sign the hash of the proofs being verified per msgs. this lets us recreate the hash on-chain and verify actions,
    //   then recontstruct action values from proof inputs after verification (i.e coin amounts, auth params, etc.)
    // confirms wavs operator set authentication.
    // recomposes aggregated key and signature to enforce threshold minimums
    fn on_auth_request(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &Box<terp_auth::AuthenticationRequest>,
    ) -> Self::AuthProcessResult {
        let cfg = HEADSTASH_CFG.load(deps.storage)?;
        let agg_g2 = deps.api.bls12_381_aggregate_g2(&req.signature)?;
        let msg = to_json_binary(&req.tx_data.msgs)?;
        // reconstruct msg that was signed (hash of req.tx_data.msgs) (TODO: BENCHMARK)
        // `Hash-to-curve: H(msg) → G2`
        let qs = deps
            .api
            .bls12_381_hash_to_g2(cosmwasm_std::HashFunction::Sha256, &msg, &G2)?;

        if !cfg.w.msg.threshold.is_zero() {
            let w: WavsOperatorSet = cfg.w;
            let agg_g1 = hex::decode(&w.msg.aggregate_key)?;
            // e(agg_g1, qs) == e(G1, agg_g2)
            if !deps
                .api
                .bls12_381_pairing_equality(&agg_g1, &qs, &G1, &agg_g2)?
            {
                return Err(StdError::msg("auth_params"));
            };
        }

        Ok(Response::default())
    }

    fn on_auth_track(
        _deps: cosmwasm_std::DepsMut,
        _env: cosmwasm_std::Env,
        _req: &terp_auth::TrackRequest,
    ) -> Self::AuthProcessResult {
        Ok(Response::default())
    }

    fn on_auth_confirm(
        deps: cosmwasm_std::DepsMut,
        env: cosmwasm_std::Env,
        req: &terp_auth::ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult {
        let auth_data: wavs::BlsThresholdAuthData =
            from_json(&req.authenticator_params.clone().expect("cw-auth params"))?;

        // verifies circuit proofs, reverts any stateful change if errors.
        // Self::extended_authenticate(deps, params.clone())
        Ok(Response::default())
    }

    fn on_hooks(deps: cosmwasm_std::DepsMut, env: cosmwasm_std::Env) -> Self::AuthProcessResult {
        Ok(Response::default())
    }
}

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
    use cosmwasm_std::{Addr, Api, HashFunction, Uint128};

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
                    proof: derive_nd("test"),
                    raw: "test".into(),
                },
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(3),
            circuit_id: None,
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
                    proof: derive_nd(&"test"),
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
            wavs: valid_wavs_proof(3),
            circuit_id: None,
        };

        let res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // Should have 1 submessage for CreateDenom (initial minting happens in reply)
        assert_eq!(res.messages.len(), 1);

        // Check that it's a SubMsg with reply_on_success
        match &res.messages[0] {
            SubMsg { msg, reply_on, .. } => {
                match msg {
                    CosmosMsg::Stargate { type_url, value } => {
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
        let contract = deps.api.addr_make("contract");

        let env = mock_env();
        let info = message_info(&creator, &[Coin::new(Uint128::new(1000), "existing_token")]);

        let msg = InstantiateMsg {
            genesis_root: Binary::from([0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: None,
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                "existing_token".to_string(),
            )),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
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
                "test".to_string(),
            )),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: invalid_wavs,

            circuit_id: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();

        assert!(
            err.to_string()
                .contains("invalid amount of operators defined")
        );
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs,

            circuit_id: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
        };

        instantiate(deps.as_mut(), env, info, msg).unwrap();

        let stored_root = GENESIS_TREE_ROOT.load(&deps.storage).unwrap();
        assert_eq!(stored_root, genesis_root);

        let cfg = HEADSTASH_CFG.load(&deps.storage).unwrap();
        assert_eq!(cfg.gr, genesis_root);
        assert_eq!(cfg.distro_hash_domain, DistroHashDomain::PoseidonV1);
        assert_eq!(cfg.ts.len(), 1);
        assert!(cfg.w.msg.nonce == 0);

        let elig = distro::require_root(&deps.storage, GENESIS_ROOT_ID).unwrap();
        assert_eq!(elig.root, genesis_root);
        assert_eq!(elig.domain, DistroHashDomain::PoseidonV1);
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
        };

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(1),
            circuit_id: Some(0),
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
                subdenom: HeadstashTokenObject::new("test".to_string()),
                metadata: mock_metadata(),
                initial_mint: None,
                manager: None,
                minters: vec![],
            }),
            wavs: valid_wavs_proof(1),
            circuit_id: None,
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
}
