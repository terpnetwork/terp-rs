//! Gas baselines on the **optimized wasm** (cosmwasm-vm metering; cw-multi-test
//! has none). The contract under test runs in the VM; contracts it queries run
//! natively in a cw-multi-test `App` that answers the VM's wasm queries.
//!
//! Run: `BENCH_WASM_DIR=<artifacts> BENCH_OUT=<file.md> cargo test -p cw-lc-zcash
//! --test vm_gas -- --ignored --nocapture`
//!
//! Columns: `vm_gas` = CosmWasm gas (wasm execution + host imports incl.
//! ed25519 / blake3 / FROST / TC01 verification). `sdk_vm` = vm_gas / 140_000
//! (wasmd default multiplier). `kv_w` = keys written, `sdk_kv_w` = Σ(2000 + 30 ×
//! (|k|+|v|)) flat write cost. Reads (1000 + 3/byte each) are not counted, so
//! `sdk_est` is a lower bound for the full tx (also excludes ante/sig/tx bytes).
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{
    Addr, Binary, ContractResult, Empty, Env, Order, QueryRequest, Querier, Response, Uint128,
    from_json, to_json_vec,
};
use cosmwasm_vm::testing::{
    MockApi, MockInstanceOptions, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR, execute,
    instantiate, mock_instance_with_options, query,
};
use cosmwasm_vm::{Instance, Storage};
use cw_multi_test::{App, Contract, ContractWrapper, Executor};
use ed25519_dalek::{Signer, SigningKey};
use frost_escrow::FrostLabCommittee;
use serde::Serialize;
use threshold_committee::{CombinedSignature, LabCommittee};

use cw_lc_aggregator::msg as agg;
use cw_lc_registry::msg as reg;
use cw_lc_rotation::committee::CommitteePolicy;
use cw_lc_rotation::msg as rot;
use cw_lc_rotation::state::Epoch;
use zap1_verify::{EventPayload, compute_leaf_hash, node_hash};

type Inst = Instance<MockApi, MockStorage, MockQuerier>;
const CLIENT: &str = "zcash-regtest-0";
const GAS_LIMIT: u64 = 1_000_000_000_000_000;

fn wasm(name: &str) -> Vec<u8> {
    let dir = std::env::var("BENCH_WASM_DIR").expect("BENCH_WASM_DIR");
    std::fs::read(format!("{dir}/{name}.wasm")).unwrap()
}

fn kv(i: &mut Inst) -> BTreeMap<Vec<u8>, Vec<u8>> {
    i.with_storage(|s| {
        let id = s.scan(None, None, Order::Ascending).0.unwrap();
        Ok(s.all(id).0.unwrap().into_iter().collect())
    })
    .unwrap()
}
fn used(i: &mut Inst) -> u64 {
    let r = i.create_gas_report();
    r.used_internally + r.used_externally
}

struct Row {
    contract: &'static str,
    call: String,
    vm: u64,
    writes: usize,
    wbytes: usize,
}

#[derive(Default)]
struct Meter(Vec<Row>);

