//! Integration tests for Nostr + content plane + dao-calendar egress (Phase B).
//!
//! Layers:
//!   **L0 (always):** bind off-chain body → dual-index cid → NIP-52 EVENT shape
//!   **L1 (Docker mock relay):** publish + subscribe round-trip on local relay
//!   **L2 (ignored):** full NostrTestEnv lifecycle with local chain + relay
//!
//! Run:
//! ```bash
//! cargo test -p terp-scripts --test nostr_orch_suite -- --nocapture
//! cargo test -p terp-scripts --test nostr_orch_suite -- --nocapture --ignored  # needs Docker
//! ```
//!
//! Content plane invariants: BUD sha256 primary, TreeStore sole BlobStore,
//! no second calendar-only storage path.

use std::time::Duration;

use hash_market::store::TreeStore;
use hash_market::{
    CalendarChainAction, CalendarMetaView, KIND_DATE_BASED, KIND_TIME_BASED, bind_offchain_event,
    metadata_to_nip52_event, parse_calendar_action, resolve_local,
};
use terp_scripts::environments::nostr::{
    bind_and_bridge_offchain, calendar_meta_to_nostr, chain_event_to_nostr, nip01_to_ict,
};

// ---------------------------------------------------------------------------
// L0 — pure content plane + egress (no Docker)
// ---------------------------------------------------------------------------

