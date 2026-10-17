use super::*;
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{coin, OwnedDeps};

type D = OwnedDeps<MockStorage, MockApi, MockQuerier>;
struct W { d: D, nft: Addr, seller: Addr, fee: Addr }
fn env_at(t: u64) -> Env { let mut e = mock_env(); e.block.time = Timestamp::from_seconds(t); e }
const T0: u64 = 1_000_000;

fn setup() -> W {
    let mut d = mock_dependencies();
    let (nft, seller, fee) = (d.api.addr_make("orbs"), d.api.addr_make("seller"), d.api.addr_make("protocol"));
    let admin = d.api.addr_make("admin");
    instantiate(d.as_mut(), env_at(T0), message_info(&admin, &[]), InstantiateMsg { protocol_fee_bps: 250, min_increment_bps: 500,
        min_reserve_price: Uint128::new(100), duration: 3600, extension: 300, accepted_denom: vec!["uterp".into()], protocol_addr: fee.to_string() }).unwrap();
    W { d, nft, seller, fee }
}
fn list(w: &mut W, token: &str, price: u128, instant: bool) -> Result<Response, ContractError> {
    let m = Cw721ReceiveMsg { sender: w.seller.to_string(), token_id: token.into(),
        msg: to_json_binary(&Cw721HookMsg::CreateAuction { denom: "uterp".into(), reserve_price: Uint128::new(price), is_instant_sale: instant }).unwrap() };
    let nft = w.nft.clone();
    execute(w.d.as_mut(), env_at(T0), message_info(&nft, &[]), ExecuteMsg::ReceiveNft(m))
}
fn bid(w: &mut W, who: &str, t: u64, amt: u128) -> Result<Response, ContractError> {
    let a = w.d.api.addr_make(who);
    execute(w.d.as_mut(), env_at(t), message_info(&a, &[coin(amt, "uterp")]), ExecuteMsg::PlaceBid { auction_id: 1 })
}
fn bank(r: &Response) -> Vec<(String, u128)> {
    r.messages.iter().filter_map(|m| match &m.msg { CosmosMsg::Bank(BankMsg::Send { to_address, amount }) =>
        Some((to_address.clone(), amount[0].amount.to_string().parse().unwrap())), _ => None }).collect()
}
fn nft_to(r: &Response) -> Option<String> {
    r.messages.iter().find_map(|m| match &m.msg { CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) => {
        let v: serde_json_value::V = from_json(msg).unwrap(); Some(v.transfer_nft.recipient) } _ => None })
}
mod serde_json_value {
    #[cosmwasm_schema::cw_serde] pub struct T { pub recipient: String, pub token_id: String }
    #[cosmwasm_schema::cw_serde] pub struct V { pub transfer_nft: T }
}
fn get(w: &W) -> AuctionResponse { from_json(query(w.d.as_ref(), mock_env(), QueryMsg::Auction { auction_id: 1 }).unwrap()).unwrap() }

#[test]
fn buy_now_transfers_and_pays_minus_fee() {
    let mut w = setup();
    list(&mut w, "3", 10_000, true).unwrap();
    assert!(bid(&mut w, "buyer", T0 + 1, 9_999).unwrap_err().to_string().contains("buy-now price"));
    let r = bid(&mut w, "buyer", T0 + 1, 10_000).unwrap();
    assert_eq!(nft_to(&r), Some(w.d.api.addr_make("buyer").to_string()));
    assert_eq!(bank(&r), vec![(w.fee.to_string(), 250), (w.seller.to_string(), 9_750)]);
    assert_eq!(get(&w).status, Status::Sold);
    let a: Option<AuctionResponse> = from_json(query(w.d.as_ref(), mock_env(), QueryMsg::NftAuction { nft_contract: w.nft.to_string(), token_id: "3".into() }).unwrap()).unwrap();
    assert!(a.is_none());
    assert!(bid(&mut w, "late", T0 + 2, 10_000).is_err());
}

