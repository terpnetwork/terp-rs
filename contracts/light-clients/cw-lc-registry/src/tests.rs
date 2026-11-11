use std::marker::PhantomData;

use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, message_info, mock_env};
use cosmwasm_std::{
    Addr, Binary, ContractResult, Empty, Env, OwnedDeps, Querier, QuerierResult, QueryRequest,
    SystemError, SystemResult, from_json,
};
use ed25519_dalek::{Signer, SigningKey};

use crate::contract::{execute, instantiate, query};
use crate::digest::report_digest;
use crate::error::ContractError;
use crate::label::{validate_client_id, validate_label};
use crate::msg::*;
use crate::quorum::{
    HASH_ROOT_PATH, HashMarketCapability, QuorumRoot, QuorumVerdict, decode_response,
    encode_request, encode_response,
};
use crate::state::{ClientRecord, Config, OperatorInfo, StoredReport};

/// MockQuerier + a stand-in for hm-vext `Query/HashRoot` over gRPC.
struct HmQuerier {
    base: MockQuerier,
    /// Encoded `QueryHashRootResponse`; `None` = module/query unavailable.
    root: Option<Vec<u8>>,
}

impl Querier for HmQuerier {
    fn raw_query(&self, bin: &[u8]) -> QuerierResult {
        if let Ok(QueryRequest::<Empty>::Grpc(g)) = from_json(bin)
            && g.path == HASH_ROOT_PATH
        {
            assert_eq!(
                g.data.as_slice(),
                encode_request("zcash-regtest", "block-hash")
            );
            return match &self.root {
                Some(r) => SystemResult::Ok(ContractResult::Ok(Binary::from(r.clone()))),
                None => SystemResult::Err(SystemError::UnsupportedRequest {
                    kind: "grpc".into(),
                }),
            };
        }
        self.base.raw_query(bin)
    }
}

type Deps = OwnedDeps<MockStorage, MockApi, HmQuerier>;

fn mock_dependencies() -> Deps {
    OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: HmQuerier {
            base: MockQuerier::new(&[]),
            root: None,
        },
        custom_query_type: PhantomData,
    }
}

fn cap(require: bool) -> HashMarketCapability {
    HashMarketCapability {
        chain_uid: "zcash-regtest".into(),
        algo: "block-hash".into(),
        require,
    }
}

fn set_root(deps: &mut Deps, height: u64, hash: u8) {
    deps.querier.root = Some(encode_response(
        "zcash-regtest",
        "block-hash",
        &QuorumRoot {
            height,
            root: h32(hash),
            attestation_count: 4,
            block_time: 1_700_000_000,
        },
    ));
}

const LABEL: &str = "terp/lc/registry/zcash-regtest/v1";
const CLIENT: &str = "zcash-regtest-0";

fn h32(b: u8) -> String {
    hex::encode([b; 32])
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn setup() -> (Deps, Env, Addr) {
    let mut deps = mock_dependencies();
    let env = mock_env();
    let admin = deps.api.addr_make("admin");
    instantiate(
        deps.as_mut(),
        env.clone(),
        message_info(&admin, &[]),
        InstantiateMsg {
            admin: None,
            label: LABEL.into(),
            hash_market_capability: Some(cap(false)),
            max_report_age_blocks: 50,
        },
    )
    .unwrap();
    (deps, env, admin)
}

fn register_op(deps: &mut Deps, env: &Env, name: &str, seed: u8) -> Addr {
    let op = deps.api.addr_make(name);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&op, &[]),
        ExecuteMsg::RegisterOperator {
            peer_id: format!("12D3Koo{name}"),
            endpoint: format!("/ip4/127.0.0.1/tcp/1823{seed}"),
            pubkey: Binary::from(key(seed).verifying_key().to_bytes().to_vec()),
            capabilities: vec!["pow-headers".into(), "node-state".into()],
        },
    )
    .unwrap();
    op
}

fn register_client(
    deps: &mut Deps,
    env: &Env,
    owner: &Addr,
    ops: &[&Addr],
    min: u32,
) -> Result<(), ContractError> {
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(owner, &[]),
        ExecuteMsg::RegisterClient {
            client_id: CLIENT.into(),
            kind: ClientKind::Permissionless,
            target: TargetChain {
                chain_id: "zcash-regtest".into(),
                algo: PowAlgo::Equihash { n: 48, k: 5 },
                target_block_time_secs: 25,
            },
            attestors: ops.iter().map(|a| a.to_string()).collect(),
            min_attestations: min,
            ibc_client_id: None,
        },
    )
    .map(|_| ())
}

