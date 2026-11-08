//! Draft the Juno community-pool spend for the JUNO / TERP / THIOL
//! liquidity deployment, and the steward-juno proposal that submits it.
//!
//! Dry-run only. This binary writes the plan and, once `--recipient` is the
//! custody-juno contract, the `dao_account` file. `dao_account` is the
//! cw-orch-kms broadcast. Nothing here signs or broadcasts.
//!
//! The recipient cannot be filled until steward-juno's Polytone proxy has
//! instantiated custody-juno. Passing the proxy address, or any contract that
//! is not that instantiate, is refused.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use clap::Parser;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
use cosmos_sdk_proto::cosmos::distribution::v1beta1::MsgCommunityPoolSpend;
use cosmwasm_std::{Binary, CosmosMsg};
use dao_scripts::kms_gate;
use dao_scripts::polytone::gov_submit_any;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const PROXY: &str = "juno1guahc2tdgvdphgyejpnps5ns9n46dgqsvfff6rn5nkdmaeaz63rqx5zds5";
const JUNO_GOV: &str = "juno10d07y265gmmuvt4z0w9aw880jnsr700jvss730";
const NOTE: &str = "terp180xdkmamk6dp30jvuve82qr5l5mesenj5d684l8ag50j8fdlk2xqpftrxe";
const DAO: &str = "terp1a35tgcr4nvqttet76uesqw5fwg55jdpkacp3htm62jxk8xkjvl5qsd4a6z";
const PROPOSAL_MODULE: &str = "terp1eks9dxycthld5vd6v0ardfhuphjjtmu4cgzrckx7aa4alp7ffp3qez2zdy";
const SIGNER: &str = "terp1drrqa2mmk4syy85hrflta73408p0t5n4fkpc8x";
const CORE_CODE: &str = "4862";
const TERP_DENOM: &str = "ibc/2578547913C40B0DEBAB7E458F191491DE6288A01E1000E9BAE34103DEB189D3";
const THIOL_DENOM: &str = "ibc/055F223602B754BF34FE269B729837A02BDF79E069F2387AEEA9CD28DE961BE7";
/// 1,590,000 JUNO. 1.59M at exponent 6.
const JUNO_AMOUNT: u128 = 1_590_000_000_000;
/// 2,100,000 TERP. 2.1M at exponent 6. This is the whole TERP balance of the pool.
const TERP_AMOUNT: u128 = 2_100_000_000_000;
/// 2,846,700 THIOL. The request is 2.85M. The pool holds 2,846,700, so the
/// message spends that balance (2846700000000 uthiol) rather than 3,300 more.
const THIOL_AMOUNT: u128 = 2_846_700_000_000;
/// 20% of juno-1 min deposit (5,000 JUNO). The proposal still needs 5,000 JUNO
/// deposited before voting. Not part of the spend.
const INITIAL_DEPOSIT: u128 = 1_000_000_000;

const TITLE: &str = "Community pool spend for a JUNO, TERP, and THIOL liquidity deployment";

#[derive(Parser, Debug)]
struct Cli {
    /// custody-juno core on juno-1. Omit while that contract does not exist.
    #[arg(long)]
    recipient: Option<String>,
    #[arg(long, default_value = "https://juno-api.polkachu.com")]
    lcd: String,
    #[arg(long, default_value = "params/morocco-1.dao-c/12-juno-liquidity.html")]
    html: PathBuf,
    #[arg(
        long,
        default_value = "params/morocco-1.dao-c/12-juno-liquidity-spend.json"
    )]
    out: PathBuf,
    /// dao_account input. Written only after the recipient check passes.
    #[arg(
        long,
        default_value = "params/morocco-1.dao-c/12-juno-liquidity-spend.dao.json"
    )]
    dao_file: PathBuf,
}

fn coins() -> Vec<Coin> {
    vec![
        Coin {
            denom: "ujuno".into(),
            amount: JUNO_AMOUNT.to_string(),
        },
        Coin {
            denom: TERP_DENOM.into(),
            amount: TERP_AMOUNT.to_string(),
        },
        Coin {
            denom: THIOL_DENOM.into(),
            amount: THIOL_AMOUNT.to_string(),
        },
    ]
}

fn page_url(cid: &str) -> String {
    format!("https://s3.terp.network/static/gov/juno-liquidity/{cid}.html")
}

