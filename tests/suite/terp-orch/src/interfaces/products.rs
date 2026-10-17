//! cw-orch interfaces for product contracts that sit outside the authenticator,
//! account, and light-client groups.
//!
//! Mock deploy uses `ContractWrapper`. Live upload reads `artifacts/<name>.wasm`.

use cosmwasm_std::{Binary, Coin, Decimal, Empty, Uint128};
use cw_orch::{interface, prelude::*};

use super::wasm_path::wasm;

macro_rules! uploadable {
    ($ty:ident, $artifact:literal, $execute:path, $instantiate:path, $query:path) => {
        impl<Chain: TxHandler> Uploadable for $ty<Chain> {
            fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
                wasm($artifact)
            }
            fn wrapper() -> Box<dyn MockContract<Empty>> {
                Box::new(ContractWrapper::new_with_empty($execute, $instantiate, $query))
            }
        }
    };
}

#[interface(
    cw_shitstrap::contract::msg::InstantiateMsg,
    cw_shitstrap::contract::msg::ExecuteMsg,
    cw_shitstrap::contract::msg::QueryMsg,
    Empty,
    id = "cw-shitstrap"
)]
pub struct CwShitstrap;
uploadable!(
    CwShitstrap,
    "cw_shitstrap",
    cw_shitstrap::contract::execute,
    cw_shitstrap::contract::instantiate,
    cw_shitstrap::contract::query
);

#[interface(
    cw_shitstrap_factory::msg::InstantiateMsg,
    cw_shitstrap_factory::msg::ExecuteMsg,
    cw_shitstrap_factory::msg::QueryMsg,
    Empty,
    id = "cw-shitstrap-factory"
)]
pub struct CwShitstrapFactory;
uploadable!(
    CwShitstrapFactory,
    "cw_shitstrap_factory",
    cw_shitstrap_factory::contract::execute,
    cw_shitstrap_factory::contract::instantiate,
    cw_shitstrap_factory::contract::query
);

#[interface(
    nftfi_raffle::InstantiateMsg,
    nftfi_raffle::ExecuteMsg,
    nftfi_raffle::QueryMsg,
    Empty,
    id = "nftfi-raffle"
)]
pub struct NftfiRaffle;
uploadable!(
    NftfiRaffle,
    "nftfi_raffle",
    nftfi_raffle::execute,
    nftfi_raffle::instantiate,
    nftfi_raffle::query
);

#[interface(
    nftfi_loan::InstantiateMsg,
    nftfi_loan::ExecuteMsg,
    nftfi_loan::QueryMsg,
    Empty,
    id = "nftfi-loan"
)]
pub struct NftfiLoan;
uploadable!(
    NftfiLoan,
    "nftfi_loan",
    nftfi_loan::execute,
    nftfi_loan::instantiate,
    nftfi_loan::query
);

#[interface(
    revenue_marketplace::InstantiateMsg,
    revenue_marketplace::ExecuteMsg,
    revenue_marketplace::QueryMsg,
    Empty,
    id = "revenue-marketplace"
)]
pub struct RevenueMarketplace;
uploadable!(
    RevenueMarketplace,
    "revenue_marketplace",
    revenue_marketplace::execute,
    revenue_marketplace::instantiate,
    revenue_marketplace::query
);

#[interface(
    revenue_reserve_auction::InstantiateMsg,
    revenue_reserve_auction::ExecuteMsg,
    revenue_reserve_auction::QueryMsg,
    Empty,
    id = "revenue-reserve-auction"
)]
pub struct RevenueReserveAuction;
uploadable!(
    RevenueReserveAuction,
    "revenue_reserve_auction",
    revenue_reserve_auction::execute,
    revenue_reserve_auction::instantiate,
    revenue_reserve_auction::query
);

#[interface(cw420::msg::InstantiateMsg, cw420::msg::ExecuteMsg, cw420::msg::QueryMsg, Empty, id = "cw420")]
pub struct Cw420;
uploadable!(Cw420, "cw420", cw420::contract::execute, cw420::contract::instantiate, cw420::contract::query);

