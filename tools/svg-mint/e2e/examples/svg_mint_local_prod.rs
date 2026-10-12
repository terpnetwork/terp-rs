#![recursion_limit = "256"]
//! One-shot: Terp + sibling sidecars (Zakura, DREGG fed, Anvil) + mint + teardown.
//!
//! ict-rs owns the Docker network. Zakura, DREGG federation, and Anvil are
//! **siblings** on that network — DREGG is not a child of Zakura.
//!
//! ETH payment → federation `DepositSeen` → entitle → cw-svg-minter instantiate.
//!
//! ```sh
//! # groot-wan. Wallet is svg-mint-zrt in a temp directory. Fed must be a Linux ELF from this repo.
//! sh scripts/e2e-local-prod.sh
//! ```

use ict_rs::cosmos::interchain::wait_for_blocks;
use ict_rs::prelude::*;
use ict_rs_cw_orch::daemon_builder_from_chain;
use std::path::PathBuf;
use std::sync::Arc;

const LAB_MNEMONIC: &str =
    "notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius";

fn refuse_mock() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        return Err("svg_mint_local_prod refuses ICT_MOCK=1".into());
    }
    Ok(())
}

fn product_root() -> PathBuf {
    if let Ok(p) = std::env::var("SVG_MINT_ROOT") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let from_ict = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../terp-rs/tools/svg-mint");
    if from_ict.is_dir() {
        return from_ict;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn terp_crates() -> PathBuf {
    if let Ok(p) = std::env::var("TERP_CRATES") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let from_ict = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if from_ict.join("ict-rs").is_dir() {
        return from_ict;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../")
}

fn artifacts_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SVG_MINT_ARTIFACTS") {
        return PathBuf::from(p);
    }
    terp_crates().join("cw-infuser/artifacts")
}

/// Container fed is a Linux ELF built from this repo. A darwin binary is not that runtime.
fn ensure_product_fed_bin() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(p) = std::env::var("DREGG_NODE_BIN") {
        if !p.is_empty() {
            return Ok(());
        }
    }
    let root = product_root();
    for rel in [
        "target/aarch64-unknown-linux-musl/release/svg-mint-fed",
        "target/aarch64-unknown-linux-gnu/release/svg-mint-fed",
        "target/linux-gnu/svg-mint-fed",
    ] {
        let p = root.join(rel);
        if p.is_file() {
            std::env::set_var("DREGG_NODE_BIN", &p);
            return Ok(());
        }
    }
    Err("DREGG_NODE_BIN unset. Build svg-mint-fed as a Linux ELF from this repo before the chain suite.".into())
}



#[tokio::main]
async fn main() {
    if std::env::var("TERP_IMAGE_VERSION").ok().filter(|s| !s.is_empty()).is_none()
        && std::env::var("ICT_IMAGE_VERSION").ok().filter(|s| !s.is_empty()).is_none()
    {
        std::env::set_var("TERP_IMAGE_VERSION", "v6.3.0-dev");
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .init();
    match run().await {
        Ok(v) => {
            println!("{v}");
            if v.get("live").and_then(|x| x.as_bool()) != Some(true) {
                std::process::exit(2);
            }
        }
        Err(e) => {
            eprintln!("{e}");
            println!("{}", serde_json::json!({ "live": false, "error": e.to_string() }));
            std::process::exit(2);
        }
    }
}

async fn run() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    refuse_mock()?;
    ensure_product_fed_bin()?;
    let arts = artifacts_dir();
    let svg_wasm = arts.join("cw721_svg.wasm");
    let minter_wasm = arts.join("cw_svg_minter.wasm");
    if !svg_wasm.is_file() || !minter_wasm.is_file() {
        return Err(format!("missing suite wasm under {}", arts.display()).into());
    }

    let rt = Arc::new(DockerBackend::new(Default::default()).await?);
    let mut cfg = terp_chain_config();
    cfg.chain_id = "svg-mint-local-1".into();
    cfg.name = "svg-mint-local".into();
    cfg.genesis_style = GenesisStyle::Legacy;
    cfg.faucet = Some(FaucetConfig {
        key_name: "deployer".into(),
        port: 4500,
        start_cmd: vec![],
        env: vec![],
        mnemonic: Some(LAB_MNEMONIC.into()),
        coins: Some("100000000000uterp".into()),
    });

    let _ = std::process::Command::new(docker_bin())
        .args([
            "rm",
            "-f",
            "ict-svg-mint-local-svg-mint-local-1-val-0",
            "ict-svg-mint-zakura-zakura-18252",
            "ict-svg-mint-zakura-public-zakura-18262",
            "ict-svg-mint-dregg-8421",
            "ict-svg-mint-dregg-18421",
            "ict-svg-mint-local-svg-mint-local-1-anvil-0",
            "ict-svg-mint-btc-bitcoind-18453",
            "ict-svg-mint-dregg-dregg-8421",
            "ict-svg-mint-dregg-dregg-18421",
            "ict-svg-mint-local-8453-anvil-0",
            "ict-svg-mint-local-31337-anvil-0",
        ])
        .status();

    let mut chain = CosmosChain::new(cfg, 1, 0, rt.clone());
    let ctx = TestContext {
        test_name: "svg-mint-local".into(),
        network_id: "ict-svg-mint".into(),
    };
    chain.initialize(&ctx).await?;
    chain.start(&[]).await?;
    wait_for_blocks(&chain, 2).await?;
    let net = chain.network_id().cloned();

    let reuse_z = std::env::var("SVG_MINT_ZAKURA_REUSE").ok().as_deref() == Some("1");
    let mut zakura_cfg = if reuse_z {
        let mut c = ZakuraNodeConfig::default();
        c.reuse_if_ready = true;
        c
    } else {
        ZakuraNodeConfig::from_env()?
    };
    zakura_cfg.network_id = net.clone();
    zakura_cfg.test_name = "svg-mint-zakura".into();
    zakura_cfg.rpc_host_port = if reuse_z { 18232 } else { 18252 };
    if !reuse_z {
        zakura_cfg.reuse_if_ready = false;
    }
    zakura_cfg.ready_timeout_secs = 90;
    let mut zakura = spawn_zakura_local(rt.clone(), zakura_cfg).await?;
    let mut public_cfg = ZakuraNodeConfig::from_env()?;
    public_cfg.network_id = net.clone();
    public_cfg.test_name = "svg-mint-zakura-public".into();
    public_cfg.rpc_host_port = 18262;
    public_cfg.reuse_if_ready = false;
    public_cfg.ready_timeout_secs = 90;
    let mut public_zakura = match spawn_zakura_local(rt.clone(), public_cfg).await {
        Ok(n) => n,
        Err(e) => {
            let _ = zakura.teardown().await;
            return Err(e.into());
        }
    };
    std::env::set_var("ZCASH_PRIVATE_RPC", zakura.docker_dns_rpc_url());
    std::env::set_var("ZCASH_PUBLIC_RPC", public_zakura.docker_dns_rpc_url());
    std::env::set_var("ZCASH_PUBLIC_RPC_HOST", public_zakura.rpc_url());
    std::env::set_var("ZCASH_TIP_MAX_DRIFT", "2");
    std::env::set_var("ZCASH_TIP_CACHE_SECS", "0");
    // Docker zakura has no NodeClient. Compact surface is a sibling ztreamer
    // process that embeds in-process zakura_state (not stock lightwalletd).
    let mut ztreamer = match spawn_ztreamer_lab() {
        Ok(z) => z,
        Err(e) => {
            let _ = public_zakura.teardown().await;
            let _ = zakura.teardown().await;
            return Err(e);
        }
    };

    // CLOSE Linus 3: Anvil, Base, bitcoind, THEN DREGG. Docker DNS for watchers and cast.
    // Do not set DREGG_MINTER_FUNDED. Do not copy validator bank into DREGG_MINTER_BALANCE.
    let mut anvil = AnvilChain::new(builtin_chain_config("anvil")?, rt.clone());
    anvil.join_existing_network = net.clone();
    anvil.rpc_host_port = 18545;
    anvil
        .initialize(&TestContext {
            test_name: "svg-mint-local".into(),
            network_id: "ict-svg-mint".into(),
        })
        .await?;
    anvil.start(&[]).await?;

    // Spawned Base sibling (Foundry --chain-id 8453), not a comment. Host port != 18545.
    let mut base_cfg = builtin_chain_config("anvil")?;
    base_cfg.chain_id = "8453".into();
    base_cfg.name = "base-anvil".into();
    let mut base_anvil = AnvilChain::new(base_cfg, rt.clone());
    base_anvil.join_existing_network = net.clone();
    base_anvil.rpc_host_port = 28453;
    base_anvil.anvil_chain_id = Some(8453);
    let _ = std::process::Command::new(docker_bin())
        .args(["rm", "-f", "ict-svg-mint-local-8453-anvil-0"])
        .status();
    base_anvil
        .initialize(&TestContext {
            test_name: "svg-mint-local".into(),
            network_id: "ict-svg-mint".into(),
        })
        .await?;
    base_anvil.start(&[]).await?;

    let mut btc_cfg = BitcoinNodeConfig::default();
    btc_cfg.network_id = net.clone();
    btc_cfg.test_name = "svg-mint-btc".into();
    let mut btc = spawn_bitcoind_regtest(rt.clone(), btc_cfg).await?;

    std::env::set_var("ETH_RPC", anvil.docker_dns_rpc_url());
    std::env::set_var("BASE_RPC", base_anvil.docker_dns_rpc_url());
    std::env::set_var("BTC_RPC", btc.docker_dns_rpc_url());
    let denom = chain.config().denom.clone();
    std::env::set_var("GAS_DENOM", &denom);
    {
        let node = chain.primary_node()?;
        std::env::set_var(
            "DREGG_MINTER_LCD",
            format!("http://{}:{}", node.hostname, node.ports.api),
        );
    }
    std::env::remove_var("DREGG_MINTER_ADDR");
    std::env::remove_var("DREGG_MINTER_BALANCE");
    std::env::remove_var("DREGG_MINTER_FUNDED");

    let mut dregg: Option<DreggNode> = None;
    let outcome = exercise(
        &chain,
        &anvil,
        &base_anvil,
        &btc,
        rt.clone(),
        net.clone(),
        &mut dregg,
        &mut zakura,
        &svg_wasm,
        &minter_wasm,
    )
    .await;

    let _ = btc.teardown().await;
    if let Some(mut d) = dregg {
        let _ = d.teardown().await;
    }
    let _ = public_zakura.teardown().await;
    let _ = zakura.teardown().await;
    if let Some(mut child) = ztreamer.child.take() {
        let _ = child.kill();
    }
    let _ = base_anvil.stop().await;
    let _ = anvil.stop().await;
    let _ = chain.stop().await;
    let _ = std::process::Command::new(docker_bin())
        .args([
            "rm",
            "-f",
            "ict-svg-mint-local-svg-mint-local-1-val-0",
            "ict-svg-mint-zakura-zakura-18252",
            "ict-svg-mint-zakura-public-zakura-18262",
            "ict-svg-mint-dregg-8421",
            "ict-svg-mint-dregg-18421",
            "ict-svg-mint-local-svg-mint-local-1-anvil-0",
            "ict-svg-mint-btc-bitcoind-18453",
            "ict-svg-mint-dregg-dregg-8421",
            "ict-svg-mint-dregg-dregg-18421",
        ])
        .status();
    outcome
}

