use crate::error::ContractError;
use crate::msg::{EventSegmentRes, ExecuteMsg, GuestRootResponse, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{
    generate_instantiate_salt2, preamble_msg_arb_036, sha256, CheckInDetails, CheckInSignatureData,
    Config, GuestDetails, RegisteringEventAddressAndPayment, RegisteringGuest, ReplaceHomieTicket,
    TicketPaymentOption, ATTENDANCE_RECORD, CONFIG, EVENT_STAGES, GUEST_DETAILS, HOMIE_TICKETS,
    LICENSE_ADDR, NULLIFIERS, RESERVED_TICKETS, TOTAL_RESERVED_BY_GUEST_TYPE,
};
use av_event_helpers::{get_license_addr, LICENSE_CANONICAL_ADDR};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    coin, from_json, instantiate2_address, to_json_binary, Addr, BankMsg, Binary, CanonicalAddr,
    Coin, CosmosMsg, Deps, DepsMut, Env, HexBinary, MessageInfo, Order, Response, StdError,
    StdResult, Storage, WasmMsg,
};
use cw2::set_contract_version;
use cw4::{Cw4QueryMsg, Member, MemberResponse};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:cw-ave";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const NAMESPACE: &[u8] = b"aves";
pub const CHARACTER_LIMIT: usize = 128;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    LICENSE_ADDR.save(deps.storage, &get_license_addr(&env.block.chain_id)?)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    // set owner
    let curator = deps.api.addr_validate(&msg.event_curator.clone())?;

    // generic validation
    if msg.title.len() > CHARACTER_LIMIT || msg.description.len() > CHARACTER_LIMIT {
        return Err(ContractError::BadEventTitleOrDescription {});
    }

    // validate guest details
    for dt in msg.guest_details {
        // ensure no duplicate guest weights
        match GUEST_DETAILS.may_load(deps.storage, dt.guest_weight)? {
            Some(_) => return Err(ContractError::DuplicateGuestWeight {}),
            None => {
                if dt.guest_type.len() > CHARACTER_LIMIT
                    || dt.max_ticket_limit > dt.total_ticket_limit
                {
                    return Err(ContractError::BadGuestDetailParams {});
                }

                // cannot set duplicate accepted tokens
                let mut unique = Vec::new();
                for fee in &dt.ticket_cost {
                    if unique.contains(&fee.denom) {
                        return Err(ContractError::DuplicateFeeDenom {});
                    }
                    unique.push(fee.denom.to_string());
                }

                GUEST_DETAILS.save(deps.storage, dt.guest_weight, &dt)?;
                TOTAL_RESERVED_BY_GUEST_TYPE.save(deps.storage, dt.guest_weight, &0)?;
            }
        }
    }

    // validate event stages
    for (i, event) in msg.event_timeline.iter().enumerate() {
        // Validate that start date is before or equal to end date
        if event.start > event.end {
            return Err(ContractError::InvalidEventDates {});
        }
        if event.stage_description.len() > 128usize {
            return Err(ContractError::BadEventDescriptionLength {});
        }

        // For events that are not the first, check that the previous end date is before or at the next start date
        if i > 0 {
            let prev_event = &msg.event_timeline[i - 1];
            if prev_event.end > event.start {
                return Err(ContractError::OverlappingEventDates {});
            }
        }

        // Save the event stage
        EVENT_STAGES.save(deps.storage, i as u64, event)?;
    }

    // setup cw420 groups
    let cw721_checksum = deps.querier.query_wasm_code_info(msg.cw420)?;
    let usher_salt = generate_instantiate_salt2(&cw721_checksum.checksum, NAMESPACE);
    let mut guest_salt_data = usher_salt.to_vec();
    guest_salt_data[0] ^= 1; // Flip the first byte to ensure it's different
    let guest_salt = Binary::new(guest_salt_data);

    let contract_address = deps.api.addr_canonicalize(env.contract.address.as_str())?;

    let usher_cw420 = match instantiate2_address(
        cw721_checksum.checksum.as_slice(),
        &contract_address,
        usher_salt.as_slice(),
    ) {
        Ok(addr) => addr,
        Err(err) => return Err(ContractError::from(err)),
    };

    let guest_cw420 = match instantiate2_address(
        cw721_checksum.checksum.as_slice(),
        &contract_address,
        guest_salt.as_slice(),
    ) {
        Ok(addr) => addr,
        Err(err) => return Err(ContractError::from(err)),
    };

    let usher_msg = WasmMsg::Instantiate2 {
        admin: Some(env.contract.address.to_string()),
        code_id: msg.cw420,
        msg: to_json_binary(&cw420::msg::InstantiateMsg {
            admin: Some(env.contract.address.to_string()),
            members: msg.usher_admins,
        })?,
        funds: vec![],
        label: "cw-ave-ushers".to_string(),
        salt: usher_salt,
    };

    let guest_msg = WasmMsg::Instantiate2 {
        admin: Some(env.contract.address.to_string()),
        code_id: msg.cw420,
        msg: to_json_binary(&cw420::msg::InstantiateMsg {
            admin: Some(env.contract.address.to_string()),
            members: vec![],
        })?,
        funds: vec![],
        label: "cw-ave-guests".to_string(),
        salt: guest_salt,
    };
    let event_usher_contract = deps.api.addr_humanize(&usher_cw420)?;
    let event_guest_contract = deps.api.addr_humanize(&guest_cw420)?;

    CONFIG.save(
        deps.storage,
        &Config {
            title: msg.title,
            description: msg.description,
            curator,
            event_usher_contract,
            event_guest_contract,
            guest_root: None,
            zkid: msg.zkid,
        },
    )?;

    Ok(Response::new().add_messages(vec![usher_msg, guest_msg]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::PurchaseTickets { guests } => perform_ticket_purchase(deps, info, guests),
        ExecuteMsg::RefundUnconfirmedTickets { guests } => {
            refund_unconfirmed_ticket_purchase(deps, info, guests)
        }
        ExecuteMsg::CheckInGuest { .. } => Err(ContractError::UseVerifyConsume {}),
        ExecuteMsg::ClaimTicketPayments {} => perform_claim_ticket_payments(deps, env, info),
        ExecuteMsg::ClaimTicketReservedByHomie { homie_addr } => {
            perform_claim_ticket_reserved_by_homie(deps, info, homie_addr)
        }
        ExecuteMsg::UpdateTicketAddress {
            new_ticket_addr,
            replace_homies_ticket,
        } => perform_update_ticket_wallet(deps, info, new_ticket_addr, replace_homies_ticket),
        ExecuteMsg::UpdateGuestRoot { guest_root } => {
            perform_update_guest_root(deps, info, guest_root)
        }
        ExecuteMsg::VerifyConsume {
            proof,
            nullifier,
            event_id,
        } => perform_verify_consume(deps, info, proof, nullifier, event_id),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
        QueryMsg::EventSegments {} => {
            let segments: Vec<EventSegmentRes> = EVENT_STAGES
                .range(deps.storage, None, None, cosmwasm_std::Order::Descending)
                .map(|item| item.map(|(seg_id, segment)| EventSegmentRes { seg_id, segment }))
                .collect::<StdResult<Vec<EventSegmentRes>>>()?;

            to_json_binary(&segments)
        }
        QueryMsg::TicketPaymentOptionsByGuestWeight { guest_weight } => {
            let gd = GUEST_DETAILS.load(deps.storage, guest_weight)?;
            Ok(to_json_binary(&TicketPaymentOption {
                guest_type: gd.guest_type,
                payment_options: gd.ticket_cost,
            })?)
        }
        QueryMsg::AllTicketPaymentOptions {} => Ok(to_json_binary(
            &GUEST_DETAILS
                .range(deps.storage, None, None, Order::Ascending)
                .map(|res| {
                    res.map(|(_, guest_details)| TicketPaymentOption {
                        guest_type: guest_details.guest_type,
                        payment_options: guest_details.ticket_cost,
                    })
                })
                .collect::<StdResult<Vec<TicketPaymentOption>>>()?,
        )?),
        QueryMsg::GuestTypeDetailsByWeight { guest_weight } => Ok(to_json_binary(
            &GUEST_DETAILS.load(deps.storage, guest_weight)?,
        )?),
        QueryMsg::GuestTypeDetailsAll {} => Ok(to_json_binary(
            &GUEST_DETAILS
                .range(deps.storage, None, None, Order::Ascending)
                .map(|res| res.map(|(_, guest_details)| guest_details))
                .collect::<StdResult<Vec<GuestDetails>>>()?,
        )?),
        QueryMsg::GuestRoot {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&GuestRootResponse {
                guest_root: cfg.guest_root,
            })
        }
        QueryMsg::IsUsher { addr } => to_json_binary(&query_is_usher(deps, addr)?),
        QueryMsg::NullifierSpent { nullifier } => to_json_binary(
            &NULLIFIERS
                .may_load(deps.storage, nullifier.as_slice())?
                .unwrap_or(false),
        ),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::new())
}

