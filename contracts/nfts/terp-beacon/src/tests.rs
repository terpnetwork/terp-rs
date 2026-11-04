use crate::attestor::{ATTESTOR_DOMAIN, attestor_message_hash};
use crate::contract::{execute, instantiate, query, reply};
use crate::drand::{round_after, round_publish_time};
use crate::error::ContractError;
use crate::fixtures::{
    ChainFixture, DEFAULT_CHAIN, DEFAULT_ROUND, QUICKNET_CHAIN, QUICKNET_ROUND, RoundFixture,
    UNRELATED_G1_PUBKEY_HEX, UNRELATED_G2_PUBKEY_HEX,
};
use crate::jobs::{MAX_DELIVERIES_PER_TX, deliver_queued};
use crate::msg::{ExecuteMsg, QueryMsg};
use crate::nois::{NoisCallback, ProxyExecuteMsg, ReceiverExecuteMsg};
use crate::source::{BeaconSource, RESERVED_IBC_SOURCE_ID};
use crate::state::{BEACONS, CONFIG, StoredBeacon};
use cosmwasm_std::{
    Addr, Binary, Coin, CosmosMsg, DepsMut, Empty, Env, HexBinary, MessageInfo, Response,
    StdResult, Timestamp, WasmMsg, coin, coins, from_json,
    testing::{MockApi, message_info, mock_dependencies, mock_env},
    to_json_binary,
};
use cw_multi_test::{App, BankSudo, Contract, ContractWrapper, Executor, SudoMsg};
use cw_storage_plus::Item;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use sha2::{Digest, Sha256};

/// `block + 5s` is the publish time of quicknet round R-1, so GetNextRandomness
/// commits to [`QUICKNET_ROUND`].
/// Valid bech32 test address, same derivation as `MockApi::addr_make`.
fn a(name: &str) -> Addr {
    MockApi::default().addr_make(name)
}

const TEST_TIME: u64 = QUICKNET_ROUND.published_unix - 3 - 5;

fn env_at(seconds: u64) -> Env {
    let mut env = mock_env();
    env.block.time = Timestamp::from_seconds(seconds);
    env.block.height = 10;
    env
}

fn init(
    chain: &ChainFixture,
    prices: Vec<Coin>,
    required: u32,
) -> (
    cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >,
    Env,
) {
    let mut deps = mock_dependencies();
    let env = env_at(TEST_TIME);
    instantiate(
        deps.as_mut(),
        env.clone(),
        message_info(&a("admin"), &[]),
        chain.instantiate_msg(a("admin").as_str(), prices, required),
    )
    .unwrap();
    (deps, env)
}

fn round_msg(round: &RoundFixture) -> ExecuteMsg {
    ExecuteMsg::AddRound {
        round: round.round,
        signature: HexBinary::from_hex(round.signature_hex).unwrap(),
        previous_signature: round
            .previous_signature_hex
            .map(|hex| HexBinary::from_hex(hex).unwrap()),
    }
}

fn tamper(hex_bytes: &str) -> HexBinary {
    let mut bytes = HexBinary::from_hex(hex_bytes).unwrap().to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    bytes.into()
}

fn add(deps: DepsMut, env: &Env, msg: ExecuteMsg) -> Result<Response, ContractError> {
    execute(deps, env.clone(), message_info(&a("bot"), &[]), msg)
}

