use crate::commands::config::is_admin;
use crate::docs::ingest as doc_ingest;
use crate::state::Context;
use tracing::info;

/// Ingest a document from a URL (GitHub repo or web page)
#[poise::command(slash_command, guild_only)]
pub async fn ingest(
    ctx: Context<'_>,
    #[description = "URL (GitHub repo or web page)"] url: String,
    #[description = "Topic label for this document"] label: String,
    #[description = "Type: documentation, code, minimal"]
    doc_type: Option<String>,
    #[description = "Git branch (default: main)"] branch: Option<String>,
    #[description = "URL attribution context, e.g. 'files in docs/ map to https://example.com/docs'"]
    url_context: Option<String>,
) -> Result<(), anyhow::Error> {
    // ACK within 3s before any admin HTTP or network ingest.
    ctx.defer().await?;
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    info!(
        user = ctx.author().name,
        url,
        label,
        "Ingestion started"
    );

    let store = &ctx.data().store;
    let is_github = url.contains("github.com");

    let result = if is_github {
        doc_ingest::ingest_github_repo(
            store,
            &url,
            &label,
            doc_type.as_deref(),
            branch.as_deref(),
            url_context.as_deref(),
        )
        .await
        .map(|(id, n)| (id, format!("{n} files")))
    } else {
        doc_ingest::ingest_url(store, &url, &label)
            .await
            .map(|(id, size)| (id, format!("{size} bytes")))
    };

    let (doc_id, detail) = match result {
        Ok(v) => v,
        Err(e) => {
            ctx.say(format!("ingest failed: {e:#}")).await?;
            return Ok(());
        }
    };

    let meta = store.get_meta(&doc_id).await?;

    let url_note = meta
        .url_context
        .as_deref()
        .map(|u| format!("\nURL context: {}", u))
        .unwrap_or_default();

    ctx.say(format!(
        "Ingested **{}** ({}) under topic **'{}'**\nDoc ID: `{}`\nSize: {} bytes{}",
        meta.name, detail, label, doc_id, meta.size, url_note
    ))
    .await?;

    Ok(())
}

/// Ingest a local file or dir from the sync root (admin)
#[poise::command(slash_command, guild_only, rename = "ingest-local")]
pub async fn ingest_local(
    ctx: Context<'_>,
    #[description = "Path relative to TERP_SYNC_ROOT"] path: String,
    #[description = "Topic label for this document"] label: String,
) -> Result<(), anyhow::Error> {
    ctx.defer().await?;
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    info!(user = ctx.author().name, path, label, "Local ingestion started");

    let store = &ctx.data().store;
    let (doc_id, file_count) = doc_ingest::ingest_local_path(store, &path, &label).await?;
    let meta = store.get_meta(&doc_id).await?;

    ctx.say(format!(
        "Ingested local **{}** ({} files) under topic **'{}'**\nDoc ID: `{}`\nSize: {} bytes",
        meta.name, file_count, label, doc_id, meta.size
    ))
    .await?;

    Ok(())
}
