use cw721::msg::Cw721ExecuteMsg;
use cw721::DefaultOptionalCollectionExtensionMsg;

use crate::{
    error::ContractError,
    msg::Terp721AccountsQueryMsg,
    state::{SudoParams, ACCOUNT_MANIFOLD, REVERSE_MAP, REVERSE_MAP_KEY, SUDO_PARAMS, VERIFIER},
    Terp721AccountContract,
};
use cosmwasm_std::{
    ensure, Addr, Binary, CanonicalAddr, ContractInfoResponse, Deps, DepsMut, Empty, Env, Event,
    MessageInfo, Response, StdError, StdResult,
};
use cw_ownable::Ownership;
use cw_utils::nonpayable;

use terp_account::{
    verify_generic::CosmosArbitrary, Metadata, PendingBid, TextRecord, MAX_TEXT_LENGTH, NFT,
};

pub mod manifest {
    #[cfg(feature = "abstract")]
    use abstract_std::objects::{
        gov_type::GovernanceDetails,
        ownership::{self, Ownership},
    };
    use cosmwasm_std::{to_json_binary, Attribute, CosmosMsg, SubMsg, WasmMsg};
    use cw721::state::NftInfo;
    use cw721::traits::Cw721Execute;
    use cw721::Expiration;

    use crate::state::{REVERSE_MAP_KEY, REVMAP_LIMIT};

    use super::*;

    pub fn associate_address(
        deps: DepsMut,
        info: MessageInfo,
        oca: Addr,
        account: String,
        address: Option<String>,
    ) -> Result<Response, ContractError> {
        only_owner(deps.as_ref(), &info.sender, &account)?;
        // prevent any changes if account is in cooldown
        let market = &ACCOUNT_MANIFOLD.load(deps.storage)?;
        ensure_not_in_cooldown(deps.as_ref(), &market, &account)?;

        #[cfg(feature = "abstract")]
        let mut ownership: Ownership<String> = Ownership {
            owner: ownership::GovernanceDetails::Renounced {},
            pending_owner: None,
            pending_expiry: None,
        };
        // println!("// 1. remove old token_uri from reverse map if it exists");
        Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, &account)
            .and_then(|prev_token_info| {
                if let Some(address) = prev_token_info.token_uri {
                    if !prev_token_info.extension.account_ownership {
                        REVERSE_MAP.remove(deps.storage, &Addr::unchecked(address));
                        Ok(())
                    } else {
                        #[cfg(not(feature = "abstract"))]
                        REVERSE_MAP.remove(deps.storage, &Addr::unchecked(address));
                        #[cfg(feature = "abstract")]
                        {
                            // token_uri has been set to the abstract-account address. Query who it has set as its current owner.
                            let current_owner: Ownership<String> = deps.querier.query_wasm_smart(
                                &address,
                                &abstract_std::account::QueryMsg::Ownership {},
                            )?;
                            // if it has the same nft & token-id set as its owner,
                            // we must also have this address associated to the account in our reverse map,
                            // so we musst remove.
                            match current_owner.owner.clone() {
                                ownership::GovernanceDetails::NFT {
                                    collection_addr,
                                    token_id,
                                } => {
                                    if account != token_id || collection_addr != oca.to_string() {
                                        // removes mapping
                                        REVERSE_MAP.remove(deps.storage, &Addr::unchecked(address));
                                    } else {
                                        // keeps mapping
                                        ownership = current_owner
                                    }
                                }

                                _ => {
                                    // removes mapping
                                    REVERSE_MAP.remove(deps.storage, &Addr::unchecked(address));
                                }
                            }
                        }

                        Ok(())
                    }
                } else {
                    Ok(())
                }
            })
            .unwrap_or(());