#[test]
fn target_round_matches_nois_round_after() {
    let genesis = Timestamp::from_seconds(QUICKNET_CHAIN.genesis_time);
    let period_ns = QUICKNET_CHAIN.period_seconds * 1_000_000_000;
    // nois round_after: 1 before genesis, else floor((after - genesis) / period) + 2.
    let expect = |after: Timestamp| -> u64 {
        if after < genesis {
            1
        } else {
            (after.nanos() - genesis.nanos()) / period_ns + 2
        }
    };
    let points = [
        Timestamp::from_seconds(0),
        genesis.minus_nanos(1),
        genesis,
        genesis.plus_nanos(1),
        genesis.plus_seconds(2),
        genesis.plus_seconds(3),
        genesis.plus_seconds(4),
        Timestamp::from_seconds(QUICKNET_ROUND.published_unix).minus_nanos(1),
        Timestamp::from_seconds(QUICKNET_ROUND.published_unix),
    ];
    for after in points {
        let got = round_after(genesis, QUICKNET_CHAIN.period_seconds, after).unwrap();
        assert_eq!(got, expect(after), "after {}", after);
    }
    let publish = Timestamp::from_seconds(QUICKNET_ROUND.published_unix);
    assert_eq!(
        round_after(
            genesis,
            QUICKNET_CHAIN.period_seconds,
            publish.minus_nanos(1)
        )
        .unwrap(),
        QUICKNET_ROUND.round
    );
    assert_eq!(
        round_after(genesis, QUICKNET_CHAIN.period_seconds, publish).unwrap(),
        QUICKNET_ROUND.round + 1
    );
}

#[test]
fn publish_time_matches_drand_formula() {
    let quick = round_publish_time(
        Timestamp::from_seconds(QUICKNET_CHAIN.genesis_time),
        QUICKNET_CHAIN.period_seconds,
        QUICKNET_ROUND.round,
    )
    .unwrap();
    assert_eq!(
        quick,
        Timestamp::from_seconds(QUICKNET_ROUND.published_unix)
    );

    let chained = round_publish_time(
        Timestamp::from_seconds(DEFAULT_CHAIN.genesis_time),
        DEFAULT_CHAIN.period_seconds,
        DEFAULT_ROUND.round,
    )
    .unwrap();
    assert_eq!(
        chained,
        Timestamp::from_seconds(DEFAULT_ROUND.published_unix)
    );
    assert!(RESERVED_IBC_SOURCE_ID > BeaconSource::Attestor.source_id());
}

#[test]
fn quicknet_round_verifies_and_rejects_tampering() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 0);
    let res = add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap();
    assert!(
        res.attributes
            .iter()
            .any(|a| a.key == "randomness" && a.value == QUICKNET_ROUND.randomness_hex)
    );

    let beacon: crate::msg::BeaconResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Beacon {
                round: QUICKNET_ROUND.round,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(beacon.randomness.to_hex(), QUICKNET_ROUND.randomness_hex);
    assert_eq!(beacon.source, BeaconSource::Drand);
    assert!(beacon.deliverable);
    assert_eq!(
        beacon.published,
        Timestamp::from_seconds(QUICKNET_ROUND.published_unix)
    );

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: QUICKNET_ROUND.round,
            signature: tamper(QUICKNET_ROUND.signature_hex),
            previous_signature: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: QUICKNET_ROUND.round + 1,
            signature: HexBinary::from_hex(QUICKNET_ROUND.signature_hex).unwrap(),
            previous_signature: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: QUICKNET_ROUND.round,
            signature: HexBinary::from_hex(QUICKNET_ROUND.signature_hex).unwrap(),
            previous_signature: Some(HexBinary::from_hex("aa").unwrap()),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::UnexpectedPreviousSignature);
}

#[test]
fn quicknet_wrong_pubkey_rejects_a_valid_round() {
    let mut chain = QUICKNET_CHAIN;
    chain.pubkey_hex = UNRELATED_G2_PUBKEY_HEX;
    let (mut deps, env) = init(&chain, vec![], 0);
    let err = add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);
}

#[test]
fn chained_round_verifies_and_rejects_tampering() {
    let (mut deps, env) = init(&DEFAULT_CHAIN, vec![], 0);
    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: DEFAULT_ROUND.round,
            signature: HexBinary::from_hex(DEFAULT_ROUND.signature_hex).unwrap(),
            previous_signature: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::MissingPreviousSignature);

    let res = add(deps.as_mut(), &env, round_msg(&DEFAULT_ROUND)).unwrap();
    assert!(
        res.attributes
            .iter()
            .any(|a| a.key == "randomness" && a.value == DEFAULT_ROUND.randomness_hex)
    );

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: DEFAULT_ROUND.round,
            signature: tamper(DEFAULT_ROUND.signature_hex),
            previous_signature: DEFAULT_ROUND
                .previous_signature_hex
                .map(|h| HexBinary::from_hex(h).unwrap()),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: DEFAULT_ROUND.round + 1,
            signature: HexBinary::from_hex(DEFAULT_ROUND.signature_hex).unwrap(),
            previous_signature: Some(HexBinary::from_hex(DEFAULT_ROUND.signature_hex).unwrap()),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);

    let err = add(
        deps.as_mut(),
        &env,
        ExecuteMsg::AddRound {
            round: DEFAULT_ROUND.round,
            signature: HexBinary::from_hex(DEFAULT_ROUND.signature_hex).unwrap(),
            previous_signature: Some(tamper(DEFAULT_ROUND.previous_signature_hex.unwrap())),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);
}

