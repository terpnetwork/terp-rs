//! Touch Vinyls AAA_TVL_00/001/002: point token_uri at the blossom copies on s3.terp.network.
//!
//! One tx from the cw721 creator (terp1fxcc..) with 3 `update_nft_info` msgs. URLs are
//! https://s3.terp.network/static/svg-collections/cid/<sha256 of the metadata JSON>; each JSON's `image` is
//! https://s3.terp.network/static/svg-collections/cid/<sha256 of the PNG>. No Pinata, no ipfs.terp.network.
//!
//! DRY RUN BY DEFAULT: read-only checks over the LCD (creator ownership, token ids), a blob
//! check (GET each URL, sha256 must match; WARN and continue in dry run if not uploaded yet),
//! and a gas simulation (unsigned tx, signer = creator). `--broadcast` plus
//! AAA_TVL_METADATA_BROADCAST=1 signs through a cw-orch-kms session (Zigner approves a grant
//! scoped to the cw721 `update_nft_info` key, 3 calls), and refuses unless every blob verifies.
//!
//! Build: cargo build -p terp-orch --features gov --bin aaa_tvl_metadata
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use cosmos_sdk_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, TxRaw, mode_info,
};
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
use cosmos_sdk_proto::prost::Message;
use cosmwasm_std::Binary;
use serde_json::{Value, json};
use std::path::PathBuf;

const CHAIN_ID: &str = "morocco-1";
const CW721: &str = "terp1vansx03g9sxntv86y2f38zu28ddyjc36ful8a3fapdrl7xrmtsmq0jv4jv";
const CREATOR: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";
/// Default public read pattern: <base><sha256>.<ext> (nginx /bud/<sha>.<png|json> -> hash-market /content/<sha>).
const BUD: &str = "https://s3.terp.network/bud/";
/// (token id, metadata JSON sha256, image PNG sha256)
const TOKENS: [(&str, &str, &str); 3] = [
    (
        "AAA_TVL_00",
        "2438120b87c3e06d496a967ab278b7b6c66e0b690a15ad05b7afa043c23ae164",
        "c86dd4429db6fc871833fae074db20945135c9100b29b5772bab362ace808d0f",
    ),
    (
        "AAA_TVL_001",
        "a9665ba710fddb3f6024f850034200beb21addc361d6f50a228a6ffae5f59b45",
        "50126f4a42357dc11d41e0648ca40827f0b8301162e325f4b0d7910d613ba145",
    ),
    (
        "AAA_TVL_002",
        "497de7e5ee8166abdad6fbff861752b4f99c6f4d84c36a6dd44a2b4a61914d16",
        "0b59121d5cbdfe9159d1b15ff91a5c9b5c35745243462a4f1ec2bb4d9df37bea",
    ),
];
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";

#[derive(Parser)]
#[command(about = "Touch Vinyls token_uri -> s3.terp.network blossom (dry run by default)")]
struct Cli {
    /// Sign and broadcast through cw-orch-kms. Also needs AAA_TVL_METADATA_BROADCAST=1.
    #[arg(long)]
    broadcast: bool,
    /// cw721 creator (Zigner). Signs the 3 update_nft_info msgs.
    #[arg(long, default_value = CREATOR)]
    granter: String,
    #[arg(long, env = "TERP_LCD", default_value = "https://api.terp.network")]
    lcd: String,
    /// gRPC for --broadcast. Default is the LAN node.
    #[arg(long, env = "GRPC_URL", default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    #[arg(long, default_value = "aaa-tvl-metadata-out")]
    out: PathBuf,
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
    /// Public blob base URL. token_uri = <base><meta_sha>.json, image = <base><img_sha>.png.
    #[arg(long, default_value = BUD)]
    base_url: String,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.out)?;
    println!(
        "mode: {}",
        if cli.broadcast {
            "BROADCAST (cw-orch-kms)"
        } else {
            "DRY RUN (simulate only, nothing signed)"
        }
    );
    let v = validate(&cli)?;
    let base = if cli.base_url.ends_with('/') {
        cli.base_url.clone()
    } else {
        format!("{}/", cli.base_url)
    };
    println!("base url: {base}");
    let blobs_ok = check_blobs(&base)?;
    let (anys, msg_json) = update_msgs(&cli.granter, &base)?;
    println!(
        "== predicted msgs\n{}",
        serde_json::to_string_pretty(&msg_json)?
    );
    if !cli.broadcast {
        if let Err(e) = grpc_preflight(&cli) {
            println!("[WARN] {e:#}");
        }
        let gas = simulate(&cli, &anys)?;
        println!(
            "simulate update_nft_info x3 (sender {}): gas_used={gas} (fee @ {} uthiol/gas x1.3 ~= {} uthiol)",
            cli.granter,
            cli.gas_price,
            fee_for(gas, cli.gas_price)
        );
        let report = json!({"_NOT_SIGNED": "dry run", "base_url": base, "validation": v, "blobs_verified": blobs_ok, "msgs": msg_json, "gas_used": gas});
        let path = cli.out.join("dry-run.json");
        std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
        println!("wrote {}\nDRY RUN done.", path.display());
        return Ok(());
    }
    if !blobs_ok {
        bail!("refusing to sign: not every blob on {base} verifies (upload first)");
    }
    broadcast(&cli, anys)
}

