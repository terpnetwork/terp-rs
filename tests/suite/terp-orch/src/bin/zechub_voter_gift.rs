//! ZecHub DAO voter gift on morocco-1. Each voter gets 1 ZEC SVG + 1 Penumbra SVG (already
//! minted, held by --sender) + 7.10 THIOL + 4.20 TERP. Signed only through cw-orch-kms.
//!
//! Input: votes.csv (DAO DAO export: "Timestamp","Voter","Voting Power","Vote","Rationale").
//! Every unique juno1 voter is re-encoded as terp1 (bech32 prefix swap; coin type 118 on both
//! chains, so the same key controls both). 32-byte contract addresses are skipped.
//!
//! No mint. Pre-flight lists `tokens{owner: sender}` on both collections, requires >= N each,
//! and assigns token ids lowest-first in CSV order. Per recipient: transfer_nft (ZEC),
//! transfer_nft (Penumbra), MsgSend {7100000uthiol, 4200000uterp}. Batched per --batch-size
//! recipients (default 14 = one tx).
//!
//! DRY RUN BY DEFAULT: read-only LCD checks, exact msgs, gas simulation (unsigned tx, signer =
//! sender) of the full batch, the bank sends alone, and each collection's transfers alone.
//! `--broadcast` + ZECHUB_GIFT_BROADCAST=1: approval page captures the sender (granter override
//! = --sender, so any key loaded in Zigner/Zafu/Keplr at --account-index can sign), grants
//! authz {transfer_nft scoped per collection, generic MsgSend} + feegrant to an ephemeral key,
//! which broadcasts MsgExec(sender); grant revoked at the end. No key material is read.
//!
//! Build: cargo build -p terp-orch --features gov --bin zechub_voter_gift
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmos::bank::v1beta1::MsgSend;
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
const CW721_SVG_CODE: u64 = 98;
const SVG_MINTER: &str = "terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80";
const COLLECTIONS: [(&str, &str, &str); 2] = [
    (
        "zec",
        "terp1q3kgl2rnx30f88t5hdfym7s7ugkll9rtvtx2slaq6rd2p5gc50jq0rh3er",
        "Zec SVG",
    ),
    (
        "penumbra",
        "terp1yldtf5nsychkasyz3gddemqy45fvl2fxpmc8jrsqvt5fv7ggghysektnqk",
        "Penumbra SVG",
    ),
];
const T_EXEC: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";
const T_SEND: &str = "/cosmos.bank.v1beta1.MsgSend";
const T_AUTHZ_EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
const MEMO: &str = "zechub voter gift";
const PLACEHOLDER_SENDER: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";

#[derive(Parser)]
#[command(
    about = "Gift ZEC + Penumbra SVG NFTs + THIOL + TERP to ZecHub DAO voters via cw-orch-kms (dry run by default)"
)]
struct Cli {
    /// Sign and broadcast through cw-orch-kms. Also needs ZECHUB_GIFT_BROADCAST=1.
    #[arg(long)]
    broadcast: bool,
    #[arg(
        long,
        default_value = "/Users/returniflost/grok-bot/bounties/votes.csv"
    )]
    votes: PathBuf,
    /// Account that owns the pre-minted SVGs and pays tokens + gas. It is the authz granter.
    #[arg(long, env = "ZECHUB_GIFT_SENDER", default_value = PLACEHOLDER_SENDER)]
    sender: String,
    /// uthiol per recipient (7.10 THIOL).
    #[arg(long, default_value_t = 7_100_000)]
    uthiol_each: u128,
    /// uterp per recipient (4.20 TERP).
    #[arg(long, default_value_t = 4_200_000)]
    uterp_each: u128,
    #[arg(long, env = "TERP_LCD", default_value = "https://api.terp.network")]
    lcd: String,
    #[arg(long, env = "GRPC_URL", default_value = "http://192.168.10.101:9090")]
    grpc_url: String,
    /// Recipients per tx (each = 2 transfer_nft + 1 MsgSend). 14 = one tx, 7 = two.
    #[arg(long, default_value_t = 14)]
    batch_size: usize,
    #[arg(long)]
    out: Option<PathBuf>,
    /// Account index of the sender key in the approval wallet.
    #[arg(long, default_value_t = 0u32)]
    account_index: u32,
    #[arg(long, default_value_t = 30 * 60)]
    ttl_secs: u64,
    #[arg(long, default_value_t = 0.05)]
    gas_price: f64,
}

