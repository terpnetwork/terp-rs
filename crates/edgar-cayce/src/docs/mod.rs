//! Document + Q/A storage: sqlite with FTS5, version pointers, and a
//! hash-chained corpus commitment log.
//!
//! Design (post-JMT review):
//! - **Blobs** are content-addressed by blake3 (dedup + stable citation
//!   anchors) and stored inline in sqlite — single file, portable, sandbox-
//!   friendly. The Merkle tree earned nothing on the read path (no proofs
//!   verified, single writer), so cnidarium is gone.
//! - **Search** is an FTS5 inverted index over chunked text (BM25 ranked,
//!   match snippets), replacing the old full-scan-per-call substring search.
//! - **Versions**: `(label, source)` points at one current `doc_id`.
//!   Re-ingesting an updated doc supersedes instead of duplicating; the RLM
//!   loop only ever sees current versions. Old blobs stay for history.
//! - **Commitment log** (JMT's correct purpose, minimal form): every
//!   corpus mutation appends a hash-chained epoch
//!   (`root = blake3(prev_root || sorted label|source|doc_id lines)`).
//!   QA records pin the epoch they were built from; verified answers are
//!   only re-served while their doc set is still current. Roots can be
//!   published later for answer attestation; a full JMT earns its place
//!   only with multi-node replication or proof verification.
//! - **Q/A two tiers**: every answer is logged (dataset/eval/export tier,
//!   never retrieved as a source); only human-verified records are served
//!   back into the loop, labeled as previously-verified context.

pub mod ingest;
pub mod pages;
pub mod types;

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use tracing::debug;