#[test]
fn chained_wrong_pubkey_rejects_a_valid_round() {
    let mut chain = DEFAULT_CHAIN;
    chain.pubkey_hex = UNRELATED_G1_PUBKEY_HEX;
    let (mut deps, env) = init(&chain, vec![], 0);
    let err = add(deps.as_mut(), &env, round_msg(&DEFAULT_ROUND)).unwrap_err();
    assert_eq!(err, ContractError::InvalidSignature);
}

#[test]
fn instantiate_rejects_a_bad_pubkey() {
    let mut deps = mock_dependencies();
    let mut msg = QUICKNET_CHAIN.instantiate_msg(a("admin").as_str(), vec![], 0);
    msg.pubkey = HexBinary::from_hex(&"11".repeat(96)).unwrap();
    let err = instantiate(
        deps.as_mut(),
        env_at(TEST_TIME),
        message_info(&a("admin"), &[]),
        msg,
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidPubkey);
}

fn sign(secret: [u8; 32], round: u64, randomness: &[u8]) -> (HexBinary, HexBinary) {
    let hash = attestor_message_hash(round, randomness);
    let sk = SigningKey::from_slice(&secret).unwrap();
    let sig: k256::ecdsa::Signature = sk.sign_prehash(&hash).unwrap();
    let pk = sk.verifying_key().to_sec1_point(true);
    (
        HexBinary::from(pk.as_bytes()),
        HexBinary::from(sig.to_bytes().as_slice()),
    )
}

#[test]
fn attestor_add_remove_and_verify() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 1);
    add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap();
    let randomness = HexBinary::from_hex(QUICKNET_ROUND.randomness_hex).unwrap();

    let mut preimage = Sha256::new();
    preimage.update(ATTESTOR_DOMAIN);
    preimage.update(QUICKNET_ROUND.round.to_be_bytes());
    preimage.update(randomness.as_slice());
    let expected: [u8; 32] = preimage.finalize().into();
    assert_eq!(
        attestor_message_hash(QUICKNET_ROUND.round, randomness.as_slice()),
        expected
    );

    let secret = *b"terp-beacon-test-attestor-key!!!";
    let (pubkey, signature) = sign(secret, QUICKNET_ROUND.round, randomness.as_slice());

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("stranger"), &[]),
        ExecuteMsg::AddAttestor {
            pubkey: pubkey.clone(),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("admin"), &[]),
        ExecuteMsg::AddAttestor {
            pubkey: pubkey.clone(),
        },
    )
    .unwrap();

    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("bot"), &[]),
        ExecuteMsg::Attest {
            round: QUICKNET_ROUND.round,
            pubkey: pubkey.clone(),
            signature: tamper(&signature.to_hex()),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidAttestation);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("bot"), &[]),
        ExecuteMsg::Attest {
            round: QUICKNET_ROUND.round,
            pubkey: pubkey.clone(),
            signature: signature.clone(),
        },
    )
    .unwrap();

    let listed: crate::msg::AttestorsResponse =
        from_json(query(deps.as_ref(), env.clone(), QueryMsg::Attestors {}).unwrap()).unwrap();
    assert_eq!(listed.attestors.len(), 1);
    assert_eq!(listed.attestors[0].pubkey, pubkey);

    let proofs: crate::msg::AttestationsResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Attestations {
                round: QUICKNET_ROUND.round,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proofs.attestations.len(), 1);
    assert_eq!(proofs.attestations[0].source, BeaconSource::Attestor);

    let beacon: crate::msg::BeaconResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Beacon {
                round: QUICKNET_ROUND.round,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(beacon.deliverable);
    assert_eq!(beacon.attestation_count, 1);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("admin"), &[]),
        ExecuteMsg::RemoveAttestor {
            pubkey: pubkey.clone(),
        },
    )
    .unwrap();
    let beacon: crate::msg::BeaconResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Beacon {
                round: QUICKNET_ROUND.round,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!beacon.deliverable);
    assert_eq!(beacon.attestation_count, 0);

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&a("bot"), &[]),
        ExecuteMsg::Attest {
            round: QUICKNET_ROUND.round,
            pubkey,
            signature,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::AttestorNotRegistered);
}

