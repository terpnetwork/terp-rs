pub mod contract;
pub mod scribing;
pub use contract::*;
pub use state::*;

/// Off-chain helper: SVP1-pack a template + variables for `packed`.
pub fn pack_svg(template: &str, variables: &[cw_svg::VariableDef]) -> cosmwasm_std::Binary {
    cw_svg_pack::pack(template, variables).expect("svp pack").into()
}

pub const CONTRACT_NAME: &str = "cw721_svg";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Contract type alias ───────────────────────────────────────────────────────

pub type InstantiateMsg = cw721::msg::Cw721InstantiateMsg<SvgCollectionMetadata>;
pub type ExecuteMsg =
    cw721::msg::Cw721ExecuteMsg<SvgMetadata, SvgCollectionMetadata, SvgExecuteMsgExt>;
pub type QueryMsg = cw721::msg::Cw721QueryMsg<SvgMetadata, SvgCollectionMetadata, SvgQueryMsgExt>;

pub type Cw721SvgContract<'a> = cw721::extension::Cw721Extensions<
    'a,
    SvgMetadata,           // TNftExtension
    SvgMetadata,           // TNftExtensionMsg
    SvgCollectionMetadata, // TCollectionExtension
    SvgCollectionMetadata, // TCollectionExtensionMsg
    SvgExecuteMsgExt,      // TExtensionMsg
    SvgQueryMsgExt,        // TExtensionQueryMsg
    cosmwasm_std::Empty,   // TCustomResponseMsg
>;

// ── Entry points ──────────────────────────────────────────────────────────────

pub mod entry {
    use super::*;
    use crate::contract::{MigrateMsg, SvgTemplateResponse, SvgTokenUriResponse};
    use crate::contract::{
        MAX_SVG_SIZE, MAX_TOTAL_SUPPLY, SVG_COLLECTION, SVG_PACKED_KEY,
    };
    use cosmwasm_std::{
        from_json, to_json_binary, Binary, Deps, DepsMut, Env, Event, MessageInfo, Response,
        StdError, StdResult, Timestamp,
    };
    use cw721::msg::Cw721InstantiateMsg;
    use cw721::traits::{Cw721Execute, Cw721Query};

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    #[inline(never)]
    pub fn instantiate(
        mut deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: InstantiateMsg,
    ) -> Result<Response, ContractError> {
        cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        let mut svg = msg.collection_info_extension.clone();

        if svg.packed.len() > MAX_SVG_SIZE {
            return Err(ContractError::SvgTemplateTooLarge {
                max: MAX_SVG_SIZE,
                got: svg.packed.len(),
            });
        }

        if svg.total > MAX_TOTAL_SUPPLY {
            return Err(ContractError::TotalSupplyTooHigh {
                max: MAX_TOTAL_SUPPLY,
                got: svg.total,
            });
        }
        let creator = msg
            .creator
            .clone()
            .unwrap_or_else(|| info.sender.to_string());

        // Compute mint start time
        let mint_start_time = match svg.mint_start_time {
            Some(secs) => Timestamp::from_seconds(env.block.time.seconds() + secs.seconds()),
            None => env.block.time,
        };
        svg.mint_start_time = Some(mint_start_time);

        // Compute mint end time (cutoff)
        svg.mint_end_time = svg
            .mint_end_time
            .map(|secs| Timestamp::from_seconds(env.block.time.seconds() + secs.seconds()));

        // Resolve payment address (defaults to owner)
        let witdraw_addr = deps
            .api
            .addr_validate(msg.withdraw_address.as_deref().unwrap_or(&creator))?;
        // Validate price tiers: each cutoff must be strictly higher than the previous
        if svg
            .price_tiers
            .windows(2)
            .any(|w| w[1].until_count <= w[0].until_count)
        {
            return Err(ContractError::PricingTierError {});
        };

        // SVP1 only: decode, run every validator on the decoded output, then
        // store the packed bytes once (raw, no JSON/base64).
        let (template, variables) = crate::contract::decode_packed(&svg.packed)?;
        cw_svg::validate_variables(&variables)?;
        let slots = crate::contract::scan_slots(&template, &variables)?;
        cw_svg::validate_template_slots(&template, &variables, &slots)?;
        cw_svg::validate_animated_paint(&template)?;
        cw_svg::links::validate_links(&template, &variables)?;
        crate::contract::scribe_layout(&variables)?;
        deps.storage.set(SVG_PACKED_KEY, svg.packed.as_slice());
        // Config copies (mint config + cw721 collection info) do not repeat
        // the template; read it with the `svg_packed` query.
        svg.packed = Binary::default();
        SVG_COLLECTION.save(deps.storage, &svg)?;
        // Prefer explicit minter (e.g. hm-nft-minter); default to this contract for
        // self-serve public mint flows.
        let minter = msg
            .minter
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| env.contract.address.to_string());

