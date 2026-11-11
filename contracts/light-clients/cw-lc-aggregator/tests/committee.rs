//! Registry + rotation + aggregator on cw-multi-test: dynamic weighted
//! committee, proactive preconfirmation, confirmation, equivocation,
//! planned rotation with hand-off (FROST → TC01), liveness fallback.
use cosmwasm_std::{Addr, Binary, Empty, StdError};

type Res = Result<AppResponse, StdError>;
use cw_multi_test::{App, AppResponse, Contract, ContractWrapper, Executor};
use ed25519_dalek::{Signer, SigningKey};
use frost_escrow::FrostLabCommittee;
use threshold_committee::{CombinedSignature, LabCommittee};

use cw_lc_aggregator::msg as agg;
use cw_lc_registry::msg as reg;
use cw_lc_rotation::committee::CommitteePolicy;
use cw_lc_rotation::msg as rot;
use cw_lc_rotation::state::{Epoch, EpochStatus};

const CLIENT: &str = "zcash-regtest-0";

fn registry() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new_with_empty(
        cw_lc_registry::contract::execute,
        cw_lc_registry::contract::instantiate,
        cw_lc_registry::contract::query,
    ))
}
fn rotation() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new_with_empty(
        cw_lc_rotation::contract::execute,
        cw_lc_rotation::contract::instantiate,
        cw_lc_rotation::contract::query,
    ))
}
fn aggregator() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new_with_empty(
        cw_lc_aggregator::contract::execute,
        cw_lc_aggregator::contract::instantiate,
        cw_lc_aggregator::contract::query,
    ))
}

fn h32(b: u8) -> String {
    hex::encode([b; 32])
}

/// Header hash for height h on fork f (distinct per (f, h)).
fn hh(f: u8, h: u64) -> String {
    let mut b = [f; 32];
    b[..8].copy_from_slice(&h.to_be_bytes());
    hex::encode(b)
}

struct Op {
    addr: Addr,
    key: SigningKey,
}

enum Signer_ {
    Frost(FrostLabCommittee),
    Tc(LabCommittee),
}

impl Signer_ {
    fn key(&self) -> Binary {
        match self {
            Signer_::Frost(c) => Binary::from(c.encoded_public.clone()),
            Signer_::Tc(c) => Binary::from(c.encoded_public.clone()),
        }
    }
    /// Sign with the shares held by `online` members of `e`.
    fn sign(&self, e: &Epoch, online: &[&Addr], digest: &[u8]) -> Result<Binary, String> {
        let ids: Vec<u16> = e
            .members
            .iter()
            .filter(|m| online.contains(&&m.operator))
            .flat_map(|m| m.share_ids().collect::<Vec<_>>())
            .collect();
        match self {
            Signer_::Frost(c) => c
                .sign_message_with(digest, &ids)
                .map(Binary::from)
                .map_err(|e| e.to_string()),
            Signer_::Tc(c) => {
                let d: [u8; 32] = digest.try_into().unwrap();
                if (ids.len() as u16) < c.roster.threshold {
                    return Err("below threshold".into());
                }
                let partials = ids
                    .iter()
                    .map(|i| c.share(i - 1).unwrap().partial_sign_digest(&d).unwrap())
                    .collect();
                Ok(Binary::from(CombinedSignature { partials }.encode()))
            }
        }
    }
}

struct World {
    app: App,
    admin: Addr,
    reg: Addr,
    rot: Addr,
    agg: Addr,
    ops: Vec<Op>,
    clock: u64,
}

