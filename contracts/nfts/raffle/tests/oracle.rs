//! Raffle BuyTicket with optional oracle pricing for an alt asset.
use cosmwasm_std::{Addr, Binary, Coin, Decimal, Empty, Uint256, coins};
use cw_multi_test::{AppResponse, BankSudo, BasicApp, ContractWrapper, Executor, SudoMsg};
use cw_oracle_registry::msg as reg;
use cw_oracle_registry::state::SourceKind;
use nftfi_raffle::msg::{AltPotResponse, ExecuteMsg, InstantiateMsg, QueryMsg, RaffleOptionsMsg};
use nftfi_randomness::RandomnessSourceMsg;
use nftfi_utils::AssetInfo;
use nftfi_utils::svg;
use terp_oracle::*;

const BASE: &str = "uterp";
const ALT: &str = "uatom";
const PRICE: u128 = 25_000_000; // 25 TERP; ATOM/TERP = 12.5 -> 2 ATOM

struct W {
    app: BasicApp,
    owner: Addr,
    alice: Addr,
    bob: Addr,
    fee: Addr,
    nft: Addr,
    raffle: Addr,
    registry: Addr,
    token: u64,
}

fn pricing(enforce: bool, registry: &Addr, policy: PricingPolicy) -> OraclePricing {
    OraclePricing {
        enforce,
        alt_assets: vec![AltAsset {
            asset: PriceAsset::native(ALT),
            base_denom: BASE.into(),
            feeds: vec![Feed {
                oracle: registry.to_string(),
                kind: FeedKind::Spot { market_id: "ATOM/TERP".into() },
                weight: None,
            }],
            combine: AggregateMethod::Median,
            quote: Quote::AltInBase,
            base_decimals: 6,
            alt_decimals: 6,
        }],
        policy,
    }
}

/// `mk(registry)` builds the raffle's instantiate pricing.
fn world(mk: impl Fn(&Addr) -> Option<OraclePricing>) -> W {
    let mut app = BasicApp::default();
    let owner = app.api().addr_make("owner");
    let alice = app.api().addr_make("alice");
    let bob = app.api().addr_make("bob");
    let fee = app.api().addr_make("fee");
    for who in [&owner, &alice, &bob] {
        for d in [BASE, ALT] {
            app.sudo(SudoMsg::Bank(BankSudo::Mint {
                to_address: who.to_string(),
                amount: coins(1_000_000_000, d),
            }))
            .unwrap();
        }
    }
    let rid = app.store_code(Box::new(ContractWrapper::new(
        cw_oracle_registry::contract::execute,
        cw_oracle_registry::contract::instantiate,
        cw_oracle_registry::contract::query,
    )));
    let registry = app
        .instantiate_contract(
            rid,
            owner.clone(),
            &reg::InstantiateMsg { admin: owner.to_string(), open_submit: true, bounty: None, note: None, ibc2_source_client: None },
            &[],
            "registry",
            None,
        )
        .unwrap();
    let drop = svg::lab().into_iter().next().unwrap();
    let nft_id = app.store_code(svg::contract());
    let nft = app
        .instantiate_contract(nft_id, owner.clone(), &drop.instantiate_msg(owner.as_str()), &[], "nft", None)
        .unwrap();
    let raffle_code = app.store_code(Box::new(
        ContractWrapper::new_with_empty(nftfi_raffle::execute, nftfi_raffle::instantiate, nftfi_raffle::query)
            .with_reply_empty(nftfi_raffle::reply),
    ));
    let raffle = app
        .instantiate_contract(
            raffle_code,
            owner.clone(),
            &InstantiateMsg {
                name: "raffle".into(),
                owner: None,
                fee_addr: fee.to_string(),
                minimum_raffle_duration: Some(1),
                max_tickets_per_raffle: Some(100),
                raffle_fee: Decimal::percent(10),
                oracle_pricing: mk(&registry),
                creation_coins: None,
                randomness: RandomnessSourceMsg::Seed { provider: None },
            },
            &[],
            "raffle",
            None,
        )
        .unwrap();
    let mut w = W { app, owner, alice, bob, fee, nft, raffle, registry, token: 0 };
    w.push("12.5");
    w
}

