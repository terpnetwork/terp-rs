//! Minimal cw4 DAO on two local terp chains, then a polytone account across them.
//!
//! Each chain opens a cw-orch-kms session first. A local cold key signs the
//! loopback amino bundles (the same check the phone page accepts). The
//! ephemeral key then stores `dao_dao_core`, `dao_proposal_single`,
//! `dao_voting_cw4`, and `cw4_group` on both chains at once. Those stores do
//! not share an account sequence. The same session grants the relay message
//! set on both chains before Hermes opens clients. Chain A instantiates the
//! DAO and the polytone note. Chain B instantiates the voice. A second
//! session lets the ephemeral key propose and execute an empty note message,
//! which creates the proxy on chain B.
//!
//! ```text
//! ICT_CW4_POLYTONE=1 TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=local-arm64 \
//!   cargo test -p dao-foundation-suite --test cw4_polytone_two_chains -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cosmwasm_std::Binary;
use cw_orch::prelude::{Addr, IndexResponse, TxHandler, WasmPath};
use cw_orch_kms::{
    ApprovalSession, AuthzSession, ScopedCall, SigningRequest, enable_session, planned_spend_limit,
};
use ict_rs::chain::cosmos::CosmosChain;
use ict_rs::chain::{Chain, GenesisStyle, TestContext};
use ict_rs::cosmwasm::CosmWasmExt;
use ict_rs::ibc::{ChannelOptions, ChannelOrdering, ClientOptions};
use ict_rs::relayer::{Relayer, RelayerType, build_relayer};
use ict_rs::runtime::DockerConfig;
use ict_rs::runtime::RuntimeBackend;
use ict_rs::testing::TestEnv;
use ict_rs::tx::WalletAmount;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use serde_json::{Value, json};
use sha2::Digest;

/// Published test mnemonic. The ephemeral cw-orch key. Never printed.
const EPHEMERAL: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// Local cold key. It signs the loopback grant page and is not the broadcaster.
const COLD: [u8; 32] = [9u8; 32];
const PAGE: &str = "127.0.0.1:8741";

const NETWORK: &str = "ict-cw4-polytone";
const DAO: [&str; 4] = [
    "dao_dao_core.wasm",
    "dao_proposal_single.wasm",
    "dao_voting_cw4.wasm",
    "cw4_group.wasm",
];
const POLY: [&str; 3] = [
    "polytone_note.wasm",
    "polytone_voice.wasm",
    "polytone_proxy.wasm",
];

struct Codes {
    core: u64,
    proposal: u64,
    voting: u64,
    group: u64,
    note: u64,
    voice: u64,
    proxy: u64,
}

#[tokio::test]
#[ignore = "docker: ICT_CW4_POLYTONE=1 TERP_IMAGE_REPO TERP_IMAGE_VERSION"]
async fn cw4_dao_opens_polytone_account_on_two_terp_chains() {
    if std::env::var("ICT_CW4_POLYTONE").ok().as_deref() != Some("1") {
        panic!(
            "set ICT_CW4_POLYTONE=1 TERP_IMAGE_REPO TERP_IMAGE_VERSION \
             before running this ignored test"
        );
    }
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("ICT_MOCK does not open a polytone channel");
    }

    let dao_dir = dao_artifacts();
    let poly_dir = poly_artifacts();
    let backend = ict_rs::runtime::docker::DockerBackend::new(DockerConfig::default())
        .await
        .expect("docker");
    let runtime: Arc<dyn RuntimeBackend> = Arc::new(backend);
    let ctx = TestContext {
        test_name: "cw4-polytone".into(),
        network_id: NETWORK.into(),
    };
    let mut chain_a = CosmosChain::new(terp_config("cw4-a"), 1, 0, runtime.clone());
    let mut chain_b = CosmosChain::new(terp_config("cw4-b"), 1, 0, runtime.clone());
    chain_a.initialize(&ctx).await.expect("init chain a");
    chain_b.initialize(&ctx).await.expect("init chain b");
    chain_a.start(&[]).await.expect("start chain a");
    chain_b.start(&[]).await.expect("start chain b");

    let result = exercise(&chain_a, &chain_b, runtime, &dao_dir, &poly_dir).await;
    let stop_a = chain_a.stop().await;
    let stop_b = chain_b.stop().await;
    result.expect("cw4 polytone exercise");
    stop_a.expect("stop chain a");
    stop_b.expect("stop chain b");
}

