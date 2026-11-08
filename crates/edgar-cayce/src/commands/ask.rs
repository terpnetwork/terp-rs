use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::commands::config::is_admin;
use crate::state::{self, Context};
use poise::serenity_prelude as serenity;
use tracing::warn;

pub(crate) struct AdmittedAsk {
    _lease: state::AskLease,
    user_id: u64,
    channel_id: u64,
    profile: String,
    started: Instant,
    outcome: Mutex<Option<&'static str>>,
    gate_verdict: Mutex<Option<String>>,
}

impl Drop for AdmittedAsk {
    fn drop(&mut self) {
        let outcome = self
            .outcome
            .get_mut()
            .ok()
            .and_then(|slot| slot.take())
            .unwrap_or("error");
        let verdict = self
            .gate_verdict
            .get_mut()
            .ok()
            .and_then(|slot| slot.take());
        state::ask_log(
            self.user_id,
            self.channel_id,
            &self.profile,
            self.started.elapsed().as_millis() as u64,
            outcome,
            verdict.as_deref(),
        )
        .emit();
    }
}

/// Admit one ask or reply and return `None` when the pilot refuses it.
pub(crate) async fn begin_ask(ctx: &Context<'_>) -> Result<Option<AdmittedAsk>, anyhow::Error> {
    let started = Instant::now();
    let user_id = ctx.author().id.get();
    let channel_id = ctx.channel_id().get();
    let profile = ctx
        .data()
        .llm
        .profile_names()
        .into_iter()
        .next()
        .unwrap_or_else(|| "none".to_string());
    let mut role_ids = match ctx.author_member().await {
        Some(member) => member
            .roles
            .iter()
            .map(|role| role.get())
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    // Discord never lists @everyone on a member. Its role ID is the guild ID,
    // so ASK_ROLE_IDS=<guild id> opens the pilot to everyone in the channel.
    if let Some(guild_id) = ctx.guild_id() {
        role_ids.push(guild_id.get());
    }
    let admit = |id| state::AskPilot::get().try_admit(user_id, id, &role_ids, started);
    let mut decision = admit(channel_id);
    // A proposal thread is a different channel id than #activity. The allowlist
    // names the parent. Admit the thread when that parent is allowed.
    if matches!(decision, Err(state::AskDeny::Channel)) {
        if let Some(parent) = thread_parent_id(ctx).await {
            if parent != channel_id {
                decision = admit(parent);
            }
        }
    }
    match decision {
        Ok(lease) => Ok(Some(AdmittedAsk {
            _lease: lease,
            user_id,
            channel_id,
            profile,
            started,
            outcome: Mutex::new(None),
            gate_verdict: Mutex::new(None),
        })),
        Err(deny) => {
            state::ask_log(
                user_id,
                channel_id,
                &profile,
                started.elapsed().as_millis() as u64,
                deny.outcome(),
                None,
            )
            .emit();
            ctx.say(deny.reply()).await?;
            Ok(None)
        }
    }
}

/// Remember the laya verdict so the ask log line carries it instead of `pending`.
pub(crate) fn record_gate_verdict(admitted: &AdmittedAsk, verdict: Option<&str>) {
    if let (Some(v), Ok(mut slot)) = (verdict, admitted.gate_verdict.lock()) {
        *slot = Some(v.to_string());
    }
}

pub(crate) fn finish_ask(admitted: &AdmittedAsk, outcome: &'static str) {
    if let Ok(mut slot) = admitted.outcome.lock() {
        *slot = Some(outcome);
    }
}


async fn thread_parent_id(ctx: &Context<'_>) -> Option<u64> {
    let channel = ctx.channel_id().to_channel(ctx.http()).await.ok()?;
    match channel {
        serenity::Channel::Guild(ch) => ch.parent_id.map(|id| id.get()),
        _ => None,
    }
}

/// Ask a question about ingested documents
#[poise::command(slash_command, guild_only)]
pub async fn ask(
    ctx: Context<'_>,
    #[description = "Topic (matches ingested label)"]
    #[autocomplete = "autocomplete_topic"]
    topic: String,
    #[description = "Your question"] question: String,
    #[description = "Verbose REPL snippets in the trace thread"] debug: Option<bool>,
) -> Result<(), anyhow::Error> {
    let question = match crate::terp::chain_watch::thread_prefix(ctx.channel_id().get()) {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}\n{question}"),
        _ => question,
    };
    let Some(admitted) = begin_ask(&ctx).await? else {
        return Ok(());
    };
    let user_mention = format!("<@{}>", ctx.author().id);
    let is_admin = is_admin(&ctx).await;
    let show_debug = debug.unwrap_or(false) && is_admin;

    let handle = ctx
        .say(format!(
            "researching **{topic}** — opening a trace thread, {user_mention}"
        ))
        .await?;
    let ack = handle.message().await?;

    let title: String = format!("edgar · {topic} · {question}")
        .chars()
        .take(90)
        .collect();
    // Do not start from the interaction webhook message — Discord returns
    // Missing Access on those even when Create Public Threads is granted.
    // Standalone public thread on the channel (same as /edgar thread).
    let _ = ack;
    let thread = ctx
        .channel_id()
        .create_thread(
            ctx.http(),
            serenity::CreateThread::new(title)
                .kind(serenity::ChannelType::PublicThread)
                .auto_archive_duration(serenity::AutoArchiveDuration::OneHour),
        )
        .await;

    let http = ctx.serenity_context().http.clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tracer = match thread {
        Ok(th) => {
            let _ = th
                .id
                .say(
                    http.as_ref(),
                    format!(
                        "trace for **{topic}**\nQ: {}\n`debug={}`",
                        crate::util::truncate(&question, 300),
                        show_debug
                    ),
                )
                .await;
            Some(spawn_tracer(http.clone(), th.id, rx))
        }
        Err(e) => {
            warn!("could not create ask thread: {e:#}");
            let _ = ctx
                .say(format!("couldn't open a thread ({e}) — traces in this channel"))
                .await;
            Some(spawn_tracer(http, ctx.channel_id(), rx))
        }
    };

    let config = ctx.data().rlm_config.read().await;
    let max_iterations = config.max_iterations;
    let min_code_executions = config.min_code_executions;
    let min_answer_len = config.min_answer_len;
    let parallel_loops = config.parallel_loops;
    drop(config);

    crate::trace_bus::attach(tx.clone());
    // Total ask budget: past this the user gets a clear reply, not silence.
    // Dropping the query future kills any ACP child process groups.
    let budget = ask_budget();
    let result = match tokio::time::timeout(
        budget,
        ctx.data().rlm.query(
            &topic,
            &question,
            max_iterations,
            min_code_executions,
            min_answer_len,
            parallel_loops,
            Some(&tx),
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => {
            let _ = tx.send(format!("ask budget {}s exhausted — giving up", budget.as_secs()));
            crate::trace_bus::detach();
            drop(tx);
            if let Some(t) = tracer {
                let _ = t.await;
            }
            warn!(secs = budget.as_secs(), "ask budget exhausted");
            finish_ask(&admitted, "error");
            let _ = ctx
                .say(format!(
                    "{user_mention} sorry, I couldn't answer that right now (the models didn't respond within {}s). Please try again in a few minutes.",
                    budget.as_secs()
                ))
                .await;
            return Ok(());
        }
    };
    crate::trace_bus::detach();
    drop(tx);
    if let Some(t) = tracer {
        let _ = t.await;
    }

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            warn!(error = format!("{e:#}"), "RLM query failed");
            finish_ask(&admitted, "error");
            let _ = ctx
                .say(format!(
                    "{user_mention} research failed (models timed out or missing). See the trace thread — not dumping the raw spawn error here."
                ))
                .await;
            return Ok(());
        }
    };
    record_gate_verdict(&admitted, result.gate_verdict.as_deref());
    let blocked = crate::rlm::gate::blocked_outcome(result.gate_verdict.as_deref());

    let mut full = format!(
        "{user_mention} here's what I found:\n\n**Q:** {question}\n**Topic:** {topic} | **Iterations:** {}\n\n**A:** {}",
        result.iterations, result.answer
    );

    if !result.cited_urls.is_empty() {
        full.push_str("\n\n**Sources:**\n");
        for url in &result.cited_urls {
            let label = url.rsplit('/').find(|s| !s.is_empty()).unwrap_or(url);
            full.push_str(&format!("- [{label}]({url})\n"));
        }
    }

    if show_debug && !result.evidence.is_empty() {
        full.push_str("\n\n---\n**[Debug] Evidence:**\n");
        for (i, ev) in result.evidence.iter().enumerate().take(3) {
            let snippet = crate::util::truncate(ev, 800);
            full.push_str(&format!("\n**[{}]** ```\n{snippet}```\n", i + 1));
        }
    }

    let sent = send_chunked(&ctx, &full).await;
    finish_ask(
        &admitted,
        match (&sent, blocked) {
            (Err(_), _) => "error",
            (Ok(_), Some(outcome)) => outcome,
            (Ok(_), None) => "ok",
        },
    );
    sent
}

