#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    ensure_eq, from_json, to_json_binary, Addr, Attribute, Binary, Coin, CosmosMsg, Decimal, Deps,
    DepsMut, Env, IbcBasicResponse, IbcDestinationCallbackMsg, MessageInfo, Response, StdAck,
    StdResult, Uint128, Uint256, WasmMsg,
};

use cosmwasm_std::{DecimalRangeExceeded, DivideByZeroError, OverflowError, StdError};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};
use cw_shit_denom::{AssetUnchecked, CheckedDenom, UncheckedDenom};
use cw_shit_denom::{DenomError, PossibleShit};
use cw_svg::SvgMintCallbackAction;
use msg::*;
use state::*;
use terp_rs::ibc::applications::transfer::v1::FungibleTokenPacketData;
use thiserror::Error;

pub mod msg {
    use super::*;
    use cosmwasm_schema::{cw_serde, QueryResponses};
    use cw_shit_denom::AssetUnchecked;
    #[cfg(not(target_arch = "wasm32"))]
    use cw_shit_denom::PossibleShit;
    use cw_svg::SvgMintCallbackAction;
    #[cw_serde]
    pub struct DaoParams {
        /// Dao addr
        pub addr: Addr,
        /// Set to 0 to disable
        pub floor: Uint128,
        /// Set to 0 to disable
        pub ceiling: Uint128,
    }

    // implements terp-speced adr-08 compat memo format:
    // <thing>MemoAdr08
    // - a: Vec<<thing>CallbackAction>
    #[cosmwasm_schema::cw_serde]
    pub struct ShitstrapMemoAdr08 {
        // ADR-8 tag
        pub ibc_callback: String,
        // array of msgs to perform after shitswapping. For shitstraps,
        pub a: Vec<CosmosMsg>,
    }
    #[cosmwasm_schema::cw_serde]
    pub struct ShitstrapCallbackAction {
        pub shitstrap: ShitstrapMsg,
        pub nfts: Vec<SvgMintCallbackAction>,
    }
    // funds must be one of accepted shit
    //

    #[cosmwasm_schema::cw_serde]
    pub struct ShitstrapMsg {
        pub shitter: String,
        pub shit: AssetUnchecked,
        pub recipient: Option<String>,
        pub dao: Option<String>,
    }

    #[cw_serde]
    pub struct InstantiateMsg {
        /// Dao one must be a member of to make use of shitstraps
        pub daos: Vec<DaoParams>,
        /// owner of the shit strap. This address will recieve all shit sent for this shitstrap.
        pub owner: Option<String>,
        /// a list of possible accepted assets, and the shit_rate you would like to set for.
        pub accepted: Vec<PossibleShit>,
        /// Desired cutoff points for shitstrap. 1000000 == 1 token.
        pub cutoff: Uint128,
        /// SHITMOS token address
        pub shitmos: UncheckedDenom,
        /// label for contract & front end
        pub title: String,
        /// description of shitstrap for recordkeeping
        pub description: String,
    }

    #[cw_serde]
    #[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
    pub enum ExecuteMsg {
        /// Entry point to participate in shit-strap
        ShitStrap {
            recp: Option<String>,
            shit: AssetUnchecked,
            dao: Option<String>,
        },
        /// Admin function to set full-of-shit status to on. *(used for emergencies or early cutoff)*
        Flush {},
        /// Cw20 Entry Point
        Receive(Cw20ReceiveMsg),
        /// Atomically deposit tokens and trigger mint from SVG collection if cutoff reached.
        /// Sends funds (native) along with the execute message.
        /// NOTE: recipient and 
        ShitStrapAndMint {
            shitstrap: ShitstrapMsg,
            mint: SvgMintCallbackAction,
        },
        /// Owner-only: set the SVG collection contract address for minting
        UpdateSvgCollection { address: String },
    }

