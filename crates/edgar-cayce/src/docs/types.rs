use serde::{Deserialize, Serialize};

/// Content-addressed document ID (blake3 hex hash).
pub type DocId = String;

/// Document metadata stored alongside content in the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocMeta {
    pub id: DocId,
    pub name: String,
    /// e.g. "github:owner/repo" or "url:https://..."
    pub source: String,
    /// Topic label for /edgar ask scoping
    pub label: String,
    pub size: usize,
    pub ingested_at: i64,
    /// Admin-provided URL attribution context for citation routing.
    /// e.g. "files in docs/ map to https://akash.network/docs"
    #[serde(default)]
    pub url_context: Option<String>,
}

/// A stored Q/A record.
///
/// Two-tier usage (see DocumentStore):
/// - every answer is logged here as **dataset curation** (eval, benchmarks,
///   fine-tune export) — never retrieved as a source in this state;
/// - only records with `verified = true` (human-approved) are served back
///   into the loop, clearly labeled, and only while their `doc_ids` are all
///   still current corpus versions (`corpus_epoch` pins the version set).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaRecord {
    pub id: String,
    pub topic: String,
    pub question: String,
    pub answer: String,
    pub cited_urls: Vec<String>,
    pub doc_ids: Vec<String>,
    pub evidence: Vec<String>,
    pub iterations: u32,
    pub timestamp: i64,
    /// Human-approved for re-use as loop context. Never set by the bot.
    #[serde(default)]
    pub verified: bool,
    /// Corpus commitment epoch this answer was built from (see commits table).
    #[serde(default)]
    pub corpus_epoch: i64,
}

/// A search result excerpt from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocExcerpt {
    pub doc_id: DocId,
    pub offset: usize,
    pub content: String,
    pub match_count: usize,
}

/// Page-collapsed FTS hit (Vocs MiniSearch semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageHit {
    pub page_id: i64,
    pub doc_id: DocId,
    pub href: String,
    pub title: String,
    pub heading_path: String,
    pub score: f64,
    pub excerpt: String,
}
