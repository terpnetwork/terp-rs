mod ask;
mod config;
mod ingest;
mod manage;
mod sources;
pub mod terp_cmd;

use crate::state::Context;

/// Edgar - Ergors Discord Knowledge Assistant (legacy, kept for compat)
#[poise::command(
    slash_command,
    subcommands(
        "ask::ask",
        "ingest::ingest",
        "ingest::ingest_local",
        "sources::sources",
        "manage::clear",
        "manage::thread",
        "config::config",
        "config::pause",
        "config::resume"
    )
)]
pub async fn edgar(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

/// Chunked sender shared by terp + edgar commands (Discord 2000-char limit).
pub async fn send_chunked_public(ctx: &Context<'_>, text: &str) -> Result<(), anyhow::Error> {
    let mut remaining = text;
    while !remaining.is_empty() {
        let (chunk, rest) = crate::util::split_head(remaining, 1990);
        remaining = rest;
        ctx.say(chunk).await?;
    }
    Ok(())
}