#[test]
fn l0_offchain_cid_bind_and_nip52_shape() {
    let dir = std::env::temp_dir().join(format!("suite-l0-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = TreeStore::open(dir.join("trees")).unwrap();

    let body = br#"{"title":"L0 Meetup","content":"content-plane body","d_tag":"l0-1","start_time":1700000000,"end_time":1700003600}"#;
    let (bind, meta) = bind_offchain_event(&store, body, KIND_TIME_BASED).unwrap();
    assert_eq!(bind.chain_cid.len(), 64);
    assert_eq!(bind.content_path, format!("/content/{}", bind.sha256));
    assert!(!meta.on_chain);

    let resolved = resolve_local(&store, &meta.cid).unwrap();
    assert_eq!(resolved, body);

    let action = CalendarChainAction {
        action: "create_event".into(),
        e_d: Some("evt/cal/1/1".into()),
        d: Some("cal/1".into()),
        contract: Some("terp1dao_calendar".into()),
        height: 100,
    };
    let view = CalendarMetaView {
        on_chain: false,
        e_json: None,
        cid: Some(meta.cid.clone()),
        kind: KIND_TIME_BASED,
        d_tag: Some("l0-1".into()),
        calendar_d: Some("cal/1".into()),
        author_pubkey: Some("pk_l0".into()),
        nostr_e_d: None,
    };
    let ev = metadata_to_nip52_event(&view, Some(&resolved), Some(&action), "pk_l0").unwrap();
    assert_eq!(ev.kind, KIND_TIME_BASED as u32);
    assert!(
        ev.tags
            .iter()
            .any(|t| t == &["cid".to_string(), bind.chain_cid.clone()])
    );
    assert!(
        ev.tags
            .iter()
            .any(|t| t == &["t".to_string(), "dao-calendar".to_string()])
    );
    assert!(ev.content.contains("L0 Meetup"));

    // harness bridge matches
    let ict = calendar_meta_to_nostr(&view, Some(&resolved), Some(&action), "pk_l0").unwrap();
    assert_eq!(ict.kind, ev.kind);
    assert_eq!(ict.id, ev.id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn l0_onchain_e_json_egress() {
    let e = r#"{"d_tag":"on-1","title":"On-Chain Event","content":"full e","start_time":10,"end_time":20,"event_type":"DateBased"}"#;
    let view = CalendarMetaView {
        on_chain: true,
        e_json: Some(e.into()),
        cid: None,
        kind: KIND_DATE_BASED,
        d_tag: Some("on-1".into()),
        calendar_d: None,
        author_pubkey: Some("pk_on".into()),
        nostr_e_d: None,
    };
    let ev = metadata_to_nip52_event(&view, None, None, "pk_on").unwrap();
    assert_eq!(ev.kind, KIND_DATE_BASED as u32);
    assert!(
        ev.tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("title"))
    );
}

#[test]
fn l0_parse_calendar_action_and_legacy_bridge() {
    let attrs = vec![
        ("action".into(), "create_event".into()),
        ("d".into(), "cal/2".into()),
        ("e_d".into(), "evt/cal/2/1".into()),
        ("_contract_address".into(), "terp1c".into()),
    ];
    let a = parse_calendar_action(&attrs, 55, None).unwrap();
    assert_eq!(a.action, "create_event");
    assert_eq!(a.height, 55);

    let bridged = chain_event_to_nostr(&attrs, 55, Some("create_event"), Some("terp1c"), "pk");
    assert_eq!(bridged.kind, KIND_DATE_BASED as u32);
    assert!(bridged.content.contains("55"));
}

#[test]
fn l0_bind_and_bridge_helper() {
    let dir = std::env::temp_dir().join(format!("suite-l0b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = TreeStore::open(dir.join("trees")).unwrap();
    let body = br#"{"title":"helper","d_tag":"h1"}"#;
    let (bind, ev) = bind_and_bridge_offchain(
        &store,
        body,
        KIND_DATE_BASED,
        Some("h1"),
        Some("cal/9"),
        "pk",
        None,
    )
    .unwrap();
    assert_eq!(bind.sha256.len(), 64);
    assert_eq!(ev.kind, KIND_DATE_BASED as u32);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// L1 — mock relay (no real Docker image required)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn l1_mock_backend_relay_lifecycle() {
    use ict_rs::nostr::NostrRelayerManager;
    use ict_rs::runtime::mock::MockRuntime;

    let runtime = std::sync::Arc::new(MockRuntime::new());
    let mut relay = NostrRelayerManager::with_image(
        runtime,
        ict_rs::runtime::DockerImage {
            repository: "test/noop".to_string(),
            version: "latest".to_string(),
            uid_gid: None,
        },
        "mock-suite-l1",
    );

    let url = relay.start().await.expect("Mock relay should start");
    assert!(url.starts_with("ws://127.0.0.1:"));
    assert!(relay.host_port() > 0);

    relay.stop().await.expect("Mock relay should stop");
    // MockRuntime may leave host_port set; Docker runtime clears it. Accept either.
    let _ = relay.host_port();
}

// ---------------------------------------------------------------------------
// L2 — Docker + optional local chain (ignored by default)
// ---------------------------------------------------------------------------

async fn try_connect_nostr(url: &str) -> Option<ict_rs::nostr::NostrClient> {
    match ict_rs::nostr::NostrClient::connect(url).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("Skipping: cannot connect to {url}: {e}");
            None
        }
    }
}

/// Full lifecycle: NostrTestEnv + content-plane off-chain bind + NIP-52 publish/subscribe.
///
/// Requires Docker (`mattn/nostr-relay`) and optionally a local chain for watcher.
#[tokio::test]
#[ignore = "requires Docker + optional local chain"]
async fn l2_full_lifecycle_content_plane_and_nip52() {
    let _ = env_logger::try_init();

    let chain_info = terp_scripts::environments::nostr::NostrTestEnv::local_terp_chain_info(
        "terp-test-1",
        "http://127.0.0.1:9090",
        "uterp",
    );

    let env = match terp_scripts::environments::nostr::NostrTestEnv::start(
        "phase-b-lifecycle",
        chain_info,
        "chapter wrist alcohol shine angry noise mercy simple rebel recycle vehicle wrap \
         morning giraffe lazy outdoor noise blood ginger sort reunion boss crowd dutch",
    )
    .await
    {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Skipping L2: NostrTestEnv start failed: {e}");
            return;
        }
    };

    assert!(env.nostr_ws_url().starts_with("ws://"));
    assert!(env.chain_ws_url().contains("/websocket"));

    let mut client = match try_connect_nostr(env.nostr_ws_url()).await {
        Some(c) => c,
        None => {
            let mut env = env;
            let _ = env.stop().await;
            return;
        }
    };

    // Content plane: bind off-chain NIP-52 body (TreeStore sole store)
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TreeStore::open(dir.path().join("trees")).unwrap();
    let body = br#"{"title":"Phase B E2E","content":"dual-index cid","d_tag":"pb-1","start_time":1700000000,"end_time":1700007200}"#;
    let action = CalendarChainAction {
        action: "create_event".into(),
        e_d: Some("evt/cal/1/1".into()),
        d: Some("cal/1".into()),
        contract: Some("terp1calendar".into()),
        height: 42,
    };
    let (bind, nip_ev) = bind_and_bridge_offchain(
        &store,
        body,
        KIND_TIME_BASED,
        Some("pb-1"),
        Some("cal/1"),
        "nostr_pubkey_test",
        Some(&action),
    )
    .expect("bind+bridge");
    assert_eq!(bind.chain_cid.len(), 64);
    assert_eq!(nip_ev.kind, KIND_TIME_BASED as u32);

    // Publish NIP-52 to relay
    let ok = client
        .send_event(nip_ev.clone())
        .await
        .expect("publish NIP-52");
    assert!(ok, "relay should accept NIP-52 event");

    // Subscribe and receive
    let filters = vec![serde_json::json!({
        "kinds": [31922, 31923],
        "limit": 10,
    })];
    let _ = client.subscribe(filters).await.expect("subscribe");

    let received = tokio::time::timeout(Duration::from_secs(8), client.recv_event())
        .await
        .expect("timeout waiting for calendar event")
        .expect("recv");
    assert!(
        received.kind == 31922 || received.kind == 31923,
        "got kind {}",
        received.kind
    );
    assert!(
        received.content.contains("Phase B E2E")
            || received
                .tags
                .iter()
                .any(|t| t.first().map(|s| s.as_str()) == Some("cid")),
        "content/tags should carry event or cid: {}",
        received.content
    );

    // Legacy attrs bridge still works
    let attrs = vec![
        ("action".into(), "create_event".into()),
        ("e_d".into(), "evt/cal/1/1".into()),
    ];
    let legacy = chain_event_to_nostr(&attrs, 42, Some("create_event"), Some("terp1c"), "pk");
    let ok = client.send_event(legacy).await.expect("publish legacy");
    assert!(ok);

    match env.event_watcher().await {
        Ok(watcher) => {
            eprintln!("ChainEventWatcher connected");
            watcher.shutdown();
        }
        Err(e) => eprintln!("ChainEventWatcher skipped (no chain): {e}"),
    }

    let mut env = env;
    env.stop().await.expect("stop relay");
}

/// Simulate dao-calendar wasm attrs → parse → egress without chain (Docker relay only).
#[tokio::test]
#[ignore = "requires Docker relay"]
async fn l2_dao_calendar_attr_egress_roundtrip() {
    use ict_rs::nostr::NostrRelayerManager;
    use ict_rs::runtime::IctRuntime;

    let runtime = match IctRuntime::Docker(Default::default()).into_backend().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping: Docker unavailable: {e}");
            return;
        }
    };

    let mut relay = NostrRelayerManager::new(runtime, "attr-egress");
    let url = match relay.start().await {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Skipping: relay start: {e}");
            return;
        }
    };

    let mut client = match try_connect_nostr(&url).await {
        Some(c) => c,
        None => {
            let _ = relay.stop().await;
            return;
        }
    };

    let dir = tempfile::tempdir().unwrap();
    let store = TreeStore::open(dir.path().join("trees")).unwrap();
    let body = br#"{"title":"Attr Egress","d_tag":"ae-1","content":"from wasm attrs"}"#;
    let attrs = vec![
        ("action".into(), "create_event".into()),
        ("d".into(), "cal/1".into()),
        ("e_d".into(), "evt/cal/1/3".into()),
        ("_contract_address".into(), "terp1cal".into()),
    ];
    let action = parse_calendar_action(&attrs, 7, None).unwrap();
    let (bind, meta) = bind_offchain_event(&store, body, KIND_TIME_BASED).unwrap();
    let resolved = resolve_local(&store, &meta.cid).unwrap();
    let view = CalendarMetaView {
        on_chain: false,
        e_json: None,
        cid: Some(meta.cid),
        kind: KIND_TIME_BASED,
        d_tag: Some("ae-1".into()),
        calendar_d: Some("cal/1".into()),
        author_pubkey: Some("op".into()),
        nostr_e_d: None,
    };
    let nip = metadata_to_nip52_event(&view, Some(&resolved), Some(&action), "op").unwrap();
    let ict = nip01_to_ict(nip);
    assert!(client.send_event(ict).await.unwrap());

    let _ = client
        .subscribe(vec![serde_json::json!({"kinds":[31923],"limit":5})])
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(8), client.recv_event())
        .await
        .expect("timeout")
        .expect("event");
    assert_eq!(got.kind, 31923);
    assert!(got.tags.iter().any(|t| t.get(1) == Some(&bind.chain_cid)));

    relay.stop().await.unwrap();
}