/// Entry point to purchase event tickets
pub fn perform_ticket_purchase(
    deps: DepsMut,
    info: MessageInfo,
    guests: Vec<RegisteringGuest>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut msgs = Vec::new();

    for guest in guests {
        // check if guest type exists
        let gd = GUEST_DETAILS.load(deps.storage, guest.guest_weight)?;
        let count = TOTAL_RESERVED_BY_GUEST_TYPE.load(deps.storage, gd.guest_weight)?;
        let to_reserve = guest.reap.len();
        // Calculate how many tickets we can actually process (respecting the limit)
        let max_possible = gd.total_ticket_limit.saturating_sub(count) as usize;
        let process_count = to_reserve.min(max_possible);

        // Split the guest list - prioritize first entries in the array
        let to_process = &guest.reap[..process_count];
        // todo: implmeent overbooking feature where we can still accept these payments if neccessary
        // let overflow = &guest.reap[process_count..];
        // if !overflow.is_empty() {
        // }

        // if tickets being reserved are greater than limit for this guest type, strip the # of entries in the tickets being reserved from the object so that we will reach the limit and not error.
        let (reserved, remaining_funds, dev_fee_msg) = count_tickets_and_remainder(
            LICENSE_ADDR.load(deps.storage)?.to_string(),
            &info.funds,
            gd.ticket_cost,
            to_process,
        );

        // return any overflow funds sent.
        msgs.extend([
            form_return_payment_overflow_msgs(&remaining_funds, &info.sender),
            dev_fee_msg,
        ]);

        RESERVED_TICKETS.update(deps.storage, &gd.guest_weight, |a| match a {
            Some(mut td) => {
                td += to_reserve as u128;
                if td > gd.max_ticket_limit.into() {
                    return Err(ContractError::CannotReserveTicketCount {});
                }
                Ok::<u128, ContractError>(td)
            }
            None => {
                if reserved <= gd.max_ticket_limit.into() {
                    Ok(reserved)
                } else {
                    Err(ContractError::CannotReserveTicketCount {})
                }
            }
        })?;

        // Eligibility is Poseidon `guest_root`. Do not grow guest cw420 ListMembers.
        let _ = (
            &info.sender,
            &guest.reap,
            gd.guest_weight,
            &cfg.event_guest_contract,
        );
    }
    // P1: GitHub returned Response::new() and dropped overflow, license fee, and
    // cw420 UpdateMembers — guests never joined the group.
    msgs.retain(msg_has_funds_or_is_not_empty_bank);
    Ok(Response::new().add_messages(msgs))
}

