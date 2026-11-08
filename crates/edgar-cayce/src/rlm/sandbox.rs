//! Sandboxed RLM executor — the ONLY code-execution backend.
//!
//! Isolation model: LLM-generated code runs as a WASIX `python` guest inside
//! `wasmer-sdk`, with [`NetworkPolicy::Disabled`](wasmer_sdk::NetworkPolicy),
//! the topic corpus on a read-only mount, and stdout captured by the host.
//! The host keeps `DocumentStore`/`LlmClient`; `llm_query` and `node_query`
//! are served host-side via nonce-scoped marker round-trips (the sandbox has
//! no network; node reads go through the guard-railed `terp::node` module).
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::docs::types::DocMeta;
use crate::docs::DocumentStore;
use crate::llm::LlmClient;
use wasmer_sdk::{Directory, MountMode, Sandbox, Wasmer};

/// Thread-safe tracker for files accessed during a session (citation fallback).
pub type FileTracker = Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// Guest prelude: document tools backed by the read-only `/corpus/` mount.
///
/// Host-service markers (`llm_query`, `node_query`) are namespaced by a
/// per-execution nonce (`__NONCE`, set by the host prelude): the host only
/// honors markers carrying the current nonce, so document text containing
/// stale or forged marker strings can never trigger a host call.
/// Caches are read live from `globals()` — the shim must NOT snapshot them
/// at import time, because the host prelude defining them runs after.
const SHIM: &str = r#"
import json as _json, re as _re, base64 as _b64

with open('/corpus/manifest.json') as _f:
    _MANIFEST = _json.load(_f)
_DOCS = {d['doc_id']: d for d in _MANIFEST['docs']}
_NONCE = globals().get('__NONCE', '')
snapshot_truncated = bool(_MANIFEST.get('snapshot_truncated', False))

def _host_cache(name):
    c = globals().get(name, {})
    return c if isinstance(c, dict) else {}

def _text(doc_id):
    if doc_id not in _TEXTS:
        d = _DOCS[doc_id]
        with open('/corpus/' + d['file']) as _f:
            _TEXTS[doc_id] = _f.read()
    return _TEXTS[doc_id]

_TEXTS = {}
documents = [{'doc_id': d['doc_id'], 'name': d['name'], 'source': d['source'], 'size': d['size']} for d in _MANIFEST['docs']]

def list_files(doc_id):
    # Same contract as the unlinked in-process tools in rlm/exec.rs:
    # githem `=== path ===` headers, char offsets. No headers → one section.
    t = _text(doc_id)
    out, offset = [], 0
    for line in t.split('\n'):
        if line.startswith('=== ') and line.endswith(' ==='):
            out.append({'offset': offset, 'name': line[4:-4], 'size': 0})
        offset += len(line) + 1
    if not out:
        name = _DOCS.get(doc_id, {}).get('name', doc_id)
        return [{'offset': 0, 'name': name, 'size': len(t)}]
    for i, item in enumerate(out):
        end = out[i + 1]['offset'] if i + 1 < len(out) else len(t)
        item['size'] = max(0, end - item['offset'])
    return out

def read_file(doc_id, filename=None):
    # Partial/fuzzy name match, capped like exec.rs (20k chars of that section).
    t = _text(doc_id)
    if filename:
        files = list_files(doc_id)
        needle = str(filename).lower()
        body = None
        for i, f in enumerate(files):
            if needle in str(f['name']).lower():
                end = files[i + 1]['offset'] if i + 1 < len(files) else len(t)
                body = t[f['offset']:end]
                break
        if body is None:
            return "Error: File '%s' not found. Use list_files() to see available files." % filename
    else:
        body = t
    if len(body) > 20000:
        return body[:20000] + '\n[... truncated %d chars ...]' % (len(body) - 20000)
    return body

def get_section(doc_id, offset, length=2000):
    return _text(doc_id)[offset:offset + length]

def grep(doc_id, pattern, context=3, max_results=10):
    t, lines = _text(doc_id), _text(doc_id).split('\n')
    rx = _re.compile(pattern)
    hits = []
    for n, line in enumerate(lines):
        if rx.search(line):
            lo, hi = max(0, n - context), min(len(lines), n + context + 1)
            hits.append({'line': n, 'context': '\n'.join(lines[lo:hi])})
            if len(hits) >= max_results:
                break
    return hits