fn update_msgs(sender: &str, base: &str) -> Result<(Vec<Any>, Value)> {
    let mut anys = vec![];
    let mut js = vec![];
    for (t, meta, _) in TOKENS {
        let exec = json!({"update_nft_info": {"token_id": t, "token_uri": format!("{base}{meta}.json"), "extension": null}});
        let m = MsgExecuteContract {
            sender: sender.into(),
            contract: CW721.into(),
            msg: serde_json::to_vec(&exec)?,
            funds: vec![],
        };
        anys.push(any(T_EXEC, &m));
        js.push(
            json!({"@type": T_EXEC, "sender": sender, "contract": CW721, "funds": [], "msg": exec}),
        );
    }
    Ok((anys, Value::Array(js)))
}

fn validate(cli: &Cli) -> Result<Value> {
    let lcd = cli.lcd.trim_end_matches('/');
    let mut fails = vec![];
    let mut ck = |ok: bool, what: String| {
        println!("[{}] {what}", if ok { " OK " } else { "FAIL" });
        if !ok {
            fails.push(what)
        }
    };
    let latest = get(lcd, "/cosmos/base/tendermint/v1beta1/blocks/latest")?;
    let chain = latest["block"]["header"]["chain_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    ck(chain == CHAIN_ID, format!("chain-id {chain}"));
    let own = smart(lcd, CW721, &json!({"get_creator_ownership": {}}))?["data"].clone();
    ck(
        own["owner"] == cli.granter.as_str(),
        format!("cw721 creator {} (signer {})", own["owner"], cli.granter),
    );
    ck(
        own["pending_owner"].is_null(),
        format!("no pending creator transfer ({})", own["pending_owner"]),
    );
    let mut uris = json!({});
    for (t, _, _) in TOKENS {
        let info = smart(lcd, CW721, &json!({"nft_info": {"token_id": t}}))?["data"].clone();
        let uri = info["token_uri"].as_str().unwrap_or("").to_string();
        ck(
            !uri.is_empty() || info.is_object(),
            format!("{t} exists, current token_uri {uri}"),
        );
        uris[t] = json!(uri);
    }
    if !fails.is_empty() {
        bail!("on-chain validation failed: {fails:?}");
    }
    Ok(json!({"chain_id": chain, "creator": own, "current_token_uris": uris}))
}

/// GET each image and metadata blob; sha256 must equal its path. Metadata `image` must point at its PNG blob.
fn check_blobs(base: &str) -> Result<bool> {
    let mut all = true;
    for (t, meta, img) in TOKENS {
        for (kind, sha, ext) in [("image", img, "png"), ("metadata", meta, "json")] {
            let url = format!("{base}{sha}.{ext}");
            match fetch(&url) {
                Ok(bytes) => {
                    let got = sha256_hex(&bytes)?;
                    let ok = got == sha;
                    println!(
                        "[{}] {t} {kind} {url} ({} bytes, sha256 {got})",
                        if ok { " OK " } else { "FAIL" },
                        bytes.len()
                    );
                    all &= ok;
                    if ok && kind == "metadata" {
                        let m: Value = serde_json::from_slice(&bytes).context("metadata JSON")?;
                        let want = format!("{base}{img}.png");
                        let good = m["image"] == want.as_str();
                        println!(
                            "[{}] {t} metadata image -> {}",
                            if good { " OK " } else { "FAIL" },
                            m["image"]
                        );
                        all &= good;
                    }
                }
                Err(e) => {
                    println!("[WARN] {t} {kind} {url}: {e} (not uploaded yet?)");
                    all = false;
                }
            }
        }
    }
    Ok(all)
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    let out = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "60", url])
        .output()?;
    if !out.status.success() {
        bail!("HTTP fetch failed (curl exit {:?})", out.status.code());
    }
    Ok(out.stdout)
}
fn sha256_hex(b: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut c = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    c.stdin.take().unwrap().write_all(b)?;
    let o = c.wait_with_output()?;
    Ok(String::from_utf8(o.stdout)?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string())
}