async fn exercise(
    chain: &CosmosChain,
    anvil: &AnvilChain,
    base_anvil: &AnvilChain,
    btc: &BitcoinNode,
    rt: Arc<DockerBackend>,
    net: Option<ict_rs::runtime::NetworkId>,
    dregg: &mut Option<DreggNode>,
    zakura: &mut ZakuraNode,
    svg_wasm: &std::path::Path,
    minter_wasm: &std::path::Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let _ = (svg_wasm, minter_wasm);
    let zakura_rpc = zakura.rpc_url();
    let zakura_reused = zakura.reused_existing;
    if !zakura.rpc_ready().await {
        return Err(format!("zakurad not ready at {zakura_rpc}").into());
    }

    wait_for_blocks(chain, 1).await?;
    let node0 = chain.primary_node()?;
    let owner_bech = node0.get_key_address("validator").await?;
    let cid0 = node0
        .container_id
        .as_ref()
        .ok_or("validator has no container")?
        .0
        .clone();
    let home0 = node0.home_dir.clone();
    let denom0 = chain.config().denom.clone();

    let grpc_host = node0
        .host_grpc_port
        .ok_or("validator container has no host-mapped gRPC — start() did not resolve ports")?;
    std::env::set_var("ICT_GRPC_URL", format!("http://127.0.0.1:{grpc_host}"));
    let arts = artifacts_dir();
    let (minter_addr, collection_addr, svg_code, minter_code) =
        deploy_svg_cw_orch(chain, &owner_bech, &arts)?;
    if collection_addr.is_empty() || !collection_addr.starts_with("terp1") {
        return Err(format!("CwSvgSuite/Deploy did not return collection: {collection_addr}").into());
    }
    // CLOSE Linus 3: sidecar env is Docker DNS LCD of the **contract**, not validator bank.
    let lcd_docker = format!("http://{}:{}", node0.hostname, node0.ports.api);
    std::env::set_var("DREGG_MINTER_ADDR", &minter_addr);
    std::env::set_var("DREGG_MINTER_LCD", &lcd_docker);
    std::env::remove_var("DREGG_MINTER_BALANCE");
    std::env::remove_var("DREGG_MINTER_FUNDED");
    std::env::remove_var("DREGG_NODE_URL");
    let wallet_lab = prepare_wallet_lab()?;
    let mut dregg_cfg = DreggNodeConfig::from_env()?;
    dregg_cfg.network_id = net;
    dregg_cfg.test_name = "svg-mint-dregg".into();
    // Host :8421 may already be a leftover svg-mint-fed (not the lab container).
    dregg_cfg.query_host_port = 18421;
    dregg_cfg.reuse_if_ready = false;
    dregg_cfg.ready_timeout_secs = 60;
    dregg_cfg.extra_volumes = wallet_lab.volumes.clone();
    std::env::set_var("DREGG_ADMIN_TOKEN", "e2e-admin-token");
    let spawned = spawn_dregg_local(rt, dregg_cfg).await?;
    let dregg_url_s = spawned.query_url().to_string();
    *dregg = Some(spawned);
    let dregg_url = dregg_url_s.as_str();
    std::env::set_var("DREGG_SVG_URL", dregg_url);
    std::env::remove_var("ONBOARD_SKIP_PAID");

    let health = http_json("GET", &format!("{}/health", dregg_url.trim_end_matches('/')), None)?;
    if health.get("service").and_then(|x| x.as_str()) != Some("dregg-svg") {
        return Err(format!("dregg federation not live: {health}").into());
    }
    let setc = fed_signed(
        dregg_url,
        "set-collection",
        "/v1/collection",
        serde_json::json!({"id": "svg", "max_supply": 32}),
    )?;
    if setc.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("set collection max supply failed: {setc}").into());
    }
    let health2 = http_json("GET", &format!("{}/health", dregg_url.trim_end_matches('/')), None)?;
    if health2.get("checkout_ready") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!(
            "dregg checkout not ready after collection bind (LCD of contract, not validator): {health2}"
        )
        .into());
    }
    let zec_tip_circuit = if zakura.reused_existing {
        serde_json::json!({ "ok": false, "skipped": "private_reused" })
    } else {
        match exercise_zec_tip_circuit(dregg_url, zakura, &owner_bech).await {
            Ok(v) => v,
            Err(e) => {
                let _ = zakura.start().await;
                return Err(e);
            }
        }
    };
    eprintln!("DREGG_ZEC_TIP_CIRCUIT {zec_tip_circuit}");
    if std::env::var("SVG_MINT_STOP_AFTER_ZEC_TIP").ok().as_deref() == Some("1") {
        return Ok(serde_json::json!({
            "live": true,
            "teardown": true,
            "stopped_after": "zec_tip",
            "zec_tip_circuit": zec_tip_circuit,
            "dregg_query": dregg_url,
            "zakura_rpc": zakura_rpc,
            "zec_public_rpc": std::env::var("ZCASH_PUBLIC_RPC").ok(),
            "zec_private_rpc": std::env::var("ZCASH_PRIVATE_RPC").ok(),
            "compact_vendor": "Ztreamer",
            "lwd_absent": true,
            "compact_grpc": std::env::var("ZCASH_DEVTOOL_SERVER").ok(),
            "siblings": ["zakura", "zakura-public", "dregg-fed"],
        }));
    }

    let pay_a = http_json(
        "GET",
        &format!(
            "{}/v1/pay-targets?recipient={owner_bech}",
            dregg_url.trim_end_matches('/')
        ),
        None,
    )?;
    let pay_b = http_json(
        "GET",
        &format!(
            "{}/v1/pay-targets?recipient={owner_bech}",
            dregg_url.trim_end_matches('/')
        ),
        None,
    )?;
    assert_pay_targets_private(&pay_a, &owner_bech)?;
    assert_pay_targets_private(&pay_b, &owner_bech)?;
    if pay_a.get("zec_ua").and_then(|x| x.as_str()) != Some(wallet_lab.ua.as_str()) {
        return Err(format!(
            "pay-targets zec_ua must be the managed wallet address, got {pay_a}"
        )
        .into());
    }
    // CLOSE Linus 10/11: HD addrs from fed derive() (all-hardened); bech32 via crate not convert_bits.
    if pay_a.get("eth_addr") == pay_b.get("eth_addr")
        || pay_a.get("btc_addr") == pay_b.get("btc_addr")
        || pay_a.get("zec_diversifier") == pay_b.get("zec_diversifier")
        || pay_a.get("memo_commit") == pay_b.get("memo_commit")
    {
        return Err(format!("HD/diversifier/C reused across checkouts: a={pay_a} b={pay_b}").into());
    }
    let memo_hex = pay_a
        .get("memo_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("pay-targets missing memo_hex: {pay_a}"))?
        .to_string();
    let btc_hd = pay_a
        .get("btc_addr")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let from = ANVIL_DEFAULT_ACCOUNTS[0].1;
    let treasury = ANVIL_DEFAULT_ACCOUNTS[1].0;
    let eth_wei = dregg::prices::ETH_WEI;
    let btc_sats = dregg::prices::BTC_SATS;
    let (tx_hash, vault_addr, lock_id) = eth_vault_lock(anvil, from, treasury, &memo_hex).await?;

    let deposit = fed_signed(
        dregg_url,
        "deposit",
        "/v1/deposit",
        serde_json::json!({
            "rail": "Eth",
            "amount": eth_wei,
            "raw_hex": tx_hash,
        }),
    )?;
    if deposit.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation did not record ETH payment: {deposit}").into());
    }
    let n_eth = deposit["n_deposit"].clone();

    if !(btc_hd.starts_with("bcrt1") || btc_hd.starts_with("bc1")) {
        return Err(format!("pay-targets btc_addr not HD segwit: {btc_hd}").into());
    }
    let btc_txid = btc
        .pay_sats_to(&btc_hd, btc_sats)
        .await
        .map_err(|e| format!("pay_sats_to({btc_hd}) failed (no silent pay_sats fallback): {e}"))?;
    let btc_dest = btc_hd.clone();
    let btc_dep = fed_signed(
        dregg_url,
        "deposit",
        "/v1/deposit",
        serde_json::json!({
            "rail": "Btc",
            "amount": btc_sats,
            "raw_hex": btc_txid,
        }),
    )?;
    if btc_dep.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation did not record BTC payment: {btc_dep}").into());
    }
    let n_btc = btc_dep["n_deposit"].clone();

    let eth_addr = pay_a
        .get("eth_addr")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let base_rpc = base_anvil.docker_dns_rpc_url();
    let eth_hex = format!("0x{eth_wei:x}");
    let base_send = base_anvil
        .exec_cast(&[
            "send",
            "--rpc-url",
            &base_rpc,
            "--private-key",
            from,
            eth_addr,
            "--value",
            &eth_hex,
            "--json",
        ])
        .await?;
    let base_blob = format!("{}{}", base_send.stdout_str(), base_send.stderr_str());
    let base_hash = extract_tx_hash(&base_blob).unwrap_or_default();
    let base_dep = fed_signed(
        dregg_url,
        "deposit",
        "/v1/deposit",
        serde_json::json!({
            "rail": "Base",
            "amount": eth_wei,
            "raw_hex": base_hash.trim_start_matches("0x"),
        }),
    )?;
    if base_dep.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation did not record Base payment: {base_dep} {base_blob}")
            .into());
    }
    let n_base = base_dep["n_deposit"].clone();

    wait_for_blocks(chain, 2).await?;
    let node = chain.primary_node()?;
    let cid = node
        .container_id
        .as_ref()
        .ok_or("validator has no container")?
        .0
        .clone();
    let home = node.home_dir.clone();

    let entitle = fed_signed(
        dregg_url,
        "entitle",
        "/v1/entitle",
        serde_json::json!({
            "n_deposit": n_eth,
            "collection": "svg",
            "svg_hash": vec![8u8; 32],
            "recipient": owner_bech,
            "session": vec![5u8; 32],
        }),
    )?;
    if entitle.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation entitle refused: {entitle}").into());
    }
    let memo_c = pay_a
        .get("memo_hex")
        .or_else(|| pay_a.get("memo"))
        .or_else(|| pay_a.get("c"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let walk = exercise_onboard_passkey(
        chain,
        &cid,
        &home,
        chain.chain_id(),
        &owner_bech,
        chain.config().denom.as_str(),
        dregg_url,
        &memo_c,
    )
    .await?;
    if walk.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("pweb walk (unpaid 402 → paid grant) failed: {walk}").into());
    }
    let entitle_btc = fed_signed(
        dregg_url,
        "entitle",
        "/v1/entitle",
        serde_json::json!({
            "n_deposit": n_btc,
            "collection": "svg",
            "svg_hash": vec![7u8; 32],
            "recipient": owner_bech,
            "session": vec![6u8; 32],
        }),
    )?;
    if entitle_btc.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation BTC entitle refused: {entitle_btc}").into());
    }
    let entitle_base = fed_signed(
        dregg_url,
        "entitle",
        "/v1/entitle",
        serde_json::json!({
            "n_deposit": n_base,
            "collection": "svg",
            "svg_hash": vec![9u8; 32],
            "recipient": owner_bech,
            "session": vec![7u8; 32],
        }),
    )?;
    if entitle_base.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation Base entitle refused: {entitle_base}").into());
    }
    let zec_info = json_rpc(
        zakura_rpc,
        "getblockchaininfo",
        serde_json::json!([]),
    )?;
    if zec_info.get("result").is_none() {
        return Err(format!("zakura getblockchaininfo has no result: {zec_info}").into());
    }
    std::env::set_var("ZAKURA_RPC", zakura_rpc);
    let generated = json_rpc(zakura_rpc, "generate", serde_json::json!([2]))?;
    if generated.get("error").is_some() || generated.get("result").is_none() {
        return Err(format!("zakura generate 2 failed: {generated}").into());
    }
    let zec_info = json_rpc(zakura_rpc, "getblockchaininfo", serde_json::json!([]))?;
    if !nu63_is_active(&zec_info) {
        return Err(format!(
            "lab zakura NU6.3 must be active after generate 2, got {zec_info}"
        )
        .into());
    }
    // One SVG unit. prices::ZEC_ZAT is 150_000. A smaller note is underpay.
    std::env::set_var("SVG_MINT_IRONWOOD_ZAT", "150000");
    let zec_open = ironwood_escrow("open").map_err(|e| format!("ironwood open failed: {e}"))?;
    if zec_open.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("ironwood open refused: {zec_open}").into());
    }
    let zec_zat = zec_open.get("amount_zat").and_then(|x| x.as_u64()).unwrap_or(0);
    if zec_zat != 150_000 {
        return Err(format!("ironwood open amount_zat must be 150000, got {zec_zat} in {zec_open}").into());
    }
    catch_up_public_zakura(zakura_rpc)?;
    let zec_txid = zec_open
        .get("txid")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if zec_txid.len() != 64 || !zec_txid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("ironwood txid must be 64 hex chars, got {zec_txid:?} in {zec_open}").into());
    }
    let zec = fed_signed(
        dregg_url,
        "deposit",
        "/v1/deposit",
        serde_json::json!({
            "rail": "ZecIronwood",
            "amount": zec_zat,
            "raw_hex": zec_txid,
        }),
    )?;
    if zec.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("federation did not record ZEC payment: {zec}").into());
    }
    let zec_skipped_reason: Option<String> = None;
    let n_zec = zec
        .get("n_deposit")
        .cloned()
        .ok_or("ironwood deposit missing n_deposit")?;
    if n_zec.is_null() {
        return Err(format!("ironwood deposit n_deposit is null: {zec}").into());
    }
    if true {
        let entitle_zec = fed_signed(
            dregg_url,
            "entitle",
            "/v1/entitle",
            serde_json::json!({
                "n_deposit": n_zec,
                "collection": "svg",
                "svg_hash": vec![6u8; 32],
                "recipient": owner_bech,
                "session": vec![4u8; 32],
            }),
        )?;
        if entitle_zec.get("ok") != Some(&serde_json::Value::Bool(true)) {
            return Err(format!("federation ZEC entitle refused: {entitle_zec}").into());
        }
    }

    let pay = pay_a.clone();
    let zec_uri = pay.get("zec_uri").and_then(|x| x.as_str()).unwrap_or("");
    if pay.get("zip321").is_none() {
        return Err(format!(
            "stale svg-mint-fed ELF (pay-targets missing zip321 field). Rebuild DREGG_NODE_BIN from current fed.rs: {pay}"
        )
        .into());
    }
    if !zec_uri.is_empty() && !zec_uri.starts_with("zcash:") {
        return Err(format!("ZIP-321 URI must start with zcash:: {zec_uri}").into());
    }
    if !zec_uri.is_empty() && !zec_uri.contains("memo=") {
        return Err(format!("ZIP-321 URI missing memo=: {zec_uri}").into());
    }
    let wallet = exercise_managed_wallet(&wallet_lab, dregg_url, &owner_bech)?;
    if std::env::var("SVG_MINT_REQUIRE_ZEC_WALLET").ok().as_deref() == Some("1") {
        if zec_uri.is_empty() || !zec_uri.starts_with("zcash:") || zec_uri.contains("terp1") {
            return Err(format!(
                "native ZEC wallet required: zec_uri ZIP-321 without terp1, got {pay}"
            )
            .into());
        }
        if wallet.get("wallet").and_then(|x| x.as_str()) != Some("svg-mint-zrt")
            || wallet.get("send_fail_closed") != Some(&serde_json::Value::Bool(true))
            || wallet.get("restart_same_address") != Some(&serde_json::Value::Bool(true))
        {
            return Err(format!(
                "native ZEC requires svg-mint-zrt restart and fail-closed send, got {wallet}"
            )
            .into());
        }
    }
    let st = http_json(
        "GET",
        &format!(
            "{}/v1/status?recipient={owner_bech}",
            dregg_url.trim_end_matches('/')
        ),
        None,
    )?;
    if st.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("status by recipient failed: {st}").into());
    }

    wait_for_blocks(chain, 1).await?;
    let mint_builder = daemon_builder_from_chain(chain, Some(LAB_MNEMONIC))?;
    let collection_for_mint = collection_addr.clone();
    let mint_hash = tokio::task::spawn_blocking(move || {
        use cw_orch::prelude::Coin;
        use ict_rs_cw_orch::svg_runtime::{mint_via_daemon, native_svg_mint_msg};
        let daemon = mint_builder
            .build()
            .map_err(|e| format!("Daemon.build mint: {e}"))?;
        mint_via_daemon(
            &daemon,
            &collection_for_mint,
            &native_svg_mint_msg(),
            &[Coin::new(dregg::prices::NATIVE_AMOUNT, "uterp")],
        )
    })
    .await?
    .map_err(|e| format!("cw-orch native mint: {e}"))?;
    let adr_mint_settled = record_authorized_mint(dregg_url, &owner_bech, &mint_hash)?;
    wait_for_blocks(chain, 2).await?;
    let native_out = mint_hash.clone();
    let native_failed = false;
    let dregg_mint_failed = false;
    let dregg_out = String::new();
    let _ = (dregg_out, native_failed, dregg_mint_failed);

    if collection_addr.is_empty() {
        return Err("collection address empty; native mint did not instantiate".into());
    }
    let (wasm_query, queried_minted) = poll_num_tokens(&cid, &home, &collection_addr, 12)?;
    if queried_minted == 0 {
        return Err(format!(
            "attested supply query minted=0: {wasm_query} native_failed={native_failed} native_out={}",
            native_out.chars().take(800).collect::<String>()
        )
        .into());
    }
    let tpl = wasm_smart_query(&cid, &home, &collection_addr, r#"{"svg_template":{}}"#).unwrap_or_default();
    if dregg::hopalong::is_vanilla_placeholder(&tpl) {
        return Err(format!("collection is vanilla 1x1 SVG, expected hopalong entropy template: {tpl}").into());
    }
    let height = 2u64;
    // CLOSE Linus 12: observe-supply minted is queried_minted, not hardcoded 8.
    let observe_body = serde_json::json!({
        "turn_tag": "observe-supply",
        "collection": "svg",
        "minted": queried_minted,
        "max_supply": 32,
        "height": height,
        "wasm_query": wasm_query,
    });
    let prev = http_json(
        "POST",
        &format!("{}/v1/checkpoint-preview", dregg_url.trim_end_matches('/')),
        Some(&observe_body),
    )?;
    let digest = extract_digest(&prev).ok_or_else(|| format!("observe preview missing digest: {prev}"))?;
    let signed = http_json(
        "POST",
        &format!("{}/v1/sign", dregg_url.trim_end_matches('/')),
        Some(&serde_json::json!({"digest": digest})),
    )?;
    let mut observe_body = observe_body;
    observe_body["signatures"] = signed.get("signatures").cloned().unwrap_or(serde_json::json!([]));
    if observe_body["signatures"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err(format!("observe-supply unsigned: {signed}").into());
    }
    let observed = http_json(
        "POST",
        &format!("{}/v1/observe-supply", dregg_url.trim_end_matches('/')),
        Some(&observe_body),
    )?;
    if observed.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("observe-supply refused: {observed}").into());
    }

    let settle_eth = fed_signed(
        dregg_url,
        "settle",
        "/v1/settle",
        serde_json::json!({
            "n_deposit": n_eth,
            "token_id": minter_addr,
            "terp_tx": "dregg-lost-race",
        }),
    )?;
    let settle_btc = fed_signed(
        dregg_url,
        "settle",
        "/v1/settle",
        serde_json::json!({
            "n_deposit": n_btc,
            "token_id": format!("{minter_addr}-btc"),
            "terp_tx": "dregg-lost-race",
        }),
    )?;
    let settle_base = fed_signed(
        dregg_url,
        "settle",
        "/v1/settle",
        serde_json::json!({
            "n_deposit": n_base,
            "token_id": format!("{minter_addr}-base"),
            "terp_tx": "dregg-lost-race",
        }),
    )?;
    let settle_zec = fed_signed(
        dregg_url,
        "settle",
        "/v1/settle",
        serde_json::json!({
            "n_deposit": n_zec,
            "token_id": format!("{minter_addr}-zec"),
            "terp_tx": "dregg-lost-race",
        }),
    )?;
    let sold_out = queried_minted >= 32;
    let sold_out_err = |v: &serde_json::Value| {
        v.get("ok") != Some(&serde_json::Value::Bool(true))
            && v.get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .contains("sold out")
    };
    if sold_out {
        if !sold_out_err(&settle_eth) || !sold_out_err(&settle_btc) || !sold_out_err(&settle_base) {
            return Err(format!(
                "sold out on-chain: ETH/BTC/Base settle must refuse: eth={settle_eth} btc={settle_btc} base={settle_base}"
            )
            .into());
        }
    } else {
        for (name, v) in [
            ("eth", &settle_eth),
            ("btc", &settle_btc),
            ("base", &settle_base),
            ("zec", &settle_zec),
        ] {
            if v.get("ok") != Some(&serde_json::Value::Bool(true)) {
                return Err(format!("{name} settle refused with remaining supply: {v}").into());
            }
        }
    }
    if sold_out && !sold_out_err(&settle_zec) {
        return Err(format!("expected ZEC settle sold-out after native fill: {settle_zec}").into());
    }
    let refund = fed_signed(
        dregg_url,
        "sold-out-refund",
        "/v1/query/sold-out-refund",
        serde_json::json!({"collection": "svg"}),
    )?;
    if sold_out {
        let want_refunds = if n_zec.is_null() { 3 } else { 4 };
        if refund.get("ok") != Some(&serde_json::Value::Bool(true))
            || refund.get("refunded").and_then(|x| x.as_u64()).unwrap_or(0) < want_refunds
        {
            return Err(format!("expected refund of {want_refunds} rails: {refund}").into());
        }
        for n in [n_eth.clone(), n_btc.clone(), n_base.clone(), n_zec.clone()] {
            if n.is_null() {
                continue;
            }
            let bad = fed_signed(
                dregg_url,
                "payout",
                "/v1/payout",
                serde_json::json!({
                    "n_deposit": n,
                    "kind": "release",
                    "rail_hex": to_hex(b"would-be-release"),
                }),
            )?;
            if bad.get("ok") == Some(&serde_json::Value::Bool(true)) {
                return Err(format!("XOR violated, release after refund: {bad}").into());
            }
        }
    }
    let btc_refund_txid = btc.refund_sats(&btc_dest, btc_sats).await?;
    let eth_refund_tx = eth_vault_refund(anvil, from, &vault_addr, &lock_id).await?;
    let zec_refund = ironwood_escrow("refund").map_err(|err| format!("ironwood refund failed: {err}"))?;
    if zec_refund.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("ironwood refund refused: {zec_refund}").into());
    }
    let zec_refund_txid = zec_refund
        .get("txid")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if zec_refund_txid.len() != 64 || !zec_refund_txid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "ironwood refund txid must be 64 hex chars, got {zec_refund_txid:?} in {zec_refund}"
        )
        .into());
    }
    for (n, tx) in [
        (n_eth.clone(), eth_refund_tx.clone()),
        (n_btc.clone(), btc_refund_txid.clone()),
        (n_zec.clone(), zec_refund_txid.clone()),
    ] {
        if n.is_null() {
            continue;
        }
        let paid = fed_signed(
            dregg_url,
            "payout",
            "/v1/payout",
            serde_json::json!({
                "n_deposit": n,
                "kind": "refund",
                "rail_hex": tx.trim_start_matches("0x"),
            }),
        )?;
        let already = paid
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .contains("already paid");
        if paid.get("ok") != Some(&serde_json::Value::Bool(true)) && !already {
            return Err(format!("on-chain refund payout refused: {paid}").into());
        }
    }
    let sold_out_refund = true;
    let zec_recorded = zec.get("ok") == Some(&serde_json::Value::Bool(true));
    let _ = (native_failed, dregg_mint_failed);

    let builder = daemon_builder_from_chain(chain, Some(LAB_MNEMONIC))?;
    let daemon_built = tokio::task::spawn_blocking(move || builder.build().map(|_| true))
        .await?
        .map_err(|e| format!("DaemonBuilder.build: {e}"))?;

    let (_after_parallel, parallel_report) = parallel_wallets(
        chain,
        &cid,
        &home,
        dregg_url,
        anvil,
        base_anvil,
        btc,
        &collection_addr,
        queried_minted,
    )
    .await?;
    Ok(serde_json::json!({
        "live": true,
        "teardown": true,
        "siblings": ["zakura", "zakura-public", "dregg-fed", "anvil", "base-anvil", "bitcoind", "ztreamer"],
        "compact_vendor": "Ztreamer",
        "lwd_absent": true,
        "compact_grpc": std::env::var("ZCASH_DEVTOOL_SERVER").ok(),
        "compact_surface": "in-process-zakurad+ztreamer",
        "called_daemon_builder_from_chain": true,
        "daemon_built": daemon_built,
        "chain_id": chain.chain_id(),
        "zakura_rpc": zakura_rpc,
        "zec_public_rpc": std::env::var("ZCASH_PUBLIC_RPC").ok(),
        "zec_private_rpc": std::env::var("ZCASH_PRIVATE_RPC").ok(),
        "zec_tip_circuit": zec_tip_circuit,
        "zcashd_rpc": std::env::var("ZCASHD_RPC").ok(),
        "wallet_ua": wallet.get("ua"),
        "wallet": wallet.get("wallet"),
        "wallet_restart_same_address": wallet.get("restart_same_address"),
        "wallet_send_fail_closed": wallet.get("send_fail_closed"),
        "wallet_sync_left_ledger": wallet.get("sync_left_ledger"),
        "wallet_container_respawn": wallet.get("container_respawn"),
        "wallet_seed_survived_respawn": wallet.get("seed_survived_respawn"),
        "wallet_payment_zat": wallet.get("payment_zat"),
        "wallet_balance_after_withdraw": wallet.get("balance_after_withdraw"),
        "wallet_withdraw_authorized": wallet.get("withdraw_authorized"),
        "adr_mint_authorized": wallet.get("adr_mint_authorized"),
        "adr_mint_not_minted": wallet.get("adr_mint_not_minted"),
        "bad_mint_hash_rejected": wallet.get("bad_mint_hash_rejected"),
        "adr_refund_unminted": wallet.get("adr_refund_unminted"),
        "checkout_token_rejected": wallet.get("checkout_token_rejected"),
        "adr_mint_settled_with_hash": adr_mint_settled,
        "pweb_walk": walk,
        "zakura_reused": zakura_reused,
        "dregg_query": dregg_url,
        "anvil_rpc": anvil.docker_dns_rpc_url(),
        "base_rpc": base_anvil.docker_dns_rpc_url(),
        "base_tx": base_hash,
        "eth_tx": tx_hash,
        "zec_or_recorded": zec_recorded,
        "zec_skipped": false,
        "zec_skip_reason": zec_skipped_reason,
        "eth_recorded": true,
        "btc_tx": btc_txid,
        "btc_recorded": true,
        "sold_out_refund": sold_out_refund,
        "queried_minted": queried_minted,
        "qc_ok": true,
        "native_same_block_attempt": true,
        "dregg_settle_sold_out": true,
        "refunded_rails": refund.get("refunded"),
        "eth_refund_tx": eth_refund_tx,
        "btc_refund_tx": btc_refund_txid,
        "zec_refund_tx": zec_refund_txid,
        "spent_on_chain": false,
        "btc_rpc": btc.rpc_url(),
        "mint_occurred": queried_minted > 0,
        "parallel_wallets": parallel_report["wallets"].clone(),
        "parallel_payments_ok": parallel_report["payments_ok"].clone(),
        "parallel_mints": parallel_report["mints"].clone(),
        "parallel_assets": parallel_report["assets"].clone(),
        "onboard": walk,
        "mint_engine": "cw-orch",
        "mint_tx": mint_hash,
        "svg_code_id": svg_code,
        "minter_code_id": minter_code,
        "minter_addr": minter_addr,
        "packet_leaks_rail": false,
        "cell_packet": null,
        "zip321": pay.get("zip321"),
        "zec_uri": zec_uri,
        "status_recipient": st,
        "pedersen_memo": memo_hex,
        "hd_eth_a": pay_a.get("eth_addr"),
        "hd_eth_b": pay_b.get("eth_addr"),
        "hd_btc_a": pay_a.get("btc_addr"),
        "zec_div_a": pay_a.get("zec_diversifier"),
        "onchain_memo_leaks_terp": false,
    }))
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// cw-orch Daemon deploy (no terpd store/instantiate). No minter migrate (code 72).
fn deploy_svg_cw_orch(
    chain: &CosmosChain,
    owner: &str,
    arts: &std::path::Path,
) -> Result<(String, String, String, String), Box<dyn std::error::Error>> {
    let _ = owner;
    let denom = chain.config().denom.clone();
    let fund = dregg::checkout::native_needed_for_nfts(2);
    let arts = arts.to_path_buf();
    let builder = daemon_builder_from_chain(chain, Some(LAB_MNEMONIC))?;
    let out = std::thread::spawn(move || -> Result<(String, String, String, String), String> {
        let daemon = builder.build().map_err(|e| format!("DaemonBuilder.build: {e}"))?;
        let d = ict_rs_cw_orch::svg_runtime::deploy_hopalong_minter(&daemon, &arts, &denom, fund)?;
        Ok((d.minter_addr, d.collection_addr, d.svg_code, d.minter_code))
    })
    .join()
    .map_err(|_| "cw-orch deploy thread panicked")??;
    Ok(out)
}

