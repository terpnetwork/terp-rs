//! Open scribing: fee proposals per epoch, shield-owner selection, owner/minter split,
//! no-selection settle, participant registry, speed, render before vs after.
use crate::contract::SvgTokenUriResponse;
use crate::entry::{execute, instantiate, query};
use crate::scribing::{EpochInfo, EpochPot, MessageRecord, ParticipantView, ProposalView};
use crate::{ExecuteMsg, InstantiateMsg, QueryMsg, SvgCollectionMetadata, SvgExecuteMsgExt, SvgQueryMsgExt};
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{coin, from_json, Addr, BankMsg, Binary, CosmosMsg, Env, OwnedDeps, Response};
use cw_svg::{VariableDef, VariableKind};

const THEMES: [&str; 4] = ["tides", "first light", "the long orbit", "quiet ships"];

fn opts(name: &str, v: &[&str]) -> VariableDef {
    VariableDef { name: name.into(), kind: VariableKind::Options(v.iter().map(|s| s.to_string()).collect()) }
}

fn template() -> (String, Vec<VariableDef>) {
    let mut t = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-40 -40 80 80\">");
    for c in 0..9 {
        let (x, y) = ((c % 3) as i32 * 25 - 25, (c / 3) as i32 * 25 - 25);
        t.push_str(&format!("<circle cx=\"{x}\" cy=\"{y}\" r=\"10\" fill=\"rgb(255,200,90)\" opacity=\"0\"><animate attributeName=\"opacity\" values=\"${{scribe.c{c}}}\" dur=\"${{scribe.dur}}s\" calcMode=\"discrete\" repeatCount=\"indefinite\"/></circle>"));
    }
    t.push_str("<text y=\"38\">${scribe}</text></svg>");
    let mut v = vec![opts("scribe", &THEMES)];
    for c in 0..9 {
        v.push(opts(&format!("scribe.c{c}"), &[""]));
    }
    v.push(opts("scribe.dur", &[""]));
    (t, v)
}

fn inst_msg(t: &str, v: &[VariableDef]) -> InstantiateMsg {
    InstantiateMsg {
        name: "orb".into(), symbol: "ORB".into(), minter: None, creator: None, withdraw_address: None,
        collection_info_extension: SvgCollectionMetadata {
            seed: Binary::from(b"marain".as_slice()), packed: cw_svg_pack::pack(t, v).unwrap().into(),
            price_tiers: vec![], total: 10, mint_start_time: None, mint_end_time: None, whitelist: None,
            require_auth: false,
        },
    }
}

type Deps = OwnedDeps<MockStorage, MockApi, MockQuerier>;
const H0: u64 = 12_345;
const EB: u64 = 100;

fn ext(m: SvgExecuteMsgExt) -> ExecuteMsg { ExecuteMsg::UpdateExtension { msg: m } }
fn at(h: u64) -> Env { let mut e = mock_env(); e.block.height = h; e }
fn q<T: serde::de::DeserializeOwned>(deps: &Deps, h: u64, m: SvgQueryMsgExt) -> T {
    from_json(query(deps.as_ref(), at(h), QueryMsg::Extension { msg: m }).unwrap()).unwrap()
}

/// admin mints 0 and 1 (original minter = admin); orb 0 is the shield and is sold to `shield`;
/// orb 1 is sold to `holder`.
fn setup() -> (Deps, Addr, Addr, Addr) {
    let mut deps = mock_dependencies();
    let admin = deps.api.addr_make("admin");
    let shield = deps.api.addr_make("shield");
    let holder = deps.api.addr_make("holder");
    let (t, v) = template();
    instantiate(deps.as_mut(), at(H0), message_info(&admin, &[]), inst_msg(&t, &v)).unwrap();
    let mint = ext(SvgExecuteMsgExt::Mint(cw_svg::MintMsg { alloc: 0, amnt: 2, proof_hashes: vec![], ext: None }));
    execute(deps.as_mut(), at(H0), message_info(&admin, &[]), mint).unwrap();
    for (id, to) in [("0", &shield), ("1", &holder)] {
        execute(deps.as_mut(), at(H0), message_info(&admin, &[]),
            ExecuteMsg::TransferNft { recipient: to.to_string(), token_id: id.into() }).unwrap();
    }
    execute(deps.as_mut(), at(H0), message_info(&admin, &[]), ext(SvgExecuteMsgExt::ConfigureScribing {
        epoch_blocks: EB, min_fee: coin(1000, "uterp"), owner_bps: 7000, max_proposals: 3, shield_token_id: "0".into() })).unwrap();
    (deps, admin, shield, holder)
}