impl World {
    /// `(name, weight, advertises preconf)`
    fn new(spec: &[(&str, u64, bool)]) -> Self {
        let mut app = App::default();
        let admin = app.api().addr_make("admin");
        let (rc, oc, ac) = (
            app.store_code(registry()),
            app.store_code(rotation()),
            app.store_code(aggregator()),
        );
        let reg = app
            .instantiate_contract(
                rc,
                admin.clone(),
                &reg::InstantiateMsg {
                    admin: None,
                    label: "terp/lc/registry/zcash-regtest/v1".into(),
                    hash_market_capability: None,
                    max_report_age_blocks: 100,
                },
                &[],
                "terp/lc/registry/zcash-regtest/v1",
                None,
            )
            .unwrap();
        let mut ops = Vec::new();
        for (i, (name, weight, preconf)) in spec.iter().enumerate() {
            let addr = app.api().addr_make(name);
            let key = SigningKey::from_bytes(&[i as u8 + 1; 32]);
            let mut caps = vec!["node-state".to_string()];
            if *preconf {
                caps.push("preconf".into());
            }
            app.execute_contract(
                addr.clone(),
                reg.clone(),
                &reg::ExecuteMsg::RegisterOperator {
                    peer_id: format!("12D3Koo{name}"),
                    endpoint: format!("/ip4/127.0.0.1/tcp/{}", 18230 + i),
                    pubkey: Binary::from(key.verifying_key().to_bytes().to_vec()),
                    capabilities: caps,
                },
                &[],
            )
            .unwrap();
            app.execute_contract(
                admin.clone(),
                reg.clone(),
                &reg::ExecuteMsg::SetOperatorWeight {
                    operator: addr.to_string(),
                    weight: *weight,
                },
                &[],
            )
            .unwrap();
            ops.push(Op { addr, key });
        }
        app.execute_contract(
            admin.clone(),
            reg.clone(),
            &reg::ExecuteMsg::RegisterClient {
                client_id: CLIENT.into(),
                kind: reg::ClientKind::Ics08Wasm,
                target: reg::TargetChain {
                    chain_id: "zcash-regtest".into(),
                    algo: reg::PowAlgo::Equihash { n: 200, k: 9 },
                    target_block_time_secs: 25,
                },
                attestors: ops.iter().map(|o| o.addr.to_string()).collect(),
                min_attestations: 3,
                ibc_client_id: None,
            },
            &[],
        )
        .unwrap();
        let rot = app
            .instantiate_contract(
                oc,
                admin.clone(),
                &rot::InstantiateMsg {
                    admin: None,
                    label: "terp/lc/rotation/zcash-regtest/v1".into(),
                    registry: reg.to_string(),
                    client_id: CLIENT.into(),
                    policy: Some(CommitteePolicy {
                        active_window_blocks: 20,
                        dkg_timeout_blocks: 50,
                        ..CommitteePolicy::default()
                    }),
                },
                &[],
                "terp/lc/rotation/zcash-regtest/v1",
                None,
            )
            .unwrap();
        let agg = app
            .instantiate_contract(
                ac,
                admin.clone(),
                &agg::InstantiateMsg {
                    label: "terp/lc/aggregator/zcash-regtest/v1".into(),
                    registry: reg.to_string(),
                    rotation: rot.to_string(),
                    client_id: CLIENT.into(),
                    confirm_depth: 2,
                    stall_blocks: 3,
                    max_confirm_walk: None,
                },
                &[],
                "terp/lc/aggregator/zcash-regtest/v1",
                None,
            )
            .unwrap();
        app.execute_contract(
            admin.clone(),
            rot.clone(),
            &rot::ExecuteMsg::SetAggregator {
                aggregator: agg.to_string(),
            },
            &[],
        )
        .unwrap();
        World {
            app,
            admin,
            reg,
            rot,
            agg,
            ops,
            clock: 1,
        }
    }

    fn addr(&self, i: usize) -> Addr {
        self.ops[i].addr.clone()
    }

    /// Operators `who` report node state tip (fork f, height h).
    fn report(&mut self, who: &[usize], f: u8, h: u64) {
        self.clock += 1;
        for &i in who {
            let state = reg::NodeState {
                tip_height: h,
                tip_hash: hh(f, h),
                prev_hash: hh(f, h - 1),
                chain_work: "0fff".into(),
                finalized_height: None,
                peer_count: 4,
                observed_at_unix: self.clock,
            };
            let d: Binary = self
                .app
                .wrap()
                .query_wasm_smart(
                    &self.reg,
                    &reg::QueryMsg::ReportDigest {
                        client_id: CLIENT.into(),
                        operator: self.ops[i].addr.to_string(),
                        state: state.clone(),
                    },
                )
                .unwrap();
            let sig = self.ops[i].key.sign(d.as_slice()).to_bytes().to_vec();
            self.app
                .execute_contract(
                    self.ops[i].addr.clone(),
                    self.reg.clone(),
                    &reg::ExecuteMsg::ReportNodeState {
                        client_id: CLIENT.into(),
                        state,
                        signature: Binary::from(sig),
                    },
                    &[],
                )
                .unwrap();
        }
    }

    fn rot_exec(&mut self, who: &Addr, msg: &rot::ExecuteMsg) -> Res {
        self.app
            .execute_contract(who.clone(), self.rot.clone(), msg, &[])
    }

    fn epoch(&self, q: rot::QueryMsg) -> Option<Epoch> {
        self.app.wrap().query_wasm_smart(&self.rot, &q).unwrap()
    }