fn page(recipient_line: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>JUNO, TERP, and THIOL liquidity deployment</title>
</head>
<body>
<h1>JUNO, TERP, and THIOL liquidity deployment</h1>
<p>Spend funds from the Juno community pool to the custody-juno DAO so that DAO can deploy liquidity on Juno.</p>
<h2>Spend</h2>
<ul>
<li>1,590,000 JUNO = <code>1590000000000</code> <code>ujuno</code></li>
<li>2,100,000 TERP = <code>2100000000000</code> <code>uterp</code> = <code>2100000000000</code> of <code>{terp}</code>, trace <code>transfer/channel-393/uterp</code></li>
<li>2,846,700 THIOL = <code>2846700000000</code> <code>uthiol</code> = <code>2846700000000</code> of <code>{thiol}</code>, trace <code>transfer/channel-393/uthiol</code></li>
</ul>
<h2>Recipient</h2>
<p>{recipient_line}</p>
<h2>Size</h2>
<p>1,590,000 JUNO, 2,100,000 TERP, and 2,846,700 THIOL. The THIOL request is 2.85M. The community pool holds 2,846,700 THIOL, so the spend is that balance. At $0.00684943 per JUNO (CoinGecko juno-network, 2026-11-01), the baseline floor $0.05012 per TERP, and the baseline floor $0.02360 per THIOL, this is about $183,324.71. The baseline is the floor from the Terp baseline valuation. It is not a market quote.</p>
<h2>Deposit</h2>
<p>The proposer pays an initial deposit of 1,000 JUNO. Voting starts after the deposit reaches 5,000 JUNO. The deposit period is 10 days and the voting period is 5 days. The deposit is not part of the spend.</p>
<h2>Accounts</h2>
<p>The spend authority is the Juno x/gov module account <code>{gov}</code>. The proposer is steward-juno's Polytone account <code>{proxy}</code>. custody-juno is a DAO-DAO core, code {code}, instantiated by that account. The Juno x/gov module account is its wasm admin, its dao admin, and its 48-hour vetoer.</p>
</body>
</html>
"#,
        terp = TERP_DENOM,
        thiol = THIOL_DENOM,
        gov = JUNO_GOV,
        proxy = PROXY,
        code = CORE_CODE,
    )
}

fn on_chain_summary(recipient: &str, cid: &str) -> String {
    format!(
        "Spend 1,590,000 JUNO (1590000000000 ujuno), 2,100,000 TERP (2100000000000 uterp, denom {terp}, trace transfer/channel-393/uterp), and 2,846,700 THIOL (2846700000000 uthiol, denom {thiol}, trace transfer/channel-393/uthiol) from the Juno community pool to {recipient} for a liquidity deployment. The full proposal is {url}.",
        terp = TERP_DENOM,
        thiol = THIOL_DENOM,
        url = page_url(cid),
    )
}

/// CIDv1, raw codec, sha2-256, base32 multibase. This is the hash of the HTML
/// bytes, not a UnixFS dag-pb CID.
fn cid_v1_raw(bytes: &[u8]) -> String {
    let dig = Sha256::digest(bytes);
    let mut mh = vec![0x12, 0x20];
    mh.extend_from_slice(&dig);
    let mut cid = vec![0x01, 0x55];
    cid.extend_from_slice(&mh);
    format!("b{}", base32_lower(&cid))
}

