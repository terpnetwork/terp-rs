mod commands;
mod docs;
mod llm;
mod llm_acp;
mod rlm;
mod state;
mod terp;
mod trace_bus;
mod util;

use std::collections::HashSet;
use std::sync::Arc;

use poise::serenity_prelude as serenity;
use poise::{Framework, FrameworkOptions};
use tokio::sync::RwLock;
use tracing::{error, info, Level};

use docs::DocumentStore;
use llm::LlmClient;
use rlm::RlmEngine;
use state::{AppState, RlmConfig};

/// Gov proposals (`#gov-updates`), IBC channels and the daily voting-power
/// summary (`#activity`, short line plus a `.md` file). LCD poll on the
/// local node (`TERP_NODE_REST`).
/// Gov posts use `GOV_ALERT_CHANNEL_ID`. If that is unset they fall back to
/// the activity channel, which is not `#gov-updates`.
fn env_channel(primary: &str, fallback: u64) -> u64 {
    std::env::var(primary)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback)
}

struct PostedFile {
    name: String,
    body: Vec<u8>,
}

async fn post_watch(
    http: &serenity::Http,
    dest: serenity::ChannelId,
    text: &str,
    files: &[PostedFile],
) -> Option<serenity::Message> {
    let text: String = text.chars().take(1900).collect();
    let mut msg = serenity::builder::CreateMessage::new().content(text);
    for file in files {
        msg = msg.add_file(serenity::builder::CreateAttachment::bytes(
            file.body.clone(),
            file.name.clone(),
        ));
    }
    match dest.send_message(http, msg).await {
        Ok(message) => Some(message),
        Err(e) => {
            tracing::warn!("watch post failed: {e}");
            None
        }
    }
}