    /// Ack + activate `e` with `signer`; `online` sign the proof, `handoff`
    /// (old signer + its epoch) signs the hand-off.
    fn activate(
        &mut self,
        e: &Epoch,
        signer: &Signer_,
        ackers: &[Addr],
        online: &[&Addr],
        handoff: Option<(&Signer_, &Epoch)>,
    ) -> Res {
        for a in ackers {
            self.rot_exec(
                a,
                &rot::ExecuteMsg::AckEpoch {
                    epoch: e.epoch,
                    group_key: signer.key(),
                },
            )?;
        }
        let d: Binary = self.app.wrap().query_wasm_smart(
            &self.rot,
            &rot::QueryMsg::ActivationDigest {
                epoch: e.epoch,
                group_key: signer.key(),
            },
        )?;
        let proof_sig = signer.sign(e, online, d.as_slice()).unwrap();
        let handoff_sig = handoff.map(|(s, old)| {
            let all: Vec<&Addr> = old.members.iter().map(|m| &m.operator).collect();
            s.sign(old, &all, d.as_slice()).unwrap()
        });
        let admin = self.admin.clone();
        self.rot_exec(
            &admin,
            &rot::ExecuteMsg::ActivateEpoch {
                epoch: e.epoch,
                group_key: signer.key(),
                proof_sig,
                handoff_sig,
            },
        )
    }

    fn header(f: u8, h: u64) -> agg::Header {
        agg::Header {
            height: h,
            hash: hh(f, h),
            prev_hash: hh(f, h - 1),
            chain_work: "0fff".into(),
            payment_root: None,
        }
    }

    fn preconf(
        &mut self,
        e: &Epoch,
        signer: &Signer_,
        online: &[&Addr],
        header: agg::Header,
    ) -> Res {
        let d: Binary = self.app.wrap().query_wasm_smart(
            &self.agg,
            &agg::QueryMsg::PreconfDigest {
                epoch: e.epoch,
                header: header.clone(),
            },
        )?;
        let signature = signer
            .sign(e, online, d.as_slice())
            .map_err(StdError::msg)?;
        let relayer = self.app.api().addr_make("relayer");
        self.app.execute_contract(
            relayer,
            self.agg.clone(),
            &agg::ExecuteMsg::SubmitPreconf {
                epoch: e.epoch,
                header,
                signature,
            },
            &[],
        )
    }

    fn best(&self) -> agg::BestHeaderResponse {
        self.app
            .wrap()
            .query_wasm_smart(&self.agg, &agg::QueryMsg::BestHeader {})
            .unwrap()
    }

    fn advance(&mut self, n: u64) {
        self.app.update_block(|b| b.height += n);
    }
}

