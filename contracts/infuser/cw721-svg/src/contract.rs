use crate::{ContractError, Cw721SvgContract, SvgMetadata};

use cosmwasm_std::{
    ensure, Addr, BankMsg, Coin, CustomMsg, Deps, DepsMut, Env, Event, MessageInfo, Response,
    Uint256,
};

use cosmwasm_schema::{cw_serde, QueryResponses};

use cw721::traits::{Cw721CustomMsg, Cw721Execute, Cw721Query};
use cw_storage_plus::{Item, Map};

use sha2::{Digest, Sha256};

// 0.043 MB limit
pub const MAX_SVG_SIZE: usize = 42 * 1024;
pub const MAX_TOTAL_SUPPLY: u64 = 10_000;
pub const PAUSED: Item<()> = Item::new("svg_paused");
pub const SVG_TEMPLATE: Item<String> = Item::new("svg_template");
pub const VARIABLES: Item<Vec<cw_svg::VariableDef>> = Item::new("variables");
pub const TEMPLATE_SLOTS: Item<Vec<cw_svg::TemplateSlot>> = Item::new("template_slots");
/// Full collection metadata. Mint must not range `collection_extension`
/// (zk-wasmd cacheMergeIterator panics on that Map::range during execute).
pub const SVG_COLLECTION: Item<crate::SvgCollectionMetadata> = Item::new("svg_collection");
pub const MINTER_ADDRS: Map<&Addr, u32> = Map::new("minter_addrs");
pub const MINTER_NONCE: Map<&(String, String), ()> = Map::new("minter_nonce");
pub const WL_MINTER_ADDRS: Map<&Addr, u32> = Map::new("wl_minter_addrs");
/// DREGG runtime Terp accounts may mint without native payment (foreign rail already paid).
pub const DREGG_RUNTIME_MINTERS: Map<&Addr, ()> = Map::new("dregg_runtime_minters");
pub const DREGG_IBC_ONLY: Item<bool> = Item::new("dregg_ibc_only");