#[test]
fn admin_transfer_and_config() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 0);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("admin"), &[]),
        ExecuteMsg::TransferAdmin {
            new_admin: a("next").to_string(),
        },
    )
    .unwrap();
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("admin"), &[]),
        ExecuteMsg::UpdateConfig {
            required_attestations: Some(2),
            prices: Some(coins(1, "unois")),
            callback_gas_limit: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized);
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("next"), &[]),
        ExecuteMsg::UpdateConfig {
            required_attestations: Some(2),
            prices: Some(coins(1, "unois")),
            callback_gas_limit: Some(800_000),
        },
    )
    .unwrap();
    let cfg: crate::msg::ConfigResponse =
        from_json(query(deps.as_ref(), env, QueryMsg::Config {}).unwrap()).unwrap();
    assert_eq!(cfg.admin, a("next").as_str());
    assert_eq!(cfg.required_attestations, 2);
    assert_eq!(cfg.prices, coins(1, "unois"));
    assert_eq!(cfg.callback_gas_limit, 800_000);
    assert_eq!(cfg.source_ids.reserved_ibc, RESERVED_IBC_SOURCE_ID);
}

#[test]
fn proxy_execute_msg_json_matches_nois() {
    let nois_msg = ProxyExecuteMsg::GetNextRandomness {
        job_id: "raffle-7".to_string(),
    };
    let parsed: ExecuteMsg = from_json(to_json_binary(&nois_msg).unwrap()).unwrap();
    assert_eq!(
        parsed,
        ExecuteMsg::GetNextRandomness {
            job_id: "raffle-7".into()
        }
    );

    let after = Timestamp::from_seconds(1_700_000_000);
    let nois_msg = ProxyExecuteMsg::GetRandomnessAfter {
        after,
        job_id: "raffle-8".to_string(),
    };
    let parsed: ExecuteMsg = from_json(to_json_binary(&nois_msg).unwrap()).unwrap();
    assert_eq!(
        parsed,
        ExecuteMsg::GetRandomnessAfter {
            after,
            job_id: "raffle-8".into()
        }
    );

    let callback = ReceiverExecuteMsg::NoisReceive {
        callback: NoisCallback {
            job_id: "raffle-7".into(),
            published: Timestamp::from_seconds(QUICKNET_ROUND.published_unix),
            randomness: HexBinary::from_hex(QUICKNET_ROUND.randomness_hex).unwrap(),
        },
    };
    let raw = to_json_binary(&callback).unwrap();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    assert!(text.contains("\"nois_receive\""));
}

const CALLBACKS: Item<Vec<NoisCallback>> = Item::new("callbacks");

fn recv_instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: Empty,
) -> StdResult<Response> {
    CALLBACKS.save(deps.storage, &vec![])?;
    Ok(Response::new())
}

fn recv_execute(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: ReceiverExecuteMsg,
) -> StdResult<Response> {
    let ReceiverExecuteMsg::NoisReceive { callback } = msg;
    let mut all = CALLBACKS.load(deps.storage)?;
    all.push(callback);
    CALLBACKS.save(deps.storage, &all)?;
    Ok(Response::new())
}

fn recv_query(deps: cosmwasm_std::Deps, _env: Env, _msg: Empty) -> StdResult<Binary> {
    to_json_binary(&CALLBACKS.load(deps.storage)?)
}

fn beacon_box() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(execute, instantiate, query).with_reply(reply))
}

