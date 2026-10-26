use super::*;
use std::future::IntoFuture;
use std::sync::Mutex as StdMutex;

const CTX: [u8; 32] = [3u8; 32];

fn key_com(n: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = n;
    k
}

fn evm_key(n: u8) -> k256::ecdsa::SigningKey {
    let mut s = [0u8; 32];
    s[31] = n;
    s[0] = 1;
    k256::ecdsa::SigningKey::from_slice(&s).unwrap()
}

fn evm_addr(sk: &k256::ecdsa::SigningKey) -> [u8; 20] {
    let p = sk.verifying_key().to_encoded_point(false);
    keccak256(&p.as_bytes()[1..])[12..].try_into().unwrap()
}

fn evm_sign(sk: &k256::ecdsa::SigningKey, msg: &str) -> [u8; 65] {
    let mut env = format!("\x19Ethereum Signed Message:\n{}", msg.len()).into_bytes();
    env.extend_from_slice(msg.as_bytes());
    let (sig, rec) = sk.sign_prehash_recoverable(&keccak256(&env)).unwrap();
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.to_bytes());
    out[64] = 27 + rec.to_byte();
    out
}

fn sol_key(n: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[n; 32])
}

struct Fixture {
    elig: Elig,
    evm: Vec<k256::ecdsa::SigningKey>,
    sol: Vec<ed25519_dalek::SigningKey>,
}

/// 3 EVM + 2 Solana holders, amount 15 each, depth 3 (padded).
fn fixture() -> Fixture {
    let evm: Vec<_> = (1..=3).map(evm_key).collect();
    let sol: Vec<_> = (1..=2).map(sol_key).collect();
    let mut entries = vec![];
    for k in &evm {
        entries.push(json!({"address_hex": hex::encode(evm_addr(k)), "amount": 15}));
    }
    for k in &sol {
        entries.push(json!({"sol_pubkey_hex": hex::encode(k.verifying_key().to_bytes()), "amount": 15}));
    }
    let tree = json!({"depth": 3, "entries": entries});
    Fixture { elig: Elig::load(tree.to_string().as_bytes()).unwrap(), evm, sol }
}

fn evm_body(sk: &k256::ecdsa::SigningKey, kc: [u8; 32], amount: u64) -> Vec<u8> {
    let sig = evm_sign(sk, &register_message(&CTX, &kc, amount));
    json!({"ctx_hex": hex::encode(CTX), "key_com_hex": hex::encode(kc), "amount": amount.to_string(), "signature_hex": hex::encode(sig)}).to_string().into_bytes()
}

fn sol_body(sk: &ed25519_dalek::SigningKey, kc: [u8; 32], amount: u64) -> Vec<u8> {
    use ed25519_dalek::Signer;
    let sig = sk.sign(register_sol_message(&CTX, &kc, amount).as_bytes());
    json!({"kind": "sol", "ctx_hex": hex::encode(CTX), "key_com_hex": hex::encode(kc), "amount": amount.to_string(),
        "pubkey_hex": hex::encode(sk.verifying_key().to_bytes()), "signature_hex": hex::encode(sig.to_bytes())}).to_string().into_bytes()
}

#[test]
fn leaf_encodings_match_pinned_vectors() {
    // Independent python sha256 vectors (see spec rev 4).
    assert_eq!(hex::encode(sol_leaf(&[1u8; 32], 15)), "e224308c3ab0143008974d281185a18a471fed40e48b331a898f4526e73af711");
    let a: [u8; 20] = hex::decode("7e5f4552091a69125d5dfcb7b8c2659029395bdf").unwrap().try_into().unwrap();
    assert_eq!(hex::encode(evm_leaf(&a, 15)), "45dd29df12c3aacbaeb75cc7696e72bec3fdd4577b28f72c189cced0e64412ed");
    assert_ne!(sol_leaf(&[1u8; 32], 15), [0u8; 32]);
}

