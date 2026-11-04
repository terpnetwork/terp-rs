//! Seed set (round-2 window) export, upload and batched create_bounty for the
//! FIAB bounty escrow. Preview by default, same gating as signing.
//!
//! create_bounty schema: terp-rs `contracts/revenue/bounty-escrow/src/msg.rs`
//! (`ExecuteMsg::CreateBounty`), validated in `src/contract.rs::exec_create`.
//! Details doc: `contracts/revenue/bounty-escrow/offchain-schemas/terp-bounty-details.v1.schema.json`,
//! canonical bytes = `escrow/schemas/lib.mjs canonicalJson` (keys sorted, no whitespace).
use super::{Plan, SessionKind, T_AUTHZ_EXEC, T_EXEC, any, hexenc, rfc3339_utc, sha256_hex};
use anyhow::{Context, Result, anyhow, bail};
use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgExec;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
use cosmos_sdk_proto::prost::Message as _;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub const SEED_SCHEMA: &str = "terp-fiab-bounty-seed/v1";
pub const DETAILS_SCHEMA: &str = "terp-bounty-details/v1";
/// Two-month window cap (review-2): never exceeded.
pub const CAP_UTERP: u128 = 50_000_000_000;
pub const CAP_UTHIOL: u128 = 100_000_000_000;
pub const SUMMARY_MAX_BYTES: usize = 280;
pub const DEFAULT_KEY_PREFIX: &str = "static/terp.network/bounties/seed";

// ---------------- canonical JSON (lib.mjs canonicalJson) ----------------
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(&m[*k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => serde_json::to_string(other).unwrap(),
    }
}

fn map_category(c: &str) -> Result<&'static str> {
    // catalog v3 categories -> the nine terp-bounty-details/v1 enums. [INFERRED] mapping.
    Ok(match c {
        "apps" | "platform" => "apps",
        "ibc" => "ibc",
        "protocol" | "security" => "protocol",
        "docs" | "content" | "translation" => "docs",
        "ops" | "chain-ops" | "qa" => "ops",
        "contracts" => "contracts",
        "community" | "cannabis" => "community",
        "partners" => "partners",
        "liquidity" => "liquidity",
        other => bail!("no details/v1 category for catalog category {other}"),
    })
}

/// Clip to <= max bytes on a char boundary; appends "..." when clipped.
pub fn clip_bytes(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max - 3;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let cut = &s[..end];
    let cut = cut.rfind(' ').map(|i| &cut[..i]).unwrap_or(cut);
    format!("{}...", cut.trim_end())
}

