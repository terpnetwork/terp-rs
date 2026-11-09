use anyhow::{Context, Result};
use tracing::info;

use super::types::DocId;
use super::DocumentStore;

/// Derive a default URL context from a GitHub repo URL + branch.
/// Returns a natural-language instruction the LLM can use to construct links.
fn derive_github_url_context(url: &str, branch: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    if trimmed.contains("github.com") {
        Some(format!(
            "Source files from this repository are publicly viewable at {}/blob/{}/{{filepath}}",
            trimmed, branch
        ))
    } else {
        None
    }
}

/// Ingest a GitHub repository using githem-core.
/// Returns (doc_id, file_count).
pub async fn ingest_github_repo(
    store: &DocumentStore,
    url: &str,
    label: &str,
    doc_type: Option<&str>,
    branch: Option<&str>,
    url_context: Option<&str>,
) -> Result<(DocId, usize)> {
    // Validate GitHub URL
    let _parsed =
        githem_core::parse_github_url(url).context("Invalid GitHub URL")?;

    let preset = match doc_type {
        Some("code") => githem_core::FilterPreset::CodeOnly,
        Some("minimal") => githem_core::FilterPreset::Minimal,
        _ => githem_core::FilterPreset::Standard,
    };

    let opts = githem_core::IngestOptions::with_preset(preset);

    // Clone and ingest — this is blocking I/O so run in spawn_blocking
    let url_owned = url.to_string();
    let output = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let ingester =
            githem_core::Ingester::from_url_cached(&url_owned, opts)?;
        let mut output = Vec::new();
        ingester.ingest(&mut output)?;
        Ok(output)
    })
    .await
    .context("spawn_blocking join failed")??;

    // Count files from githem output format: "=== path/to/file ===\n"
    let text = String::from_utf8_lossy(&output);
    let file_count = text.matches("=== ").count();

    // Extract repo name from URL for the document name
    let name = url
        .trim_end_matches('/')
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/");

    let source = format!("github:{}", name);

    // Derive url_context: explicit override > auto-derived from GitHub URL
    let effective_branch = branch.unwrap_or("main");
    let effective_url_context = url_context
        .map(|s| s.to_string())
        .or_else(|| derive_github_url_context(url, effective_branch));

    let doc_id = store
        .store(
            &output,
            &name,
            &source,
            label,
            effective_url_context.as_deref(),
        )
        .await?;

    info!(
        doc_id = %doc_id,
        file_count,
        size = output.len(),
        label,
        url_context = ?effective_url_context,
        "GitHub repo ingested"
    );

    Ok((doc_id, file_count))
}

/// Ingest a web page by fetching its content.
///
/// Vocs SPA HTML is rejected as a corpus. Prefer `.md` routes, then
/// `llms.txt` (one page per listed route), never the chrome shell.
pub async fn ingest_url(
    store: &DocumentStore,
    url: &str,
    label: &str,
) -> Result<(DocId, usize)> {
    let url = normalize_http_url(url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("edgar-rlm/0.1 (docs-ingest)")
        .build()
        .context("http client")?;
    let origin = origin_of(&url);

    // Vocs static hosts: one dump beats N HTML/SPA fetches (Discord follow-up
    // window is 15m; slash ACK is 3s and must already be deferred).
    let mut text = None;
    if let Ok(full) = fetch_as_text(&client, &format!("{origin}/llms-full.txt")).await {
        if full.len() > 500 && !super::pages::looks_like_vocs_shell(&full) {
            info!(bytes = full.len(), origin, "using Vocs llms-full.txt");
            text = Some(
                super::pages::bundle_llms_full(&full).unwrap_or(full),
            );
        }
    }
    if text.is_none() {
        if let Some(bundle) = ingest_llms_txt(&client, &origin).await? {
            text = Some(bundle);
        }
    }
    if text.is_none() {
        let mut page = fetch_as_text(&client, &url).await?;
        if super::pages::looks_like_vocs_shell(&page) {
            if let Ok(md) = fetch_as_text(&client, &md_url(&url)).await {
                if !super::pages::looks_like_vocs_shell(&md) && md.len() > 200 {
                    page = md;
                }
            }
        }
        if super::pages::looks_like_vocs_shell(&page) {
            anyhow::bail!(
                "refusing Vocs SPA shell at {url} — site has no usable llms-full.txt / llms.txt / .md"
            );
        }
        text = Some(page);
    }
    let text = text.unwrap();

    let name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url.as_str());
    let source = format!("url:{}", url);
    let url_context = Some(format!(
        "Public docs: pages map to {origin}{{href}} (append .md for markdown)."
    ));
    let doc_id = store
        .store(text.as_bytes(), name, &source, label, url_context.as_deref())
        .await?;

    info!(doc_id = %doc_id, size = text.len(), label, "URL ingested");
    Ok((doc_id, text.len()))
}

fn normalize_http_url(url: &str) -> String {
    let u = url.trim();
    if u.starts_with("http://") || u.starts_with("https://") {
        u.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", u.trim_end_matches('/'))
    }
}

fn origin_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split('/').next().unwrap_or(rest);
    let scheme = url.split("://").next().unwrap_or("https");
    format!("{scheme}://{host}")
}

fn md_url(url: &str) -> String {
    let u = url.trim_end_matches('/');
    if u.ends_with(".md") {
        u.to_string()
    } else if u.ends_with(".html") {
        format!("{}.md", u.trim_end_matches(".html"))
    } else {
        format!("{u}.md")
    }
}