fn err_str(r: Res) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn weighted_committee_preconf_confirm_and_equivocation() {
    // e: weight 2. d: no `preconf` capability. f: weight 0.
    let mut w = World::new(&[
        ("op-a", 1, true),
        ("op-b", 1, true),
        ("op-c", 1, true),
        ("op-d", 3, false),
        ("op-e", 2, true),
        ("op-f", 0, true),
    ]);
    let all: Vec<usize> = (0..6).collect();
    w.report(&all, 0xa0, 99);
    let admin = w.admin.clone();

    // No committee yet → fallback to registry k-of-n.
    assert_eq!(w.best().source, agg::HeaderSource::RegistryFallback);

    w.rot_exec(&admin, &rot::ExecuteMsg::Bootstrap {}).unwrap();
    let e1 = w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap();
    let members: Vec<_> = e1
        .members
        .iter()
        .map(|m| (m.operator.clone(), m.shares))
        .collect();
    assert_eq!(members.len(), 4, "a, b, c, e (d lacks preconf, f weight 0)");
    assert_eq!((e1.total_shares, e1.threshold), (5, 4), "ceil(2/3 * 5)");

    let frost = Signer_::Frost(FrostLabCommittee::dkg(4, 5).unwrap());
    let (a, b, c, e) = (w.addr(0), w.addr(1), w.addr(2), w.addr(4));

    // Non-member ack rejected.
    let d = w.addr(3);
    assert!(
        err_str(w.rot_exec(
            &d,
            &rot::ExecuteMsg::AckEpoch {
                epoch: 1,
                group_key: frost.key()
            }
        ))
        .contains("not a member")
    );
    // Wrong (t, n) key rejected.
    let wrong = Signer_::Frost(FrostLabCommittee::dkg(3, 5).unwrap());
    assert!(
        err_str(w.activate(
            &e1,
            &wrong,
            &[a.clone(), b.clone(), c.clone(), e.clone()],
            &[&a, &b, &c, &e],
            None
        ))
        .contains("do not match")
    );
    // 3 of 5 shares acked (a, b, c) → below threshold.
    assert!(
        err_str(w.activate(
            &e1,
            &frost,
            &[a.clone(), b.clone(), c.clone()],
            &[&a, &b, &e],
            None
        ))
        .contains("below threshold")
    );
    // e (2 shares) acks → 5 ≥ 4; proof signed by b + c + e (4 shares).
    w.activate(&e1, &frost, std::slice::from_ref(&e), &[&b, &c, &e], None)
        .unwrap();
    let e1 = w.epoch(rot::QueryMsg::ActiveEpoch {}).unwrap();
    assert_eq!(e1.status, EpochStatus::Active);

    // Proactive preconf of 100..=103 by different online subsets.
    w.preconf(&e1, &frost, &[&a, &b, &e], W::header(0xa0, 100))
        .unwrap();
    w.preconf(&e1, &frost, &[&b, &c, &e], W::header(0xa0, 101))
        .unwrap();
    w.preconf(&e1, &frost, &[&a, &c, &e], W::header(0xa0, 102))
        .unwrap();
    w.preconf(&e1, &frost, &[&a, &b, &c, &e], W::header(0xa0, 103))
        .unwrap();
    // a + b + c = 3 shares < 4: the committee cannot sign.
    assert!(
        w.preconf(&e1, &frost, &[&a, &b, &c], W::header(0xa0, 104))
            .is_err()
    );
    // A header that does not link to its preconfirmed parent.
    let mut bad = W::header(0xa0, 104);
    bad.prev_hash = h32(0x11);
    assert!(err_str(w.preconf(&e1, &frost, &[&a, &b, &e], bad)).contains("link"));
    let best = w.best();
    assert_eq!(
        (best.source, best.height),
        (agg::HeaderSource::Preconf, Some(103))
    );

    // Registry k-of-n reaches 103 → confirm 100..=101 (depth 2).
    let ops = [0, 1, 2, 4];
    w.report(&ops, 0xa0, 103);
    let relayer = w.app.api().addr_make("relayer");
    w.app
        .execute_contract(
            relayer.clone(),
            w.agg.clone(),
            &agg::ExecuteMsg::Confirm { height: 100 },
            &[],
        )
        .unwrap();
    let r: Option<cw_lc_aggregator::state::PreconfRecord> = w
        .app
        .wrap()
        .query_wasm_smart(&w.agg, &agg::QueryMsg::Preconf { height: 101 })
        .unwrap();
    assert_eq!(
        r.unwrap().status,
        cw_lc_aggregator::state::PreconfStatus::Confirmed
    );
    assert!(
        w.app
            .execute_contract(
                relayer,
                w.agg.clone(),
                &agg::ExecuteMsg::Confirm { height: 102 },
                &[]
            )
            .is_err(),
        "102 is only 1 deep"
    );

    // Equivocation: the committee signs a competing 103.
    w.preconf(&e1, &frost, &[&a, &b, &e], W::header(0xb0, 103))
        .unwrap();
    let ev: Option<cw_lc_aggregator::state::Equivocation> = w
        .app
        .wrap()
        .query_wasm_smart(&w.agg, &agg::QueryMsg::Evidence { epoch: 1 })
        .unwrap();
    assert_eq!(ev.unwrap().height, 103);
    let e1 = w.epoch(rot::QueryMsg::Epoch { epoch: 1 }).unwrap();
    assert!(matches!(e1.status, EpochStatus::Faulted { .. }));
    assert!(w.epoch(rot::QueryMsg::ActiveEpoch {}).is_none());
    // Fallback to registry while epoch 2 is pending.
    assert_eq!(w.best().source, agg::HeaderSource::RegistryFallback);
    let e2 = w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap();
    assert_eq!(e2.epoch, 2);
    // Epoch 1 key can no longer preconfirm.
    assert!(
        err_str(w.preconf(&e1, &frost, &[&a, &b, &e], W::header(0xa0, 104)))
            .contains("not the active")
    );
}

type W = World;