async fn exercise(
    chain_a: &CosmosChain,
    chain_b: &CosmosChain,
    runtime: Arc<dyn RuntimeBackend>,
    dao_dir: &Path,
    poly_dir: &Path,
) -> Result<(), String> {
    let granter = cold_address(&COLD)?;
    let ephemeral = ephemeral_address(chain_a)?;
    if granter == ephemeral {
        return Err("granter and ephemeral are the same key".into());
    }
    println!("cw-orch-kms granter {granter}");
    println!("cw-orch-kms ephemeral {ephemeral}");
    fund_address(chain_a, &granter).await?;
    fund_address(chain_b, &granter).await?;
    fund_address(chain_a, &ephemeral).await?;
    fund_address(chain_b, &ephemeral).await?;

    let wasms = read_wasms(dao_dir, poly_dir)?;
    let mut signer = tokio::spawn(sign_pages(COLD, 2));
    let mut session_a =
        await_session(open_wasm_session(chain_a, &granter, &wasms), &mut signer).await?;
    let _ = session_a.detach_page();
    let mut session_b =
        await_session(open_wasm_session(chain_b, &granter, &wasms), &mut signer).await?;
    let _ = session_b.detach_page();
    finish_signer(&mut signer).await?;
    println!("cw-orch-kms sessions open on cw4-a and cw4-b");

    let (codes_a, codes_b) = tokio::join!(
        store_codes(&session_a, dao_dir, poly_dir),
        store_codes(&session_b, dao_dir, poly_dir),
    );
    let codes_a = codes_a?;
    let codes_b = codes_b?;
    println!(
        "chain a codes core={} proposal={} voting={} group={} note={}",
        codes_a.core, codes_a.proposal, codes_a.voting, codes_a.group, codes_a.note
    );
    println!(
        "chain b codes core={} proposal={} voting={} group={} voice={} proxy={}",
        codes_b.core, codes_b.proposal, codes_b.voting, codes_b.group, codes_b.voice, codes_b.proxy
    );

    let (dao_a, dao_b) = tokio::join!(
        instantiate_dao(chain_a, &session_a, &ephemeral, &codes_a),
        instantiate_dao(chain_b, &session_b, &ephemeral, &codes_b),
    );
    let dao_a = dao_a?;
    let dao_b = dao_b?;
    println!("dao {dao_a} on cw4-a; dao {dao_b} on cw4-b");

    let note = instantiate_note(&session_a, codes_a.note).await?;
    let voice = instantiate_voice(&session_b, codes_b.voice, codes_b.proxy).await?;
    println!("note {note}; voice {voice}");

    let relayer = build_relayer(RelayerType::Hermes, runtime, "cw4-polytone", NETWORK)
        .await
        .map_err(|err| err.to_string())?;
    configure_relayer(relayer.as_ref(), chain_a, chain_b).await?;
    relayer
        .create_channel(
            "polytone",
            &ChannelOptions {
                src_port: format!("wasm.{note}"),
                dst_port: format!("wasm.{voice}"),
                ordering: ChannelOrdering::Unordered,
                version: "polytone-1".into(),
            },
        )
        .await
        .map_err(|err| format!("polytone channel: {err}"))?;
    relayer
        .start(&["polytone"])
        .await
        .map_err(|err| err.to_string())?;

    off_runtime(|| session_a.close()).map_err(|err| format!("close cw4-a session: {err}"))?;
    off_runtime(|| session_b.close()).map_err(|err| format!("close cw4-b session: {err}"))?;

    let proposal = proposal_module(chain_a, &dao_a).await?;
    let mut signer = tokio::spawn(sign_pages(COLD, 1));
    let mut exec = await_session(
        open_execute_session(chain_a, &granter, &proposal),
        &mut signer,
    )
    .await?;
    let _ = exec.detach_page();
    finish_signer(&mut signer).await?;
    let note_msg = json!({
        "execute": { "msgs": [], "callback": null, "timeout_seconds": "300" }
    });
    let propose = json!({
        "propose": {
            "title": "open polytone account",
            "description": "empty execute creates the remote proxy",
            "msgs": [{
                "wasm": {
                    "execute": {
                        "contract_addr": note,
                        "msg": Binary::new(serde_json::to_vec(&note_msg).expect("note msg")).to_base64(),
                        "funds": []
                    }
                }
            }],
            "proposer": null,
            "vote": {"vote": "yes", "rationale": null}
        }
    });
    let proposal_addr = Addr::unchecked(&proposal);
    off_runtime(|| exec.daemon.execute(&propose, &[], &proposal_addr))
        .map_err(|err| format!("dao propose: {err}"))?;
    off_runtime(|| {
        exec.daemon
            .execute(&json!({"execute": {"proposal_id": 1}}), &[], &proposal_addr)
    })
    .map_err(|err| format!("dao execute proposal: {err}"))?;
    off_runtime(|| exec.close()).map_err(|err| format!("close proposal session: {err}"))?;

    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
    let remote = chain_a
        .query_contract(
            &note,
            &json!({ "remote_address": { "local_address": dao_a } }).to_string(),
        )
        .await
        .map_err(|err| format!("remote address: {err}"))?;
    let text = remote.to_string();
    assert!(
        text.contains("terp1"),
        "voice chain did not create a proxy for the dao: {text}"
    );
    println!("remote account {text}");
    relayer.stop().await.map_err(|err| err.to_string())?;
    Ok(())
}