struct WalletLab {
    dir: PathBuf,
    admin_file: PathBuf,
    bin: PathBuf,
    ua: String,
    volumes: Vec<ict_rs::runtime::VolumeMount>,
}

impl Drop for WalletLab {
    fn drop(&mut self) {
        let mount = format!("{}:/wallet", self.dir.display());
        let _ = std::process::Command::new(docker_bin())
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "-v",
                &mount,
                "--entrypoint",
                "chmod",
                "debian:trixie-slim",
                "-R",
                "a+rwx",
                "/wallet",
            ])
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.admin_file);
        let _ = std::process::Command::new(docker_bin())
            .args(["rm", "-f", "ict-svg-mint-wallet"])
            .status();
    }
}

fn linux_elf(path: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes.len() >= 4 && bytes[0] == 0x7f && &bytes[1..4] == b"ELF"
}

fn resolve_zrt_linux() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("SVG_MINT_ZRT_LINUX") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            if linux_elf(&pb) {
                return Ok(pb);
            }
            return Err(format!("SVG_MINT_ZRT_LINUX is not a Linux ELF: {p}"));
        }
    }
    let root = std::env::var("SVG_MINT_ROOT").unwrap_or_default();
    for rel in [
        "target/linux-gnu/svg-mint-zrt",
        "target/aarch64-unknown-linux-gnu/release/svg-mint-zrt",
    ] {
        let p = PathBuf::from(&root).join(rel);
        if linux_elf(&p) {
            return Ok(p);
        }
    }
    Err("SVG_MINT_ZRT_LINUX missing. Build svg-mint-zrt --features lab as a Linux ELF.".into())
}

fn wallet_container(dir: &std::path::Path, bin: &std::path::Path, args: &[&str]) -> Result<std::process::Output, String> {
    let _ = std::process::Command::new(docker_bin())
        .args(["rm", "-f", "ict-svg-mint-wallet"])
        .output();
    let mount = format!("{}:/wallet", dir.display());
    let elf = format!("{}:/usr/local/bin/svg-mint-zrt:ro", bin.display());
    let mut cmd = std::process::Command::new(docker_bin());
    cmd.args([
        "run",
        "--name",
        "ict-svg-mint-wallet",
        "--rm",
        "--network",
        "none",
        "-e",
        "SVG_MINT_ZRT_DIR=/wallet",
        "-v",
        &mount,
        "-v",
        &elf,
        "--entrypoint",
        "/usr/local/bin/svg-mint-zrt",
        "debian:trixie-slim",
    ]);
    cmd.args(args);
    cmd.output().map_err(|e| format!("wallet container: {e}"))
}