impl W {
    fn push(&mut self, mid: &str) {
        let h = self.app.block_info().height;
        self.app
            .execute_contract(
                self.owner.clone(),
                self.registry.clone(),
                &reg::ExecuteMsg::Submit {
                    market_id: "ATOM/TERP".into(),
                    source: SourceKind::Relayer,
                    source_chain: "terp-local".into(),
                    pool_id: Some(1),
                    height: h,
                    mid: mid.into(),
                },
                &[],
            )
            .unwrap();
        self.advance(1, 0);
    }
    fn advance(&mut self, blocks: u64, secs: u64) {
        self.app.update_block(|b| {
            b.height += blocks;
            b.time = b.time.plus_seconds(secs.max(5 * blocks));
        });
    }
    fn create(&mut self, min: Option<u32>) -> u64 {
        let token_id = self.token.to_string();
        self.token += 1;
        let owner = self.owner.clone();
        self.app.execute_contract(owner.clone(), self.nft.clone(), &svg::mint_msg(1), &[]).unwrap();
        self.app
            .execute_contract(
                owner.clone(),
                self.nft.clone(),
                &cw721::msg::Cw721ExecuteMsg::<Empty, Empty, Empty>::Approve {
                    spender: self.raffle.to_string(),
                    token_id: token_id.clone(),
                    expires: None,
                },
                &[],
            )
            .unwrap();
        let res = self
            .app
            .execute_contract(
                owner,
                self.raffle.clone(),
                &ExecuteMsg::CreateRaffle {
                    owner: None,
                    assets: vec![AssetInfo::cw721(self.nft.as_str(), &token_id)],
                    raffle_options: RaffleOptionsMsg {
                        start: None,
                        duration: 100,
                        comment: None,
                        max_ticket_number: Some(10),
                        max_ticket_per_address: Some(4),
                        min_ticket_number: min,
                        one_winner_per_asset: false,
                    },
                    raffle_ticket_price: AssetInfo::coin(PRICE, BASE),
                },
                &[],
            )
            .unwrap();
        res.events
            .iter()
            .flat_map(|e| e.attributes.iter())
            .find(|a| a.key == "raffle_id")
            .unwrap()
            .value
            .parse()
            .unwrap()
    }
    fn buy(&mut self, who: &Addr, id: u64, count: u32, funds: &[Coin]) -> Result<AppResponse, String> {
        self.app
            .execute_contract(
                who.clone(),
                self.raffle.clone(),
                &ExecuteMsg::BuyTicket { raffle_id: id, ticket_count: count, on_behalf_of: None },
                funds,
            )
            .map_err(|e| format!("{e:?} {e}"))
    }
    fn finish(&mut self, id: u64) -> AppResponse {
        self.advance(1, 101);
        // Under-minimum raffles are already Finished and skip the draw.
        let _ = self.app
            .execute_contract(
                self.owner.clone(),
                self.raffle.clone(),
                &ExecuteMsg::Draw { raffle_id: id, seed: Binary::from([9u8; 32]) },
                &[],
            );
        self.advance(1, 0);
        self.app
            .execute_contract(self.owner.clone(), self.raffle.clone(), &ExecuteMsg::ClaimRaffle { raffle_id: id }, &[])
            .unwrap()
    }
    fn bal(&self, a: &Addr, d: &str) -> u128 {
        let v: Uint256 = self.app.wrap().query_balance(a.as_str(), d).unwrap().amount;
        v.to_string().parse().unwrap()
    }
    fn alt_pot(&self, id: u64) -> AltPotResponse {
        self.app.wrap().query_wasm_smart(&self.raffle, &QueryMsg::AltPot { raffle_id: id }).unwrap()
    }
}

const START: u128 = 1_000_000_000;

#[test]
fn no_pricing_is_unchanged() {
    let mut w = world(|_| None);
    let id = w.create(None);
    let (alice, bob) = (w.alice.clone(), w.bob.clone());
    let e = w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap_err();
    assert!(!e.contains("oracle"), "{e}");
    assert!(w.buy(&alice, id, 1, &coins(PRICE - 1, BASE)).is_err());
    assert!(w.buy(&alice, id, 1, &coins(PRICE + 1, BASE)).is_err());
    w.buy(&bob, id, 2, &coins(2 * PRICE, BASE)).unwrap();
    assert_eq!(w.alt_pot(id).alt_tickets, 0);
    let p: Option<OraclePricing> =
        w.app.wrap().query_wasm_smart(&w.raffle, &QueryMsg::OraclePricing {}).unwrap();
    assert!(p.is_none());
}

#[test]
fn not_enforced_quotes_but_rejects_alt() {
    let mut w = world(|r| Some(pricing(false, r, PricingPolicy::default())));
    let id = w.create(None);
    let alice = w.alice.clone();
    let q: QuoteResponse = w
        .app
        .wrap()
        .query_wasm_smart(
            &w.raffle,
            &QueryMsg::OracleQuote { base: Coin::new(PRICE, BASE), asset: PriceAsset::native(ALT) },
        )
        .unwrap();
    assert_eq!(q.required, Uint256::from(2_000_000u128));
    assert!(!q.enforced);
    assert!(w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap_err().contains("not enforced"));
    w.buy(&alice, id, 1, &coins(PRICE, BASE)).unwrap();
}