fn no_ctrl(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn capability(s: &str) -> Option<String> {
    let t: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._+#/-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let t = t
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();
    if t.is_empty() || t.len() > 48 {
        None
    } else {
        Some(t)
    }
}

pub struct SeedDoc {
    pub bounty_ref: String,
    pub catalog_id: String,
    pub denom: String,
    pub amount: u128,
    pub doc: Value,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// One details doc per (catalog item, denom): the escrow takes exactly one Coin per bounty.
/// Parse a non-negative decimal string into (digits, scale): "0.05172" -> (5172, 5).
pub fn parse_dec(s: &str) -> Result<(u128, u32)> {
    let s = s.trim();
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    if i.is_empty() && f.is_empty() || !i.chars().chain(f.chars()).all(|c| c.is_ascii_digit()) {
        bail!("bad decimal {s:?}");
    }
    Ok((format!("{i}{f}").parse()?, f.len() as u32))
}

/// floor(usd / price_usd * 10^6) in exact integer arithmetic.
pub fn usd_to_micro(usd: &str, price_usd: &str) -> Result<u128> {
    let (un, us) = parse_dec(usd)?;
    let (pn, ps) = parse_dec(price_usd)?;
    if pn == 0 {
        bail!("price is zero");
    }
    Ok(un * 1_000_000 * 10u128.pow(ps) / (pn * 10u128.pow(us)))
}

/// Payout mode for export-seed: one bounty per item in `uterp` (default) or the legacy TERP+THIOL split.
#[derive(Clone, Debug)]
pub enum Payout {
    Single { denom: String, price_usd: String },
    Split,
}

pub fn build_docs(
    catalog: &Value,
    horizon: &str,
    deadline: &str,
    payout: &Payout,
) -> Result<(String, Vec<SeedDoc>)> {
    let pv = catalog["meta"]["pricing"]["params_version"]
        .as_str()
        .ok_or_else(|| anyhow!("catalog meta.pricing.params_version missing"))?
        .to_string();
    let items = catalog["bounties"]
        .as_array()
        .ok_or_else(|| anyhow!("catalog.bounties missing"))?;
    let mut out = vec![];
    for b in items.iter().filter(|b| b["horizon"] == horizon) {
        let id = b["id"].as_str().unwrap_or_default();
        if b["visibility"] != "public" {
            bail!("{id}: visibility is not public; refusing to seed it");
        }
        let item_pv = b["reward"]["params_version"].as_str().unwrap_or_default();
        if item_pv != pv {
            bail!("{id}: params_version {item_pv} != catalog {pv}");
        }
        let w = &b["reward"]["window_commitment"];
        let full = no_ctrl(b["summary"].as_str().unwrap_or_default());
        let title = no_ctrl(b["title"].as_str().unwrap_or_default());
        if title.trim().is_empty() || title.len() > 140 {
            bail!("{id}: title length {}", title.len());
        }
        let ac: Vec<String> = b["acceptance_criteria"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|x| x.as_str().map(|s| clip_bytes(&no_ctrl(s), 500)))
            .collect();
        let ev: Vec<String> = b["evidence_required"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|x| x.as_str().map(|s| clip_bytes(&no_ctrl(s), 300)))
            .collect();
        if ac.is_empty() || ev.is_empty() {
            bail!("{id}: acceptance_criteria/evidence_required empty");
        }
        let mut caps: Vec<String> = vec![];
        for s in b["skills"].as_array().cloned().unwrap_or_default() {
            if let Some(c) = s.as_str().and_then(capability) {
                if !caps.contains(&c) {
                    caps.push(c);
                }
            }
        }
        caps.truncate(24);
        let units = w["units"].as_u64().unwrap_or(1);
        let months = w["stipend_months"].as_u64().unwrap_or(0);
        let mut scope = full.clone();
        if units > 1 {
            scope.push_str(&format!(" Funded units in this window: {units}."));
        }
        if months > 0 {
            scope.push_str(&format!(" Includes {months} stipend month(s)."));
        }
        let legs: Vec<(&str, String, u128, String)> = match payout {
            Payout::Single { denom, price_usd } => {
                if denom != "uterp" {
                    bail!("single-denom payout supports uterp only (got {denom})");
                }
                let usd = w["usd"]
                    .as_f64()
                    .ok_or_else(|| anyhow!("{id}: window_commitment.usd missing"))?;
                // serde_json prints the shortest round-trip form ("15.6"), so this is the catalog's own decimal.
                let amount = usd_to_micro(&w["usd"].to_string(), price_usd)?;
                if usd <= 0.0 || amount == 0 {
                    bail!("{id}: zero reward");
                }
                vec![("TERP", denom.clone(), amount, id.to_string())]
            }
            Payout::Split => {
                let mut v = vec![];
                for (asset, denom, key) in [
                    ("TERP", "uterp", "TERP_micro"),
                    ("THIOL", "uthiol", "THIOL_micro"),
                ] {
                    let amount: u128 = w[key]
                        .as_str()
                        .ok_or_else(|| anyhow!("{id}: {key} missing"))?
                        .parse()?;
                    if amount > 0 {
                        v.push((asset, denom.to_string(), amount, format!("{id}:{denom}")));
                    }
                }
                v
            }
        };
        for (asset, denom, amount, bounty_ref) in legs {
            let doc = json!({
                "schema": DETAILS_SCHEMA,
                "bounty_ref": bounty_ref,
                "title": title,
                "summary": clip_bytes(&full, SUMMARY_MAX_BYTES),
                "description": scope,
                "category": map_category(b["category"].as_str().unwrap_or_default())?,
                "capabilities": caps,
                "tier": b["reward_tier"],
                "priority": b["priority"],
                "acceptance_criteria": ac,
                "evidence_required": ev,
                "deadline": deadline,
                "reward_quote": {
                    "asset": asset, "denom": denom, "amount": amount.to_string(),
                    "params_version": pv, "label": "proposed, pending vote"
                }
            });
            let bytes = canonical_json(&doc).into_bytes();
            let sha256 = sha256_hex(&bytes)?;
            out.push(SeedDoc {
                bounty_ref,
                catalog_id: id.into(),
                denom,
                amount,
                doc,
                bytes,
                sha256,
            });
        }
    }
    Ok((pv, out))
}

// ---------------- export-seed ----------------
pub fn export_seed(
    pack: &Path,
    catalog_path: &Path,
    horizon: &str,
    deadline: &str,
    public_base: &str,
    payout: &Payout,
) -> Result<()> {
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(catalog_path).with_context(|| format!("read {}", catalog_path.display()))?,
    )?;
    let catalog_sha = sha256_hex(&std::fs::read(catalog_path)?)?;
    let (pv, docs) = build_docs(&catalog, horizon, deadline, payout)?;
    let expires_at = rfc3339_utc(&format!("{deadline}T00:00:00Z"))?;
    let dir = pack.join("seed").join(&pv);
    std::fs::create_dir_all(&dir)?;
    let mut entries = vec![];
    for d in &docs {
        std::fs::write(dir.join(format!("{}.json", d.sha256)), &d.bytes)?;
        let key = format!("{DEFAULT_KEY_PREFIX}/{pv}/{}.json", d.sha256);
        entries.push(json!({
            "id": d.bounty_ref, "catalog_id": d.catalog_id, "sha256": d.sha256, "bytes": d.bytes.len(),
            "key": key, "url": format!("{}/{}", public_base.trim_end_matches('/'), key.trim_start_matches("static/")),
            "title": d.doc["title"], "summary": d.doc["summary"],
            "reward": {"denom": d.denom, "amount": d.amount.to_string()},
            "deadline": deadline, "expires_at_unix": expires_at,
        }));
    }
    let totals = totals_of(docs.iter().map(|d| (d.denom.as_str(), d.amount)));
    let manifest = json!({
        "schema": SEED_SCHEMA, "params_version": pv, "horizon": horizon,
        "catalog_sha256": catalog_sha, "deadline": deadline,
        "items": docs.iter().map(|d| d.catalog_id.clone()).collect::<std::collections::BTreeSet<_>>().len(),
        "bounties": entries, "totals": totals,
        "label": "proposed, pending vote",
        "payout": match payout {
            Payout::Single { denom, price_usd } => json!({"mode": "single", "denom": denom, "terp_usd": price_usd,
                "price_source": "catalog meta baseline (Prop 61), amount = floor(window_commitment.usd / terp_usd * 1e6)"}),
            Payout::Split => json!({"mode": "split", "source": "window_commitment.TERP_micro/THIOL_micro"}),
        },
    });
    let mbytes = canonical_json(&manifest).into_bytes();
    let msha = sha256_hex(&mbytes)?;
    std::fs::write(dir.join("manifest.json"), &mbytes)?;
    std::fs::write(
        dir.join("manifest.sha256"),
        format!("{msha}  manifest.json\n"),
    )?;
    println!(
        "export-seed: {} docs ({} items) -> {}",
        docs.len(),
        manifest["items"],
        dir.display()
    );
    println!("totals: {}", serde_json::to_string(&totals)?);
    println!("manifest sha256 {msha}");
    Ok(())
}

pub fn totals_of<'a>(it: impl Iterator<Item = (&'a str, u128)>) -> BTreeMap<String, String> {
    let mut t: BTreeMap<String, u128> = BTreeMap::new();
    for (d, a) in it {
        *t.entry(d.to_string()).or_default() += a;
    }
    t.into_iter().map(|(k, v)| (k, v.to_string())).collect()
}