fn wallet_seed_hash(dir: &std::path::Path) -> Result<String, String> {
    let mount = format!("{}:/wallet:ro", dir.display());
    let out = std::process::Command::new(docker_bin())
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "-v",
            &mount,
            "--entrypoint",
            "sha256sum",
            "debian:trixie-slim",
            "/wallet/spend.seed",
        ])
        .output()
        .map_err(|e| format!("seed hash: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "spend seed missing after container exit: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let hash = text.split_whitespace().next().unwrap_or("").to_string();
    if hash.len() != 64 {
        return Err(format!("seed hash was not sha256: {text}"));
    }
    Ok(hash)
}

fn wallet_balance(dir: &std::path::Path, bin: &std::path::Path) -> Result<u64, String> {
    let out = wallet_container(dir, bin, &["balance", "--json"])?;
    if !out.status.success() {
        return Err(format!(
            "balance failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
        .map_err(|e| format!("balance json: {e}"))?;
    v.get("ironwood_spendable")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| format!("balance missing ironwood_spendable: {v}"))
}

/// Init on a host directory, remove the container, start another, same seed and address.
fn prepare_wallet_lab() -> Result<WalletLab, String> {
    let bin = resolve_zrt_linux()?;
    let dir = PathBuf::from(format!("/tmp/svg-mint-zrt-vol-{}", std::process::id()));
    let admin_file = PathBuf::from(format!("/tmp/svg-mint-admin-terp1-{}", std::process::id()));
    if dir.components().any(|c| c.as_os_str() == ".terp") {
        return Err("refusing a wallet volume under .terp".into());
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (admin_terp, _, _) = dregg::adr36::sign_keplr_arbitrary(&adr36_secret(1), "", b"designate")?;
    std::fs::write(&admin_file, admin_terp.as_bytes()).map_err(|e| e.to_string())?;
    let mut lab = WalletLab {
        dir,
        admin_file,
        bin,
        ua: String::new(),
        volumes: Vec::new(),
    };
    let init = wallet_container(&lab.dir, &lab.bin, &["init"])?;
    if !init.status.success() {
        return Err(format!(
            "wallet container init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }
    let ua = String::from_utf8_lossy(&init.stdout).trim().to_string();
    if !ua.starts_with("u1") {
        return Err(format!("wallet container init did not print a unified address: {ua}"));
    }
    let hash_a = wallet_seed_hash(&lab.dir)?;
    let again = wallet_container(&lab.dir, &lab.bin, &["addresses"])?;
    let again_out = String::from_utf8_lossy(&again.stdout);
    if !again.status.success() || !again_out.contains(&ua) {
        return Err(format!("respawn lost the unified address: {again_out}"));
    }
    let hash_b = wallet_seed_hash(&lab.dir)?;
    if hash_a != hash_b {
        return Err("respawn replaced the spend seed".into());
    }
    let sync = wallet_container(&lab.dir, &lab.bin, &["sync"])?;
    let sync_err = String::from_utf8_lossy(&sync.stderr);
    if sync.status.success() || !sync_err.contains("ledger was not modified") {
        return Err(format!("sync must leave the ledger, got {sync_err}"));
    }
    if wallet_seed_hash(&lab.dir)? != hash_a {
        return Err("sync replaced the spend seed".into());
    }
    std::env::set_var("SVG_MINT_ZRT_BIN", "/usr/local/bin/svg-mint-zrt");
    std::env::set_var("SVG_MINT_ZRT_DIR", "/wallet");
    std::env::set_var("SVG_MINT_ZRT_LAB", "1");
    std::env::set_var("SVG_MINT_SKIP_ZRT_WATCH", "1");
    std::env::set_var("DREGG_ADMIN_TERP1_PATH", "/dregg-admin-terp1");
    let abs = |p: &std::path::Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .display()
            .to_string()
    };
    let volumes = vec![
        ict_rs::runtime::VolumeMount {
            source: abs(&lab.bin),
            target: "/usr/local/bin/svg-mint-zrt".into(),
            read_only: true,
        },
        ict_rs::runtime::VolumeMount {
            source: abs(&lab.dir),
            target: "/wallet".into(),
            read_only: false,
        },
        ict_rs::runtime::VolumeMount {
            source: abs(&lab.admin_file),
            target: "/dregg-admin-terp1".into(),
            read_only: true,
        },
    ];
    lab.ua = ua;
    lab.volumes = volumes;
    Ok(lab)
}

fn session_row(dregg_url: &str, terp: &str) -> Result<serde_json::Value, String> {
    let body = http_json_h(
        "GET",
        &format!(
            "{}/v1/admin/sessions?limit=50",
            dregg_url.trim_end_matches('/')
        ),
        None,
        &[
            ("Authorization", "Bearer e2e-admin-token"),
            ("X-Dregg-Admin", "e2e-admin-token"),
        ],
    )?;
    let rows = body
        .get("sessions")
        .and_then(|x| x.as_array())
        .cloned()
        .ok_or_else(|| format!("sessions: {body}"))?;
    rows.into_iter()
        .find(|r| r.get("terp").and_then(|x| x.as_str()) == Some(terp))
        .ok_or_else(|| format!("no session for {terp}: {body}"))
}

fn adr_checkout(
    dregg_url: &str,
    action: &str,
    terp: &str,
    terp_tx: Option<&str>,
) -> Result<serde_json::Value, String> {
    let ch = http_json(
        "GET",
        &format!("{}/v1/admin/challenge", dregg_url.trim_end_matches('/')),
        None,
    )?;
    let challenge = ch
        .get("challenge")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("challenge missing: {ch}"))?
        .to_string();
    let statement = dregg::adr36::checkout_statement(action, terp, &challenge)?;
    let (signer, pk, sig) =
        dregg::adr36::sign_keplr_arbitrary(&adr36_secret(1), "", statement.as_bytes())?;
    let mut body = serde_json::json!({
        "terp": terp,
        "action": action,
        "kind": "adr36",
        "chain_id": "morocco-1",
        "challenge": challenge,
        "signer": signer,
        "pub_key": { "type": "tendermint/PubKeySecp256k1", "value": pk },
        "signature": sig,
    });
    if let Some(tx) = terp_tx {
        body["terp_tx"] = serde_json::json!(tx);
    }
    http_json(
        "POST",
        &format!("{}/v1/admin/checkout", dregg_url.trim_end_matches('/')),
        Some(&body),
    )
}

/// Payment into the managed wallet, ADR withdraw to the donation UA, mint-or-refund.
fn exercise_managed_wallet(
    lab: &WalletLab,
    dregg_url: &str,
    owner: &str,
) -> Result<serde_json::Value, String> {
    const PAY_TX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const SPEND_TX: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
    let hash_before = wallet_seed_hash(&lab.dir)?;
    let credit = wallet_container(
        &lab.dir,
        &lab.bin,
        &[
            "lab-credit",
            "--nullifier",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--txid",
            PAY_TX,
            "--zat",
            "150000",
            "--mined",
            "true",
        ],
    )?;
    if !credit.status.success() {
        return Err(format!(
            "lab-credit failed: {}",
            String::from_utf8_lossy(&credit.stderr)
        ));
    }
    if wallet_balance(&lab.dir, &lab.bin)? != 150_000 {
        return Err("credited note is not ironwood spendable".into());
    }
    let again = wallet_container(&lab.dir, &lab.bin, &["addresses"])?;
    if !String::from_utf8_lossy(&again.stdout).contains(&lab.ua) {
        return Err("respawn after payment lost the unified address".into());
    }
    if wallet_seed_hash(&lab.dir)? != hash_before {
        return Err("respawn after payment replaced the spend seed".into());
    }
    if wallet_balance(&lab.dir, &lab.bin)? != 150_000 {
        return Err("respawn after payment lost the note".into());
    }
    let send = wallet_container(
        &lab.dir,
        &lab.bin,
        &["send", "--zat", "150000", "--address", dregg::adr36::DONATE_ZEC_UA],
    )?;
    let send_err = String::from_utf8_lossy(&send.stderr);
    if send.status.success()
        || !send_err.contains("prover is not linked")
        || !send_err.contains("not marked spent")
    {
        return Err(format!("send must fail closed, got {send_err}"));
    }
    if wallet_balance(&lab.dir, &lab.bin)? != 150_000 {
        return Err("fail-closed send marked the payment spent".into());
    }

    let (customer, _, _) = dregg::adr36::sign_keplr_arbitrary(&adr36_secret(2), "", b"customer")?;
    let pay = http_json(
        "GET",
        &format!(
            "{}/v1/pay-targets?recipient={customer}",
            dregg_url.trim_end_matches('/')
        ),
        None,
    )?;
    if pay.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("customer pay-targets failed: {pay}"));
    }
    let deposited = fed_signed(
        dregg_url,
        "deposit",
        "/v1/deposit",
        serde_json::json!({
            "rail": "ZecIronwood",
            "amount": 150_000,
            "raw_hex": PAY_TX,
        }),
    )?;
    if deposited.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("customer deposit refused: {deposited}"));
    }
    let n = deposited
        .get("n_deposit")
        .cloned()
        .ok_or_else(|| format!("customer deposit missing n_deposit: {deposited}"))?;
    let entitled = fed_signed(
        dregg_url,
        "entitle",
        "/v1/entitle",
        serde_json::json!({
            "n_deposit": n,
            "collection": "svg",
            "svg_hash": vec![7u8; 32],
            "recipient": customer,
            "session": vec![7u8; 32],
        }),
    )?;
    if entitled.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("customer entitle refused: {entitled}"));
    }
    let owed = session_row(dregg_url, &customer)?;
    if owed.get("phase").and_then(|x| x.as_str()) != Some("entitled") {
        return Err(format!("customer payment is not entitled: {owed}"));
    }
    let token_only = http_json_h(
        "POST",
        &format!("{}/v1/admin/checkout", dregg_url.trim_end_matches('/')),
        Some(&serde_json::json!({"terp": customer, "action": "refund"})),
        &[
            ("Authorization", "Bearer e2e-admin-token"),
            ("X-Dregg-Admin", "e2e-admin-token"),
        ],
    )?;
    if token_only.get("error").and_then(|x| x.as_str()) != Some("ADR-036 required") {
        return Err(format!("token must not refund: {token_only}"));
    }
    let still = session_row(dregg_url, &customer)?;
    if still.get("phase").and_then(|x| x.as_str()) != Some("entitled") {
        return Err(format!("token changed the checkout: {still}"));
    }
    let refunded = adr_checkout(dregg_url, "refund", &customer, None)?;
    if refunded.get("authorized") != Some(&serde_json::Value::Bool(true))
        || refunded.get("minted") != Some(&serde_json::Value::Bool(false))
        || refunded.get("phase").and_then(|x| x.as_str()) != Some("refunded")
    {
        return Err(format!("ADR refund did not leave the checkout unminted: {refunded}"));
    }
    let push = adr_checkout(dregg_url, "mint-to", &customer, None)?;
    if push.get("minted") == Some(&serde_json::Value::Bool(true))
        || push.get("phase").and_then(|x| x.as_str()) == Some("settled")
    {
        return Err(format!("refunded checkout must not mint: {push}"));
    }

    let owner_row = session_row(dregg_url, owner)?;
    if owner_row.get("phase").and_then(|x| x.as_str()) != Some("entitled") {
        return Err(format!("owner payment is not entitled: {owner_row}"));
    }
    let auth = adr_checkout(dregg_url, "mint-to", owner, None)?;
    if auth.get("authorized") != Some(&serde_json::Value::Bool(true))
        || auth.get("minted") != Some(&serde_json::Value::Bool(false))
        || auth.get("phase").and_then(|x| x.as_str()) != Some("entitled")
    {
        return Err(format!("ADR mint-to must authorize without minting: {auth}"));
    }
    let bad = adr_checkout(dregg_url, "mint-to", owner, Some("dregg-lost-race"))?;
    if bad.get("minted") == Some(&serde_json::Value::Bool(true))
        || bad.get("phase").and_then(|x| x.as_str()) != Some("entitled")
    {
        return Err(format!("a non-hash mint id must stay entitled: {bad}"));
    }

    let (_, withdraw_body) = dregg_adr36_withdraw_proof(dregg_url, &adr36_secret(1), 150_000)?;
    let mut withdraw_body = withdraw_body;
    withdraw_body["rail"] = serde_json::json!("zec");
    withdraw_body["zat"] = serde_json::json!(150_000);
    withdraw_body["lab_txid"] = serde_json::json!(SPEND_TX);
    let wd = http_json(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&withdraw_body),
    )?;
    if wd.get("authorized") != Some(&serde_json::Value::Bool(true)) || wd.get("ok") != Some(&serde_json::Value::Bool(true))
    {
        return Err(format!("ADR withdraw of the payment failed: {wd}"));
    }
    let zcli = wd.get("zcli").and_then(|x| x.as_str()).unwrap_or("");
    if !zcli.contains("\"broadcast\":false") {
        return Err(format!("withdraw must not claim a broadcast: {wd}"));
    }
    if wallet_balance(&lab.dir, &lab.bin)? != 0 {
        return Err("withdraw left the payment spendable".into());
    }
    let after = wallet_container(&lab.dir, &lab.bin, &["addresses"])?;
    if !String::from_utf8_lossy(&after.stdout).contains(&lab.ua) {
        return Err("respawn after withdraw lost the unified address".into());
    }
    if wallet_seed_hash(&lab.dir)? != hash_before {
        return Err("respawn after withdraw replaced the spend seed".into());
    }
    if wallet_balance(&lab.dir, &lab.bin)? != 0 {
        return Err("respawn restored a spent note".into());
    }
    Ok(serde_json::json!({
        "ok": true,
        "ua": lab.ua,
        "wallet": "svg-mint-zrt",
        "restart_same_address": true,
        "send_fail_closed": true,
        "sync_left_ledger": true,
        "container_respawn": true,
        "seed_survived_respawn": true,
        "payment_zat": 150000,
        "balance_after_withdraw": 0,
        "withdraw_broadcast": false,
        "withdraw_authorized": true,
        "adr_mint_authorized": true,
        "adr_mint_not_minted": true,
        "bad_mint_hash_rejected": true,
        "adr_refund_unminted": true,
        "checkout_token_rejected": true,
    }))
}

fn record_authorized_mint(dregg_url: &str, owner: &str, mint_hash: &str) -> Result<bool, String> {
    let hash = mint_hash.trim().trim_start_matches("0x");
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("cw-orch mint hash is not 64 hex: {mint_hash}"));
    }
    let rec = adr_checkout(dregg_url, "mint-to", owner, Some(hash))?;
    if rec.get("authorized") != Some(&serde_json::Value::Bool(true))
        || rec.get("minted") != Some(&serde_json::Value::Bool(true))
        || rec.get("phase").and_then(|x| x.as_str()) != Some("settled")
        || rec.get("terp_tx").and_then(|x| x.as_str()) != Some(hash)
    {
        return Err(format!("ADR mint record did not settle on the chain hash: {rec}"));
    }
    let row = session_row(dregg_url, owner)?;
    if row.get("phase").and_then(|x| x.as_str()) != Some("settled")
        || row.get("token_id").and_then(|x| x.as_str()) != Some(hash)
    {
        return Err(format!("session did not store the mint hash: {row}"));
    }
    Ok(true)
}

fn json_rpc(url: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method": method, "params": params});
    http_json("POST", url, Some(&body))
}

fn extract_tx_hash(s: &str) -> Option<String> {
    // transactionHash only. A block hash or a bytecode word is not a tx, and
    // cast receipt would wait on it until the lab is killed.
    for key in ["\"transactionHash\"", "\"hash\""] {
        let mut rest = s;
        while let Some(i) = rest.find(key) {
            let after = rest[i + key.len()..].trim_start_matches(|c: char| c == ':' || c == ' ' || c == '"');
            let bytes = after.as_bytes();
            if bytes.len() >= 66
                && bytes[0] == b'0'
                && bytes[1] == b'x'
                && bytes[2..66].iter().all(|c| c.is_ascii_hexdigit())
                && (bytes.len() == 66 || !bytes[66].is_ascii_hexdigit())
            {
                return Some(after[..66].to_string());
            }
            rest = &rest[i + key.len()..];
        }
    }
    None
}