#[test]
fn enforced_alt_and_base_pots_split_on_claim() {
    let mut w = world(|r| Some(pricing(true, r, PricingPolicy::default())));
    let id = w.create(None);
    let (alice, bob, owner, fee) = (w.alice.clone(), w.bob.clone(), w.owner.clone(), w.fee.clone());
    // 2 tickets = 50 TERP = 4 ATOM; overpay 0.5 ATOM, refunded to sender.
    let r = w.buy(&alice, id, 2, &coins(4_500_000, ALT)).unwrap();
    assert!(r.events.iter().flat_map(|e| &e.attributes).any(|a| a.key == "paid_alt"));
    assert_eq!(w.bal(&alice, ALT), START - 4_000_000);
    assert!(w.buy(&bob, id, 1, &coins(1_999_999, ALT)).unwrap_err().contains("below"));
    w.buy(&bob, id, 1, &coins(PRICE, BASE)).unwrap();
    let pot = w.alt_pot(id);
    assert_eq!(pot.alt_tickets, 2);
    assert_eq!(pot.pot, coins(4_000_000, ALT));
    assert_eq!(w.bal(&w.raffle.clone(), ALT), 4_000_000);
    assert_eq!(w.bal(&w.raffle.clone(), BASE), PRICE);
    w.finish(id);
    assert_eq!(w.bal(&fee, ALT), 400_000);
    assert_eq!(w.bal(&fee, BASE), PRICE / 10);
    assert_eq!(w.bal(&owner, ALT), START + 3_600_000);
    assert_eq!(w.bal(&owner, BASE), START + PRICE * 9 / 10);
    assert_eq!(w.bal(&w.raffle.clone(), ALT), 0);
    assert_eq!(w.bal(&w.raffle.clone(), BASE), 0);
}

#[test]
fn price_moves_change_required_alt() {
    let mut w = world(|r| Some(pricing(true, r, PricingPolicy::default())));
    let id = w.create(None);
    let alice = w.alice.clone();
    w.push("25"); // ATOM doubles -> 1 ATOM per ticket
    assert!(w.buy(&alice, id, 1, &coins(999_999, ALT)).is_err());
    w.buy(&alice, id, 1, &coins(1_000_000, ALT)).unwrap();
    assert_eq!(w.alt_pot(id).pot, coins(1_000_000, ALT));
}

#[test]
fn stale_price_rejected_until_fresh() {
    let policy = PricingPolicy { max_staleness_blocks: Some(5), ..Default::default() };
    let mut w = world(move |r| Some(pricing(true, r, policy.clone())));
    let id = w.create(None);
    let alice = w.alice.clone();
    w.advance(10, 0);
    let e = w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap_err();
    assert!(e.to_lowercase().contains("stale"), "{e}");
    assert_eq!(w.bal(&alice, ALT), START);
    w.push("12.5");
    w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap();
}

#[test]
fn cancelled_raffle_refunds_alt_in_alt() {
    let mut w = world(|r| Some(pricing(true, r, PricingPolicy::default())));
    let id = w.create(Some(5));
    let (alice, bob) = (w.alice.clone(), w.bob.clone());
    w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap();
    w.buy(&alice, id, 1, &coins(PRICE, BASE)).unwrap();
    w.buy(&bob, id, 1, &coins(PRICE, BASE)).unwrap();
    w.push("50"); // later moves must not change the refund
    w.finish(id);
    assert_eq!(w.bal(&alice, ALT), START);
    assert_eq!(w.bal(&alice, BASE), START);
    assert_eq!(w.bal(&bob, BASE), START);
    assert_eq!(w.bal(&w.raffle.clone(), ALT), 0);
    assert_eq!(w.bal(&w.raffle.clone(), BASE), 0);
}

#[test]
fn update_oracle_pricing_owner_only_and_validated() {
    let mut w = world(|_| None);
    let (owner, alice, registry) = (w.owner.clone(), w.alice.clone(), w.registry.clone());
    let p = pricing(true, &registry, PricingPolicy::default());
    let msg = ExecuteMsg::UpdateOraclePricing { pricing: Some(p.clone()) };
    assert!(w.app.execute_contract(alice.clone(), w.raffle.clone(), &msg, &[]).is_err());
    let mut bad = p.clone();
    bad.alt_assets[0].feeds.clear();
    assert!(w
        .app
        .execute_contract(owner.clone(), w.raffle.clone(), &ExecuteMsg::UpdateOraclePricing { pricing: Some(bad) }, &[])
        .is_err());
    w.app.execute_contract(owner.clone(), w.raffle.clone(), &msg, &[]).unwrap();
    let got: Option<OraclePricing> =
        w.app.wrap().query_wasm_smart(&w.raffle, &QueryMsg::OraclePricing {}).unwrap();
    assert_eq!(got, Some(p));
    let id = w.create(None);
    w.buy(&alice, id, 1, &coins(2_000_000, ALT)).unwrap();
    w.app
        .execute_contract(owner, w.raffle.clone(), &ExecuteMsg::UpdateOraclePricing { pricing: None }, &[])
        .unwrap();
    assert!(w.buy(&alice, id, 1, &coins(2_000_000, ALT)).is_err());
}