fn receiver_box() -> Box<dyn Contract<Empty>> {
    Box::new(ContractWrapper::new(
        recv_execute,
        recv_instantiate,
        recv_query,
    ))
}

struct World {
    app: App,
    beacon: Addr,
    receiver: Addr,
}

fn world(prices: Vec<Coin>, required: u32) -> World {
    let mut app = App::default();
    app.update_block(|block| {
        block.time = Timestamp::from_seconds(TEST_TIME);
        block.height = 10;
    });
    let beacon_id = app.store_code(beacon_box());
    let receiver_id = app.store_code(receiver_box());
    let receiver = app
        .instantiate_contract(receiver_id, a("admin"), &Empty {}, &[], "receiver", None)
        .unwrap();
    let beacon = app
        .instantiate_contract(
            beacon_id,
            a("admin"),
            &QUICKNET_CHAIN.instantiate_msg(a("admin").as_str(), prices, required),
            &[],
            "beacon",
            None,
        )
        .unwrap();
    World {
        app,
        beacon,
        receiver,
    }
}

fn callbacks(app: &App, receiver: &Addr) -> Vec<NoisCallback> {
    app.wrap().query_wasm_smart(receiver, &Empty {}).unwrap()
}

#[test]
fn queues_until_round_then_delivers_and_batches() {
    let mut world = world(vec![], 0);
    for i in 0..MAX_DELIVERIES_PER_TX + 2 {
        world
            .app
            .execute_contract(
                world.receiver.clone(),
                world.beacon.clone(),
                &ExecuteMsg::GetNextRandomness {
                    job_id: format!("job-{i}"),
                },
                &[],
            )
            .unwrap();
    }
    assert!(callbacks(&world.app, &world.receiver).is_empty());

    world
        .app
        .execute_contract(
            a("bot"),
            world.beacon.clone(),
            &round_msg(&QUICKNET_ROUND),
            &[],
        )
        .unwrap();
    let got = callbacks(&world.app, &world.receiver);
    assert_eq!(got.len(), MAX_DELIVERIES_PER_TX);
    assert_eq!(got[0].job_id, "job-0");
    assert_eq!(got[0].randomness.to_hex(), QUICKNET_ROUND.randomness_hex);
    assert_eq!(
        got[0].published,
        Timestamp::from_seconds(QUICKNET_ROUND.published_unix)
    );

    world
        .app
        .execute_contract(
            a("anyone"),
            world.beacon.clone(),
            &ExecuteMsg::ProcessQueue {},
            &[],
        )
        .unwrap();
    assert_eq!(
        callbacks(&world.app, &world.receiver).len(),
        MAX_DELIVERIES_PER_TX + 2
    );
}

#[test]
fn delivers_immediately_when_round_is_already_stored() {
    let mut world = world(vec![], 0);
    world
        .app
        .execute_contract(
            a("bot"),
            world.beacon.clone(),
            &round_msg(&QUICKNET_ROUND),
            &[],
        )
        .unwrap();
    world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetNextRandomness {
                job_id: "now".into(),
            },
            &[],
        )
        .unwrap();
    let got = callbacks(&world.app, &world.receiver);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].job_id, "now");

    world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetRandomnessAfter {
                after: Timestamp::from_seconds(QUICKNET_ROUND.published_unix),
                job_id: "too-late".into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(callbacks(&world.app, &world.receiver).len(), 1);
    let pending: crate::msg::JobResponse = world
        .app
        .wrap()
        .query_wasm_smart(
            world.beacon.clone(),
            &QueryMsg::Job {
                requester: world.receiver.to_string(),
                job_id: "too-late".into(),
            },
        )
        .unwrap();
    let pending_job = pending.job.unwrap();
    assert_eq!(pending_job.round, QUICKNET_ROUND.round + 1);

    world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetRandomnessAfter {
                after: Timestamp::from_seconds(QUICKNET_ROUND.published_unix - 1),
                job_id: "in-time".into(),
            },
            &[],
        )
        .unwrap();
    let got = callbacks(&world.app, &world.receiver);
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].job_id, "in-time");
}

