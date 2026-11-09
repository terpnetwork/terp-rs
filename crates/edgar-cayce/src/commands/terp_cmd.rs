use crate::state::Context;
use crate::terp::{bounty as bounty_mod, dao as dao_mod, digest, gov as gov_mod, profile};

/// Terp — Terp Network bot engine (style, digests, RLM docs)
#[poise::command(
    slash_command,
    subcommands(
        "ask_terp",
        "profile_refresh",
        "digest_preview",
        "verify_answer",
        "verified_list",
        "corpus_status"
    )
)]
pub async fn terp(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

/// Ask terp docs (RLM, styled in returniflost voice)
#[poise::command(slash_command, guild_only, rename = "ask")]
pub async fn ask_terp(
    ctx: Context<'_>,
    #[description = "Your question"] question: String,
    #[description = "Topic label (default: terp)"] topic: Option<String>,
) -> Result<(), anyhow::Error> {
    let Some(admitted) = super::ask::begin_ask(&ctx).await? else {
        return Ok(());
    };
    let topic = topic.unwrap_or_else(|| "terp".to_string());
    let user_mention = format!("<@{}>", ctx.author().id);
    ctx.say(format!(
        "on it — digging through {topic} docs, {user_mention}"
    ))
    .await?;
    let cfg = ctx.data().rlm_config.read().await;
    let (mi, mce, mal, pl) = (
        cfg.max_iterations,
        cfg.min_code_executions,
        cfg.min_answer_len,
        cfg.parallel_loops,
    );
    drop(cfg);
    // The voice hint is model input. It goes into the loop prompt and never
    // into the Discord reply.
    let corpora = profile::load_returniflost_corpus(std::path::Path::new("./data/docs"));
    let hint = if corpora.is_empty() {
        profile::STYLE_PREAMBLE.to_string()
    } else {
        profile::StyleProfile::from_messages(&corpora).style_hint()
    };
    let result = match ctx
        .data()
        .rlm
        .query_styled(&topic, &question, mi, mce, mal, pl, None, Some(&hint))
        .await
    {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(error = format!("{e:#}"), "RLM query failed");
            super::ask::finish_ask(&admitted, "error");
            ctx.say(format!(
                "{user_mention} research failed (models timed out or missing)."
            ))
            .await?;
            return Ok(());
        }
    };
    super::ask::record_gate_verdict(&admitted, result.gate_verdict.as_deref());
    let blocked = crate::rlm::gate::blocked_outcome(result.gate_verdict.as_deref());
    let full = render_ask_reply(&user_mention, &question, &result.answer, &hint, &result.cited_urls);
    let sent = crate::commands::send_chunked_public(&ctx, &full).await;
    super::ask::finish_ask(
        &admitted,
        match (&sent, blocked) {
            (Err(_), _) => "error",
            (Ok(_), Some(outcome)) => outcome,
            (Ok(_), None) => "ok",
        },
    );
    sent
}

/// The public `/terp ask` reply. `hint` is only used to scrub a model echo of
/// the style notes; it is never appended.
fn render_ask_reply(
    user_mention: &str,
    question: &str,
    answer: &str,
    hint: &str,
    cited_urls: &[String],
) -> String {
    let answer = scrub_style_echo(answer, hint);
    let mut full = format!("{user_mention} **Q:** {question}\n\n{answer}");
    if !cited_urls.is_empty() {
        full.push_str("\n\n**Sources:**\n");
        for u in cited_urls {
            full.push_str(&format!("- <{u}>\n"));
        }
    }
    full
}

