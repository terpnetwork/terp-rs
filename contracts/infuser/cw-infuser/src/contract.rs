use crate::error::{AnyOfErr, ContractError};
use crate::msg::{ExecuteMsg, InfusionsResponse, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{
    CONFIG, Config, ELIGIBLE_COLLECTION, INFUSION, INFUSION_ID, MINT_COUNT, MINTABLE_NUM_TOKENS,
    MINTABLE_TOKEN_VECTORS, TokenPositionMapping, UpdatingConfig, WAVS_ADMIN, WAVS_TRACKED,
};
use cosmwasm_schema::serde::Serialize;
use cosmwasm_std::{
    Addr, Attribute, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps, DepsMut, Empty, Env,
    Fraction, HexBinary, MessageInfo, MigrateInfo, QuerierWrapper, QueryRequest, Response,
    StdError, StdResult, Storage, Uint256, WasmMsg, WasmQuery, entry_point, instantiate2_address,
    to_json_binary,
};
use cw2::set_contract_version;

use cw721::EmptyOptionalCollectionExtensionMsg;
use cw721::msg::{Cw721QueryMsg, OwnerOfResponse};
// use cw721_v18::Cw721ExecuteMsg;
use cw_controllers::AdminError;

use cw721::msg::Cw721ExecuteMsg;

use cw_infusions::state::UpdateInfusion;
use cw_infusions::{
    bundles::{AnyOfCount, Bundle, BundleBlend, BundleType},
    nfts::{CollectionInfo, InfusedCollection, NFT, RoyaltyInfoResponse, SgInstantiateMsg},
    state::{EligibleNFTCollection, Infusion, InfusionState},
    wavs::{WavsBundle, WavsMintCountResponse, WavsRecordResponse},
};

use cw_infusions::CompatibleTraits;
// use rand_core::SeedableRng;
use rand_xoshiro::Xoshiro128PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use sha2::{Digest, Sha256};
use shuffle::{fy::FisherYates, shuffler::Shuffler};
use url::Url;

// version info for migration info
pub(crate) const CONTRACT_NAME: &str = "cw-infuser";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    if msg.max_infusions.is_some_and(|f| f > 2u64) {
        return Err(ContractError::MaxInfusionErrror);
    }
    if msg.max_bundles.is_some_and(|f| f > 5u64) {
        return Err(ContractError::MaxBundleError);
    }
    if msg.min_per_bundle.is_some_and(|f| f < 1u64) {
        return Err(ContractError::MaxInfusionErrror);
    }
    if msg.max_per_bundle.is_some_and(|f| f > 10u64) {
        return Err(ContractError::MaxInfusionErrror);
    }
    if msg.owner_fee > Decimal::one() {
        return Err(ContractError::Std(StdError::msg(
            "admin fee incorrect. Must be less than or 100%",
        )));
    }
    if msg
        .min_creation_fee
        .clone()
        .is_some_and(|f| f.amount == Uint256::zero())
    {
        return Err(ContractError::Std(StdError::msg(
            "admin fee incorrect. Must be less than 100%",
        )));
    }

    // admin is either sender or manually set.
    let mut contract_owner = info.sender;
    if let Some(ad) = msg.contract_owner {
        contract_owner = deps.api.addr_validate(&ad)?;
    }
    if let Some(wavs) = msg.wavs_public_key {
        WAVS_ADMIN.save(deps.storage, &wavs)?;
    }

    // get checksum of cw721
    let cw721_checksum = deps.querier.query_wasm_code_info(msg.cw721_code_id)?;
    CONFIG.save(
        deps.storage,
        &Config {
            contract_owner,
            min_per_bundle: msg.min_per_bundle.unwrap_or(1),
            max_per_bundle: msg.max_per_bundle.unwrap_or(10u64),
            code_id: msg.cw721_code_id,
            code_hash: HexBinary::from_hex(&cw721_checksum.checksum.to_hex())?,
            latest_infusion_id: 0,
            max_infusions: msg.max_infusions.unwrap_or(2u64),
            max_bundles: msg.max_bundles.unwrap_or(5),
            owner_fee: msg.owner_fee,
            min_creation_fee: msg.min_creation_fee,
            min_infusion_fee: msg.min_infusion_fee,
            // shuffle_fee: todo!(),
        },
    )?;
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateInfusion { infusions } => {
            execute_create_infusion(deps, info.clone(), env, infusions)
        }
        ExecuteMsg::Infuse { id, bundle } => execute_infuse_bundle(deps, env, info, id, bundle),
        ExecuteMsg::UpdateConfig { config } => update_config(deps, info, config),
        ExecuteMsg::EndInfusion { id } => execute_end_infusion(deps, info, id),
        ExecuteMsg::UpdateInfusionBaseUri { id, base_uri } => {
            update_infused_base_uri(deps, info, id, base_uri)
        }
        ExecuteMsg::UpdateInfusionsEligibleCollectionsOrBundleType {
            id,
            to_add,
            to_remove,
            bundle_type,
        } => update_infusion_eligible_collections(deps, info, id, to_add, to_remove, &bundle_type),
        ExecuteMsg::UpdateInfusionMintFee { id, mint_fee } => {
            update_infusion_mint_fee(deps, info, id, mint_fee)
        }
        ExecuteMsg::Shuffle { id } => execute_shuffle(deps, env, info, id),
        ExecuteMsg::WavsEntryPoint { infusions } => {
            update_wavs_infusion_state(deps, info, infusions)
        }
        ExecuteMsg::UpdateInfusionParams { id, params } => {
            update_infusion_params(deps, info, id, params)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Infusion { addr, id } => to_json_binary(&query_infusion(deps, addr, id)?),
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::InfusionById { id } => to_json_binary(&query_infusion_by_id(deps, id)?),
        QueryMsg::Infusions { addr, index } => to_json_binary(&query_infusions(deps, addr, index)?),
        QueryMsg::WavsRecord { burner, nfts } => {
            to_json_binary(&query_retrieve_wavs_record(deps, burner, nfts)?)
        }
        QueryMsg::InfusionGenetics { id } => to_json_binary(&query_infusion_genetics(deps, id)?),
    }
}

// #[cfg_attr(not(feature = "library"), entry_point)]
// pub fn reply(_deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
//     match msg.id {
//         INFUSION_COLLECTION_INIT_MSG_ID => Ok(Response::new()),
//         _ => Err(ContractError::Unauthorized {}),
//     }
// }

/// Update the infused collection,
fn update_infusion_eligible_collections(
    deps: DepsMut,
    msg: MessageInfo,
    id: u64,
    to_add: Vec<EligibleNFTCollection>,
    to_remove: Vec<Addr>,
    bt: &BundleType,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let key = INFUSION_ID.load(deps.storage, id)?;
    let mut infusion = INFUSION.load(deps.storage, key.clone())?;

    if infusion.owner != msg.sender {
        return Err(ContractError::Admin(AdminError::NotAdmin {}));
    }

    for nft in to_remove {
        // Iterate through the collections to find and remove the NFT
        for i in 0..infusion.collections.len() {
            if infusion.collections[i].addr == nft {
                infusion.collections.remove(i);
                break;
            }
        }
    }
    // ensure new eligible collection params
    if !to_add.is_empty() {
        let collections = validate_eligible_collection_list(
            deps.storage,
            // deps.querier,
            &cfg,
            &infusion.infusion_params.bundle_type,
            &infusion.collections,
            &to_add,
            true,
            id,
        )?;
        infusion.collections = collections;
    }

    match &bt {
        BundleType::AllOf {} => {}
        BundleType::AnyOf { addrs } => {
            let mut unique = vec![];
            for col in infusion.collections.iter() {
                unique.push(col.addr.clone());
            }
            for addr in addrs {
                if !unique.contains(addr) {
                    return Err(ContractError::AnyOfConfigError {
                        err: AnyOfErr::Uneligible,
                    });
                }
            }
        }
        BundleType::AnyOfBlend { blends: _ } => return Err(ContractError::UnImplemented),
    }
    infusion.infusion_params.bundle_type = bt.clone();

    INFUSION.save(deps.storage, key, &infusion)?;
    Ok(Response::new())
}