pub fn execute_mint(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    m: cw_svg::MintMsg,
) -> Result<Response, ContractError> {
    let cw_svg::MintMsg {
        alloc,
        amnt,
        proof_hashes,
        ext,
    } = m;
    if let Some(_) = PAUSED.may_load(deps.storage)? {
        return Err(ContractError::MintingPaused {});
    }
    let c = Cw721SvgContract::default();
    let config = SVG_COLLECTION.load(deps.storage)?;
    let mint_count = c.query_num_tokens(deps.storage)?.count;

    if env.block.time < config.mint_start_time.unwrap_or_default() {
        return Err(ContractError::MintingNotStarted {});
    }

    if let Some(end_time) = config.mint_end_time {
        if env.block.time > end_time {
            return Err(ContractError::MintingEnded {});
        }
    }
    if mint_count + m.amnt > config.total {
        return Err(ContractError::CannotMintMoreThanTotal {});
    }

    // ADR-36 mint auth (CosmosArbitrary over DST+nonce[|digest]).
    // - require_auth collections (pledges): ext is mandatory.
    // - open-mint collections: ext optional; when present it is fully verified.
    if config.require_auth && ext.is_none() {
        return Err(ContractError::GenericMsg {
            m: "authentication required before mint: provide ext (CosmosArbitrary over DST+nonce[|digest])".into(),
        });
    }

    let recp = match ext {
        Some(e) => {
            let msg_bytes = e.message.as_slice();
            ensure!(
                msg_bytes.starts_with(cw_svg::CW721SVG_DST.as_bytes()),
                ContractError::GenericMsg {
                    m: "auth message must start with CW721SVG_DST".into(),
                }
            );
            ensure!(
                msg_bytes.len() <= cw_svg::CW721SVG_DST.len() + cw_svg::AUTH_MSG_MAX_EXTRA,
                ContractError::GenericMsg {
                    m: "auth message too long (DST + nonce[|digest_hex] max)".into(),
                }
            );
            ensure!(
                msg_bytes.len() > cw_svg::CW721SVG_DST.len(),
                ContractError::GenericMsg {
                    m: "auth message missing nonce after DST".into(),
                }
            );

            let k = (e.message.to_hex(), e.pubkey.to_hex());
            ensure!(
                !MINTER_NONCE.has(deps.storage, &k),
                ContractError::GenericMsg {
                    m: "invalid auth nonce".into(),
                }
            );
            let recovered = e.verify_return_readable(deps.api).map_err(|err| {
                ContractError::GenericMsg {
                    m: format!("auth verify failed: {err}"),
                }
            })?;
            // Signer derived from pubkey must be the tx sender (no relay of foreign auth).
            ensure!(
                recovered == info.sender.as_str(),
                ContractError::GenericMsg {
                    m: "auth pubkey does not match tx sender".into(),
                }
            );
            MINTER_NONCE.save(deps.storage, &k, &())?;
            info.sender.clone()
        }
        None => info.sender.clone(),
    };
    let recp = &recp;

    let is_whitelisted = match !proof_hashes.is_empty() {
        true => verify_whitelist(deps.as_ref(), &recp, proof_hashes, alloc, amnt)?,
        false => false,
    };
    let is_dregg_runtime = DREGG_RUNTIME_MINTERS.has(deps.storage, &info.sender);
    if DREGG_IBC_ONLY.may_load(deps.storage)?.unwrap_or(false) && !is_dregg_runtime && !is_whitelisted
    {
        return Err(ContractError::Unauthorized {});
    }
    // Native collection payment skipped for whitelist OR DREGG runtime (OR-rail foreign pay).
    let payment_msg = if is_whitelisted || is_dregg_runtime {
        None
    } else {
        validate_and_build_payment(
            c.config.withdraw_address.load(deps.storage)?,
            &config,
            &info,
            mint_count,
            amnt,
        )?
    };

    let variables = VARIABLES.load(deps.storage)?;
    let mut events = vec![];
    for i in 0..amnt {
        let tid = mint_count + i;
        let token_id = tid.to_string();
        // Generate per-token entropy: sha256(tid || seed || block_height)
        let mut hasher = Sha256::new();
        hasher.update(tid.to_le_bytes());
        hasher.update(config.seed.as_slice());
        hasher.update(env.block.height.to_le_bytes());
        let token_seed = hasher.finalize();
        // Resolve each variable using entropy
        let params: Vec<cw_svg::TokenParam> = variables
            .iter()
            .enumerate()
            .map(|(var_idx, var_def)| {
                cw_svg::resolve_variable(&token_seed, var_idx, var_def, derive_entropy)
            })
            .collect();
        let extension = SvgMetadata { params };

        let token = cw721::state::NftInfo {
            owner: info.sender.clone(),
            approvals: vec![],
            token_uri: None,
            extension,
        };

        Cw721SvgContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |old| match old {
                Some(_) => Err(ContractError::Base(
                    cw721::error::Cw721ContractError::Claimed {},
                )),
                None => Ok(token),
            },
        )?;
        Cw721SvgContract::default()
            .config
            .increment_tokens(deps.storage)?;

        events.push(
            Event::new("mint")
                .add_attribute("token_id", &token_id)
                .add_attribute("owner", info.sender.to_string()),
        );
    }

    // Track per-address mint count
    let prev_count = MINTER_ADDRS
        .may_load(deps.storage, &info.sender)?
        .unwrap_or(0);
    MINTER_ADDRS.save(deps.storage, &info.sender, &(prev_count + amnt as u32))?;
    // Track whitelist mint count if applicable
    if is_whitelisted {
        let prev_wl = WL_MINTER_ADDRS
            .may_load(deps.storage, &info.sender)?
            .unwrap_or(0);
        WL_MINTER_ADDRS.save(deps.storage, &info.sender, &(prev_wl + amnt as u32))?;
    }

    let mut res = Response::new()
        .add_attribute("action", "mint")
        .add_attribute("minter", info.sender.to_string())
        .add_attribute("amount", amnt.to_string());

    if let Some(bank_msg) = payment_msg {
        res = res.add_message(bank_msg);
    }

    for event in events {
        res = res.add_event(event);
    }

    Ok(res)
}

pub fn execute_pause(
    deps: DepsMut,
    info: MessageInfo,
    pause: bool,
) -> Result<Response, ContractError> {
    match Cw721SvgContract::default()
        .query_creator_ownership(deps.storage)?
        .owner
    {
        Some(ow) => match ow == info.sender {
            true => {
                match PAUSED.may_load(deps.storage)? {
                    Some(_) => PAUSED.remove(deps.storage),
                    None => PAUSED.save(deps.storage, &())?,
                }
                Ok(Response::new()
                    .add_attribute("action", "pause")
                    .add_attribute("paused", pause.to_string()))
            }
            false => Err(ContractError::Unauthorized {}),
        },
        None => Err(ContractError::Unauthorized {}),
    }
}