    #[cw_serde]
    pub enum ReceiveMsg {
        /// Manually register an address for a shit strap when sending cw20 tokens.
        /// This can be a different address than the sender, if desired.
        ShitStrap {
            // will recieve shit unless recp is some.
            shitter: String,
            // overrides shit going to shitter.
            recp: Option<String>,
            // // shit sender is swapping for contracts shit
            dao: Option<String>,
        },
        // Atomically deposit cw20 tokens and trigger mint from SVG collection if cutoff reached.
        ShitStrapAndMint {
            shitter: String,
            recp: Option<String>,
            dao: Option<String>,
            mint: SvgMintCallbackAction,
        },
    }

    #[cw_serde]
    #[derive(QueryResponses)]
    #[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
    pub enum QueryMsg {
        /// Returns max possible deposit value for a shit-strap instance
        #[returns(super::state::Config)]
        Config {},
        #[returns(Uint128)]
        /// Current amount of shit value that has been deposited in the shit-strap.
        /// Can be used to calculate how much more is needed for a full-of-shit status.
        HasShit {},
        #[returns(Uint128)]
        /// Current amount of shit value that still is to be shit by the shitstrap.
        #[returns(Uint128)]
        ToShit {},
        #[returns(bool)]
        /// Query if the shit strap contract is no longer active
        FullOfShit {},
        #[returns(Uint128)]
        /// Query the shit conversation ratio for a given asset
        #[returns(Option<Uint128>)]
        ShitRate { asset: String },
        /// Query the shit conversation ratio for a given asset
        #[returns(Vec<PossibleShit>)]
        ShitRates {},
        // /// Query maximum token to be able to send before shitstrap will become full of shit.
        // LeftToShit { shit: String },
    }
    pub struct MigrateMsg {}
    
}

use cosmwasm_schema::{cw_schema, cw_serde};

pub mod state {
    use super::*;
    use cosmwasm_std::{Addr, Uint128, Uint256};
    use cw_shit_denom::{CheckedDenom, PossibleShit};
    use cw_storage_plus::{Item, Map};

    #[cw_serde]
    pub struct Config {
        pub owner: Addr,
        pub cutoff: Uint256,
        pub shitmos_addr: CheckedDenom,
        pub full_of_shit: bool, // once cutoff is reached, full of shit set to true
        pub title: String,
        pub description: String,
    }

