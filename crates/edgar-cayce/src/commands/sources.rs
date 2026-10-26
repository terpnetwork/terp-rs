use crate::state::Context;

/// List ingested document sources
#[poise::command(slash_command, guild_only)]
pub async fn sources(
    ctx: Context<'_>,
    #[description = "Max documents to show"] limit: Option<u32>,
) -> Result<(), anyhow::Error> {
    let limit = limit.unwrap_or(20) as usize;
    let docs = ctx.data().store.list(limit, 0).await?;

    if docs.is_empty() {
        ctx.say("No documents ingested yet. Use `/edgar ingest` to add some.")
            .await?;
        return Ok(());
    }

    // Group by label
    let mut by_label: std::collections::BTreeMap<String, Vec<_>> =
        std::collections::BTreeMap::new();
    for doc in &docs {
        by_label
            .entry(doc.label.clone())
            .or_default()
            .push(doc);
    }

    let mut output = String::from("**Ingested Documents**\n\n");
    for (label, label_docs) in &by_label {
        output.push_str(&format!("**Topic: {}**\n", label));
        for doc in label_docs {
            let size_kb = doc.size / 1024;
            output.push_str(&format!(
                "  - {} ({} KB) — `{}`\n    Source: {}\n",
                doc.name, size_kb, &doc.id[..12], doc.source
            ));
        }
        output.push('\n');
    }

    // Chunk if needed — use ctx.say() for all chunks so follow-ups go
    // through the interaction webhook (no Send Messages permission required).
    if output.len() <= 2000 {
        ctx.say(output).await?;
    } else {
        let mut remaining = output.as_str();
        while !remaining.is_empty() {
            let (chunk, rest) = crate::util::split_head(remaining, 1990);
            remaining = rest;
            ctx.say(chunk).await?;
        }
    }

    Ok(())
}