fn read_wasms(dao_dir: &Path, poly_dir: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    for name in DAO {
        out.push(std::fs::read(dao_dir.join(name)).map_err(|err| format!("read {name}: {err}"))?);
    }
    for name in POLY {
        out.push(std::fs::read(poly_dir.join(name)).map_err(|err| format!("read {name}: {err}"))?);
    }
    Ok(out)
}

fn wasm_approval(granter: &str, wasms: &[Vec<u8>]) -> Result<ApprovalSession, String> {
    let mut session = ApprovalSession::open(granter).allow_instantiate();
    session = session.allow_relay()?;
    for wasm in wasms {
        session = session.allow_store(wasm)?;
    }
    session.ttl_secs = 60 * 60;
    session.allowance_limit = planned_spend_limit(10);
    Ok(session)
}

fn daemon_builder(chain: &CosmosChain) -> Result<cw_orch::prelude::DaemonBuilder, String> {
    ict_rs_cw_orch::daemon_builder_from_chain(chain, Some(EPHEMERAL)).map_err(|err| err.to_string())
}

async fn await_session(
    opening: impl std::future::Future<Output = Result<AuthzSession, String>>,
    signer: &mut tokio::task::JoinHandle<Result<(), String>>,
) -> Result<AuthzSession, String> {
    tokio::pin!(opening);
    let sleep = tokio::time::sleep(Duration::from_secs(240));
    tokio::pin!(sleep);
    // The cold key finishes when the page says `authorized`. The grant
    // transactions are still being included then, so that is not a failure.
    let mut signer_done = false;
    loop {
        tokio::select! {
            opened = &mut opening => return opened,
            signed = &mut *signer, if !signer_done => {
                signer_done = true;
                signed.map_err(|err| err.to_string()).and_then(|result| result)?;
            }
            _ = &mut sleep => return Err("session grant timed out".into()),
        }
    }
}

async fn finish_signer(
    signer: &mut tokio::task::JoinHandle<Result<(), String>>,
) -> Result<(), String> {
    if signer.is_finished() {
        return Ok(());
    }
    signer.await.map_err(|err| err.to_string())?
}

fn ephemeral_address(chain: &CosmosChain) -> Result<String, String> {
    let info = ict_rs_cw_orch::chain_info_from_cosmos(chain).map_err(|err| err.to_string())?;
    // DaemonBuilder::build starts cw-orch's runtime. That panics on a tokio
    // worker, so the build runs on a plain thread.
    off_runtime(move || {
        let mut builder = cw_orch::prelude::DaemonBuilder::new(info);
        builder.is_test(true).mnemonic(EPHEMERAL);
        let daemon = builder.build().map_err(|err| err.to_string())?;
        Ok(daemon.sender_addr().into_string())
    })
}

fn off_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(work).join().expect("daemon thread panicked"))
}

async fn open_wasm_session(
    chain: &CosmosChain,
    granter: &str,
    wasms: &[Vec<u8>],
) -> Result<AuthzSession, String> {
    let builder = daemon_builder(chain)?;
    let session = wasm_approval(granter, wasms)?;
    let chain_id = chain.chain_id().to_string();
    let actions = vec![SigningRequest::new("session-key").body(format!(
        "store cw4 and polytone on {chain_id}, instantiate, and grant relay messages"
    ))];
    tokio::task::spawn_blocking(move || enable_session(&builder, session, actions))
        .await
        .map_err(|err| err.to_string())?
}