/// Update the baseuri used for infused collection metadata
fn update_infused_base_uri(
    deps: DepsMut,
    msg: MessageInfo,
    id: u64,
    base_uri: String,
) -> Result<Response, ContractError> {
    let key = INFUSION_ID.load(deps.storage, id)?;
    let mut infusion = INFUSION.load(deps.storage, key.clone())?;
    if infusion.owner != msg.sender {
        return Err(ContractError::Admin(AdminError::NotAdmin {}));
    }
    infusion.infused_collection.base_uri = base_uri;
    INFUSION.save(deps.storage, key, &infusion)?;
    Ok(Response::new())
}

/// Update the mint fee for an infusion
fn update_infusion_mint_fee(
    deps: DepsMut,
    msg: MessageInfo,
    id: u64,
    mint_fee: Option<Coin>,
) -> Result<Response, ContractError> {
    let key = INFUSION_ID.load(deps.storage, id)?;
    let mut infusion = INFUSION.load(deps.storage, key.clone())?;
    if infusion.owner != msg.sender {
        return Err(ContractError::Admin(AdminError::NotAdmin {}));
    }
    infusion.infusion_params.mint_fee = mint_fee;
    INFUSION.save(deps.storage, key, &infusion)?;

    Ok(Response::new())
}

pub fn execute_end_infusion(
    deps: DepsMut,
    info: MessageInfo,
    id: u64,
) -> Result<Response, ContractError> {
    let key = INFUSION_ID.load(deps.storage, id)?;
    let mut infusion = INFUSION.load(deps.storage, key.clone())?;
    if infusion.owner != info.sender {
        return Err(ContractError::Unauthorized {});
    }
    if !infusion.enabled {
        return Err(ContractError::InfusionIsEnded {});
    } else {
        infusion.enabled = false;
        INFUSION.save(deps.storage, key, &infusion)?;
    }

    Ok(Response::new())
}

pub fn execute_create_infusion(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    infusions: Vec<Infusion>,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    let mut msgs = Vec::new();
    let mut fee_msgs: Vec<CosmosMsg<Empty>> = Vec::new();
    let mut attrs = vec![];

    if cfg.max_infusions < infusions.len() as u64 {
        return Err(ContractError::MaxInfusionsError {});
    }

    let collection_checksum = cfg.code_hash.clone();
    let salt1 = generate_instantiate_salt2(
        &collection_checksum,
        env.block.height,
        info.sender.as_bytes(),
    );

    // loop through each infusion
    for infusion in infusions {
        // assert description length
        if infusion.description.is_some_and(|a| a.len() > 512) {
            return Err(ContractError::InfusionDescriptionLengthError {});
        }

        if infusion.infusion_params.params.iter().len() > 4 {
            return Err(ContractError::MetadataArrayLengthError);
        }

        // assert creation fees
        if let Some(creation_fee) = cfg.min_creation_fee.clone() {
            if info.sender == cfg.contract_owner {
                // skip over creation fees for admin
            } else if info.funds.iter().any(|e| e == &creation_fee) {
                let base_fee = CosmosMsg::Bank(BankMsg::Send {
                    to_address: cfg.contract_owner.to_string(),
                    amount: vec![creation_fee],
                });
                fee_msgs.push(base_fee);
            } else {
                return Err(ContractError::RequirednfusionFeeError { fee: creation_fee });
            }
        }

        // assert fees being set
        if let Some(mf) = infusion.infusion_params.mint_fee.clone() {
            if !mf.amount.is_zero() {
                if cfg
                    .min_infusion_fee
                    .clone()
                    .is_none_or(|f| f.amount <= mf.amount)
                {
                } else {
                    return Err(ContractError::InfusionFeeLessThanMinimumRequired {
                        min: cfg
                            .min_infusion_fee
                            .expect("should never be empty if errors"),
                    });
                }
            } else {
                return Err(ContractError::InfusionFeeCannotbeZero);
            }
        }

        // get the global infusion id
        let infusion_id: u64 = cfg.latest_infusion_id + 1;
        cfg.latest_infusion_id = infusion_id;

        validate_eligible_collection_list(
            deps.storage,
            // deps.querier,
            &cfg,
            &infusion.infusion_params.bundle_type,
            &vec![],
            &infusion.collections,
            false,
            infusion_id,
        )?;
        // sanitize base token uri
        let mut base_token_uri = infusion.infused_collection.base_uri.trim().to_string();
        // Token URI must be a valid URL (ipfs, https, etc.)
        let parsed_token_uri =
            Url::parse(&base_token_uri).map_err(|_| ContractError::InvalidBaseTokenURI {})?;
        base_token_uri = parsed_token_uri.to_string();

        // predict the infused collection contract address
        let infusion_addr = match instantiate2_address(
            collection_checksum.as_slice(),
            &deps.api.addr_canonicalize(env.contract.address.as_str())?,
            salt1.as_slice(),
        ) {
            Ok(addr) => addr,
            Err(err) => return Err(ContractError::from(err)),
        };

        let infusion_collection_addr_human = deps.api.addr_humanize(&infusion_addr)?;

        // sets msg sender as infusion admin for infused collection if not specified
        let infusion_admin = infusion
            .infused_collection
            .admin
            .unwrap_or(info.sender.to_string());

        // select if sg or vanilla cw721
        let init_msg = to_json_binary(&SgInstantiateMsg {
            name: infusion.infused_collection.name.clone(),
            symbol: infusion.infused_collection.symbol.clone(),
            minter: env.contract.address.to_string(), // this contract
            collection_info: CollectionInfo::<RoyaltyInfoResponse> {
                creator: infusion_admin.clone(),
                description: "Infused Collection: ".to_owned()
                    + &infusion.infused_collection.description,
                image: infusion.infused_collection.image.clone(),
                external_link: infusion.infused_collection.external_link.clone(),
                explicit_content: infusion.infused_collection.explicit_content,
                start_trading_time: infusion.infused_collection.start_trading_time,
                royalty_info: infusion.infused_collection.royalty_info.clone(),
            },
        })?;

        let init_infusion_msg = WasmMsg::Instantiate2 {
            admin: Some(infusion_admin.clone()),
            code_id: cfg.code_id,
            msg: init_msg,
            funds: vec![],
            label: "Infused without permission".to_string()
                + infusion.infused_collection.name.as_ref()
                + &env.block.height.to_string(),
            salt: salt1.clone(),
        };

        let token_ids = random_token_list(
            &env,
            deps.api.addr_validate(info.sender.as_ref())?,
            (1..=infusion.infused_collection.num_tokens).collect::<Vec<u32>>(),
        )?;

        // Save the updated vector
        MINTABLE_TOKEN_VECTORS.save(deps.storage, infusion_id, &token_ids)?;

        let infusion_config = InfusionState {
            collections: infusion.collections,
            infused_collection: InfusedCollection {
                addr: Some(infusion_collection_addr_human.to_string()),
                admin: Some(infusion_admin),
                base_uri: base_token_uri,
                ..infusion.infused_collection
            },
            infusion_params: infusion.infusion_params,
            payment_recipient: infusion.payment_recipient.unwrap_or(info.sender.clone()),
            owner: infusion.owner.unwrap_or(info.sender.clone()),
            enabled: true,
        };

        // saves the infusion bundle to state with (infused_collection, id)
        let key = (infusion_collection_addr_human.clone(), infusion_id);
        INFUSION.save(deps.storage, key.clone(), &infusion_config)?;
        INFUSION_ID.save(deps.storage, infusion_id, &key)?;
        // contribute to contract randomness
        let mc = MINT_COUNT.load(deps.storage).unwrap_or_default() + 1;
        MINT_COUNT.save(deps.storage, &mc)?;
        MINTABLE_NUM_TOKENS.save(
            deps.storage,
            infusion_collection_addr_human.to_string(),
            &infusion.infused_collection.num_tokens,
        )?;
        CONFIG.save(deps.storage, &cfg)?;

        // map with vector of infusion ids registered for a given NFT collection  addr:

        msgs.push(init_infusion_msg);
        attrs.push(Attribute::new("infusion-id", infusion_id.to_string()));
    }

    Ok(Response::new()
        .add_messages(msgs)
        .add_messages(fee_msgs)
        .add_attributes(attrs))
}