        Cw721SvgContract::default()
            .instantiate(
                deps.branch(),
                &env,
                &info,
                Cw721InstantiateMsg::<SvgCollectionMetadata> {
                    name: msg.name.clone(),
                    symbol: msg.symbol.clone(),
                    minter: Some(minter),
                    creator: Some(creator),
                    collection_info_extension: svg,
                    withdraw_address: Some(witdraw_addr.into_string()),
                },
            )
            .map_err(|e| ContractError::Std(StdError::msg(e.to_string())))?;

        Ok(Response::new()
            .add_attribute("action", "instantiate")
            .add_attribute("contract", env.contract.address.to_string()))
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    #[inline(never)]
    pub fn execute(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: ExecuteMsg,
    ) -> Result<Response, ContractError> {
        match msg {
            ExecuteMsg::UpdateExtension { msg } => match msg {
                SvgExecuteMsgExt::Mint(m) => execute_mint(deps, env, info, m),
                SvgExecuteMsgExt::Pause { pause } => execute_pause(deps, info, pause),
                SvgExecuteMsgExt::UpdateWhitelist { address } => {
                    execute_update_whitelist(deps, info, env, address)
                }
                SvgExecuteMsgExt::ConfigureScribing { epoch_blocks, min_fee, owner_bps, max_proposals, shield_token_id } => {
                    crate::scribing::configure(deps, env, info, epoch_blocks, min_fee, owner_bps, max_proposals, shield_token_id)
                }
                SvgExecuteMsgExt::Propose { token_id, text } => crate::scribing::propose(deps, env, info, token_id, text),
                SvgExecuteMsgExt::Select { token_id, epoch, index } => crate::scribing::select(deps, env, info, token_id, epoch, index),
                SvgExecuteMsgExt::Settle { token_id, epoch } => crate::scribing::settle(deps, env, token_id, epoch),
                SvgExecuteMsgExt::ClaimParticipation { token_id, epoch } => crate::scribing::claim(deps, env, info, token_id, epoch),
                SvgExecuteMsgExt::SetSpeed { token_id, speed_pct } => crate::scribing::set_speed(deps, info, token_id, speed_pct),
            },
            ExecuteMsg::Mint { .. } => Err(ContractError::IncorrectEntrypoint),
            _ => Cw721SvgContract::default()
                .execute(deps, &env, &info, msg)
                .map_err(Into::into),
        }
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    #[inline(never)]
    pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
        match msg {
            QueryMsg::Extension { msg } => match msg {
                SvgQueryMsgExt::SvgTokenUri { token_id } => {
                    let svg = query_svg_token_uri(deps, token_id)
                        .map_err(|e| StdError::msg(e.to_string()))?;
                    to_json_binary(&SvgTokenUriResponse { svg })
                }
                SvgQueryMsgExt::SvgPlaceholder { seed } => {
                    let svg = query_svg_placeholder(deps, seed)
                        .map_err(|e| StdError::msg(e.to_string()))?;
                    to_json_binary(&SvgTokenUriResponse { svg })
                }

                SvgQueryMsgExt::SvgPacked {} => to_json_binary(&Binary::from(
                    deps.storage.get(SVG_PACKED_KEY).unwrap_or_default(),
                )),
                SvgQueryMsgExt::SvgTemplate {} => {
                    let template =
                        query_svg_template(deps).map_err(|e| StdError::msg(e.to_string()))?;
                    to_json_binary(&SvgTemplateResponse { template })
                }
                SvgQueryMsgExt::Whitelist {} => {
                    let wl = query_whitelist(deps).map_err(|e| StdError::msg(e.to_string()))?;
                    to_json_binary(&wl)
                }
                SvgQueryMsgExt::Ownership {} => {
                    to_json_binary(&cw_ownable::get_ownership(deps.storage)?)
                }

                SvgQueryMsgExt::MintCount { address } => {
                    let addr = deps.api.addr_validate(&address)?;
                    let count = mint_count(deps, &addr);
                    to_json_binary(&MintCountResponse { address, count })
                }
                SvgQueryMsgExt::WlMintCount { address } => {
                    let addr = deps.api.addr_validate(&address)?;
                    let count = whitelist_mint_count(deps, &addr);
                    to_json_binary(&MintCountResponse { address, count })
                }
                SvgQueryMsgExt::Scribed { token_id } => to_json_binary(&crate::scribing::MESSAGES.may_load(deps.storage, &token_id)?),
                SvgQueryMsgExt::ScribeEpoch {} => to_json_binary(&crate::scribing::q_epoch(deps, &env)),
                SvgQueryMsgExt::Proposals { token_id, epoch } => to_json_binary(&crate::scribing::q_proposals(deps, token_id, epoch).map_err(|e| StdError::msg(e.to_string()))?),
                SvgQueryMsgExt::EpochPot { token_id, epoch } => to_json_binary(&crate::scribing::q_pot(deps, token_id, epoch).map_err(|e| StdError::msg(e.to_string()))?),
                SvgQueryMsgExt::ScribeHistory { token_id } => to_json_binary(&crate::scribing::q_history(deps, token_id).map_err(|e| StdError::msg(e.to_string()))?),
                SvgQueryMsgExt::Participants { epoch, limit } => to_json_binary(&crate::scribing::q_participants(deps, epoch, limit).map_err(|e| StdError::msg(e.to_string()))?),
                SvgQueryMsgExt::Speed { token_id } => to_json_binary(&crate::scribing::SPEED.may_load(deps.storage, &token_id)?),
                SvgQueryMsgExt::OriginalMinter { token_id } => to_json_binary(&crate::scribing::ORIGIN.may_load(deps.storage, &token_id)?),
                SvgQueryMsgExt::CurrentPriceTier {} => {
                    let c = Cw721SvgContract::default();
                    let mc = c.config.token_count(deps.storage)?;
                    let pt = c
                        .query_collection_info_and_extension(deps)?
                        .extension
                        .price_tiers;

                    to_json_binary(&current_tier_price(&pt, mc))
                }
            },
            other => Cw721SvgContract::default()
                .query(deps, &env, other)
                .map_err(|e| cosmwasm_std::StdError::msg(e.to_string())),
        }
    }

