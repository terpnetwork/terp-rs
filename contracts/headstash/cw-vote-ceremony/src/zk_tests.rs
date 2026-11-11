//! zk proposal module tests. The host verifier is a registered toy circuit (it accepts
//! `proof == TAG || instances[0..32]`), so these tests check every binding the module
//! adds around `proof_instance_verify`; real vote-sdk proofs are an e2e concern.
use crate::contract::{execute, instantiate, query};
use crate::msg::*;
use crate::zk::{self, slot};
use cosmwasm_std::testing::{
    MockApi, MockQuerier, MockStorage, clear_test_circuits, message_info, mock_dependencies,
    mock_env, register_test_circuit,
};
use cosmwasm_std::{
    Addr, Api, Binary, ContractResult, CosmosMsg, Env, HexBinary, OwnedDeps, SystemResult,
    Timestamp, Uint128, WasmMsg, WasmQuery, from_json, to_json_binary,
};
use frost_ed25519 as frost;
use rand010::SeedableRng;
use std::collections::BTreeMap;

type D = OwnedDeps<MockStorage, MockApi, MockQuerier>;
const CAST_ZKID: u64 = 7;
const TALLY_ZKID: u64 = 9;
const T0: u64 = 2_000_000;
const PERIOD: u64 = 86_400;
const ROOT: [u8; 32] = [0x11; 32];
const EA: [u8; 64] = [0x22; 64];

fn good_key() -> Vec<u8> {
    let p = zk::cast_appstate_prefix();
    let mut k = Vec::new();
    k.extend_from_slice(&p);
    k.extend_from_slice(&[0x33; 32]);
    k.extend_from_slice(&p);
    k.extend_from_slice(&[0x44; 32]);
    k
}

struct W {
    d: D,
    dao: Addr,
    alice: Addr,
    relayer: Addr,
    tally_keys: BTreeMap<frost::Identifier, frost::keys::KeyPackage>,
    tally_pub: frost::keys::PublicKeyPackage,
}

fn env_at(t: u64) -> Env {
    let mut e = mock_env();
    e.block.time = Timestamp::from_seconds(t);
    e
}

fn querier(d: &mut D, anchor: Option<Addr>) {
    let api = MockApi::default();
    let (dao, voting, alice) = (
        api.addr_make("dao"),
        api.addr_make("voting"),
        api.addr_make("alice"),
    );
    let anchor = anchor.map(|a| a.to_string());
    d.querier.update_wasm(move |q| {
        let ok = |b: Binary| SystemResult::Ok(ContractResult::Ok(b));
        match q {
            WasmQuery::Smart { contract_addr, msg } => {
                let v: serde_json::Value = from_json(msg).unwrap();
                if *contract_addr == dao.to_string() && v.get("voting_module").is_some() {
                    return ok(to_json_binary(&voting).unwrap());
                }
                if *contract_addr == voting.to_string() {
                    if let Some(a) = v.get("voting_power_at_height") {
                        let p = if a["address"] == alice.as_str() {
                            10u128
                        } else {
                            0
                        };
                        return ok(to_json_binary(
                            &serde_json::json!({"power": p.to_string(), "height": 12345}),
                        )
                        .unwrap());
                    }
                    if v.get("total_power_at_height").is_some() {
                        return ok(to_json_binary(
                            &serde_json::json!({"power": "100", "height": 12345}),
                        )
                        .unwrap());
                    }
                }
                if Some(contract_addr.clone()) == anchor {
                    let a = &v["get_anchor_at_height"];
                    return ok(to_json_binary(&serde_json::json!({
                        "session_id": a["session_id"], "height": a["height"],
                        "root": Binary::from(ROOT.to_vec())
                    }))
                    .unwrap());
                }
                SystemResult::Ok(ContractResult::Err(format!("unexpected query {v}")))
            }
            WasmQuery::CircuitInfo { zk_id } => ok(to_json_binary(&serde_json::json!({
                "zk_id": zk_id,
                "creator": api.addr_make("uploader"),
                "circuit_key": Binary::from(good_key()),
            }))
            .unwrap()),
            _ => SystemResult::Ok(ContractResult::Err("unsupported".into())),
        }
    });
}

fn toy_circuits() {
    clear_test_circuits();
    register_test_circuit(
        CAST_ZKID,
        "vote_proof_toy",
        Box::new(|proof, inst| {
            Ok(inst.len() == 352
                && proof.len() == 40
                && proof[..8] == [0xAA; 8]
                && proof[8..] == inst[..32])
        }),
    );
    register_test_circuit(
        TALLY_ZKID,
        "tally_toy",
        Box::new(|proof, inst| {
            Ok(inst.len() == 192 && proof[..5] == *b"tally" && proof[5..] == inst[..32])
        }),
    );
}