async fn open_execute_session(
    chain: &CosmosChain,
    granter: &str,
    proposal: &str,
) -> Result<AuthzSession, String> {
    let builder = daemon_builder(chain)?;
    let mut session = ApprovalSession::open(granter);
    session = session.allow_execute(ScopedCall {
        address: proposal.to_string(),
        keys: vec!["propose".into(), "execute".into()],
        calls: 2,
        funds: vec![],
    })?;
    session.ttl_secs = 60 * 60;
    session.allowance_limit = planned_spend_limit(3);
    let actions =
        vec![SigningRequest::new("session-key").body(format!("propose and execute on {proposal}"))];
    tokio::task::spawn_blocking(move || enable_session(&builder, session, actions))
        .await
        .map_err(|err| err.to_string())?
}

async fn store_codes(
    session: &AuthzSession,
    dao_dir: &Path,
    poly_dir: &Path,
) -> Result<Codes, String> {
    let files = [
        ("core", dao_dir.join(DAO[0])),
        ("proposal", dao_dir.join(DAO[1])),
        ("voting", dao_dir.join(DAO[2])),
        ("group", dao_dir.join(DAO[3])),
        ("note", poly_dir.join(POLY[0])),
        ("voice", poly_dir.join(POLY[1])),
        ("proxy", poly_dir.join(POLY[2])),
    ];
    let mut ids = Vec::with_capacity(files.len());
    for (name, path) in files {
        let wasm = WasmPath::new(&path).map_err(|err| format!("wasm {name}: {err}"))?;
        let response = off_runtime(|| {
            let handle = session.daemon.rt_handle.clone();
            handle.block_on(cw_orch::daemon::upload_wasm_plain(
                session.daemon.sender(),
                wasm,
                None,
            ))
        })
        .map_err(|err| format!("store {name}: {err}"))?;
        let id = response
            .uploaded_code_id()
            .map_err(|err| format!("code id {name}: {err}"))?;
        println!("stored {name} code {id} tx {}", response.txhash);
        ids.push(id);
    }
    Ok(Codes {
        core: ids[0],
        proposal: ids[1],
        voting: ids[2],
        group: ids[3],
        note: ids[4],
        voice: ids[5],
        proxy: ids[6],
    })
}

async fn instantiate_dao(
    chain: &CosmosChain,
    session: &AuthzSession,
    member: &str,
    codes: &Codes,
) -> Result<String, String> {
    let msg = json!({
        "admin": null,
        "name": "cw4",
        "description": "cw4",
        "image_url": null,
        "automatically_add_cw20s": false,
        "automatically_add_cw721s": false,
        "voting_module_instantiate_info": {
            "code_id": codes.voting,
            "msg": b64(&json!({
                "group_contract": {
                    "new": {
                        "cw4_group_code_id": codes.group,
                        "cw4_group_salt": null,
                        "initial_members": [{"addr": member, "weight": 1}]
                    }
                }
            })),
            "admin": {"core_module": {}},
            "funds": null,
            "label": "voting",
            "salt": null
        },
        "proposal_modules_instantiate_info": [{
            "code_id": codes.proposal,
            "msg": b64(&json!({
                "threshold": {"absolute_count": {"threshold": "1"}},
                "max_voting_period": {"time": 86400},
                "min_voting_period": null,
                "only_members_execute": false,
                "allow_revoting": false,
                "pre_propose_info": {"anyone_may_propose": {}},
                "close_proposal_on_execution_failure": true,
                "veto": null,
                "delegation_module": null
            })),
            "admin": {"core_module": {}},
            "funds": null,
            "label": "proposal",
            "salt": null
        }],
        "initial_items": null,
        "initial_actions": null,
        "dao_uri": null
    });
    let response = off_runtime(|| {
        session
            .daemon
            .instantiate(codes.core, &msg, Some("cw4-dao"), None, &[])
    })
    .map_err(|err| format!("instantiate dao: {err}"))?;
    println!("dao instantiate tx {}", response.txhash);
    let first = response
        .instantiated_contract_address()
        .map(|addr| addr.into_string())
        .unwrap_or_default();
    let listed = chain
        .chain_exec(&[
            "query",
            "wasm",
            "list-contract-by-code",
            &codes.core.to_string(),
            "--output",
            "json",
        ])
        .await
        .map_err(|err| format!("list cores: {err}"))?;
    let parsed: Value =
        serde_json::from_slice(&listed.stdout).map_err(|err| format!("list cores json: {err}"))?;
    let addrs = parsed["contracts"].as_array().cloned().unwrap_or_default();
    for addr in addrs {
        let Some(addr) = addr.as_str() else { continue };
        if chain
            .query_contract(addr, &json!({"proposal_modules": {}}).to_string())
            .await
            .is_ok()
        {
            return Ok(addr.to_string());
        }
    }
    Ok(first)
}