/// Performs various validations on an infusions eligilbe collections being set. If triggered via config update,
/// we validate any possible conficts from existing store values with ones to_add.
fn validate_eligible_collection_list(
    storage: &mut dyn Storage,
    // query: QuerierWrapper,
    cfg: &Config,
    bundle_type: &BundleType,
    existing: &Vec<EligibleNFTCollection>,
    to_add: &Vec<EligibleNFTCollection>,
    updating_config: bool,
    infusion_id: u64,
) -> Result<Vec<EligibleNFTCollection>, ContractError> {
    // checks min_per_bundle
    if cfg.max_bundles < to_add.len() as u64 {
        return Err(ContractError::TooManyCollectionsInInfusion {
            have: to_add.len() as u64,
            max: cfg.max_bundles,
        });
    }

    let mut validate_nfts = to_add.clone();
    if updating_config {
        // prevent duplicates,
        if !to_add.is_empty() {
            let elig = existing;
            let mut elig_to_save = elig.clone();

            for elig in elig_to_save.iter_mut() {
                if let Some(new_elig) = to_add.iter().find(|n| n.addr == elig.addr) {
                    *elig = new_elig.clone();
                }
            }

            for new_elig in to_add {
                if !elig_to_save.iter().any(|e| e.addr == new_elig.addr) {
                    elig_to_save.push(new_elig.clone());
                }
            }

            validate_nfts = elig_to_save;
        }
    }

    // checks for any unique collections
    let mut unique = Vec::new();
    let mut unique_feesub = Vec::new();
    for col in validate_nfts.iter() {
        let addr = &col.addr.to_string();
        if unique.contains(&col.addr) {
            return Err(ContractError::DuplicateCollectionInInfusion);
        }

        // let _res: cw721_v18::ContractInfoResponse = query
        //     .query_wasm_smart(col.addr.clone(), &cw721_v18::Cw721QueryMsg::ContractInfo {})
        //     .map_err(|e| ContractError::Std(e))?;

        // // check if addr is cw721 collection
        // let _res: cw721::msg::CollectionInfoAndExtensionResponse<
        //     Option<cw721::CollectionExtension<RoyaltyInfoResponse>>,
        // > = query
        //     .query_wasm_smart(
        //         col.addr.clone(),
        //         &cw721_base::msg::QueryMsg::GetCollectionInfoAndExtension {},
        //     )
        //     .map_err(|_| ContractError::AddrIsNotNFTCol {
        //         addr: col.addr.to_string(),
        //     })?;

        // checks # of nft required per bundle
        if col.addr.to_string().is_empty() {
            return Err(ContractError::BundleCollectionContractEmpty {});
        }
        if col.min_req < cfg.min_per_bundle || col.min_req > cfg.max_per_bundle
        // || col.max_req.is_some_and(|a| cfg.max_per_bundle < a)
        // || col.max_req.is_some_and(|a| cfg.min_per_bundle > a)
        {
            return Err(ContractError::BadBundle {
                have: col.min_req,
                min: cfg.min_per_bundle,
                max: cfg.max_per_bundle,
            });
        }

        // check feesub tokens are unique
        if let Some(fs) = &col.payment_substitute {
            match bundle_type.strain() {
                1 => {}
                _ => {
                    if unique_feesub.contains(&fs) {
                        return Err(ContractError::DuplicateFeeSubToken {
                            token: fs.denom.clone(),
                        });
                    }
                }
            }
            unique_feesub.push(fs)
        }

        unique.push(col.addr.clone());

        match ELIGIBLE_COLLECTION.may_load(storage, addr)? {
            Some(mut e) => ELIGIBLE_COLLECTION.save(storage, addr, {
                e.push(infusion_id);
                &e
            })?,
            None => ELIGIBLE_COLLECTION.save(storage, addr, &vec![infusion_id])?,
        };
    }

    let unique_len = unique.len();
    match bundle_type {
        BundleType::AnyOf { addrs } => {
            if addrs.is_empty() {
                return Err(ContractError::AnyOfConfigError {
                    err: AnyOfErr::Empty,
                });
            }
            for bc_addr in addrs.iter() {
                if !unique.contains(bc_addr) {
                    return Err(ContractError::AnyOfConfigError {
                        err: AnyOfErr::Uneligible,
                    });
                }
            }
        }
        BundleType::AnyOfBlend { blends } => {
            for blend in blends.iter() {
                let b_len = blend.blend_nfts.len();

                if b_len == 0 || b_len > unique_len {
                    return Err(ContractError::AnyOfConfigError {
                        err: AnyOfErr::Empty,
                    });
                }
                let mut unique_blend = Vec::new();
                for nfts in blend.blend_nfts.iter() {
                    if nfts.min_req == 0 && !nfts.payment_substitute {
                        return Err(ContractError::AnyOfBlendConfigError);
                    }
                    if !unique.contains(&nfts.addr) {
                        return Err(ContractError::DuplicateCollectionInInfusion);
                    }
                    if unique_blend.contains(&nfts.addr) {
                        return Err(ContractError::DuplicateCollectionInInfusion);
                    } else {
                        unique_blend.push(nfts.addr.clone());
                    }
                }
            }
        }
        BundleType::AllOf {} => {}
    }
    Ok(validate_nfts.to_vec())
}

// Infuse bundles. Burns nfts in eligilbe bundles
fn execute_infuse_bundle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    infusion_id: u64,
    bundle: Vec<Bundle>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let key = INFUSION_ID.load(deps.storage, infusion_id)?;
    let infusion = INFUSION.load(deps.storage, key)?;

    if !infusion.enabled {
        return Err(ContractError::InfusionIsEnded {});
    }

    let res = Response::new();
    let sender = info.sender.clone();
    let querier = deps.querier;

    let mut funds = info.funds.clone();
    let mut response_msgs: Vec<CosmosMsg> = Vec::new();
    let mut mc = 0u64;

    // first, any fee parameters are validated
    if let Some(fee) = infusion.infusion_params.mint_fee.clone() {
        if info.sender == infusion.owner {
            // infusion owner omitted from fee payment
        } else {
            let mut fee_error = None;
            funds
                .iter_mut()
                .filter(|a| a.denom == fee.denom)
                .for_each(|a: &mut Coin| {
                    if a.amount < fee.amount {
                        fee_error = Some(ContractError::FeeNotAccepted {
                            have: a.clone(),
                            want: fee.clone(),
                        });
                    } else {
                        a.amount -= fee.amount;
                    }
                });
            if let Some(e) = fee_error {
                return Err(e);
            } else if !funds.is_empty() {
                let fee_msgs = form_feesplit_helper(
                    cfg.owner_fee,
                    cfg.contract_owner.to_string(),
                    infusion.payment_recipient.to_string(),
                    fee,
                )?;
                response_msgs.extend(fee_msgs);
            }
        }
    }

    // // check lens
    if bundle.is_empty() {
        if infusion.infusion_params.wavs_enabled {
            let burn = check_bundles(deps.storage, &cfg, &infusion, &sender, vec![], &funds)?;
            if burn.0.is_empty() {
                return Err(ContractError::EmptyBundle);
            }
            let prep_msgs = prepare_wasm_events(
                env.clone(),
                deps.storage,
                &Addr::unchecked(
                    infusion
                        .infused_collection
                        .addr
                        .clone()
                        .expect("no-infusion-collection"),
                ),
                infusion.infused_collection.base_uri.clone(),
                infusion_id,
                burn.1,
                &sender,
            )?;
            response_msgs.extend(burn.0);
            response_msgs.extend(prep_msgs.0);
            // check if we can satisfy burnt
        } else {
            return Err(ContractError::EmptyBundle);
        }
    }

    // for each nft collection bundle sent to infuse
    for bundle in bundle {
        // assert ownership
        is_nft_owner(querier, sender.clone(), bundle.nfts.clone())?;
        // add each burn nft & mint infused token to response
        let burn = burn_bundle(
            deps.storage,
            env.clone(),
            &cfg,
            bundle.nfts,
            &sender,
            &infusion,
            infusion_id,
            &funds,
        )?;
        // println!("burn: {:#?}", burn);
        response_msgs.extend(burn.0);
        mc += burn.1;
    }
    MINT_COUNT.save(deps.storage, &mc)?;

    Ok(res.add_messages(response_msgs))
}