fn init_msg(
    w_dao: Option<String>,
    tally: TallyAuth,
    key: Option<Vec<u8>>,
    anchor: Option<String>,
) -> InstantiateMsg {
    InstantiateMsg {
        admin: None,
        max_nullifiers_per_mark: 8,
        zk_proposals: Some(ZkModuleInit {
            dao: w_dao,
            voting_module: None,
            cast_zkid: CAST_ZKID,
            cast_circuit_key: key.map(HexBinary::from),
            anchor_registry: anchor,
            tally,
            voting_period: PERIOD,
            threshold_bps: 5_000,
            quorum_bps: 3_000,
            require_rvpk_sig: true,
        }),
    }
}

fn setup_with(tally_circuit: bool, anchor: bool) -> W {
    toy_circuits();
    let mut d = mock_dependencies();
    let (dao, alice, relayer) = (
        d.api.addr_make("dao"),
        d.api.addr_make("alice"),
        d.api.addr_make("relayer"),
    );
    let anchor_addr = anchor.then(|| d.api.addr_make("anchor"));
    querier(&mut d, anchor_addr.clone());
    let rng = rand08::rngs::OsRng;
    let (shares, tally_pub) =
        frost::keys::generate_with_dealer(3, 2, frost::keys::IdentifierList::Default, rng).unwrap();
    let tally_keys: BTreeMap<_, _> = shares
        .into_iter()
        .map(|(id, s)| (id, frost::keys::KeyPackage::try_from(s).unwrap()))
        .collect();
    let tally = if tally_circuit {
        TallyAuth::Circuit {
            zkid: TALLY_ZKID,
            circuit_key: None,
        }
    } else {
        TallyAuth::Committee {
            group_vk: HexBinary::from(tally_pub.verifying_key().serialize().unwrap()),
        }
    };
    // dao-core instantiates the module: dao = sender, voting module read from dao-core.
    instantiate(
        d.as_mut(),
        env_at(T0),
        message_info(&dao, &[]),
        init_msg(
            None,
            tally,
            Some(good_key()),
            anchor_addr.map(|a| a.to_string()),
        ),
    )
    .unwrap();
    W {
        d,
        dao,
        alice,
        relayer,
        tally_keys,
        tally_pub,
    }
}

fn propose(w: &mut W, anchor: AnchorRef) -> Result<cosmwasm_std::Response, crate::ContractError> {
    let alice = w.alice.clone();
    execute(
        w.d.as_mut(),
        env_at(T0 + 1),
        message_info(&alice, &[]),
        ExecuteMsg::ZkPropose {
            title: "Fund the ZEC corridor".into(),
            description: "private vote".into(),
            msgs: vec![CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
                to_address: alice.to_string(),
                amount: cosmwasm_std::coins(1, "uthiol"),
            })],
            ea_pk: HexBinary::from(EA.to_vec()),
            anchor,
        },
    )
}

struct Voter {
    sk: reddsa::SigningKey<reddsa::orchard::SpendAuth>,
    rk: [u8; 32],
    rng: rand010::rngs::StdRng,
}
fn voter(seed: u64) -> Voter {
    let mut rng = rand010::rngs::StdRng::seed_from_u64(seed);
    let sk = reddsa::SigningKey::<reddsa::orchard::SpendAuth>::new(&mut rng);
    let rk: [u8; 32] = reddsa::VerificationKey::from(&sk).into();
    Voter { sk, rk, rng }
}

fn instances(w: &W, nf: u8, v: &Voter, tweak: Option<(usize, [u8; 32])>) -> Vec<u8> {
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            env_at(T0),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    let mut x = v.rk;
    x[31] &= 0x7f;
    let mut y = [0u8; 32];
    y[0] = v.rk[31] >> 7;
    let mut s: Vec<[u8; 32]> = vec![
        zk::to_fe([nf; 32]),
        x,
        y,
        zk::to_fe([3; 32]),
        zk::to_fe([nf ^ 0x55; 32]),
        ROOT,
        zk::fe_u64(p.anchor_height),
        zk::fe_u64(zk::QUESTION_INDEX),
        p.round_id.as_slice().try_into().unwrap(),
        EA[..32].try_into().unwrap(),
        EA[32..].try_into().unwrap(),
    ];
    if let Some((i, val)) = tweak {
        s[i] = val;
    }
    s.concat()
}