#[interface(
    cw_ave_factory::msg::InstantiateMsg,
    cw_ave_factory::msg::ExecuteMsg,
    cw_ave_factory::msg::QueryMsg,
    Empty,
    id = "cw-ave-factory"
)]
pub struct CwAveFactory;
uploadable!(
    CwAveFactory,
    "cw_ave_factory",
    cw_ave_factory::contract::execute,
    cw_ave_factory::contract::instantiate,
    cw_ave_factory::contract::query
);

#[interface(cw_ave::msg::InstantiateMsg, cw_ave::msg::ExecuteMsg, cw_ave::msg::QueryMsg, Empty, id = "cw-ave")]
pub struct CwAve;
uploadable!(CwAve, "cw_ave", cw_ave::contract::execute, cw_ave::contract::instantiate, cw_ave::contract::query);

#[interface(
    cw_headstash::InstantiateMsg,
    cw_headstash::ExecuteMsg,
    cw_headstash::QueryMsg,
    Empty,
    id = "cw-headstash"
)]
pub struct CwHeadstash;
uploadable!(
    CwHeadstash,
    "cw_headstash",
    cw_headstash::execute,
    cw_headstash::instantiate,
    cw_headstash::query
);

#[interface(
    cw_headstash_manifold::msg::InstantiateMsg,
    cw_headstash_manifold::msg::ExecuteMsg,
    cw_headstash_manifold::msg::QueryMsg,
    Empty,
    id = "cw-headstash-manifold"
)]
pub struct CwHeadstashManifold;
uploadable!(
    CwHeadstashManifold,
    "cw_headstash_manifold",
    cw_headstash_manifold::contract::execute,
    cw_headstash_manifold::contract::instantiate,
    cw_headstash_manifold::contract::query
);

#[interface(cw_norick::InstantiateMsg, cw_norick::ExecuteMsg, cw_norick::QueryMsg, Empty, id = "cw-norick")]
pub struct CwNorick;
uploadable!(CwNorick, "cw_norick", cw_norick::execute, cw_norick::instantiate, cw_norick::query);

#[interface(
    cw_nullifier_auth::msg::InstantiateMsg,
    cw_nullifier_auth::msg::ExecuteMsg,
    cw_nullifier_auth::msg::QueryMsg,
    Empty,
    id = "cw-nullifier-auth"
)]
pub struct CwNullifierAuth;
uploadable!(
    CwNullifierAuth,
    "cw_nullifier_auth",
    cw_nullifier_auth::contract::execute,
    cw_nullifier_auth::contract::instantiate,
    cw_nullifier_auth::contract::query
);

#[interface(
    cw_nullifier_gate::msg::InstantiateMsg,
    cw_nullifier_gate::msg::ExecuteMsg,
    cw_nullifier_gate::msg::QueryMsg,
    Empty,
    id = "cw-nullifier-gate"
)]
pub struct CwNullifierGate;
uploadable!(
    CwNullifierGate,
    "cw_nullifier_gate",
    cw_nullifier_gate::contract::execute,
    cw_nullifier_gate::contract::instantiate,
    cw_nullifier_gate::contract::query
);

#[interface(
    cw_vote_anchor::InstantiateMsg,
    cw_vote_anchor::ExecuteMsg,
    cw_vote_anchor::QueryMsg,
    Empty,
    id = "cw-vote-anchor"
)]
pub struct CwVoteAnchor;
uploadable!(
    CwVoteAnchor,
    "cw_vote_anchor",
    cw_vote_anchor::contract::execute,
    cw_vote_anchor::contract::instantiate,
    cw_vote_anchor::contract::query
);

#[interface(
    cw_vote_ceremony::InstantiateMsg,
    cw_vote_ceremony::ExecuteMsg,
    cw_vote_ceremony::QueryMsg,
    Empty,
    id = "cw-vote-ceremony"
)]
pub struct CwVoteCeremony;
uploadable!(
    CwVoteCeremony,
    "cw_vote_ceremony",
    cw_vote_ceremony::contract::execute,
    cw_vote_ceremony::contract::instantiate,
    cw_vote_ceremony::contract::query
);