def search_document(doc_id, query=None, max_results=8):
    # Host FTS5, page-collapsed (title/heading weighted). The guest has no
    # SQLite of the corpus. Same nonce-guarded marker protocol as llm_query
    # and node_query, so document text cannot forge a host call.
    # The cache key is this canonical JSON. The host stores under that exact
    # string; a mismatch burns the round budget and returns nothing.
    try:
        limit = int(max_results)
    except (TypeError, ValueError):
        limit = 8
    if limit < 1:
        limit = 1
    if limit > 20:
        limit = 20
    if query is None:
        payload = {'query': str(doc_id), 'max_results': limit}
    else:
        payload = {'query': str(query), 'doc_id': str(doc_id), 'max_results': limit}
    key = _json.dumps(payload, sort_keys=True)
    cache = _host_cache('__SEARCH_CACHE')
    if key in cache:
        return cache[key]
    print('@@' + _NONCE + '::SEARCH_QUERY_B64@@' + _b64.b64encode(key.encode()).decode() + '@@END@@')
    raise SystemExit(79)

def llm_query(prompt):
    prompt = str(prompt)
    cache = _host_cache('__LLM_CACHE')
    if prompt in cache:
        return cache[prompt]
    print('@@' + _NONCE + '::LLM_QUERY_B64@@' + _b64.b64encode(prompt.encode()).decode() + '@@END@@')
    raise SystemExit(77)

def node_query(path):
    # Live on-chain read served host-side through the guard-railed node module
    # (read-only allowlisted LCD paths, 32KB cap). No guest network involved.
    path = str(path)
    cache = _host_cache('__NODE_CACHE')
    if path in cache:
        return cache[path]
    print('@@' + _NONCE + '::NODE_QUERY_B64@@' + _b64.b64encode(path.encode()).decode() + '@@END@@')
    raise SystemExit(78)
"#;

const LLM_EXIT_HINT: i32 = 77;
const NODE_EXIT_HINT: i32 = 78;
const SEARCH_EXIT_HINT: i32 = 79;
const MAX_HOST_ROUNDS: usize = 8;

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

/// Sandboxed RLM executor: LLM-generated code runs as a network-less WASIX
/// `python` guest. This is the ONLY execution backend — there is no
/// in-process eval path.
pub struct WasmerSandboxSession {
    sandbox: Sandbox,
    llm: Arc<LlmClient>,
    store: Arc<DocumentStore>,
    http: reqwest::Client,
    docs: Vec<DocMeta>,
    topic: String,
    history: Mutex<Vec<String>>,
    accessed: FileTracker,
    timeout: Duration,
    history_max: usize,
}