fn cast(
    w: &mut W,
    v: &mut Voter,
    inst: Vec<u8>,
    ballot: &[u8],
    good_proof: bool,
) -> Result<cosmwasm_std::Response, crate::ContractError> {
    let env = env_at(T0 + 100);
    let msg = zk::cast_sighash(
        &w.d.api,
        &env.block.chain_id,
        env.contract.address.as_str(),
        1,
        &inst[..32],
        &inst[slot::VOTE_COMMITMENT * 32..slot::VOTE_COMMITMENT * 32 + 32],
        ballot,
    );
    let sig: [u8; 64] = v.sk.sign(&mut v.rng, &msg).into();
    let mut proof = vec![0xAA; 8];
    proof.extend_from_slice(&inst[..32]);
    if !good_proof {
        proof[0] = 0;
    }
    let relayer = w.relayer.clone();
    execute(
        w.d.as_mut(),
        env,
        message_info(&relayer, &[]),
        ExecuteMsg::ZkCastVote {
            proposal_id: 1,
            proof: Binary::from(proof),
            instances: Binary::from(inst),
            ballot: Binary::from(ballot.to_vec()),
            rvpk_sig: Some(HexBinary::from(sig.to_vec())),
        },
    )
}

fn committee_sign(w: &W, digest: &[u8]) -> HexBinary {
    let mut rng = rand08::rngs::OsRng;
    let ids: Vec<_> = w.tally_keys.keys().take(2).cloned().collect();
    let mut nonces = BTreeMap::new();
    let mut commits = BTreeMap::new();
    for id in &ids {
        let (n, c) = frost::round1::commit(w.tally_keys[id].signing_share(), &mut rng);
        nonces.insert(*id, n);
        commits.insert(*id, c);
    }
    let pkg = frost::SigningPackage::new(commits, digest);
    let shares: BTreeMap<_, _> = ids
        .iter()
        .map(|id| {
            (
                *id,
                frost::round2::sign(&pkg, &nonces[id], &w.tally_keys[id]).unwrap(),
            )
        })
        .collect();
    HexBinary::from(
        frost::aggregate(&pkg, &shares, &w.tally_pub)
            .unwrap()
            .serialize()
            .unwrap(),
    )
}