        // println!("// 2. validate the new address, prepare to save into token_uri")
        let token_uri = address
            .clone()
            .map(|address| {
                #[cfg(feature = "abstract")]
                match ownership.owner.clone() {
                    ownership::GovernanceDetails::NFT {
                        collection_addr,
                        token_id,
                    } => {
                        if account != token_id || collection_addr != oca.to_string() {
                            return Err(ContractError::IncorrectBillboardToken {
                                got: ownership.owner.to_string(),
                                wanted: ownership::GovernanceDetails::NFT {
                                    collection_addr,
                                    token_id,
                                }
                                .to_string(),
                            });
                        }
                        Ok(address)
                    }
                    _ => {
                        let addr = deps.api.addr_validate(&address)?;
                        Ok(validate_address(deps.as_ref(), &info.sender, addr).map(|_| address)?)
                    }
                }
                #[cfg(not(feature = "abstract"))]
                {
                    let addr = deps.api.addr_validate(&address)?;
                    Ok::<std::string::String, ContractError>(
                        validate_address(deps.as_ref(), &info.sender, addr).map(|_| address)?,
                    )
                }
            })
            .transpose()?;

        // println!("// 3. look up prev account if it exists for the new address");
        let old_account = token_uri.clone().and_then(|addr: String| {
            REVERSE_MAP
                .may_load(deps.storage, &Addr::unchecked(addr))
                .unwrap_or(None)
        });

        // println!("// 4. remove old token_uri / address from previous account");
        old_account.map(|token_id| {
            Terp721AccountContract::default()
                .config
                .nft_info
                .update(deps.storage, &token_id, |t| match t {
                    Some(mut ti) => {
                        ti.token_uri = None;
                        Ok(ti)
                    }
                    None => Err(ContractError::AccountNotFound {}),
                })
        });

        // println!("// 5. associate new token_uri / address with new account / token_id");
        Terp721AccountContract::default()
            .config
            .nft_info
            .update(deps.storage, &account, |t| match t {
                Some(mut token_info) => {
                    token_info.token_uri = token_uri.clone().map(|addr| addr.to_string());
                    // save ownership metadata if set
                    #[cfg(not(feature = "abstract"))]
                    {
                        token_info.extension.account_ownership = false;
                    }
                    #[cfg(feature = "abstract")]
                    {
                        match ownership.owner {
                            GovernanceDetails::NFT { .. } => {
                                token_info.extension.account_ownership = true
                            }
                            _ => token_info.extension.account_ownership = false,
                        }
                    }

                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            })?;

        // println!("// 6. update new manager in token metadata");
        let canonv = deps.api.addr_canonicalize(info.sender.as_str())?;
        REVMAP_LIMIT.save(deps.storage, &canonv.to_string(), &0u32)?;
        token_uri.map(|addr| REVERSE_MAP.save(deps.storage, &Addr::unchecked(addr), &account));

        let mut event = Event::new("associate-address")
            .add_attribute("account", &account)
            .add_attribute("owner", info.sender);

        if let Some(address) = address {
            event = event.add_attribute("address", address);
        }

        // remove bids since changes were made and may create differences between what is being bid on
        Ok(Response::new()
            .add_event(event)
            .add_message(WasmMsg::Execute {
                contract_addr: market.to_string(),
                msg: to_json_binary(&terp_account::manifold::ExecuteMsg::RemoveBids {
                    token_id: account,
                })?,
                funds: vec![],
            }))
    }

    pub fn execute_approve_all_via_market(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        owner: String,
        expires: Option<Expiration>,
    ) -> Result<Response, ContractError> {
        let market = ACCOUNT_MANIFOLD.load(deps.storage)?;
        if &market != &info.sender {
            return Err(ContractError::UnauthorizedCreatorOrAdmin {});
        }

        // reject expired data as invalid
        let expires = expires.unwrap_or_default();
        if expires.is_expired(&env.block) {
            return Err(ContractError::Std(StdError::msg("unauthorized")));
        }

        // set the operator for us
        let owner_addr = deps.api.addr_validate(&owner)?;

        Terp721AccountContract::default().config.operators.save(
            deps.storage,
            (&owner_addr, &market),
            &expires,
        )?;

        Ok(Response::new()
            .add_attribute("action", "approve_all")
            .add_attribute("operator", market.to_string()))
    }