fn uri(deps: &Deps, id: &str) -> String {
    let r: SvgTokenUriResponse = q(deps, H0, SvgQueryMsgExt::SvgTokenUri { token_id: id.into() });
    r.svg
}
fn propose(deps: &mut Deps, h: u64, who: &Addr, id: &str, text: &str, fee: u128) -> Result<Response, crate::ContractError> {
    execute(deps.as_mut(), at(h), message_info(who, &[coin(fee, "uterp")]),
        ext(SvgExecuteMsgExt::Propose { token_id: id.into(), text: text.into() }))
}
fn sends(r: &Response) -> Vec<(String, u128)> {
    r.messages.iter().filter_map(|m| match &m.msg {
        CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => Some((to_address.clone(), amount[0].amount.to_string().parse().unwrap())),
        _ => None }).collect()
}

#[test]
fn propose_select_render_and_split() {
    let (mut deps, admin, shield, holder) = setup();
    let (alice, bob) = (deps.api.addr_make("alice"), deps.api.addr_make("bob"));
    let before = uri(&deps, "1");
    let theme = THEMES.iter().find(|t| before.contains(&format!("<text y=\"38\">{t}</text>"))).expect("theme shown").to_string();
    let (cells, _) = cw_svg::marain::derived(&theme);
    assert!(before.contains(&format!("values=\"{}\"", cells[0])));

    propose(&mut deps, H0 + 1, &alice, "1", "light keeps its own counsel", 1000).unwrap();
    propose(&mut deps, H0 + 2, &bob, "1", "ships hum low", 1500).unwrap();
    let props: Vec<ProposalView> = q(&deps, H0 + 3, SvgQueryMsgExt::Proposals { token_id: "1".into(), epoch: 0 });
    assert_eq!(props.len(), 2);
    assert_eq!(props[1].proposer, bob);
    // cannot select during the epoch; only the shield owner selects
    let sel = |i| ext(SvgExecuteMsgExt::Select { token_id: "1".into(), epoch: 0, index: i });
    assert!(execute(deps.as_mut(), at(H0 + 50), message_info(&shield, &[]), sel(1)).unwrap_err().to_string().contains("has not ended"));
    for who in [&admin, &holder, &bob] {
        assert!(execute(deps.as_mut(), at(H0 + EB + 1), message_info(who, &[]), sel(1)).unwrap_err().to_string().contains("Unauthorized"));
    }
    let r = execute(deps.as_mut(), at(H0 + EB + 1), message_info(&shield, &[]), sel(1)).unwrap();
    // pot 2500: 70% to the orb owner, 30% to the original minter
    assert_eq!(sends(&r), vec![(holder.to_string(), 1750), (admin.to_string(), 750)]);
    let ev = r.events.iter().find(|e| e.ty == "scribe").unwrap();
    let get = |k: &str| ev.attributes.iter().find(|a| a.key == k).unwrap().value.clone();
    assert_eq!((get("text"), get("scribe"), get("theme"), get("epoch")), ("ships hum low".into(), bob.to_string(), theme.clone(), "0".into()));
    // theme kept, animation now spells the message
    let after = uri(&deps, "1");
    let (c2, d2) = cw_svg::marain::derived("ships hum low");
    assert!(after.contains(&format!("<text y=\"38\">{theme}</text>")));
    assert!(after.contains(&format!("values=\"{}\"", c2[4])) && after.contains(&format!("dur=\"{d2}s\"")));
    cw_svg::validate_animated_paint(&after).unwrap();
    assert!(!uri(&deps, "0").contains("ships"));
    // double select / settle rejected
    assert!(execute(deps.as_mut(), at(H0 + EB + 2), message_info(&shield, &[]), sel(0)).is_err());
    let m: Option<MessageRecord> = q(&deps, H0, SvgQueryMsgExt::Scribed { token_id: "1".into() });
    assert_eq!(m.unwrap().selected_by, shield);
    let h: Vec<MessageRecord> = q(&deps, H0, SvgQueryMsgExt::ScribeHistory { token_id: "1".into() });
    assert_eq!(h.len(), 1);
}

#[test]
fn shield_follows_ownership() {
    let (mut deps, _, shield, _) = setup();
    let alice = deps.api.addr_make("alice");
    let newshield = deps.api.addr_make("newshield");
    propose(&mut deps, H0 + 1, &alice, "1", "hello there", 1000).unwrap();
    execute(deps.as_mut(), at(H0 + 2), message_info(&shield, &[]),
        ExecuteMsg::TransferNft { recipient: newshield.to_string(), token_id: "0".into() }).unwrap();
    let sel = ext(SvgExecuteMsgExt::Select { token_id: "1".into(), epoch: 0, index: 0 });
    assert!(execute(deps.as_mut(), at(H0 + EB), message_info(&shield, &[]), sel.clone()).is_err());
    execute(deps.as_mut(), at(H0 + EB), message_info(&newshield, &[]), sel).unwrap();
}

