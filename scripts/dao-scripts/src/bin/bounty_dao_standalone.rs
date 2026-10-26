//! Standalone bounty DAO plan (dry-run only). Superseded by `bounty_dao`
//! (BOUNTY-SUBDAO under the Foundation); kept for reference.
//!
//! Builds the dao-dao-core `MsgInstantiateContract2` for the Terp bounty DAO
//! (cw4 voting, single-choice proposals, modules admin = CoreModule) with the
//! DAO itself as core wasm admin (predicted instantiate2 address), and the
//! cw-orch-kms session scope the ephemeral sender would need
//! (`MsgInstantiateContract2` only, ttl-bounded, fee-capped).
//! It never boots a granter page and never broadcasts. The live path is gated off.
//!
//! ```sh
//! cargo run -p dao-scripts --bin bounty_dao -- --codes 1,2,3,4 \
//!   --core-checksum <hex> --creator terp1... --member terp1...:1 --out bounty_dao.plan.json
//! ```

use anyhow::{anyhow, Result};
use base64::Engine;
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(name = "bounty-dao-standalone")]
struct Cli {
    /// core,voting_cw4,cw4_group,proposal_single code ids on morocco-1.
    #[clap(long, value_delimiter = ',')]
    codes: Vec<u64>,
    /// sha256 of the dao-dao-core wasm (hex) — needed to predict the address.
    #[clap(long)]
    core_checksum: Option<String>,
    /// cw-orch-kms ephemeral sender that will instantiate2 (the creator).
    #[clap(long)]
    creator: Option<String>,
    /// terp1...:weight, repeatable.
    #[clap(long = "member")]
    members: Vec<String>,
    #[clap(long, default_value_t = 1)]
    threshold: u64,
    #[clap(long, default_value = "terp-bounty-dao")]
    name: String,
    #[clap(long, default_value = "terp-bounty-dao-v1")]
    salt: String,
    /// Voting period seconds.
    #[clap(long, default_value_t = 3 * 24 * 3600)]
    voting_secs: u64,
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,
    #[clap(long, default_value = "bounty_dao.plan.json")]
    out: String,
    /// Refused in this build. Going live needs the approvals listed in the plan.
    #[clap(long)]
    live: bool,
}

fn b64(v: &serde_json::Value) -> String {
    base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(v).expect("json"))
}

fn parse_members(raw: &[String]) -> Result<Vec<serde_json::Value>> {
    if raw.is_empty() {
        return Err(anyhow!("at least one --member terp1...:weight"));
    }
    raw.iter()
        .map(|m| {
            let (addr, w) = m.split_once(':').ok_or_else(|| anyhow!("member {m}: want addr:weight"))?;
            cw_orch_kms::validate_terp_address(addr).map_err(|e| anyhow!(e))?;
            let weight: u64 = w.parse().map_err(|_| anyhow!("member {m}: bad weight"))?;
            if weight == 0 {
                return Err(anyhow!("member {m}: weight 0"));
            }
            Ok(serde_json::json!({"addr": addr, "weight": weight}))
        })
        .collect()
}

fn dao_init(cli: &Cli, members: Vec<serde_json::Value>) -> Result<serde_json::Value> {
    let [_, voting, group, proposal] = cli.codes[..] else {
        return Err(anyhow!("--codes core,voting_cw4,cw4_group,proposal_single"));
    };
    let total: u64 = members.iter().map(|m| m["weight"].as_u64().unwrap_or(0)).sum();
    if cli.threshold == 0 || cli.threshold > total {
        return Err(anyhow!("threshold {} must be 1..={total}", cli.threshold));
    }
    Ok(serde_json::json!({
        "admin": null,
        "name": cli.name,
        "description": "Terp bounty DAO: owner and wasm admin of the bounty escrow and auction contracts.",
        "image_url": null,
        "automatically_add_cw20s": true,
        "automatically_add_cw721s": true,
        "voting_module_instantiate_info": {
            "code_id": voting,
            "msg": b64(&serde_json::json!({"group_contract": {"new": {
                "cw4_group_code_id": group, "cw4_group_salt": null, "initial_members": members}}})),
            "admin": {"core_module": {}}, "funds": null,
            "label": format!("{}-voting", cli.name), "salt": null
        },
        "proposal_modules_instantiate_info": [{
            "code_id": proposal,
            "msg": b64(&serde_json::json!({
                "threshold": {"absolute_count": {"threshold": cli.threshold.to_string()}},
                "max_voting_period": {"time": cli.voting_secs},
                "min_voting_period": null,
                "only_members_execute": true,
                "allow_revoting": false,
                "pre_propose_info": {"anyone_may_propose": {}},
                "close_proposal_on_execution_failure": true,
                "veto": null,
                "delegation_module": null
            })),
            "admin": {"core_module": {}}, "funds": null,
            "label": format!("{}-proposal", cli.name), "salt": null
        }],
        "initial_items": null,
        "initial_actions": null,
        "dao_uri": null
    }))
}