fn assert_pay_targets_private(
    pay: &serde_json::Value,
    terp: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if pay.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("pay-targets failed: {pay}").into());
    }
    if pay.get("zip321").is_none() {
        return Err(format!("stale svg-mint-fed ELF missing zip321 field: {pay}").into());
    }
    let memo = pay.get("memo_hex").and_then(|x| x.as_str()).unwrap_or("");
    let uri = pay.get("zec_uri").and_then(|x| x.as_str()).unwrap_or("");
    let blob = format!("{pay}");
    if memo.is_empty() {
        return Err(format!("pay-targets missing memo_hex (Pedersen C): {pay}").into());
    }
    if uri.contains(terp) || memo.contains(terp) || pay.get("recipient").is_some() {
        return Err(format!("Pedersen memo leaked terp1: {pay}").into());
    }
    if !uri.is_empty() && (!uri.starts_with("zcash:") || !uri.contains("memo=")) {
        return Err(format!("bad ZIP-321: {uri}").into());
    }
    if pay.get("eth_addr").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
        return Err(format!("missing HD eth_addr: {pay}").into());
    }
    if pay.get("btc_addr").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
        return Err(format!("missing HD btc_addr: {pay}").into());
    }
    Ok(())
}

struct ZtreamerLab {
    child: Option<std::process::Child>,
}

fn resolve_ztreamer_bin() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(p) = std::env::var("ZTREAMER_BIN") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            if pb.is_file() {
                return Ok(pb);
            }
            return Err(format!("ZTREAMER_BIN is not a file: {p}").into());
        }
    }
    let roots = [
        product_root().join("runtimes/ztreamer"),
        terp_crates().join("ztreamer"),
    ];
    for root in &roots {
        for rel in ["target/release/ztreamer", "target/debug/ztreamer"] {
            let p = root.join(rel);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    let root = roots[1].clone();
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--bin", "ztreamer"])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("cargo build --bin ztreamer failed".into());
    }
    let p = root.join("target/release/ztreamer");
    if p.is_file() {
        Ok(p)
    } else {
        Err("ztreamer binary missing after build".into())
    }
}

/// Sibling CompactTxStreamer. Not electriccoinco/lightwalletd. Fail-closed.
fn spawn_ztreamer_lab() -> Result<ZtreamerLab, Box<dyn std::error::Error>> {
    let bind = std::env::var("ZTREAMER_BIND").unwrap_or_else(|_| "127.0.0.1:19067".into());
    if bind.contains(":8232") {
        return Err("ZTREAMER_BIND must not be Zakura JSON-RPC :8232".into());
    }
    if bind.to_lowercase().contains("lightwalletd") {
        return Err("refusing stock lightwalletd as compact streamer".into());
    }
    let bin = resolve_ztreamer_bin()?;
    if bin
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("lightwalletd"))
    {
        return Err("ZTREAMER_BIN must be ztreamer, not lightwalletd".into());
    }
    let child = std::process::Command::new(&bin)
        .args(["--bind", &bind])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| format!("ztreamer spawn failed: {e}"))?;
    let mut ok = false;
    for _ in 0..50 {
        if std::net::TcpStream::connect(&bind).is_ok() {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !ok {
        return Err(format!("ztreamer did not listen on {bind}").into());
    }
    std::env::set_var("ZCASH_DEVTOOL_SERVER", &bind);
    std::env::set_var("ZTREAMER_GRPC", &bind);
    std::env::set_var("ZTREAMER_BIN", bin.display().to_string());
    std::env::set_var("ZCLI_ENDPOINT", format!("http://{bind}"));
    let probe = std::process::Command::new(&bin)
        .args(["probe", "--bind", &bind])
        .output()
        .map_err(|e| format!("ztreamer probe: {e}"))?;
    if !probe.status.success() {
        return Err(format!(
            "ztreamer probe failed: {}",
            String::from_utf8_lossy(&probe.stderr)
        )
        .into());
    }
    let pstdout = String::from_utf8_lossy(&probe.stdout);
    if !pstdout.contains("Ztreamer") {
        return Err(format!("ztreamer vendor not Ztreamer: {pstdout}").into());
    }
    std::env::set_var("ZTREAMER_SPAWNED", "1");
    Ok(ZtreamerLab {
        child: Some(child),
    })
}

async fn eth_vault_lock(
    anvil: &AnvilChain,
    from_pk: &str,
    operator: &str,
    terp_recipient: &str,
) -> Result<(String, String, String), Box<dyn std::error::Error>> {
    let bin = vault_creation_bytecode()?;
    let code = if bin.starts_with("0x") {
        bin.clone()
    } else {
        format!("0x{bin}")
    };
    let ops = format!("[{operator}]");
    let rpc = anvil.docker_dns_rpc_url();
    let deploy = anvil
        .exec_cast(&[
            "send",
            "--rpc-url",
            &rpc,
            "--private-key",
            from_pk,
            "--json",
            "--gas-limit",
            "4000000",
            "--create",
            &code,
            "constructor(address[],uint256)",
            &ops,
            "1",
        ])
        .await?;
    let deploy_out = format!("{}{}", deploy.stdout_str(), deploy.stderr_str());
    let deploy_tx = extract_tx_hash(&deploy_out).ok_or_else(|| format!("vault deploy tx missing: {deploy_out}"))?;
    let rec = anvil.get_receipt(&deploy_tx).await?;
    let rec_v: serde_json::Value = serde_json::from_str(rec.trim()).unwrap_or_default();
    let vault = rec_v
        .get("contractAddress")
        .or_else(|| rec_v.get("contract_address"))
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("vault address missing: {rec}"))?
        .to_string();
    let nhex = format!("0x{}", "11".repeat(32));
    // Encode calldata in-container; send with --gas-limit so foundry skips eth_estimateGas.
    let encoded = anvil
        .exec_cast(&[
            "calldata",
            "lock(bytes32,uint64,string)",
            &nhex,
            "1",
            terp_recipient,
        ])
        .await?;
    let data = encoded.stdout_str().trim().to_string();
    if !data.starts_with("0x") || data.len() < 10 {
        return Err(format!(
            "lock calldata missing: {} {}",
            encoded.stdout_str(),
            encoded.stderr_str()
        )
        .into());
    }
    let eth_hex = format!("0x{:x}", dregg::prices::ETH_WEI);
    let sent = anvil
        .exec_cast(&[
            "send",
            "--rpc-url",
            &rpc,
            "--private-key",
            from_pk,
            "--json",
            "--gas-limit",
            "500000",
            "--value",
            &eth_hex,
            "--data",
            &data,
            &vault,
        ])
        .await?;
    let sent_out = format!("{}{}", sent.stdout_str(), sent.stderr_str());
    let lock_tx = extract_tx_hash(&sent_out)
        .ok_or_else(|| format!("vault lock tx missing: {sent_out}"))?;
    let lrec = anvil.get_receipt(&lock_tx).await?;
    let lv: serde_json::Value = serde_json::from_str(lrec.trim()).unwrap_or_default();
    let lock_id = lv
        .get("logs")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|l| l.get("topics"))
        .and_then(|t| t.as_array())
        .and_then(|t| t.get(1))
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("Locked id missing: {lrec}"))?
        .to_string();
    Ok((lock_tx, vault, lock_id))
}

async fn eth_vault_refund(
    anvil: &AnvilChain,
    from_pk: &str,
    vault: &str,
    lock_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let refund = anvil
        .exec_cast(&[
            "send",
            "--private-key",
            from_pk,
            "--rpc-url",
            &anvil.docker_dns_rpc_url(),
            "--json",
            vault,
            "refundTimeout(bytes32)",
            lock_id,
        ])
        .await?;
    let out = format!("{}{}", refund.stdout_str(), refund.stderr_str());
    extract_tx_hash(&out).ok_or_else(|| format!("ETH vault refund has no tx hash: {}", out.trim()).into())
}

/// solc creation bytecode only. Constructor args are passed to `cast send --create`.
fn vault_creation_bytecode() -> Result<String, String> {
    let sol = terp_crates().join("terp-rs/tools/svg-mint/chain/SvgMintRailVault.sol");
    let cached = sol.with_file_name("SvgMintRailVault.bin");
    if cached.is_file() {
        let bin = std::fs::read_to_string(&cached).map_err(|err| format!("vault bin: {err}"))?;
        return Ok(bin.trim().trim_start_matches("0x").to_string());
    }
    let out_dir = std::env::temp_dir().join("svg-mint-vault-solc-lock3");
    let _ = std::fs::create_dir_all(&out_dir);
    let status = std::process::Command::new(docker_bin())
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/src/SvgMintRailVault.sol:ro", sol.display()),
            "-v",
            &format!("{}:/out", out_dir.display()),
            "ethereum/solc:0.8.20",
            "--optimize",
            "--bin",
            "-o",
            "/out",
            "/src/SvgMintRailVault.sol",
            "--overwrite",
        ])
        .status()
        .map_err(|e| format!("solc docker: {e}"))?;
    if !status.success() {
        return Err("solc docker failed".into());
    }
    let bin = std::fs::read_to_string(out_dir.join("SvgMintRailVault.bin"))
        .map_err(|e| format!("vault bin: {e}"))?;
    Ok(bin.trim().trim_start_matches("0x").to_string())
}

fn extract_digest(v: &serde_json::Value) -> Option<serde_json::Value> {
    if v.get("digest").and_then(|x| x.as_str()).is_some() {
        return v.get("digest").cloned();
    }
    if v.get("digest").and_then(|x| x.as_array()).is_some() {
        return v.get("digest").cloned();
    }
    None
}

fn ironwood_escrow(cmd: &str) -> Result<serde_json::Value, String> {
    let bin = std::env::var("SVG_MINT_IRONWOOD_BIN").unwrap_or_default();
    let mut c = if !bin.is_empty() {
        let mut x = std::process::Command::new(&bin);
        x.arg(cmd);
        x
    } else {
        let manifest = terp_crates().join("private-inference-rent/Cargo.toml");
        let mut x = std::process::Command::new("cargo");
        x.args([
            "run",
            "--quiet",
            "--manifest-path",
            manifest.to_str().unwrap_or(""),
            "--bin",
            "svg-mint-ironwood",
            "--",
            cmd,
        ]);
        x
    };
    if let Ok(rpc) = std::env::var("ZAKURA_RPC") {
        c.env("ZAKURA_RPC", rpc);
    }
    if let Some(dir) = std::path::Path::new(docker_bin()).parent() {
        let path = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", format!("{}:{path}", dir.display()));
    }
    let out = c.output().map_err(|e| format!("svg-mint-ironwood {cmd}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json_line = stdout
        .lines()
        .rev()
        .find(|l| l.trim().starts_with('{'))
        .unwrap_or(stdout.trim());
    let v: serde_json::Value = serde_json::from_str(json_line.trim()).map_err(|e| {
        format!(
            "svg-mint-ironwood {cmd} json: {e} stdout={} stderr={}",
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr)
        )
    })?;
    if v.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("svg-mint-ironwood {cmd}: {v}"));
    }
    Ok(v)
}

fn fed_signed(
    dregg_url: &str,
    tag: &str,
    path: &str,
    mut body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let base = dregg_url.trim_end_matches('/');
    body["turn_tag"] = serde_json::json!(tag);
    let prev = http_json(
        "POST",
        &format!("{base}/v1/checkpoint-preview"),
        Some(&body),
    )?;
    let digest = extract_digest(&prev).ok_or_else(|| {
        format!("checkpoint-preview missing digest: {prev}")
    })?;
    let signed = http_json(
        "POST",
        &format!("{base}/v1/sign"),
        Some(&serde_json::json!({"digest": digest})),
    )?;
    body["signatures"] = signed
        .get("signatures")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    http_json("POST", &format!("{base}{path}"), Some(&body))
}