#[test]
fn evm_and_sol_verify_against_tree() {
    let f = fixture();
    let r = verify(&parse(&evm_body(&f.evm[1], key_com(9), 15)).ok().unwrap(), &CTX, &f.elig).unwrap();
    assert_eq!(r.identity(), hex::encode(evm_addr(&f.evm[1])));
    assert_eq!(r.path.len(), 3);
    let s = verify(&parse(&sol_body(&f.sol[0], key_com(8), 15)).ok().unwrap(), &CTX, &f.elig).unwrap();
    assert!(s.identity().starts_with("sol:"));
    assert_eq!(s.index, 3);
    assert!(s.execute_msg().get("register_headstash_sol").is_some());
    assert!(r.execute_msg().get("register_headstash").is_some());
}

#[test]
fn rejects_bad_inputs() {
    let f = fixture();
    let bad = |b: &[u8]| match parse(b) {
        Err(Reject::Bad(p)) => p,
        Ok(p) => match verify(&p, &CTX, &f.elig) {
            Err(Reject::Bad(p)) => p,
            _ => "accepted",
        },
        _ => "other",
    };
    // wrong amount -> not eligible
    assert_eq!(bad(&evm_body(&f.evm[0], key_com(1), 16)), "eligibility");
    assert_eq!(bad(&sol_body(&f.sol[0], key_com(1), 16)), "eligibility");
    // non-member
    assert_eq!(bad(&evm_body(&evm_key(77), key_com(1), 15)), "eligibility");
    assert_eq!(bad(&sol_body(&sol_key(77), key_com(1), 15)), "eligibility");
    // tampered sol signature
    let mut v: Value = serde_json::from_slice(&sol_body(&f.sol[0], key_com(1), 15)).unwrap();
    v["key_com_hex"] = json!(hex::encode(key_com(2)));
    assert_eq!(bad(v.to_string().as_bytes()), "signature");
    // wrong ctx
    let mut v: Value = serde_json::from_slice(&evm_body(&f.evm[0], key_com(1), 15)).unwrap();
    v["ctx_hex"] = json!(hex::encode([4u8; 32]));
    assert_eq!(bad(v.to_string().as_bytes()), "registration");
    // formats
    assert_eq!(bad(br#"{"ctx_hex":"aa","key_com_hex":"bb","amount":"15","signature_hex":"ABCD"}"#), "registration");
    let mut v: Value = serde_json::from_slice(&evm_body(&f.evm[0], key_com(1), 15)).unwrap();
    v["amount"] = json!("015");
    assert_eq!(bad(v.to_string().as_bytes()), "amount");
    v["amount"] = json!("15");
    v["kind"] = json!("btc");
    assert_eq!(bad(v.to_string().as_bytes()), "kind");
    v["kind"] = json!("evm");
    v["key_com_hex"] = json!("ff".repeat(32));
    assert_eq!(bad(v.to_string().as_bytes()), "key commitment");
}

#[test]
fn sol_malleated_signature_rejected() {
    use ed25519_dalek::Signer;
    let f = fixture();
    let sk = &f.sol[0];
    let sig = sk.sign(register_sol_message(&CTX, &key_com(1), 15).as_bytes()).to_bytes();
    // S + L (group order) is the same point equation but non-canonical.
    const L: [u8; 32] = [0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10];
    let mut s = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let t = sig[32 + i] as u16 + L[i] as u16 + carry;
        s[i] = t as u8;
        carry = t >> 8;
    }
    let mut m = sig;
    m[32..].copy_from_slice(&s);
    let mut v: Value = serde_json::from_slice(&sol_body(sk, key_com(1), 15)).unwrap();
    v["signature_hex"] = json!(hex::encode(m));
    let p = parse(v.to_string().as_bytes()).ok().unwrap();
    assert!(matches!(verify(&p, &CTX, &f.elig), Err(Reject::Bad("signature"))));
}

#[test]
fn classify_node_errors() {
    assert_eq!(classify("account sequence mismatch, expected 5, got 4: incorrect account sequence"), ChainErr::Sequence);
    match classify("failed to execute message; message index: 2: already registered: execute wasm contract failed") {
        ChainErr::Msg { index: Some(2), .. } => {}
        e => panic!("{e:?}"),
    }
    assert!(matches!(classify("connection refused"), ChainErr::Other(_)));
}

#[test]
fn bind_must_be_loopback() {
    assert!(check_bind("127.0.0.1:8792").is_ok());
    assert!(check_bind("0.0.0.0:8792").is_err());
    assert!(check_bind("10.0.0.2:8792").is_err());
}

#[test]
fn x_real_ip_only_trusted_from_loopback() {
    let mut h = HeaderMap::new();
    h.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));
    assert_eq!(client_ip("127.0.0.1:5000".parse().unwrap(), &h), "203.0.113.9");
    assert_eq!(client_ip("198.51.100.1:5000".parse().unwrap(), &h), "198.51.100.1");
}