#[derive(Clone)]
struct Recipient {
    juno: String,
    terp: String,
    zec: String,
    penumbra: String,
    status: String,
}

struct Coll {
    label: &'static str,
    addr: &'static str,
    owned: Vec<String>,
}

struct Batch {
    label: String,
    recipients: Vec<String>,
    msgs: Vec<Any>,
    bank: Vec<Any>,
    nft: [Vec<Any>; 2],
    json: Value,
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
    if cli.batch_size == 0 {
        bail!("--batch-size must be >= 1");
    }
    let out = cli.out.clone().unwrap_or_else(|| {
        let ts = std::process::Command::new("date")
            .arg("+%Y%m%dT%H%M%S")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "now".into());
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(format!("grok-bot/staging/zechub-gift-{ts}"))
    });
    std::fs::create_dir_all(&out)?;
    println!(
        "mode: {}",
        if cli.broadcast {
            "BROADCAST (cw-orch-kms)"
        } else {
            "DRY RUN (simulate only, nothing signed)"
        }
    );
    println!(
        "sender: {}{}",
        cli.sender,
        if cli.sender == PLACEHOLDER_SENDER {
            "  (PLACEHOLDER: pass --sender / ZECHUB_GIFT_SENDER)"
        } else {
            ""
        }
    );
    println!("out: {}", out.display());
    if cli.broadcast && cli.sender == PLACEHOLDER_SENDER {
        bail!("--broadcast needs the real --sender");
    }

    let mut recips = load_recipients(&cli.votes)?;
    let ok: Vec<String> = recips
        .iter()
        .filter(|r| r.status == "ok")
        .map(|r| r.terp.clone())
        .collect();
    let n = ok.len();
    println!(
        "voters: {} unique, {n} giftable, {} skipped",
        recips.len(),
        recips.len() - n
    );
    for r in recips.iter().filter(|r| r.status != "ok") {
        println!("  SKIP {} ({})", r.juno, r.status);
    }
    if ok.is_empty() {
        bail!("no giftable recipients");
    }

    let (colls, nft_ok) = validate(&cli, n)?;
    // lowest-first token ids in CSV order
    let mut k = 0;
    for r in recips.iter_mut().filter(|r| r.status == "ok") {
        r.zec = colls[0].owned.get(k).cloned().unwrap_or_default();
        r.penumbra = colls[1].owned.get(k).cloned().unwrap_or_default();
        k += 1;
    }
    let batches = plan(&cli, &recips)?;
    let need_thiol = cli.uthiol_each * n as u128;
    let need_terp = cli.uterp_each * n as u128;

    if !cli.broadcast {
        if let Err(e) = grpc_preflight(&cli) {
            println!("[WARN] {e:#}");
        }
        let (mut total_gas, mut total_fee, mut est) = (0u64, 0u64, false);
        let mut rows = vec![];
        for b in &batches {
            let full = simulate(&cli, &b.msgs);
            let bank = simulate(&cli, &b.bank);
            let nz = simulate(&cli, &b.nft[0]);
            let np = simulate(&cli, &b.nft[1]);
            let show = |r: &Result<u64>| match r {
                Ok(g) => format!("gas {g}"),
                Err(e) => format!("FAIL {e:#}"),
            };
            println!(
                "simulate {} ({} msgs) full: {}",
                b.label,
                b.msgs.len(),
                show(&full)
            );
            println!(
                "  bank sends only ({} MsgSend): {}",
                b.bank.len(),
                show(&bank)
            );
            println!("  zec transfer_nft only: {}", show(&nz));
            println!("  penumbra transfer_nft only: {}", show(&np));
            let gas = match (&full, &bank) {
                (Ok(g), _) => *g,
                // full batch fails (sender owns no tokens): estimate = bank + 2 * per-transfer (~150k) * n
                (Err(_), Ok(g)) => {
                    est = true;
                    *g + 2 * 150_000 * b.recipients.len() as u64
                }
                (Err(e), Err(_)) => bail!("{} simulate failed: {e:#}", b.label),
            };
            let fee = fee_for(gas, cli.gas_price);
            total_gas += gas;
            total_fee += fee;
            rows.push(json!({"batch": b.label, "recipients": b.recipients, "gas": gas, "gas_estimated": full.is_err(), "fee_uthiol": fee,
                "sim": {"full": show(&full), "bank_only": show(&bank), "zec_only": show(&nz), "penumbra_only": show(&np)}, "msgs": b.json}));
        }
        let bal_thiol = balance(&cli, "uthiol").unwrap_or(0);
        let bal_terp = balance(&cli, "uterp").unwrap_or(0);
        let allowance = cw_orch_kms::planned_spend_limit(batches.len() as u64 + 1);
        let need_u = need_thiol + total_fee as u128;
        println!(
            "TOTAL: {} tx(s), {n} recipients: {n} ZEC SVG + {n} Penumbra SVG + {need_thiol} uthiol ({:.2} THIOL) + {need_terp} uterp ({:.2} TERP)",
            batches.len(),
            need_thiol as f64 / 1e6,
            need_terp as f64 / 1e6
        );
        println!(
            "gas {total_gas}{}, fee ~= {total_fee} uthiol ({:.6} THIOL); session fee allowance cap {allowance} uthiol",
            if est { " (estimated)" } else { "" },
            total_fee as f64 / 1e6
        );
        println!(
            "sender uthiol {bal_thiol} vs ~{need_u} (+ allowance cap {allowance}) -> {}",
            if bal_thiol >= need_u + allowance {
                "OK"
            } else {
                "INSUFFICIENT"
            }
        );
        println!(
            "sender uterp  {bal_terp} vs {need_terp} -> {}",
            if bal_terp >= need_terp {
                "OK"
            } else {
                "INSUFFICIENT"
            }
        );
        println!(
            "NFT ownership: {}",
            if nft_ok {
                "OK"
            } else {
                "INSUFFICIENT (see above)"
            }
        );
        for r in recips.iter_mut().filter(|r| r.status == "ok") {
            r.status = "planned (dry run)".into();
        }
        write_csv(&out, &recips)?;
        let plan = json!({"_NOT_SIGNED": "dry run; nothing signed or broadcast", "chain_id": CHAIN_ID, "sender": cli.sender,
            "sender_is_placeholder": cli.sender == PLACEHOLDER_SENDER,
            "per_recipient": {"zec_svg": 1, "penumbra_svg": 1, "uthiol": cli.uthiol_each.to_string(), "uterp": cli.uterp_each.to_string()},
            "collections": colls.iter().map(|c| json!({"label": c.label, "address": c.addr, "sender_owns": c.owned})).collect::<Vec<_>>(),
            "batches": rows, "total_gas": total_gas, "gas_estimated": est, "total_fee_uthiol": total_fee,
            "total_uthiol": need_thiol.to_string(), "total_uterp": need_terp.to_string(),
            "sender_uthiol": bal_thiol.to_string(), "sender_uterp": bal_terp.to_string(), "nft_ownership_ok": nft_ok});
        std::fs::write(
            out.join("plan.json"),
            serde_json::to_string_pretty(&plan)? + "\n",
        )?;
        println!(
            "wrote {} and {}",
            out.join("recipients.csv").display(),
            out.join("plan.json").display()
        );
        println!("DRY RUN done.");
        return Ok(());
    }
    if !nft_ok {
        bail!("sender does not own enough SVGs");
    }
    broadcast(&cli, &out, &colls, batches, recips, need_thiol, need_terp)
}