/// Entry point to checkin guests as event usher
pub fn perform_checkin_guest(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    checkin: CheckInDetails,
) -> Result<Response, ContractError> {
    // sender must be one of event ushers
    let cfg = CONFIG.load(deps.storage)?;
    if check_if_cw420_member(deps.as_ref(), &cfg.event_usher_contract, &info.sender)?.is_none() {
        return Err(ContractError::NotAnEventUsher {});
    };

    // verify signature came from guest and is valid
    if !deps.api.secp256k1_verify(
        &sha256(preamble_msg_arb_036(&checkin.ticket_addr, &checkin.signed_data).as_bytes()),
        &checkin.signature,
        &checkin.pubkey,
    )? {
        return Err(ContractError::CheckinVerificationFailed {});
    };

    // parse signed_data to retrieve specific guest weight
    let signature_data: CheckInSignatureData = from_json(checkin.signed_data)?;
    if signature_data.event_contract_addr != env.contract.address.as_str() {
        return Err(ContractError::IncorrectCheckinSignature {});
    }
    if signature_data.usher_wallet_addr != info.sender.as_str() {
        return Err(ContractError::IncorrectCheckinSignature {});
    }

    if let Some(guest_weight) = check_if_cw420_member(
        deps.as_ref(),
        &cfg.event_guest_contract,
        &deps.api.addr_validate(&checkin.ticket_addr)?,
    )? {
        let guest_details = GUEST_DETAILS.load(deps.storage, guest_weight)?;

        // if giving ticket to another external wallet,
        // create composite storage key to prevent unauthorized checkin for another wallet address

        // consume ticket
        // RESERVED_TICKETS.update(deps.storage, &checkin.ticket_addr, |a| match a {
        //     Some(mut td) => {
        //         if td == 0u128 {
        //             return Err(ContractError::GuestAlreadyCheckedIn {});
        //         }
        //         td -= 1;
        //         Ok::<u128, ContractError>(td)
        //     }
        //     None => Err(ContractError::NoReservedTicketsForGuest {}),
        // })?;

        // recurisvely update guest status for specific segment
        match guest_details.event_segment_access {
            crate::state::EventSegmentAccessType::SingleSegment { id } => {
                if !signature_data.event_segment_ids.contains(&id) {
                    return Err(ContractError::IncorrectEventSegmentId {});
                }
                update_attendance_record(deps.storage, &checkin.ticket_addr, id)?;
            }
            crate::state::EventSegmentAccessType::AnyOfSpecificSegments { ids } => {
                for checkin_id in signature_data.event_segment_ids {
                    if !ids.contains(&checkin_id) {
                        return Err(ContractError::IncorrectEventSegmentId {});
                    }
                    update_attendance_record(deps.storage, &checkin.ticket_addr, checkin_id)?;
                }
            }
            crate::state::EventSegmentAccessType::AllOfSpecificSegments { ids } => {
                // we just checkin all automatically
                for checkin_id in ids {
                    update_attendance_record(deps.storage, &checkin.ticket_addr, checkin_id)?;
                }
            }
        }
    } else {
        return Err(ContractError::GuestTypeIncorrect {});
    };

    Ok(Response::new())
}