#[test]
fn payment_is_checked_and_withdrawn() {
    let price = coin(500_000, "unois");
    let mut world = world(vec![price.clone()], 0);
    let err = world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetNextRandomness {
                job_id: "paid".into(),
            },
            &[],
        )
        .unwrap_err();
    assert!(format!("{err:?}").contains("Payment"));

    world
        .app
        .sudo(SudoMsg::Bank(BankSudo::Mint {
            to_address: world.receiver.to_string(),
            amount: vec![price.clone()],
        }))
        .unwrap();
    world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetNextRandomness {
                job_id: "paid".into(),
            },
            &[price],
        )
        .unwrap();

    world
        .app
        .execute_contract(
            a("admin"),
            world.beacon.clone(),
            &ExecuteMsg::Withdraw {
                recipient: Some(a("admin").to_string()),
                denom: "unois".into(),
                amount: None,
            },
            &[],
        )
        .unwrap();
    let left = world
        .app
        .wrap()
        .query_balance(world.beacon.clone(), "unois")
        .unwrap();
    assert!(left.amount.is_zero());
    let admin_bal = world.app.wrap().query_balance(a("admin"), "unois").unwrap();
    assert_eq!(admin_bal.amount, cosmwasm_std::Uint256::from(500_000u128));
}

#[test]
fn required_attestations_gate_delivery() {
    let mut world = world(vec![], 1);
    world
        .app
        .execute_contract(
            world.receiver.clone(),
            world.beacon.clone(),
            &ExecuteMsg::GetNextRandomness {
                job_id: "gated".into(),
            },
            &[],
        )
        .unwrap();
    world
        .app
        .execute_contract(
            a("bot"),
            world.beacon.clone(),
            &round_msg(&QUICKNET_ROUND),
            &[],
        )
        .unwrap();
    assert!(callbacks(&world.app, &world.receiver).is_empty());

    let randomness = HexBinary::from_hex(QUICKNET_ROUND.randomness_hex).unwrap();
    let (pubkey, signature) = sign(
        *b"terp-beacon-test-attestor-key!!!",
        QUICKNET_ROUND.round,
        randomness.as_slice(),
    );
    world
        .app
        .execute_contract(
            a("admin"),
            world.beacon.clone(),
            &ExecuteMsg::AddAttestor {
                pubkey: pubkey.clone(),
            },
            &[],
        )
        .unwrap();
    world
        .app
        .execute_contract(
            a("bot"),
            world.beacon.clone(),
            &ExecuteMsg::Attest {
                round: QUICKNET_ROUND.round,
                pubkey,
                signature,
            },
            &[],
        )
        .unwrap();
    let got = callbacks(&world.app, &world.receiver);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].job_id, "gated");
}

fn callback_job(msg: &CosmosMsg) -> (String, String) {
    let CosmosMsg::Wasm(WasmMsg::Execute { msg, .. }) = msg else {
        panic!("expected wasm execute");
    };
    match from_json::<ReceiverExecuteMsg>(msg).unwrap() {
        ReceiverExecuteMsg::NoisReceive { callback } => {
            (callback.job_id, callback.randomness.to_hex())
        }
    }
}

#[test]
fn later_round_does_not_deliver_an_earlier_job() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 0);
    let publish = Timestamp::from_seconds(QUICKNET_ROUND.published_unix);
    let config = CONFIG.load(deps.as_ref().storage).unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("user"), &[]),
        ExecuteMsg::GetRandomnessAfter {
            after: publish.minus_nanos(1),
            job_id: "early".into(),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("user"), &[]),
        ExecuteMsg::GetRandomnessAfter {
            after: publish,
            job_id: "later".into(),
        },
    )
    .unwrap();

    let later_round = QUICKNET_ROUND.round + 1;
    let later_randomness = HexBinary::from(vec![9u8; 32]);
    BEACONS
        .save(
            deps.as_mut().storage,
            (BeaconSource::Drand.source_id(), later_round),
            &StoredBeacon {
                round: later_round,
                randomness: later_randomness.clone(),
                signature: HexBinary::from(vec![1u8; 48]),
                published: round_publish_time(
                    config.genesis_time,
                    config.period_seconds,
                    later_round,
                )
                .unwrap(),
                verified_at: env.block.time,
                source: BeaconSource::Drand,
            },
        )
        .unwrap();

    let msgs = deliver_queued(deps.as_mut().storage, &config, Some(later_round), None).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        callback_job(&msgs[0].msg),
        ("later".to_string(), later_randomness.to_hex())
    );
    let early: crate::msg::JobResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::Job {
                requester: a("user").to_string(),
                job_id: "early".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(early.job.unwrap().round, QUICKNET_ROUND.round);

    let res = add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap();
    assert_eq!(res.messages.len(), 1);
    assert_eq!(
        callback_job(&res.messages[0].msg),
        ("early".into(), QUICKNET_ROUND.randomness_hex.into())
    );
}