async fn proposal_module(chain: &CosmosChain, dao: &str) -> Result<String, String> {
    let value = chain
        .query_contract(dao, &json!({"proposal_modules": {}}).to_string())
        .await
        .map_err(|err| format!("proposal modules: {err}"))?;
    let rows = value.get("data").unwrap_or(&value);
    rows.as_array()
        .and_then(|items| items.first())
        .and_then(|row| row.get("address").and_then(|addr| addr.as_str()))
        .map(str::to_string)
        .ok_or_else(|| format!("proposal module missing in {value}"))
}

async fn instantiate_note(session: &AuthzSession, code_id: u64) -> Result<String, String> {
    let response = off_runtime(|| {
        session.daemon.instantiate(
            code_id,
            &json!({ "pair": null, "block_max_gas": "100000000" }),
            Some("polytone-note"),
            None,
            &[],
        )
    })
    .map_err(|err| format!("instantiate note: {err}"))?;
    println!("note instantiate tx {}", response.txhash);
    response
        .instantiated_contract_address()
        .map(|addr| addr.into_string())
        .map_err(|err| format!("note address: {err}"))
}

async fn instantiate_voice(
    session: &AuthzSession,
    voice_id: u64,
    proxy_id: u64,
) -> Result<String, String> {
    let response = off_runtime(|| {
        session.daemon.instantiate(
            voice_id,
            &json!({
                "proxy_code_id": proxy_id.to_string(),
                "block_max_gas": "100000000"
            }),
            Some("polytone-voice"),
            None,
            &[],
        )
    })
    .map_err(|err| format!("instantiate voice: {err}"))?;
    println!("voice instantiate tx {}", response.txhash);
    response
        .instantiated_contract_address()
        .map(|addr| addr.into_string())
        .map_err(|err| format!("voice address: {err}"))
}

async fn configure_relayer(
    relayer: &dyn Relayer,
    chain_a: &CosmosChain,
    chain_b: &CosmosChain,
) -> Result<(), String> {
    for chain in [chain_a, chain_b] {
        let chain_id = chain.chain_id();
        let key = format!("relayer-rly-{chain_id}");
        relayer
            .add_chain_configuration(
                chain.config(),
                &key,
                chain.rpc_address(),
                chain.grpc_address(),
            )
            .await
            .map_err(|err| format!("relayer config {chain_id}: {err}"))?;
        let wallet = relayer
            .add_key(chain_id, &key)
            .await
            .map_err(|err| format!("relayer key {chain_id}: {err}"))?;
        chain
            .send_funds(
                "validator",
                &WalletAmount {
                    address: wallet.formatted_address(),
                    denom: "uterp".into(),
                    amount: 100_000_000,
                },
            )
            .await
            .map_err(|err| format!("fund relayer on {chain_id}: {err}"))?;
    }
    relayer
        .generate_path(chain_a.chain_id(), chain_b.chain_id(), "polytone")
        .await
        .map_err(|err| format!("path: {err}"))?;
    relayer
        .create_clients("polytone", &ClientOptions::default())
        .await
        .map_err(|err| format!("clients: {err}"))?;
    relayer
        .create_connections("polytone")
        .await
        .map_err(|err| format!("connections: {err}"))?;
    Ok(())
}