/// checks all bundles nfts, determines how many nfts to mint,
/// returns msgs to burn, mint nfts, & transfer any fee substitute funds to their destination.
fn burn_bundle(
    storage: &mut dyn Storage,
    env: Env,
    cfg: &Config,
    nfts: Vec<NFT>,
    sender: &Addr,
    infusion: &InfusionState,
    infusion_id: u64,
    funds: &Vec<Coin>,
) -> Result<(Vec<CosmosMsg>, u64), ContractError> {
    let mut msgs: Vec<CosmosMsg> = Vec::new();
    // confirm bundle is in current infusion, and expected amount sent
    let paysub_msg_and_mc = check_bundles(storage, cfg, infusion, sender, nfts.clone(), funds)?;
    // println!("paysub_msg_and_mc: {:#?}", paysub_msg_and_mc);
    // println!("mint_num: {:#?}", mint_num);
    for nft in nfts {
        let burn_msg = cw721::msg::Cw721ExecuteMsg::<
            cw721::EmptyOptionalNftExtensionMsg,
            cw721::EmptyOptionalCollectionExtensionMsg,
            Empty,
        >::Burn {
            token_id: nft.token_id.to_string(),
        };
        msgs.push(into_cosmos_msg(burn_msg, nft.addr, None)?);
    }

    let prep_msgs = prepare_wasm_events(
        env.clone(),
        storage,
        &Addr::unchecked(
            infusion
                .infused_collection
                .addr
                .clone()
                .expect("no-infusion-collection"),
        ),
        infusion.infused_collection.base_uri.clone(),
        infusion_id,
        paysub_msg_and_mc.1,
        sender,
    )?;
    msgs.extend(prep_msgs.0);
    msgs.extend(paysub_msg_and_mc.0);
    // Return the final mint count which has been properly incremented for each token ID
    Ok((msgs, prep_msgs.1))
}

fn prepare_wasm_events(
    env: Env,
    storage: &mut dyn Storage,
    infused_col_addr: &Addr,
    infusion_base_uri: String,
    infusion_id: u64,
    mint_num: u64,
    sender: &Addr,
) -> Result<(Vec<CosmosMsg>, u64), ContractError> {
    let mut msgs: Vec<CosmosMsg> = Vec::new();
    let mut mc = MINT_COUNT.load(storage)?;

    for _ in 0..mint_num {
        mc += 1;
        // increment tokens
        let token_id = random_mintable_token_mapping(
            storage,
            env.clone(),
            sender,
            infusion_id,
            &Addr::unchecked(infused_col_addr.as_ref()),
        )?;

        // mint_msg
        let mint_msg: Cw721ExecuteMsg<_, EmptyOptionalCollectionExtensionMsg, Empty> =
            Cw721ExecuteMsg::Mint {
                token_id: token_id.token_id.to_string(),
                owner: sender.to_string(),
                token_uri: Some(format!(
                    "{}/{}{}",
                    infusion_base_uri.clone(),
                    token_id.token_id,
                    ".json"
                )),
                extension: Some(Empty {}),
            };

        msgs.push(into_cosmos_msg(mint_msg, infused_col_addr.clone(), None)?);
    }
    Ok((msgs, mc))
}

/// Checks all parameters of a bundle, returning the number of infused nfts to mint,
/// along with a list of ComosMsgs (contract fees,etc) to append to the response.
fn check_bundles(
    storage: &mut dyn Storage,
    cfg: &Config,
    infusion: &InfusionState,
    sender: &Addr,
    bundle: Vec<NFT>,
    sent: &Vec<Coin>,
) -> Result<(Vec<CosmosMsg>, u64), ContractError> {
    let mut funds_sent = sent.to_vec();
    let iclen: usize = infusion.collections.len();
    let bundle_type = infusion.infusion_params.bundle_type.clone();
    let btype: i32 = bundle_type.strain();
    let wavs_enabled = infusion.infusion_params.wavs_enabled;
    let mut check_bundle_msgs = Vec::new();
    let mut total_bundle_map: Vec<AnyOfCount> = Vec::with_capacity(iclen);
    let mut fee_sub_map = Vec::new();
    let mut eligible_in_bundle_map = Vec::with_capacity(iclen);
    let mut wavs_satisfy_minimum = 0u64;
    let mut wavs_burn_count = 0u64;
    let mut wavs_overflow = 0u64;
    let mut infused_mint_count = 0u64;

    for eli in &infusion.collections {
        let elig = bundle
            .iter()
            .filter(|b| b.addr == eli.addr)
            .collect::<Vec<_>>();
        let elig_len = elig.len() as u64;
        if elig_len > 0 {
            eligible_in_bundle_map.push((eli.addr).clone());
        }

        if wavs_enabled {
            // count how many are burned from wavs record
            // determine how many times minimum requirement is satisfied, returning count + any overflow
            wavs_burn_count += wavs_burn_count_helper(storage, eli.addr.clone(), sender)?;
            let wmch = wavs_mint_count_helper(wavs_burn_count, eli.min_req)?;
            wavs_satisfy_minimum = wmch.to_mint;
            wavs_overflow = wmch.remaining;
        }

        if elig_len != eli.min_req {
            if let Some(mut ps) = eli.payment_substitute.clone() {
                let (len, remaining_funds) = check_fee_sub(btype, &eli.addr, &ps, &funds_sent)?;
                println!("{:#?}", remaining_funds);
                funds_sent = remaining_funds;
                let len_u64 = len
                    .to_string()
                    .parse::<u64>()
                    .expect("mint len is never > u64 MAX");

                match btype {
                    1 => {
                        if len != Uint256::one() {
                            return Err(ContractError::FeeSubNotProvided {
                                col: eli.addr.to_string(),
                                want: ps.clone(),
                            });
                        }

                        check_bundle_msgs.extend(form_feesplit_helper(
                            cfg.owner_fee,
                            cfg.contract_owner.to_string(),
                            infusion.payment_recipient.to_string(),
                            ps.clone(),
                        )?);
                        fee_sub_map.push(eli.addr.to_string());
                    }
                    2 => {
                        if len_u64 > 0 {
                            let fee = ps.amount * len;
                            ps.amount = fee;
                            let msgmsgs = form_feesplit_helper(
                                cfg.owner_fee,
                                cfg.contract_owner.to_string(),
                                infusion.payment_recipient.to_string(),
                                ps.clone(),
                            )?;
                            // println!("msgmsgs: {:#?}", msgmsgs);
                            check_bundle_msgs.extend(msgmsgs);
                            fee_sub_map.push(eli.addr.to_string());
                            infused_mint_count += len_u64;
                        }
                    }
                    _ => {
                        if len_u64 > 0 {
                            let fee = ps.amount * len;
                            ps.amount = fee;
                            let fee_msgs = form_feesplit_helper(
                                cfg.owner_fee,
                                cfg.contract_owner.to_string(),
                                infusion.payment_recipient.to_string(),
                                ps.clone(),
                            )?;
                            check_bundle_msgs.extend(fee_msgs);
                            infused_mint_count += len_u64;
                            fee_sub_map.push(eli.addr.to_string());
                        }
                    }
                }
            } else if elig.is_empty() && btype == 1 && (!wavs_enabled || wavs_burn_count == 0) {
                return Err(ContractError::BundleCollectionNotEligilbe {
                    bun_type: btype,
                    col: eli.addr.to_string(),
                    wavs: wavs_enabled,
                    min_req: eli.min_req,
                });
            }

            match bundle_type {
                BundleType::AllOf {} => {
                    if !fee_sub_map.contains(&eli.addr.to_string()) {
                        if !wavs_enabled {
                            return Err(ContractError::BundleNotAccepted {
                                have: elig_len,
                                want: eli.min_req,
                                addr: eli.addr.to_string(),
                            });
                        } else if elig_len + wavs_burn_count < eli.min_req {
                            return Err(ContractError::WavsBundleNotAccepted {
                                have: wavs_burn_count,
                                need: eli.min_req,
                            });
                        }
                    }
                }
                BundleType::AnyOf { addrs: ref any_of } => {
                    if !fee_sub_map.contains(&eli.addr.to_string()) {
                        let mc = check_anyof_bundle_helper(
                            any_of.clone(),
                            vec![],
                            vec![AnyOfCount {
                                nft: eli.clone(),
                                count: elig_len + wavs_burn_count,
                            }],
                            &fee_sub_map,
                            funds_sent.to_vec(),
                            wavs_satisfy_minimum,
                        )?;
                        // println!("after check_anyof_bundle_helper mc: {:#?}", mc);
                        infused_mint_count += mc;
                        // save to map outside of elig loop
                        total_bundle_map.push(AnyOfCount {
                            nft: eli.clone(),
                            count: elig_len,
                        });
                    }
                }
                BundleType::AnyOfBlend { ref blends } => {
                    let mc = check_anyof_bundle_helper(
                        vec![],
                        blends.to_vec(),
                        vec![AnyOfCount {
                            nft: eli.clone(),
                            count: elig_len,
                        }],
                        &fee_sub_map,
                        funds_sent.to_vec(),
                        wavs_satisfy_minimum,
                    )?;
                    infused_mint_count += mc;
                    total_bundle_map.push(AnyOfCount {
                        nft: eli.clone(),
                        count: elig_len,
                    });
                }
            }
        } else {
            match bundle_type {
                BundleType::AnyOf { addrs: ref any_of } => {
                    let mc = check_anyof_bundle_helper(
                        any_of.clone(),
                        vec![],
                        vec![AnyOfCount {
                            nft: eli.clone(),
                            count: elig_len,
                        }],
                        &fee_sub_map,
                        sent.to_vec(),
                        wavs_satisfy_minimum,
                    )?;
                    total_bundle_map.push(AnyOfCount {
                        nft: eli.clone(),
                        count: elig_len,
                    });
                    infused_mint_count += mc;
                }
                BundleType::AnyOfBlend { ref blends } => {
                    check_anyof_bundle_helper(
                        vec![],
                        blends.to_vec(),
                        vec![AnyOfCount {
                            nft: eli.clone(),
                            count: elig_len,
                        }],
                        &fee_sub_map,
                        sent.to_vec(),
                        wavs_satisfy_minimum,
                    )?;
                }
                BundleType::AllOf {} => {}
            }
        }

        if wavs_enabled {
            let total = wavs_burn_count + elig_len;
            let used = wavs_satisfy_minimum * eli.min_req;

            if used != 0 && total >= used {
                wavs_overflow = total - used
            }

            WAVS_TRACKED.save(storage, (sender, eli.addr.to_string()), &wavs_overflow)?;
        }
    }
    // prevent any ineligible nft from being included
    for nft in bundle {
        if !eligible_in_bundle_map.contains(&nft.addr) {
            return Err(ContractError::NftIsNotEligible {
                col: nft.addr.to_string(),
            });
        }
    }

    match btype {
        1 => {
            if infused_mint_count == 0 {
                infused_mint_count = 1;
            }
        }
        _ => {
            if infused_mint_count == 0 {
                return Err(ContractError::InvalidAnyOfBundle {});
            }
        }
    }

    if btype == 1 && infused_mint_count == 0 {
        infused_mint_count = 1;
    }

    Ok((check_bundle_msgs, infused_mint_count))
}