fn tally(
    w: &mut W,
    t: u64,
    y: u128,
    n: u128,
    a: u128,
) -> Result<cosmwasm_std::Response, crate::ContractError> {
    let d: HexBinary = from_json(
        query(
            w.d.as_ref(),
            env_at(t),
            QueryMsg::ZkTallyDigest {
                proposal_id: 1,
                yes: y.into(),
                no: n.into(),
                abstain: a.into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let signature = committee_sign(w, d.as_slice());
    let r = w.relayer.clone();
    execute(
        w.d.as_mut(),
        env_at(t),
        message_info(&r, &[]),
        ExecuteMsg::ZkSubmitTally {
            proposal_id: 1,
            yes: y.into(),
            no: n.into(),
            abstain: a.into(),
            attestation: TallyAttestation::Committee { signature },
        },
    )
}

#[test]
fn added_to_an_existing_dao_as_a_proposal_module() {
    let w = setup_with(false, false);
    let dao: Addr = from_json(query(w.d.as_ref(), mock_env(), QueryMsg::Dao {}).unwrap()).unwrap();
    assert_eq!(dao, w.dao);
    let c: ZkConfig =
        from_json(query(w.d.as_ref(), mock_env(), QueryMsg::ZkConfig {}).unwrap()).unwrap();
    assert_eq!(c.voting_module, w.d.api.addr_make("voting"));
    let i: InfoResponse =
        from_json(query(w.d.as_ref(), mock_env(), QueryMsg::Info {}).unwrap()).unwrap();
    assert_eq!(i.info.contract, "crates.io:cw-vote-ceremony");
    let n: u64 =
        from_json(query(w.d.as_ref(), mock_env(), QueryMsg::NextProposalId {}).unwrap()).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn stale_or_mismatched_circuit_footer_is_rejected() {
    toy_circuits();
    let mut d = mock_dependencies();
    querier(&mut d, None);
    let dao = d.api.addr_make("dao");
    let tally = TallyAuth::Committee {
        group_vk: HexBinary::from(vec![1u8; 32]),
    };
    // zk-vote-bridge's older footer put the circuit id in prover_id (2 = Stwo's id).
    let mut stale = good_key();
    let bad = u32::from_be_bytes([2, 0, 14, 0]).to_le_bytes();
    stale[..4].copy_from_slice(&bad);
    stale[36..40].copy_from_slice(&bad);
    let e = instantiate(
        d.as_mut(),
        env_at(T0),
        message_info(&dao, &[]),
        init_msg(None, tally.clone(), Some(stale), None),
    )
    .unwrap_err();
    assert!(e.to_string().contains("footer prefix"), "{e}");
    let mut other = good_key();
    other[10] ^= 1;
    let e = instantiate(
        d.as_mut(),
        env_at(T0),
        message_info(&dao, &[]),
        init_msg(None, tally, Some(other), None),
    )
    .unwrap_err();
    assert!(e.to_string().contains("does not match"), "{e}");
}

#[test]
fn full_private_vote_flow() {
    let mut w = setup_with(false, false);
    propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(ROOT.to_vec()),
            height: 777,
        },
    )
    .unwrap();
    let mut voters: Vec<Voter> = (1..=3).map(voter).collect();
    for (i, v) in voters.iter_mut().enumerate() {
        let inst = instances(&w, i as u8 + 1, v, None);
        let r = cast(&mut w, v, inst, b"enc-shares", true).unwrap();
        assert!(!format!("{:?}", r.events).contains(w.relayer.as_str()));
    }
    // Same VAN nullifier again: rejected by the shared spent map.
    let inst = instances(&w, 1, &voters[0], None);
    let e = cast(&mut w, &mut voters[0], inst, b"x", true).unwrap_err();
    assert!(e.to_string().contains("already spent"), "{e}");
    let spent: IsSpentResponse = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::IsSpent {
                domain: zk::ZK_DOMAIN.into(),
                session_id: "zkp-1".into(),
                nullifier: Binary::from(zk::to_fe([1; 32]).to_vec()),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(spent.spent);
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p.ballots, 3);
    // Too early to tally.
    assert!(
        tally(&mut w, T0 + 50, 1, 0, 0)
            .unwrap_err()
            .to_string()
            .contains("not ended")
    );
    tally(&mut w, T0 + 1 + PERIOD, 25, 5, 2).unwrap();
    let r = execute(
        w.d.as_mut(),
        env_at(T0 + PERIOD + 2),
        message_info(&w.relayer.clone(), &[]),
        ExecuteMsg::ZkExecute { proposal_id: 1 },
    )
    .unwrap();
    match &r.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr, msg, ..
        }) => {
            assert_eq!(contract_addr, &w.dao.to_string());
            assert!(String::from_utf8_lossy(msg).contains("execute_proposal_hook"));
        }
        m => panic!("{m:?}"),
    }
    let c: CeremonyResponse = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::Ceremony {
                session_id: "zkp-1".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(c.ceremony.status, CeremonyStatus::Closed);
    assert!(
        execute(
            w.d.as_mut(),
            env_at(T0 + PERIOD + 3),
            message_info(&w.relayer.clone(), &[]),
            ExecuteMsg::ZkExecute { proposal_id: 1 }
        )
        .is_err()
    );
}

#[test]
fn cast_bindings_are_enforced() {
    let mut w = setup_with(false, false);
    propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(ROOT.to_vec()),
            height: 777,
        },
    )
    .unwrap();
    let mut v = voter(9);
    for (s, want) in [
        (slot::TREE_ROOT, "tree_root"),
        (slot::ANCHOR_HEIGHT, "anchor height"),
        (slot::PROPOSAL_ID, "question index"),
        (slot::ROUND_ID, "round_id"),
        (slot::EA_PK_X, "election authority"),
    ] {
        let inst = instances(&w, 40, &v, Some((s, [0x01; 32])));
        let e = cast(&mut w, &mut v, inst, b"b", true).unwrap_err();
        assert!(e.to_string().contains(want), "{s}: {e}");
    }
    // Proof the host rejects.
    let inst = instances(&w, 41, &v, None);
    assert!(
        cast(&mut w, &mut v, inst, b"b", false)
            .unwrap_err()
            .to_string()
            .contains("rejected")
    );
    // r_vpk signature from another key: the ballot is not bound to this proof.
    let mut other = voter(10);
    let inst = instances(&w, 42, &v, None);
    let e = cast(&mut w, &mut other, inst, b"b", true).unwrap_err();
    assert!(e.to_string().contains("rvpk_sig"), "{e}");
    // Wrong length.
    let r = w.relayer.clone();
    let e = execute(
        w.d.as_mut(),
        env_at(T0 + 100),
        message_info(&r, &[]),
        ExecuteMsg::ZkCastVote {
            proposal_id: 1,
            proof: Binary::default(),
            instances: Binary::from(vec![0u8; 320]),
            ballot: Binary::default(),
            rvpk_sig: None,
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("352"));
}