async fn fund_address(chain: &CosmosChain, address: &str) -> Result<(), String> {
    chain
        .send_funds(
            "validator",
            &WalletAmount {
                address: address.to_string(),
                denom: "uterp".into(),
                amount: 50_000_000_000,
            },
        )
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn cold_address(secret: &[u8; 32]) -> Result<String, String> {
    let sk = k256::ecdsa::SigningKey::from_slice(secret).map_err(|err| err.to_string())?;
    let pk = sk.verifying_key().to_sec1_point(true);
    let hash = ripemd::Ripemd160::digest(sha2::Sha256::digest(pk.as_bytes()));
    bech32::encode::<bech32::Bech32>(
        bech32::Hrp::parse("terp").map_err(|err| err.to_string())?,
        hash.as_slice(),
    )
    .map_err(|err| err.to_string())
}

fn sign_amino(secret: &[u8; 32], message: &[u8]) -> Result<(String, String), String> {
    let sk = k256::ecdsa::SigningKey::from_slice(secret).map_err(|err| err.to_string())?;
    let pk = sk.verifying_key().to_sec1_point(true);
    let hash = sha2::Sha256::digest(message);
    let sig: k256::ecdsa::Signature = sk
        .sign_prehash(hash.as_slice())
        .map_err(|err| err.to_string())?;
    let sig = sig.normalize_s();
    let pk_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pk.as_bytes());
    let sig_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig.to_bytes());
    Ok((pk_b64, sig_b64))
}

async fn sign_pages(secret: [u8; 32], pages: usize) -> Result<(), String> {
    for index in 0..pages {
        sign_one_page(&secret).await?;
        if index + 1 < pages {
            wait_page_down().await?;
        }
    }
    Ok(())
}

async fn sign_one_page(secret: &[u8; 32]) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let body = loop {
        if tokio::time::Instant::now() > deadline {
            return Err("approval page did not open".into());
        }
        match http_get("/").await {
            Ok(body) if body.contains("id=\"bundles\"") => break body,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    };
    let docs = cold_docs(&body)?;
    if docs.is_empty() {
        return Err("approval page has no cold bundle".into());
    }
    let mut socket = ws_connect().await?;
    let _prompt = socket.read_text().await?;
    for (index, doc) in docs.iter().enumerate() {
        let (pk, sig) = sign_amino(secret, doc)?;
        socket
            .send_text(&format!("check {index} {pk} {sig}"))
            .await?;
        let reply = socket.read_text().await?;
        println!("cw-orch-kms page {reply}");
        if reply.starts_with("refused") {
            return Err(reply);
        }
        if reply.starts_with("authorized") || reply.starts_with("session-ready") {
            return Ok(());
        }
    }
    Err("approval page did not authorize the cold key".into())
}

fn cold_docs(body: &str) -> Result<Vec<Vec<u8>>, String> {
    let marker = "id=\"bundles\">";
    let start = body
        .find(marker)
        .ok_or("approval page has no bundle list")?
        + marker.len();
    let rest = &body[start..];
    let end = rest
        .find("</script>")
        .ok_or("approval page bundle list is truncated")?;
    let bundles: Vec<Value> = serde_json::from_str(&rest[..end])
        .map_err(|err| format!("approval page bundles: {err}"))?;
    let mut docs = Vec::new();
    for bundle in bundles {
        if bundle.get("sign").and_then(|value| value.as_bool()) != Some(true) {
            continue;
        }
        let hex = bundle
            .get("doc")
            .and_then(|value| value.as_str())
            .ok_or("cold bundle has no amino doc")?;
        docs.push(decode_hex(hex)?);
    }
    Ok(docs)
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("amino doc hex is truncated".into());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("amino doc hex is not hex".into()),
    }
}