#[test]
fn no_selection_settles_with_split_and_keeps_message() {
    let (mut deps, admin, shield, holder) = setup();
    let alice = deps.api.addr_make("alice");
    propose(&mut deps, H0 + 1, &alice, "1", "unheard", 2000).unwrap();
    let settle = ext(SvgExecuteMsgExt::Settle { token_id: "1".into(), epoch: 0 });
    // window (epoch 1) still open
    assert!(execute(deps.as_mut(), at(H0 + EB + 5), message_info(&alice, &[]), settle.clone()).unwrap_err().to_string().contains("still open"));
    // selection after the window is refused
    let sel = ext(SvgExecuteMsgExt::Select { token_id: "1".into(), epoch: 0, index: 0 });
    assert!(execute(deps.as_mut(), at(H0 + 2 * EB), message_info(&shield, &[]), sel).unwrap_err().to_string().contains("closed"));
    let before = uri(&deps, "1");
    let r = execute(deps.as_mut(), at(H0 + 2 * EB), message_info(&alice, &[]), settle.clone()).unwrap();
    assert_eq!(sends(&r), vec![(holder.to_string(), 1400), (admin.to_string(), 600)]);
    assert_eq!(uri(&deps, "1"), before);
    let pot: Option<EpochPot> = q(&deps, H0, SvgQueryMsgExt::EpochPot { token_id: "1".into(), epoch: 0 });
    assert_eq!(pot.unwrap().outcome, "unselected");
    assert!(execute(deps.as_mut(), at(H0 + 2 * EB), message_info(&alice, &[]), settle).is_err());
}

#[test]
fn owner_is_minter_gets_whole_pot() {
    let (mut deps, admin, shield, _) = setup();
    let alice = deps.api.addr_make("alice");
    // orb 0 (shield) minted by admin, owned by shield; give it back to admin
    execute(deps.as_mut(), at(H0), message_info(&shield, &[]),
        ExecuteMsg::TransferNft { recipient: admin.to_string(), token_id: "0".into() }).unwrap();
    propose(&mut deps, H0 + 1, &alice, "0", "home again", 1000).unwrap();
    let r = execute(deps.as_mut(), at(H0 + EB), message_info(&admin, &[]),
        ext(SvgExecuteMsgExt::Select { token_id: "0".into(), epoch: 0, index: 0 })).unwrap();
    assert_eq!(sends(&r), vec![(admin.to_string(), 1000)]);
}

#[test]
fn fee_and_spam_limits() {
    let (mut deps, _, _, _) = setup();
    let a = deps.api.addr_make("alice");
    assert!(propose(&mut deps, H0 + 1, &a, "1", "cheap", 999).unwrap_err().to_string().contains("at least"));
    let e = execute(deps.as_mut(), at(H0 + 1), message_info(&a, &[]), ext(SvgExecuteMsgExt::Propose { token_id: "1".into(), text: "free".into() })).unwrap_err();
    assert!(e.to_string().to_lowercase().contains("no funds"), "{e}");
    assert!(execute(deps.as_mut(), at(H0 + 1), message_info(&a, &[coin(5000, "uatom")]), ext(SvgExecuteMsgExt::Propose { token_id: "1".into(), text: "wrong".into() })).is_err());
    let long_glyphs = "a".repeat(41);
    for bad in ["", "Hello", "a<b", "x\" onload=\"y", "caf\u{e9}", "two  spaces", long_glyphs.as_str()] {
        let e = propose(&mut deps, H0 + 1, &a, "1", bad, 1000).unwrap_err();
        assert!(e.to_string().contains("Scribe text rejected"), "{bad:?}: {e}");
    }
    assert!(propose(&mut deps, H0 + 1, &a, "9", "no orb", 1000).is_err());
    for i in 0..3 { propose(&mut deps, H0 + 1, &a, "1", &format!("line {}", ["one","two","three"][i]), 1000).unwrap(); }
    assert!(propose(&mut deps, H0 + 1, &a, "1", "four", 1000).unwrap_err().to_string().contains("already has 3"));
    // next epoch resets the per-orb limit
    propose(&mut deps, H0 + EB, &a, "1", "four", 1000).unwrap();
}