    // version info for migration info
    pub const MINT_REPLY_ID: u64 = 1;
    pub const CW_SHITSTRAP: &str = "cw-shitstrap";
    pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
    pub const CONFIG: Item<Config> = Item::new("s");
    pub const CURRENT_SHITSTRAP_VALUE: Item<Uint256> = Item::new("h");
    pub const SHITSTRAP_STATE: Map<String, Uint256> = Map::new("t");
    pub const POSSIBLE_SHIT: Map<String, PossibleShit> = Map::new("ps");
    /// map of eligible daos an the floor and celing limits to vp for participation.
    /// 0,0 == no min & no max
    /// 0,y == no min, vp ceiling at y
    /// x,0 == floor at x, no ceiling
    /// x,y == floor at x, ceiling at y
    pub const DAOS: Map<Addr, (Uint256, Uint256)> = Map::new("ty");
    /// SVG collection address for minting NFTs when full_of_shit
    pub const SVG_MINTER: Item<Addr> = Item::new("svg_minter");
    /// Optional mint price override in the native gas denom (0 = use collection's price tier)
    pub const MINT_PRICE: Item<Uint128> = Item::new("mint_price");
    pub const ATOMINC_DECIMALS: u32 = 6u32;
    pub const MAX_DEC_PRECISION: u32 = 18u32;
    pub const SHIT_RATE_SCALE: u128 = 1_000_000;
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CW_SHITSTRAP, CONTRACT_VERSION)?;
    let owner = match msg.owner {
        Some(o) => deps.api.addr_validate(&o)?,
        None => info.sender,
    };

    if msg.cutoff == Uint128::zero() {
        return Err(ContractError::ShittyCutoffRatio {});
    }
    if msg.title.len() > 100usize {
        return Err(ContractError::ShittyTitle {});
    }
    if msg.description.len() > 1000usize {
        return Err(ContractError::ShittyDescription {});
    }
    if msg.accepted.is_empty() || msg.accepted.len() == 4usize {
        return Err(ContractError::UnnaceptableShitAmount {});
    }

    let mut unique_fee = Vec::new();
    for accepted in msg.accepted.iter() {
        if unique_fee.contains(&accepted) {
            return Err(ContractError::SameShit {});
        } else {
            accepted.token.clone().into_checked(deps.as_ref())?;
            unique_fee.push(accepted);
        }
        if accepted.shit_rate == Uint128::zero() {
            return Err(ContractError::ShittyConversionRatio {});
        };
    }

    let shitmos_addr = msg.shitmos.into_checked(deps.as_ref())?;
    for shit in msg.accepted {
        POSSIBLE_SHIT.save(deps.storage, shit.token.to_string(), &shit)?;
    }
    CONFIG.save(
        deps.storage,
        &Config {
            owner,
            cutoff: msg.cutoff.into(),
            shitmos_addr,
            full_of_shit: false,
            title: msg.title,
            description: msg.description,
        },
    )?;

    if msg.daos.len() > 10 {
        return Err(ContractError::ShittyDaoCount {});
    }
    for dao in msg.daos {
        DAOS.save(
            deps.storage,
            dao.addr,
            &(dao.floor.into(), dao.ceiling.into()),
        )?;
    }
    CURRENT_SHITSTRAP_VALUE.save(deps.storage, &Uint256::zero())?;

    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let sender = info.sender.clone();
    match msg {
        ExecuteMsg::ShitStrap { recp, shit, dao } => {
            execute_shit_strap_internal(deps, info, shit, sender, recp, dao, None)
        }
        ExecuteMsg::Flush {} => execute_flush(deps, sender),
        ExecuteMsg::Receive(cw20_msg) => receive_cw20_message(deps, info, cw20_msg),
        ExecuteMsg::ShitStrapAndMint {
            shitstrap: ss,
            mint: m,
        } => {
            let shitter = deps.api.addr_validate(&ss.shitter)?;
            execute_shit_strap_internal(deps, info, ss.shit, shitter, ss.recipient, ss.dao, Some(m))
        }
        ExecuteMsg::UpdateSvgCollection { address } => {
            execute_update_svg_collection(deps, info, address)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::HasShit {} => to_json_binary(&CURRENT_SHITSTRAP_VALUE.load(deps.storage)?),
        QueryMsg::FullOfShit {} => to_json_binary(&CONFIG.load(deps.storage)?.full_of_shit),
        QueryMsg::ShitRate { asset } => match &POSSIBLE_SHIT.may_load(deps.storage, asset)? {
            Some(ps) => to_json_binary(&Some(ps.shit_rate)),
            None => to_json_binary::<Option<Uint128>>(&None),
        },
        QueryMsg::ShitRates {} => {
            let shit_rates: Vec<PossibleShit> = POSSIBLE_SHIT
                .range(deps.storage, None, None, cosmwasm_std::Order::Descending)
                .map(|c| c.unwrap().1)
                .collect();
            to_json_binary(&shit_rates)
        }
        QueryMsg::ToShit {} => {
            let config = CONFIG.load(deps.storage)?;
            let this = CURRENT_SHITSTRAP_VALUE.load(deps.storage)?;
            to_json_binary(&config.cutoff.checked_sub(this)?)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_destination_callback(
    deps: DepsMut,
    env: Env,
    msg: IbcDestinationCallbackMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let me = env.contract.address;
    ensure_eq!(
        msg.packet.dest.port_id,
        "transfer",
        StdError::msg("only want to handle transfer packets")
    );

    if !cosmwasm_std::from_json::<StdAck>(&msg.ack.data)?.is_success() {
        return Err(ContractError::AckNotSuccess {});
    }

    let packet_data: FungibleTokenPacketData = from_json(&msg.packet.data)?;
    let t_funds = &match &msg.transfer {
        Some(t) => Ok::<&cosmwasm_std::IbcTransferCallback, ContractError>(t),
        None => return Err(ContractError::DidntSendShit {}),
    }?
    .funds;

    let receiver = deps.api.addr_validate(packet_data.receiver.as_ref())?;
    ensure_eq!(
        receiver,
        me,
        ContractError::ReceiverMismatch {
            w: me.to_string(),
            f: receiver.to_string(),
        }
    );

    let actions: Vec<CosmosMsg> = match !packet_data.memo.is_empty() {
        true => {
            let memo: ShitstrapMemoAdr08 = from_json(packet_data.memo.as_bytes())
                .map_err(|e| ContractError::MemoParseError { e: e.to_string() })?;

            if memo.ibc_callback != me.to_string() {
                return Err(ContractError::CallbackAddrMismatch {});
            }

            memo.a
        }
        _ => Default::default(),
    };

    // here we must sanity check sent funds.
    // - require t_funds does not undeflow available allocations to ibc-callback msgs (cannot spend more than sent)
    use std::collections::HashMap;
    let mut available: HashMap<String, Uint256> = HashMap::new();
    for coin in t_funds {
        let entry = available
            .entry(coin.denom.clone())
            .or_insert(Uint256::zero());
        *entry = entry.checked_add(coin.amount)?;
    }
    for a in &actions {
        match a {
            // limit use to only call specific mint msg
            CosmosMsg::Wasm(wasm_msg) => match wasm_msg {
                WasmMsg::Execute {
                    contract_addr,
                    funds,
                    ..
                } => {
                    if &me.to_string() != contract_addr {
                        return Err(ContractError::IbcCallbackError {
                            e: format!("only msgs to {} accepted", &me),
                        });
                    }
                    // require each specs funds to be accepted shit
                    for coin in funds {
                        if !SHITSTRAP_STATE.has(deps.storage, coin.denom.clone()) {
                            return Err(ContractError::IbcCallbackError {
                                e: format!("sent shit not accepted: {}", &coin.denom),
                            });
                        };
                        //  Must not exceed available funds for this denom
                        let remaining = available
                            .get(&coin.denom)
                            .copied()
                            .unwrap_or(Uint256::zero());
                        let new_remaining = remaining.checked_sub(coin.amount)?;
                        available.insert(coin.denom.clone(), new_remaining);
                    }
                }

                _ => {
                    return Err(ContractError::IbcCallbackError {
                        e: format!("only WasmMsg::Execute accepted"),
                    })
                }
            },
            _ => {
                return Err(ContractError::IbcCallbackError {
                    e: format!("only CosmosMsg::Wasm accepted"),
                })
            }
        }
    }
    Ok(IbcBasicResponse::new()
        .add_messages(actions)
        .add_attribute("action", "ibc_destination_callback")
        .add_attribute("receiver", receiver.to_string())
        .add_attribute("num_transfers", t_funds.len().to_string()))
}

// ── Shared core deposit logic ──

/// Result of execute_deposit — metadata that callers use to decide cutoff / mint paths.
struct DepositData {
    shit_value: Uint256,
    received_denom: CheckedDenom,
    new_val: Uint256,
    shit_rate: Uint128,
    denom_key: String,
}

fn execute_deposit(
    deps: &mut DepsMut,
    info: &MessageInfo,
    shit: &AssetUnchecked,
    dao: &Option<String>,
) -> Result<DepositData, ContractError> {
    // DAO membership check
    let daos: Vec<(Addr, (Uint256, Uint256))> = DAOS
        .range(deps.storage, None, None, cosmwasm_std::Order::Descending)
        .collect::<StdResult<Vec<_>>>()?;

    if let Some(d) = dao {
        #[cfg(feature = "dao")]
        {
            let room = daos
                .iter()
                .find(|(a, _)| a == d)
                .ok_or(ContractError::DidntSendShit {})?;
            check_voting_power(
                deps.as_ref(),
                info.sender(),
                d,
                Uint128::try_from(room.1 .0).unwrap_or(Uint128::zero()),
                Uint128::try_from(room.1 .1).unwrap_or(Uint128::zero()),
            )?;
        }
        #[cfg(not(feature = "dao"))]
        {
            let _ = (daos, d);
            return Err(ContractError::DaoNotEnabled {});
        }
    } else {
        #[cfg(feature = "dao")]
        {
            for (dao_addr, (floor_raw, ceiling_raw)) in &daos {
                check_voting_power(
                    deps.as_ref(),
                    info.sender(), // sender must be dao member
                    dao_addr,
                    Uint128::try_from(*floor_raw).unwrap_or(Uint128::zero()),
                    Uint128::try_from(*ceiling_raw).unwrap_or(Uint128::zero()),
                )?;
            }
        }
    }

    // Match the deposit token against accepted list
    let shitkey = shit.denom.to_string();
    let matched = match POSSIBLE_SHIT.may_load(deps.storage, shitkey)? {
        Some(d) => d,
        None => return Err(ContractError::WrongShit {}),
    };

    // Validate funds sent
    match &shit.denom {
        UncheckedDenom::Native(t) => {
            if !info
                .funds
                .iter()
                .any(|c| c.denom == *t && c.amount == shit.amount)
            {
                return Err(ContractError::DidntSendShit {});
            }
        }
        UncheckedDenom::Cw20(c) => {
            if info.sender.to_string() != *c {
                return Err(ContractError::ShittyCw20 {});
            }
        }
    }

    // Calculate shit value (conversion rate)
    let current_shit_value = CURRENT_SHITSTRAP_VALUE.load(deps.storage)?;
    let (shit_value, received_denom) = calculate_shit_value(deps.as_ref(), &matched, shit.amount)?;

    // Update SHITSTRAP_STATE for this denom (or create it)
    let denom_key = received_denom.to_string();
    SHITSTRAP_STATE.update::<_, ContractError>(deps.storage, denom_key.clone(), |prev| {
        let this = prev.unwrap_or_default();
        let new_amount = this
            .checked_add(shit.amount)
            .map_err(|e| ContractError::ShitStd(StdError::msg(e)))?;
        Ok(new_amount)
    })?;

    // Calculate new total value
    let new_val = shit_value + current_shit_value;

    Ok(DepositData {
        shit_value,
        received_denom,
        new_val,
        shit_rate: matched.shit_rate,
        denom_key,
    })
}

// ── Messaging helpers ──

fn build_transfer_msg(
    is_cw20: bool,
    denom: String,
    amount: Uint256,
    recipient: Addr,
) -> Result<CosmosMsg, ContractError> {
    if is_cw20 {
        Ok(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: denom,
            msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.to_string(),
                amount: Uint128::try_from(amount)
                    .map_err(|_| ContractError::DigginForShitTreasure {})?
                    .into(),
            })?,
            funds: vec![],
        }))
    } else {
        Ok(CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
            to_address: recipient.to_string(),
            amount: vec![Coin { denom, amount }],
        }))
    }
}