pub fn update_attendance_record(
    storage: &mut dyn Storage,
    ticket_addr: &String,
    event_segment_id: u64,
) -> Result<bool, ContractError> {
    ATTENDANCE_RECORD.update(storage, (ticket_addr, event_segment_id), |ci| {
        if let Some(status) = ci {
            match status {
                true => Err(ContractError::GuestAlreadyCheckedIn {}),
                false => Ok(true),
            }
        } else {
            Err(ContractError::GuestTypeIncorrect {})
        }
    })
}

fn refund_unconfirmed_ticket_purchase(
    _deps: DepsMut,
    _info: MessageInfo,
    _guest: Vec<String>,
) -> Result<Response, ContractError> {
    Ok(Response::new())
}

/// counts how many tickets are purchased, returning any overflow amounts sent.
fn count_tickets_and_remainder(
    dev_addr: String,
    funds_sent: &[Coin],
    gd_ticket_cost: Vec<Coin>,
    reap: &[RegisteringEventAddressAndPayment],
) -> (u128, Vec<Coin>, CosmosMsg) {
    let mut remaining_funds = funds_sent.to_vec();
    let mut total_tickets = 0;

    let mut dev_fee_coins = Vec::new();

    for guest in reap {
        let denom = guest.payment_asset.clone();

        // Find required payment amount for this denom
        if let Some(cost) = gd_ticket_cost.iter().find(|c| c.denom == denom) {
            // Find matching coin in remaining funds
            if let Some(fund) = remaining_funds.iter_mut().find(|c| c.denom == denom) {
                // Check if sufficient funds are available
                if fund.amount >= cost.amount {
                    // 3% flat fee
                    dev_fee_coins.push(Coin::new(
                        cost.amount.multiply_ratio(3u128, 100u128),
                        fund.denom.to_string(),
                    ));
                    // Deduct payment and count ticket
                    fund.amount = fund.amount.checked_sub(cost.amount).unwrap();
                    total_tickets += 1;
                }
            }
        }
    }

    // Filter out zero-amount coins
    remaining_funds.retain(|coin| !coin.amount.is_zero());
    (
        total_tickets,
        remaining_funds,
        CosmosMsg::Bank(BankMsg::Send {
            to_address: dev_addr,
            amount: dev_fee_coins,
        }),
    )
}