use types::{DocExcerpt, DocId, DocMeta, PageHit, QaRecord};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS docs(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    label TEXT NOT NULL,
    size INTEGER NOT NULL,
    ingested_at INTEGER NOT NULL,
    url_context TEXT,
    content BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS versions(
    label TEXT NOT NULL,
    source TEXT NOT NULL,
    doc_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(label, source)
);
CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
    doc_id UNINDEXED,
    chunk_idx UNINDEXED,
    chunk_off UNINDEXED,
    text
);
CREATE TABLE IF NOT EXISTS pages(
    page_id INTEGER PRIMARY KEY,
    doc_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    href TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT '',
    search_priority REAL NOT NULL DEFAULT 1,
    content_hash TEXT NOT NULL,
    UNIQUE(doc_id, href)
);
CREATE TABLE IF NOT EXISTS page_chunks(
    chunk_id INTEGER PRIMARY KEY,
    page_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    heading_path TEXT NOT NULL,
    text TEXT NOT NULL,
    FOREIGN KEY(page_id) REFERENCES pages(page_id) ON DELETE CASCADE
);
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
    page_id UNINDEXED,
    chunk_id UNINDEXED,
    href UNINDEXED,
    title,
    description,
    headings,
    body,
    category,
    tokenize = 'porter unicode61'
);
CREATE TABLE IF NOT EXISTS qa(
    topic TEXT NOT NULL,
    id TEXT NOT NULL,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    cited_urls TEXT NOT NULL,
    doc_ids TEXT NOT NULL,
    evidence TEXT NOT NULL,
    iterations INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    corpus_epoch INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(topic, id)
);
CREATE TABLE IF NOT EXISTS commits(
    epoch INTEGER PRIMARY KEY AUTOINCREMENT,
    root_hash TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
";

/// FTS chunk size in chars (with word-boundary splitting).
const CHUNK: usize = 1200;
/// Genesis previous-hash for the commitment chain.
const GENESIS: &str = "GENESIS";

pub struct DocumentStore {
    conn: std::sync::Mutex<Connection>,
}

impl DocumentStore {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("terp-docs.sqlite");
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        conn.execute_batch(SCHEMA).context("Failed to init schema")?;
        // Modest, bot-friendly pragmas. busy_timeout so parallel sub-loops
        // block briefly instead of failing; WAL keeps readers concurrent.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        )
        .ok();
        debug!(path = %path.display(), "document store initialized (sqlite+fts5)");
        let store = Self {
            conn: std::sync::Mutex::new(conn),
        };
        store.reindex_pages_if_empty()?;
        Ok(store)
    }

    /// Poison-tolerant lock: a panicking holder must not kill the store.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Deterministic Q/A id (same scheme as the RLM engine).
    pub fn qa_id(topic: &str, question: &str) -> String {
        blake3::hash(format!("{topic}{question}").as_bytes())
            .to_hex()
            .to_string()
    }

    // ── corpus ──────────────────────────────────────────────────────────

    /// Store a document. Returns its content-addressed DocId.
    /// Idempotent: same content = same ID. Same (label, source) with new
    /// content supersedes the previous version (pointer swing, old blob kept).
    pub async fn store(
        &self,
        content: &[u8],
        name: &str,
        source: &str,
        label: &str,
        url_context: Option<&str>,
    ) -> Result<DocId> {
        let id = blake3::hash(content).to_hex().to_string();
        let now = chrono::Utc::now().timestamp();
        // One transaction: doc row + FTS chunks + version pointer + epoch
        // commit land atomically (parallel sub-loops share this connection).
        let mut conn = self.lock();
        let tx = conn.transaction()?;

        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM docs WHERE id = ?1)",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !exists {
            tx.execute(
                "INSERT INTO docs(id, name, source, label, size, ingested_at, url_context, content)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, name, source, label, content.len() as i64, now, url_context, content],
            )?;
            let text = String::from_utf8_lossy(content);
            index_chunks(&tx, &id, &text)?;
            index_pages(&tx, &id, name, source, label, &text)?;
        }
        tx.execute(
            "INSERT INTO versions(label, source, doc_id, updated_at)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(label, source) DO UPDATE SET doc_id = excluded.doc_id, updated_at = excluded.updated_at",
            params![label, source, id, now],
        )?;
        Self::append_commit_tx(&tx, now)?;
        tx.commit()?;

        debug!(doc_id = %id, name, label, source, size = content.len(), "document stored");
        Ok(id)
    }

    pub async fn get_content(&self, doc_id: &str) -> Result<Vec<u8>> {
        let conn = self.lock();
        let content: Vec<u8> = conn
            .query_row("SELECT content FROM docs WHERE id = ?1", params![doc_id], |r| {
                r.get(0)
            })
            .map_err(|_| anyhow::anyhow!("document not found: {}", doc_id))?;
        Ok(content)
    }

    pub async fn get_meta(&self, doc_id: &str) -> Result<DocMeta> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, name, source, label, size, ingested_at, url_context FROM docs WHERE id = ?1",
            params![doc_id],
            row_to_meta,
        )
        .map_err(|_| anyhow::anyhow!("document metadata not found: {}", doc_id))
    }

    /// List all documents, newest first (includes superseded — history view).
    pub async fn list(&self, limit: usize, _offset: usize) -> Result<Vec<DocMeta>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, source, label, size, ingested_at, url_context
             FROM docs ORDER BY ingested_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_meta)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// List **current-version** documents for a label (what the RLM loop sees).
    pub async fn list_by_label(&self, label: &str) -> Result<Vec<DocMeta>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT d.id, d.name, d.source, d.label, d.size, d.ingested_at, d.url_context
             FROM versions v JOIN docs d ON d.id = v.doc_id WHERE v.label = ?1",
        )?;
        let rows = stmt
            .query_map(params![label], row_to_meta)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete a document, its index rows, and its version pointer (if current).
    pub async fn delete(&self, doc_id: &str) -> Result<()> {
        let meta = self.get_meta(doc_id).await?;
        let now = chrono::Utc::now().timestamp();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM docs_fts WHERE doc_id = ?1", params![doc_id])?;
        drop_pages_for_doc(&tx, doc_id)?;
        tx.execute("DELETE FROM docs WHERE id = ?1", params![doc_id])?;
        tx.execute(
            "DELETE FROM versions WHERE label = ?1 AND source = ?2 AND doc_id = ?3",
            params![meta.label, meta.source, doc_id],
        )?;
        Self::append_commit_tx(&tx, now)?;
        tx.commit()?;
        debug!(doc_id, "document deleted");
        Ok(())
    }

    /// Read a char-range section from a document. Capped at 100K chars.
    pub async fn get_section(
        &self,
        doc_id: &str,
        offset: usize,
        length: usize,
    ) -> Result<String> {
        let content = self.get_content(doc_id).await?;
        let text = String::from_utf8_lossy(&content);
        let chars: Vec<char> = text.chars().collect();
        let len = length.min(100_000);
        let start = offset.min(chars.len());
        let end = (start + len).min(chars.len());
        Ok(chars[start..end].iter().collect())
    }

    /// Chunk FTS5 within one document (OR logic, BM25 ranked, snippeted).
    ///
    /// The live WASIX guest does not call this. It calls [`Self::search_pages`].
    /// `rlm/exec.rs` (unlinked in-process tools) now calls `search_pages` too.
    /// This stays for the chunk-snippet tests and any caller that still wants
    /// a char `offset` into the blob.
    pub async fn search(
        &self,
        doc_id: &str,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<DocExcerpt>> {
        let Some(match_query) = fts_query(query) else {
            return Ok(vec![]);
        };
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT chunk_off, snippet(docs_fts, 3, '【', '】', '…', 24)
             FROM docs_fts WHERE doc_id = ?1 AND text MATCH ?2
             ORDER BY bm25(docs_fts) LIMIT ?3",
        )?;
        let terms: Vec<String> = match_query_terms(query);
        let rows = stmt.query_map(params![doc_id, match_query, max_results as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (chunk_off, snippet) = row?;
            let plain = snippet.replace('【', "").replace('】', "");
            let match_count = terms.iter().filter(|t| plain.to_lowercase().contains(t.as_str())).count();
            results.push(DocExcerpt {
                doc_id: doc_id.to_string(),
                offset: chunk_off.max(0) as usize,
                content: snippet,
                match_count,
            });
        }
        // Rows arrive BM25-ordered. `sort_by` is stable, so this only promotes
        // chunks that match more query terms and keeps BM25 order within a tie.
        results.sort_by(|a, b| b.match_count.cmp(&a.match_count));
        Ok(results)
    }

    /// Page-collapsed FTS5 search (title/heading weighted, prefix + aliases).
    pub async fn search_pages(
        &self,
        topic: &str,
        query: &str,
        doc_id: Option<&str>,
        max_pages: usize,
    ) -> Result<Vec<PageHit>> {
        let (primary, extra) = pages::expand_query_terms(query);
        if primary.is_empty() {
            return Ok(vec![]);
        }
        let match_sql = fts_pages_match(&primary, &extra);
        let conn = self.lock();
        let sql = if doc_id.is_some() {
            "SELECT p.page_id, p.doc_id, p.href, p.title, c.heading_path, c.text,
                    bm25(pages_fts, 0, 0, 0, 4.0, 3.0, 2.5, 1.0, 1.0) AS rank
             FROM pages_fts
             JOIN page_chunks c ON c.chunk_id = pages_fts.chunk_id
             JOIN pages p ON p.page_id = pages_fts.page_id
             WHERE p.topic = ?1 AND p.doc_id = ?2 AND pages_fts MATCH ?3
             ORDER BY rank
             LIMIT 40"
        } else {
            "SELECT p.page_id, p.doc_id, p.href, p.title, c.heading_path, c.text,
                    bm25(pages_fts, 0, 0, 0, 4.0, 3.0, 2.5, 1.0, 1.0) AS rank
             FROM pages_fts
             JOIN page_chunks c ON c.chunk_id = pages_fts.chunk_id
             JOIN pages p ON p.page_id = pages_fts.page_id
             WHERE p.topic = ?1 AND pages_fts MATCH ?2
             ORDER BY rank
             LIMIT 40"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut hits: Vec<PageHit> = Vec::new();
        if let Some(did) = doc_id {
            let rows = stmt.query_map(params![topic, did, match_sql], page_hit_row)?;
            for row in rows {
                hits.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![topic, match_sql], page_hit_row)?;
            for row in rows {
                hits.push(row?);
            }
        }
        Ok(collapse_pages(hits, max_pages))
    }

    fn reindex_pages_if_empty(&self) -> Result<()> {
        let n: i64 = {
            let conn = self.lock();
            conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
                .unwrap_or(0)
        };
        if n > 0 {
            return Ok(());
        }
        let rows: Vec<(String, String, String, String, Vec<u8>)> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT d.id, d.name, d.source, d.label, d.content
                 FROM versions v JOIN docs d ON d.id = v.doc_id",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            })?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for (id, name, source, label, content) in rows {
            let text = String::from_utf8_lossy(&content);
            index_pages(&tx, &id, &name, &source, &label, &text)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Extract file/section headers from an ingested document.
    /// Githem-core uses `=== filename ===` as section delimiters.
    pub async fn list_files(&self, doc_id: &str) -> Result<Vec<(usize, String)>> {
        let content = self.get_content(doc_id).await?;
        let text = String::from_utf8_lossy(&content);
        let mut files = Vec::new();
        let mut char_offset = 0;

        for line in text.lines() {
            if line.starts_with("=== ") && line.ends_with(" ===") {
                let name = line.trim_start_matches("=== ").trim_end_matches(" ===");
                files.push((char_offset, name.to_string()));
            }
            char_offset += line.chars().count() + 1; // +1 for newline
        }

        Ok(files)
    }

    /// Unique labels with a current version (for autocomplete).
    pub async fn labels(&self) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT label FROM versions ORDER BY label")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// True when `doc_id` is the current version for its (label, source).
    pub async fn is_current(&self, doc_id: &str) -> Result<bool> {
        let conn = self.lock();
        let current: Option<String> = conn
            .query_row(
                "SELECT v.doc_id FROM versions v JOIN docs d
                 ON d.label = v.label AND d.source = v.source WHERE d.id = ?1",
                params![doc_id],
                |r| r.get(0),
            )
            .ok();
        Ok(current.as_deref() == Some(doc_id))
    }

    // ── commitment log ──────────────────────────────────────────────────

    /// Latest corpus epoch (0 when empty).
    pub async fn current_epoch(&self) -> Result<i64> {
        let conn = self.lock();
        let epoch: Option<i64> =
            conn.query_row("SELECT MAX(epoch) FROM commits", [], |r| r.get(0))?;
        Ok(epoch.unwrap_or(0))
    }

    /// Latest corpus root hash (empty when no commits yet).
    pub async fn corpus_root(&self) -> Result<String> {
        let conn = self.lock();
        let root: Option<String> = conn
            .query_row(
                "SELECT root_hash FROM commits ORDER BY epoch DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(root.unwrap_or_default())
    }

    /// Append a hash-chained epoch over the current version set.
    /// Must be called inside the mutation transaction.
    fn append_commit_tx(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<()> {
        let mut stmt = tx.prepare("SELECT label, source, doc_id FROM versions ORDER BY label, source, doc_id")?;
        let mut lines = Vec::new();
        let rows = stmt.query_map([], |r| {
            Ok(format!("{}|{}|{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        for row in rows {
            lines.push(row?);
        }
        let prev: Option<String> = tx
            .query_row("SELECT root_hash FROM commits ORDER BY epoch DESC LIMIT 1", [], |r| r.get(0))
            .ok()
            .flatten();
        let prev = prev.unwrap_or_else(|| GENESIS.to_string());
        let mut h = blake3::Hasher::new();
        h.update(prev.as_bytes());
        h.update(b"\n");
        h.update(lines.join("\n").as_bytes());
        let root = h.finalize().to_hex().to_string();
        tx.execute(
            "INSERT INTO commits(root_hash, prev_hash, created_at) VALUES(?1, ?2, ?3)",
            params![root, prev, now],
        )?;
        Ok(())
    }

    // ── Q/A tiers ───────────────────────────────────────────────────────

    /// Log a Q/A record (dataset tier). Upserts on (topic, id).
    ///
    /// Integrity rule: re-logging with *changed* content (answer, doc set,
    /// or evidence) clears `verified` — a human approved the old text, not
    /// the new text. Identical re-logs keep the bit.
    pub async fn store_qa(&self, record: &QaRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO qa(topic, id, question, answer, cited_urls, doc_ids, evidence,
                            iterations, timestamp, verified, corpus_epoch)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(topic, id) DO UPDATE SET
               question = excluded.question, answer = excluded.answer,
               cited_urls = excluded.cited_urls, doc_ids = excluded.doc_ids,
               evidence = excluded.evidence, iterations = excluded.iterations,
               timestamp = excluded.timestamp, corpus_epoch = excluded.corpus_epoch,
               verified = CASE
                 WHEN qa.answer = excluded.answer
                  AND qa.doc_ids = excluded.doc_ids
                  AND qa.evidence = excluded.evidence
                 THEN qa.verified ELSE 0 END",
            params![
                record.topic,
                record.id,
                record.question,
                record.answer,
                serde_json::to_string(&record.cited_urls).unwrap_or_default(),
                serde_json::to_string(&record.doc_ids).unwrap_or_default(),
                serde_json::to_string(&record.evidence).unwrap_or_default(),
                record.iterations as i64,
                record.timestamp,
                record.verified as i64,
                record.corpus_epoch,
            ],
        )?;
        debug!(qa_id = %record.id, topic = %record.topic, verified = record.verified, "Q/A record stored");
        Ok(())
    }

    /// Mark a record human-verified (promotion to the re-usable tier).
    /// Only verified records are ever served back into the loop.
    pub async fn verify_qa(&self, topic: &str, id: &str) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE qa SET verified = 1 WHERE topic = ?1 AND id = ?2",
            params![topic, id],
        )?;
        Ok(n > 0)
    }

    /// List Q/A records for a topic, newest first (dataset tier — everything).
    pub async fn list_qa(&self, topic: &str, limit: usize) -> Result<Vec<QaRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT topic, id, question, answer, cited_urls, doc_ids, evidence,
                    iterations, timestamp, verified, corpus_epoch
             FROM qa WHERE topic = ?1 ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![topic, limit as i64], row_to_qa)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Verified records whose entire doc set is still current.
    /// This is the ONLY Q/A tier the reasoning loop may consume, and only
    /// as labeled previously-verified context — never as primary evidence.
    pub async fn list_verified(&self, topic: &str, limit: usize) -> Result<Vec<QaRecord>> {
        let all = self.list_qa(topic, limit * 4).await?;
        let mut out = Vec::new();
        for r in all {
            if !r.verified {
                continue;
            }
            let mut fresh = true;
            for id in &r.doc_ids {
                if !self.is_current(id).await.unwrap_or(false) {
                    fresh = false;
                    break;
                }
            }
            if fresh {
                out.push(r);
            }
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn row_to_meta(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocMeta> {
    Ok(DocMeta {
        id: r.get(0)?,
        name: r.get(1)?,
        source: r.get(2)?,
        label: r.get(3)?,
        size: r.get::<_, i64>(4)? as usize,
        ingested_at: r.get(5)?,
        url_context: r.get(6)?,
    })
}

fn row_to_qa(r: &rusqlite::Row<'_>) -> rusqlite::Result<QaRecord> {
    let cited: String = r.get(4)?;
    let ids: String = r.get(5)?;
    let evidence: String = r.get(6)?;
    Ok(QaRecord {
        topic: r.get(0)?,
        id: r.get(1)?,
        question: r.get(2)?,
        answer: r.get(3)?,
        cited_urls: serde_json::from_str(&cited).unwrap_or_default(),
        doc_ids: serde_json::from_str(&ids).unwrap_or_default(),
        evidence: serde_json::from_str(&evidence).unwrap_or_default(),
        iterations: r.get::<_, i64>(7)? as u32,
        timestamp: r.get(8)?,
        verified: r.get::<_, i64>(9)? != 0,
        corpus_epoch: r.get(10)?,
    })
}

fn page_hit_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PageHit> {
    let body: String = r.get(5)?;
    let excerpt: String = body.chars().take(280).collect();
    let rank: f64 = r.get(6)?;
    Ok(PageHit {
        page_id: r.get(0)?,
        doc_id: r.get(1)?,
        href: r.get(2)?,
        title: r.get(3)?,
        heading_path: r.get(4)?,
        score: -rank,
        excerpt,
    })
}

fn collapse_pages(hits: Vec<PageHit>, max_pages: usize) -> Vec<PageHit> {
    let mut by_href: Vec<PageHit> = Vec::new();
    for h in hits {
        if let Some(ex) = by_href.iter_mut().find(|p| p.href == h.href) {
            if h.score > ex.score {
                *ex = h;
            }
        } else {
            by_href.push(h);
        }
    }
    by_href.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    by_href.truncate(max_pages);
    by_href
}

fn fts_pages_match(primary: &[String], extra: &[String]) -> String {
    let mut parts = Vec::new();
    for t in primary {
        let q = t.replace('"', "");
        if q.len() >= 3 {
            parts.push(format!("\"{q}\" OR {q}*"));
        } else {
            parts.push(format!("\"{q}\""));
        }
    }
    for t in extra {
        let q = t.replace('"', "");
        if q.len() >= 3 {
            parts.push(format!("{q}*"));
        }
    }
    parts.join(" OR ")
}

fn drop_pages_for_doc(tx: &rusqlite::Transaction<'_>, doc_id: &str) -> Result<()> {
    let ids: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT page_id FROM pages WHERE doc_id = ?1")?;
        let rows = stmt.query_map(params![doc_id], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for pid in ids {
        tx.execute("DELETE FROM pages_fts WHERE page_id = ?1", params![pid])?;
        tx.execute("DELETE FROM page_chunks WHERE page_id = ?1", params![pid])?;
    }
    tx.execute("DELETE FROM pages WHERE doc_id = ?1", params![doc_id])?;
    Ok(())
}

fn index_pages(
    tx: &rusqlite::Transaction<'_>,
    doc_id: &str,
    name: &str,
    source: &str,
    label: &str,
    text: &str,
) -> Result<()> {
    drop_pages_for_doc(tx, doc_id)?;
    let parsed = pages::parse_pages(name, source, text);
    let mut used_href = std::collections::HashSet::new();
    for page in parsed {
        let mut href = page.href.clone();
        let mut n = 2u32;
        while !used_href.insert(href.clone()) {
            href = format!("{}-{}", page.href, n);
            n += 1;
        }
        tx.execute(
            "INSERT INTO pages(doc_id, topic, href, title, description, category, search_priority, content_hash)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![
                doc_id,
                label,
                href,
                page.title,
                page.description,
                page.category,
                blake3::hash(page.chunks.iter().map(|c| c.text.as_str()).collect::<String>().as_bytes()).to_hex().to_string()
            ],
        )?;
        let page_id: i64 = tx.last_insert_rowid();
        for (ord, chunk) in page.chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO page_chunks(page_id, ordinal, heading_path, text) VALUES(?1, ?2, ?3, ?4)",
                params![page_id, ord as i64, chunk.heading_path, chunk.text],
            )?;
            let chunk_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO pages_fts(page_id, chunk_id, href, title, description, headings, body, category)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    page_id,
                    chunk_id,
                    href,
                    page.title,
                    page.description,
                    chunk.heading_path,
                    chunk.text,
                    page.category
                ],
            )?;
        }
    }
    Ok(())
}

/// Split text into word-boundary chunks and index each with its char offset.
fn index_chunks(tx: &rusqlite::Transaction<'_>, doc_id: &str, text: &str) -> Result<()> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0usize;
    let mut idx = 0i64;
    while start < chars.len() {
        let mut end = (start + CHUNK).min(chars.len());
        if end < chars.len() {
            // Back off to a word boundary.
            let mut back = end;
            while back > start && !chars[back].is_whitespace() && end - back < 120 {
                back -= 1;
            }
            if back > start {
                end = back;
            }
        }
        let chunk: String = chars[start..end].iter().collect();
        tx.execute(
            "INSERT INTO docs_fts(doc_id, chunk_idx, chunk_off, text) VALUES(?1, ?2, ?3, ?4)",
            params![doc_id, idx, start as i64, chunk],
        )?;
        idx += 1;
        start = end;
    }
    Ok(())
}

/// Raw query terms (lowercased, for match-count ranking).
fn match_query_terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect()
}

/// Build a safe FTS5 MATCH expression (OR logic, terms quoted).
fn fts_query(query: &str) -> Option<String> {
    let mut terms = match_query_terms(query);
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> DocumentStore {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "terp-test-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            n
        ));
        DocumentStore::new(&dir).await.unwrap()
    }

    #[tokio::test]
    async fn version_supersede_keeps_one_current() {
        let s = test_store().await;
        let v1 = s.store(b"staking rewards are 5%", "a.md", "src:x", "terp", None).await.unwrap();
        let v2 = s.store(b"staking rewards are 7%", "a.md", "src:x", "terp", None).await.unwrap();
        assert_ne!(v1, v2);
        let current = s.list_by_label("terp").await.unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].id, v2);
        assert!(!s.is_current(&v1).await.unwrap());
        assert!(s.is_current(&v2).await.unwrap());
        // History retained.
        assert_eq!(s.list(10, 0).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn fts_search_ranks_and_snippets() {
        let s = test_store().await;
        let id = s
            .store(b"the quick brown fox jumps. staking rewards compound daily for validators.", "a.md", "src:x", "terp", None)
            .await
            .unwrap();
        let hits = s.search(&id, "staking validators", 5).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].match_count >= 1);
        assert!(hits[0].content.contains('【'));
    }

    #[tokio::test]
    async fn page_search_collapses_and_boosts_title() {
        let s = test_store().await;
        let body = b"=== src/pages/guides/validators.mdx ===\n# Validators\n\n## Staking\n\nValidators participate in the bonded set.\n\n=== src/pages/guides/ibc.mdx ===\n# IBC\n\nChannels mention validators only in passing.\n";
        s.store(body, "docs", "github:terp/docs", "terp-docs", None)
            .await
            .unwrap();
        let hits = s
            .search_pages("terp-docs", "validator staking", None, 5)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].href, "/guides/validators");
        assert!(hits[0].title.to_lowercase().contains("validator"));
        assert!(hits.iter().filter(|h| h.href == "/guides/validators").count() <= 1);
    }

    #[tokio::test]
    async fn verify_bit_clears_on_changed_relog() {
        let s = test_store().await;
        let v1 = s.store(b"material", "a.md", "src:x", "terp", None).await.unwrap();
        let epoch = s.current_epoch().await.unwrap();
        let mk = |answer: &str| QaRecord {
            id: DocumentStore::qa_id("terp", "q?"),
            topic: "terp".into(),
            question: "q?".into(),
            answer: answer.into(),
            cited_urls: vec![],
            doc_ids: vec![v1.clone()],
            evidence: vec![],
            iterations: 1,
            timestamp: 1,
            verified: false,
            corpus_epoch: epoch,
        };
        s.store_qa(&mk("first")).await.unwrap();
        assert!(s.verify_qa("terp", &DocumentStore::qa_id("terp", "q?")).await.unwrap());
        // Identical re-log keeps the bit.
        s.store_qa(&mk("first")).await.unwrap();
        assert_eq!(s.list_verified("terp", 10).await.unwrap().len(), 1);
        // Changed answer clears it.
        s.store_qa(&mk("second")).await.unwrap();
        assert!(s.list_verified("terp", 10).await.unwrap().is_empty());
        // Dataset tier still holds the latest text.
        assert_eq!(s.list_qa("terp", 10).await.unwrap()[0].answer, "second");
    }

    #[tokio::test]
    async fn commitment_chain_advances() {
        let s = test_store().await;
        assert_eq!(s.current_epoch().await.unwrap(), 0);
        s.store(b"v1", "a.md", "src:x", "terp", None).await.unwrap();
        let e1 = s.current_epoch().await.unwrap();
        assert_eq!(e1, 1);
        let r1 = s.corpus_root().await.unwrap();
        assert!(!r1.is_empty());
        s.store(b"v2", "a.md", "src:x", "terp", None).await.unwrap();
        assert_eq!(s.current_epoch().await.unwrap(), 2);
        assert_ne!(s.corpus_root().await.unwrap(), r1);
    }

    #[tokio::test]
    async fn verified_tier_filters_unverified_and_stale() {
        let s = test_store().await;
        let v1 = s.store(b"answer material v1", "a.md", "src:x", "terp", None).await.unwrap();
        let epoch = s.current_epoch().await.unwrap();
        let rec = QaRecord {
            id: DocumentStore::qa_id("terp", "q?"),
            topic: "terp".into(),
            question: "q?".into(),
            answer: "a".into(),
            cited_urls: vec![],
            doc_ids: vec![v1.clone()],
            evidence: vec![],
            iterations: 3,
            timestamp: 1,
            verified: false,
            corpus_epoch: epoch,
        };
        s.store_qa(&rec).await.unwrap();
        // Logged but not served.
        assert_eq!(s.list_qa("terp", 10).await.unwrap().len(), 1);
        assert!(s.list_verified("terp", 10).await.unwrap().is_empty());
        // Verify → served while current.
        assert!(s.verify_qa("terp", &rec.id).await.unwrap());
        assert_eq!(s.list_verified("terp", 10).await.unwrap().len(), 1);
        // Supersede the doc → record goes stale, drops out of serving tier.
        s.store(b"answer material v2", "a.md", "src:x", "terp", None).await.unwrap();
        assert!(!s.is_current(&v1).await.unwrap());
        assert!(s.list_verified("terp", 10).await.unwrap().is_empty());
        // …but remains in the dataset tier.
        assert_eq!(s.list_qa("terp", 10).await.unwrap().len(), 1);
    }
}