impl Meter {
    fn measure<T>(
        &mut self,
        c: &'static str,
        call: &str,
        i: &mut Inst,
        f: impl FnOnce(&mut Inst) -> T,
    ) -> T {
        let (k0, g0) = (kv(i), used(i));
        let out = f(i);
        let (k1, g1) = (kv(i), used(i));
        let changed: Vec<_> = k1.iter().filter(|(k, v)| k0.get(*k) != Some(*v)).collect();
        self.0.push(Row {
            contract: c,
            call: call.into(),
            vm: g1 - g0,
            writes: changed.len(),
            wbytes: changed.iter().map(|(k, v)| k.len() + v.len()).sum(),
        });
        out
    }
    fn exec<M: Serialize>(
        &mut self,
        c: &'static str,
        call: &str,
        i: &mut Inst,
        env: Env,
        sender: &Addr,
        msg: &M,
    ) -> Response {
        let r: ContractResult<Response> =
            self.measure(c, call, i, |i| execute(i, env, message_info(sender, &[]), msg));
        r.into_result().unwrap_or_else(|e| panic!("{c} {call}: {e}"))
    }
    fn inst<M: Serialize>(&mut self, c: &'static str, i: &mut Inst, env: Env, sender: &Addr, msg: &M) {
        let r: ContractResult<Response> =
            self.measure(c, "instantiate", i, |i| instantiate(i, env, message_info(sender, &[]), msg));
        r.into_result().unwrap_or_else(|e| panic!("{c} instantiate: {e}"));
    }
    fn query<M: Serialize, R: serde::de::DeserializeOwned>(
        &mut self,
        c: &'static str,
        call: &str,
        i: &mut Inst,
        env: Env,
        msg: &M,
    ) -> R {
        let r = self.measure(c, call, i, |i| query(i, env, msg));
        from_json(r.into_result().unwrap_or_else(|e| panic!("{c} {call}: {e}"))).unwrap()
    }
    fn table(&self) -> String {
        let mut s = String::from(
            "| contract | call | vm_gas | sdk_vm | kv_w | kv_bytes | sdk_kv_w | sdk_est (lower bound) |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
        );
        for r in &self.0 {
            let sdk_vm = r.vm.div_ceil(140_000);
            let sdk_kv = r.writes as u64 * 2000 + r.wbytes as u64 * 30;
            s += &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.contract, r.call, r.vm, sdk_vm, r.writes, r.wbytes, sdk_kv, sdk_vm + sdk_kv
            );
        }
        s
    }
}

fn vm(name: &str, app: Option<Rc<RefCell<App>>>) -> Inst {
    let mut i = mock_instance_with_options(
        &wasm(name),
        MockInstanceOptions {
            gas_limit: GAS_LIMIT,
            // Chain capabilities (x/wasm): the image's wasm-opt keeps bulk-memory.
            available_capabilities: cosmwasm_vm::capabilities_from_csv(
                "iterator,staking,stargate,bulk_memory,ibc2,hash_blake,hash_poseidon,bn254,redpallas,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0",
            ),
            ..Default::default()
        },
    );
    if let Some(app) = app {
        i.with_querier(|q| {
            q.update_wasm(move |w| {
                app.borrow()
                    .raw_query(&to_json_vec(&QueryRequest::<Empty>::Wasm(w.clone())).unwrap())
            });
            Ok(())
        })
        .unwrap();
    }
    i
}

fn env_of(app: &Rc<RefCell<App>>) -> Env {
    let mut e = mock_env();
    e.block = app.borrow().block_info();
    e
}

fn hh(f: u8, h: u64) -> String {
    let mut b = [f; 32];
    b[..8].copy_from_slice(&h.to_be_bytes());
    hex::encode(b)
}
fn header(h: u64, payment_root: Option<[u8; 32]>) -> agg::Header {
    agg::Header {
        height: h,
        hash: hh(0xa0, h),
        prev_hash: hh(0xa0, h - 1),
        chain_work: "0fff".into(),
        payment_root: payment_root.map(hex::encode),
    }
}

enum Sig {
    Frost(FrostLabCommittee),
    Tc(LabCommittee),
}
impl Sig {
    fn new(frost: bool, t: u16, n: u16) -> Self {
        if frost {
            Sig::Frost(FrostLabCommittee::dkg(t, n).unwrap())
        } else {
            Sig::Tc(LabCommittee::generate(t, n).unwrap())
        }
    }
    fn key(&self) -> Binary {
        match self {
            Sig::Frost(c) => Binary::from(c.encoded_public.clone()),
            Sig::Tc(c) => Binary::from(c.encoded_public.clone()),
        }
    }
    /// Sign with the first `k` share ids.
    fn sign(&self, k: u16, d: &[u8]) -> Binary {
        let ids: Vec<u16> = (1..=k).collect();
        match self {
            Sig::Frost(c) => Binary::from(c.sign_message_with(d, &ids).unwrap()),
            Sig::Tc(c) => {
                let d: [u8; 32] = d.try_into().unwrap();
                let partials = ids
                    .iter()
                    .map(|i| c.share(i - 1).unwrap().partial_sign_digest(&d).unwrap())
                    .collect();
                Binary::from(CombinedSignature { partials }.encode())
            }
        }
    }
}