impl WasmerSandboxSession {
    /// Build the sandbox and seed the topic corpus on a **read-only** mount.
    ///
    /// The guest can read `/corpus/*` but never modify it, so history
    /// replays always see the same snapshot. Sizes recorded in the manifest
    /// are seeded bytes (honest), and `snapshot_truncated` flags budget cuts.
    pub async fn spawn(
        store: Arc<DocumentStore>,
        llm: Arc<LlmClient>,
        docs: Vec<DocMeta>,
    ) -> Result<Self> {
        let snapshot_max: usize = env_or("RLM_SANDBOX_SNAPSHOT_MAX", "8388608")
            .parse()
            .unwrap_or(8 << 20);
        let timeout_secs: u64 = env_or("RLM_SANDBOX_TIMEOUT_SECS", "90")
            .parse()
            .unwrap_or(90);
        let history_max: usize = env_or("RLM_SANDBOX_HISTORY_MAX", "24000")
            .parse()
            .unwrap_or(24_000);
        let package = env_or("WASMER_PYTHON_PACKAGE", "python/python");

        let wasmer = Wasmer::new().context("wasmer client init failed")?;
        let corpus = Directory::new();

        // Manifest + one text file per doc (truncated to fit the snapshot budget).
        let mut entries = Vec::new();
        let mut budget = snapshot_max;
        let mut truncated_any = false;
        for (i, d) in docs.iter().enumerate() {
            let raw = store.get_content(&d.id).await.unwrap_or_default();
            let mut text = String::from_utf8_lossy(&raw).into_owned();
            if text.len() > budget {
                text.truncate(crate::util::floor_char_boundary(&text, budget));
                text.push_str("\n[... snapshot truncated: doc exceeds remaining budget ...]");
                budget = 0;
                truncated_any = true;
            } else {
                budget -= text.len();
            }
            let seeded = text.len();
            let fname = format!("doc-{i}.txt");
            corpus
                .write_text(fname.as_str(), text)
                .await
                .map_err(|e| anyhow::anyhow!("corpus seed failed: {e}"))?;
            entries.push(serde_json::json!({
                "doc_id": d.id, "name": d.name, "source": d.source,
                "size": seeded, "file": fname,
            }));
        }
        corpus
            .write_text(
                "manifest.json",
                serde_json::to_string(&serde_json::json!({
                    "docs": entries,
                    "snapshot_truncated": truncated_any,
                }))?,
            )
            .await
            .map_err(|e| anyhow::anyhow!("manifest seed failed: {e}"))?;
        if truncated_any {
            warn!(
                doc_count = docs.len(),
                budget = snapshot_max,
                "corpus snapshot truncated to fit budget"
            );
        }

        // Network is disabled by default (NetworkPolicy::Disabled); the guest
        // sees the corpus read-only and a writable workspace for _run.py.
        let sandbox: Sandbox = wasmer
            .sandboxes()
            .create()
            .package(package.as_str())
            .mount("/corpus", corpus, MountMode::ReadOnly)
            .await
            .context("sandbox creation failed")?;
        debug!(doc_count = docs.len(), "wasmer sandbox ready");

        let topic = docs
            .first()
            .map(|d| d.label.clone())
            .unwrap_or_default();
        Ok(Self {
            sandbox,
            llm,
            store: store.clone(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            docs,
            topic,
            history: Mutex::new(Vec::new()),
            accessed: Arc::new(std::sync::Mutex::new(Vec::new())),
            timeout: Duration::from_secs(timeout_secs),
            history_max,
        })
    }

    pub fn accessed_files(&self) -> Vec<(String, String)> {
        self.accessed.lock().unwrap().clone()
    }

    /// Record which known doc_ids appear in the code (approximation of
    /// observed read_file usage, for citation fallback).
    fn track_access(&self, code: &str) {
        let mut acc = self.accessed.lock().unwrap();
        for d in &self.docs {
            if code.contains(d.id.as_str()) && !acc.iter().any(|(id, _)| id == &d.id) {
                acc.push((d.id.clone(), d.name.clone()));
            }
        }
    }

    async fn run_guest(&self, script: &str) -> Result<String> {
        self.sandbox
            .fs()
            .write_text("_run.py", script)
            .await
            .context("sandbox write failed")?;
        let out = self
            .sandbox
            .command("python")
            .arg("/workspace/_run.py")
            .timeout(self.timeout)
            .output_bytes(65536)
            // `Command::run()` errors on any non-zero exit. Host round-trips
            // leave the guest with status 77/78/79 on purpose, so inspect
            // `output()` instead of treating those as spawn failures.
            .output()
            .await
            .context("sandbox run failed")?;
        if out.ok()
            || out.status.code() == LLM_EXIT_HINT
            || out.status.code() == NODE_EXIT_HINT
            || out.status.code() == SEARCH_EXIT_HINT
        {
            Ok(String::from_utf8_lossy(out.stdout.bytes()).into_owned())
        } else {
            let err = String::from_utf8_lossy(out.stderr.bytes());
            let tail: String = err
                .chars()
                .rev()
                .take(1500)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Ok(format!(
                "Error: guest failed (code {}): {}",
                out.status.code(),
                tail.trim()
            ))
        }
    }

    /// Extract a base64 host-service payload for this execution's nonce.
    /// Markers without the current nonce (stale replays, forged strings in
    /// document text) are ignored.
    fn extract_marker(out: &str, nonce: &str, kind: &str) -> Option<String> {
        let marker = format!("@@{nonce}::{kind}@@");
        let s = out.find(&marker)?;
        let rest = &out[s + marker.len()..];
        let e = rest.find("@@END@@")?;
        let raw = &rest[..e];
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw.trim()).ok()?;
        String::from_utf8(bytes).ok()
    }