/// Find the price per token for the current mint count from the tier list.
/// Returns None if no tiers are configured (free mint).
pub fn current_tier_price(tiers: &[PriceTier], mint_count: u64) -> Option<&Coin> {
    tiers
        .iter()
        .find(|tier| mint_count < tier.until_count)
        .map(|tier| &tier.price)
}

/// A price tier that applies until the mint count reaches `until_count`.
/// Tiers must be sorted ascending by `until_count`.
/// Example: [{ until_count: 100, price: 1_000_000utoken }, { until_count: 500, price: 5_000_000utoken }]
/// means: first 100 mints cost 1 token, mints 101-500 cost 5 tokens.
#[cw_serde]
pub struct PriceTier {
    /// This tier applies while mint_count < until_count
    pub until_count: u64,
    /// Price per token in this tier
    pub price: Coin,
}

/// Query message for the merkle whitelist contract.
/// Must match the whitelist contract's QueryMsg::HasMember variant.
#[cw_serde]
pub enum WhitelistHasMemberMsg {
    HasMember {
        member: String,
        proof_hashes: Vec<String>,
    },
}

#[cw_serde]
#[cfg_attr(feature = "interface", derive(cw_orch::ExecuteFns))]
pub enum SvgExecuteMsgExt {
    Mint(cw_svg::MintMsg),
    Pause {
        pause: bool,
    },
    /// Owner-only: set or clear the merkle whitelist contract address
    UpdateWhitelist {
        /// Set to Some(addr) to enable whitelist, None to disable
        address: Option<String>,
    },
}

#[cw_ownable::cw_ownable_query]
#[cfg_attr(feature = "interface", derive(cw_orch::QueryFns))]
#[cw_serde]
#[derive(QueryResponses)]
pub enum SvgQueryMsgExt {
    #[returns(SvgTokenUriResponse)]
    SvgTokenUri { token_id: String },
    /// Returns a preview SVG with random placeholder values filled in.
    #[returns(SvgTokenUriResponse)]
    SvgPlaceholder { seed: Option<String> },
    #[returns(SvgTemplateResponse)]
    SvgTemplate {},
    #[returns(Option<Addr>)]
    Whitelist {},
    /// Returns the total mint count for a given address.
    #[returns(MintCountResponse)]
    MintCount { address: String },
    /// Returns the whitelist mint count for a given address.
    #[returns(MintCountResponse)]
    WlMintCount { address: String },
    #[returns(PriceTier)]
    CurrentPriceTier {},
}

impl CustomMsg for SvgQueryMsgExt {}
impl Cw721CustomMsg for SvgQueryMsgExt {}

#[cw_serde]
pub struct MigrateMsg {
    /// Existing collections: open the DREGG IBC mint gate (runtime accounts + IBC-only).
    pub dregg_ibc_only: Option<bool>,
    pub runtime_minters: Option<Vec<String>>,
}

/// Response from the whitelist HasMember query
#[cw_serde]
pub struct HasMemberResponse {
    pub has_member: bool,
}

#[cw_serde]
pub struct SvgTokenUriResponse {
    pub svg: String,
}

#[cw_serde]
pub struct SvgTemplateResponse {
    pub template: String,
}

#[cw_serde]
pub struct MintCountResponse {
    pub address: String,
    pub count: u32,
}