#[test]
fn planned_rotation_handoff_and_liveness_fallback() {
    let mut w = World::new(&[
        ("op-a", 1, true),
        ("op-b", 1, true),
        ("op-c", 1, true),
        ("op-d", 1, true),
    ]);
    w.report(&[0, 1, 2, 3], 0xa0, 50);
    let admin = w.admin.clone();
    let (a, b, c, d) = (w.addr(0), w.addr(1), w.addr(2), w.addr(3));
    w.rot_exec(&admin, &rot::ExecuteMsg::Bootstrap {}).unwrap();
    let e1 = w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap();
    assert_eq!((e1.total_shares, e1.threshold), (4, 3));
    let frost = Signer_::Frost(FrostLabCommittee::dkg(3, 4).unwrap());
    let everyone = [a.clone(), b.clone(), c.clone(), d.clone()];
    w.activate(&e1, &frost, &everyone, &[&a, &b, &c], None)
        .unwrap();
    let e1 = w.epoch(rot::QueryMsg::ActiveEpoch {}).unwrap();
    w.preconf(&e1, &frost, &[&a, &b, &d], W::header(0xa0, 51))
        .unwrap();

    // Non-member cannot rotate; a member can (planned reshare).
    let outsider = w.app.api().addr_make("outsider");
    assert!(
        w.rot_exec(
            &outsider,
            &rot::ExecuteMsg::RotateShard { note: "x".into() }
        )
        .is_err()
    );
    // Weights change before the rotation: b now carries 2 shares.
    w.app
        .execute_contract(
            admin.clone(),
            w.reg.clone(),
            &reg::ExecuteMsg::SetOperatorWeight {
                operator: b.to_string(),
                weight: 2,
            },
            &[],
        )
        .unwrap();
    w.rot_exec(
        &c,
        &rot::ExecuteMsg::RotateShard {
            note: "reshare".into(),
        },
    )
    .unwrap();
    let e2 = w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap();
    assert_eq!((e2.total_shares, e2.threshold), (5, 4));
    // Old epoch keeps serving during the DKG.
    assert_eq!(w.best().source, agg::HeaderSource::Preconf);

    // New committee uses the TC01 threshold scheme (threshold_committee).
    let tc = Signer_::Tc(LabCommittee::generate(4, 5).unwrap());
    // Hand-off from the active key is mandatory.
    assert!(err_str(w.activate(&e2, &tc, &everyone, &[&a, &b, &c], None)).contains("hand-off"));
    w.activate(&e2, &tc, &[], &[&a, &b, &c], Some((&frost, &e1)))
        .unwrap();
    assert_eq!(
        w.epoch(rot::QueryMsg::Epoch { epoch: 1 }).unwrap().status,
        EpochStatus::Retired
    );
    let e2 = w.epoch(rot::QueryMsg::ActiveEpoch {}).unwrap();
    // TC01 preconf; reorg of the unconfirmed epoch-1 header at 51 is allowed.
    w.preconf(&e2, &tc, &[&b, &c, &d], W::header(0xc0, 51))
        .unwrap();
    w.preconf(&e2, &tc, &[&a, &b, &c], W::header(0xc0, 52))
        .unwrap();
    let best = w.best();
    assert_eq!(
        (best.source, best.epoch, best.height),
        (agg::HeaderSource::Preconf, Some(2), Some(52))
    );

    // Committee goes quiet: stall → fallback, then liveness fault rotates.
    let anyone = w.app.api().addr_make("anyone");
    assert!(
        err_str(w.rot_exec(&anyone, &rot::ExecuteMsg::ReportLivenessFault {})).contains("no stall")
    );
    w.advance(4);
    w.report(&[0, 1, 2, 3], 0xc0, 52);
    let best = w.best();
    assert_eq!(
        (best.source, best.height),
        (agg::HeaderSource::RegistryFallback, Some(52))
    );
    w.rot_exec(&anyone, &rot::ExecuteMsg::ReportLivenessFault {})
        .unwrap();
    assert!(matches!(
        w.epoch(rot::QueryMsg::Epoch { epoch: 2 }).unwrap().status,
        EpochStatus::Faulted { .. }
    ));
    let e3 = w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap();
    assert!(matches!(
        e3.reason,
        cw_lc_rotation::state::ProposalReason::LivenessFault
    ));

    // Pending epochs expire; Bootstrap re-proposes from the current set.
    w.advance(51);
    w.report(&[0, 1, 2, 3], 0xc0, 53);
    let frost3 = Signer_::Frost(FrostLabCommittee::dkg(e3.threshold, e3.total_shares).unwrap());
    assert!(
        err_str(w.activate(
            &e3,
            &frost3,
            &everyone, // Expired pending epochs no longer accept acks.
            &[&a, &b, &c, &d],
            None
        ))
        .contains("not pending")
    );
    w.rot_exec(&anyone, &rot::ExecuteMsg::Bootstrap {}).unwrap();
    assert_eq!(
        w.epoch(rot::QueryMsg::Epoch { epoch: 3 }).unwrap().status,
        EpochStatus::Expired
    );
    assert_eq!(w.epoch(rot::QueryMsg::PendingEpoch {}).unwrap().epoch, 4);
}