fn predicted(cli: &Cli) -> Result<Option<String>> {
    let (Some(sum), Some(creator)) = (&cli.core_checksum, &cli.creator) else {
        return Ok(None);
    };
    cw_orch_kms::validate_terp_address(creator).map_err(|e| anyhow!(e))?;
    let sum = (0..sum.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(sum.get(i..i + 2).unwrap_or("zz"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| anyhow!("--core-checksum is not hex"))?;
    if sum.len() != 32 {
        return Err(anyhow!("--core-checksum must be 32 bytes"));
    }
    let (_, creator_bytes) = bech32::decode(creator).map_err(|e| anyhow!("{e}"))?;
    let addr = cw_orch_kms::instantiate2_bech32("terp", &sum, &creator_bytes, cli.salt.as_bytes())
        .map_err(|e| anyhow!(e))?;
    Ok(Some(addr))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.live {
        return Err(anyhow!(
            "live creation is disabled: review the dry-run plan, then get the approvals it lists"
        ));
    }
    cw_orch_kms::validate_ttl(cli.ttl_secs).map_err(|e| anyhow!(e))?;
    let members = parse_members(&cli.members)?;
    let init = dao_init(&cli, members)?;
    let dao = predicted(&cli)?;
    let admin = dao.clone().unwrap_or_else(|| "<predicted DAO core address: pass --core-checksum and --creator>".into());
    let plan = serde_json::json!({
        "dry_run": true,
        "broadcast": false,
        "chain_id": "morocco-1",
        "predicted_dao": dao,
        "msg": {"@type": "/cosmwasm.wasm.v1.MsgInstantiateContract2",
            "sender": cli.creator.clone().unwrap_or_else(|| "<cw-orch-kms ephemeral sender>".into()),
            "admin": admin, "code_id": cli.codes.first(), "label": cli.name,
            "msg": init, "funds": [], "salt": base64::engine::general_purpose::STANDARD.encode(cli.salt.as_bytes()),
            "fix_msg": false},
        "cw_orch_kms_session": {"factor": "zafu/zigner cold key signs cw-orch-granter:v2 challenge (purpose=bounty-dao, chain-id=morocco-1)",
            "type_urls": ["/cosmwasm.wasm.v1.MsgInstantiateContract2"], "execute": [], "ttl_secs": cli.ttl_secs,
            "fee_allowance_txs": 2, "revoke_after": true},
        "then": "app_setup --bounty-dao <predicted_dao> --dry-run, review, then live with approval",
    });
    std::fs::write(&cli.out, serde_json::to_string_pretty(&plan)?)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> String {
        bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("terp").unwrap(), &[b; 20]).unwrap()
    }

    fn cli(members: Vec<String>, threshold: u64) -> Cli {
        Cli::parse_from(
            ["bounty_dao", "--codes", "1,2,3,4", "--threshold", &threshold.to_string()]
                .into_iter()
                .map(String::from)
                .chain(members.into_iter().flat_map(|m| ["--member".to_string(), m])),
        )
    }

    #[test]
    fn modules_are_admin_by_core_and_threshold_is_bounded() {
        let c = cli(vec![format!("{}:1", addr(1)), format!("{}:1", addr(2))], 2);
        let init = dao_init(&c, parse_members(&c.members).unwrap()).unwrap();
        assert_eq!(init["voting_module_instantiate_info"]["admin"], serde_json::json!({"core_module": {}}));
        assert_eq!(init["proposal_modules_instantiate_info"][0]["admin"], serde_json::json!({"core_module": {}}));
        let c = cli(vec![format!("{}:1", addr(1))], 2);
        assert!(dao_init(&c, parse_members(&c.members).unwrap()).is_err());
    }

    #[test]
    fn bad_members_refused() {
        assert!(parse_members(&[]).is_err());
        assert!(parse_members(&["cosmos1x:1".into()]).is_err());
        assert!(parse_members(&[format!("{}:0", addr(1))]).is_err());
    }
}