fn spawn_tracer(
    http: std::sync::Arc<serenity::Http>,
    dest: serenity::ChannelId,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut tick = tokio::time::interval(Duration::from_millis(1600));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if !buf.is_empty() {
                        flush(&http, dest, &mut buf).await;
                    }
                }
                line = rx.recv() => {
                    match line {
                        None => {
                            if !buf.is_empty() {
                                flush(&http, dest, &mut buf).await;
                            }
                            break;
                        }
                        Some(l) => {
                            if buf.len() + l.len() + 1 > 1800 {
                                flush(&http, dest, &mut buf).await;
                            }
                            if !buf.is_empty() {
                                buf.push('\n');
                            }
                            buf.push_str(&l);
                        }
                    }
                }
            }
        }
    })
}

async fn flush(http: &serenity::Http, dest: serenity::ChannelId, buf: &mut String) {
    let text = std::mem::take(buf);
    if let Err(e) = dest.say(http, text).await {
        warn!("trace post failed: {e}");
    }
}

/// Send a message in Discord-safe chunks (max 1990 chars).
async fn send_chunked(ctx: &Context<'_>, text: &str) -> Result<(), anyhow::Error> {
    let mut remaining = text;
    while !remaining.is_empty() {
        let (chunk, rest) = crate::util::split_head(remaining, 1990);
        remaining = rest;
        ctx.say(chunk).await?;
    }
    Ok(())
}

async fn autocomplete_topic(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let labels = ctx.data().store.labels().await.unwrap_or_default();
    labels
        .into_iter()
        .filter(|l| l.to_lowercase().contains(&partial.to_lowercase()))
        .take(25)
        .collect()
}

/// Wall-clock budget for one `/terp ask` (env `ASK_BUDGET_SECS`, default 120).
fn ask_budget() -> Duration {
    let secs = std::env::var("ASK_BUDGET_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(120);
    Duration::from_secs(secs)
}