pub fn load_manifest(pack: &Path, pv: &str) -> Result<(Value, std::path::PathBuf)> {
    let dir = pack.join("seed").join(pv);
    let m: Value = serde_json::from_slice(
        &std::fs::read(dir.join("manifest.json"))
            .with_context(|| format!("read {}/manifest.json (run export-seed)", dir.display()))?,
    )?;
    // every file must still hash to its name
    for e in m["bounties"]
        .as_array()
        .ok_or_else(|| anyhow!("manifest.bounties"))?
    {
        let sha = e["sha256"].as_str().unwrap_or_default();
        let bytes = std::fs::read(dir.join(format!("{sha}.json")))?;
        if sha256_hex(&bytes)? != sha {
            bail!("{sha}.json does not hash to its name");
        }
    }
    Ok((m, dir))
}

// ---------------- upload-seed ----------------
/// Uploads via the `mc` client and an alias that already exists (mc config or MC_HOST_<alias>).
/// The alias name comes from BOUNTY_MC_ALIAS; credentials are never read or printed by this tool.
pub fn upload_seed(pack: &Path, pv: &str, execute: bool, check_remote: bool) -> Result<()> {
    let (m, dir) = load_manifest(pack, pv)?;
    let mut files: Vec<(String, std::path::PathBuf, usize, String)> = vec![];
    for e in m["bounties"].as_array().unwrap() {
        let sha = e["sha256"].as_str().unwrap().to_string();
        files.push((
            e["key"].as_str().unwrap().to_string(),
            dir.join(format!("{sha}.json")),
            e["bytes"].as_u64().unwrap() as usize,
            sha,
        ));
    }
    let mbytes = std::fs::read(dir.join("manifest.json"))?;
    let msha = sha256_hex(&mbytes)?;
    files.push((
        format!("{DEFAULT_KEY_PREFIX}/{pv}/manifest-{msha}.json"),
        dir.join("manifest.json"),
        mbytes.len(),
        msha,
    ));
    let alias = std::env::var("BOUNTY_MC_ALIAS").unwrap_or_default();
    let gated = execute && std::env::var("BOUNTY_UPLOAD_EXECUTE").ok().as_deref() == Some("1");
    if execute && !gated {
        bail!("refusing to upload: --execute also needs BOUNTY_UPLOAD_EXECUTE=1");
    }
    if gated && alias.is_empty() {
        bail!("BOUNTY_MC_ALIAS (an existing mc alias name) is required to upload");
    }
    let mut plan = vec![];
    let total: usize = files.iter().map(|f| f.2).sum();
    for (key, path, size, sha) in &files {
        let remote = if gated || (check_remote && !alias.is_empty()) {
            mc_stat(&alias, key)?
        } else {
            None
        };
        let action = match remote {
            Some(rsize) if rsize == *size as u64 => "skip (exists, same hash key and size)",
            Some(_) => bail!(
                "{key} exists with a different size: refusing to overwrite a content-addressed key"
            ),
            None if gated => "upload",
            None => "would upload",
        };
        println!("{action:>40}  {size:>6} B  sha256 {sha}  {key}");
        plan.push(json!({"key": key, "bytes": size, "sha256": sha, "action": action}));
        if action == "upload" {
            let st = std::process::Command::new("mc")
                .args([
                    "cp",
                    "--quiet",
                    "--attr",
                    &format!("x-amz-meta-sha256={sha}"),
                ])
                .arg(path)
                .arg(format!("{alias}/{key}"))
                .stdout(std::process::Stdio::null())
                .status()
                .context("run mc")?;
            if !st.success() {
                bail!("mc cp failed for {key}");
            }
            if mc_stat(&alias, key)? != Some(*size as u64) {
                bail!("post-upload size check failed for {key}");
            }
        }
    }
    println!(
        "{} objects, {total} bytes, prefix {DEFAULT_KEY_PREFIX}/{pv}/",
        files.len()
    );
    let out = json!({"_NOT_UPLOADED": !gated, "params_version": pv, "alias_env": "BOUNTY_MC_ALIAS",
        "objects": plan, "total_bytes": total, "manifest_sha256": files.last().unwrap().3});
    let p = pack
        .join("cw-orch-preview")
        .join(format!("05-upload-seed-{pv}.json"));
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, serde_json::to_string_pretty(&out)? + "\n")?;
    println!("wrote {}", p.display());
    if !gated {
        println!(
            "mode: PREVIEW (nothing uploaded). Upload needs --execute, BOUNTY_UPLOAD_EXECUTE=1 and BOUNTY_MC_ALIAS."
        );
    }
    Ok(())
}