    /// Update Abstract Account support for a token.
    /// Enables, updates, or disables Abstract Account linkage safely.
    pub fn execute_update_abstract_account_support(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        account: &String,
        abs_account: Option<String>,
    ) -> Result<Response, ContractError> {
        only_owner(deps.as_ref(), &info.sender, account)?;
        let mut remove_bids_trigger = Vec::with_capacity(1);
        let mut token = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?;

        if let Some(_cooldown) = deps.querier.query_wasm_smart::<Option<PendingBid>>(
            ACCOUNT_MANIFOLD.load(deps.storage)?.to_string(),
            &terp_account::manifold::QueryMsg::Cooldown {
                token_id: account.to_string(),
            },
        )? {
            return Err(ContractError::AccountIsInCooldownStage {});
        }

        match (token.extension.account_ownership, abs_account) {
            // Already enabled: updating
            (true, Some(new_aa)) => {
                remove_bids_trigger.push(());
                #[cfg(feature = "abstract")]
                // New AA must point to this token
                terp_account::validate_aa_ownership(
                    deps.as_ref(),
                    &new_aa,
                    account,
                    &env.contract.address,
                    true, // must be in use
                )?;

                // Old AA must NOT still point to this token
                if let Some(old_aa) = &token.token_uri {
                    #[cfg(feature = "abstract")]
                    terp_account::validate_aa_ownership(
                        deps.as_ref(),
                        old_aa,
                        account,
                        &env.contract.address,
                        false, // must not be in use
                    )?;
                    REVERSE_MAP.remove(deps.storage, &Addr::unchecked(old_aa));
                }

                REVERSE_MAP.save(deps.storage, &Addr::unchecked(&new_aa), account)?;
                token.token_uri = Some(new_aa);
            }

            (true, None) => {
                // Disable: ensure current AA isn't still using this token
                remove_bids_trigger.push(());
                if let Some(old_aa) = &token.token_uri {
                    #[cfg(feature = "abstract")]
                    validate_aa_ownership(
                        deps.as_ref(),
                        old_aa,
                        account,
                        &env.contract.address,
                        false,
                    )?;
                    REVERSE_MAP.remove(deps.storage, &Addr::unchecked(old_aa));
                }
                token.extension.account_ownership = false;
            }

            // Enable abstract-ownership
            (false, Some(new_aa)) => {
                // no need to remove bids, owner can discern bids to accept
                #[cfg(feature = "abstract")]
                validate_aa_ownership(
                    deps.as_ref(),
                    &new_aa,
                    account,
                    &env.contract.address,
                    true,
                )?;
                REVERSE_MAP.save(deps.storage, &Addr::unchecked(&new_aa), account)?;
                token.extension.account_ownership = true;
                token.token_uri = Some(new_aa);
            }

            // No action
            (false, None) => {}
        }

        Terp721AccountContract::default()
            .config
            .nft_info
            .save(deps.storage, account, &token)?;

        let mut res = Response::default();
        // refund all bids as they the condition of the bids may now differ from when they were created
        if !remove_bids_trigger.is_empty() {
            res.messages.push(SubMsg::new(WasmMsg::Execute {
                contract_addr: ACCOUNT_MANIFOLD.load(deps.storage)?.to_string(),
                msg: to_json_binary(&terp_account::manifold::ExecuteMsg::RemoveBids {
                    token_id: account.to_string(),
                })?,
                funds: vec![],
            }));
        }

        Ok(res)
    }