fn curl(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "25"])
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!("curl failed: {}", args.last().unwrap_or(&""));
    }
    Ok(String::from_utf8(out.stdout)?)
}
fn get(lcd: &str, path: &str) -> Result<Value> {
    Ok(serde_json::from_str(&curl(&[&format!("{lcd}{path}")])?)?)
}
fn smart(lcd: &str, contract: &str, q: &Value) -> Result<Value> {
    let b64 = Binary::from(serde_json::to_vec(q)?).to_base64();
    get(
        lcd,
        &format!("/cosmwasm/wasm/v1/contract/{contract}/smart/{b64}"),
    )
    .with_context(|| format!("query {contract} {q}"))
}

fn simulate(cli: &Cli, msgs: &[Any]) -> Result<u64> {
    let lcd = cli.lcd.trim_end_matches('/');
    let acct = get(
        lcd,
        &format!("/cosmos/auth/v1beta1/accounts/{}", cli.granter),
    )?;
    let seq: u64 = find_key(&acct["account"], "sequence")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("signer account has no sequence"))?
        .parse()?;
    let body = TxBody {
        messages: msgs.to_vec(),
        memo: "aaa-tvl-metadata dry run".into(),
        ..Default::default()
    };
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: None,
            sequence: seq,
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
            }),
        }],
        fee: Some(Fee {
            amount: vec![ProtoCoin {
                denom: "uthiol".into(),
                amount: "0".into(),
            }],
            gas_limit: 0,
            payer: String::new(),
            granter: String::new(),
        }),
        ..Default::default()
    };
    let raw = TxRaw {
        body_bytes: body.encode_to_vec(),
        auth_info_bytes: auth.encode_to_vec(),
        signatures: vec![vec![]],
    };
    let req = json!({"tx_bytes": Binary::from(raw.encode_to_vec()).to_base64()}).to_string();
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            &req,
            &format!("{lcd}/cosmos/tx/v1beta1/simulate"),
        ])
        .output()?;
    let v: Value = serde_json::from_slice(&out.stdout).context("simulate response")?;
    match v["gas_info"]["gas_used"].as_str() {
        Some(g) => Ok(g.parse()?),
        None => bail!(
            "simulate failed: {}",
            v["message"].as_str().unwrap_or(&v.to_string())
        ),
    }
}
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}
fn fee_for(gas: u64, price: f64) -> u64 {
    ((gas as f64) * 1.3 * price).ceil() as u64
}
fn any<M: Message>(type_url: &str, m: &M) -> Any {
    Any {
        type_url: type_url.into(),
        value: m.encode_to_vec(),
    }
}