/// Returns the # of nfts burned for an eligible collection, tracked by the wavs service.
fn wavs_burn_count_helper(
    storage: &dyn Storage,
    elig_col: Addr,
    sender: &Addr,
) -> Result<u64, ContractError> {
    let mut wavs_burn_count = 0;
    if let Some(count) = WAVS_TRACKED.may_load(storage, (sender, elig_col.to_string()))? {
        wavs_burn_count = count;
    }
    Ok(wavs_burn_count)
}
/// Returns the # of infused nfts to mint given the min_required for this eligible collection
fn wavs_mint_count_helper(
    burn_count: u64,
    min_req: u64,
) -> Result<WavsMintCountResponse, ContractError> {
    let mut wavs_mint_addition = 0u64;
    let mut remaining = 0u64;
    if burn_count != 0u64 {
        if let Some(t) = burn_count.checked_div(min_req) {
            wavs_mint_addition += t
        };
        if wavs_mint_addition != 0u64 {
            // determine the unused burned token count to return as well
            // println!("debug mitn count burn_count:  {:#?}", burn_count);
            remaining = wavs_mint_addition * min_req - burn_count;
        }
        Ok(WavsMintCountResponse {
            to_mint: wavs_mint_addition,
            remaining,
        })
    } else {
        Ok(WavsMintCountResponse {
            to_mint: wavs_mint_addition,
            remaining,
        })
    }
}

/// Update the infused collection
fn update_infusion_params(
    deps: DepsMut,
    info: MessageInfo,
    infusion_id: u64,
    params: UpdateInfusion,
) -> Result<Response, ContractError> {
    let key = INFUSION_ID.load(deps.storage, infusion_id)?;
    let mut infusion = INFUSION.load(deps.storage, key.clone())?;
    if info.sender != infusion.owner {
        return Err(ContractError::Unauthorized);
    }
    if let Some(descr) = params.infusion_description {
        infusion.infused_collection.description = descr;
    }
    if let Some(owner) = params.owner {
        infusion.owner = owner;
    }
    if let Some(pr) = params.payment_recipient {
        infusion.payment_recipient = pr;
    }

    INFUSION.save(deps.storage, key, &infusion)?;

    Ok(Response::new())
}
/// Update the infused collection
fn update_wavs_infusion_state(
    deps: DepsMut,
    info: MessageInfo,
    to_add: Vec<WavsBundle>,
) -> Result<Response, ContractError> {
    let key = WAVS_ADMIN.load(deps.storage)?;
    if key != info.sender.to_string() {
        return Err(ContractError::Admin(AdminError::NotAdmin {}));
    }
    // load infusion & assert there are only eligible collections
    for req in to_add {
        // expect all objects in array to be sorted by ccollection address
        let mut count = req.infused_ids.len() as u64;

        // save map of burned nfts for each collection by token burner
        WAVS_TRACKED.update(
            deps.storage,
            (&deps.api.addr_validate(&req.infuser)?, req.nft_addr),
            |state| match state {
                Some(len) => {
                    count += len;
                    Ok::<u64, ContractError>(count)
                }
                None => Ok(count),
            },
        )?;
    }
    Ok(Response::new())
}

fn check_anyof_bundle_helper(
    anyof_list: Vec<Addr>,
    anyofblend_list: Vec<BundleBlend>,
    elig_count: Vec<AnyOfCount>,
    fee_substituted: &Vec<String>,
    sent: Vec<Coin>,
    wavs_mint_count: u64,
) -> Result<u64, ContractError> {
    let mut mc = 0u64;
    // let mut error = ContractError::UnTriggered;
    // println!("elig_count: {:#?}", elig_count);
    // println!("anyof: {:#?}", anyof_list);
    // println!("sent: {:#?}", sent);
    // println!("fee_substituted: {:#?}", fee_substituted);

    for any in anyof_list.iter() {
        // will iterate only once as we use vec to prevent using option
        for elig in elig_count.iter() {
            //  only occurs once for each entry in both
            if any == elig.nft.addr {
                // determine how many times to increment
                if elig.nft.min_req <= elig.count {
                    let complete_sets = elig.count / elig.nft.min_req;
                    mc += wavs_mint_count + complete_sets;
                    // println!("Eligible NFTs satisfied, incrementing mint count ");
                    // println!("mc : {:#?}", mc);
                    continue;
                }
                // check for accurate fee substitute amount
                if let Some(fps) = &elig.nft.payment_substitute {
                    // first check if any already has been sent
                    let sent = sent.iter().find(|coin| coin.denom == fps.denom);
                    if let Some(pay) = sent {
                        if pay.amount != fps.amount {
                            if !fee_substituted.contains(&any.to_string()) {
                                continue;
                            } else {
                                // println!("feepayment substitute has been provided, increment mint count.");
                                mc += wavs_mint_count + 1;
                                continue;
                            }
                        } else {
                            mc += wavs_mint_count + 1;
                            continue;
                        }
                    }
                }
            }
        }
    }

    // println!("error: {:#?}", error);
    // println!("mc after: {:#?}", mc);

    if anyofblend_list != vec![] {
        //  todo:anyofBlendLogic
        return Err(ContractError::UnImplemented);
    }
    // if error.to_string() != ContractError::UnTriggered.to_string() {
    //     return Err(error);
    // };

    Ok(mc)
}