    /// updates the (usually) non `terp1...` addresses mapped to a `terp1...` account.
    /// Verify it is this account updating before removing.
    pub fn execute_update_reverse_map_keys(
        deps: DepsMut,
        _env: Env,
        info: MessageInfo,
        mut to_add: Vec<CosmosArbitrary>,
        mut to_remove: Vec<String>,
    ) -> Result<Response, ContractError> {
        let mut attr = vec![];
        nonpayable(&info)?;
        // the sender is always the value mapped to the store.
        let canonv = deps.api.addr_canonicalize(info.sender.as_str())?;

        let max = SUDO_PARAMS.load(deps.storage)?.max_reverse_map_key_limit;
        let count = REVMAP_LIMIT.may_load(deps.storage, &canonv.to_string())?;

        if count.is_none() {
            return Err(ContractError::AccountNotFound {});
        } else {
            let mut count = count.unwrap();
            // Calculate new count after adding and removing
            let mut new_count = count + to_add.len() as u32;

            // Check if we're trying to remove more than we're going to have
            if new_count != 0 && new_count < to_remove.len() as u32 || to_remove.len() as u32 > max
            {
                return Err(ContractError::CannotRemoveMoreThanWillExists {});
            }

            if new_count != 0 {
                new_count -= to_remove.len() as u32;
            }

            if new_count > max {
                return Err(ContractError::TooManyReverseMaps {
                    max,
                    have: new_count,
                });
            }
            // Process additions
            for add in to_add.drain(..) {
                // verify signature
                let hraddr = add.verify_return_readable(deps.api)?;
                count += 1;
                if count > max {
                    return Err(ContractError::TooManyReverseMaps { max, have: count });
                } else if let Some(cv) = REVERSE_MAP_KEY.may_load(deps.storage, &hraddr)? {
                    // override any mapping if this sender is value in map.
                    let canon_map = &CanonicalAddr::from(cv.clone());
                    if canon_map == &canonv {
                    } else {
                        return Err(ContractError::RecordAccountAlreadyExists {});
                    }
                }

                REVMAP_LIMIT.save(deps.storage, &canonv.to_string(), &(count))?;
                REVERSE_MAP_KEY.save(deps.storage, &hraddr, &Binary::new(canonv.to_vec()))?;
                attr.push(Attribute::new("added", &hraddr));
            }

            // Process removals
            for rem in to_remove.drain(..) {
                if count == 0 {
                    count = 1
                }
                if let Some(addr) = REVERSE_MAP_KEY.may_load(deps.storage, &rem)? {
                    let canonv = &CanonicalAddr::from(addr.clone());
                    let human_addr = deps.api.addr_humanize(canonv)?;
                    if human_addr == info.sender {
                        REVMAP_LIMIT.save(deps.storage, &canonv.to_string(), &(count - 1))?;
                        // println!("removed-key:   {:#?}", rem);
                        // println!("removed-value:   {:#?}", info.sender.as_str());
                        REVERSE_MAP_KEY.remove(deps.storage, &rem);
                        attr.push(Attribute::new("chain-cointype-removed", rem));
                        count -= 1;
                    } else {
                        return Err(ContractError::OwnershipError(
                            cw_ownable::OwnershipError::NotOwner,
                        ));
                    }
                }
            }
        }

        Ok(Response::new().add_attributes(attr))
    }

    /// Remove all text records, diables abstract-account features
    fn reset_token_metadata_and_reverse_map(
        deps: &mut DepsMut,
        contract_addr: Addr,
        account: &str,
    ) -> StdResult<()> {
        let mut extension = Metadata::default();
        let token = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?;

        if let Some(tokenuri) = token.token_uri.clone() {
            #[cfg(feature = "abstract")]
            if token.extension.account_ownership {
                // confirm this token is still set as owner of account
                let owner: Ownership<String> = deps
                    .querier
                    .query_wasm_smart(tokenuri, &abstract_std::account::QueryMsg::Ownership {})?;

                if let ownership::GovernanceDetails::NFT {
                    collection_addr,
                    token_id,
                } = owner.owner
                {
                    if collection_addr == contract_addr.to_string() && token_id == account {
                        extension = Metadata::default_with_account();
                    }
                }
            }
        }

        // Reset image, records
        Terp721AccountContract::default()
            .config
            .nft_info
            .save(deps.storage, account, &token)?;

        remove_reverse_mapping(deps, account, extension)?;

        Ok(())
    }

    fn remove_reverse_mapping(
        deps: &mut DepsMut,
        token_id: &str,
        metadata: Metadata,
    ) -> StdResult<()> {
        let mut token = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, token_id)?;

        // keep reverse mapping if tokenized ownership has been validated
        if let Some(token_uri) = token.token_uri.clone() {
            if !metadata.account_ownership {
                REVERSE_MAP.remove(deps.storage, &Addr::unchecked(token_uri));
                token.token_uri = None;
            }
        }

        Terp721AccountContract::default()
            .config
            .nft_info
            .save(deps.storage, token_id, &token)?;