// ---------------- recipients ----------------
fn load_recipients(path: &PathBuf) -> Result<Vec<Recipient>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let raw = raw.trim_start_matches('\u{feff}');
    let mut lines = raw.lines();
    let header: Vec<String> = split_csv(lines.next().ok_or_else(|| anyhow!("empty csv"))?);
    let col = header
        .iter()
        .position(|h| h.eq_ignore_ascii_case("voter"))
        .ok_or_else(|| anyhow!("no Voter column: {header:?}"))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = vec![];
    for l in lines.filter(|l| !l.trim().is_empty()) {
        let juno = split_csv(l)
            .get(col)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !seen.insert(juno.clone()) {
            continue;
        }
        let (terp, status) = match bech32_decode(&juno) {
            Err(e) => (String::new(), format!("invalid bech32: {e}")),
            Ok((hrp, _)) if hrp != "juno" => (String::new(), format!("prefix {hrp}, not juno")),
            Ok((_, data)) if data.len() == 32 => (
                bech32_encode("terp", &data),
                "skipped: 32-byte contract address".into(),
            ),
            Ok((_, data)) if data.len() != 20 => {
                (String::new(), format!("unexpected length {}", data.len()))
            }
            Ok((_, data)) => {
                let t = bech32_encode("terp", &data);
                match bech32_decode(&t) {
                    Ok((h, d)) if h == "terp" && d == data => (t, "ok".into()),
                    _ => (t, "re-encode check failed".into()),
                }
            }
        };
        out.push(Recipient {
            juno,
            terp,
            zec: String::new(),
            penumbra: String::new(),
            status,
        });
    }
    Ok(out)
}
fn split_csv(l: &str) -> Vec<String> {
    let (mut v, mut cur, mut q) = (vec![], String::new(), false);
    let mut it = l.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if q && it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            '"' => q = !q,
            ',' if !q => v.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    v.push(cur);
    v
}
fn write_csv(out: &PathBuf, r: &[Recipient]) -> Result<()> {
    let mut s = String::from("juno1,terp1,zec_token_id,penumbra_token_id,status\n");
    for x in r {
        s += &format!(
            "{},{},{},{},\"{}\"\n",
            x.juno,
            x.terp,
            x.zec,
            x.penumbra,
            x.status.replace('"', "'")
        );
    }
    std::fs::write(out.join("recipients.csv"), s)?;
    Ok(())
}