/// Send accumulated tokens to owner. Used on cutoff in both ShitStrap and ShitStrapAndMint reply success.
fn send_accumulated_to_owner(
    deps: &Deps,
    exclude_denom: &str,
    exclude_shit_amount: Uint256,
    overflow: Uint256,
    recipient: &Addr,
) -> Result<Vec<CosmosMsg>, ContractError> {
    let mut msgs = vec![];

    for owned in SHITSTRAP_STATE.range(deps.storage, None, None, cosmwasm_std::Order::Ascending) {
        let tokens = owned?;
        let denom_key = &tokens.0;
        let total_amount = tokens.1;

        let is_cw20 = match POSSIBLE_SHIT.may_load(deps.storage, denom_key.to_string())? {
            Some(d) => match d.token {
                UncheckedDenom::Cw20(_) => true,
                _ => false,
            },
            None => unimplemented!(),
        };

        // ensure we do not recalculate  current sent
        let send_amount = if denom_key == exclude_denom {
            total_amount - (exclude_shit_amount - overflow)
        } else {
            total_amount
        };

        if send_amount.is_zero() {
            continue;
        }

        let msg = build_transfer_msg(
            is_cw20,
            denom_key.to_string(),
            send_amount,
            recipient.clone(),
        )?;
        msgs.push(msg);
    }

    Ok(msgs)
}