#[test]
fn participants_registry_and_claim() {
    let (mut deps, _, _, _) = setup();
    let (a, b) = (deps.api.addr_make("alice"), deps.api.addr_make("bob"));
    propose(&mut deps, H0 + 1, &a, "1", "one", 1000).unwrap();
    propose(&mut deps, H0 + 1, &a, "1", "two", 1000).unwrap();
    propose(&mut deps, H0 + 1, &b, "0", "three", 1000).unwrap();
    let ps: Vec<ParticipantView> = q(&deps, H0, SvgQueryMsgExt::Participants { epoch: 0, limit: None });
    assert_eq!(ps.len(), 2);
    assert_eq!(ps.iter().find(|p| p.proposer == a).unwrap().proposals, 2);
    let claim = ext(SvgExecuteMsgExt::ClaimParticipation { token_id: "1".into(), epoch: 0 });
    assert!(execute(deps.as_mut(), at(H0 + 2), message_info(&a, &[]), claim.clone()).is_err());
    assert!(execute(deps.as_mut(), at(H0 + EB), message_info(&b, &[]), claim.clone()).is_err());
    let r = execute(deps.as_mut(), at(H0 + EB), message_info(&a, &[]), claim.clone()).unwrap();
    assert!(r.events.iter().any(|e| e.ty == "participant"));
    assert!(execute(deps.as_mut(), at(H0 + EB), message_info(&a, &[]), claim).unwrap_err().to_string().contains("already claimed"));
    let ps: Vec<ParticipantView> = q(&deps, H0, SvgQueryMsgExt::Participants { epoch: 0, limit: None });
    assert!(ps.iter().find(|p| p.proposer == a).unwrap().claimed);
}

#[test]
fn speed_owner_only_and_scales_dur() {
    let (mut deps, _, _, holder) = setup();
    let rando = deps.api.addr_make("rando");
    let sp = |s| ext(SvgExecuteMsgExt::SetSpeed { token_id: "1".into(), speed_pct: s });
    assert!(execute(deps.as_mut(), at(H0), message_info(&rando, &[]), sp(Some(200))).is_err());
    assert!(execute(deps.as_mut(), at(H0), message_info(&holder, &[]), sp(Some(500))).is_err());
    let before = uri(&deps, "1");
    let dur = before.split("dur=\"").nth(1).unwrap().split('s').next().unwrap().to_string();
    execute(deps.as_mut(), at(H0), message_info(&holder, &[]), sp(Some(200))).unwrap();
    let after = uri(&deps, "1");
    assert!(after.contains(&format!("dur=\"{}s\"", crate::scribing::scale_dur(&dur, 200))));
    let s: Option<u16> = q(&deps, H0, SvgQueryMsgExt::Speed { token_id: "1".into() });
    assert_eq!(s, Some(200));
    execute(deps.as_mut(), at(H0), message_info(&holder, &[]), sp(None)).unwrap();
    assert_eq!(uri(&deps, "1"), before);
    assert_eq!(crate::scribing::scale_dur("4.05", 200), "2.02");
    assert_eq!(crate::scribing::scale_dur("4.05", 50), "8.10");
}

#[test]
fn configure_admin_only_and_epoch_query() {
    let (mut deps, _, shield, _) = setup();
    let e: EpochInfo = q(&deps, H0 + EB + 7, SvgQueryMsgExt::ScribeEpoch {});
    assert_eq!((e.epoch, e.start_height, e.end_height), (1, H0 + EB, H0 + 2 * EB));
    assert!(execute(deps.as_mut(), at(H0), message_info(&shield, &[]), ext(SvgExecuteMsgExt::ConfigureScribing {
        epoch_blocks: 10, min_fee: coin(1, "uterp"), owner_bps: 5000, max_proposals: 3, shield_token_id: "0".into() })).unwrap_err().to_string().contains("Unauthorized"));
    let o: Option<Addr> = q(&deps, H0, SvgQueryMsgExt::OriginalMinter { token_id: "1".into() });
    assert_eq!(o, Some(deps.api.addr_make("admin")));
}

#[test]
fn instantiate_checks_scribe_vars_and_links() {
    let mut deps = mock_dependencies();
    let admin = deps.api.addr_make("admin");
    let (t, mut v) = template();
    v[0] = opts("scribe", &["Bad Theme"]);
    let e = instantiate(deps.as_mut(), mock_env(), message_info(&admin, &[]), inst_msg(&t, &v)).unwrap_err();
    assert!(e.to_string().contains("theme"), "{e}");
    let (t, mut v) = template();
    v.pop(); // drop scribe.dur
    let t2 = t.replace("${scribe.dur}", "1");
    assert!(instantiate(deps.as_mut(), mock_env(), message_info(&admin, &[]), inst_msg(&t2, &v)).is_err());
    let (t, v) = template();
    let t3 = t.replace("<text", "<a href=\"javascript:alert(1)\"><text").replace("</text>", "</text></a>");
    let e = instantiate(deps.as_mut(), mock_env(), message_info(&admin, &[]), inst_msg(&t3, &v)).unwrap_err();
    assert!(e.to_string().contains("Unsafe link"), "{e}");
}