#[test]
fn only_members_propose_and_quorum_rejects() {
    let mut w = setup_with(false, false);
    let bob = w.d.api.addr_make("bob");
    let e = execute(
        w.d.as_mut(),
        env_at(T0),
        message_info(&bob, &[]),
        ExecuteMsg::ZkPropose {
            title: "t".into(),
            description: "d".into(),
            msgs: vec![],
            ea_pk: HexBinary::from(EA.to_vec()),
            anchor: AnchorRef::Root {
                root: HexBinary::from(ROOT.to_vec()),
                height: 1,
            },
        },
    )
    .unwrap_err();
    assert!(matches!(e, crate::ContractError::Unauthorized));
    propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(ROOT.to_vec()),
            height: 777,
        },
    )
    .unwrap();
    assert!(
        tally(&mut w, T0 + PERIOD + 1, 90, 20, 0)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    // 20% turnout < 30% quorum.
    tally(&mut w, T0 + PERIOD + 1, 20, 0, 0).unwrap();
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p.status, ZkStatus::Rejected);
    assert!(
        execute(
            w.d.as_mut(),
            env_at(T0 + PERIOD + 2),
            message_info(&bob, &[]),
            ExecuteMsg::ZkExecute { proposal_id: 1 }
        )
        .is_err()
    );
}

#[test]
fn forged_tally_signature_rejected() {
    let mut w = setup_with(false, false);
    propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(ROOT.to_vec()),
            height: 777,
        },
    )
    .unwrap();
    let d: HexBinary = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkTallyDigest {
                proposal_id: 1,
                yes: 40u128.into(),
                no: 0u128.into(),
                abstain: 0u128.into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let sig = committee_sign(&w, d.as_slice());
    // Signature for 40 yes submitted as 50 yes.
    let r = w.relayer.clone();
    let e = execute(
        w.d.as_mut(),
        env_at(T0 + PERIOD + 1),
        message_info(&r, &[]),
        ExecuteMsg::ZkSubmitTally {
            proposal_id: 1,
            yes: 50u128.into(),
            no: Uint128::zero(),
            abstain: Uint128::zero(),
            attestation: TallyAttestation::Committee { signature: sig },
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("signature invalid"));
}

#[test]
fn tally_circuit_path() {
    let mut w = setup_with(true, false);
    propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(ROOT.to_vec()),
            height: 777,
        },
    )
    .unwrap();
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    let inst = zk::tally_instances(&p, 31u128.into(), 1u128.into(), 0u128.into());
    let mut proof = b"tally".to_vec();
    proof.extend_from_slice(&inst[..32]);
    let r = w.relayer.clone();
    execute(
        w.d.as_mut(),
        env_at(T0 + PERIOD + 1),
        message_info(&r, &[]),
        ExecuteMsg::ZkSubmitTally {
            proposal_id: 1,
            yes: 31u128.into(),
            no: 1u128.into(),
            abstain: Uint128::zero(),
            attestation: TallyAttestation::Proof {
                proof: Binary::from(proof),
            },
        },
    )
    .unwrap();
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p.status, ZkStatus::Passed);
}

#[test]
fn anchor_registry_is_authoritative() {
    let mut w = setup_with(false, true);
    let e = propose(
        &mut w,
        AnchorRef::Root {
            root: HexBinary::from(vec![9u8; 32]),
            height: 1,
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("anchor registry"));
    propose(
        &mut w,
        AnchorRef::Registry {
            session_id: "tree-1".into(),
            height: 777,
        },
    )
    .unwrap();
    let p: ZkProposal = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::ZkProposal { proposal_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p.tree_root.as_slice(), &ROOT);
    assert_eq!(p.anchor_height, 777);
}

#[test]
fn round_id_is_canonical_and_bound_to_contract() {
    let api = MockApi::default();
    let a = zk::round_id(&api, "terp-1", "terp1aaa", 1);
    assert_eq!(a[31] & 0xc0, 0);
    assert_ne!(a, zk::round_id(&api, "terp-1", "terp1bbb", 1));
    assert_ne!(a, zk::round_id(&api, "terp-1", "terp1aaa", 2));
    assert_eq!(zk::cast_appstate_prefix(), [0x00, 11, 2, 0]);
    let _ = api.blake2b_256(b"");
}