/// Entry point to participate in shitstrap.
/// recp - if set, will recieve funds from shitstrap action
/// shitter - the address set to recieve the funds from the shitstrap action
///
/// SubMsg when cutoff is reached (ShitStrapAndMint path) instead of the normal
/// owner token distribution (which happens asynchronously in the reply handler).
pub fn execute_shit_strap_internal(
    mut deps: DepsMut,
    info: MessageInfo,
    shit: AssetUnchecked,
    shitter: Addr,
    recp: Option<String>,
    dao: Option<String>,
    nft: Option<SvgMintCallbackAction>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if config.full_of_shit {
        return Err(ContractError::FullOfShit {});
    }
    let deposit = execute_deposit(&mut deps, &info, &shit, &dao)?;
    let mut msgs: Vec<CosmosMsg> = vec![];
    // let mut submsgs: Vec<SubMsg> = vec![];
    let mut attrs: Vec<Attribute> = vec![];

    let new_val = deposit.new_val;
    let cutoff = config.cutoff;

    if new_val >= cutoff {
        // Normal ShitStrap cutoff — distribute to owner, refund overflow
        let (_, return_amount) = calculate_shit_return(new_val, cutoff, deposit.shit_rate.into())?;

        //  refunds always are obligated to shitter, even if there is some recp.
        msgs.push(build_transfer_msg(
            matches!(deposit.received_denom, CheckedDenom::Cw20(_)),
            deposit.received_denom.to_string(),
            return_amount,
            shitter.clone(),
        )?);

        let mut updated = config.clone();
        updated.full_of_shit = true;
        CONFIG.save(deps.storage, &updated)?;
        attrs.push(Attribute::new("cutoff_reached", "true"));
    }

    // send all funds we just shitstrap into the actions
    let send_shitmos = match nft {
        Some(nft) => into_cosmos_msg(
            nft.c,
            &nft.msg,
            vec![config.shitmos_addr.to_cw_coin(deposit.shit_value)?],
        )?,
        None => config.shitmos_addr.get_transfer_to_message(
            &match recp {
                Some(r) => deps.api.addr_validate(&r)?,
                None => shitter,
            },
            deposit.shit_value,
        )?,
    };

    // transfer funds that were shitstrapped to the owner.
    let send_shit = build_transfer_msg(
        match deposit.received_denom {
            CheckedDenom::Native(_) => false,
            CheckedDenom::Cw20(_) => true,
        },
        deposit.denom_key.to_string(),
        shit.amount,
        config.owner.clone(),
    )?;
    msgs.extend(vec![send_shitmos, send_shit]);

    // Save updated total value
    CURRENT_SHITSTRAP_VALUE.save(deps.storage, &new_val)?;
    Ok(Response::new().add_messages(msgs).add_attributes(attrs))
}