fn msg_has_funds_or_is_not_empty_bank(msg: &CosmosMsg) -> bool {
    match msg {
        CosmosMsg::Bank(BankMsg::Send { amount, .. }) => {
            amount.iter().any(|c| !c.amount.is_zero())
        }
        _ => true,
    }
}

#[allow(dead_code)]
fn form_update_guestlist_msg(
    sender: &Addr,
    storage: &mut dyn Storage,
    guest_addrs: &[RegisteringEventAddressAndPayment],
    guest_weight: u64,
    gust_cw420: &Addr,
) -> Result<WasmMsg, ContractError> {
    // P2: GitHub never wrote ATTENDANCE_RECORD false, so first check_in_guest
    // hit None → GuestTypeIncorrect. Do not change update_attendance_record.
    let gd = GUEST_DETAILS.load(storage, guest_weight)?;
    let stage_ids: Vec<u64> = match gd.event_segment_access {
        crate::state::EventSegmentAccessType::SingleSegment { id } => vec![id],
        crate::state::EventSegmentAccessType::AnyOfSpecificSegments { ids }
        | crate::state::EventSegmentAccessType::AllOfSpecificSegments { ids } => ids,
    };
    for a in guest_addrs {
        for stage_id in &stage_ids {
            if ATTENDANCE_RECORD.may_load(storage, (&a.ticket_addr, *stage_id))? == Some(true) {
                return Err(ContractError::GuestAlreadyCheckedIn {});
            }
            ATTENDANCE_RECORD.save(storage, (&a.ticket_addr, *stage_id), &false)?;
        }
    }
    Ok(WasmMsg::Execute {
        contract_addr: gust_cw420.to_string(),
        msg: to_json_binary(&cw420::msg::ExecuteMsg::UpdateMembers {
            remove: vec![],
            add: guest_addrs
                .iter()
                .map(|a| -> Result<Member, StdError> {
                    let ticket = a.ticket_addr.clone();
                    if sender.as_str() != &ticket {
                        HOMIE_TICKETS.update(storage, &sender.to_string(), |b| match b {
                            Some(mut c) => {
                                if c.contains(&ticket) {
                                    return Err(StdError::msg(
                                        "This ticket is already registered for homies",
                                    ));
                                } else {
                                    c.push(ticket.clone());
                                    Ok(c)
                                }
                            }
                            None => Ok(vec![ticket.clone()]),
                        })?;
                    }
                    Ok(Member {
                        addr: ticket,
                        weight: guest_weight,
                    })
                })
                .collect::<Result<Vec<Member>, StdError>>()?,
        })?,
        funds: vec![],
    })
}