/// `mc stat --json`: Some(size) when the object exists. Output is parsed, never echoed.
fn mc_stat(alias: &str, key: &str) -> Result<Option<u64>> {
    let o = std::process::Command::new("mc")
        .args(["stat", "--json"])
        .arg(format!("{alias}/{key}"))
        .output()
        .context("run mc (is it installed?)")?;
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
    if o.status.success() && v["status"] == "success" {
        Ok(v["size"].as_u64())
    } else {
        Ok(None)
    }
}

// ---------------- create-bounties ----------------
pub struct Limits {
    pub calls: u64,
    pub grant: BTreeMap<String, u128>,
    pub cap: BTreeMap<String, u128>,
    pub rule: BTreeMap<String, (u128, u128)>,
}

pub fn limits_from(cfg: &Value) -> Result<Limits> {
    // The live on-chain grant (config.live_grant, FIAB proposal 4) wins over the planned 02 grant.
    let g = if cfg["live_grant"].is_object() {
        &cfg["live_grant"]
    } else {
        &cfg["grant"]
    };
    let mut grant = BTreeMap::new();
    for c in g["amounts"]
        .as_array()
        .ok_or_else(|| anyhow!("config.grant.amounts"))?
    {
        grant.insert(
            c["denom"].as_str().unwrap().to_string(),
            c["amount"].as_str().unwrap().parse()?,
        );
    }
    let mut rule = BTreeMap::new();
    // Prefer the live escrow's denom rules (config.live_escrow_denoms) over the planned instantiate_msg.
    let denoms = if cfg["live_escrow_denoms"].is_array() {
        &cfg["live_escrow_denoms"]
    } else {
        &cfg["instantiate_msg"]["denoms"]
    };
    for d in denoms.as_array().ok_or_else(|| anyhow!("config denoms"))? {
        rule.insert(
            d["denom"].as_str().unwrap().to_string(),
            (
                d["min_amount"].as_str().unwrap().parse()?,
                d["max_amount"].as_str().unwrap().parse()?,
            ),
        );
    }
    let cap = BTreeMap::from([
        ("uterp".to_string(), CAP_UTERP),
        ("uthiol".to_string(), CAP_UTHIOL),
    ]);
    Ok(Limits {
        calls: g["calls_remaining"]
            .as_u64()
            .ok_or_else(|| anyhow!("calls_remaining"))?,
        grant,
        cap,
        rule,
    })
}