    /// Fresh execution nonce (blake3 over time + counter: no RNG dep needed).
    fn fresh_nonce(hist_len: usize) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let c = CTR.fetch_add(1, Ordering::SeqCst);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        blake3::hash(format!("{t}:{c}:{hist_len}").as_bytes())
            .to_hex()
            .to_string()[..16]
            .to_string()
    }

    pub async fn execute(&self, code: &str) -> Result<String> {
        self.track_access(code);
        // Cumulative script emulates the persistent REPL (bounded history).
        let mut hist = self.history.lock().await;
        hist.push(code.to_string());
        while hist.join("\n").len() > self.history_max && hist.len() > 1 {
            hist.remove(0);
        }
        let base: String = hist.join("\n");
        let nonce = Self::fresh_nonce(hist.len());
        drop(hist);

        let mut llm_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut node_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut search_cache: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut out = String::new();
        for _ in 0..=MAX_HOST_ROUNDS {
            let mut prelude = format!("__NONCE = {nonce:?}\n");
            if !llm_cache.is_empty() {
                prelude.push_str(&format!(
                    "__LLM_CACHE = {}\n",
                    serde_json::to_string(&llm_cache)?
                ));
            }
            if !node_cache.is_empty() {
                prelude.push_str(&format!(
                    "__NODE_CACHE = {}\n",
                    serde_json::to_string(&node_cache)?
                ));
            }
            if !search_cache.is_empty() {
                prelude.push_str(&format!(
                    "__SEARCH_CACHE = {}\n",
                    serde_json::to_string(&search_cache)?
                ));
            }
            let script = format!("{prelude}{SHIM}\n{base}");
            out = self.run_guest(&script).await?;
            if let Some(prompt) = Self::extract_marker(&out, &nonce, "LLM_QUERY_B64") {
                if llm_cache.contains_key(&prompt) {
                    break;
                }
                debug!("serving sandboxed llm_query host-side");
                let ans = match self.llm.sub_query(&prompt).await {
                    Ok(a) => a,
                    Err(e) => {
                        warn!("host llm_query failed: {e:#}");
                        format!("[llm_query failed: {e}]")
                    }
                };
                llm_cache.insert(prompt, ans);
                continue;
            }
            if let Some(path) = Self::extract_marker(&out, &nonce, "NODE_QUERY_B64") {
                if node_cache.contains_key(&path) {
                    break;
                }
                debug!(path = %path, "serving sandboxed node_query host-side");
                let ans = match crate::terp::node::node_query(&self.http, &path).await {
                    Ok(a) => a,
                    Err(e) => format!("[node_query refused/failed: {e}]"),
                };
                node_cache.insert(path, ans);
                continue;
            }
            if let Some(raw) = Self::extract_marker(&out, &nonce, "SEARCH_QUERY_B64") {
                if search_cache.contains_key(&raw) {
                    break;
                }
                let req = parse_search_payload(&raw);
                debug!(query = %req.query, max = req.max_results, "serving sandboxed search_document host-side");
                let hits = match self
                    .store
                    .search_pages(&self.topic, &req.query, req.doc_id.as_deref(), req.max_results)
                    .await
                {
                    Ok(h) => h,
                    Err(e) => {
                        warn!("host search_pages failed: {e:#}");
                        vec![]
                    }
                };
                let json = serde_json::to_value(&hits).unwrap_or(serde_json::json!([]));
                search_cache.insert(raw, json);
                continue;
            }
            break;
        }
        // Strip marker lines before returning to the reasoning loop.
        let clean: Vec<&str> = out
            .lines()
            .filter(|l| {
                !l.contains("LLM_QUERY_B64@@")
                    && !l.contains("NODE_QUERY_B64@@")
                    && !l.contains("SEARCH_QUERY_B64@@")
            })
            .collect();
        Ok(clean.join("\n"))
    }
}

/// Host view of one `search_document` marker. `max_results` is clamped to
/// the same 1..=20 window the guest uses, so a forged payload cannot ask
/// for an unbounded page collapse.
struct SearchRequest {
    query: String,
    doc_id: Option<String>,
    max_results: usize,
}

fn parse_search_payload(raw: &str) -> SearchRequest {
    const DEFAULT_MAX: usize = 8;
    const MAX_CAP: usize = 20;
    let clamp = |n: usize| n.clamp(1, MAX_CAP);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        let query = v
            .get("query")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(raw)
            .to_string();
        let doc_id = v
            .get("doc_id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let max_results = v
            .get("max_results")
            .and_then(|x| x.as_u64())
            .map(|n| clamp(n as usize))
            .unwrap_or(DEFAULT_MAX);
        return SearchRequest {
            query,
            doc_id,
            max_results,
        };
    }
    SearchRequest {
        query: raw.to_string(),
        doc_id: None,
        max_results: DEFAULT_MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_search_payload;

    #[test]
    fn search_payload_keeps_doc_query_and_limit() {
        // Canonical guest key from json.dumps(..., sort_keys=True), which
        // inserts spaces after separators.
        let raw = r#"{"doc_id": "abc", "max_results": 5, "query": "staking rewards"}"#;
        let req = parse_search_payload(raw);
        assert_eq!(req.query, "staking rewards");
        assert_eq!(req.doc_id.as_deref(), Some("abc"));
        assert_eq!(req.max_results, 5);
    }

    #[test]
    fn search_payload_query_only_defaults_and_caps() {
        let wide = parse_search_payload(r#"{"max_results": 100, "query": "ibc"}"#);
        assert!(wide.doc_id.is_none());
        assert_eq!(wide.query, "ibc");
        assert_eq!(wide.max_results, 20);

        let zero = parse_search_payload(r#"{"doc_id": "d", "max_results": 0, "query": "gov"}"#);
        assert_eq!(zero.max_results, 1);

        let bare = parse_search_payload("not-json");
        assert_eq!(bare.query, "not-json");
        assert!(bare.doc_id.is_none());
        assert_eq!(bare.max_results, 8);
    }

    #[test]
    fn guest_shim_matches_exec_tool_signature() {
        let shim = super::SHIM;
        assert!(shim.contains("def search_document(doc_id, query=None, max_results=8):"));
        assert!(shim.contains("def list_files(doc_id):"));
        assert!(shim.contains("def read_file(doc_id, filename=None):"));
        assert!(shim.contains("=== "));
        assert!(!shim.contains("part-%d"));
    }
}