#[interface(
    cw_reg_eligibility::msg::InstantiateMsg,
    cw_reg_eligibility::msg::ExecuteMsg,
    cw_reg_eligibility::msg::QueryMsg,
    Empty,
    id = "cw-reg-eligibility"
)]
pub struct CwRegEligibility;
uploadable!(
    CwRegEligibility,
    "cw_reg_eligibility",
    cw_reg_eligibility::contract::execute,
    cw_reg_eligibility::contract::instantiate,
    cw_reg_eligibility::contract::query
);

#[interface(svg_auction::InstantiateMsg, svg_auction::ExecuteMsg, svg_auction::QueryMsg, Empty, id = "svg-auction")]
pub struct SvgAuction;
uploadable!(
    SvgAuction,
    "svg_auction",
    svg_auction::execute,
    svg_auction::instantiate,
    svg_auction::query
);

use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::generate_test_wavs_proof;

/// Lab instantiate messages. Code ids that must already be uploaded are passed in.
pub fn shitstrap_msg(admin: &str) -> cw_shitstrap::contract::msg::InstantiateMsg {
    cw_shitstrap::contract::msg::InstantiateMsg {
        owner: Some(admin.to_string()),
        accepted: vec![cw_shit_denom::PossibleShit::native_denom("uterp", 1_000_000_000_000_000_000)],
        cutoff: Uint128::new(1),
        shitmos: cw_shit_denom::UncheckedDenom::Native("ushit".into()),
        title: "lab".into(),
        description: "lab shitstrap".into(),
        daos: Vec::new(),
    }
}

pub fn headstash_msg() -> cw_headstash::InstantiateMsg {
    // Existing denom: the mock chain has no tokenfactory module, so NewFungible's
    // MsgCreateDenom cannot be executed here. Live chains with tokenfactory can.
    cw_headstash::InstantiateMsg {
        genesis_root: Binary::from([0u8; 32]),
        distro_hash_domain: Default::default(),
        genesis_label: None,
        token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new("existing_token".into())),
        wavs: generate_test_wavs_proof(1),
        circuit_id: None,
    }
}

pub fn ave_msg(admin: &str, cw420_code_id: u64) -> cw_ave::msg::InstantiateMsg {
    cw_ave::msg::InstantiateMsg {
        event_curator: admin.to_string(),
        title: "lab".into(),
        description: "lab event".into(),
        usher_admins: vec![],
        guest_details: vec![],
        cw420: cw420_code_id,
        zkid: 0,
        event_timeline: vec![],
    }
}

pub fn marketplace_msg(admin: &str) -> revenue_marketplace::InstantiateMsg {
    revenue_marketplace::InstantiateMsg {
        fee_manager: admin.to_string(),
        royalty_registry: None,
        protocol_fee_bps: 100,
        max_royalty_fee_bps: 1000,
        maker_reward_bps: 0,
        taker_reward_bps: 0,
        burn_bps: 0,
        default_denom: "uterp".into(),
    }
}

pub fn reserve_auction_msg(admin: &str) -> revenue_reserve_auction::InstantiateMsg {
    revenue_reserve_auction::InstantiateMsg {
        fee_manager: admin.to_string(),
        trading_fee_percent: Decimal::percent(2),
        min_bid_increment_percent: Decimal::percent(5),
        min_duration: 60,
        max_duration: 3600,
        extend_duration: 60,
        create_auction_fee: Coin::new(0u128, "uterp"),
        max_auctions_to_settle_per_block: 10,
        halt_duration_threshold: 3600,
        halt_buffer_duration: 60,
        halt_postpone_duration: 60,
        burn_bps: 0,
        min_reserve_prices: vec![Coin::new(1u128, "uterp")],
    }
}

pub fn svg_auction_msg(admin: &str) -> svg_auction::InstantiateMsg {
    svg_auction::InstantiateMsg {
        protocol_fee_bps: 100,
        min_increment_bps: 100,
        min_reserve_price: Uint128::new(1),
        duration: 3600,
        extension: 60,
        accepted_denom: vec!["uterp".into()],
        protocol_addr: admin.to_string(),
    }
}