async fn wait_page_down() -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        if tokio::net::TcpStream::connect(PAGE).await.is_err() {
            return Ok(());
        }
        if tokio::time::Instant::now() > deadline {
            return Err("approval page stayed bound".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn http_get(path: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(PAGE)
        .await
        .map_err(|err| err.to_string())?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {PAGE}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|err| err.to_string())?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|err| err.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    Ok(text.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

struct PageSocket {
    stream: tokio::net::TcpStream,
    buf: Vec<u8>,
}

impl PageSocket {
    async fn read_text(&mut self) -> Result<String, String> {
        loop {
            let (opcode, payload) = self.read_frame().await?;
            match opcode {
                0x1 => {
                    return String::from_utf8(payload).map_err(|err| format!("page frame: {err}"));
                }
                0x8 => return Err("approval page closed the socket".into()),
                0x9 => self.send_frame(0xA, &payload).await?,
                _ => {}
            }
        }
    }

    async fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.send_frame(0x1, text.as_bytes()).await
    }

    async fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        if payload.len() > u16::MAX as usize {
            return Err("page check does not fit in one frame".into());
        }
        let mask = [1u8, 2, 3, 4];
        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.push(0x80 | opcode);
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ mask[i % 4]),
        );
        self.stream
            .write_all(&frame)
            .await
            .map_err(|err| err.to_string())
    }

    async fn read_frame(&mut self) -> Result<(u8, Vec<u8>), String> {
        self.fill(2).await?;
        let opcode = self.buf[0] & 0x0f;
        let masked = self.buf[1] & 0x80 != 0;
        let mut len = (self.buf[1] & 0x7f) as usize;
        let mut header = 2;
        if len == 126 {
            self.fill(4).await?;
            len = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
            header = 4;
        } else if len == 127 {
            return Err("approval page frame is too large".into());
        }
        let mask_len = if masked { 4 } else { 0 };
        self.fill(header + mask_len + len).await?;
        let mask = if masked {
            let mut mask = [0u8; 4];
            mask.copy_from_slice(&self.buf[header..header + 4]);
            Some(mask)
        } else {
            None
        };
        let start = header + mask_len;
        let mut payload = self.buf[start..start + len].to_vec();
        if let Some(mask) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }
        self.buf.drain(..start + len);
        Ok((opcode, payload))
    }

    async fn fill(&mut self, need: usize) -> Result<(), String> {
        use tokio::io::AsyncReadExt;
        while self.buf.len() < need {
            let mut tmp = [0u8; 4096];
            let read = self
                .stream
                .read(&mut tmp)
                .await
                .map_err(|err| err.to_string())?;
            if read == 0 {
                return Err("approval page socket closed".into());
            }
            self.buf.extend_from_slice(&tmp[..read]);
        }
        Ok(())
    }
}

async fn ws_connect() -> Result<PageSocket, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(PAGE)
        .await
        .map_err(|err| err.to_string())?;
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {PAGE}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|err| err.to_string())?;
    let mut buf = Vec::new();
    loop {
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        let mut tmp = [0u8; 1024];
        let read = stream.read(&mut tmp).await.map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("approval page did not accept the socket".into());
        }
        buf.extend_from_slice(&tmp[..read]);
        if buf.len() > 65536 {
            return Err("approval page upgrade did not finish".into());
        }
    }
    let head = String::from_utf8_lossy(&buf);
    if !head.contains("101") {
        return Err(format!("approval page upgrade refused: {head}"));
    }
    let split = buf
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("approval page upgrade has no header end")?
        + 4;
    Ok(PageSocket {
        stream,
        buf: buf[split..].to_vec(),
    })
}

fn b64(value: &Value) -> String {
    Binary::from(value.to_string().into_bytes()).to_base64()
}

fn dao_artifacts() -> PathBuf {
    let dir = std::env::var("DAO_CW4_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../dao-contracts/artifacts")
        });
    for name in DAO {
        let path = dir.join(name);
        if !path.is_file() {
            panic!("missing {} (set DAO_CW4_ARTIFACTS)", path.display());
        }
    }
    dir
}

fn poly_artifacts() -> PathBuf {
    let dir = std::env::var("POLYTONE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../polytone/artifacts")
        });
    for name in POLY {
        let path = dir.join(name);
        if !path.is_file() {
            panic!("missing {} (set POLYTONE_ARTIFACTS)", path.display());
        }
    }
    dir
}

fn terp_config(chain_id: &str) -> ict_rs::chain::ChainConfig {
    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = chain_id.into();
    cfg.gas_prices = "0uterp".into();
    cfg.trusting_period = "10days".into();
    cfg.gas_adjustment = 2.5;
    cfg.block_time = "500ms".into();
    cfg.genesis_style = GenesisStyle::Legacy;
    cfg.config_file_overrides.insert(
        "config/config.toml".into(),
        json!({
            "rpc": { "max_body_bytes": 104_857_600 },
            "mempool": { "max_tx_bytes": 104_857_600 }
        }),
    );
    cfg.modify_genesis = Some(Box::new(|_cfg, raw| {
        let mut genesis: Value = serde_json::from_slice(&raw)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis json: {err}")))?;
        if let Some(block) = genesis.pointer_mut("/consensus_params/block") {
            block["max_gas"] = json!("-1");
            block["max_bytes"] = json!("104857600");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/staking/params") {
            params["unbonding_time"] = json!("2592000s");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/wasm/params") {
            params["code_upload_access"] = json!({"permission": "Everybody", "addresses": []});
            params["instantiate_default_permission"] = json!("Everybody");
        }
        serde_json::to_vec(&genesis)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis encode: {err}")))
    }));
    cfg
}