    #[cfg_attr(not(feature = "library"), cosmwasm_std::entry_point)]
    pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> StdResult<Response> {
        let prev_version = cw2::get_contract_version(deps.storage)?;
        if let Some(only) = msg.dregg_ibc_only {
            crate::contract::DREGG_IBC_ONLY.save(deps.storage, &only)?;
        }
        if let Some(addrs) = msg.runtime_minters {
            for a in addrs {
                let addr = deps.api.addr_validate(&a)?;
                crate::contract::DREGG_RUNTIME_MINTERS.save(deps.storage, &addr, &())?;
            }
        }
        cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        let event = Event::new("migrate")
            .add_attribute("from_name", prev_version.contract)
            .add_attribute("from_version", prev_version.version)
            .add_attribute("to_name", CONTRACT_NAME)
            .add_attribute("to_version", CONTRACT_VERSION);
        Ok(Response::new().add_event(event))
    }

    /// Second caller for the message decoders.
    ///
    /// `wasm-opt -Oz` inlines every function with one caller, which folds
    /// `ExecuteMsg` / `QueryMsg` / `InstantiateMsg` decoding into the exports.
    /// After the stack limiter adds a branch per block, that body no longer
    /// fits an arm64 relocation. This export is not a CosmWasm entry point.
    #[cfg(target_arch = "wasm32")]
    #[no_mangle]
    pub extern "C" fn cw721_svg_fn_split() -> usize {
        let bytes: &[u8] = b"{}";
        let execute = from_json::<ExecuteMsg>(bytes).is_err() as usize;
        let query = from_json::<QueryMsg>(bytes).is_err() as usize;
        let instantiate = from_json::<InstantiateMsg>(bytes).is_err() as usize;
        execute + query + instantiate
    }
}