/// Validate that the sent funds match the required payment for `amount` tokens.
/// Returns the total cost as a BankMsg to forward to the payment address.
fn validate_and_build_payment(
    payment_address: String,
    config: &crate::SvgCollectionMetadata,
    info: &MessageInfo,
    mint_count: u64,
    amount: u64,
) -> Result<Option<BankMsg>, ContractError> {
    // If no price tiers, mint is free — require no funds
    if config.price_tiers.is_empty() {
        if !info.funds.is_empty() {
            return Err(ContractError::IncorrectPayment {
                expected: "free mint (no funds)".into(),
            });
        }
        return Ok(None);
    }

    // Calculate total cost across all tokens being minted.
    // Each token might span different tiers.
    let mut total_cost: Uint256 = Uint256::zero();
    let mut expected_denom: Option<String> = None;

    for i in 0..amount {
        let count_at = mint_count + i;
        let price = current_tier_price(&config.price_tiers, count_at)
            .ok_or(ContractError::CannotMintMoreThanTotal {})?;

        // All tiers must use the same denom
        match &expected_denom {
            Some(d) => {
                if d != &price.denom {
                    return Err(ContractError::IncorrectPayment {
                        expected: format!("all tiers must use denom {}", d),
                    });
                }
            }
            None => expected_denom = Some(price.denom.clone()),
        }

        total_cost += price.amount;
    }
    let denom = expected_denom.unwrap();
    if total_cost.is_zero() {
        return Ok(None);
    }

    // Check the sender sent exactly the right amount
    let sent = info
        .funds
        .iter()
        .find(|c| c.denom == denom)
        .map(|c| c.amount)
        .unwrap_or(Uint256::zero());

    if sent != total_cost {
        return Err(ContractError::IncorrectPayment {
            expected: format!("{}{} got {}", total_cost, denom, sent),
        });
    }

    Ok(Some(BankMsg::Send {
        to_address: payment_address,
        amount: vec![Coin {
            denom,
            amount: total_cost,
        }],
    }))
}

/// Verify that the sender is on the merkle whitelist and has not exceeded their mint allocation.
/// Returns true if whitelisted (fees should be bypassed), false if no whitelist check was performed.
///
/// When `allocation` is Some, the merkle leaf is `hash(sender || allocation)` and the
/// address may mint up to `allocation` tokens via whitelist. When None, the leaf is
/// just `hash(sender)` and the whitelist contract's `per_address_limit` is used.
fn verify_whitelist(
    deps: Deps,
    recp: &Addr,
    proof_hashes: Vec<String>,
    allocation: u32,
    mint_amount: u64,
) -> Result<bool, ContractError> {
    let whitelist = match Cw721SvgContract::default()
        .query_collection_info_and_extension(deps)?
        .extension
        .whitelist
    {
        Some(a) => a,
        None => {
            return Ok(false);
        }
    };

    // Build the member string: sender + allocation
    let member = format!("{}{}", recp, allocation);
    let res: HasMemberResponse = deps.querier.query_wasm_smart(
        whitelist.clone(),
        &WhitelistHasMemberMsg::HasMember {
            member,
            proof_hashes,
        },
    )?;

    if !res.has_member {
        return Err(ContractError::NotWhitelisted {
            addr: recp.to_string(),
        });
    }

    // Determine max mints allowed for this address
    // Enforce per-address whitelist mint limit
    let current_wl_count = whitelist_mint_count(deps, &recp);
    if current_wl_count + mint_amount as u32 > allocation {
        return Err(ContractError::MaxPerAddressLimitExceeded {});
    }

    Ok(true)
}

/// Get the total mint count for an address.
pub fn mint_count(deps: Deps, addr: &Addr) -> u32 {
    MINTER_ADDRS
        .may_load(deps.storage, addr)
        .unwrap_or(None)
        .unwrap_or(0)
}

/// Get the whitelist mint count for an address.
pub fn whitelist_mint_count(deps: Deps, addr: &Addr) -> u32 {
    WL_MINTER_ADDRS
        .may_load(deps.storage, addr)
        .unwrap_or(None)
        .unwrap_or(0)
}

/// Derive per-variable entropy via sha256(token_seed || var_idx), returning a u64.
fn derive_entropy(token_seed: &[u8], var_idx: usize) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(token_seed);
    hasher.update((var_idx as u64).to_le_bytes());
    let hash = hasher.finalize();
    u64::from_le_bytes(hash[0..8].try_into().unwrap())
}

pub fn query_svg_token_uri(
    deps: cosmwasm_std::Deps,
    token_id: String,
) -> Result<String, ContractError> {
    let template = SVG_TEMPLATE.load(deps.storage)?;
    let slots = TEMPLATE_SLOTS.load(deps.storage)?;
    let token = Cw721SvgContract::default()
        .config
        .nft_info
        .load(deps.storage, &token_id)?;

    let mut result = String::with_capacity(template.len());
    let mut cursor = 0usize;

    for slot in &slots {
        let start = slot.start as usize;
        let end = slot.end as usize;
        result.push_str(&template[cursor..start]);
        result.push_str(&token.extension.params[slot.var_idx as usize].value);
        cursor = end;
    }
    result.push_str(&template[cursor..]);

    Ok(result)
}