fn base32_lower(data: &[u8]) -> String {
    const ALPH: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        buf = (buf << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPH[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPH[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn lcd_get(lcd: &str, path: &str) -> Result<Value> {
    let url = format!(
        "{}/{}",
        lcd.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "25", &url])
        .output()
        .map_err(|e| anyhow!("curl {url}: {e}"))?;
    if !out.status.success() {
        bail!("GET {url} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn require_custody(lcd: &str, recipient: &str) -> Result<()> {
    if recipient == PROXY {
        bail!("{recipient} is the Polytone proxy, not the custody-juno DAO");
    }
    if recipient == JUNO_GOV {
        bail!("{recipient} is the x/gov module account");
    }
    let body = lcd_get(lcd, &format!("cosmwasm/wasm/v1/contract/{recipient}"))?;
    let info = &body["contract_info"];
    let code = info["code_id"].as_str().unwrap_or("");
    let creator = info["creator"].as_str().unwrap_or("");
    let label = info["label"].as_str().unwrap_or("");
    let admin = info["admin"].as_str().unwrap_or("");
    if code != CORE_CODE || creator != PROXY || label != "custody-juno" || admin != JUNO_GOV {
        bail!(
            "{recipient} is not custody-juno (code {code}, creator {creator}, label {label}, admin {admin})"
        );
    }
    Ok(())
}

fn spend_any(recipient: &str) -> Result<cosmrs::Any> {
    Ok(kms_gate::any(
        "/cosmos.distribution.v1beta1.MsgCommunityPoolSpend",
        &MsgCommunityPoolSpend {
            authority: JUNO_GOV.into(),
            recipient: recipient.into(),
            amount: coins(),
        },
    ))
}

fn note_msg(submit: &cosmrs::Any) -> Result<String> {
    let stargate = CosmosMsg::<cosmwasm_std::Empty>::Any(cosmwasm_std::AnyMsg {
        type_url: submit.type_url.clone(),
        value: Binary::from(submit.value.clone()),
    });
    let wrapped = json!({
        "execute": { "msgs": [stargate], "callback": null, "timeout_seconds": "3600" }
    });
    Ok(base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&wrapped)?))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (html, recipient_line, ready) = match &cli.recipient {
        None => {
            let line = format!(
                "The recipient is the custody-juno contract, DAO-DAO core code {CORE_CODE}, instantiated by {PROXY} with label custody-juno. That contract is not named here because it is not on chain yet."
            );
            (page(&line), line, false)
        }
        Some(recipient) => {
            require_custody(&cli.lcd, recipient)?;
            let line = format!(
                "The recipient is <code>{recipient}</code>, the custody-juno contract (code {CORE_CODE}) instantiated by {PROXY}."
            );
            (page(&line), line, true)
        }
    };
    if let Some(parent) = cli.html.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cli.html, &html)?;
    let cid = cid_v1_raw(html.as_bytes());
    let url = page_url(&cid);
    let summary = cli.recipient.as_ref().map(|r| on_chain_summary(r, &cid));
    if let Some(s) = &summary {
        if s.len() > 10_000 || TITLE.len() > 255 || url.len() > 255 {
            bail!("title, summary, or metadata is over the gov limit");
        }
        if s.to_lowercase().contains("appendix") {
            bail!("on-chain summary still contains drafting language");
        }
    }
    let mut plan = json!({
        "ready": ready,
        "broadcast": false,
        "chain_id": "juno-1",
        "kind": "community_pool_spend",
        "authority": JUNO_GOV,
        "proposer": PROXY,
        "recipient": cli.recipient,
        "recipient_line": recipient_line,
        "amounts": [
            {"amount": JUNO_AMOUNT.to_string(), "denom": "ujuno", "display": "1,590,000 JUNO", "base": "ujuno", "exponent": 6},
            {"amount": TERP_AMOUNT.to_string(), "denom": TERP_DENOM, "display": "2,100,000 TERP", "base": "uterp", "exponent": 6, "trace": "transfer/channel-393/uterp"},
            {"amount": THIOL_AMOUNT.to_string(), "denom": THIOL_DENOM, "display": "2,846,700 THIOL", "base": "uthiol", "exponent": 6, "trace": "transfer/channel-393/uthiol"}
        ],
        "pricing": "1,590,000 JUNO, 2,100,000 TERP, and 2,846,700 THIOL. About $183,324.71 at JUNO $0.00684943 (2026-11-01) and baseline floors TERP $0.05012 and THIOL $0.02360. 2.85M THIOL is 3,300 over the pool, so the message spends the pool balance.",
        "initial_deposit": format!("{INITIAL_DEPOSIT}ujuno"),
        "min_deposit": "5000000000ujuno",
        "deposit_period": "10 days",
        "voting_period": "5 days",
        "html": cli.html.display().to_string(),
        "cid_v1_raw": cid,
        "page": url,
        "title": TITLE,
        "summary": summary,
        "blocker": if ready {
            Value::Null
        } else {
            json!("custody-juno is not instantiated, so this spend has no recipient. Run 11-steward-juno-create-custody-on-juno.json through dao_account and relay the packet, then rerun with --recipient. The spend proposal is separate: the proxy needs 1,000 JUNO before it can submit, and the deposit must reach 5,000 JUNO.")
        }
    });
    if ready {
        let recipient = cli.recipient.as_deref().unwrap();
        let submit = gov_submit_any(
            vec![spend_any(recipient)?],
            PROXY,
            (INITIAL_DEPOSIT, "ujuno"),
            TITLE,
            summary.as_deref().unwrap(),
        )?;
        // gov_submit_any leaves metadata empty. The page URL is the metadata.
        let mut decoded =
            cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal::decode(submit.value.as_slice())?;
        decoded.metadata = url.clone();
        let submit = kms_gate::any("/cosmos.gov.v1.MsgSubmitProposal", &decoded);
        let dao_file = json!({
            "dao": DAO,
            "proposal_module": PROPOSAL_MODULE,
            "signer": SIGNER,
            "flow": "propose+vote+execute on steward-juno, then Hermes relays the note. The proxy must hold 1,000 JUNO before execute. Not broadcast by this binary.",
            "note": NOTE,
            "proposer": PROXY,
            "recipient": recipient,
            "page": url,
            "propose": {
                "propose": {
                    "title": "Submit the Juno liquidity community-pool spend",
                    "description": format!(
                        "steward-juno's Juno account {PROXY} submits a juno-1 governance proposal. It spends 1,590,000 JUNO (1590000000000 ujuno), 2,100,000 TERP (2100000000000 uterp), and 2,846,700 THIOL (2846700000000 uthiol) from the community pool to {recipient}. The account must already hold 1,000 JUNO for the initial deposit. The full proposal is {url}."
                    ),
                    "msgs": [{
                        "wasm": {
                            "execute": {
                                "contract_addr": NOTE,
                                "funds": [],
                                "msg": note_msg(&submit)?
                            }
                        }
                    }]
                }
            }
        });
        std::fs::write(
            &cli.dao_file,
            serde_json::to_string_pretty(&dao_file)? + "\n",
        )?;
        plan["dao_file"] = json!(cli.dao_file.display().to_string());
    }
    if let Some(parent) = cli.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cli.out, serde_json::to_string_pretty(&plan)? + "\n")?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

use cosmos_sdk_proto::prost::Message;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_matches_the_requested_bootstrap_and_names_minimal_denoms() {
        assert_eq!(JUNO_AMOUNT, 1_590_000 * 1_000_000);
        assert_eq!(TERP_AMOUNT, 2_100_000 * 1_000_000);
        assert_eq!(THIOL_AMOUNT, 2_846_700 * 1_000_000);
        let juno = 1_590_000.0 * 0.00684943;
        let terp = 2_100_000.0 * 0.05012;
        let thiol = 2_846_700.0 * 0.02360;
        let total: f64 = juno + terp + thiol;
        assert!((total - 183_324.71).abs() < 0.02, "{total}");
        let summary = on_chain_summary(
            "juno1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq2zgs3g",
            "bafk",
        );
        assert!(summary.contains("1,590,000 JUNO"));
        assert!(summary.contains("1590000000000 ujuno"));
        assert!(summary.contains("2,100,000 TERP"));
        assert!(summary.contains(&format!("2100000000000 uterp, denom {TERP_DENOM}")));
        assert!(summary.contains("2,846,700 THIOL"));
        assert!(summary.contains(&format!("2846700000000 uthiol, denom {THIOL_DENOM}")));
        assert!(!summary.contains("2100000000000ibc/"));
        assert!(!summary.contains("2846700000000ibc/"));
        assert!(summary.contains("https://s3.terp.network/static/gov/juno-liquidity/bafk.html"));
        assert!(!summary.to_lowercase().contains("appendix"));
        assert!(!summary.contains("IPFS link to this document"));
    }

    #[test]
    fn raw_cid_is_stable_and_not_the_proxy() {
        let html = page("recipient line");
        assert_eq!(cid_v1_raw(html.as_bytes()), cid_v1_raw(html.as_bytes()));
        assert!(cid_v1_raw(html.as_bytes()).starts_with('b'));
        // The proxy is refused before any network call.
        assert!(require_custody("http://127.0.0.1:1", PROXY).is_err());
    }

    #[test]
    fn spend_message_carries_the_three_coins() {
        let any =
            spend_any("juno1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq2zgs3g").unwrap();
        let msg = MsgCommunityPoolSpend::decode(any.value.as_slice()).unwrap();
        assert_eq!(msg.authority, JUNO_GOV);
        assert_eq!(msg.amount.len(), 3);
        assert_eq!(msg.amount[0].amount, "1590000000000");
        assert_eq!(msg.amount[1].denom, TERP_DENOM);
        assert_eq!(msg.amount[1].amount, "2100000000000");
        assert_eq!(msg.amount[2].amount, "2846700000000");
    }
}