// ------------------------------------------------------------ in-process batch test

/// In-memory ledger with contract semantics for the batch path: a msg for an
/// identity already registered fails simulation with its message index, and
/// one broadcast may report a sequence mismatch.
#[derive(Default)]
struct Ledger {
    registered: HashSet<String>,
    seq: u64,
    blocks: Vec<(String, u64, Vec<String>)>,
    simulations: usize,
    stale_once: bool,
}

#[derive(Clone, Default)]
struct TestChain(Arc<StdMutex<Ledger>>);

fn identity_of(msg: &Value) -> String {
    let (k, v) = msg.as_object().unwrap().iter().next().unwrap();
    let d = |s: &str| base64::engine::general_purpose::STANDARD.decode(v[s].as_str().unwrap()).unwrap();
    if k == "register_headstash_sol" {
        format!("sol:{}", hex::encode(d("pubkey")))
    } else {
        // The contract recovers it; tests tag EVM messages by signature.
        format!("evm:{}", hex::encode(&d("signature")[..8]))
    }
}

#[async_trait]
impl Chain for TestChain {
    async fn registered(&self, identity: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().registered.contains(identity))
    }
    async fn sequence(&self) -> Result<(u64, u64), String> {
        Ok((7, self.0.lock().unwrap().seq))
    }
    async fn simulate(&self, msgs: &[Value], _a: u64, seq: u64) -> Result<u64, ChainErr> {
        let mut l = self.0.lock().unwrap();
        l.simulations += 1;
        if seq != l.seq {
            return Err(ChainErr::Sequence);
        }
        for (i, m) in msgs.iter().enumerate() {
            if l.registered.contains(&identity_of(m)) {
                return Err(classify(&format!("message index: {i}: already registered")));
            }
        }
        Ok(100_000 * msgs.len() as u64)
    }
    async fn broadcast(&self, msgs: &[Value], _g: u64, _a: u64, seq: u64) -> Result<String, ChainErr> {
        let mut l = self.0.lock().unwrap();
        if l.stale_once {
            l.stale_once = false;
            l.seq += 1; // someone else used the account
            return Err(ChainErr::Sequence);
        }
        if seq != l.seq {
            return Err(ChainErr::Sequence);
        }
        l.seq += 1;
        let ids: Vec<String> = msgs.iter().map(identity_of).collect();
        for id in &ids {
            l.registered.insert(id.clone());
        }
        let h = format!("TX{}", l.blocks.len());
        let height = 100 + l.blocks.len() as u64;
        l.blocks.push((h.clone(), height, ids));
        Ok(h)
    }
    async fn wait(&self, txhash: &str) -> Result<TxResult, String> {
        let l = self.0.lock().unwrap();
        let b = l.blocks.iter().find(|b| b.0 == txhash).ok_or("missing")?;
        Ok(TxResult { code: 0, height: b.1, log: String::new() })
    }
}

fn test_limits() -> Limits {
    Limits { flush: Duration::from_secs(3600), max_batch: 10, max_queue: 4, ..Limits::default() }
}