fn wrap<E, I, Q, A, B, C>(
    e: fn(cosmwasm_std::DepsMut, Env, cosmwasm_std::MessageInfo, A) -> Result<Response, E>,
    i: fn(cosmwasm_std::DepsMut, Env, cosmwasm_std::MessageInfo, B) -> Result<Response, I>,
    q: fn(cosmwasm_std::Deps, Env, C) -> Result<Binary, Q>,
) -> Box<dyn Contract<Empty>>
where
    A: serde::de::DeserializeOwned + std::fmt::Debug + 'static,
    B: serde::de::DeserializeOwned + std::fmt::Debug + 'static,
    C: serde::de::DeserializeOwned + std::fmt::Debug + 'static,
    E: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    I: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    Q: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
{
    Box::new(ContractWrapper::new_with_empty(e, i, q))
}

/// Native world (registry [+ rotation + aggregator]) with 4 weight-1 operators.
struct Native {
    app: Rc<RefCell<App>>,
    admin: Addr,
    reg: Addr,
    rot: Option<Addr>,
    agg: Option<Addr>,
    ops: Vec<(Addr, SigningKey)>,
}

impl Native {
    fn registry_only() -> Self {
        let mut app = App::default();
        let admin = app.api().addr_make("admin");
        let rc = app.store_code(wrap(
            cw_lc_registry::contract::execute,
            cw_lc_registry::contract::instantiate,
            cw_lc_registry::contract::query,
        ));
        let reg = app
            .instantiate_contract(rc, admin.clone(), &registry_init(), &[], "reg", None)
            .unwrap();
        let mut ops = vec![];
        for i in 0..4u8 {
            let a = app.api().addr_make(&format!("op-{i}"));
            let k = SigningKey::from_bytes(&[i + 1; 32]);
            app.execute_contract(a.clone(), reg.clone(), &register_op(i, &k), &[])
                .unwrap();
            ops.push((a, k));
        }
        app.execute_contract(admin.clone(), reg.clone(), &register_client(&ops), &[])
            .unwrap();
        let mut n = Native {
            app: Rc::new(RefCell::new(app)),
            admin,
            reg,
            rot: None,
            agg: None,
            ops,
        };
        n.report(50);
        n
    }
    fn report(&mut self, h: u64) {
        let mut app = self.app.borrow_mut();
        for (a, k) in &self.ops {
            let state = node_state(h);
            let d: Binary = app
                .wrap()
                .query_wasm_smart(&self.reg, &report_digest(a, &state))
                .unwrap();
            let msg = reg::ExecuteMsg::ReportNodeState {
                client_id: CLIENT.into(),
                state,
                signature: Binary::from(k.sign(d.as_slice()).to_bytes().to_vec()),
            };
            app.execute_contract(a.clone(), self.reg.clone(), &msg, &[])
                .unwrap();
        }
    }
    /// + native rotation with an active epoch; aggregator = `agg_addr`.
    fn with_rotation(mut self, signer: &Sig, agg_addr: Option<&str>) -> (Self, Epoch) {
        let rot_addr;
        {
            let mut app = self.app.borrow_mut();
            let oc = app.store_code(wrap(
                cw_lc_rotation::contract::execute,
                cw_lc_rotation::contract::instantiate,
                cw_lc_rotation::contract::query,
            ));
            rot_addr = app
                .instantiate_contract(oc, self.admin.clone(), &rotation_init(&self.reg), &[], "rot", None)
                .unwrap();
            if let Some(agg_addr) = agg_addr {
                app.execute_contract(
                    self.admin.clone(),
                    rot_addr.clone(),
                    &rot::ExecuteMsg::SetAggregator {
                        aggregator: agg_addr.into(),
                    },
                    &[],
                )
                .unwrap();
            }
            app.execute_contract(self.admin.clone(), rot_addr.clone(), &rot::ExecuteMsg::Bootstrap {}, &[])
                .unwrap();
            let e: Epoch = app
                .wrap()
                .query_wasm_smart(&rot_addr, &rot::QueryMsg::PendingEpoch {})
                .unwrap();
            for (a, _) in &self.ops {
                app.execute_contract(
                    a.clone(),
                    rot_addr.clone(),
                    &rot::ExecuteMsg::AckEpoch {
                        epoch: e.epoch,
                        group_key: signer.key(),
                    },
                    &[],
                )
                .unwrap();
            }
            let d: Binary = app
                .wrap()
                .query_wasm_smart(
                    &rot_addr,
                    &rot::QueryMsg::ActivationDigest {
                        epoch: e.epoch,
                        group_key: signer.key(),
                    },
                )
                .unwrap();
            app.execute_contract(
                self.admin.clone(),
                rot_addr.clone(),
                &rot::ExecuteMsg::ActivateEpoch {
                    epoch: e.epoch,
                    group_key: signer.key(),
                    proof_sig: signer.sign(e.threshold, d.as_slice()),
                    handoff_sig: None,
                },
                &[],
            )
            .unwrap();
        }
        let e: Epoch = self
            .app
            .borrow()
            .wrap()
            .query_wasm_smart(&rot_addr, &rot::QueryMsg::ActiveEpoch {})
            .unwrap();
        self.rot = Some(rot_addr);
        (self, e)
    }
    /// + native aggregator (rotation must point at it).
    fn with_aggregator(mut self) -> Self {
        let mut app = self.app.borrow_mut();
        let ac = app.store_code(wrap(
            cw_lc_aggregator::contract::execute,
            cw_lc_aggregator::contract::instantiate,
            cw_lc_aggregator::contract::query,
        ));
        let a = app
            .instantiate_contract(
                ac,
                self.admin.clone(),
                &aggregator_init(&self.reg, self.rot.as_ref().unwrap()),
                &[],
                "agg",
                None,
            )
            .unwrap();
        app.execute_contract(
            self.admin.clone(),
            self.rot.clone().unwrap(),
            &rot::ExecuteMsg::SetAggregator { aggregator: a.to_string() },
            &[],
        )
        .unwrap();
        drop(app);
        self.agg = Some(a);
        self
    }
}