/// Takes any cw-serde compatible type and wraps it in a WasmMsg::Execute
///
/// # Type Parameters
/// * `T` - Any type that implements `Serialize` (what `#[cw_serde]` gives you)
///
/// # Arguments
/// * `contract_addr` - The contract to execute against
/// * `action` - The struct containing the action data
/// * `funds` - Optional coins to send with the message
pub fn into_cosmos_msg<T: cw_schema::Schemaifier + cosmwasm_schema::serde::Serialize>(
    contract_addr: impl Into<String>,
    action: &T,
    funds: Vec<cosmwasm_std::Coin>,
) -> Result<CosmosMsg, cosmwasm_std::StdError> {
    let msg = to_json_binary(action)?;

    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: contract_addr.into(),
        msg,
        funds,
    }))
}

/// Entry point to manually set contract to full of shit. Owner only.
pub fn execute_flush(deps: DepsMut, sender: Addr) -> Result<Response, ContractError> {
    if sender != CONFIG.load(deps.storage)?.owner {
        return Err(ContractError::ShittyAuthorization {});
    }
    let mut config = CONFIG.load(deps.storage)?;
    if config.full_of_shit {
        return Err(ContractError::FullOfShit {});
    }
    config.full_of_shit = true;
    CONFIG.save(deps.storage, &config)?;

    let mut msgs = vec![];

    let current_shit_value = CURRENT_SHITSTRAP_VALUE.load(deps.storage)?;

    // Send leftover SHITMOS to owner
    let send_shitmos = config
        .shitmos_addr
        .get_transfer_to_message(&config.owner, config.cutoff - current_shit_value)?;
    msgs.push(send_shitmos);

    // Send all collected tokens to owner
    let own_msgs = send_accumulated_to_owner(
        &deps.as_ref(),
        "",
        Uint256::zero(),
        Uint256::zero(),
        &config.owner,
    )?;
    msgs.extend(own_msgs);

    Ok(Response::new()
        .add_attribute("flusher", sender.to_string())
        .add_messages(msgs))
}

