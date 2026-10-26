//! Governance reads for slash commands. The chain-watch loop owns alerts.
//! Open proposals are deposit period and voting period, same query as the watcher.

use serde::{Deserialize, Serialize};

use super::chain_watch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovProposal {
    pub id: String,
    pub title: String,
    pub status: String,
    pub summary: String,
}

/// Deposit-period and voting-period proposals for a chain REST endpoint.
pub async fn active_proposals(
    client: &reqwest::Client,
    rest: &str,
) -> anyhow::Result<Vec<GovProposal>> {
    let notes = chain_watch::list_proposals(client, rest).await?;
    Ok(notes
        .into_iter()
        .map(|p| GovProposal {
            id: p.id,
            title: p.title,
            status: p.status,
            summary: p.summary,
        })
        .collect())
}

pub fn format_proposal(chain: &str, p: &GovProposal) -> String {
    let url = chain_watch::proposal_url(&chain_watch::explorer_base(), &p.id);
    let mut msg = format!(
        "🗳️ **[{chain} #{id}] {title}**\n`{status}`",
        chain = chain,
        id = p.id,
        title = p.title,
        status = p.status
    );
    if !p.summary.is_empty() {
        msg.push('\n');
        msg.push_str(&p.summary);
    }
    msg.push('\n');
    msg.push_str(&url);
    msg
}