fn registry_init() -> reg::InstantiateMsg {
    reg::InstantiateMsg {
        admin: None,
        label: "terp/lc/registry/zcash-regtest/v1".into(),
        hash_market_capability: None,
        max_report_age_blocks: 100,
    }
}
fn register_op(i: u8, k: &SigningKey) -> reg::ExecuteMsg {
    reg::ExecuteMsg::RegisterOperator {
        peer_id: format!("12D3KooWop{i}"),
        endpoint: format!("/ip4/127.0.0.1/tcp/{}", 18230 + i as u16),
        pubkey: Binary::from(k.verifying_key().to_bytes().to_vec()),
        capabilities: vec!["node-state".into(), "preconf".into()],
    }
}
fn register_client(ops: &[(Addr, SigningKey)]) -> reg::ExecuteMsg {
    reg::ExecuteMsg::RegisterClient {
        client_id: CLIENT.into(),
        kind: reg::ClientKind::Ics08Wasm,
        target: reg::TargetChain {
            chain_id: "zcash-regtest".into(),
            algo: reg::PowAlgo::Equihash { n: 200, k: 9 },
            target_block_time_secs: 25,
        },
        attestors: ops.iter().map(|o| o.0.to_string()).collect(),
        min_attestations: 3,
        ibc_client_id: None,
    }
}
fn node_state(h: u64) -> reg::NodeState {
    reg::NodeState {
        tip_height: h,
        tip_hash: hh(0xa0, h),
        prev_hash: hh(0xa0, h - 1),
        chain_work: "0fff".into(),
        finalized_height: None,
        peer_count: 4,
        observed_at_unix: h,
    }
}
fn report_digest(a: &Addr, state: &reg::NodeState) -> reg::QueryMsg {
    reg::QueryMsg::ReportDigest {
        client_id: CLIENT.into(),
        operator: a.to_string(),
        state: state.clone(),
    }
}
fn rotation_init(reg: &Addr) -> rot::InstantiateMsg {
    rot::InstantiateMsg {
        admin: None,
        label: "terp/lc/rotation/zcash-regtest/v1".into(),
        registry: reg.to_string(),
        client_id: CLIENT.into(),
        policy: Some(CommitteePolicy::default()),
    }
}
fn aggregator_init(reg: &Addr, rot: &Addr) -> agg::InstantiateMsg {
    agg::InstantiateMsg {
        label: "terp/lc/aggregator/zcash-regtest/v1".into(),
        registry: reg.to_string(),
        rotation: rot.to_string(),
        client_id: CLIENT.into(),
        confirm_depth: 2,
        stall_blocks: 30,
        max_confirm_walk: None,
    }
}

