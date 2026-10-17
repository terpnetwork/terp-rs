use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, HexBinary};
use cw4::Member;

use crate::state::{
    CheckInDetails, Config, EventSegment, GuestDetails, RegisteringGuest, ReplaceHomieTicket,
    TicketPaymentOption,
};

#[cw_serde]
pub struct InstantiateMsg {
    /// if not set, sender
    pub event_curator: String,
    /// label for contract & front end
    pub title: String,
    /// description of avEvent for recordkeeping
    pub description: String,
    /// list of admin keys able to checkin guests
    pub usher_admins: Vec<Member>,
    /// details of each type of guest attendees can participate as
    pub guest_details: Vec<GuestDetails>,
    /// code-id of cw420 contract
    pub cw420: u64,
    /// x/wasm circuit id from MsgStoreFullCircuit (proof_instance_verify).
    #[serde(default)]
    pub zkid: u64,
    /// timeline of events segments
    pub event_timeline: Vec<EventSegment>,
}

#[cw_serde]
#[derive(cw_orch::ExecuteFns)]
pub enum ExecuteMsg {
    // /// Cw20 Entry Point
    // Receive(Cw20ReceiveMsg),
    PurchaseTickets {
        guests: Vec<RegisteringGuest>,
    },
    CheckInGuest {
        checkin: CheckInDetails,
    },
    RefundUnconfirmedTickets {
        guests: Vec<String>,
    },
    ClaimTicketPayments {},
    /// Claim your ticket that your homie has purchased for you.
    /// They will not be able to check you in once you claimed your ticket
    ClaimTicketReservedByHomie {
        homie_addr: String,
    },
    UpdateTicketAddress {
        /// Optional value to update your own. Left empty 
        new_ticket_addr: Option<String>,
        replace_homies_ticket: Vec<ReplaceHomieTicket>,
    },
    /// Curator posts a new merkle root of paid leaves (SoT eligibility).
    UpdateGuestRoot {
        guest_root: HexBinary,
    },
    /// Usher-only door consume. Proof is Halo2 Pasta (`verify_halo2_proof`, prover_id=0).
    /// Public instances: on-chain `guest_root`, `nullifier`, `event_id` (32-byte Pasta Fp).
    /// Spent = `NULLIFIERS`. Not `ATTENDANCE_RECORD[ticket_addr]`.
    VerifyConsume {
        proof: Binary,
        nullifier: HexBinary,
        event_id: HexBinary,
    },
}

#[cw_serde]
pub enum ReceiveMsg {
    PurchaseTickets { guests: Vec<RegisteringGuest> },
}

#[cw_serde]
#[derive(cw_orch::QueryFns, QueryResponses)]
pub enum QueryMsg {
    /// returns basic details regarding this event
    #[returns(Config)]
    Config {},
    /// All segements for this event
    #[returns(Vec<EventSegmentRes>)]
    EventSegments {},
    /// Details of a specific type of guest able to participate in event as.
    #[returns(GuestDetails)]
    GuestTypeDetailsByWeight { guest_weight: u64 },
    /// All details of types of guest
    #[returns(Vec<GuestDetails>)]
    GuestTypeDetailsAll {},
    /// All payment options accepted for a given ticket type
    #[returns(TicketPaymentOption)]
    TicketPaymentOptionsByGuestWeight { guest_weight: u64 },
    /// All payment options available
    #[returns(Vec<TicketPaymentOption>)]
    AllTicketPaymentOptions {},
    /// Door SoT: 32-byte merkle root of paid guests. None until first UpdateGuestRoot.
    #[returns(GuestRootResponse)]
    GuestRoot {},
    /// cw420 usher membership. Intended public instance.
    #[returns(bool)]
    IsUsher { addr: String },
    /// Door spent. Not attendance-by-addr.
    #[returns(bool)]
    NullifierSpent { nullifier: HexBinary },
}

#[cw_serde]
pub struct GuestRootResponse {
    pub guest_root: Option<HexBinary>,
}

#[cw_serde]
pub struct EventSegmentRes {
    pub seg_id: u64,
    pub segment: EventSegment,
}

#[cw_serde]
pub struct MigrateMsg {}