pub mod state {

    use cosmwasm_schema::cw_serde;
    use cosmwasm_std::{
        from_json, to_json_binary, Binary, Deps, Env, MessageInfo, StdError, Timestamp,
    };
    use cw721::{
        error::Cw721ContractError,
        traits::{Contains, Cw721CustomMsg, Cw721State, StateFactory},
        Attribute,
    };
    use cw_svg::TokenParam;

    use crate::PriceTier;
    // ── Trait impls required by Cw721Extensions ──────────────────────────────────

    impl Cw721State for SvgMetadata {}
    impl Cw721CustomMsg for SvgMetadata {}

    impl Contains for SvgMetadata {
        fn contains(&self, _other: &Self) -> bool {
            true
        }
    }

    impl StateFactory<SvgMetadata> for SvgMetadata {
        fn create(
            &self,
            _deps: Deps,
            _env: &Env,
            _info: Option<&MessageInfo>,
            _current: Option<&SvgMetadata>,
        ) -> Result<SvgMetadata, Cw721ContractError> {
            Ok(self.clone())
        }
        fn validate(
            &self,
            _deps: Deps,
            _env: &Env,
            _info: Option<&MessageInfo>,
            _current: Option<&SvgMetadata>,
        ) -> Result<(), Cw721ContractError> {
            Ok(())
        }
    }

    #[cw_serde]
    #[derive(Default)]
    pub struct SvgMetadata {
        pub params: Vec<TokenParam>,
    }

    #[cw_serde]
    pub struct SvgCollectionMetadata {
        pub seed: Binary,
        /// SVP1-packed template + variables (`cw-svg-pack`). Required on create.
        /// Stored once under its own key; empty in stored config / collection info.
        pub packed: Binary,
        /// Price tiers for minting. Empty or None = free mint.
        pub price_tiers: Vec<PriceTier>,
        pub total: u64,
        /// Timestamp of mint start. If less than current block, mints available
        pub mint_start_time: Option<Timestamp>,
        pub mint_end_time: Option<Timestamp>,
        /// Optional merkle whitelist contract address. Whitelisted minters bypass fees.
        pub whitelist: Option<String>,
        /// When true, every mint must include valid ADR-36 `ext` auth
        /// (`CosmosArbitrary` over DST+nonce[|digest]). Required for trustless
        /// pledge collections; default false preserves open-mint SVG drops.
        #[serde(default)]
        pub require_auth: bool,
    }

    impl cw721::traits::Cw721State for SvgCollectionMetadata {}
    impl cw721::traits::Cw721CustomMsg for SvgCollectionMetadata {}
    impl cw721::traits::ToAttributesState for SvgCollectionMetadata {
        fn to_attributes_state(&self) -> Result<Vec<Attribute>, Cw721ContractError> {
            Ok(vec![Attribute {
                key: "metadata".to_string(),
                value: to_json_binary(self)?,
            }])
        }
    }