/// Checks if sent coins contains correct payment substitute for a given elig_addr.\
/// Returns the number of nfts to mint for a given fee substitute.\
/// Bundle types 2 & 3 return 0 if fee-sub not satisfied, bundle type 1 returns error.
fn check_fee_sub(
    btype: i32,
    elig_addr: &Addr,
    ps: &Coin,
    sent: &Vec<Coin>,
) -> Result<(Uint256, Vec<Coin>), ContractError> {
    let mut remaining_funds = sent.to_vec();
    let mut fee_ti = 0usize;
    // Calculate the total number of whole divisions of the sent amounts by the required amount
    let mint_count = sent
        .iter()
        .enumerate()
        .filter(|(_, coin)| coin.denom == ps.denom)
        .map(|(ii, coin)| {
            fee_ti = ii;
            coin.amount / ps.amount
        })
        .sum::<Uint256>();
    println!("feesub_mint_count: {:#?}, bundle_type", mint_count);
    println!("remaining_funds: {:#?}", remaining_funds);

    // calculated mint count may include other eligible collection fee sub payments, if they are using same token denom.
    // if calulated is more than provided, we use the provided, unless provided is more that the max (if there is one).
    // if calculated mint count is less than what has been provided, error for sanity.
    // if mint count is more than expected, set reduce by what should be expected from feesub

    match btype {
        1 => {
            if mint_count == Uint256::zero() {
                // Collect all relevant coins for the error message
                let mut have = Coin::default();
                for coin in sent {
                    if coin.denom == ps.denom {
                        have = coin.clone();
                        break;
                    }
                }
                return Err(ContractError::PaymentSubstituteNotProvided {
                    have: Coin::new(have.amount, ps.denom.to_string()),
                    want: ps.clone(),
                    col: elig_addr.to_string(),
                });
            }
            remaining_funds[fee_ti].amount -= ps.amount;
            Ok((Uint256::one(), remaining_funds))
        }
        _ => {
            if !remaining_funds.is_empty() {
                let fs = ps.amount * mint_count;
                if remaining_funds[fee_ti].amount >= fs {
                    remaining_funds[fee_ti].amount -= fs;
                }
            }
            Ok((mint_count, remaining_funds))
        } // _ => return Err(ContractError::InfusionFeeCannotbeZero {}),
    }
}

/// Update the configuration of the app
fn update_config(
    deps: DepsMut,
    msg: MessageInfo,
    uc: UpdatingConfig,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    // Only the admin should be able to call this
    if config.contract_owner != msg.sender {
        return Err(ContractError::Admin(AdminError::NotAdmin {}));
    }

    if let Some(owner) = uc.contract_owner {
        config.contract_owner = deps.api.addr_validate(&owner)?;
    }

    if let Some(of) = uc.owner_fee {
        config.owner_fee = of;
    }

    if let Some(cf) = uc.min_creation_fee {
        config.min_creation_fee = Some(cf);
    }

    if let Some(mif) = uc.min_infusion_fee {
        config.min_infusion_fee = Some(mif);
    }

    if let Some(mi) = uc.max_infusions {
        config.max_infusions = mi;
    }

    if let Some(mpb) = uc.min_per_bundle {
        config.min_per_bundle = mpb;
    }

    if let Some(mb) = uc.max_bundles {
        config.max_bundles = mb;
    }

    if let Some(ci) = uc.code_id {
        config.code_id = ci;
    }

    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new())
}

pub fn into_cosmos_msg<M: Serialize, T: Into<String>>(
    message: M,
    contract_addr: T,
    funds: Option<Vec<Coin>>,
) -> StdResult<CosmosMsg> {
    let msg = to_json_binary(&message)?;
    let execute = WasmMsg::Execute {
        contract_addr: contract_addr.into(),
        msg,
        funds: funds.unwrap_or_default(),
    };
    Ok(execute.into())
}

pub fn query_infusion_genetics(deps: Deps, id: u64) -> StdResult<Vec<CompatibleTraits>> {
    match INFUSION
        .load(deps.storage, INFUSION_ID.load(deps.storage, id)?)?
        .infusion_params
        .params
    {
        Some(n) => Ok(n.compatible_traits),
        None => Ok(vec![]),
    }
}
pub fn query_retrieve_wavs_record(
    deps: Deps,
    addr: Option<Addr>,
    nfts: Vec<String>,
) -> StdResult<Vec<WavsRecordResponse>> {
    // Limit 10 nfts
    if nfts.len() > 10usize {
        return Err(StdError::msg("try to query less nft at once"));
    }
    // querying count for specific  addr
    if let Some(burn) = addr {
        let responses: Vec<WavsRecordResponse> = nfts
            .into_iter()
            .map(|nft| {
                let count = WAVS_TRACKED
                    .may_load(deps.storage, (&burn.clone(), nft.clone()))
                    .unwrap_or(Some(0));

                WavsRecordResponse {
                    addr: burn.to_string(),
                    count: Some(count.expect("ahhh")),
                }
            })
            .collect();

        Ok(responses)
    } else {
        Ok(nfts
            .into_iter()
            .map(|nft| {
                match ELIGIBLE_COLLECTION
                    .may_load(deps.storage, &nft)
                    .unwrap_or_default()
                {
                    Some(_) => WavsRecordResponse {
                        addr: nft,
                        count: Some(0u64),
                    },
                    None => WavsRecordResponse {
                        addr: nft,
                        count: None,
                    },
                }
            })
            .collect())
    }
}

pub fn query_config(deps: Deps) -> StdResult<Config> {
    let config: Config = CONFIG.load(deps.storage)?;
    Ok(config)
}

pub fn query_infusion(deps: Deps, addr: Addr, id: u64) -> StdResult<InfusionState> {
    let infusion = INFUSION.load(deps.storage, (addr, id))?;
    Ok(infusion)
}
pub fn query_infusion_by_id(deps: Deps, id: u64) -> StdResult<InfusionState> {
    let infuser = INFUSION_ID.load(deps.storage, id)?;
    let infusion = INFUSION.load(deps.storage, infuser)?;
    Ok(infusion)
}

pub fn query_infusions(deps: Deps, addr: Addr, index: u64) -> StdResult<InfusionsResponse> {
    let mut infusions = vec![];

    for i in index..=index + 30 {
        let id = i;
        // return the response for each
        let state = INFUSION.load(deps.storage, (addr.clone(), id))?;
        infusions.push(state);
    }

    Ok(InfusionsResponse { infusions })
}

pub fn query_if_is_in_bundle(deps: Deps, addr: Addr, id: u64) -> StdResult<bool> {
    let key = INFUSION_ID.load(deps.storage, id)?;
    Ok(INFUSION
        .load(deps.storage, key)?
        .collections
        .iter()
        .any(|a| a.addr == addr))
}

/// verifies all nfts defined in bundle are of ownership of the current sender
pub fn is_nft_owner(
    querier: QuerierWrapper,
    sender: Addr,
    nfts: Vec<NFT>,
) -> Result<(), ContractError> {
    for nft in nfts {
        let nft_address = nft.addr;
        let token_id = nft.token_id;

        let owner_response: OwnerOfResponse =
            querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: nft_address.to_string(),
                msg: to_json_binary(&Cw721QueryMsg::<
                    cw721::EmptyOptionalNftExtension,
                    cw721::EmptyOptionalCollectionExtension,
                    Empty,
                >::OwnerOf {
                    token_id: token_id.to_string(),
                    include_expired: None,
                })?,
            }))?;

        if owner_response.owner != sender.to_string() {
            return Err(ContractError::SenderNotOwner {});
        }
    }
    Ok(())
}
/// Generates the value used with instantiate2, via a hash of the infusers checksum.
pub fn generate_instantiate_salt2(checksum: &HexBinary, height: u64, sender: &[u8]) -> Binary {
    let mut hash = Vec::new();
    hash.extend_from_slice(checksum.as_slice());
    hash.extend_from_slice(&height.to_be_bytes());
    hash.extend_from_slice(sender);
    let checksum_hash = <sha2::Sha256 as sha2::Digest>::digest(hash);
    Binary::new(checksum_hash.to_vec())
}

pub fn random_token_list(
    env: &Env,
    sender: Addr,
    mut tokens: Vec<u32>,
) -> Result<Vec<u32>, ContractError> {
    let tx_index = if let Some(tx) = &env.transaction {
        tx.index
    } else {
        0
    };
    let sha256 = Sha256::digest(
        format!("{}{}{}{}", sender, env.block.height, tokens.len(), tx_index).into_bytes(),
    );
    // Cut first 16 bytes from 32 byte value
    let randomness: [u8; 16] = sha256.to_vec()[0..16].try_into().unwrap();
    let mut rng = Xoshiro128PlusPlus::from_seed(randomness);
    let mut shuffler = FisherYates::default();
    shuffler
        .shuffle(&mut tokens, &mut rng)
        .map_err(StdError::msg)?;
    Ok(tokens)
}