fn state(height: u64, hash: u8, fin: Option<u64>, t: u64) -> NodeState {
    NodeState {
        tip_height: height,
        tip_hash: h32(hash),
        prev_hash: h32(hash.wrapping_sub(1)),
        chain_work: "00ff".into(),
        finalized_height: fin,
        peer_count: 3,
        observed_at_unix: t,
    }
}

fn report(
    deps: &mut Deps,
    env: &Env,
    op: &Addr,
    seed: u8,
    s: NodeState,
) -> Result<(), ContractError> {
    let d = report_digest(
        &deps.api,
        &env.block.chain_id,
        &env.contract.address,
        CLIENT,
        op,
        &s,
    );
    let sig = key(seed).sign(&d).to_bytes().to_vec();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(op, &[]),
        ExecuteMsg::ReportNodeState {
            client_id: CLIENT.into(),
            state: s,
            signature: Binary::from(sig),
        },
    )
    .map(|_| ())
}

fn tip(deps: &Deps, env: &Env) -> ConsensusTipResponse {
    from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::ConsensusTip {
                client_id: CLIENT.into(),
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn labels() {
    assert_eq!(validate_label(LABEL, "registry").unwrap(), "zcash-regtest");
    assert!(validate_label("terp/lc/registry/zcash-main/v12", "registry").is_ok());
    assert!(validate_label("terp/lc/rotation/zcash-main/v1", "registry").is_err());
    assert!(validate_label("terp/lc/bogus/zcash-main/v1", "bogus").is_err());
    assert!(validate_label("terp/lc/registry/Zcash/v1", "registry").is_err());
    assert!(validate_label("terp/lc/registry/zcash/v0", "registry").is_err());
    assert!(validate_label("terp/lc/registry/zcash/v01", "registry").is_err());
    assert!(validate_label("lc/registry/zcash/v1", "registry").is_err());
    assert!(validate_client_id("08-wasm-7").is_ok());
    assert!(validate_client_id("Bad_Id").is_err());
}

#[test]
fn instantiate_rejects_bad_label() {
    let mut deps = mock_dependencies();
    let admin = deps.api.addr_make("admin");
    let err = instantiate(
        deps.as_mut(),
        mock_env(),
        message_info(&admin, &[]),
        InstantiateMsg {
            admin: None,
            label: "terp/lc/aggregator/zcash-regtest/v1".into(),
            hash_market_capability: None,
            max_report_age_blocks: 10,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidLabel { .. }));
    let (deps, env, admin) = setup();
    let cfg: Config = from_json(query(deps.as_ref(), env, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.admin, admin);
    assert_eq!(cfg.target, "zcash-regtest");
}

#[test]
fn operator_and_client_registration_rules() {
    let (mut deps, env, admin) = setup();
    let a = register_op(&mut deps, &env, "op-a", 1);
    // duplicate operator
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a, &[]),
        ExecuteMsg::RegisterOperator {
            peer_id: "x".into(),
            endpoint: "y".into(),
            pubkey: Binary::from(vec![0u8; 32]),
            capabilities: vec![],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::OperatorExists {}));
    // bad pubkey
    let z = deps.api.addr_make("op-z");
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&z, &[]),
        ExecuteMsg::RegisterOperator {
            peer_id: "x".into(),
            endpoint: "y".into(),
            pubkey: Binary::from(vec![0u8; 31]),
            capabilities: vec![],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::BadPubkey {}));
    // unregistered attestor
    assert!(matches!(
        register_client(&mut deps, &env, &admin, &[&a, &z], 1),
        Err(ContractError::OperatorNotFound(_))
    ));
    // threshold above n
    assert!(matches!(
        register_client(&mut deps, &env, &admin, &[&a], 2),
        Err(ContractError::BadThreshold { .. })
    ));
    register_client(&mut deps, &env, &admin, &[&a, &a], 1).unwrap();
    let c: ClientRecord = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Client {
                client_id: CLIENT.into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(c.attestors, vec![a.clone()], "dedup attestors");
    assert!(matches!(
        register_client(&mut deps, &env, &admin, &[&a], 1),
        Err(ContractError::ClientExists(_))
    ));
    // bind: owner only
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a, &[]),
        ExecuteMsg::BindIbcClient {
            client_id: CLIENT.into(),
            ibc_client_id: "08-wasm-3".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::Unauthorized {}));
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&admin, &[]),
        ExecuteMsg::BindIbcClient {
            client_id: CLIENT.into(),
            ibc_client_id: "08-wasm-3".into(),
        },
    )
    .unwrap();
    let c: ClientRecord = from_json(
        query(
            deps.as_ref(),
            env,
            QueryMsg::Client {
                client_id: CLIENT.into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(c.ibc_client_id.as_deref(), Some("08-wasm-3"));
}

#[test]
fn report_auth_signature_and_replay() {
    let (mut deps, env, admin) = setup();
    let a = register_op(&mut deps, &env, "op-a", 1);
    let b = register_op(&mut deps, &env, "op-b", 2);
    register_client(&mut deps, &env, &admin, &[&a], 1).unwrap();
    // b is registered but not an attestor
    assert!(matches!(
        report(&mut deps, &env, &b, 2, state(5, 5, None, 100)),
        Err(ContractError::NotAttestor {})
    ));
    // a signs with the wrong key
    assert!(matches!(
        report(&mut deps, &env, &a, 9, state(5, 5, None, 100)),
        Err(ContractError::BadSignature {})
    ));
    // bad hex
    let mut s = state(5, 5, None, 100);
    s.tip_hash = "zz".into();
    assert!(
        matches!(report(&mut deps, &env, &a, 1, s), Err(ContractError::BadHex(f)) if f == "tip_hash")
    );
    report(&mut deps, &env, &a, 1, state(5, 5, None, 100)).unwrap();
    // replay / non-increasing clock
    assert!(matches!(
        report(&mut deps, &env, &a, 1, state(6, 6, None, 100)),
        Err(ContractError::StaleReport {})
    ));
    report(&mut deps, &env, &a, 1, state(6, 6, None, 101)).unwrap();
    let r: Option<StoredReport> = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::NodeState {
                client_id: CLIENT.into(),
                operator: a.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(r.unwrap().state.tip_height, 6);
    // ReportDigest helper equals what the contract verifies
    let s = state(7, 7, Some(3), 102);
    let want = report_digest(
        &deps.api,
        &env.block.chain_id,
        &env.contract.address,
        CLIENT,
        &a,
        &s,
    );
    let got: Binary = from_json(
        query(
            deps.as_ref(),
            env,
            QueryMsg::ReportDigest {
                client_id: CLIENT.into(),
                operator: a.to_string(),
                state: s,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(got.as_slice(), &want);
}

#[test]
fn consensus_tip_k_of_n_freshness_and_deregister() {
    let (mut deps, mut env, admin) = setup();
    let a = register_op(&mut deps, &env, "op-a", 1);
    let b = register_op(&mut deps, &env, "op-b", 2);
    let c = register_op(&mut deps, &env, "op-c", 3);
    register_client(&mut deps, &env, &admin, &[&a, &b, &c], 2).unwrap();
    assert!(tip(&deps, &env).tip.is_none());

    report(&mut deps, &env, &a, 1, state(10, 0xaa, Some(8), 1)).unwrap();
    report(&mut deps, &env, &b, 2, state(10, 0xaa, Some(7), 1)).unwrap();
    report(&mut deps, &env, &c, 3, state(11, 0xbb, Some(9), 1)).unwrap();
    let t = tip(&deps, &env);
    assert_eq!(t.fresh_reports, 3);
    let agreed = t.tip.unwrap();
    assert_eq!(
        (agreed.height, agreed.hash.as_str()),
        (10, h32(0xaa).as_str())
    );
    assert_eq!(agreed.finalized_height, Some(7), "k-th highest finalized");

    // a moves to 11 with c → 11 wins
    report(&mut deps, &env, &a, 1, state(11, 0xbb, Some(9), 2)).unwrap();
    assert_eq!(tip(&deps, &env).tip.unwrap().height, 11);

    // a conflicting hash at 12 from one attestor alone does not win
    report(&mut deps, &env, &b, 2, state(12, 0xcc, None, 2)).unwrap();
    assert_eq!(tip(&deps, &env).tip.unwrap().height, 11);

    // deregistered operators stop counting
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&c, &[]),
        ExecuteMsg::DeregisterOperator {},
    )
    .unwrap();
    assert!(tip(&deps, &env).tip.is_none());

    // stale reports drop out
    env.block.height += 51;
    let t = tip(&deps, &env);
    assert_eq!(t.fresh_reports, 0);
    assert!(t.tip.is_none());
}

#[test]
fn operator_weight_admin_only() {
    let (mut deps, env, admin) = setup();
    let a = register_op(&mut deps, &env, "op-a", 1);
    let op: OperatorInfo = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Operator {
                operator: a.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(op.weight, 1, "default weight");
    let set = |deps: &mut Deps, who: &Addr, w: u64| {
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(who, &[]),
            ExecuteMsg::SetOperatorWeight {
                operator: a.to_string(),
                weight: w,
            },
        )
    };
    assert!(matches!(
        set(&mut deps, &a, 9),
        Err(ContractError::Unauthorized {})
    ));
    set(&mut deps, &admin, 3).unwrap();
    let op: OperatorInfo = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Operator {
                operator: a.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(op.weight, 3);
}

#[test]
fn hash_market_quorum_gate() {
    // protobuf round trip
    let r = QuorumRoot {
        height: 300,
        root: h32(7),
        attestation_count: 5,
        block_time: 42,
    };
    assert_eq!(
        decode_response(&encode_response("zcash-regtest", "block-hash", &r)),
        Some(r)
    );

    let (mut deps, env, admin) = setup();
    let a = register_op(&mut deps, &env, "op-a", 1);
    let b = register_op(&mut deps, &env, "op-b", 2);
    register_client(&mut deps, &env, &admin, &[&a, &b], 2).unwrap();
    report(&mut deps, &env, &a, 1, state(10, 0xaa, None, 1)).unwrap();
    report(&mut deps, &env, &b, 2, state(10, 0xaa, None, 1)).unwrap();

    // not required + module unavailable → tip served, verdict Unavailable
    let t = tip(&deps, &env);
    assert_eq!(t.tip.as_ref().unwrap().height, 10);
    assert_eq!(t.quorum.unwrap().verdict, QuorumVerdict::Unavailable);

    // quorum root agrees
    set_root(&mut deps, 10, 0xaa);
    let t = tip(&deps, &env);
    assert!(t.tip.is_some());
    assert_eq!(t.quorum.unwrap().verdict, QuorumVerdict::Agrees);

    // quorum lags / leads
    set_root(&mut deps, 9, 0x01);
    assert_eq!(
        tip(&deps, &env).quorum.unwrap().verdict,
        QuorumVerdict::QuorumBehind
    );
    set_root(&mut deps, 11, 0x01);
    assert_eq!(
        tip(&deps, &env).quorum.unwrap().verdict,
        QuorumVerdict::QuorumAhead
    );

    // quorum root conflicts at the tip height → tip withheld, new reports rejected
    set_root(&mut deps, 10, 0xdd);
    let t = tip(&deps, &env);
    assert!(t.tip.is_none());
    assert_eq!(t.quorum.unwrap().verdict, QuorumVerdict::Conflicts);
    assert!(matches!(
        report(&mut deps, &env, &a, 1, state(10, 0xaa, None, 2)),
        Err(ContractError::QuorumConflict { height: 10 })
    ));
    report(&mut deps, &env, &a, 1, state(10, 0xdd, None, 3)).unwrap();

    // require = true fails closed when the quorum is unreachable
    deps.querier.root = None;
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&admin, &[]),
        ExecuteMsg::UpdateConfig {
            admin: None,
            hash_market_capability: Some(cap(true)),
            max_report_age_blocks: None,
        },
    )
    .unwrap();
    report(&mut deps, &env, &b, 2, state(10, 0xdd, None, 4)).unwrap();
    let t = tip(&deps, &env);
    assert!(t.tip.is_none(), "require=true + unavailable → withheld");
    set_root(&mut deps, 10, 0xdd);
    assert_eq!(tip(&deps, &env).tip.unwrap().hash, h32(0xdd));
}