// ---------------- bech32 (BIP-173) ----------------
const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
fn polymod(v: &[u8]) -> u32 {
    let g = [
        0x3b6a57b2u32,
        0x26508e6d,
        0x1ea119fa,
        0x3d4233dd,
        0x2a1462b3,
    ];
    let mut c = 1u32;
    for &x in v {
        let b = c >> 25;
        c = ((c & 0x1ffffff) << 5) ^ x as u32;
        for (i, gi) in g.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                c ^= gi;
            }
        }
    }
    c
}
fn hrp_expand(h: &str) -> Vec<u8> {
    let mut v: Vec<u8> = h.bytes().map(|b| b >> 5).collect();
    v.push(0);
    v.extend(h.bytes().map(|b| b & 31));
    v
}
fn convert(data: &[u8], from: u32, to: u32, pad: bool) -> Result<Vec<u8>> {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, vec![]);
    let maxv = (1u32 << to) - 1;
    for &d in data {
        acc = (acc << from) | d as u32;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & maxv) as u8);
        }
    }
    if pad {
        if bits > 0 {
            out.push(((acc << (to - bits)) & maxv) as u8);
        }
    } else if bits >= from || ((acc << (to - bits)) & maxv) != 0 {
        bail!("bad padding");
    }
    Ok(out)
}
fn bech32_decode(s: &str) -> Result<(String, Vec<u8>)> {
    if s.len() > 90 || (s.to_lowercase() != s && s.to_uppercase() != s) {
        bail!("bad length/case");
    }
    let s = s.to_lowercase();
    let pos = s.rfind('1').ok_or_else(|| anyhow!("no separator"))?;
    if pos == 0 || pos + 7 > s.len() {
        bail!("bad separator position");
    }
    let (hrp, rest) = (&s[..pos], &s[pos + 1..]);
    let data: Vec<u8> = rest
        .bytes()
        .map(|b| CHARSET.iter().position(|&c| c == b).map(|p| p as u8))
        .collect::<Option<_>>()
        .ok_or_else(|| anyhow!("bad char"))?;
    let mut v = hrp_expand(hrp);
    v.extend(&data);
    if polymod(&v) != 1 {
        bail!("bad checksum");
    }
    Ok((hrp.into(), convert(&data[..data.len() - 6], 5, 8, false)?))
}
fn bech32_encode(hrp: &str, bytes: &[u8]) -> String {
    let data = convert(bytes, 8, 5, true).expect("pad");
    let mut v = hrp_expand(hrp);
    v.extend(&data);
    v.extend([0u8; 6]);
    let m = polymod(&v) ^ 1;
    let mut s = format!("{hrp}1");
    for b in data
        .iter()
        .copied()
        .chain((0..6).map(|i| ((m >> (5 * (5 - i))) & 31) as u8))
    {
        s.push(CHARSET[b as usize] as char);
    }
    s
}