fn bench_registry(m: &mut Meter) {
    let app = Rc::new(RefCell::new(App::default()));
    let env = env_of(&app);
    let api = MockApi::default();
    let admin = Addr::unchecked(api.addr_make("admin"));
    let mut i = vm("cw_lc_registry", None);
    m.inst("cw-lc-registry", &mut i, env.clone(), &admin, &registry_init());
    let mut ops = vec![];
    for n in 0..4u8 {
        let a = Addr::unchecked(api.addr_make(&format!("op-{n}")));
        let k = SigningKey::from_bytes(&[n + 1; 32]);
        m.exec("cw-lc-registry", "RegisterOperator", &mut i, env.clone(), &a, &register_op(n, &k));
        ops.push((a, k));
    }
    m.exec(
        "cw-lc-registry",
        "SetOperatorWeight",
        &mut i,
        env.clone(),
        &admin,
        &reg::ExecuteMsg::SetOperatorWeight {
            operator: ops[0].0.to_string(),
            weight: 2,
        },
    );
    m.exec("cw-lc-registry", "RegisterClient (4 attestors)", &mut i, env.clone(), &admin, &register_client(&ops));
    for (n, (a, k)) in ops.iter().enumerate() {
        let state = node_state(50);
        let d: Binary = query(&mut i, env.clone(), report_digest(a, &state))
            .into_result()
            .map(|b| from_json(b).unwrap())
            .unwrap();
        m.exec(
            "cw-lc-registry",
            &format!("ReportNodeState #{} (ed25519)", n + 1),
            &mut i,
            env.clone(),
            a,
            &reg::ExecuteMsg::ReportNodeState {
                client_id: CLIENT.into(),
                state,
                signature: Binary::from(k.sign(d.as_slice()).to_bytes().to_vec()),
            },
        );
    }
    let _: reg::ConsensusTipResponse = m.query(
        "cw-lc-registry",
        "query ConsensusTip (4 reports)",
        &mut i,
        env.clone(),
        &reg::QueryMsg::ConsensusTip { client_id: CLIENT.into() },
    );
    let _: reg::NodeStatesResponse = m.query(
        "cw-lc-registry",
        "query NodeStates",
        &mut i,
        env,
        &reg::QueryMsg::NodeStates { client_id: CLIENT.into() },
    );
}