#[test]
fn reply_rejects_unknown_ids() {
    let mut deps = mock_dependencies();
    let err = reply(
        deps.as_mut(),
        env_at(TEST_TIME),
        #[allow(deprecated)]
        cosmwasm_std::Reply {
            id: 99,
            payload: Binary::default(),
            gas_used: 0,
            result: cosmwasm_std::SubMsgResult::Ok(cosmwasm_std::SubMsgResponse {
                events: vec![],
                data: None,
                msg_responses: vec![],
            }),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::UnknownReplyId { id: 99 });
}

/// Raffle whose end (`start + duration`) is the publish time of quicknet
/// round R-1, so the first round strictly after the end is [`QUICKNET_ROUND`].
fn mock_raffle(
    deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        MockApi,
        cosmwasm_std::testing::MockQuerier,
    >,
) {
    let duration = 60u64;
    let start = QUICKNET_ROUND.published_unix - QUICKNET_CHAIN.period_seconds - duration;
    deps.querier.update_wasm(move |_| {
        let body = serde_json::json!({
            "raffle_id": 7,
            "state": "closed",
            "info": {
                "owner": "x",
                "options": {
                    "start": Timestamp::from_seconds(start),
                    "duration": duration,
                    "comment": null,
                    "max_ticket_number": null,
                    "max_ticket_per_address": null,
                    "min_ticket_number": null,
                    "one_winner_per_asset": false
                },
                "randomness": null,
                "winners": [],
                "is_cancelled": false
            }
        });
        cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(
            to_json_binary(&body).unwrap(),
        ))
    });
}

#[test]
fn provide_answers_raffle_with_round_after_end() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 0);
    mock_raffle(&mut deps);
    let owner = a("owner");
    let provide = ExecuteMsg::Provide {
        raffle_id: 7,
        owner: owner.to_string(),
    };
    let err = execute(
        deps.as_mut(),
        env.clone(),
        message_info(&a("raffle"), &[]),
        provide.clone(),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::RoundUnknown {
            round: QUICKNET_ROUND.round
        }
    );

    add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap();
    let res = execute(deps.as_mut(), env, message_info(&a("raffle"), &[]), provide).unwrap();
    let get = |key: &str| {
        res.attributes
            .iter()
            .find(|attr| attr.key == key)
            .map(|attr| attr.value.clone())
            .unwrap()
    };
    assert_eq!(get("action"), "provide");
    assert_eq!(get("raffle_id"), "7");
    assert_eq!(get("round"), QUICKNET_ROUND.round.to_string());
    assert_eq!(get("owner"), owner.to_string());
    let seed = Binary::from_base64(&get("randomness")).unwrap();
    assert_eq!(
        HexBinary::from(seed.as_slice()).to_hex(),
        QUICKNET_ROUND.randomness_hex
    );
}

#[test]
fn provide_waits_for_attestations() {
    let (mut deps, env) = init(&QUICKNET_CHAIN, vec![], 1);
    mock_raffle(&mut deps);
    add(deps.as_mut(), &env, round_msg(&QUICKNET_ROUND)).unwrap();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&a("raffle"), &[]),
        ExecuteMsg::Provide {
            raffle_id: 7,
            owner: a("owner").to_string(),
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::RoundNotDeliverable {
            round: QUICKNET_ROUND.round
        }
    );
}