fn form_return_payment_overflow_msgs(overflow: &Vec<Coin>, sender: &Addr) -> CosmosMsg {
    CosmosMsg::Bank(BankMsg::Send {
        to_address: sender.to_string(),
        amount: overflow.clone(),
    })
}

fn check_if_cw420_member(
    deps: Deps,
    cw420: &Addr,
    wallet: &Addr,
) -> Result<Option<u64>, ContractError> {
    // check if guest already is member
    let res: cw4::MemberResponse = deps.querier.query_wasm_smart(
        cw420,
        &cw420::msg::QueryMsg::Member {
            addr: wallet.to_string(),
            at_height: None,
        },
    )?;

    Ok(res.weight)
}

/// Entry point to claim funds sent for ticket payments
pub fn perform_claim_ticket_payments(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.curator {
        return Err(ContractError::NotAnEventUsher {});
    }

    // retrive all tokens to query
    let tokens: Vec<String> = GUEST_DETAILS
        .range(deps.storage, None, None, Order::Ascending)
        .filter_map(|res| res.ok())
        .flat_map(|(_, guest_details)| guest_details.ticket_cost.into_iter().map(|c| c.denom))
        .fold(vec![], |mut acc, denom| {
            if !acc.contains(&denom) {
                acc.push(denom);
            }
            acc
        });

    let balances: Vec<Coin> = tokens
        .iter()
        .map(|denom| deps.querier.query_balance(&env.contract.address, denom))
        .collect::<StdResult<Vec<Coin>>>()?;

    Ok(Response::new().add_message(BankMsg::Send {
        to_address: config.curator.to_string(),
        amount: balances,
    }))
}

/// allows a wallet that was reserved a ticket from another wallet to claim their ticket,
/// preventing the chance of the homie checkin-in on their behalf.
pub fn perform_claim_ticket_reserved_by_homie(
    deps: DepsMut,
    info: MessageInfo,
    reserver: String,
) -> Result<Response, ContractError> {
    HOMIE_TICKETS.update(
        deps.storage,
        &reserver,
        |current_list| -> Result<_, StdError> {
            match current_list {
                Some(mut list) => {
                    if let Some(pos) = list.iter().position(|x| x == &info.sender.to_string()) {
                        list.remove(pos);
                    }
                    Ok(list)
                }
                None => Err(StdError::msg("this homie did not reserve a ticket for your current wallet address.. maybe try a different wallet?")),
            }
        },
    )?;

    Ok(Response::new().add_attribute("claimed_homie_ticket", info.sender.to_string()))
}

/// allows a wallet has a ticket reserved to update the address to use to checkin.
pub fn perform_update_ticket_wallet(
    deps: DepsMut,
    info: MessageInfo,
    new_ticket_addr: Option<String>,
    homies_to_update: Vec<ReplaceHomieTicket>,
) -> Result<Response, ContractError> {
    if homies_to_update.len() > 10 {
        return Err(ContractError::TooManyHomieTickets {});
    }
    let guest_cw420 = CONFIG.load(deps.storage)?.event_guest_contract;

    let querier = deps.querier;

    let res: MemberResponse = querier.query_wasm_smart(
        &guest_cw420,
        &Cw4QueryMsg::Member {
            addr: info.sender.to_string(),
            at_height: None,
        },
    )?;

    if let Some(weight) = res.weight {
        let sender = info.sender.to_string();
        let mut list = vec![];
        if let Some(homies) = HOMIE_TICKETS.may_load(deps.storage, &sender)? {
            homies.iter().for_each(|h| {
                if let Some(update) = homies_to_update.iter().position(|htu| &htu.old == h) {
                    list.push(homies_to_update[update].new.clone());
                } else {
                    list.push(h.to_string());
                }
            });
        }

        let mut to_add = Vec::new();
        let mut to_remove = Vec::new();

        // remap any homie tickets to new address
        if list.len() > 0 {
            let mut ticket_addr = sender.clone();
            if let Some(new) = new_ticket_addr {
                HOMIE_TICKETS.remove(deps.storage, &ticket_addr);
                ticket_addr = new;
                to_remove.push(sender);
                to_add.push(Member {
                    addr: ticket_addr.clone(),
                    weight,
                });
            }
            HOMIE_TICKETS.save(deps.storage, &ticket_addr, &list)?;
        }

        // get weights for each addr
        for homie in homies_to_update {
            let res: MemberResponse = querier.query_wasm_smart(
                &guest_cw420,
                &Cw4QueryMsg::Member {
                    addr: info.sender.to_string(),
                    at_height: None,
                },
            )?;
            if let Some(weight) = res.weight {
                to_add.push(Member {
                    addr: homie.new,
                    weight,
                });
                to_remove.push(homie.old);
            }
        }

        Ok(
            Response::new().add_message(form_cw420_msg(
                guest_cw420.to_string(),
                to_add,
                to_remove,
            )?),
        )
    } else {
        return Err(ContractError::NoReservedTicketsForGuest {});
    }
}