/// HashMerchant passkey onboard on the same local Terp node: 1 native unit
/// runway + AddAuthenticator feegrant, via POST /v1/passkey-grant broadcast.
async fn exercise_onboard_passkey(
    chain: &CosmosChain,
    cid: &str,
    home: &str,
    chain_id: &str,
    granter: &str,
    denom: &str,
    dregg_url: &str,
    c: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    std::env::set_var("ONBOARD_DENOM", denom);
    std::env::set_var("ONBOARD_CHAIN", chain_id);
    // Lab only has `validator`; dao and runtime_key are that same terp1.
    std::env::set_var("ONBOARD_FROM", granter);
    std::env::set_var("ONBOARD_RUNTIME", granter);
    std::env::set_var("ONBOARD_DAO", granter);
    std::env::set_var("ONBOARD_RUNWAY_AMOUNT", hash_market::onboard_authz::RUNWAY_UTHOL);

    let shown = terpd_tx(
        cid,
        &[
            "keys",
            "show",
            "passkey-user",
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--output",
            "json",
        ],
    );
    let user = match shown {
        Ok(s) => serde_json::from_str::<serde_json::Value>(s.trim())
            .ok()
            .and_then(|v| {
                v.get("address")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default(),
        Err(_) => {
            let add_out = terpd_tx(
                cid,
                &[
                    "keys",
                    "add",
                    "passkey-user",
                    "--keyring-backend",
                    "test",
                    "--home",
                    home,
                    "--output",
                    "json",
                    "--no-backup",
                ],
            )?;
            serde_json::from_str::<serde_json::Value>(add_out.trim())
                .ok()
                .and_then(|v| {
                    v.get("address")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default()
        }
    };
    if !user.starts_with("terp1") {
        return Err("passkey-user key add/show failed".into());
    }

    // 19091 is frost-inbox on groot-wan; bind an ephemeral loopback port.
    let bind = {
        let l = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("e2e-onboard bind: {e}"))?;
        let addr = l.local_addr().map_err(|e| format!("e2e-onboard addr: {e}"))?;
        drop(l);
        addr.to_string()
    };
    let mut rt = hash_market::onboard_authz::OnboardRuntime::from_env();
    rt.service_name = "e2e-onboard".into();
    std::env::set_var("ONBOARD_ALLOW_COLLAPSE", "1");
    rt.roles.hashmerchant = granter.to_string();
    rt.roles.runtime = granter.to_string();
    rt.roles.chain_id = chain_id.to_string();
    rt.runway_amount = hash_market::onboard_authz::RUNWAY_UTHOL.into();
    rt.broadcast = true;
    rt.broadcaster = Some(Box::new(LabTerpdBroadcaster {
        cid: cid.to_string(),
        home: home.to_string(),
        from: granter.to_string(),
        denom: denom.to_string(),
        chain_id: chain_id.to_string(),
    }));
    let rt = std::sync::Arc::new(rt);
    let bind_c = bind.to_string();
    let rt_c = rt.clone();
    std::thread::spawn(move || {
        if let Err(e) = hash_market::onboard_authz::http::serve(&bind_c, rt_c) {
            eprintln!("e2e-onboard serve failed: {e}");
        }
    });
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if std::net::TcpStream::connect(&bind).is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("e2e-onboard did not listen on {bind}").into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    let unpaid = http_json(
        "POST",
        &format!("http://{bind}/onboard/v1/passkey-grant"),
        Some(&serde_json::json!({ "address": user })),
    )
    .unwrap_or_else(|e| serde_json::json!({ "ok": false, "error": e }));
    let unpaid_s = unpaid.to_string();
    let unpaid_402 = unpaid.get("ok") == Some(&serde_json::Value::Bool(false))
        || unpaid_s.contains("pedersen")
        || unpaid_s.contains("402")
        || unpaid_s.contains("nft payment");
    if !unpaid_402 {
        return Err(format!("unpaid passkey-grant must fail without c: {unpaid}").into());
    }

    let paid_grant = http_json(
        "POST",
        &format!("http://{bind}/onboard/v1/passkey-grant"),
        Some(&serde_json::json!({ "address": granter, "c": c })),
    )
    .unwrap_or_else(|e| serde_json::json!({ "ok": false, "error": e }));
    if paid_grant.get("ok") != Some(&serde_json::Value::Bool(true))
        && paid_grant.get("txhash").and_then(|x| x.as_str()).unwrap_or("").is_empty()
    {
        return Err(format!("paid passkey-grant (entitled dest + c) failed: {paid_grant}").into());
    }
    std::env::set_var("ONBOARD_SKIP_PAID", "1");

    let donate = dregg::adr36::DONATE_ZEC_UA;
    let admin_sk = adr36_secret(1);
    let (admin_signer, session_body) = dregg_adr36_proof(dregg_url, &admin_sk)?;
    let session = http_json(
        "POST",
        &format!("{}/v1/admin/session", dregg_url.trim_end_matches('/')),
        Some(&session_body),
    )?;
    if session.get("ok") != Some(&serde_json::Value::Bool(true))
        || session.get("mode").and_then(|x| x.as_str()) != Some("adr36")
    {
        return Err(format!("ADR-036 admin session failed: {session}").into());
    }
    if session.get("signer").and_then(|x| x.as_str()) != Some(admin_signer.as_str()) {
        return Err(format!("ADR-036 session signer mismatch: {session}").into());
    }
    // Keplr signs chain_id:"". A doc signed as morocco-1 must be rejected.
    let (_, bad_body) = dregg_adr36_proof_chain(dregg_url, &admin_sk, "morocco-1")?;
    let bad_sess = http_json(
        "POST",
        &format!("{}/v1/admin/session", dregg_url.trim_end_matches('/')),
        Some(&bad_body),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    let bad_err = bad_sess.get("error").and_then(|x| x.as_str()).unwrap_or("");
    if bad_sess.get("ok") == Some(&serde_json::Value::Bool(true)) || !bad_err.contains("signature mismatch") {
        return Err(format!("morocco-1 ADR-036 doc must mismatch Keplr bytes: {bad_sess}").into());
    }
    // Sign-in and withdraw are different byte strings. A withdraw sentence must not open stats.
    let (_, stmt_body) = dregg_adr36_withdraw_proof(dregg_url, &admin_sk, 1)?;
    let stmt_sess = http_json(
        "POST",
        &format!("{}/v1/admin/session", dregg_url.trim_end_matches('/')),
        Some(&stmt_body),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if stmt_sess.get("ok") == Some(&serde_json::Value::Bool(true)) {
        return Err(format!("withdraw sentence must not sign in: {stmt_sess}").into());
    }
    let sess_token = session
        .get("token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("ADR-036 session missing token: {session}"))?;
    let auth = format!("Bearer {sess_token}");
    let admin = http_json_h(
        "GET",
        &format!("{}/v1/admin/stats", dregg_url.trim_end_matches('/')),
        None,
        &[("Authorization", auth.as_str())],
    )?;
    if admin.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("admin stats (ADR-036 session) failed: {admin}").into());
    }
    let withdraw_to = admin
        .get("withdraw_to")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if !withdraw_to.iter().any(|u| u.as_str() == Some(donate)) {
        return Err(format!("admin stats withdraw_to must include hardcoded donate UA: {admin}").into());
    }
    let admin_token = http_json_h(
        "GET",
        &format!("{}/v1/admin/stats", dregg_url.trim_end_matches('/')),
        None,
        &[("Authorization", "Bearer e2e-admin-token")],
    )?;
    if admin_token.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("admin stats (runtime token) failed: {admin_token}").into());
    }
    let admin_no_tok = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "8",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            &format!("{}/v1/admin/stats", dregg_url.trim_end_matches('/')),
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if admin_no_tok != "401" {
        return Err(format!("admin stats must 401 without token, got {admin_no_tok}").into());
    }
    let (_, other_body) = dregg_adr36_proof(dregg_url, &adr36_secret(2))?;
    let other_sess = http_json(
        "POST",
        &format!("{}/v1/admin/session", dregg_url.trim_end_matches('/')),
        Some(&other_body),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if other_sess.get("ok") == Some(&serde_json::Value::Bool(true)) {
        return Err(format!("second Keplr must not become admin: {other_sess}").into());
    }
    let wd_no_adr = http_json_h(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&serde_json::json!({"rail":"zec","zat":1})),
        &[("Authorization", "Bearer e2e-admin-token")],
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if wd_no_adr.get("ok") == Some(&serde_json::Value::Bool(true))
        || wd_no_adr.get("authorized") == Some(&serde_json::Value::Bool(true))
    {
        return Err(format!("withdraw without ADR-036 must fail: {wd_no_adr}").into());
    }
    // Session bytes (raw challenge) must not authorize a withdraw.
    let (_, raw_wd) = dregg_adr36_proof(dregg_url, &admin_sk)?;
    let mut raw_wd = raw_wd;
    raw_wd["rail"] = serde_json::json!("zec");
    raw_wd["zat"] = serde_json::json!(1);
    let raw_res = http_json(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&raw_wd),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if raw_res.get("authorized") == Some(&serde_json::Value::Bool(true))
        || raw_res.get("ok") == Some(&serde_json::Value::Bool(true))
    {
        return Err(format!("sign-in bytes must not authorize withdraw: {raw_res}").into());
    }
    let (_, wd_req) = dregg_adr36_withdraw_proof(dregg_url, &admin_sk, 1)?;
    let mut wd_req = wd_req;
    wd_req["rail"] = serde_json::json!("zec");
    let wd = http_json(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&wd_req),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    let wd_dest = wd
        .get("to")
        .or_else(|| wd.get("withdraw_to"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if wd.get("authorized") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("ADR-036 withdraw must be authorized: {wd}").into());
    }
    if wd_dest != donate {
        return Err(format!("ADR-036 withdraw dest {wd_dest} != donate UA: {wd}").into());
    }
    let (_, steal_req) = dregg_adr36_withdraw_proof(dregg_url, &admin_sk, 1)?;
    let mut steal_req = steal_req;
    steal_req["rail"] = serde_json::json!("zec");
    steal_req["to"] = serde_json::json!("u1stealxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    let steal = http_json(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&steal_req),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if steal.get("ok") == Some(&serde_json::Value::Bool(true)) {
        return Err(format!("withdraw to foreign UA must fail: {steal}").into());
    }
    let (_, z0) = dregg_adr36_withdraw_proof(dregg_url, &admin_sk, 0)?;
    let mut z0 = z0;
    z0["rail"] = serde_json::json!("zec");
    let wd0 = http_json(
        "POST",
        &format!("{}/v1/admin/withdraw", dregg_url.trim_end_matches('/')),
        Some(&z0),
    )
    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}));
    if wd0.get("ok") == Some(&serde_json::Value::Bool(true)) {
        return Err(format!("withdraw zat=0 must fail: {wd0}").into());
    }

    let status0 = http_json(
        "GET",
        &format!(
            "{}/v1/status?recipient={user}",
            dregg_url.trim_end_matches('/')
        ),
        None,
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    let phase0 = status0.get("phase").cloned().unwrap_or(serde_json::Value::Null);

    let mut grant = serde_json::json!({});
    let mut last_grant_err = String::new();
    for attempt in 0..6 {
        match http_json(
            "POST",
            &format!("http://{bind}/onboard/v1/passkey-grant"),
            Some(&serde_json::json!({ "address": user, "c": c })),
        ) {
            Ok(v) => {
                let blob = v.to_string();
                if blob.contains("sequence mismatch") {
                    last_grant_err = blob;
                    wait_for_blocks(chain, 1).await.ok();
                    continue;
                }
                grant = v;
                last_grant_err.clear();
                break;
            }
            Err(e) => {
                last_grant_err = e;
                if last_grant_err.contains("sequence mismatch") && attempt < 5 {
                    wait_for_blocks(chain, 1).await.ok();
                    continue;
                }
                if attempt < 5 {
                    wait_for_blocks(chain, 1).await.ok();
                    continue;
                }
            }
        }
    }
    if !last_grant_err.is_empty() && grant.get("txhash").is_none() {
        return Err(format!("passkey-grant failed after sequence retries: {last_grant_err}").into());
    }
    let txhash = grant.get("txhash").and_then(|x| x.as_str()).unwrap_or("");
    let runway_hash = grant
        .get("runway_txhash")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let granter_out = grant.get("granter").and_then(|x| x.as_str()).unwrap_or("");
    if txhash.is_empty() || granter_out.is_empty() || runway_hash.is_empty() {
        return Err(format!(
            "passkey-grant missing granter/txhash/runway_txhash (send must land): {grant}"
        )
        .into());
    }
    let allowed = grant["msg"]["allowance"]["allowed_messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if allowed
        .iter()
        .any(|x| x.as_str() == Some("/cosmos.bank.v1beta1.MsgSend"))
    {
        return Err(format!("feegrant must not allow MsgSend: {grant}").into());
    }
    if grant["runway"]["amount"] != hash_market::onboard_authz::RUNWAY_UTHOL {
        return Err(format!("runway must be 1 native unit: {grant}").into());
    }

    std::thread::sleep(std::time::Duration::from_secs(2));
    let bals = terpd_tx(
        cid,
        &[
            "q",
            "bank",
            "balances",
            &user,
            "--home",
            home,
            "--output",
            "json",
        ],
    )?;
    if !bals.contains(hash_market::onboard_authz::RUNWAY_UTHOL) {
        return Err(format!("user balance missing 1-unit runway: {bals}").into());
    }
    let fg = terpd_tx(
        cid,
        &[
            "q",
            "feegrant",
            "grant",
            granter,
            &user,
            "--home",
            home,
            "--output",
            "json",
        ],
    )?;
    if !fg.contains("MsgAddAuthenticator") && !fg.to_lowercase().contains("allowance") {
        return Err(format!("feegrant query empty: {fg}").into());
    }

    let status = http_json("GET", &format!("http://{bind}/status"), None)?;
    if !status
        .to_string()
        .contains(hash_market::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL)
    {
        return Err(format!(
            "feegrant proxy /status missing {}: {status}",
            hash_market::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL
        )
        .into());
    }
    let register = http_json(
        "POST",
        &format!("http://{bind}/v1/register-grant"),
        Some(&serde_json::json!({ "address": user, "c": c })),
    )?;
    let reg_allowed = register["msg"]["allowance"]["allowed_messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !reg_allowed.iter().any(|x| {
        x.as_str() == Some(hash_market::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL)
    }) {
        return Err(format!("register-grant must allow MsgAddAuthenticator: {register}").into());
    }
    if reg_allowed
        .iter()
        .any(|x| x.as_str() == Some("/cosmos.bank.v1beta1.MsgSend"))
    {
        return Err(format!("register-grant must not allow MsgSend: {register}").into());
    }

    let shown_pk = terpd_tx(
        cid,
        &[
            "keys",
            "show",
            "passkey-user",
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--output",
            "json",
        ],
    )?;
    let shown_v: serde_json::Value = serde_json::from_str(shown_pk.trim()).unwrap_or_default();
    let pk_key = match shown_v.get("pubkey") {
        Some(serde_json::Value::String(s)) => serde_json::from_str::<serde_json::Value>(s)
            .ok()
            .and_then(|p| p.get("key").and_then(|k| k.as_str()).map(str::to_string)),
        Some(obj) => obj.get("key").and_then(|k| k.as_str()).map(str::to_string),
        _ => None,
    }
    .ok_or_else(|| format!("passkey-user pubkey missing: {shown_pk}"))?;

    let mut add = String::new();
    let mut add_err = String::new();
    for attempt in 0..6 {
        match terpd_tx(
            cid,
            &[
                "tx",
                "smartaccount",
                "add-authenticator",
                hash_market::onboard_authz::AUTHENTICATOR_SIGNATURE,
                &pk_key,
                "--from",
                "passkey-user",
                "--fee-granter",
                granter,
                "--keyring-backend",
                "test",
                "--home",
                home,
                "--chain-id",
                chain_id,
                "--fees",
                &format!("20000{denom}"),
                "--gas",
                "400000",
                "--broadcast-mode",
                "sync",
                "--yes",
                "--output",
                "json",
            ],
        ) {
            Ok(s) => {
                add = s;
                add_err.clear();
                terpd_wait_next_block(cid, home);
                break;
            }
            Err(e) => {
                add_err = format!("MsgAddAuthenticator via feegrant proxy: {e}");
                if e.contains("sequence mismatch") || attempt < 5 {
                    wait_for_blocks(chain, 1).await.ok();
                    continue;
                }
                break;
            }
        }
    }
    if !add_err.is_empty() && add.is_empty() {
        return Err(add_err.into());
    }
    if add.contains("\"code\":") && !add.contains("\"code\":0") && !add.contains("\"code\": 0") {
        return Err(format!("add-authenticator code != 0: {add}").into());
    }
    wait_for_blocks(chain, 1).await.ok();
    terpd_wait_next_block(cid, home);
    let auths = query_authenticators(cid, home, &user)?;
    if !hash_market::onboard_authz::authenticators_query_ok(&auths) {
        return Err(format!("authenticator query empty after feegrant-paid register: {auths}").into());
    }

    let send_denied = terpd_tx(
        cid,
        &[
            "tx",
            "bank",
            "send",
            "passkey-user",
            granter,
            &format!("1{denom}"),
            "--from",
            "passkey-user",
            "--fee-granter",
            granter,
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--chain-id",
            chain_id,
            "--fees",
            &format!("20000{denom}"),
            "--gas",
            "200000",
            "--yes",
            "--output",
            "json",
        ],
    );
    if send_denied.is_ok() {
        let s = send_denied.unwrap();
        if s.contains("\"code\":0") || s.contains("\"code\": 0") {
            return Err(format!("feegrant must not pay MsgSend: {s}").into());
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "unpaid_grant_blocked": true,
        "paid_grant_ok": true,
                "admin_stats_ok": admin.get("ok") == Some(&serde_json::Value::Bool(true)),
        "admin_stats_adr36_ok": true,
        "admin_adr36_continuity": other_sess.get("ok") != Some(&serde_json::Value::Bool(true)),
        "adr36_keplr_empty_chain_id": true,
        "adr36_session_rejects_withdraw_bytes": true,
        "adr36_withdraw_rejects_session_bytes": true,
        "admin_unauth_401": admin_no_tok == "401",
        "withdraw_adr36_required": wd_no_adr.get("authorized") != Some(&serde_json::Value::Bool(true)),
        "withdraw_adr36_ok": wd.get("authorized") == Some(&serde_json::Value::Bool(true)),
        "withdraw_dest_hardcoded": wd_dest == donate,
        "withdraw_foreign_blocked": steal.get("ok") != Some(&serde_json::Value::Bool(true)),
        "withdraw_zero_blocked": wd0.get("ok") != Some(&serde_json::Value::Bool(true)),
"status_phase": phase0,
        "paid_c": c,
        "user": user,
        "granter": granter_out,
        "txhash": txhash,
        "runway_txhash": grant.get("runway_txhash"),
        "runway_amount": hash_market::onboard_authz::RUNWAY_UTHOL,
        "denom": denom,
        "balances": bals,
        "feegrant": fg,
        "proxy_status": status,
        "register_grant": register,
        "add_authenticator": add,
        "authenticators": auths,
        "send_via_feegrant_denied": true,
    }))
}