fn broadcast(cli: &Cli, msgs: Vec<Any>) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    use cw_orch_kms::progress::Status;
    if std::env::var("AAA_TVL_METADATA_BROADCAST").ok().as_deref() != Some("1") {
        bail!("refusing to sign: --broadcast also needs AAA_TVL_METADATA_BROADCAST=1");
    }
    let chain = ChainInfoOwned {
        chain_id: CHAIN_ID.into(),
        gas_denom: "uthiol".into(),
        gas_price: cli.gas_price,
        grpc_urls: vec![cli.grpc_url.clone()],
        rpc_urls: vec![],
        lcd_url: Some(cli.lcd.clone()),
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: "terp".into(),
            pub_address_prefix: "terp".into(),
            coin_type: 118,
        },
        kind: ChainKind::Mainnet,
    };
    // SAFETY: single-threaded here, before cw-orch reads it.
    unsafe { std::env::set_var("CW_ORCH_WALLET_BALANCE_ASSERTION", "false") };
    grpc_preflight(cli)?;
    let mut builder = DaemonBuilder::new(chain);
    builder.load_network(false);
    cw_orch_kms::ensure_ephemeral_mnemonic(ChainKind::Mainnet).map_err(|e| anyhow!(e))?;
    builder
        .build()
        .map_err(|e| anyhow!("pre-flight Daemon build on {}: {e}", cli.grpc_url))?;
    let page = cw_orch_kms::page_addr();
    println!(
        "sign the cw-orch-granter challenge on the approval page (port {})",
        page.port()
    );
    let granter = cw_orch_kms::capture_granter_for_chain_opts_sync(
        page,
        cli.account_index,
        "aaa-tvl-metadata",
        CHAIN_ID,
        cli.ttl_secs,
        Some(&cli.granter),
    )
    .map_err(|e| anyhow!(e))?;
    if granter != cli.granter {
        bail!("Zigner granter {granter} != expected {}", cli.granter);
    }
    let mut session = cw_orch_kms::ApprovalSession::open(&granter)
        .allow_execute(cw_orch_kms::ScopedCall {
            address: CW721.into(),
            keys: vec!["update_nft_info".into()],
            calls: 3,
            funds: vec![],
        })
        .map_err(|e| anyhow!(e))?;
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cw_orch_kms::planned_spend_limit(2);
    let actions =
        vec![cw_orch_kms::SigningRequest::new("session-key").body("aaa-tvl-metadata".to_string())];
    let run = cw_orch_kms::enable_session(&builder, session, actions).map_err(|e| anyhow!(e))?;
    let daemon = run.daemon.clone();
    let label = "update_nft_info";
    let result = (|| -> Result<()> {
        let memo = "aaa-tvl-metadata s3.terp.network".to_string();
        let sim = vec![any(
            T_AUTHZ_EXEC,
            &cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec {
                grantee: run.grantee().into(),
                msgs: msgs.clone(),
            },
        )];
        run.step(label, Status::Running, None, None);
        let (gas, fee) = daemon
            .rt_handle
            .block_on(daemon.sender().simulate(sim, Some(&memo)))
            .map_err(|e| anyhow!(e))?;
        println!("simulate {label}: gas={gas} fee={fee}");
        let resp = daemon
            .rt_handle
            .block_on(daemon.sender().commit_tx_any(msgs, Some(&memo)))
            .map_err(|e| anyhow!(e))?;
        println!(
            "BROADCAST {label} txhash: {} code {}",
            resp.txhash, resp.code
        );
        if resp.code != 0 {
            run.step(label, Status::Error, Some(&resp.txhash), None);
            bail!("code {}", resp.code);
        }
        run.step(label, Status::Confirmed, Some(&resp.txhash), None);
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed (revoked)");
    result
}

fn grpc_preflight(cli: &Cli) -> Result<()> {
    use cw_orch::daemon::{GrpcChannel, RUNTIME, queriers::Node};
    let url = cli.grpc_url.clone();
    let h = RUNTIME.block_on(async {
        let ch = cw_orch::tokio::time::timeout(
            std::time::Duration::from_secs(20),
            GrpcChannel::connect(&[url.clone()], CHAIN_ID),
        )
        .await
        .map_err(|_| anyhow!("gRPC {url}: timed out"))?
        .map_err(|e| anyhow!("gRPC {url}: {e}"))?;
        Node::new_async(ch)
            ._block_height()
            .await
            .map_err(|e| anyhow!("gRPC {url} height: {e}"))
    })?;
    println!("[ OK ] grpc {url} {CHAIN_ID} height {h}");
    Ok(())
}