fn query_is_usher(deps: Deps, addr: String) -> StdResult<bool> {
    let cfg = CONFIG.load(deps.storage)?;
    let wallet = deps.api.addr_validate(&addr)?;
    let res: MemberResponse = deps.querier.query_wasm_smart(
        cfg.event_usher_contract,
        &Cw4QueryMsg::Member {
            addr: wallet.to_string(),
            at_height: None,
        },
    )?;
    Ok(res.weight.is_some())
}

/// Usher-only consume. Stores Poseidon nullifier. Does not write ticket_addr attendance.
pub fn perform_verify_consume(
    deps: DepsMut,
    info: MessageInfo,
    proof: Binary,
    nullifier: HexBinary,
    event_id: HexBinary,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if check_if_cw420_member(deps.as_ref(), &cfg.event_usher_contract, &info.sender)?.is_none()
    {
        return Err(ContractError::NotAnEventUsher {});
    }
    let root = cfg.guest_root.ok_or(ContractError::GuestRootNotSet {})?;
    if root.len() != 32 || nullifier.len() != 32 || event_id.len() != 32 {
        return Err(ContractError::InvalidConsumeInstance {});
    }
    if NULLIFIERS
        .may_load(deps.storage, nullifier.as_slice())?
        .unwrap_or(false)
    {
        return Err(ContractError::NullifierSpent {});
    }
    let instances = [
        Binary::from(root.to_vec()),
        Binary::from(nullifier.to_vec()),
        Binary::from(event_id.to_vec()),
    ];
    if !crate::zk::verify_halo2_proof(deps.api, cfg.zkid, proof.as_slice(), &instances)? {
        return Err(ContractError::InvalidProof {});
    }
    NULLIFIERS.save(deps.storage, nullifier.as_slice(), &true)?;
    Ok(Response::new()
        .add_attribute("action", "verify_consume")
        .add_attribute("nullifier", nullifier.to_hex())
        .add_attribute("usher", info.sender))
}

/// Sale runtime / curator posts a new paid-set merkle root. This is door SoT.
pub fn perform_update_guest_root(
    deps: DepsMut,
    info: MessageInfo,
    guest_root: HexBinary,
) -> Result<Response, ContractError> {
    if guest_root.len() != 32 {
        return Err(ContractError::InvalidGuestRoot {});
    }
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.curator {
        return Err(ContractError::NotEventCurator {});
    }
    cfg.guest_root = Some(guest_root.clone());
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "update_guest_root")
        .add_attribute("guest_root", guest_root.to_hex()))
}

fn form_cw420_msg(
    contract_addr: String,
    add: Vec<Member>,
    remove: Vec<String>,
) -> Result<WasmMsg, ContractError> {
    Ok(WasmMsg::Execute {
        contract_addr,
        msg: to_json_binary(&cw420::msg::ExecuteMsg::UpdateMembers { remove, add })?,
        funds: vec![],
    })
}