// Does a baby shuffle, picking a token_id from the first or last 50 mintable positions.
fn random_mintable_token_mapping(
    storage: &mut dyn Storage,
    env: Env,
    sender: &Addr,
    infusion_id: u64,
    infused_col_addr: &Addr,
) -> Result<TokenPositionMapping, ContractError> {
    let mintable_num_tokens = MINTABLE_NUM_TOKENS.load(storage, infused_col_addr.to_string())?;
    if mintable_num_tokens == 0 {
        return Err(ContractError::SoldOut {});
    }
    let tx_index = if let Some(tx) = &env.transaction {
        tx.index
    } else {
        0
    };
    //generate unique SHA256 sum
    let sha256 = Sha256::digest(
        format!(
            "{}{}{}{}",
            env.block.height, tx_index, sender, mintable_num_tokens
        )
        .into_bytes(),
    );

    let randomness: [u8; 32] = sha256.into();
    let mut rem = 50u32;
    if rem > mintable_num_tokens {
        rem = mintable_num_tokens;
    }
    let n = u32::from_be_bytes(randomness[..4].try_into().unwrap()) % rem;
    let mut infusion_positions = MINTABLE_TOKEN_VECTORS.load(storage, infusion_id)?;
    let token_id = infusion_positions[n as usize];

    infusion_positions.remove(n as usize);

    MINTABLE_TOKEN_VECTORS.save(storage, infusion_id, &infusion_positions)?;

    MINTABLE_NUM_TOKENS.save(
        storage,
        infused_col_addr.to_string(),
        &(mintable_num_tokens - 1),
    )?;

    Ok(TokenPositionMapping {
        position: n,
        token_id,
    })
}

/// Creates the msgs that split any fees between the contract owner and an infusion owner, if configured.
fn form_feesplit_helper(
    owner_fee: Decimal,
    owner: String,
    payment_recipient: String,
    fee: Coin,
) -> Result<Vec<CosmosMsg>, ContractError> {
    let mut msgs = vec![];
    // split fees between contract owner and infusion owner
    let dev_fee = fee
        .amount
        .checked_multiply_ratio(owner_fee.numerator(), owner_fee.denominator())?;

    let dec = Decimal::one() - owner_fee;
    let remaining_fee_amount = fee
        .amount
        .checked_multiply_ratio(dec.numerator(), dec.denominator())?;

    if dev_fee != Uint256::zero() {
        let base_fee = CosmosMsg::Bank(BankMsg::Send {
            to_address: owner,
            amount: vec![Coin::new(dev_fee, fee.denom.clone())],
        });
        msgs.push(base_fee);
    }

    // remaining fee to infusion owner
    let fee_msg = CosmosMsg::Bank(BankMsg::Send {
        to_address: payment_recipient.to_string(),
        amount: vec![Coin::new(remaining_fee_amount, fee.denom.clone())],
    });

    msgs.push(fee_msg);
    Ok(msgs)
}

// source: https://github.com/public-awesome/launchpad/blob/main/contracts/minters/token-merge-minter/src/contract.rs#L338
// Anyone can pay to shuffle at any time
// Introduces another source of randomness to minting
// There's a fee because this action is expensive.
pub fn execute_shuffle(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    inf_id: u64,
) -> Result<Response, ContractError> {
    let res = Response::new();

    let inf_col_addr = INFUSION_ID.load(deps.storage, inf_id)?.0;

    // Check not sold out
    let mintable_num_tokens = MINTABLE_NUM_TOKENS.load(deps.storage, inf_col_addr.to_string())?;
    if mintable_num_tokens == 0 {
        return Err(ContractError::SoldOut {});
    }

    // get positions and token_ids, then randomize token_ids and reassign positions
    let mut positions = vec![];
    let mut token_ids = vec![];

    let mapping = MINTABLE_TOKEN_VECTORS.load(deps.storage, inf_id)?;

    mapping.iter().enumerate().for_each(|(pos, token_id)| {
        positions.push(pos);
        token_ids.push(*token_id);
    });

    let randomized_token_ids = random_token_list(&env, info.sender.clone(), token_ids.clone())?;
    MINTABLE_TOKEN_VECTORS.save(deps.storage, inf_id, &randomized_token_ids)?;

    Ok(res
        .add_attribute("action", "shuffle")
        .add_attribute("sender", info.sender))
}

// //  source: https://github.com/public-awesome/launchpad/blob/main/contracts/minters/vending-minter/src/contract.rs#L1371
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(
    deps: DepsMut,
    _env: Env,
    _msg: MigrateMsg,
    _info: MigrateInfo,
) -> StdResult<Response> {
    let _prev_version = cw2::get_contract_version(deps.storage)?;
    // if prev_version.contract != CONTRACT_NAME {
    //     return Err(StdError::msg(
    //         "Cannot upgrade to a different contract",
    //     ));
    // }

    let res = Response::new();
    // let version: Version = prev_version
    //     .version
    //     .parse()
    //     .map_err(|_| StdError::msg("Invalid current contract version"))?;
    // let new_version: Version = CONTRACT_VERSION
    //     .parse()
    //     .map_err(|_| StdError::msg("Invalid new contract version"))?;

    // if version > new_version {
    //     return Err(StdError::msg(
    //         "Cannot upgrade to a previous contract version",
    //     ));
    // }

    // #[allow(clippy::cmp_owned)]
    // if prev_version.version < "0.6.0".to_string() {
    //     // v050_patch_upgrade(deps.storage).map_err(|e| StdError::msg(e.to_string()))?;
    // }
    // let to_address = CONFIG.load(deps.storage)?.contract_owner.to_string();
    // let amount = deps
    //     .querier
    //     .query_all_balances(env.contract.address.to_string())?;

    // // set new contract version
    // let event = Event::new("migrate")
    //     .add_attribute("from_name", prev_version.contract)
    //     .add_attribute("from_version", prev_version.version)
    //     .add_attribute("to_name", CONTRACT_NAME)
    //     .add_attribute("to_version", CONTRACT_VERSION);

    Ok(res)
}

// #[cfg(test)]
// mod tests {
//     use cosmwasm_std::testing::{mock_dependencies, mock_env};

//     use super::*;
//     use std::str::FromStr;

//     fn mint_sim(
//         mut env: Env,
//         storage: &mut dyn Storage,
//         inf_col_addr: &Addr,
//         sender: &Addr,
//         mc: u64,
//         token_id: u64,
//         infusion_id: u64,
//     ) -> TokenPositionMapping {
//         env.block.height += 1;

//         random_mintable_token_mapping(storage, env.clone(), &sender, mc, inf_col_addr).unwrap()
//     }

//     #[test]
//     fn test_validate_token_selecta_aka_random_token_list() {
//         let mut binding = mock_dependencies();
//         let deps = binding.as_mut();
//         let api = deps.api.clone();
//         let info = mock_info("sender", &[]);
//         let mut env = mock_env();
//         let inf_col_addr_1 = Addr::unchecked("cosmos1abc");
//         let inf_col_addr_2 = Addr::unchecked("cosmos1zya");

//         let token_ids1 =
//             random_token_list(&env, info.sender.clone(), (1..=666).collect::<Vec<u32>>()).unwrap();
//         env.block.height += 1;
//         let token_ids2 =
//             random_token_list(&env, info.sender.clone(), (1..=100).collect::<Vec<u32>>()).unwrap();

//         // Save the updated vector
//         MINTABLE_TOKEN_VECTORS
//             .save(deps.storage, 1, &token_ids1)
//             .unwrap();
//         // Save the updated vector
//         MINTABLE_TOKEN_VECTORS
//             .save(deps.storage, 2, &token_ids2)
//             .unwrap();

//         MINTABLE_NUM_TOKENS
//             .save(deps.storage, inf_col_addr_1.to_string(), &666)
//             .unwrap();
//         MINTABLE_NUM_TOKENS
//             .save(deps.storage, inf_col_addr_2.to_string(), &100)
//             .unwrap();