/// Foundation DAO proposals, votes, and execution/closure.
/// Needs `TERP_NODE_REST`, `DAO_NOTIFY_CHANNEL_ID`, and `DAO_CONTRACT`
/// or `DAO_ALLOWLIST`. The contract address is config, never a constant.
async fn dao_watch_loop(http: std::sync::Arc<serenity::Http>, rest: String, channel: u64) {
    let path = std::path::PathBuf::from("./data/dao-watch.json");
    let poll_secs: u64 = std::env::var("DAO_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
        .max(20);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let dest = serenity::ChannelId::new(channel);
    info!(poll_secs, channel, "dao watch started");
    loop {
        let mut state = terp::dao::load_state(&path);
        match terp::dao::tick(&client, &rest, &mut state).await {
            Ok(notes) => {
                if let Err(e) = terp::dao::save_state(&path, &state) {
                    tracing::warn!("dao watch state save failed: {e:#}");
                }
                for text in notes {
                    if post_watch(&http, dest, &text, &[]).await.is_none() {
                        break;
                    }
                }
            }
            Err(e) => tracing::warn!("dao watch poll failed: {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
    }
}

async fn chain_watch_loop(
    http: std::sync::Arc<serenity::Http>,
    rest: String,
    fallback: u64,
    store: Arc<DocumentStore>,
) {
    let path = std::path::PathBuf::from("./data/chain-watch.json");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let gov_dedicated = std::env::var("GOV_ALERT_CHANNEL_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n != 0);
    let gov = serenity::ChannelId::new(gov_dedicated.unwrap_or(fallback));
    let activity = serenity::ChannelId::new(env_channel(
        "ACTIVITY_CHANNEL_ID",
        env_channel("STAKE_ALERT_CHANNEL_ID", fallback),
    ));
    if gov_dedicated.is_none() {
        tracing::warn!(
            fallback = gov.get(),
            "GOV_ALERT_CHANNEL_ID is unset; gov alerts go to the activity fallback and will not reach #gov-updates"
        );
    }
    info!(
        gov = gov.get(),
        activity = activity.get(),
        explorer = %terp::chain_watch::explorer_base(),
        "chain watch started (LCD poll 45s; gov follow-through, ibc state, morning board)"
    );
    loop {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut state = terp::chain_watch::load_state(&path);
        let gov_seen = state.seen_proposals.clone();
        let gov_phase = state.proposal_phase.clone();
        let gov_reminded = state.reminded_24h.clone();
        match terp::chain_watch::tick(&client, &rest, &mut state, &today).await {
            Ok(notes) => {
                let mut gov_failed = false;
                for note in notes {
                    let dest = match note.kind {
                        terp::chain_watch::NoteKind::Proposal => gov,
                        terp::chain_watch::NoteKind::Channel
                        | terp::chain_watch::NoteKind::Stake
                        | terp::chain_watch::NoteKind::Baseline
                        | terp::chain_watch::NoteKind::Notice => activity,
                    };
                    let mut files = Vec::new();
                    if let (Some(name), Some(body)) = (&note.file_name, &note.file_body) {
                        files.push(PostedFile {
                            name: name.clone(),
                            body: body.as_bytes().to_vec(),
                        });
                    }
                    if note.kind == terp::chain_watch::NoteKind::Stake {
                        match morning_clip(&note.text) {
                            Ok(body) => files.push(PostedFile {
                                name: "morning.mp4".into(),
                                body,
                            }),
                            Err(e) => tracing::warn!("morning clip skipped: {e:#}"),
                        }
                    }
                    if post_watch(&http, dest, &note.text, &files).await.is_some() {
                        if let Some(query) = note.thread_query.as_deref() {
                            open_proposal_thread(&http, dest, &store, query, &note.text).await;
                        }
                    } else {
                        tracing::warn!(
                            channel = dest.get(),
                            kind = ?note.kind,
                            "chain watch post failed"
                        );
                        if note.kind == terp::chain_watch::NoteKind::Proposal {
                            gov_failed = true;
                        }
                    }
                }
                if gov_failed {
                    // A failed gov post must not stick. Saving first made a
                    // missed #gov-updates message permanent.
                    state.seen_proposals = gov_seen;
                    state.proposal_phase = gov_phase;
                    state.reminded_24h = gov_reminded;
                    tracing::warn!("gov alert was not delivered; will retry next poll");
                }
                if let Err(e) = terp::chain_watch::save_state(&path, &state) {
                    tracing::warn!("chain watch state save failed: {e:#}");
                }
            }
            Err(e) => tracing::warn!("chain watch poll failed: {e:#}"),
        }
        let poll_secs: u64 = std::env::var("CHAIN_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(45)
            .max(1);
        tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
    }
}

fn orb_script() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("EDGAR_ORB_SCRIPT") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let mut candidates = vec![std::path::PathBuf::from("scripts/morning_orb.py")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("scripts/morning_orb.py"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Render the morning board to an MP4. A missing tool posts the text board alone.
fn morning_clip(speech: &str) -> anyhow::Result<Vec<u8>> {
    let script = orb_script().ok_or_else(|| anyhow::anyhow!("morning_orb.py not found"))?;
    let out = std::env::temp_dir().join(format!(
        "edgar-morning-{}.mp4",
        chrono::Utc::now().timestamp_millis()
    ));
    let mut child = std::process::Command::new("python3")
        .arg(&script)
        .arg("--out")
        .arg(&out)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("morning orb stdin closed"))?;
        stdin.write_all(speech.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&out);
        return Err(anyhow::anyhow!(
            "morning orb failed: {}",
            err.chars().take(400).collect::<String>()
        ));
    }
    let bytes = std::fs::read(&out)?;
    let _ = std::fs::remove_file(&out);
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err(anyhow::anyhow!("morning orb is {} bytes", bytes.len()));
    }
    Ok(bytes)
}

async fn open_proposal_thread(
    http: &serenity::Http,
    channel: serenity::ChannelId,
    store: &DocumentStore,
    query: &str,
    headline: &str,
) {
    let title: String = format!("edgar · {query}").chars().take(90).collect();
    let thread = match channel
        .create_thread(
            http,
            serenity::CreateThread::new(title)
                .kind(serenity::ChannelType::PublicThread)
                .auto_archive_duration(serenity::AutoArchiveDuration::OneHour),
        )
        .await
    {
        Ok(thread) => thread,
        Err(e) => {
            tracing::warn!("proposal thread failed: {e:#}");
            return;
        }
    };
    let topic = std::env::var("EDGAR_DOCS_TOPIC").unwrap_or_else(|_| "terp-docs".into());
    let page = match store.search_pages(&topic, query, None, 1).await {
        Ok(mut hits) => hits.drain(..).next(),
        Err(e) => {
            tracing::warn!("proposal docs search failed: {e:#}");
            None
        }
    };
    let head: String = headline.lines().next().unwrap_or(query).chars().take(180).collect();
    let body = match page {
        Some(hit) => {
            let excerpt: String = hit.excerpt.chars().take(400).collect();
            format!(
                "{head}\n{excerpt}\n\n**Docs:** [{}]({})",
                hit.title, hit.href
            )
        }
        None => format!(
            "{head}\nno docs page matched this proposal. `/edgar ask` in this thread still has the proposal id and title attached."
        ),
    };
    match thread.id.say(http, body).await {
        Ok(message) => {
            if let Err(e) = message.pin(http).await {
                tracing::warn!("proposal docs pin failed: {e}");
            }
        }
        Err(e) => tracing::warn!("proposal thread post failed: {e}"),
    }
    let prefix = format!(
        "{query}\n{}",
        headline.lines().next().unwrap_or(query)
    );
    terp::chain_watch::remember_thread(thread.id.get(), &prefix);
}

fn discord_http(token: &str) -> anyhow::Result<serenity::Http> {
    let Some(base) = std::env::var("EDGAR_DISCORD_API")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return Ok(serenity::Http::new(token));
    };
    if !token.contains(".twin.") {
        anyhow::bail!("EDGAR_DISCORD_API is set but DISCORD_TOKEN is not a digital-twin token");
    }
    Ok(serenity::http::HttpBuilder::new(token)
        .proxy(base)
        .ratelimiter_disabled(true)
        .build())
}

/// Background validator liveness monitor.
///
/// Polls the full bonded set + slashing params, diffs against in-memory
/// tracking (offline episodes, escalation, slashing ETA, recovery), and
/// posts automated messages to the alert channel. This is the internal
/// Hermes-style watchdog: push, not slash commands.
async fn validator_monitor_loop(http: std::sync::Arc<serenity::Http>, rest: String, channel: u64) {
    let poll_secs: u64 = std::env::var("VALIDATOR_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
        .max(1);
    let remind_secs: i64 = std::env::var("VALIDATOR_REMIND_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800);
    let prefix = std::env::var("VALIDATOR_PREFIX").unwrap_or_else(|_| "terp".into());
    let chain = terp::validator::ChainConfig {
        name: "terp".into(),
        rest,
        prefix,
        missed_threshold: 50,
        validators: vec![],
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut monitor = terp::validator::Monitor::new(remind_secs);
    let channel_id = serenity::ChannelId::new(channel);
    info!(poll_secs, channel, "validator monitor started");
    loop {
        let params = terp::validator::slashing_params(&client, &chain.rest).await;
        let allowed = if params.signed_blocks_window > 0 {
            params.allowed_missed()
        } else {
            chain.missed_threshold
        };
        match terp::validator::fetch_all(&client, &chain).await {
            Ok(set) => {
                terp::validator::write_closest_line(
                    std::path::Path::new("./data/closest-jail.txt"),
                    &set,
                    allowed,
                );
                let now = chrono::Utc::now().timestamp();
                for alert in monitor.poll("terp", &set, now, allowed) {
                    if alert.critical {
                        info!(kind = alert.kind, valoper = %alert.valoper, "validator alert");
                    }
                    if let Err(e) = channel_id.say(&http, &alert.message).await {
                        tracing::warn!("validator alert post failed: {e}");
                        break;
                    }
                }
            }
            Err(e) => tracing::warn!("validator poll failed: {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "discord_demo=info,serenity=warn,poise=warn,wasmer_wasix=warn".into()
            }),
        ))
        .with(
            trace_bus::DiscordTraceLayer.with_filter(tracing_subscriber::filter::filter_fn(
                |meta| meta.target().starts_with("discord_demo::") && *meta.level() <= Level::INFO,
            )),
        )
        .init();

    // Load env
    let _ = dotenv::dotenv();

    let mut argv = std::env::args().skip(1);
    match argv.next().as_deref() {
        Some("ingest") => return cli_ingest(argv).await,
        Some("llm-probe") => return cli_llm_probe(argv).await,
        Some("providers") => {
            let llm = LlmClient::from_env()?;
            for n in llm.profile_names() {
                println!("{n}");
            }
            return Ok(());
        }
        _ => {}
    }

    // First kev call is a cold start of about 20s. Warm it before anyone asks.
    rlm::gate::spawn_warm_up();

    let token = dotenv::var("DISCORD_TOKEN").expect("DISCORD_TOKEN required");
    let guild_id: Option<serenity::GuildId> = dotenv::var("DISCORD_GUILD_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(serenity::GuildId::new);

    // Init storage
    let data_dir = std::path::PathBuf::from("./data/docs");
    let store = Arc::new(DocumentStore::new(&data_dir).await?);
    info!("Document store initialized at {:?}", data_dir);

    // Init LLM client
    let llm_client = Arc::new(LlmClient::from_env()?);
    info!("LLM client initialized");

    // Parse admin user IDs from env
    let admin_ids: HashSet<u64> = dotenv::var("ADMIN_USER_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .collect();
    if !admin_ids.is_empty() {
        info!(count = admin_ids.len(), "Admin users configured");
    }

    // Parse admin role IDs from env
    let admin_role_ids: HashSet<u64> = dotenv::var("ADMIN_ROLE_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .collect();
    if !admin_role_ids.is_empty() {
        info!(count = admin_role_ids.len(), "Admin roles configured");
    }
    let admin_role_ids = Arc::new(RwLock::new(admin_role_ids));

    let rlm_config = Arc::new(RwLock::new(RlmConfig::default()));

    // Init RLM engine
    let rlm = Arc::new(RlmEngine::new(llm_client.clone(), store.clone()));
    let watch_store = store.clone();

    let app_state = AppState {
        store,
        llm: llm_client,
        rlm,
        admin_ids,
        admin_role_ids,
        rlm_config,
        bounties: Arc::new(terp::bounty::BountyBoard::default()),
    };

    let intents = serenity::GatewayIntents::GUILDS
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    // Anti-spam message handler (port of DA0-DA0 discord-anti-spam-bot/bot.py)
    let event_handler = SpamHandler;
    struct SpamHandler;
    #[serenity::async_trait]
    impl serenity::EventHandler for SpamHandler {
        async fn message(&self, ctx: serenity::Context, msg: serenity::Message) {
            if msg.author.bot {
                return;
            }
            if !terp::spam::is_spam(&msg.content) {
                return;
            }
            tracing::warn!(user = %msg.author.name, content = %msg.content, "spam detected");
            if let Err(e) = msg.delete(&ctx.http).await {
                tracing::warn!("spam delete failed: {e}");
                return;
            }
            // Long timeout (DA0-DA0 pattern). Requires ModerateMembers.
            // Discord caps timeouts at 28 days.
            let days: u64 = std::env::var("SPAM_TIMEOUT_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(27)
                .min(28);
            let until = serenity::Timestamp::from_unix_timestamp(
                chrono::Utc::now().timestamp() + (days as i64) * 86_400,
            )
            .ok();
            let mut timed_out = false;
            if let (Some(guild_id), Some(until)) = (msg.guild_id, until) {
                match guild_id.member(&ctx.http, msg.author.id).await {
                    Ok(mut member) => {
                        match member
                            .disable_communication_until_datetime(&ctx.http, until)
                            .await
                        {
                            Ok(()) => timed_out = true,
                            Err(e) => tracing::warn!("spam timeout failed: {e}"),
                        }
                    }
                    Err(e) => tracing::warn!("spam member fetch failed: {e}"),
                }
            }
            if let Ok(chan_id) =
                std::env::var("SPAM_NOTIFY_CHANNEL_ID").map(|s| s.parse::<u64>().unwrap_or(0))
            {
                if chan_id != 0 {
                    use serenity::Mentionable;
                    let action = if timed_out {
                        format!("{} timed out for spam ({}d)", msg.author.mention(), days)
                    } else {
                        format!(
                            "deleted spam message from {} (timeout failed — check ModerateMembers)",
                            msg.author.mention()
                        )
                    };
                    let _ = serenity::ChannelId::new(chan_id)
                        .say(&ctx.http, action)
                        .await;
                }
            }
        }
    }

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands: vec![
                commands::edgar(),
                commands::terp_cmd::terp(),
                commands::terp_cmd::validator(),
                commands::terp_cmd::gov(),
                commands::terp_cmd::dao(),
                commands::terp_cmd::bounty(),
            ],
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                info!("Bot connected as: {} ({})", ready.user.name, ready.user.id);

                let commands = &framework.options().commands;
                info!("Registering {} top-level command(s):", commands.len());
                for cmd in commands {
                    info!("  /{} ({} subcommands)", cmd.name, cmd.subcommands.len());
                    for sub in &cmd.subcommands {
                        info!("    /{} {}", cmd.name, sub.name);
                    }
                }

                if let Some(gid) = guild_id {
                    info!("Registering to guild {} (instant)", gid);
                    poise::builtins::register_in_guild(ctx, &framework.options().commands, gid)
                        .await?;
                } else {
                    info!("Registering globally (up to 1 hour delay)");
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                }

                // Background validator monitor: automated liveness messages,
                // not slash commands. Needs TERP_NODE_REST + alert channel.
                let http = ctx.http.clone();
                tokio::spawn(async move {
                    let rest = terp::node::base_url();
                    let chan: u64 = std::env::var("VALIDATOR_ALERT_CHANNEL_ID")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    match (rest, chan) {
                        (Some(rest), c) if c != 0 => validator_monitor_loop(http, rest, c).await,
                        _ => info!(
                            "validator monitor off (need TERP_NODE_REST + VALIDATOR_ALERT_CHANNEL_ID)"
                        ),
                    }
                });
                let http = ctx.http.clone();
                tokio::spawn(async move {
                    let rest = terp::node::base_url();
                    let chan: u64 = std::env::var("DAO_NOTIFY_CHANNEL_ID")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let configured = !terp::dao::configured_daos().is_empty();
                    match (rest, chan, configured) {
                        (Some(rest), c, true) if c != 0 => dao_watch_loop(http, rest, c).await,
                        _ => info!(
                            "dao watch off (need TERP_NODE_REST + DAO_NOTIFY_CHANNEL_ID + DAO_CONTRACT or DAO_ALLOWLIST)"
                        ),
                    }
                });
                let http = ctx.http.clone();
                let watch_docs = watch_store.clone();
                tokio::spawn(async move {
                    let rest = terp::node::base_url();
                    let chan: u64 = std::env::var("CHAIN_ALERT_CHANNEL_ID")
                        .ok()
                        .or_else(|| std::env::var("VALIDATOR_ALERT_CHANNEL_ID").ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    match (rest, chan) {
                        (Some(rest), c) if c != 0 => {
                            chain_watch_loop(http, rest, c, watch_docs).await
                        }
                        _ => info!(
                            "chain watch off (need TERP_NODE_REST + CHAIN_ALERT_CHANNEL_ID or VALIDATOR_ALERT_CHANNEL_ID)"
                        ),
                    }
                });

                Ok(app_state)
            })
        })
        .build();

    info!("Starting Edgar Discord bot...");

    let http = discord_http(&token)?;
    let mut client = serenity::ClientBuilder::new_with_http(http, intents)
        .framework(framework)
        .event_handler(event_handler)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create client: {}", e))?;

    if let Err(e) = client.start().await {
        error!("Client error: {}", e);
    }

    Ok(())
}

/// Operator ingest without Discord: `edgar ingest --url URL --label LABEL`
async fn cli_ingest(mut argv: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let mut url = None;
    let mut label = None;
    let mut doc_type = None;
    let mut branch = None;
    while let Some(a) = argv.next() {
        match a.as_str() {
            "--url" => url = argv.next(),
            "--label" => label = argv.next(),
            "--type" => doc_type = argv.next(),
            "--branch" => branch = argv.next(),
            other => anyhow::bail!("unknown ingest arg: {other}"),
        }
    }
    let url = url.ok_or_else(|| anyhow::anyhow!("ingest requires --url"))?;
    let label = label.ok_or_else(|| anyhow::anyhow!("ingest requires --label"))?;
    let data_dir = std::path::PathBuf::from("./data/docs");
    let store = docs::DocumentStore::new(&data_dir).await?;
    let is_github = url.contains("github.com");
    let (doc_id, detail) = if is_github {
        let (id, n) = docs::ingest::ingest_github_repo(
            &store,
            &url,
            &label,
            doc_type.as_deref(),
            branch.as_deref(),
            None,
        )
        .await?;
        (id, format!("{n} files"))
    } else {
        let (id, size) = docs::ingest::ingest_url(&store, &url, &label).await?;
        (id, format!("{size} bytes"))
    };
    println!("ingested {doc_id} ({detail}) label={label}");
    Ok(())
}

/// `edgar llm-probe "<question>" [doc-file...]`: send one docs-grounded
/// question through every LLM profile (same LlmClient path as /terp ask) and
/// print per-profile latency. No Discord needed.
async fn cli_llm_probe(mut argv: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let question = argv.next().unwrap_or_else(|| "what version of ibc is terp network on".into());
    let mut ctx = String::new();
    for path in argv {
        let text = std::fs::read_to_string(&path)?;
        let cut = util::floor_char_boundary(&text, 12_000);
        ctx.push_str(&format!("--- {path} ---\n{}\n", &text[..cut]));
    }
    let llm = LlmClient::from_env()?;
    let messages = vec![
        llm::Message {
            role: "system".into(),
            content: format!("Answer briefly using only these docs.\n{ctx}"),
        },
        llm::Message { role: "user".into(), content: question },
    ];
    for (name, took, res) in llm.chat_each(&messages).await {
        match res {
            Ok(t) => println!("[{name}] OK {:.1}s: {}", took.as_secs_f64(), util::truncate(t.trim(), 300)),
            Err(e) => println!("[{name}] FAIL {:.1}s: {e}", took.as_secs_f64()),
        }
    }
    Ok(())
}