fn bench_rotation(m: &mut Meter, frost: bool) {
    let n = Native::registry_only();
    let env = env_of(&n.app);
    let scheme = if frost { "FE01 FROST" } else { "TC01" };
    let mut i = vm("cw_lc_rotation", Some(n.app.clone()));
    if frost {
        m.inst("cw-lc-rotation", &mut i, env.clone(), &n.admin, &rotation_init(&n.reg));
    } else {
        let r: ContractResult<Response> =
            instantiate(&mut i, env.clone(), message_info(&n.admin, &[]), rotation_init(&n.reg));
        r.into_result().unwrap();
    }
    let set = rot::ExecuteMsg::SetAggregator {
        aggregator: n.ops[0].0.to_string(),
    };
    let r: ContractResult<Response> = execute(&mut i, env.clone(), message_info(&n.admin, &[]), &set);
    r.into_result().unwrap();
    m.exec("cw-lc-rotation", "Bootstrap (derive 4-op committee)", &mut i, env.clone(), &n.admin, &rot::ExecuteMsg::Bootstrap {});
    let e: Option<Epoch> = query(&mut i, env.clone(), rot::QueryMsg::PendingEpoch {})
        .into_result()
        .map(|b| from_json(b).unwrap())
        .unwrap();
    let e = e.unwrap();
    let signer = Sig::new(frost, e.threshold, e.total_shares);
    for (k, (a, _)) in n.ops.iter().enumerate() {
        m.exec(
            "cw-lc-rotation",
            &format!("AckEpoch #{}", k + 1),
            &mut i,
            env.clone(),
            a,
            &rot::ExecuteMsg::AckEpoch {
                epoch: e.epoch,
                group_key: signer.key(),
            },
        );
    }
    let d: Binary = query(
        &mut i,
        env.clone(),
        rot::QueryMsg::ActivationDigest {
            epoch: e.epoch,
            group_key: signer.key(),
        },
    )
    .into_result()
    .map(|b| from_json(b).unwrap())
    .unwrap();
    m.exec(
        "cw-lc-rotation",
        &format!("ActivateEpoch ({scheme} {}-of-{})", e.threshold, e.total_shares),
        &mut i,
        env,
        &n.admin,
        &rot::ExecuteMsg::ActivateEpoch {
            epoch: e.epoch,
            group_key: signer.key(),
            proof_sig: signer.sign(e.threshold, d.as_slice()),
            handoff_sig: None,
        },
    );
}

fn bench_aggregator(m: &mut Meter, frost: bool) {
    let probe = Native::registry_only();
    let signer = Sig::new(frost, 3, 4);
    let scheme = if frost { "FE01 FROST 3-of-4" } else { "TC01 3-of-4" };
    let (mut n, e) = probe.with_rotation(&signer, Some(MOCK_CONTRACT_ADDR));
    let env = env_of(&n.app);
    let mut i = vm("cw_lc_aggregator", Some(n.app.clone()));
    let init = aggregator_init(&n.reg, n.rot.as_ref().unwrap());
    let relayer = n.ops[0].0.clone();
    if frost {
        m.inst("cw-lc-aggregator", &mut i, env.clone(), &n.admin, &init);
    } else {
        let r: ContractResult<Response> = instantiate(&mut i, env.clone(), message_info(&n.admin, &[]), init);
        r.into_result().unwrap();
    }
    for h in 51..=53u64 {
        let root = (h == 51).then_some([0x5a; 32]);
        let hd = header(h, root);
        let d: Binary = query(&mut i, env.clone(), agg::QueryMsg::PreconfDigest { epoch: e.epoch, header: hd.clone() })
            .into_result()
            .map(|b| from_json(b).unwrap())
            .unwrap();
        let label = if root.is_some() {
            format!("SubmitPreconf ({scheme}, +payment_root)")
        } else {
            format!("SubmitPreconf ({scheme})")
        };
        let msg = agg::ExecuteMsg::SubmitPreconf {
            epoch: e.epoch,
            header: hd,
            signature: signer.sign(e.threshold, d.as_slice()),
        };
        if h == 51 || h == 52 {
            m.exec("cw-lc-aggregator", &label, &mut i, env.clone(), &relayer, &msg);
        } else {
            let r: ContractResult<Response> = execute(&mut i, env.clone(), message_info(&relayer, &[]), &msg);
            r.into_result().unwrap();
        }
    }
    if frost {
        n.report(53);
        let env = env_of(&n.app);
        m.exec("cw-lc-aggregator", "Confirm (walks 51..=53 vs registry tip)", &mut i, env.clone(), &relayer, &agg::ExecuteMsg::Confirm { height: 51 });
        let _: agg::BestHeaderResponse = m.query("cw-lc-aggregator", "query BestHeader", &mut i, env, &agg::QueryMsg::BestHeader {});
    }
}