/// Enforce cumulative calls / per-denom grant limits and the two-month cap after `spent`.
pub fn enforce(
    l: &Limits,
    spent_calls: u64,
    spent: &BTreeMap<String, u128>,
    rewards: &[(String, u128)],
) -> Result<BTreeMap<String, u128>> {
    if spent_calls + rewards.len() as u64 > l.calls {
        bail!(
            "calls {} + {} > grant calls_remaining {}",
            spent_calls,
            rewards.len(),
            l.calls
        );
    }
    let mut tot = spent.clone();
    for (d, a) in rewards {
        let (min, max) = l
            .rule
            .get(d)
            .ok_or_else(|| anyhow!("denom {d} not allowed by the escrow"))?;
        if a < min || a > max {
            bail!("{a}{d} outside escrow range [{min}, {max}]");
        }
        *tot.entry(d.clone()).or_default() += a;
    }
    for (d, t) in &tot {
        let g = l.grant.get(d).copied().unwrap_or(0);
        if *t > g {
            bail!("{d} total {t} > grant limit {g}");
        }
        let c = l.cap.get(d).copied().unwrap_or(0);
        if *t > c {
            bail!("{d} total {t} > two-month cap {c}");
        }
    }
    Ok(tot)
}

pub fn create_bounty_msg(e: &Value) -> Value {
    json!({"create_bounty": {
        "bounty_ref": e["id"], "title": e["title"], "summary": e["summary"],
        "details_hash": e["sha256"], "details_cid": null,
        "reward": e["reward"],
        // cosmwasm Timestamp = nanoseconds as a decimal string
        "expires_at": format!("{}", e["expires_at_unix"].as_i64().unwrap_or(0) as i128 * 1_000_000_000),
    }})
}