pub fn query_svg_placeholder(
    deps: cosmwasm_std::Deps,
    seed: Option<String>,
) -> Result<String, ContractError> {
    let template = SVG_TEMPLATE.load(deps.storage)?;
    let slots = TEMPLATE_SLOTS.load(deps.storage)?;
    let variables = VARIABLES.load(deps.storage)?;

    let seed_bytes = seed.unwrap_or_else(|| "placeholder".to_string());
    let mut hasher = Sha256::new();
    hasher.update(seed_bytes.as_bytes());
    let token_seed = hasher.finalize();

    let params: Vec<cw_svg::TokenParam> = variables
        .iter()
        .enumerate()
        .map(|(var_idx, var_def)| {
            cw_svg::resolve_variable(&token_seed, var_idx, var_def, derive_entropy)
        })
        .collect();

    let mut result = String::with_capacity(template.len());
    let mut cursor = 0usize;

    for slot in &slots {
        let start = slot.start as usize;
        let end = slot.end as usize;
        result.push_str(&template[cursor..start]);
        result.push_str(&params[slot.var_idx as usize].value);
        cursor = end;
    }
    result.push_str(&template[cursor..]);

    Ok(result)
}

pub fn query_config(
    deps: cosmwasm_std::Deps,
) -> Result<
    cw721::msg::CollectionInfoAndExtensionResponse<crate::SvgCollectionMetadata>,
    ContractError,
> {
    Ok(Cw721SvgContract::default().query_collection_info_and_extension(deps)?)
}

pub fn query_svg_template(deps: cosmwasm_std::Deps) -> Result<String, ContractError> {
    Ok(SVG_TEMPLATE.load(deps.storage)?)
}

pub fn execute_update_whitelist(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    address: Option<String>,
) -> Result<Response, ContractError> {
    let contract = Cw721SvgContract::default();
    let mut config = contract.query_collection_info_and_extension(deps.as_ref())?;
    match contract.query_creator_ownership(deps.storage)?.owner {
        Some(own) => match info.sender == own {
            true => {}
            false => return Err(ContractError::Unauthorized {}),
        },
        None => return Err(ContractError::Unauthorized {}),
    };

    match address {
        Some(addr) => {
            let validated = deps.api.addr_validate(&addr)?;
            config.extension.whitelist = Some(validated.to_string());
            contract.update_collection_info(
                deps,
                Some(&info),
                &env,
                cw721::msg::CollectionInfoMsg {
                    name: Some(config.name),
                    symbol: Some(config.symbol),
                    extension: config.extension,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "update_whitelist")
                .add_attribute("whitelist", validated.to_string()))
        }
        None => {
            config.extension.whitelist = None;
            contract.update_collection_info(
                deps,
                Some(&info),
                &env,
                cw721::msg::CollectionInfoMsg {
                    name: Some(config.name),
                    symbol: Some(config.symbol),
                    extension: config.extension,
                },
            )?;
            Ok(Response::new()
                .add_attribute("action", "update_whitelist")
                .add_attribute("whitelist", "removed"))
        }
    }
}

pub fn query_whitelist(deps: cosmwasm_std::Deps) -> Result<Option<String>, ContractError> {
    Ok(Cw721SvgContract::default()
        .query_collection_info_and_extension(deps)?
        .extension
        .whitelist)
}

#[cfg(feature = "interface")]
pub use interface::Cw721SvgContractSuite;
#[cfg(feature = "interface")]
mod interface {
    pub use crate::{
        entry::{execute, instantiate, query},
        CONTRACT_NAME as CW721_SVG_CONTRACT_NAME, *,
    };
    use cw_orch::prelude::*;

    #[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = CW721_SVG_CONTRACT_NAME)]
    pub struct Cw721SvgContractSuite;

    impl<Chain: CwEnv> Uploadable for Cw721SvgContractSuite<Chain> {
        /// Return the path to the wasm file corresponding to the contract
        fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
            artifacts_dir_from_workspace!()
                .find_wasm_path_from_crates_label(CW721_SVG_CONTRACT_NAME)
                .unwrap()
        }
        /// Returns a CosmWasm contract wrapper
        fn wrapper() -> Box<dyn MockContract<Empty>> {
            Box::new(ContractWrapper::new_with_empty(execute, instantiate, query))
        }
    }
}