/// Owner-only: set the SVG collection contract address for minting
pub fn execute_update_svg_collection(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::ShittyAuthorization {});
    }
    let addr = deps.api.addr_validate(&address)?;
    SVG_MINTER.save(deps.storage, &addr)?;
    Ok(Response::new()
        .add_attribute("action", "update_svg_collection")
        .add_attribute("address", address))
}

#[cfg(feature = "dao")]
fn check_voting_power(
    deps: Deps,
    sender: &Addr,
    dao: &Addr,
    floor: Uint128,
    ceiling: Uint128,
) -> Result<(), ContractError> {
    use dao_interface::voting;

    let vp = deps
        .querier
        .query_wasm_smart::<voting::VotingPowerAtHeightResponse>(
            dao,
            &voting::Query::VotingPowerAtHeight {
                address: sender.to_string(),
                height: None,
            },
        )?
        .power;

    if vp.is_zero() {
        return Err(ContractError::DontHaveShitStaked {});
    }
    if floor != Uint128::zero() && floor.gt(&vp) {
        return Err(ContractError::DontHaveShitStaked {});
    }
    if ceiling != Uint128::zero() && ceiling.lt(&vp) {
        return Err(ContractError::DontHaveShitStaked {});
    }
    Ok(())
}

fn receive_cw20_message(
    deps: DepsMut,
    info: MessageInfo,
    msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    let sender_str = info.sender.to_string();
    match from_json(&msg.msg)? {
        ReceiveMsg::ShitStrap { shitter, recp, dao } => {
            let shitter = deps.api.addr_validate(&shitter)?;
            execute_shit_strap_internal(
                deps,
                info,
                AssetUnchecked {
                    denom: UncheckedDenom::Cw20(sender_str),
                    amount: msg.amount,
                },
                shitter,
                recp,
                dao,
                None,
            )
        }
        ReceiveMsg::ShitStrapAndMint {
            shitter,
            recp,
            dao,
            mint,
        } => {
            let sender = deps.api.addr_validate(&shitter)?;
            execute_shit_strap_internal(
                deps,
                info,
                AssetUnchecked {
                    denom: UncheckedDenom::Cw20(sender_str),
                    amount: msg.amount,
                },
                sender,
                recp,
                dao,
                Some(mint),
            )
        }
    }
}

use cosmwasm_std::Fraction;