// ---------------- read-only chain validation ----------------
fn validate(cli: &Cli, n: usize) -> Result<(Vec<Coll>, bool)> {
    let lcd = cli.lcd.trim_end_matches('/');
    let mut fails = vec![];
    let mut nft_ok = true;
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
    ck(
        chain == CHAIN_ID,
        format!(
            "chain-id {chain} (height {})",
            latest["block"]["header"]["height"].as_str().unwrap_or("?")
        ),
    );
    let (sh, sd) = bech32_decode(&cli.sender).map_err(|e| anyhow!("sender: {e}"))?;
    ck(
        sh == "terp" && sd.len() == 20,
        format!("sender {} is a terp1 wallet address", cli.sender),
    );
    // denoms: no x/bank denom metadata on morocco-1 for uthiol/uterp; check supply + bond denom.
    for d in ["uthiol", "uterp"] {
        let s = get(
            lcd,
            &format!("/cosmos/bank/v1beta1/supply/by_denom?denom={d}"),
        )?;
        let amt: u128 = s["amount"]["amount"]
            .as_str()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        ck(
            amt > 0,
            format!("denom {d} exists (supply {amt}); 6 decimals (u-prefix, no on-chain metadata)"),
        );
    }
    let bond = get(lcd, "/cosmos/staking/v1beta1/params")?["params"]["bond_denom"]
        .as_str()
        .unwrap_or("")
        .to_string();
    ck(bond == "uterp", format!("bond denom {bond} (TERP)"));
    let mut colls = vec![];
    for (label, addr, name) in COLLECTIONS {
        let info = get(lcd, &format!("/cosmwasm/wasm/v1/contract/{addr}"))?;
        let code = info["contract_info"]["code_id"].as_str().unwrap_or("");
        let creator = info["contract_info"]["creator"].as_str().unwrap_or("");
        ck(
            code == CW721_SVG_CODE.to_string(),
            format!("{label} {addr} code_id {code}"),
        );
        ck(
            creator == SVG_MINTER,
            format!("{label} created by svg-minter ({creator})"),
        );
        let ci = smart(lcd, addr, &json!({"contract_info": {}}))?["data"].clone();
        ck(ci["name"] == name, format!("{label} name {}", ci["name"]));
        let mut owned: Vec<String> = vec![];
        let mut after: Option<String> = None;
        loop {
            let q = json!({"tokens": {"owner": cli.sender, "start_after": after, "limit": 100}});
            let page: Vec<String> = smart(lcd, addr, &q)?["data"]["tokens"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            if page.is_empty() {
                break;
            }
            after = page.last().cloned();
            let len = page.len();
            owned.extend(page);
            if len < 100 {
                break;
            }
        }
        owned.sort_by_key(|t| (t.parse::<u64>().unwrap_or(u64::MAX), t.clone()));
        let enough = owned.len() >= n;
        if !enough {
            nft_ok = false;
        }
        println!(
            "[{}] {label} sender owns {} token(s) {:?}, needs {n}",
            if enough { " OK " } else { "FAIL" },
            owned.len(),
            owned
        );
        owned.truncate(n);
        colls.push(Coll { label, addr, owned });
    }
    if !fails.is_empty() {
        bail!("on-chain validation failed: {fails:?}");
    }
    Ok((colls, nft_ok))
}

// ---------------- plan ----------------
fn exec(sender: &str, contract: &str, msg: &Value) -> Result<Any> {
    let m = MsgExecuteContract {
        sender: sender.into(),
        contract: contract.into(),
        msg: serde_json::to_vec(msg)?,
        funds: vec![],
    };
    Ok(Any {
        type_url: T_EXEC.into(),
        value: m.encode_to_vec(),
    })
}
fn send(cli: &Cli, to: &str) -> Any {
    let m = MsgSend {
        from_address: cli.sender.clone(),
        to_address: to.into(),
        amount: vec![
            ProtoCoin {
                denom: "uterp".into(),
                amount: cli.uterp_each.to_string(),
            },
            ProtoCoin {
                denom: "uthiol".into(),
                amount: cli.uthiol_each.to_string(),
            },
        ],
    };
    Any {
        type_url: T_SEND.into(),
        value: m.encode_to_vec(),
    }
}
fn plan(cli: &Cli, recips: &[Recipient]) -> Result<Vec<Batch>> {
    let ok: Vec<&Recipient> = recips.iter().filter(|r| r.status == "ok").collect();
    let mut out = vec![];
    for (bi, chunk) in ok.chunks(cli.batch_size).enumerate() {
        let (mut msgs, mut bank, mut nz, mut np, mut js) = (vec![], vec![], vec![], vec![], vec![]);
        for r in chunk {
            for (ci, tid) in [(0usize, &r.zec), (1usize, &r.penumbra)] {
                let addr = COLLECTIONS[ci].1;
                let tid = if tid.is_empty() {
                    "UNOWNED".to_string()
                } else {
                    tid.clone()
                };
                let t = json!({"transfer_nft": {"recipient": r.terp, "token_id": tid}});
                let m = exec(&cli.sender, addr, &t)?;
                msgs.push(m.clone());
                if ci == 0 {
                    nz.push(m)
                } else {
                    np.push(m)
                }
                js.push(json!({"type": T_EXEC, "contract": addr, "msg": t}));
            }
            let s = send(cli, &r.terp);
            msgs.push(s.clone());
            bank.push(s);
            js.push(json!({"type": T_SEND, "from": cli.sender, "to": r.terp, "amount": format!("{}uterp,{}uthiol", cli.uterp_each, cli.uthiol_each)}));
        }
        out.push(Batch {
            label: format!("gift-{}", bi + 1),
            recipients: chunk.iter().map(|r| r.terp.clone()).collect(),
            msgs,
            bank,
            nft: [nz, np],
            json: json!(js),
        });
    }
    Ok(out)
}

// ---------------- helpers ----------------
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
fn balance(cli: &Cli, denom: &str) -> Result<u128> {
    let b = get(
        cli.lcd.trim_end_matches('/'),
        &format!(
            "/cosmos/bank/v1beta1/balances/{}/by_denom?denom={denom}",
            cli.sender
        ),
    )?;
    Ok(b["balance"]["amount"].as_str().unwrap_or("0").parse()?)
}
/// Unsigned tx with the sender as signer (no pubkey; simulate skips sig checks).
fn simulate(cli: &Cli, msgs: &[Any]) -> Result<u64> {
    let lcd = cli.lcd.trim_end_matches('/');
    let acct = get(
        lcd,
        &format!("/cosmos/auth/v1beta1/accounts/{}", cli.sender),
    )?;
    let seq: u64 = find_key(&acct["account"], "sequence")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| anyhow!("sender account has no sequence"))?
        .parse()?;
    let body = TxBody {
        messages: msgs.to_vec(),
        memo: format!("{MEMO} dry run"),
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

// ---------------- broadcast (cw-orch-kms) ----------------
fn broadcast(
    cli: &Cli,
    out: &PathBuf,
    colls: &[Coll],
    batches: Vec<Batch>,
    mut recips: Vec<Recipient>,
    need_thiol: u128,
    need_terp: u128,
) -> Result<()> {
    use cw_orch::daemon::{DaemonBuilder, TxSender};
    use cw_orch::environment::{ChainInfoOwned, ChainKind, NetworkInfoOwned};
    use cw_orch_kms::progress::Status;
    if std::env::var("ZECHUB_GIFT_BROADCAST").ok().as_deref() != Some("1") {
        bail!("refusing to sign: --broadcast also needs ZECHUB_GIFT_BROADCAST=1");
    }
    let mut fees = 0u128;
    for b in &batches {
        let g = simulate(cli, &b.msgs)?;
        println!("pre-sign simulate {}: gas {g}", b.label);
        fees += fee_for(g, cli.gas_price) as u128;
    }
    let allowance = cw_orch_kms::planned_spend_limit(batches.len() as u64 + 1);
    let (bt, bu) = (balance(cli, "uthiol")?, balance(cli, "uterp")?);
    if bt < need_thiol + allowance {
        bail!("sender uthiol {bt} < {need_thiol} + fee allowance {allowance} (fees ~{fees})");
    }
    if bu < need_terp {
        bail!("sender uterp {bu} < {need_terp}");
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
    {
        let d = builder
            .build()
            .map_err(|e| anyhow!("pre-flight Daemon build on {}: {e}", cli.grpc_url))?;
        use cw_orch::prelude::QueryHandler;
        let h = d
            .block_info()
            .map_err(|e| anyhow!("pre-flight Daemon query: {e}"))?
            .height;
        println!(
            "[ OK ] cw-orch Daemon built on {} (session channel), height {h}",
            cli.grpc_url
        );
    }
    let page = cw_orch_kms::page_addr();
    println!(
        "sign the cw-orch-granter challenge with the SENDER key {} on the approval page (port {})",
        cli.sender,
        page.port()
    );
    let granter = cw_orch_kms::capture_granter_for_chain_opts_sync(
        page,
        cli.account_index,
        "zechub-voter-gift",
        CHAIN_ID,
        cli.ttl_secs,
        Some(&cli.sender),
    )
    .map_err(|e| anyhow!(e))?;
    if granter != cli.sender {
        bail!("approval granter {granter} != --sender {}", cli.sender);
    }

    let n = recips.iter().filter(|r| r.status == "ok").count() as u64;
    let mut session = cw_orch_kms::ApprovalSession::open(&granter);
    for c in colls {
        session = session
            .allow_execute(cw_orch_kms::ScopedCall {
                address: c.addr.into(),
                keys: vec!["transfer_nft".into()],
                calls: n,
                funds: vec![],
            })
            .map_err(|e| anyhow!(e))?;
    }
    // Generic authz for MsgSend (not a spend-capped SendAuthorization; the grant is revoked at close).
    session = session.allow_sdk(T_SEND).map_err(|e| anyhow!(e))?;
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = allowance;
    let actions =
        vec![cw_orch_kms::SigningRequest::new("session-key").body("zechub-voter-gift".to_string())];
    let mut attempt = 0;
    let run = loop {
        attempt += 1;
        match cw_orch_kms::enable_session(&builder, session.clone(), actions.clone()) {
            Ok(r) => break r,
            Err(e)
                if attempt < 5
                    && (e.contains("grpc") || e.contains("gRPC") || e.contains("transport")) =>
            {
                println!("[WARN] enable_session attempt {attempt}/5: {e}; retrying in 3s");
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            Err(e) => bail!(e),
        }
    };
    let daemon = run.daemon.clone();
    println!("session key (grantee): {}", run.grantee());
    let mut log = vec![];
    let result = (|| -> Result<()> {
        for b in &batches {
            let label = b.label.as_str();
            let memo = format!("{MEMO} {label}");
            let sim = vec![Any {
                type_url: T_AUTHZ_EXEC.into(),
                value: cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec {
                    grantee: run.grantee().into(),
                    msgs: b.msgs.clone(),
                }
                .encode_to_vec(),
            }];
            run.step(label, Status::Running, None, None);
            let (gas, fee) = daemon
                .rt_handle
                .block_on(daemon.sender().simulate(sim, Some(&memo)))
                .map_err(|e| {
                    run.step(label, Status::Error, None, Some(&e.to_string()));
                    anyhow!(e)
                })?;
            println!("simulate {label}: gas={gas} fee={fee}");
            let resp = daemon
                .rt_handle
                .block_on(daemon.sender().commit_tx_any(b.msgs.clone(), Some(&memo)))
                .map_err(|e| {
                    run.step(label, Status::Error, None, Some(&e.to_string()));
                    anyhow!(e)
                })?;
            println!(
                "BROADCAST {label} txhash: {} code {}",
                resp.txhash, resp.code
            );
            log.push(json!({"batch": label, "txhash": resp.txhash, "code": resp.code}));
            if resp.code != 0 {
                run.step(
                    label,
                    Status::Error,
                    Some(&resp.txhash),
                    Some(&format!("code {}", resp.code)),
                );
                bail!("{label} failed: code {}", resp.code);
            }
            run.step(label, Status::Confirmed, Some(&resp.txhash), None);
            for r in recips.iter_mut().filter(|r| b.recipients.contains(&r.terp)) {
                r.status = format!("sent {}", resp.txhash);
            }
        }
        Ok(())
    })();
    run.close().map_err(|e| anyhow!(e))?;
    println!("session grant closed (revoked)");
    write_csv(out, &recips)?;
    std::fs::write(
        out.join("broadcast.json"),
        serde_json::to_string_pretty(&json!(log))? + "\n",
    )?;
    result
}

fn grpc_preflight(cli: &Cli) -> Result<()> {
    use cw_orch::daemon::{
        GrpcChannel, RUNTIME,
        queriers::{Bank, Node},
    };
    let url = cli.grpc_url.clone();
    let (height, bal) = RUNTIME.block_on(async {
        let ch = tokio_timeout(GrpcChannel::connect(&[url.clone()], CHAIN_ID))
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url}: {e}"))?;
        let h = Node::new_async(ch.clone())
            ._block_height()
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url} block height: {e}"))?;
        let b = Bank::new_async(ch)
            ._balance(
                &cosmwasm_std::Addr::unchecked(cli.sender.clone()),
                Some("uthiol".to_string()),
            )
            .await
            .map_err(|e| anyhow!("gRPC pre-flight {url} bank balance: {e}"))?;
        Ok::<_, anyhow::Error>((
            h,
            b.first().map(|c| c.amount.to_string()).unwrap_or_default(),
        ))
    })?;
    let lcd_h: u64 = get(
        cli.lcd.trim_end_matches('/'),
        "/cosmos/base/tendermint/v1beta1/blocks/latest",
    )
    .ok()
    .and_then(|v| {
        v["block"]["header"]["height"]
            .as_str()
            .and_then(|h| h.parse().ok())
    })
    .unwrap_or(0);
    if lcd_h > 0 && height + 20 < lcd_h {
        bail!("gRPC pre-flight {url}: node at {height} lags tip {lcd_h}");
    }
    println!("[ OK ] grpc {url} (cw-orch channel) {CHAIN_ID} height {height}, sender uthiol {bal}");
    Ok(())
}
async fn tokio_timeout<F: std::future::Future<Output = Result<T, E>>, T, E: std::fmt::Display>(
    f: F,
) -> Result<T, String> {
    match cw_orch::tokio::time::timeout(std::time::Duration::from_secs(20), f).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err("timed out after 20s".into()),
    }
}