fn scrub_style_echo(answer: &str, hint: &str) -> String {
    let mut out = answer.to_string();
    for needle in [hint.trim(), profile::STYLE_PREAMBLE.trim()] {
        if !needle.is_empty() {
            out = out.replace(&format!("_{needle}_"), "").replace(needle, "");
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod reply_tests {
    use super::*;

    #[test]
    fn style_hint_never_reaches_the_reply() {
        let hint = profile::STYLE_PREAMBLE;
        let r = render_ask_reply("<@1>", "q?", "validators secure the chain.", hint, &[]);
        assert!(!r.contains("Voice:"));
        assert!(!r.contains("Never invent numbers"));
        assert!(r.ends_with("validators secure the chain."));
        let echoed = format!("validators secure the chain.\n\n_{hint}_");
        let r = render_ask_reply("<@1>", "q?", &echoed, hint, &["https://docs.terp.network".into()]);
        assert!(!r.contains("Voice:"));
        assert!(r.contains("<https://docs.terp.network>"));
    }
}

/// Rebuild the returniflost style profile from data/style/returniflost.json
#[poise::command(slash_command, guild_only, rename = "profile-refresh")]
pub async fn profile_refresh(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let msgs = profile::load_returniflost_corpus(std::path::Path::new("./data/docs"));
    if msgs.is_empty() {
        ctx.say("no corpus found. drop messages into `data/style/returniflost.json` (array of strings or {content} objects).").await?;
        return Ok(());
    }
    let p = profile::StyleProfile::from_messages(&msgs);
    ctx.say(format!(
        "profile rebuilt: n={} avg_len={:.0} lowercase={:.2}\ntop: {}",
        p.message_count,
        p.avg_len,
        p.lowercase_ratio,
        p.top_tokens
            .iter()
            .take(10)
            .map(|(t, c)| format!("{t}({c})"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .await?;
    Ok(())
}

/// Preview today's dev digest
#[poise::command(slash_command, guild_only, rename = "digest-preview")]
pub async fn digest_preview(
    ctx: Context<'_>,
    #[description = "lines, | separated"] lines: Option<String>,
) -> Result<(), anyhow::Error> {
    let items: Vec<String> = lines
        .map(|s| s.split('|').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    ctx.say(digest::render_digest(&items)).await?;
    Ok(())
}

/// Promote a logged answer to verified (admin only)
#[poise::command(slash_command, guild_only, rename = "verify")]
pub async fn verify_answer(
    ctx: Context<'_>,
    #[description = "Topic label"] topic: String,
    #[description = "Exact question text as asked"] question: String,
) -> Result<(), anyhow::Error> {
    if !crate::commands::config::is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }
    let id = crate::docs::DocumentStore::qa_id(&topic, &question);
    match ctx.data().store.verify_qa(&topic, &id).await {
        Ok(true) => {
            ctx.say("verified. this answer may now serve as labeled context while its docs stay current.").await?;
        }
        _ => {
            ctx.say("no logged answer found for that topic+question.").await?;
        }
    }
    Ok(())
}

/// List verified, still-current answers for a topic.
#[poise::command(slash_command, guild_only, rename = "verified")]
pub async fn verified_list(
    ctx: Context<'_>,
    #[description = "Topic label"] topic: Option<String>,
) -> Result<(), anyhow::Error> {
    let topic = topic.unwrap_or_else(|| "terp".to_string());
    let items = ctx.data().store.list_verified(&topic, 10).await?;
    if items.is_empty() {
        ctx.say("no verified answers for this topic.").await?;
        return Ok(());
    }
    let mut msg = format!("**verified answers ({}):**\n", topic);
    for r in &items {
        let a: String = r.answer.chars().take(200).collect();
        msg.push_str(&format!("- **Q:** {}\n  {}\n", r.question, a));
    }
    crate::commands::send_chunked_public(&ctx, &msg).await
}

/// Show corpus commitment epoch + root (tamper-evident version log).
#[poise::command(slash_command, guild_only, rename = "corpus-status")]
pub async fn corpus_status(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let epoch = ctx.data().store.current_epoch().await.unwrap_or(-1);
    let root = ctx.data().store.corpus_root().await.unwrap_or_default();
    let short = if root.len() > 16 { &root[..16] } else { &root };
    ctx.say(format!("corpus epoch `{epoch}` root `{short}…`")).await?;
    Ok(())
}

/// Validator monitoring (Cosmos SDK, multi-chain)
#[poise::command(slash_command, subcommands("val_list", "val_proposals"))]
pub async fn validator(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

/// List the bonded set with liveness flags
#[poise::command(slash_command, guild_only, rename = "list")]
pub async fn val_list(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    // Pinned to the operator endpoint: callers never supply a base URL.
    let Some(rest) = crate::terp::node::base_url() else {
        ctx.say("node API not configured (TERP_NODE_REST).").await?;
        return Ok(());
    };
    let prefix = std::env::var("VALIDATOR_PREFIX").unwrap_or_else(|_| "terp".into());
    let client = reqwest::Client::new();
    let chain = crate::terp::validator::ChainConfig {
        name: "terp".into(),
        rest,
        prefix,
        missed_threshold: 50,
        validators: vec![],
    };
    match crate::terp::validator::fetch_all(&client, &chain).await {
        Ok(set) => {
            let mut msg = format!("**bonded validators ({})** 🟢 ok 🟡 missing 🔴 jailed 💀 tombstoned\n", set.len());
            for s in set.iter().take(60) {
                msg.push_str(&crate::terp::validator::format_row(s));
                msg.push('\n');
            }
            if set.len() > 60 {
                msg.push_str(&format!("…and {} more\n", set.len() - 60));
            }
            crate::commands::send_chunked_public(&ctx, &msg).await?;
        }
        Err(e) => {
            ctx.say(format!("fetch failed: {e}")).await?;
        }
    }
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "proposals")]
pub async fn val_proposals(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    fetch_proposals(&ctx).await
}

async fn fetch_proposals(ctx: &Context<'_>) -> Result<(), anyhow::Error> {
    let Some(rest) = crate::terp::node::base_url() else {
        ctx.say("node API not configured (TERP_NODE_REST).").await?;
        return Ok(());
    };
    let client = reqwest::Client::new();
    match gov_mod::active_proposals(&client, &rest).await {
        Ok(list) if list.is_empty() => {
            ctx.say("no active proposals.").await?;
        }
        Ok(list) => {
            let mut msg = String::new();
            for p in list.iter().take(10) {
                msg.push_str(&gov_mod::format_proposal("chain", p));
                msg.push('\n');
            }
            crate::commands::send_chunked_public(ctx, &msg).await?;
        }
        Err(e) => {
            ctx.say(format!("gov fetch failed: {e}")).await?;
        }
    }
    Ok(())
}

/// Governance proposal feed
#[poise::command(slash_command, subcommands("gov_active"))]
pub async fn gov(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "active")]
pub async fn gov_active(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    fetch_proposals(&ctx).await
}

/// DAO notifier (foundation + subDAOs only)
#[poise::command(slash_command, subcommands("dao_test", "dao_allowlist"))]
pub async fn dao(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "test")]
pub async fn dao_test(
    ctx: Context<'_>,
    #[description = "DAO address"] dao_addr: String,
    #[description = "Title"] title: String,
) -> Result<(), anyhow::Error> {
    let ev = dao_mod::DaoEvent {
        dao: dao_addr,
        kind: "proposal".into(),
        title,
        url: "https://daodao.zone".into(),
        status_key: "open".into(),
    };
    match dao_mod::format_event(&ev) {
        Some(m) => {
            ctx.say(m).await?;
        }
        None => {
            ctx.say("dropped: dao not in foundation allowlist.").await?;
        }
    }
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "allowlist")]
pub async fn dao_allowlist(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let list: Vec<String> = dao_mod::allowlist().into_iter().collect();
    ctx.say(format!(
        "**Allowed DAOs ({}):**\n{}",
        list.len(),
        list.join("\n")
    ))
    .await?;
    Ok(())
}

/// Bounty board
#[poise::command(
    slash_command,
    subcommands("bounty_post", "bounty_list", "bounty_claim")
)]
pub async fn bounty(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "post")]
pub async fn bounty_post(
    ctx: Context<'_>,
    #[description = "Title"] title: String,
    #[description = "Reward"] reward: String,
) -> Result<(), anyhow::Error> {
    let b = ctx
        .data()
        .bounties
        .post(title, reward, ctx.author().name.clone())
        .await;
    ctx.say(bounty_mod::format_bounty(&b)).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "list")]
pub async fn bounty_list(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let items = ctx.data().bounties.list().await;
    if items.is_empty() {
        ctx.say("no bounties open.").await?;
        return Ok(());
    }
    let msg = items
        .iter()
        .take(15)
        .map(bounty_mod::format_bounty)
        .collect::<Vec<_>>()
        .join("\n");
    ctx.say(msg).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "claim")]
pub async fn bounty_claim(
    ctx: Context<'_>,
    #[description = "Bounty id"] id: u64,
) -> Result<(), anyhow::Error> {
    match ctx.data().bounties.claim(id, &ctx.author().name).await {
        Some(b) => {
            ctx.say(format!("claimed: {}", bounty_mod::format_bounty(&b)))
                .await?;
        }
        None => {
            ctx.say("not found or already claimed.").await?;
        }
    }
    Ok(())
}