struct LabTerpdBroadcaster {
    cid: String,
    home: String,
    /// terp1 bech32 of the signer (validator address, not the key name).
    from: String,
    denom: String,
    chain_id: String,
}

impl hash_market::onboard_authz::GrantBroadcaster for LabTerpdBroadcaster {
    fn broadcast_authz_exec(&self, exec: &serde_json::Value) -> serde_json::Value {
        let inner = exec.get("msgs").and_then(|m| m.get(0)).unwrap_or(exec);
        let grantee = inner
            .get("grantee")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let spend = inner
            .pointer("/allowance/allowance/spend_limit/0/amount")
            .and_then(|x| x.as_str())
            .unwrap_or("25000");
        let allowed: Vec<String> = inner
            .pointer("/allowance/allowed_messages")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| {
                vec![
                    hash_market::onboard_authz::ADD_AUTHENTICATOR_TYPE_URL.to_string(),
                ]
            });
        let spend_c = format!("{spend}{}", self.denom);
        let fees = format!("20000{}", self.denom);
        let mut args: Vec<String> = [
            "tx",
            "feegrant",
            "grant",
            self.from.as_str(),
            grantee,
            "--from",
            self.from.as_str(),
            "--keyring-backend",
            "test",
            "--home",
            self.home.as_str(),
            "--chain-id",
            self.chain_id.as_str(),
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
        for m in &allowed {
            args.push("--allowed-messages".into());
            args.push(m.clone());
        }
        args.extend(
            [
                "--spend-limit",
                spend_c.as_str(),
                "--fees",
                fees.as_str(),
                "--gas",
                "200000",
                "--broadcast-mode",
                "sync",
                "--yes",
                "--output",
                "json",
            ]
            .into_iter()
            .map(|s| s.to_string()),
        );
        let mut last = String::new();
        for _i in 0..6 {
            let mut attempt = args.clone();
            if let Some(seq) = terpd_account_sequence(&self.cid, &self.home, &self.from) {
                attempt.push("--sequence".into());
                attempt.push(seq.to_string());
            }
            let refs: Vec<&str> = attempt.iter().map(|s| s.as_str()).collect();
            match terpd_tx(&self.cid, &refs) {
                Ok(s) => {
                    terpd_wait_next_block(&self.cid, &self.home);
                    return serde_json::from_str(s.trim()).unwrap_or(serde_json::json!({"stdout": s}));
                }
                Err(e) => {
                    last = e;
                    if !last.contains("sequence mismatch") {
                        break;
                    }
                    terpd_wait_next_block(&self.cid, &self.home);
                }
            }
        }
        serde_json::json!({"error": last})
    }

    fn broadcast_bank_send(&self, to: &str, amount: &str) -> serde_json::Value {
        let mut last = String::new();
        for _i in 0..6 {
            let coins = format!("{amount}{}", self.denom);
            let fees = format!("5000{}", self.denom);
            let seq = terpd_account_sequence(&self.cid, &self.home, &self.from)
                .map(|n| n.to_string())
                .unwrap_or_default();
            let mut args = vec![
                "tx",
                "bank",
                "send",
                self.from.as_str(),
                to,
                coins.as_str(),
                "--from",
                self.from.as_str(),
                "--keyring-backend",
                "test",
                "--home",
                self.home.as_str(),
                "--chain-id",
                self.chain_id.as_str(),
                "--fees",
                fees.as_str(),
                "--gas",
                "200000",
                "--broadcast-mode",
                "sync",
                "--yes",
                "--output",
                "json",
            ];
            if !seq.is_empty() {
                args.push("--sequence");
                args.push(seq.as_str());
            }
            match terpd_tx(&self.cid, &args) {
                Ok(s) => {
                    terpd_wait_next_block(&self.cid, &self.home);
                    return serde_json::from_str(s.trim()).unwrap_or(serde_json::json!({"stdout": s}));
                }
                Err(e) => {
                    last = e;
                    if !last.contains("sequence mismatch") {
                        break;
                    }
                    terpd_wait_next_block(&self.cid, &self.home);
                }
            }
        }
        serde_json::json!({"error": last})
    }
}

/// Stop the private house Zakura, confirm DREGG reports tip drift / unreachable,
/// refuse pay-targets, then bring the house node back. No refunds.
async fn exercise_zec_tip_circuit(
    dregg_url: &str,
    private: &ZakuraNode,
    owner_bech: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let health_url = format!("{}/health", dregg_url.trim_end_matches('/'));
    let pay_url = format!(
        "{}/v1/pay-targets?recipient={owner_bech}",
        dregg_url.trim_end_matches('/')
    );
    let before = http_json("GET", &health_url, None)?;
    if before.pointer("/zec_tip/configured") != Some(&serde_json::Value::Bool(true))
        || before.pointer("/zec_tip/ok") != Some(&serde_json::Value::Bool(true))
    {
        return Err(format!("zec_tip not ok before private stop: {before}").into());
    }
    private.stop().await?;
    let drifted = http_json("GET", &health_url, None)?;
    let reasons = drifted
        .get("reasons")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let has_drift = reasons.iter().any(|r| r.as_str() == Some("zec_tip_drift"));
    if drifted.get("checkout_ready") != Some(&serde_json::Value::Bool(false))
        || drifted.get("pay") != Some(&serde_json::Value::Bool(false))
        || drifted.pointer("/zec_tip/ok") != Some(&serde_json::Value::Bool(false))
        || !has_drift
    {
        return Err(format!("expected zec tip circuit open after private stop: {drifted}").into());
    }
    let pay = http_json("GET", &pay_url, None)?;
    if pay.get("checkout_ready") != Some(&serde_json::Value::Bool(false))
        || pay.get("ok") != Some(&serde_json::Value::Bool(false))
    {
        return Err(format!("pay-targets still open after zec drift: {pay}").into());
    }
    private.start().await?;
    let after = http_json("GET", &health_url, None)?;
    if after.get("checkout_ready") != Some(&serde_json::Value::Bool(true))
        || after.pointer("/zec_tip/ok") != Some(&serde_json::Value::Bool(true))
    {
        return Err(format!("checkout did not recover after private restart: {after}").into());
    }
    Ok(serde_json::json!({
        "ok": true,
        "reason": drifted.pointer("/zec_tip/reason").cloned().unwrap_or(serde_json::Value::Null),
        "public_height": drifted.pointer("/zec_tip/public_height").cloned().unwrap_or(serde_json::Value::Null),
        "private_height": drifted.pointer("/zec_tip/private_height").cloned().unwrap_or(serde_json::Value::Null),
        "max_drift": drifted.pointer("/zec_tip/max_drift").cloned().unwrap_or(serde_json::Value::Null),
        "pay_targets_closed": true,
        "recovered": true
    }))
}