        Ok(())
    }

    pub fn execute_add_text_record(
        deps: DepsMut,
        info: MessageInfo,
        account: String,
        mut record: TextRecord,
    ) -> Result<Response, ContractError> {
        let token_id = account;
        only_owner(deps.as_ref(), &info.sender, &token_id)?;

        let params = SUDO_PARAMS.load(deps.storage)?;
        let max_record_count = params.max_record_count;
        // new records should reset verified to None
        record.verified = None;

        nonpayable(&info)?;
        validate_record(&record)?;

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |token| match token {
                Some(mut token_info) => {
                    // can not add a record with existing account
                    for r in token_info.extension.records.iter() {
                        if r.account == record.account {
                            return Err(ContractError::RecordAccountAlreadyExists {});
                        }
                    }
                    token_info.extension.records.push(record.clone());
                    // check record length
                    if token_info.extension.records.len() > max_record_count as usize {
                        return Err(ContractError::TooManyRecords {
                            max: max_record_count,
                        });
                    }
                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            },
        )?;

        let event = Event::new("add-text-record")
            .add_attribute("sender", info.sender)
            .add_attribute("account", token_id)
            .add_attribute("record", record.into_json_string());
        Ok(Response::new().add_event(event))
    }

    pub fn execute_remove_text_record(
        deps: DepsMut,
        info: MessageInfo,
        account: String,
        record_account: String,
    ) -> Result<Response, ContractError> {
        let token_id = account;
        only_owner(deps.as_ref(), &info.sender, &token_id)?;
        nonpayable(&info)?;

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |token| match token {
                Some(mut token_info) => {
                    token_info
                        .extension
                        .records
                        .retain(|r| r.account != record_account);
                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            },
        )?;

        let event = Event::new("remove-text-record")
            .add_attribute("sender", info.sender)
            .add_attribute("account", token_id)
            .add_attribute("record_account", record_account);
        Ok(Response::new().add_event(event))
    }

    pub fn execute_update_text_record(
        deps: DepsMut,
        info: MessageInfo,
        account: String,
        mut record: TextRecord,
    ) -> Result<Response, ContractError> {
        let token_id = account;
        only_owner(deps.as_ref(), &info.sender, &token_id)?;
        let params = SUDO_PARAMS.load(deps.storage)?;
        let max_record_count = params.max_record_count;

        // updated records should reset verified to None
        record.verified = None;

        nonpayable(&info)?;
        validate_record(&record)?;

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |token| match token {
                Some(mut token_info) => {
                    token_info
                        .extension
                        .records
                        .retain(|r| r.account != record.account);
                    token_info.extension.records.push(record.clone());
                    // check record length
                    if token_info.extension.records.len() > max_record_count as usize {
                        return Err(ContractError::TooManyRecords {
                            max: max_record_count,
                        });
                    }
                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            },
        )?;

        let event = Event::new("update-text-record")
            .add_attribute("sender", info.sender)
            .add_attribute("account", token_id)
            .add_attribute("record", record.into_json_string());
        Ok(Response::new().add_event(event))
    }

    pub fn execute_verify_text_record(
        deps: DepsMut,
        info: MessageInfo,
        account: String,
        record_account: String,
        result: bool,
    ) -> Result<Response, ContractError> {
        nonpayable(&info)?;
        VERIFIER.assert_admin(deps.as_ref(), &info.sender)?;

        let token_id = account;

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |token| match token {
                Some(mut token_info) => {
                    if let Some(r) = token_info
                        .extension
                        .records
                        .iter_mut()
                        .find(|r| r.account == record_account)
                    {
                        r.verified = Some(result);
                    }
                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            },
        )?;

        let event = Event::new("verify-text-record")
            .add_attribute("sender", info.sender)
            .add_attribute("account", token_id)
            .add_attribute("record", record_account)
            .add_attribute("result", result.to_string());
        Ok(Response::new().add_event(event))
    }

    pub fn update_image_nft(
        deps: DepsMut,
        info: MessageInfo,
        account: String,
        nft: Option<NFT>,
    ) -> Result<Response, ContractError> {
        let token_id = account.clone();

        only_owner(deps.as_ref(), &info.sender, &token_id)?;
        nonpayable(&info)?;

        let mut event = Event::new("update_image_nft")
            .add_attribute("owner", info.sender.to_string())
            .add_attribute("token_id", account);

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |token| match token {
                Some(mut token_info) => {
                    token_info.extension.image_nft.clone_from(&nft);
                    Ok(token_info)
                }
                None => Err(ContractError::AccountNotFound {}),
            },
        )?;

        if let Some(nft) = nft {
            event = event.add_attribute("image_nft", nft.into_json_string());
        }

        Ok(Response::new().add_event(event))
    }

    pub fn set_profile_marketplace(
        deps: DepsMut,
        info: MessageInfo,
        address: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&info)?;
        // minter only function
        let minter_ownership = cw721::state::MINTER.get_ownership(deps.storage)?;
        match minter_ownership.owner {
            Some(minter) if info.sender == minter => {}
            _ => {
                return Err(ContractError::OwnershipError(
                    cw_ownable::OwnershipError::NotOwner,
                ));
            }
        }

        ACCOUNT_MANIFOLD.save(deps.storage, &deps.api.addr_validate(&address)?)?;

        let event = Event::new("set-account-marketplace")
            .add_attribute("sender", info.sender)
            .add_attribute("address", address);
        Ok(Response::new().add_event(event))
    }

    fn only_owner(deps: Deps, sender: &Addr, token_id: &str) -> Result<Addr, ContractError> {
        let owner = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, token_id)?
            .owner;

        if owner != sender {
            return Err(ContractError::OwnershipError(
                cw_ownable::OwnershipError::NotOwner,
            ));
        }

        Ok(owner)
    }

    fn validate_record(record: &TextRecord) -> Result<(), ContractError> {
        if record.verified.is_some() {
            return Err(ContractError::UnauthorizedVerification {});
        }
        let name_len = record.account.len();
        if name_len > MAX_TEXT_LENGTH as usize {
            return Err(ContractError::RecordAccountTooLong {});
        } else if name_len == 0 {
            return Err(ContractError::RecordAccountEmpty {});
        }

        if record.value.len() > MAX_TEXT_LENGTH as usize {
            return Err(ContractError::RecordValueTooLong {});
        } else if record.value.is_empty() {
            return Err(ContractError::RecordValueEmpty {});
        }
        Ok(())
    }

    /// BS721 FUNCTIONS
    pub fn execute_mint(
        deps: DepsMut,
        info: MessageInfo,
        token_id: String,
        owner: String,
        _token_uri: Option<String>,
        extension: Metadata,
    ) -> Result<Response, ContractError> {
        let minter_ownership = cw721::state::MINTER.get_ownership(deps.storage)?;
        match minter_ownership.owner {
            Some(minter) if info.sender == minter => {}
            _ => return Err(ContractError::UnauthorizedMinter {}),
        }

        // create the token
        let token = NftInfo {
            owner: deps.api.addr_validate(&owner)?,
            approvals: vec![],
            token_uri: None, // reserved for reverse map
            extension,
        };

        Terp721AccountContract::default().config.nft_info.update(
            deps.storage,
            &token_id,
            |old| match old {
                Some(_) => Err(ContractError::Cw721(
                    cw721::error::Cw721ContractError::Claimed {},
                )),
                None => Ok(token),
            },
        )?;
        Terp721AccountContract::default()
            .config
            .increment_tokens(deps.storage)?;

        // save with token owner canonv as key
        let canonv = deps.api.addr_canonicalize(&owner)?;
        REVMAP_LIMIT.save(deps.storage, &canonv.to_string(), &0)?;

        let event = Event::new("mint")
            .add_attribute("minter", info.sender)
            .add_attribute("token_id", &token_id)
            .add_attribute("owner", &owner);
        Ok(Response::new().add_event(event))
    }

    pub fn execute_burn(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        account: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&info)?;
        let market = &ACCOUNT_MANIFOLD.load(deps.storage)?;
        ensure_not_in_cooldown(deps.as_ref(), market, &account)?;

        let terp721 = Terp721AccountContract::default();

        terp721.execute(
            deps,
            &env,
            &info,
            Cw721ExecuteMsg::<Metadata, DefaultOptionalCollectionExtensionMsg, Empty>::Burn {
                token_id: account.to_string(),
            },
        )?;

        Ok(Response::new()
            .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: market.to_string(),
                msg: to_json_binary(&terp_account::manifold::ExecuteMsg::RemoveAsk {
                    token_id: account.to_string(),
                })?,
                funds: vec![],
            }))
            .add_event(Event::new("burn-account").add_attribute("account", account)))
    }

    pub fn execute_transfer_nft(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        recipient: String,
        token_id: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&info)?;
        let recipient = deps.api.addr_validate(&recipient)?;

        let names_marketplace = ACCOUNT_MANIFOLD.load(deps.storage)?;
        ensure_not_in_cooldown(deps.as_ref(), &names_marketplace, &token_id)?;

        let update_ask_msg =
            _transfer_nft(deps, env, &info, &recipient, &token_id, &names_marketplace)?;

        let event = Event::new("transfer")
            .add_attribute("sender", info.sender)
            .add_attribute("recipient", recipient)
            .add_attribute("token_id", token_id);

        Ok(Response::new().add_message(update_ask_msg).add_event(event))
    }

    // Update the ask on the marketplace
    fn ensure_not_in_cooldown(
        deps: Deps,
        names_marketplace: &Addr,
        token_id: &str,
    ) -> Result<(), ContractError> {
        // ensure token is not in cooldown
        let res: Option<PendingBid> = deps.querier.query_wasm_smart(
            names_marketplace.clone(),
            &terp_account::manifold::QueryMsg::Cooldown {
                token_id: token_id.to_string(),
            },
        )?;

        match res {
            Some(_) => {
                return Err(ContractError::AccountCannotBeTransfered {
                    reason: "Account is in cooldown".to_string(),
                })
            }
            None => Ok(()),
        }
    }

    // Update the ask on the marketplace
    fn update_ask_on_marketplace(
        token_id: &str,
        recipient: Addr,
        names_marketplace: &Addr,
    ) -> Result<WasmMsg, ContractError> {
        let msg = terp_account::manifold::ExecuteMsg::UpdateAsk {
            token_id: token_id.to_string(),
            seller: recipient.to_string(),
        };
        let update_ask_msg = WasmMsg::Execute {
            contract_addr: names_marketplace.to_string(),
            funds: vec![],
            msg: to_json_binary(&msg)?,
        };
        Ok(update_ask_msg)
    }

    fn _transfer_nft(
        mut deps: DepsMut,
        env: Env,
        info: &MessageInfo,
        recipient: &Addr,
        token_id: &str,
        names_marketplace: &Addr,
    ) -> Result<WasmMsg, ContractError> {
        let update_ask_msg =
            update_ask_on_marketplace(token_id, recipient.clone(), names_marketplace)?;

        reset_token_metadata_and_reverse_map(&mut deps, env.contract.address.clone(), token_id)?;

        let msg = Cw721ExecuteMsg::<Metadata, DefaultOptionalCollectionExtensionMsg, Empty>::TransferNft {
            recipient: recipient.to_string(),
            token_id: token_id.to_string(),
        };

        let terp721 = Terp721AccountContract::default();

        // Force account marketplace address as operator
        terp721.config.operators.save(
            deps.storage,
            (&info.sender, names_marketplace),
            &Expiration::Never {},
        )?;

        terp721.execute(deps, &env, info, msg)?;

        Ok(update_ask_msg)
    }

    pub fn execute_send_nft(
        mut deps: DepsMut,
        env: Env,
        info: MessageInfo,
        contract: String,
        token_id: String,
        msg: Binary,
    ) -> Result<Response, ContractError> {
        let contract_addr = deps.api.addr_validate(&contract)?;
        let names_marketplace = ACCOUNT_MANIFOLD.load(deps.storage)?;
        ensure_not_in_cooldown(deps.as_ref(), &names_marketplace, &token_id)?;
        let update_ask_msg =
            update_ask_on_marketplace(&token_id, contract_addr.clone(), &names_marketplace)?;

        reset_token_metadata_and_reverse_map(&mut deps, env.contract.address.clone(), &token_id)?;

        let msg =
            Cw721ExecuteMsg::<Metadata, DefaultOptionalCollectionExtensionMsg, Empty>::SendNft {
                contract: contract_addr.to_string(),
                token_id: token_id.to_string(),
                msg,
            };

        Terp721AccountContract::default().execute(deps, &env, &info, msg)?;

        let event = Event::new("send")
            .add_attribute("sender", info.sender)
            .add_attribute("contract", contract_addr.to_string())
            .add_attribute("token_id", token_id);

        Ok(Response::new().add_message(update_ask_msg).add_event(event))
    }
}