pub fn calculate_shit_value(
    deps: Deps,
    matched: &PossibleShit,
    eligible_shit_amount: Uint256,
) -> Result<(Uint256, CheckedDenom), ContractError> {
    let rate = Decimal::from_atomics(matched.shit_rate, MAX_DEC_PRECISION)?;
    Ok((
        eligible_shit_amount.multiply_ratio(rate.numerator(), rate.denominator()),
        matched.clone().token.into_checked(deps)?,
    ))
}

pub fn calculate_shit_return(
    new_val: Uint256,
    cutoff: Uint256,
    shit_rate: Uint256,
) -> Result<(Uint256, Uint256), ContractError> {
    let overflow = new_val - cutoff;
    let return_to_shitter_amnt =
        overflow.multiply_ratio(10u128.pow(MAX_DEC_PRECISION as u32), shit_rate);
    Ok((overflow, return_to_shitter_amnt))
}

#[cfg(feature = "interface")]
pub mod interface {
    pub use super::msg::*;
    use crate::contract::{execute, instantiate, query, CW_SHITSTRAP};
    use cw_orch::prelude::*;

    #[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CW_SHITSTRAP)]
    pub struct CwShitstrap;

    impl<Chain: CwEnv> Uploadable for CwShitstrap<Chain> {
        /// Return the path to the wasm file corresponding to the contract
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path_from_crates_label(CW_SHITSTRAP)
                .unwrap()
        }
        /// Returns a CosmWasm contract wrapper
        fn wrapper() -> Box<dyn MockContract<Empty>> {
            Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
        }
    }
}
#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    ShitStd(#[from] StdError),

    #[error("{0}")]
    ShitDenomError(#[from] DenomError),

    #[error("{0}")]
    DecimalRangeExceeded(#[from] DecimalRangeExceeded),

    #[error("{0}")]
    OverflowError(#[from] OverflowError),

    #[error("{0}")]
    DivideByZeroError(#[from] DivideByZeroError),

    #[error("MemoParseError: {e}")]
    MemoParseError { e: String },

    #[error("IbcCallbackError: {e}")]
    IbcCallbackError { e: String },

    #[error("CallbackAddrMismatch")]
    CallbackAddrMismatch,

    #[error("Wrong Shit.")]
    WrongShit {},

    #[error("Full of Shit.")]
    FullOfShit {},

    #[error("Stake your shit to use this shitstrap (you ain't a DAO member)")]
    DontHaveShitStaked {},

    #[error("Tryna shit with too many DAOs, max 10")]
    ShittyDaoCount {},

    #[error("Did Not Send Shit, Liar.")]
    DidntSendShit {},

    #[error("Unable to claim refund")]
    DigginForShitTreasure {},

    #[error("Dao Feature Not Enabled")]
    DaoNotEnabled {},

    #[error("Unauthorized")]
    ShittyAuthorization {},

    #[error(
        "you are trying to set identical accepted_shit values. Try again with unique accepted_shit"
    )]
    SameShit {},

    #[error("You are trying to set too much shit to be accepted for this shitstrap.")]
    UnnaceptableShitAmount {},

    #[error("Cannot set cutoff as 0")]
    ShittyCutoffRatio {},

    #[error("Cannot set conversion ratio as 0")]
    ShittyConversionRatio {},

    #[error("Shitstrap title set for too long")]
    ShittyTitle {},

    #[error("Shitstrap title set for too long")]
    ShittyDescription {},

    #[error("You are trying to participate with a cw20. use the Cw20RecieveMsg.")]
    ShittyCw20 {},
    // Add any other custom errors you like here.
    // Look at https://docs.rs/thiserror/1.0.21/thiserror/ for details.
    #[error("No SVG collection address configured for minting")]
    NoSvgCollection {},

    #[error("SVG minting failed")]
    MintFailed {},

    #[error("Mint temp storage not found")]
    MintTempNotFound {},

    #[error("IBC ack is not a success")]
    AckNotSuccess {},

    #[error("No transfer data in IBC destination callback")]
    NoTransferData {},

    #[error("Receiver mismatch: expected {w}, got {f}")]
    ReceiverMismatch { w: String, f: String },
}

impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}