#[tokio::test]
async fn http_to_one_batched_tx_with_isolation_and_sequence_refresh() {
    let f = fixture();
    let chain = TestChain::default();
    // An EVM holder that some other sidecar already registered on-chain after
    // we queued it: simulate must drop just that msg.
    let late = evm_sign(&f.evm[2], &register_message(&CTX, &key_com(3), 15));
    let late_id = format!("evm:{}", hex::encode(&late[..8]));
    chain.0.lock().unwrap().stale_once = true;
    let batcher = Arc::new(Batcher::new(chain.clone(), test_limits()));
    let app = Arc::new(App { ctx: CTX, elig: f.elig, batcher: batcher.clone(), origins: vec![DEFAULT_ORIGIN.into()] });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>()).into_future());
    let http = reqwest::Client::new();
    let post = |b: Vec<u8>| {
        let http = http.clone();
        async move {
            let r = http.post(format!("http://{addr}/register")).header("origin", DEFAULT_ORIGIN)
                .header("content-type", "application/json").body(b).send().await.unwrap();
            let code = r.status().as_u16();
            let cors = r.headers().get("access-control-allow-origin").is_some();
            (code, cors, r.json::<Value>().await.unwrap())
        }
    };
    let (c1, cors, e) = post(evm_body(&f.evm[0], key_com(1), 15)).await;
    assert_eq!((c1, cors), (202, true));
    let (c2, _, s) = post(sol_body(&f.sol[0], key_com(2), 15)).await;
    assert_eq!(c2, 202);
    let (c3, _, late_r) = post(evm_body(&f.evm[2], key_com(3), 15)).await;
    assert_eq!(c3, 202);
    // dedupe: same identity again -> same id, not queued twice
    let (c4, _, dup) = post(sol_body(&f.sol[0], key_com(2), 15)).await;
    assert_eq!((c4, dup["id"].clone()), (202, s["id"].clone()));
    assert_eq!(batcher.inner.lock().await.queue.len(), 3);
    // bad request never reaches the queue
    let (c5, _, err) = post(sol_body(&f.sol[1], key_com(4), 99)).await;
    assert_eq!((c5, err["error"].as_str()), (400, Some("eligibility")));
    let (c6, _, _) = post(vec![b'x'; BODY_LIMIT + 1]).await;
    assert_eq!(c6, 400);

    chain.0.lock().unwrap().registered.insert(late_id);
    assert_eq!(batcher.flush_once().await, 3);

    let l = chain.0.lock().unwrap();
    assert_eq!(l.blocks.len(), 1, "one tx");
    assert_eq!(l.blocks[0].2.len(), 2, "two msgs in it");
    assert!(l.blocks[0].2.iter().any(|i| i.starts_with("sol:")));
    assert!(l.blocks[0].2.iter().any(|i| i.starts_with("evm:")));
    assert_eq!(l.seq, 2, "refreshed after mismatch then advanced");
    drop(l);

    for (body, want) in [(&e, "registered"), (&s, "registered"), (&late_r, "failed")] {
        let id = body["id"].as_str().unwrap();
        let st: Value = http.get(format!("http://{addr}/register/status/{id}")).send().await.unwrap().json().await.unwrap();
        assert_eq!(st["status"], want, "{st}");
        if want == "registered" {
            assert_eq!(st["txhash"], "TX0");
            assert_eq!(st["height"], 100);
        } else {
            assert_eq!(st["error"], "already registered");
        }
    }
    // Registered identities answer from chain without queuing.
    let (c7, _, again) = post(sol_body(&f.sol[0], key_com(2), 15)).await;
    assert_eq!((c7, again["status"].as_str()), (200, Some("registered")));
}

#[tokio::test]
async fn queue_cap_and_identity_rate_limit() {
    let f = fixture();
    let batcher = Batcher::new(TestChain::default(), Limits { max_queue: 2, per_identity: (2, Duration::from_secs(60)), ..test_limits() });
    let reg = |b: Vec<u8>| verify(&parse(&b).ok().unwrap(), &CTX, &f.elig).unwrap();
    batcher.submit(reg(evm_body(&f.evm[0], key_com(1), 15))).await.ok().unwrap();
    batcher.submit(reg(evm_body(&f.evm[1], key_com(1), 15))).await.ok().unwrap();
    assert!(matches!(batcher.submit(reg(evm_body(&f.evm[2], key_com(1), 15))).await, Err(Reject::Full)));
    assert!(batcher.rate_identity("x").await.is_ok());
    assert!(batcher.rate_identity("x").await.is_ok());
    assert!(matches!(batcher.rate_identity("x").await, Err(Reject::Rate)));
}