pub mod queries {

    use super::*;
    pub fn query_profile_marketplace(deps: Deps) -> StdResult<Addr> {
        ACCOUNT_MANIFOLD.load(deps.storage)
    }

    pub fn query_account(deps: Deps, mut address: String) -> StdResult<String> {
        if !address.starts_with("terp") {
            address = transcode(deps, &address)?
        }

        REVERSE_MAP
            .load(deps.storage, &deps.api.addr_validate(&address)?)
            .map_err(|_| StdError::msg(format!("No account associated with address {}", address)))
    }

    pub fn query_params(deps: Deps) -> StdResult<SudoParams> {
        SUDO_PARAMS.load(deps.storage)
    }

    pub fn query_associated_address(deps: Deps, account: &str) -> StdResult<String> {
        let token = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?;
        Ok(token.token_uri.unwrap_or(token.owner.to_string()))
    }

    pub fn query_image_nft(deps: Deps, account: &str) -> StdResult<Option<NFT>> {
        Ok(Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?
            .extension
            .image_nft)
    }

    pub fn query_text_records(deps: Deps, account: &str) -> StdResult<Vec<TextRecord>> {
        Ok(Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?
            .extension
            .records)
    }
    pub fn query_is_twitter_verified(deps: Deps, account: &str) -> StdResult<bool> {
        let records = Terp721AccountContract::default()
            .config
            .nft_info
            .load(deps.storage, account)?
            .extension
            .records;

        for record in records {
            if record.account == "twitter" {
                return Ok(record.verified.unwrap_or(false));
            }
        }

        Ok(false)
    }
}