#[test]
fn auction_bids_refund_extend_and_settle() {
    let mut w = setup();
    list(&mut w, "1", 1_000, false).unwrap();
    assert!(bid(&mut w, "a", T0 + 1, 999).is_err());
    assert!(bid(&mut w, "a", T0 + 1, 1_000).unwrap().messages.is_empty());
    assert!(bid(&mut w, "b", T0 + 2, 1_049).unwrap_err().to_string().contains("at least 1050"));
    let r = bid(&mut w, "b", T0 + 2, 1_050).unwrap();
    assert_eq!(bank(&r), vec![(w.d.api.addr_make("a").to_string(), 1_000)]);
    // anti-snipe
    let r = bid(&mut w, "c", T0 + 3500, 2_000).unwrap();
    assert_eq!(bank(&r)[0].1, 1_050);
    assert_eq!(get(&w).end_time.seconds(), T0 + 3800);
    let s = w.d.api.addr_make("anyone");
    assert!(execute(w.d.as_mut(), env_at(T0 + 3700), message_info(&s, &[]), ExecuteMsg::Settle { auction_id: 1 }).unwrap_err().to_string().contains("not ended"));
    let r = execute(w.d.as_mut(), env_at(T0 + 3800), message_info(&s, &[]), ExecuteMsg::Settle { auction_id: 1 }).unwrap();
    assert_eq!(nft_to(&r), Some(w.d.api.addr_make("c").to_string()));
    assert_eq!(bank(&r), vec![(w.fee.to_string(), 50), (w.seller.to_string(), 1_950)]);
    let hist: Vec<Bid> = from_json(query(w.d.as_ref(), mock_env(), QueryMsg::BidHistoryByAuctionId { auction_id: 1, limit: None }).unwrap()).unwrap();
    assert_eq!(hist.len(), 3);
    assert!(hist[1].refunded && hist[2].refunded && !hist[0].refunded);
    assert!(execute(w.d.as_mut(), env_at(T0 + 3900), message_info(&s, &[]), ExecuteMsg::Settle { auction_id: 1 }).is_err());
}

#[test]
fn no_bid_settle_returns_nft_and_cancel_rules() {
    let mut w = setup();
    list(&mut w, "1", 1_000, false).unwrap();
    let s = w.d.api.addr_make("x");
    let r = execute(w.d.as_mut(), env_at(T0 + 3600), message_info(&s, &[]), ExecuteMsg::Settle { auction_id: 1 }).unwrap();
    assert_eq!(nft_to(&r), Some(w.seller.to_string()));
    assert!(bank(&r).is_empty());
    // second listing: cancel by seller only, not after a bid
    list(&mut w, "2", 1_000, false).unwrap();
    let seller = w.seller.clone();
    assert!(matches!(execute(w.d.as_mut(), env_at(T0), message_info(&s, &[]), ExecuteMsg::CancelAuction { auction_id: 2 }), Err(ContractError::Unauthorized)));
    let r = execute(w.d.as_mut(), env_at(T0), message_info(&seller, &[]), ExecuteMsg::CancelAuction { auction_id: 2 }).unwrap();
    assert_eq!(nft_to(&r), Some(seller.to_string()));
    list(&mut w, "4", 1_000, false).unwrap();
    let b = w.d.api.addr_make("b");
    execute(w.d.as_mut(), env_at(T0 + 1), message_info(&b, &[coin(1_000, "uterp")]), ExecuteMsg::PlaceBid { auction_id: 3 }).unwrap();
    assert!(execute(w.d.as_mut(), env_at(T0 + 2), message_info(&seller, &[]), ExecuteMsg::CancelAuction { auction_id: 3 }).is_err());
    let all: Vec<AuctionResponse> = from_json(query(w.d.as_ref(), mock_env(), QueryMsg::AuctionByContract { nft_contract: w.nft.to_string(), start_after: None, limit: None }).unwrap()).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn listing_and_bid_validation() {
    let mut w = setup();
    assert!(list(&mut w, "1", 99, false).unwrap_err().to_string().contains("below minimum"));
    let m = Cw721ReceiveMsg { sender: w.seller.to_string(), token_id: "1".into(),
        msg: to_json_binary(&Cw721HookMsg::CreateAuction { denom: "uatom".into(), reserve_price: Uint128::new(500), is_instant_sale: false }).unwrap() };
    let nft = w.nft.clone();
    assert!(execute(w.d.as_mut(), env_at(T0), message_info(&nft, &[]), ExecuteMsg::ReceiveNft(m)).unwrap_err().to_string().contains("not accepted"));
    list(&mut w, "1", 500, false).unwrap();
    let seller = w.seller.clone();
    assert!(execute(w.d.as_mut(), env_at(T0 + 1), message_info(&seller, &[coin(600, "uterp")]), ExecuteMsg::PlaceBid { auction_id: 1 }).unwrap_err().to_string().contains("seller"));
    let b = w.d.api.addr_make("b");
    assert!(execute(w.d.as_mut(), env_at(T0 + 1), message_info(&b, &[coin(600, "uatom")]), ExecuteMsg::PlaceBid { auction_id: 1 }).is_err());
    assert!(bid(&mut w, "b", T0 + 3600, 600).unwrap_err().to_string().contains("not open"));
}