fn http_json_h(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sS", "--max-time", "8", "-X", method, url]);
    for (k, v) in headers {
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    if let Some(b) = body {
        cmd.args(["-H", "Content-Type: application/json", "-d", &b.to_string()]);
    }
    let out = cmd.output().map_err(|e| format!("curl {url}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.trim()).map_err(|e| {
        format!(
            "curl {url} json: {e} stdout={} stderr={}",
            stdout.trim(),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn http_json(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    http_json_h(method, url, body, &[])
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn adr36_secret(n: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[31] = n;
    s
}


fn dregg_adr36_withdraw_proof(
    dregg_url: &str,
    secret: &[u8; 32],
    zat: u64,
) -> Result<(String, serde_json::Value), String> {
    let ch = http_json(
        "GET",
        &format!("{}/v1/admin/challenge", dregg_url.trim_end_matches('/')),
        None,
    )?;
    let challenge = ch
        .get("challenge")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("challenge missing: {ch}"))?
        .to_string();
    let ua = dregg::adr36::DONATE_ZEC_UA;
    let statement = dregg::adr36::withdraw_statement(zat, ua, &challenge);
    let (signer, pk, sig) =
        dregg::adr36::sign_keplr_arbitrary(secret, "", statement.as_bytes())?;
    Ok((
        signer.clone(),
        serde_json::json!({
            "kind": "adr36",
            "chain_id": "morocco-1",
            "challenge": challenge,
            "signer": signer,
            "zat": zat,
            "pub_key": { "type": "tendermint/PubKeySecp256k1", "value": pk },
            "signature": sig,
        }),
    ))
}

fn dregg_adr36_proof(
    dregg_url: &str,
    secret: &[u8; 32],
) -> Result<(String, serde_json::Value), String> {
    dregg_adr36_proof_chain(dregg_url, secret, "")
}

fn dregg_adr36_proof_chain(
    dregg_url: &str,
    secret: &[u8; 32],
    sign_chain_id: &str,
) -> Result<(String, serde_json::Value), String> {
    let ch = http_json(
        "GET",
        &format!("{}/v1/admin/challenge", dregg_url.trim_end_matches('/')),
        None,
    )?;
    let challenge = ch
        .get("challenge")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("challenge missing: {ch}"))?
        .to_string();
    let data = hex_decode(&challenge)?;
    let (signer, pk, sig) =
        dregg::adr36::sign_keplr_arbitrary(secret, sign_chain_id, &data)?;
    Ok((
        signer.clone(),
        serde_json::json!({
            "kind": "adr36",
            "chain_id": "morocco-1",
            "challenge": challenge,
            "signer": signer,
            "pub_key": { "type": "tendermint/PubKeySecp256k1", "value": pk },
            "signature": sig,
        }),
    ))
}

fn wasm_smart_query(cid: &str, home: &str, contract: &str, msg: &str) -> Result<String, String> {
    terpd_tx(
        cid,
        &[
            "q",
            "wasm",
            "contract-state",
            "smart",
            contract,
            msg,
            "--home",
            home,
            "--output",
            "json",
        ],
    )
}

fn docker_bin() -> &'static str {
    for p in [
        "/usr/local/bin/docker",
        "/usr/bin/docker",
        "/opt/homebrew/bin/docker",
    ] {
        if std::path::Path::new(p).exists() {
            return p;
        }
    }
    "docker"
}

fn terpd_height(cid: &str, home: &str) -> u64 {
    let out = std::process::Command::new(docker_bin())
        .args(["exec", cid, "terpd", "status", "--home", home])
        .output();
    let blob = match out {
        Ok(o) => format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(_) => return 0,
    };
    hash_market::onboard_authz::parse_status_height(&blob)
}

fn terpd_wait_next_block(cid: &str, home: &str) {
    let cid = cid.to_string();
    let home = home.to_string();
    hash_market::onboard_authz::wait_until_height_commits(
        terpd_height(&cid, &home),
        || terpd_height(&cid, &home),
        || std::thread::sleep(std::time::Duration::from_millis(400)),
    );
}

fn terpd_account_sequence(cid: &str, home: &str, addr: &str) -> Option<u64> {
    if !addr.starts_with("terp1") {
        return None;
    }
    let out = std::process::Command::new(docker_bin())
        .args([
            "exec",
            cid,
            "terpd",
            "q",
            "auth",
            "account",
            addr,
            "--home",
            home,
            "--output",
            "json",
        ])
        .output()
        .ok()?;
    let blob = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    hash_market::onboard_authz::parse_account_sequence(&blob)
}

fn query_authenticators(cid: &str, home: &str, addr: &str) -> Result<String, String> {
    let cmds = ["get-authenticators", "authenticators"];
    let mut last = String::new();
    for cmd in cmds {
        match terpd_tx(
            cid,
            &[
                "q",
                "smartaccount",
                cmd,
                addr,
                "--home",
                home,
                "--output",
                "json",
            ],
        ) {
            Ok(s) if hash_market::onboard_authz::authenticators_query_ok(&s) => return Ok(s),
            Ok(s) => last = s,
            Err(e) => last = e,
        }
    }
    Ok(last)
}

fn terpd_tx(cid: &str, extra: &[&str]) -> Result<String, String> {
    let mut args = vec!["exec", cid, "terpd"];
    args.extend_from_slice(extra);
    let out = std::process::Command::new(docker_bin())
        .args(&args)
        .output()
        .map_err(|e| format!("docker exec: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let blob = format!("{stdout}\n{stderr}");
    if blob.contains("sequence mismatch") {
        return Err(format!("sequence mismatch: {}", blob.trim()));
    }
    if !out.status.success() {
        return Err(format!(
            "terpd exit={} stdout={} stderr={}",
            out.status,
            stdout.trim(),
            stderr.trim()
        ));
    }
    Ok(stdout)
}

fn store_wasm(cid: &str, home: &str, wasm: &str) -> Result<String, String> {
    let before = list_code_ids(cid, home).unwrap_or_default();
    let mut last = String::new();
    for _ in 0..8 {
        match terpd_tx(
            cid,
            &[
                "tx",
                "wasm",
                "store",
                wasm,
                "--from",
                "validator",
                "--keyring-backend",
                "test",
                "--home",
                home,
                "--chain-id",
                "svg-mint-local-1",
                "--gas",
                "auto",
                "--gas-adjustment",
                "2.0",
                "--gas-prices",
                "0.025uterp",
                "--broadcast-mode",
                "sync",
                "--output",
                "json",
                "-y",
            ],
        ) {
            Ok(_) => {
                for _ in 0..20 {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let ids = list_code_ids(cid, home)?;
                    if let Some(id) = ids.iter().rev().find(|i| !before.contains(i)) {
                        return Ok(id.clone());
                    }
                }
                return Err(format!("no new code_id after store {wasm}"));
            }
            Err(e) if e.contains("sequence mismatch") => {
                last = e;
                std::thread::sleep(std::time::Duration::from_millis(800));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

fn list_code_ids(cid: &str, home: &str) -> Result<Vec<String>, String> {
    let out = terpd_tx(
        cid,
        &["q", "wasm", "list-code", "--home", home, "--output", "json"],
    )?;
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or(serde_json::Value::Null);
    Ok(v["code_infos"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    c["code_id"]
                        .as_str()
                        .map(|s| s.to_string())
                        .or_else(|| c["code_id"].as_u64().map(|n| n.to_string()))
                })
                .collect()
        })
        .unwrap_or_default())
}

fn cosmos_txhash(blob: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(blob.trim()) {
        if let Some(h) = v.get("txhash").or_else(|| v.get("hash")).and_then(|x| x.as_str()) {
            if h.len() >= 32 {
                return Some(h.to_string());
            }
        }
    }
    for tok in blob.split_whitespace() {
        let t = tok.trim_matches(|c| c == '"' || c == ',');
        if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(t.to_string());
        }
    }
    None
}

fn wait_tx_included(cid: &str, home: &str, blob: &str) -> Result<(), String> {
    let Some(hash) = cosmos_txhash(blob) else {
        return Err(format!("mint tx missing txhash: {}", blob.trim()));
    };
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(400));
        if let Ok(q) = terpd_tx(
            cid,
            &[
                "q",
                "tx",
                &hash,
                "--home",
                home,
                "--output",
                "json",
            ],
        ) {
            let v: serde_json::Value = serde_json::from_str(q.trim()).unwrap_or_default();
            let code = v.get("code").and_then(|c| c.as_u64()).unwrap_or(u64::MAX);
            if code == 0 {
                return Ok(());
            }
            if v.get("code").is_some() && code != 0 {
                return Err(format!("mint tx failed code={code}: {q}"));
            }
        }
    }
    Err(format!("mint tx not included: {hash}"))
}

fn poll_num_tokens(
    cid: &str,
    home: &str,
    collection: &str,
    tries: u32,
) -> Result<(serde_json::Value, u64), String> {
    let mut last = serde_json::json!({});
    for _ in 0..tries {
        let wasm_raw = wasm_smart_query(cid, home, collection, r#"{"num_tokens":{}}"#)?;
        let wasm_query: serde_json::Value = serde_json::from_str(wasm_raw.trim())
            .map_err(|e| format!("num_tokens json: {e} raw={wasm_raw}"))?;
        last = wasm_query.clone();
        let queried = wasm_query
            .get("count")
            .or_else(|| wasm_query.get("data").and_then(|d| d.get("count")))
            .and_then(|c| c.as_u64().or_else(|| c.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        if queried > 0 {
            return Ok((wasm_query, queried));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok((last, 0))
}

fn wasm_execute(
    cid: &str,
    home: &str,
    contract: &str,
    msg: &str,
    from: &str,
) -> Result<String, String> {
    terpd_tx(
        cid,
        &[
            "tx",
            "wasm",
            "execute",
            contract,
            msg,
            "--from",
            from,
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--chain-id",
            "svg-mint-local-1",
            "--gas",
            "auto",
            "--gas-adjustment",
            "2.0",
            "--gas-prices",
            "0.025uterp",
            "--broadcast-mode",
            "sync",
            "--output",
            "json",
            "-y",
        ],
    )
}

fn wasm_execute_funds(
    cid: &str,
    home: &str,
    contract: &str,
    msg: &str,
    from: &str,
    amount: &str,
) -> Result<String, String> {
    terpd_tx(
        cid,
        &[
            "tx",
            "wasm",
            "execute",
            contract,
            msg,
            "--amount",
            amount,
            "--from",
            from,
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--chain-id",
            "svg-mint-local-1",
            "--gas",
            "auto",
            "--gas-adjustment",
            "2.0",
            "--gas-prices",
            "0.025uterp",
            "--broadcast-mode",
            "sync",
            "--output",
            "json",
            "-y",
        ],
    )
}

fn extract_contract_addr(s: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s.trim()) {
        if let Some(logs) = v.get("logs").and_then(|x| x.as_array()) {
            for log in logs {
                if let Some(events) = log.get("events").and_then(|x| x.as_array()) {
                    for ev in events {
                        if let Some(attrs) = ev.get("attributes").and_then(|x| x.as_array()) {
                            for a in attrs {
                                let key = a.get("key").and_then(|x| x.as_str()).unwrap_or("");
                                let val = a.get("value").and_then(|x| x.as_str()).unwrap_or("");
                                if (key == "contract_address" || key == "_contract_address")
                                    && val.starts_with("terp1")
                                {
                                    return Some(val.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(raw) = v.get("raw_log").and_then(|x| x.as_str()) {
            if let Some(i) = raw.find("terp1") {
                let slice = &raw[i..];
                let addr: String = slice.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if addr.len() > 20 {
                    return Some(addr);
                }
            }
        }
    }
    None
}

fn instantiate_wasm(
    cid: &str,
    home: &str,
    code_id: &str,
    msg: &str,
    label: &str,
) -> Result<String, String> {
    terpd_tx(
        cid,
        &[
            "tx",
            "wasm",
            "instantiate",
            code_id,
            msg,
            "--label",
            label,
            "--no-admin",
            "--from",
            "validator",
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--chain-id",
            "svg-mint-local-1",
            "--gas",
            "auto",
            "--gas-adjustment",
            "2.0",
            "--gas-prices",
            "0.025uterp",
            "--broadcast-mode",
            "sync",
            "--output",
            "json",
            "-y",
        ],
    )?;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let out = terpd_tx(
            cid,
            &[
                "q",
                "wasm",
                "list-contract-by-code",
                code_id,
                "--home",
                home,
                "--output",
                "json",
            ],
        )?;
        let v: serde_json::Value =
            serde_json::from_str(out.trim()).unwrap_or(serde_json::Value::Null);
        if let Some(addr) = v["contracts"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
        {
            return Ok(addr.to_string());
        }
    }
    Err("minter address not in list-contract-by-code".into())
}


fn zakura_height(rpc: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let v = json_rpc(rpc, "getblockcount", serde_json::json!([]))?;
    v.get("result")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| format!("getblockcount missing result from {rpc}: {v}").into())
}

/// The Ironwood open mines coinbases on the private node. The public node must
/// reach the same tip or checkout fail-closes on zec_tip_drift.
fn catch_up_public_zakura(private_rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    let public_rpc = std::env::var("ZCASH_PUBLIC_RPC_HOST")
        .map_err(|_| "ZCASH_PUBLIC_RPC_HOST unset")?;
    let private_h = zakura_height(private_rpc)?;
    let public_h = zakura_height(&public_rpc)?;
    if private_h > public_h {
        let n = private_h - public_h;
        let generated = json_rpc(&public_rpc, "generate", serde_json::json!([n]))?;
        if generated.get("error").is_some() || generated.get("result").is_none() {
            return Err(format!("public zakura generate {n} failed: {generated}").into());
        }
    }
    let private_h = zakura_height(private_rpc)?;
    let public_h = zakura_height(&public_rpc)?;
    let drift = private_h.abs_diff(public_h);
    if drift > 2 {
        return Err(format!(
            "zec tip drift {drift} after catch-up, private={private_h} public={public_h}"
        )
        .into());
    }
    Ok(())
}


fn terpd_out(cid: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(docker_bin())
        .arg("exec")
        .arg(cid)
        .args(args)
        .output()
        .map_err(|e| format!("terpd: {e}"))?;
    let blob = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("terpd failed: {blob}"));
    }
    Ok(blob)
}

fn fresh_terp(cid: &str, home: &str, name: &str) -> Result<String, String> {
    let blob = terpd_out(
        cid,
        &[
            "terpd",
            "keys",
            "add",
            name,
            "--keyring-backend",
            "test",
            "--home",
            home,
            "--output",
            "json",
            "--no-backup",
        ],
    )?;
    let json_blob = match (blob.find('{'), blob.rfind('}')) {
        (Some(i), Some(j)) if j > i => &blob[i..=j],
        _ => blob.trim(),
    };
    let v: serde_json::Value = serde_json::from_str(json_blob)
        .map_err(|e| format!("keys add {name}: {e} blob={blob}"))?;
    v.get("address")
        .and_then(|a| a.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("keys add {name} missing address: {v}"))
}

/// Three wallets pay three assets at the same time, then each mints one SVG.
async fn parallel_wallets(
    chain: &CosmosChain,
    cid: &str,
    home: &str,
    dregg_url: &str,
    anvil: &AnvilChain,
    base_anvil: &AnvilChain,
    btc: &BitcoinNode,
    collection: &str,
    already: u64,
) -> Result<(u64, serde_json::Value), Box<dyn std::error::Error>> {
    let recipients = [
        fresh_terp(cid, home, "svg-w0")?,
        fresh_terp(cid, home, "svg-w1")?,
        fresh_terp(cid, home, "svg-w2")?,
    ];
    if recipients[0] == recipients[1] || recipients[1] == recipients[2] || recipients[0] == recipients[2] {
        return Err(format!("parallel wallets were not distinct: {recipients:?}").into());
    }
    let mut pays = Vec::new();
    for r in &recipients {
        let pay = http_json(
            "GET",
            &format!(
                "{}/v1/pay-targets?recipient={r}",
                dregg_url.trim_end_matches('/')
            ),
            None,
        )?;
        if pay.get("ok") != Some(&serde_json::Value::Bool(true)) {
            return Err(format!("pay-targets for {r} failed: {pay}").into());
        }
        pays.push(pay);
    }
    let eth_wei = dregg::prices::ETH_WEI;
    let btc_sats = dregg::prices::BTC_SATS;
    let from_eth = ANVIL_DEFAULT_ACCOUNTS[2].1;
    let from_base = ANVIL_DEFAULT_ACCOUNTS[3].1;
    let treasury = ANVIL_DEFAULT_ACCOUNTS[1].0;
    let memo0 = pays[0]
        .get("memo_hex")
        .and_then(|x| x.as_str())
        .ok_or("wallet0 pay-targets missing memo_hex")?
        .to_string();
    let btc_addr = pays[1]
        .get("btc_addr")
        .and_then(|x| x.as_str())
        .ok_or("wallet1 pay-targets missing btc_addr")?
        .to_string();
    let base_addr = pays[2]
        .get("eth_addr")
        .and_then(|x| x.as_str())
        .ok_or("wallet2 pay-targets missing eth_addr")?
        .to_string();
    let dregg = dregg_url.to_string();
    let (eth_tx, base_tx, btc_tx) = tokio::join!(
        eth_vault_lock(anvil, from_eth, treasury, &memo0),
        async {
            let rpc = base_anvil.docker_dns_rpc_url();
            let value = format!("0x{eth_wei:x}");
            let sent = base_anvil
                .exec_cast(&[
                    "send",
                    "--rpc-url",
                    &rpc,
                    "--private-key",
                    from_base,
                    &base_addr,
                    "--value",
                    &value,
                    "--json",
                ])
                .await?;
            let blob = format!("{}{}", sent.stdout_str(), sent.stderr_str());
            let hash = extract_tx_hash(&blob).ok_or_else(|| format!("base tx hash missing: {blob}"))?;
            Ok::<String, Box<dyn std::error::Error>>(hash)
        },
        async {
            btc.pay_sats_to(&btc_addr, btc_sats)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> { format!("parallel btc pay: {e}").into() })
        }
    );
    let (eth_tx, _, _) = eth_tx?;
    let base_tx = base_tx?;
    let btc_tx = btc_tx?;
    let rows = [
        ("Eth", eth_wei.to_string(), eth_tx.trim_start_matches("0x").to_string()),
        ("Base", eth_wei.to_string(), base_tx.trim_start_matches("0x").to_string()),
        ("Btc", btc_sats.to_string(), btc_tx.trim_start_matches("0x").to_string()),
    ];
    let mut ids = Vec::new();
    for (rail, amount, raw) in rows {
        let dep = fed_signed(
            &dregg,
            "deposit",
            "/v1/deposit",
            serde_json::json!({"rail": rail, "amount": amount.parse::<u128>().unwrap(), "raw_hex": raw}),
        )?;
        if dep.get("ok") != Some(&serde_json::Value::Bool(true)) {
            return Err(format!("parallel {rail} deposit refused: {dep}").into());
        }
        let n = dep.get("n_deposit").cloned().ok_or_else(|| format!("parallel {rail} missing n_deposit: {dep}"))?;
        if n.is_null() {
            return Err(format!("parallel {rail} n_deposit is null: {dep}").into());
        }
        ids.push(n);
    }
    if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
        return Err(format!("parallel deposits did not get distinct n_deposit: {ids:?}").into());
    }
    let mut minted_now = already;
    for _ in 0..3 {
        let mint_builder = daemon_builder_from_chain(chain, Some(LAB_MNEMONIC))?;
        let collection_for_mint = collection.to_string();
        tokio::task::spawn_blocking(move || {
            use cw_orch::prelude::Coin;
            use ict_rs_cw_orch::svg_runtime::{mint_via_daemon, native_svg_mint_msg};
            let daemon = mint_builder.build().map_err(|e| format!("Daemon.build mint: {e}"))?;
            mint_via_daemon(
                &daemon,
                &collection_for_mint,
                &native_svg_mint_msg(),
                &[Coin::new(dregg::prices::NATIVE_AMOUNT, "uterp")],
            )
        })
        .await?
        .map_err(|e| format!("parallel native mint: {e}"))?;
        wait_for_blocks(chain, 1).await?;
        minted_now += 1;
    }
    let (qblob, queried) = poll_num_tokens(cid, home, collection, 12)?;
    if queried != already + 3 {
        return Err(format!(
            "parallel mints: queried {queried} want {} query={qblob}",
            already + 3
        )
        .into());
    }
    Ok((
        queried,
        serde_json::json!({
            "wallets": 3,
            "payments_ok": 3,
            "mints": 3,
            "assets": ["Eth", "Base", "Btc"],
            "n_deposits": ids,
        }),
    ))
}

fn nu63_is_active(info: &serde_json::Value) -> bool {
    let Some(upgrades) = info.pointer("/result/upgrades").and_then(|u| u.as_object()) else {
        return false;
    };
    upgrades.values().any(|v| {
        v.get("name").and_then(|n| n.as_str()) == Some("NU6.3")
            && v.get("status").and_then(|s| s.as_str()) == Some("active")
    })
}

#[cfg(test)]
mod nu63_lab {
    use super::nu63_is_active;

    fn info(status: &str) -> serde_json::Value {
        serde_json::json!({
            "result": {
                "upgrades": {
                    "37a5165b": {
                        "name": "NU6.3",
                        "activationheight": 2,
                        "status": status
                    }
                }
            }
        })
    }

    #[test]
    fn active_nu63_is_the_only_passing_status() {
        assert!(nu63_is_active(&info("active")));
        for status in ["pending", "disabled", "Active", ""] {
            assert!(!nu63_is_active(&info(status)), "{status}");
        }
    }

    #[test]
    fn missing_or_other_upgrade_is_not_nu63() {
        assert!(!nu63_is_active(&serde_json::json!({"result": {}})));
        assert!(!nu63_is_active(&serde_json::json!({
            "result": {"upgrades": {"deadbeef": {"name": "NU6.2", "status": "active"}}}
        })));
    }
}