fn bench_zcash(m: &mut Meter) {
    // Native registry + rotation + aggregator; payment root at 51, confirmed.
    let signer = Sig::new(true, 3, 4);
    let probe = Native::registry_only();
    let (n, e) = probe.with_rotation(&signer, None);
    let mut n = n.with_aggregator();
    let agg_addr = n.agg.clone().unwrap();
    let leaf = compute_leaf_hash(&EventPayload::HostingPayment {
        serial_number: b"auction-lot-42",
        month: 8,
        year: 2026,
    });
    let sib = [0x33u8; 32];
    let root = node_hash(&leaf, &sib);
    for h in 51..=53u64 {
        let hd = header(h, (h == 51).then_some(root));
        let mut app = n.app.borrow_mut();
        let d: Binary = app
            .wrap()
            .query_wasm_smart(&agg_addr, &agg::QueryMsg::PreconfDigest { epoch: e.epoch, header: hd.clone() })
            .unwrap();
        let relayer = n.ops[0].0.clone();
        app.execute_contract(
            relayer,
            agg_addr.clone(),
            &agg::ExecuteMsg::SubmitPreconf { epoch: e.epoch, header: hd, signature: signer.sign(3, d.as_slice()) },
            &[],
        )
        .unwrap();
    }
    n.report(53);
    n.app
        .borrow_mut()
        .execute_contract(n.admin.clone(), agg_addr.clone(), &agg::ExecuteMsg::Confirm { height: 51 }, &[])
        .unwrap();
    let env = env_of(&n.app);
    let mut i = vm("cw_lc_zcash", Some(n.app.clone()));
    m.inst(
        "cw-lc-zcash",
        &mut i,
        env.clone(),
        &n.admin,
        &cw_lc_zcash::msg::InstantiateMsg {
            label: "terp/lc/client/zcash-regtest/v1".into(),
            registry: n.reg.to_string(),
            aggregator: agg_addr.to_string(),
            registry_client_id: CLIENT.into(),
            ibc_client_id: "08-wasm-zcash-0".into(),
            accept_preconfirmed: false,
        },
    );
    let (note, frost) = ([0x11u8; 32], [0x22u8; 32]);
    let adh = cw_ics08_wasm_zap1::app_data_hash(&note, 50_000, &frost, &leaf);
    m.exec(
        "cw-lc-zcash",
        "VerifyPaymentInclusion (zap-1, 1-step path)",
        &mut i,
        env.clone(),
        &n.admin,
        &cw_lc_zcash::msg::ExecuteMsg::VerifyPaymentInclusion(Box::new(cw_lc_zcash::msg::PaymentInclusionMsg {
            height: 51,
            source_client: "zcash-0".into(),
            dest_client: "08-wasm-zcash-0".into(),
            sequence: 1,
            timeout_timestamp_ns: 0,
            app_data_hash_hex: hex::encode(adh),
            note_hex: hex::encode(note),
            amount_zat: 50_000,
            frost_group_hex: hex::encode(frost),
            leaf_hex: hex::encode(leaf),
            event_kind: "hosting_payment".into(),
            wallet_or_serial: "auction-lot-42".into(),
            proof: vec![cw_lc_zcash::msg::Zap1Step { sibling_hex: hex::encode(sib), sibling_is_left: false }],
        })),
    );
    let r: Option<cw_lc_zcash::msg::PaymentInclusionResponse> = m.query(
        "cw-lc-zcash",
        "query PaymentInclusion",
        &mut i,
        env.clone(),
        &cw_lc_zcash::msg::QueryMsg::PaymentInclusion { app_data_hash_hex: hex::encode(adh) },
    );
    assert_eq!(r.unwrap().status, cw_lc_zcash::msg::InclusionStatus::Confirmed);
    let _: agg::BestHeaderResponse =
        m.query("cw-lc-zcash", "query LatestHeader (proxy)", &mut i, env, &cw_lc_zcash::msg::QueryMsg::LatestHeader {});
}