pub fn transcode(deps: Deps, addr: &str) -> StdResult<String> {
    if let Some(canonv) = REVERSE_MAP_KEY.may_load(deps.storage, &addr.to_string())? {
        let human = &CanonicalAddr::from(canonv);
        Ok(deps.api.addr_humanize(human)?.to_string())
    } else {
        Err(StdError::msg(
            "no mappping set. Set a non `terp1...` addr mapped to your`terp1..` that owns this account token with UpdateMyReverseMapKey",
        ))
    }
}

fn validate_address(deps: Deps, sender: &Addr, addr: Addr) -> Result<Addr, ContractError> {
    // no need to validate if sender is address
    if sender == addr {
        return Ok(addr);
    }

    let contract_details = cw2::query_contract_info(&deps.querier, addr.to_string())?;
    if contract_details.contract.contains("cw721-base") {
        let collection_info: Ownership<Addr> = deps
            .querier
            .query_wasm_smart(&addr, &Terp721AccountsQueryMsg::Minter {})?;
        if let Some(col_owner) = &collection_info.owner {
            if col_owner == sender {
                return Ok(addr);
            }
        }
    }

    let ContractInfoResponse { admin, creator, .. } =
        deps.querier.query_wasm_contract_info(&addr)?;

    if let Some(admin) = admin {
        ensure!(
            admin == sender,
            ContractError::UnauthorizedCreatorOrAdmin {}
        );
    } else {
        // If there is no admin and the creator is not the sender, check creator's admin
        let creator_info = deps.querier.query_wasm_contract_info(creator)?;
        if creator_info.admin.is_none_or(|a| a != sender) {
            return Err(ContractError::UnauthorizedCreatorOrAdmin {});
        }
    }

    // we have a contract registration
    Ok(addr)
}

pub fn sudo_update_params(
    deps: DepsMut,
    max_record_count: u32,
    max_reverse_map_key_limit: u32,
) -> Result<Response, ContractError> {
    SUDO_PARAMS.save(
        deps.storage,
        &SudoParams {
            max_record_count,
            max_reverse_map_key_limit,
        },
    )?;

    let event =
        Event::new("update-params").add_attribute("max_record_count", max_record_count.to_string());
    Ok(Response::new().add_event(event))
}

mod test {

    #[test]
    pub fn test_transcode() {
        let deps = cosmwasm_std::testing::mock_dependencies();

        let res = super::transcode(
            deps.as_ref(),
            "akash1fxccvvhhy43tvet2ah7jqwq4cwl9k3dx3l2y8r",
        )
        .unwrap_err();

        let err_msg = res.to_string();
        assert!(
            err_msg.contains("no mappping set. Set a non `terp1...` addr mapped to your`terp1..` that owns this account token with UpdateMyReverseMapKey"),
            "Unexpected error: {err_msg}"
        )
    }
}