//         let mut found1 = vec![];
//         let mut found2 = vec![];
//         for id in &token_ids1 {
//             if found1.contains(&id) {
//                 panic!("ahhh")
//             } else {
//                 found1.push(id);
//             }
//         }
//         for id in &token_ids2 {
//             if found2.contains(&id) {
//                 panic!("ahhh")
//             } else {
//                 found2.push(id);
//             }
//         }
//         let mut sim_mint_count = 2;
//         for i in 1..token_ids1.len() {
//             // ensure we do not have token id collisions
//             let tpm = mint_sim(
//                 env.clone(),
//                 deps.storage,
//                 &inf_col_addr_1,
//                 &info.sender,
//                 sim_mint_count + 1,
//                 token_ids1[i].into(),
//                 1,
//             );
//         }
//         for i in 1..token_ids2.len() {
//             // ensure we do not have token id collisions
//             let tpm = mint_sim(
//                 env.clone(),
//                 deps.storage,
//                 &inf_col_addr_2,
//                 &info.sender,
//                 sim_mint_count,
//                 token_ids2[i].into(),
//                 2,
//             );
//             println!(
//                 "tpm.position: {:#?}, tpm.token_id:{:#?} ",
//                 tpm.position, tpm.token_id
//             );
//             sim_mint_count += 1;
//         }
//     }

//     #[test]
//     fn test_form_feesplit_helper() {
//         let owner_fee = Decimal::from_str("0.1").unwrap(); // 10% fee for owner
//         let owner = addr!("owner");
//         let payment_recipient = addr!("recipient");
//         let fee = Coin {
//             denom: String::from("uthiol"),
//             amount: Uint256::from(1000u128), //
//         };

//         let result = form_feesplit_helper(
//             owner_fee,
//             owner.to_string(),
//             payment_recipient.to_string(),
//             fee,
//         )
//         .expect("Should not return error");

//         // 2 msg: one for  devs and one for fee recipient
//         assert_eq!(result.len(), 2);

//         // First message should send 300 uthiol to owner (30% of 1000)
//         let dev_fee_msg = &result[0];
//         match dev_fee_msg {
//             CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
//                 assert_eq!(to_address, &owner.to_string());
//                 assert_eq!(amount[0].denom, "uthiol");
//                 assert_eq!(amount[0].amount, Uint256::from(100u128));
//             }
//             _ => panic!("First message should be a Bank Send message"),
//         }

//         // Second message should send 700 uiuthiolnf to recipient (70% of 1000)
//         let fee_msg = &result[1];
//         match fee_msg {
//             CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
//                 assert_eq!(to_address, &payment_recipient.to_string());
//                 assert_eq!(amount[0].denom, "uthiol");
//                 assert_eq!(amount[0].amount, Uint256::from(900u128));
//             }
//             _ => panic!("Second message should be a Bank Send message"),
//         }
//     }

//     // 47KB for 10k
//     #[test]
//     fn verify_storage_size() {
//         let mut deps = mock_dependencies();
//         let data: Vec<u32> = (1..=10000).collect();

//         // Store the data
//         MINTABLE_TOKEN_VECTORS
//             .save(&mut deps.storage, 1, &data)
//             .unwrap();

//         // Load raw bytes
//         let key = MINTABLE_TOKEN_VECTORS.key(1);
//         let raw_bytes = deps.storage.get(&key).unwrap();

//         assert_eq!(raw_bytes.len(), 48895);

//         // Passes
//     }

//     #[test]
//     fn test_unique_token_ids_in_bundle() {
//         let mut binding = mock_dependencies();
//         let infuser = binding.api.addr_make("eretskeret");
//         let sender = addr!("sender");
//         let deps = binding.as_mut();
//         let info = mock_info(sender, &[]);
//         let mut env = mock_env();

//         let infused_collection_addr = Addr::unchecked(addr!("cosmos1abc"));
//         let sender1 = Addr::unchecked(addr!("cosmosender1s1abc"));
//         let sender2 = Addr::unchecked(addr!("sender2"));

//         // Set up a small set of token IDs (1-10)
//         let token_ids =
//             random_token_list(&env, info.sender.clone(), (1..=1000).collect::<Vec<u32>>()).unwrap();
//         MINTABLE_TOKEN_VECTORS
//             .save(deps.storage, 1, &token_ids)
//             .unwrap();
//         MINTABLE_NUM_TOKENS
//             .save(deps.storage, infused_collection_addr.to_string(), &1000)
//             .unwrap();

//         // Initialize mint count
//         MINT_COUNT.save(deps.storage, &0u64).unwrap();

//         // Simulate multiple mints in the same bundle

//         let mut selected_tokens = Vec::new();

//         // Try to mint 5 tokens (half of our supply)
//         for i in 0..1000 {
//             env.block.height += 2;
//             let mut sender = sender1.clone();
//             if i % 3 == 0 {
//                 sender = sender2.clone()
//             }
//             let token_mapping = random_mintable_token_mapping(
//                 deps.storage,
//                 env.clone(),
//                 &sender,
//                 1,
//                 &infused_collection_addr,
//             )
//             .unwrap();

//             // Make sure we don't get a duplicate token ID
//             assert!(
//                 !selected_tokens.contains(&token_mapping.token_id),
//                 "Duplicate token ID found: {}, iteration: {}",
//                 token_mapping.token_id,
//                 i
//             );

//             selected_tokens.push(token_mapping.token_id);
//         }

//         // Ensure we got 100 unique tokens
//         assert_eq!(selected_tokens.len(), 1000);
//     }

//     #[test]
//     fn test_update_wavs_infusion_state() {
//         let mut deps = mock_dependencies();
//         let admin = addr!("admin");
//         let non_admin = addr!("non_admin");
//         let infuser1 = addr!("infuser1");
//         let infuser2 = addr!("infuser2");
//         let nft_addr1 = addr!("nft_addr1");
//         let nft_addr2 = addr!("nft_addr2");

//         // Set admin
//         WAVS_ADMIN
//             .save(deps.as_mut().storage, &admin.to_string())
//             .unwrap();

//         // Test with non-admin
//         let info = mock_info(non_admin, &[]);
//         let to_add = vec![WavsBundle {
//             infuser: infuser1.to_string(),
//             nft_addr: nft_addr1.to_string(),
//             infused_ids: vec![1.to_string(), 2.to_string(), 3.to_string()],
//         }];
//         let result = update_wavs_infusion_state(deps.as_mut(), info, to_add.clone()).unwrap_err();
//         assert_eq!(
//             result.to_string(),
//             ContractError::Admin(AdminError::NotAdmin {}).to_string()
//         );

//         // Test with admin
//         let info = mock_info(admin, &[]);
//         let result = update_wavs_infusion_state(deps.as_mut(), info, to_add.clone());
//         println!("{:#?}", result);
//         assert!(result.is_ok());

//         // Check if data is saved correctly
//         let stored_count = WAVS_TRACKED
//             .load(
//                 deps.as_ref().storage,
//                 (&Addr::unchecked(infuser1), nft_addr1.to_string()),
//             )
//             .unwrap();
//         assert_eq!(stored_count, 3);

//         // Test with multiple bundles
//         let to_add = vec![
//             WavsBundle {
//                 infuser: infuser1.to_string(),
//                 nft_addr: nft_addr1.to_string(),
//                 infused_ids: vec![4.to_string(), 5.to_string()],
//             },
//             WavsBundle {
//                 infuser: infuser2.to_string(),
//                 nft_addr: nft_addr2.to_string(),
//                 infused_ids: vec![1.to_string(), 2.to_string(), 3.to_string()],
//             },
//         ];
//         let info = mock_info(admin, &[]);
//         let result = update_wavs_infusion_state(deps.as_mut(), info, to_add.clone());
//         assert!(result.is_ok());

//         // Check if data is saved correctly
//         let stored_count = WAVS_TRACKED
//             .load(
//                 deps.as_ref().storage,
//                 (&Addr::unchecked(infuser1), nft_addr1.to_string()),
//             )
//             .unwrap();
//         assert_eq!(stored_count, 5); // 3 + 2

//         let stored_count = WAVS_TRACKED
//             .load(
//                 deps.as_ref().storage,
//                 (&Addr::unchecked(infuser2), nft_addr2.to_string()),
//             )
//             .unwrap();
//         assert_eq!(stored_count, 3);
//     }
// }