fn bench_private_dex(m: &mut Meter) {
    use cw_private_dex::msg::*;
    let api = MockApi::default();
    let owner = Addr::unchecked(api.addr_make("owner"));
    let trader = Addr::unchecked(api.addr_make("trader"));
    let env = mock_env();
    let mut i = vm("cw_private_dex", None);
    let mut a = [0u8; 32];
    a[..3].copy_from_slice(b"HUB");
    let hub = Binary::from(a.to_vec());
    let mut b = [0u8; 32];
    b[0] = b'B';
    let bb = Binary::from(b.to_vec());
    m.inst(
        "cw-private-dex",
        &mut i,
        env.clone(),
        &owner,
        &InstantiateMsg { mock_verify: true, zkid: Some(42), allowed_root: Some(Binary::from(vec![7u8; 32])) },
    );
    m.exec(
        "cw-private-dex",
        "CreatePool",
        &mut i,
        env.clone(),
        &owner,
        &ExecuteMsg::CreatePool {
            asset_a: hub.clone(),
            asset_b: bb.clone(),
            r_a: Uint128::new(1_000_000),
            r_b: Uint128::new(2_000_000),
            gamma: 997,
            gamma_den: 1000,
        },
    );
    let delta_out = cw_private_dex::seams::quote_exact_in(1_000_000, 2_000_000, 1000, 997, 1000).unwrap();
    let st = SwapStatementPublic {
        pool_id: 1,
        asset_in: hub.clone(),
        asset_out: bb,
        root: Binary::from(vec![7u8; 32]),
        nullifiers: vec![Binary::from(vec![0xAB; 32])],
        cm_out: vec![Binary::from(vec![9u8; 32])],
        delta_r_in: Uint128::new(1000),
        delta_r_out: Uint128::new(delta_out),
        min_out: Uint128::new(1),
        gamma: 997,
        gamma_den: 1000,
        r_in_before: Uint128::new(1_000_000),
        r_out_before: Uint128::new(2_000_000),
        oracle_mid: None,
        oracle_params: None,
    };
    m.exec(
        "cw-private-dex",
        "SettleSwap (mock_verify: host recompute + nullifier, no proof)",
        &mut i,
        env.clone(),
        &trader,
        &ExecuteMsg::SettleSwap { statement: Box::new(st), proof: Binary::from(b"mock-proof".as_slice()) },
    );
    let r = m.measure("cw-private-dex", "query QuoteExactIn", &mut i, |i| {
        query(i, env, QueryMsg::QuoteExactIn { pool_id: 1, asset_in: hub, delta_in: Uint128::new(1000) })
    });
    r.into_result().unwrap();
}

#[test]
#[ignore = "needs optimized wasm: BENCH_WASM_DIR"]
fn vm_gas_baselines() {
    let mut m = Meter::default();
    bench_registry(&mut m);
    bench_rotation(&mut m, true);
    bench_rotation(&mut m, false);
    bench_aggregator(&mut m, true);
    bench_aggregator(&mut m, false);
    bench_zcash(&mut m);
    bench_private_dex(&mut m);
    let t = m.table();
    println!("{t}");
    if let Ok(p) = std::env::var("BENCH_OUT") {
        std::fs::write(p, &t).unwrap();
    }
}