async fn fetch_as_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await.context("Failed to fetch URL")?;
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.bytes().await.context("Failed to read response body")?;
    if content_type.contains("html") {
        html2text::from_read(&body[..], 120)
            .map_err(|e| anyhow::anyhow!("html2text: {e}"))
    } else {
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

async fn ingest_llms_txt(client: &reqwest::Client, origin: &str) -> Result<Option<String>> {
    let idx = match fetch_as_text(client, &format!("{origin}/llms.txt")).await {
        Ok(t) if t.len() > 40 => t,
        _ => return Ok(None),
    };
    let links = super::pages::parse_llms_txt(&idx);
    if links.is_empty() {
        return Ok(None);
    }
    let mut out = String::new();
    let mut n = 0usize;
    for (href, _title) in links.into_iter().take(80) {
        let path = if href.starts_with("http") {
            href.clone()
        } else {
            format!("{origin}{}", if href.starts_with('/') { href.clone() } else { format!("/{href}") })
        };
        let md_u = if path.ends_with(".md") {
            path.clone()
        } else {
            format!("{}.md", path.trim_end_matches('/'))
        };
        let body = match fetch_as_text(client, &md_u).await {
            Ok(b) if !super::pages::looks_like_vocs_shell(&b) && b.len() > 80 => b,
            _ => match fetch_as_text(client, &path).await {
                Ok(b) if !super::pages::looks_like_vocs_shell(&b) => b,
                _ => continue,
            },
        };
        let rel = href.split("://").nth(1).and_then(|r| r.find('/').map(|i| &r[i..])).unwrap_or(href.as_str());
        out.push_str(&format!("=== {} ===\n{}\n", rel.trim_start_matches('/').trim_end_matches(".md"), body));
        n += 1;
    }
    info!(pages = n, origin, "ingested Vocs llms.txt pages");
    if n == 0 {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

/// Extensions ingested as text (everything else is skipped).
const TEXT_EXTS: &[&str] = &[
    "md", "mdx", "txt", "rst", "toml", "yaml", "yml", "json", "rs", "py", "js",
    "ts", "tsx", "jsx", "go", "sh", "sql", "css", "html", "xml", "ini", "cfg",
    "env", "dockerfile", "justfile",
];

/// Directories never descended into.
const SKIP_DIRS: &[&str] = &[
    "target", "node_modules", ".git", "dist", "build", ".next", "vendor",
    "data", "__pycache__", ".cargo",
];

/// Filenames never ingested.
const SKIP_FILES: &[&str] = &["package-lock.json", "Cargo.lock", "pnpm-lock.yaml", "yarn.lock"];

/// Max bytes per file and per ingested document (local sync corpus).
const MAX_FILE_BYTES: usize = 200_000;
const MAX_DOC_BYTES: usize = 4_000_000;

/// Ingest a local file or directory into the store using the same
/// `=== relpath ===` section format githem-core produces, so `list_files`
/// and citations keep working.
/// Paths are confined to `TERP_SYNC_ROOT` (default: the current directory).
/// Returns (doc_id, file_count).
pub async fn ingest_local_path(
    store: &DocumentStore,
    path: &str,
    label: &str,
) -> Result<(DocId, usize)> {
    let root = std::env::var("TERP_SYNC_ROOT").unwrap_or_else(|_| ".".to_string());
    let root = std::path::PathBuf::from(&root)
        .canonicalize()
        .context("bad TERP_SYNC_ROOT")?;
    let target = root.join(path).canonicalize().context("path not found")?;
    if !target.starts_with(&root) {
        anyhow::bail!("path escapes TERP_SYNC_ROOT");
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target);
    } else {
        collect_text_files(&target, &mut files);
    }
    files.sort();

    let mut output = String::new();
    let mut count = 0usize;
    for f in files {
        let rel = f.strip_prefix(&root).unwrap_or(&f);
        if output.len() > MAX_DOC_BYTES {
            output.push_str("\n[... sync snapshot truncated: doc budget reached ...]\n");
            break;
        }
        let bytes = std::fs::read(&f).with_context(|| format!("read {}", f.display()))?;
        if bytes.contains(&0) {
            continue; // binary
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() > MAX_FILE_BYTES {
            text.truncate(MAX_FILE_BYTES);
            text.push_str("\n[... file truncated ...]");
        }
        output.push_str(&format!("=== {} ===\n{}\n", rel.display(), text));
        count += 1;
    }
    if count == 0 {
        anyhow::bail!("no ingestible text files under {}", path);
    }

    let name = format!("local:{path}");
    let source = format!("local:{path}");
    let doc_id = store.store(output.as_bytes(), &name, &source, label, None).await?;

    info!(doc_id = %doc_id, files = count, label, "local path ingested");
    Ok((doc_id, count))
}

fn collect_text_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                collect_text_files(&p, out);
            }
        } else if p.is_file() {
            if SKIP_FILES.contains(&name.as_str()) {
                continue;
            }
            let keep = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| TEXT_EXTS.contains(&e.to_lowercase().as_str()))
                .unwrap_or(false)
                || name.eq_ignore_ascii_case("dockerfile")
                || name.eq_ignore_ascii_case("justfile")
                || name.eq_ignore_ascii_case("just");
            if keep {
                out.push(p);
            }
        }
    }
}