    impl cw721::traits::FromAttributesState for SvgCollectionMetadata {
        fn from_attributes_state(value: &[Attribute]) -> Result<Self, Cw721ContractError> {
            // Find the metadata attribute
            let metadata_attr = value
                .iter()
                .find(|attr| attr.key == "metadata")
                .ok_or_else(|| {
                    Cw721ContractError::Std(StdError::msg(
                        "Missing 'metadata' attribute".to_string(),
                    ))
                })?;

            from_json(&metadata_attr.value).map_err(Cw721ContractError::Std)
        }
    }

    impl cw721::traits::StateFactory<SvgCollectionMetadata> for SvgCollectionMetadata {
        fn create(
            &self,
            _deps: cosmwasm_std::Deps,
            _env: &cosmwasm_std::Env,
            _info: Option<&cosmwasm_std::MessageInfo>,
            _current: Option<&SvgCollectionMetadata>,
        ) -> Result<SvgCollectionMetadata, Cw721ContractError> {
            Ok(self.clone())
        }

        fn validate(
            &self,
            _deps: cosmwasm_std::Deps,
            _env: &cosmwasm_std::Env,
            _info: Option<&cosmwasm_std::MessageInfo>,
            _current: Option<&SvgCollectionMetadata>,
        ) -> Result<(), Cw721ContractError> {
            // Add your validation logic here
            if self.total == 0 {
                return Err(Cw721ContractError::Std(StdError::msg(
                    "Total supply must be greater than 0".to_string(),
                )));
            }
            Ok(())
        }
    }
}

pub use error::ContractError;
mod error {
    use cosmwasm_std::StdError;
    use cw_ownable::OwnershipError;
    use cw_svg::SvgError;
    use cw_utils::PaymentError;
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum ContractError {
        #[error("{0}")]
        Std(#[from] StdError),

        #[error("{0}")]
        OwnershipError(#[from] OwnershipError),

        #[error("{0}")]
        Svg(#[from] SvgError),

        #[error("{0}")]
        Payment(#[from] PaymentError),

        #[error("{0}")]
        Base(#[from] cw721::error::Cw721ContractError),

        #[error("Minting is paused")]
        MintingPaused {},

        #[error("Please wrap the SvgExecuteMsgExt::Mint inside the default ExecuteMsg::UpdateExtension option.")]
        IncorrectEntrypoint,

        #[error("Minting has not started yet")]
        MintingNotStarted {},

        #[error("Minting has not started yet")]
        PricingTierError {},

        #[error("Cannot mint more than total supply")]
        CannotMintMoreThanTotal {},

        #[error("Unauthorized")]
        Unauthorized {},
        
        #[error("GenericMsg: {m}")]
        GenericMsg {m: String},

        #[error("SVG template exceeds max size of {max} bytes (got {got})")]
        SvgTemplateTooLarge { max: usize, got: usize },

        #[error("Total supply {got} exceeds maximum of {max}")]
        TotalSupplyTooHigh { max: u64, got: u64 },

        #[error("Minting period has ended")]
        MintingEnded {},

        #[error("Incorrect payment: expected {expected}")]
        IncorrectPayment { expected: String },

        #[error("Proof hashes required for whitelist verification")]
        MissingProofHashes {},

        #[error("Address {addr} is not whitelisted")]
        NotWhitelisted { addr: String },

        #[error("No whitelist contract configured")]
        NoWhitelistConfigured {},

        #[error("{e}")]
        General { e: String },

        #[error("Whitelist per-address mint limit exceeded")]
        MaxPerAddressLimitExceeded {},

        #[error("Invalid variable definition: {reason}")]
        InvalidVariableDef { reason: String },

        #[error("Invalid template placeholder: {reason}")]
        InvalidTemplatePlaceholder { reason: String },

        #[error("Sender is not an allowlisted scribe")]
        NotScribe {},

        #[error("Token {token_id} already has a scribed text")]
        AlreadyScribed { token_id: String },

        #[error("Scribe text rejected: {reason}")]
        BadScribeText { reason: String },
    }
}

#[cfg(test)]
mod svp_tests;
#[cfg(test)]
mod scribe_tests;