/// Inner MsgExec(grantee = member){ MsgExecuteContract(sender = core, escrow, create_bounty, [reward]) ... }.
/// cw-orch-kms adds the outer MsgExec(grantee = session key) at broadcast.
pub fn inner_exec(
    core: &str,
    member: &str,
    escrow: &str,
    entries: &[Value],
) -> Result<(MsgExec, Vec<Value>)> {
    let mut anys = vec![];
    let mut views = vec![];
    for e in entries {
        let m = create_bounty_msg(e);
        let r = &e["reward"];
        let funds = vec![Coin {
            denom: r["denom"].as_str().unwrap().into(),
            amount: r["amount"].as_str().unwrap().into(),
        }];
        let x = MsgExecuteContract {
            sender: core.into(),
            contract: escrow.into(),
            msg: serde_json::to_vec(&m)?,
            funds,
        };
        anys.push(any(T_EXEC, &x));
        views.push(
            json!({"@type": T_EXEC, "sender": core, "contract": escrow, "msg": m, "funds": [r]}),
        );
    }
    Ok((
        MsgExec {
            grantee: member.into(),
            msgs: anys,
        },
        views,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn create_bounties_plans(
    cfg: &Value,
    core: &str,
    member: &str,
    escrow: &str,
    pack: &Path,
    pv: &str,
    batch_size: usize,
    spent_calls: u64,
    spent: &BTreeMap<String, u128>,
    now_unix: i64,
) -> Result<Vec<Plan>> {
    let (m, _) = load_manifest(pack, pv)?;
    let entries = m["bounties"].as_array().unwrap().clone();
    let lim = limits_from(cfg)?;
    let all: Vec<(String, u128)> = entries
        .iter()
        .map(|e| {
            Ok((
                e["reward"]["denom"].as_str().unwrap().to_string(),
                e["reward"]["amount"].as_str().unwrap().parse()?,
            ))
        })
        .collect::<Result<_>>()?;
    enforce(&lim, spent_calls, spent, &all)?; // whole seed must fit before any batch is built
    // expiry window: contract needs now+min <= expires_at <= now+max at inclusion time
    let im = &cfg["instantiate_msg"];
    let (mn, mx) = (
        im["min_duration_secs"].as_i64().unwrap_or(86400),
        im["max_duration_secs"].as_i64().unwrap_or(7776000),
    );
    let margin = cfg["create_bounty_inclusion_margin_secs"]
        .as_i64()
        .unwrap_or(21600);
    let g = if cfg["live_grant"].is_object() {
        &cfg["live_grant"]
    } else {
        &cfg["grant"]
    };
    let grant_exp = rfc3339_utc(
        g["expiration_rfc3339"]
            .as_str()
            .unwrap_or("1970-01-01T00:00:00Z"),
    )?;
    if now_unix + margin > grant_exp {
        bail!("grant expires before inclusion");
    }
    for e in &entries {
        let x = e["expires_at_unix"].as_i64().unwrap_or(0);
        if x < now_unix + margin + mn || x > now_unix + mx {
            bail!(
                "{}: expires_at {x} outside [now+min+margin, now+max]",
                e["id"]
            );
        }
    }
    let mut plans = vec![];
    let (mut calls, mut run) = (spent_calls, spent.clone());
    for (i, chunk) in entries.chunks(batch_size.max(1)).enumerate() {
        let rewards: Vec<(String, u128)> =
            all[i * batch_size..i * batch_size + chunk.len()].to_vec();
        run = enforce(&lim, calls, &run, &rewards)?;
        calls += chunk.len() as u64;
        let (exec, views) = inner_exec(core, member, escrow, chunk)?;
        let bt = totals_of(rewards.iter().map(|(d, a)| (d.as_str(), *a)));
        let after: BTreeMap<String, String> = run
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect();
        plans.push(Plan {
            step: format!("05-create-bounties-{pv}-batch-{:02}", i + 1),
            signer: member.into(),
            note: format!("nested: MsgExec(session){{MsgExec(member){{{} x MsgExecuteContract(core) create_bounty}}}}; batch totals {bt:?}; cumulative calls {calls}/{} totals {after:?}", chunk.len(), lim.calls),
            session: json!({"nested_exec": true, "type_urls": [T_AUTHZ_EXEC],
                "unsigned_tx": unsigned_tx(&exec, &views, member, pv, i + 1)}),
            msgs: vec![(any(T_AUTHZ_EXEC, &exec), json!({"@type": T_AUTHZ_EXEC, "grantee": member, "msgs": views}))],
            kind: SessionKind::NestedExec,
        });
    }
    Ok(plans)
}

pub const SESSION_PLACEHOLDER: &str = "<SESSION_KEY: generated in memory at --execute>";

/// Full unsigned tx (proto-JSON) as broadcast: outer MsgExec by the session key, fee via the session feegrant.
pub fn unsigned_tx(
    inner: &MsgExec,
    views: &[Value],
    member: &str,
    pv: &str,
    batch: usize,
) -> Value {
    json!({
        "body": {
            "messages": [{"@type": T_AUTHZ_EXEC, "grantee": SESSION_PLACEHOLDER,
                "msgs": [{"@type": T_AUTHZ_EXEC, "grantee": member, "msgs": views}]}],
            "memo": format!("fiab-bounty-escrow seed {pv} batch {batch}"),
            "timeout_height": "0", "extension_options": [], "non_critical_extension_options": []
        },
        "auth_info": {"signer_infos": [{"public_key": SESSION_PLACEHOLDER, "mode_info": {"single": {"mode": "SIGN_MODE_DIRECT"}}, "sequence": "0"}],
            "fee": {"amount": "<from simulate, uterp>", "gas_limit": "<from simulate>", "payer": "", "granter": member}},
        "signatures": [],
        "inner_msg_exec_proto_hex": hexenc(inner.encode_to_vec()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_sdk_proto::prost::Message;

    fn cat() -> Value {
        json!({"meta": {"pricing": {"params_version": "pv1"}}, "bounties": [
            {"id": "TB-X-001", "horizon": "Now", "visibility": "public", "title": "T", "summary": "s ".repeat(200),
             "category": "chain-ops", "reward_tier": "T2", "priority": "P1", "skills": ["Cosmos-SDK", "go"],
             "acceptance_criteria": ["a"], "evidence_required": ["PR"],
             "reward": {"params_version": "pv1", "window_commitment": {"units": 1, "stipend_months": 0, "usd": 15.6, "TERP_micro": "1000000", "THIOL_micro": "2000000"}}},
            {"id": "TB-X-002", "horizon": "Later", "visibility": "public"}]})
    }

    #[test]
    fn canonical_matches_lib_mjs_rules() {
        let v = json!({"b": 1, "a": {"d": [1, "x\n\u{e9}"], "c": null}});
        assert_eq!(
            canonical_json(&v),
            "{\"a\":{\"c\":null,\"d\":[1,\"x\\n\u{e9}\"]},\"b\":1}"
        );
    }

    #[test]
    fn export_hashing_is_content_addressed_and_stable() {
        let (pv, d1) = build_docs(&cat(), "Now", "2027-01-01", &Payout::Split).unwrap();
        let (_, d2) = build_docs(&cat(), "Now", "2027-01-01", &Payout::Split).unwrap();
        assert_eq!(pv, "pv1");
        assert_eq!(d1.len(), 2); // one doc per denom, Later item excluded
        assert_eq!(d1[0].bounty_ref, "TB-X-001:uterp");
        for (a, b) in d1.iter().zip(&d2) {
            assert_eq!(a.sha256, b.sha256);
            assert_eq!(a.sha256, sha256_hex(&a.bytes).unwrap());
            assert_eq!(a.sha256.len(), 64);
        }
        assert_ne!(d1[0].sha256, d1[1].sha256);
        assert!(d1[0].doc["summary"].as_str().unwrap().len() <= SUMMARY_MAX_BYTES);
        assert_eq!(d1[0].doc["category"], "ops");
        // known vector: sha256("abc")
        assert_eq!(
            sha256_hex(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let (_, d3) = build_docs(&cat(), "Now", "2027-01-02", &Payout::Split).unwrap();
        assert_ne!(d1[0].sha256, d3[0].sha256);
    }

    fn single() -> Payout {
        Payout::Single {
            denom: "uterp".into(),
            price_usd: "0.05172".into(),
        }
    }

    #[test]
    fn single_denom_terp_payout() {
        assert_eq!(usd_to_micro("15.6", "0.05172").unwrap(), 301_624_129); // == catalog amounts_if_single_asset
        assert_eq!(usd_to_micro("8.8", "0.05172").unwrap(), 170_146_945);
        assert_eq!(usd_to_micro("140.8", "0.05172").unwrap(), 2_722_351_121);
        let (_, d) = build_docs(&cat(), "Now", "2027-01-01", &single()).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].bounty_ref, "TB-X-001");
        assert_eq!((d[0].denom.as_str(), d[0].amount), ("uterp", 301_624_129));
        assert_eq!(d[0].doc["reward_quote"]["denom"], "uterp");
        assert!(
            build_docs(
                &cat(),
                "Now",
                "2027-01-01",
                &Payout::Single {
                    denom: "uthiol".into(),
                    price_usd: "1".into()
                }
            )
            .is_err()
        );
        // 20 x the largest window item stays far under the 50k TERP cap
        let l = lim();
        assert!(
            enforce(
                &l,
                0,
                &BTreeMap::new(),
                &vec![("uterp".to_string(), 2_722_351_121u128); 18]
            )
            .is_ok()
        );
    }

    fn lim() -> Limits {
        Limits {
            calls: 50,
            grant: BTreeMap::from([
                ("uterp".into(), 100_000_000_000),
                ("uthiol".into(), 100_000_000_000),
            ]),
            cap: BTreeMap::from([("uterp".into(), CAP_UTERP), ("uthiol".into(), CAP_UTHIOL)]),
            rule: BTreeMap::from([
                ("uterp".into(), (1_000_000, 100_000_000_000)),
                ("uthiol".into(), (1_000_000, 100_000_000_000)),
            ]),
        }
    }

    #[test]
    fn cap_enforcement() {
        let l = lim();
        let ok = vec![
            ("uterp".to_string(), 3_873_051_277u128),
            ("uthiol".to_string(), 7_746_126_103),
        ];
        assert!(enforce(&l, 0, &BTreeMap::new(), &ok).is_ok());
        // two-month cap binds TERP before the 100k grant
        let over = vec![("uterp".to_string(), 50_000_000_001u128)];
        assert!(
            enforce(&l, 0, &BTreeMap::new(), &over)
                .unwrap_err()
                .to_string()
                .contains("cap")
        );
        let spent = BTreeMap::from([("uterp".to_string(), 49_000_000_000u128)]);
        assert!(enforce(&l, 0, &spent, &[("uterp".into(), 1_000_000_001)]).is_err());
        assert!(enforce(&l, 0, &spent, &[("uterp".into(), 1_000_000_000)]).is_ok());
        assert!(
            enforce(&l, 49, &BTreeMap::new(), &ok)
                .unwrap_err()
                .to_string()
                .contains("calls")
        );
        assert!(enforce(&l, 0, &BTreeMap::new(), &[("uterp".into(), 999_999)]).is_err());
        assert!(enforce(&l, 0, &BTreeMap::new(), &[("uatom".into(), 1_000_000)]).is_err());
    }

    #[test]
    fn nested_msg_encoding() {
        let e = json!({"id": "TB-X-001:uterp", "title": "T", "summary": "s", "sha256": "ab".repeat(32),
            "reward": {"denom": "uterp", "amount": "1000000"}, "expires_at_unix": 1798761600});
        let (inner, _) =
            inner_exec("terp1core", "terp1member", "terp1escrow", &[e.clone(), e]).unwrap();
        let outer = MsgExec {
            grantee: "terp1session".into(),
            msgs: vec![any(T_AUTHZ_EXEC, &inner)],
        };
        let o = MsgExec::decode(outer.encode_to_vec().as_slice()).unwrap();
        assert_eq!(o.grantee, "terp1session");
        assert_eq!(o.msgs[0].type_url, T_AUTHZ_EXEC);
        let i = MsgExec::decode(o.msgs[0].value.as_slice()).unwrap();
        assert_eq!(i.grantee, "terp1member");
        assert_eq!(i.msgs.len(), 2);
        assert_eq!(i.msgs[0].type_url, T_EXEC);
        let x = MsgExecuteContract::decode(i.msgs[0].value.as_slice()).unwrap();
        assert_eq!(
            (x.sender.as_str(), x.contract.as_str()),
            ("terp1core", "terp1escrow")
        );
        assert_eq!(
            x.funds,
            vec![Coin {
                denom: "uterp".into(),
                amount: "1000000".into()
            }]
        );
        let m: Value = serde_json::from_slice(&x.msg).unwrap();
        let cb = &m["create_bounty"];
        assert_eq!(cb["reward"], json!({"denom": "uterp", "amount": "1000000"}));
        assert_eq!(cb["expires_at"], "1798761600000000000");
        assert_eq!(cb["details_hash"].as_str().unwrap().len(), 64);
        assert!(cb.get("proposer").is_none());
    }
}
