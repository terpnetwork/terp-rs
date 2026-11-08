//! Split ingested blobs into Vocs-like pages and heading-aware chunks.

/// One logical documentation route.
#[derive(Debug, Clone)]
pub struct ParsedPage {
    pub href: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub chunks: Vec<ParsedChunk>,
}

#[derive(Debug, Clone)]
pub struct ParsedChunk {
    pub heading_path: String,
    pub text: String,
}

/// Split githem/`=== path ===` blobs, else one page from markdown.
pub fn parse_pages(name: &str, source: &str, text: &str) -> Vec<ParsedPage> {
    let sections = split_githem(text);
    if !sections.is_empty() {
        return sections
            .into_iter()
            .filter(|(_, body)| !body.trim().is_empty())
            .map(|(path, body)| page_from_markdown(&path_to_href(&path), &body))
            .collect();
    }
    let href = href_from_source(name, source);
    vec![page_from_markdown(&href, text)]
}

fn split_githem(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current_path = String::new();
    let mut current_body = String::new();
    let mut any = false;
    for line in text.lines() {
        if line.starts_with("=== ") && line.ends_with(" ===") {
            if any {
                out.push((current_path, std::mem::take(&mut current_body)));
            }
            current_path = line
                .trim_start_matches("=== ")
                .trim_end_matches(" ===")
                .to_string();
            any = true;
        } else if any {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if any {
        out.push((current_path, current_body));
    }
    out
}

pub fn path_to_href(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    for prefix in [
        "src/pages/",
        "pages/",
        "docs/pages/",
        "docs/",
        "site/pages/",
        "content/",
        "src/content/",
    ] {
        if let Some(rest) = p.strip_prefix(prefix) {
            p = rest.to_string();
            break;
        }
    }
    for ext in [".mdx", ".md", ".tsx", ".jsx", ".html"] {
        if let Some(rest) = p.strip_suffix(ext) {
            p = rest.to_string();
            break;
        }
    }
    if p.ends_with("/index") {
        p.truncate(p.len() - 6);
    }
    if p.is_empty() {
        return "/".into();
    }
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    p
}

fn href_from_source(name: &str, source: &str) -> String {
    if let Some(url) = source.strip_prefix("url:") {
        let path = url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
            .unwrap_or("/");
        let path = path.split('?').next().unwrap_or(path);
        let path = path.trim_end_matches(".md").trim_end_matches(".html");
        if path.is_empty() {
            return "/".into();
        }
        return path.to_string();
    }
    path_to_href(name)
}

fn page_from_markdown(href: &str, body: &str) -> ParsedPage {
    let (title_fm, description, rest) = split_frontmatter(body);
    let mut title = title_fm.unwrap_or_else(|| title_from_heading(&rest).unwrap_or_else(|| {
        href.rsplit('/').find(|s| !s.is_empty()).unwrap_or(href).replace('-', " ")
    }));
    if title.is_empty() {
        title = href.to_string();
    }
    let category = href
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    let chunks = heading_chunks(&title, &rest);
    ParsedPage {
        href: href.to_string(),
        title,
        description,
        category,
        chunks,
    }
}

fn split_frontmatter(body: &str) -> (Option<String>, String, String) {
    let t = body.trim_start();
    if !t.starts_with("---") {
        return (None, String::new(), body.to_string());
    }
    let after = &t[3..];
    let Some(end) = after.find("\n---") else {
        return (None, String::new(), body.to_string());
    };
    let fm = &after[..end];
    let rest = after[end + 4..].trim_start().to_string();
    let mut title = None;
    let mut description = String::new();
    for line in fm.lines() {
        if let Some(v) = line.strip_prefix("title:") {
            title = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
        if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    (title, description, rest)
}

fn title_from_heading(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn heading_chunks(page_title: &str, body: &str) -> Vec<ParsedChunk> {
    let mut chunks = Vec::new();
    let mut stack: Vec<(usize, String)> = vec![(1, page_title.to_string())];
    let mut buf = String::new();
    let flush = |stack: &[(usize, String)], buf: &mut String, chunks: &mut Vec<ParsedChunk>| {
        let text = buf.trim().to_string();
        buf.clear();
        if text.is_empty() {
            return;
        }
        let path = stack.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" > ");
        chunks.push(ParsedChunk {
            heading_path: path,
            text,
        });
    };
    for line in body.lines() {
        let t = line.trim();
        if let Some(level) = heading_level(t) {
            flush(&stack, &mut buf, &mut chunks);
            let title = t.trim_start_matches('#').trim().to_string();
            stack.retain(|(l, _)| *l < level);
            stack.push((level, title));
        } else {
            buf.push_str(line);
            buf.push('\n');
            if buf.len() > 2500 {
                flush(&stack, &mut buf, &mut chunks);
            }
        }
    }
    flush(&stack, &mut buf, &mut chunks);
    if chunks.is_empty() {
        chunks.push(ParsedChunk {
            heading_path: page_title.to_string(),
            text: body.chars().take(4000).collect(),
        });
    }
    chunks
}

fn heading_level(line: &str) -> Option<usize> {
    if !line.starts_with('#') {
        return None;
    }
    let n = line.chars().take_while(|c| *c == '#').count();
    if n >= 1 && n <= 6 && line.as_bytes().get(n).copied() == Some(b' ') {
        Some(n)
    } else {
        None
    }
}

pub fn looks_like_vocs_shell(text: &str) -> bool {
    let n = text.chars().count();
    if n > 16_000 {
        return false;
    }
    let low = text.to_lowercase();
    let chrome = ["skip to content", "⌘", "search...", "welcome"];
    chrome.iter().filter(|s| low.contains(*s)).count() >= 2
}

/// Turn Vocs `llms-full.txt` into githem-style `=== href ===` pages.
pub fn bundle_llms_full(full: &str) -> Option<String> {
    let links = parse_llms_txt(full);
    if links.len() < 2 {
        return None;
    }
    let body = full
        .find("-->")
        .map(|i| full[i + 3..].trim_start())
        .unwrap_or(full);
    let mut out = String::new();
    let mut from = 0usize;
    for (i, (href, title)) in links.iter().enumerate() {
        let needle = format!("# {title}\n");
        let rel = body.get(from..).and_then(|s| {
            s.find(&format!("\n{needle}"))
                .map(|p| p + 1)
                .or_else(|| s.find(&needle).filter(|_| from == 0 || s.starts_with(&needle)))
        });
        let Some(rel) = rel else {
            continue;
        };
        let start = from + rel;
        let end = links.get(i + 1).and_then(|(_, next)| {
            let n = format!("\n# {next}\n");
            body[start + 1..].find(&n).map(|p| start + 1 + p + 1)
        }).unwrap_or(body.len());
        let chunk = body[start..end].trim();
        if chunk.is_empty() {
            continue;
        }
        let path = href
            .trim_start_matches('/')
            .trim_end_matches('/')
            .trim_end_matches(".md");
        let path = if path.is_empty() { "index" } else { path };
        out.push_str(&format!("=== {path} ===\n{chunk}\n"));
        from = start + 1;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Parse `llms.txt` markdown list links: `- [Title](/path): desc`
pub fn parse_llms_txt(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if !t.starts_with('-') {
            continue;
        }
        let Some(lb) = t.find('[') else { continue };
        let Some(rb) = t.find("](") else { continue };
        let title = t[lb + 1..rb].to_string();
        let rest = &t[rb + 2..];
        let Some(rp) = rest.find(')') else { continue };
        let href = rest[..rp].trim().to_string();
        if href.starts_with("http") || href.starts_with('/') {
            out.push((href, title));
        }
    }
    out
}

/// Domain aliases: extra terms at lower weight (handled in MATCH OR).
pub fn expand_query_terms(query: &str) -> (Vec<String>, Vec<String>) {
    let primary: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect();
    let mut extra = Vec::new();
    for p in &primary {
        for (key, alts) in ALIASES {
            if p == *key || alts.iter().any(|a| a == p) {
                extra.push((*key).to_string());
                for a in *alts {
                    extra.push((*a).to_string());
                }
            }
        }
    }
    extra.sort();
    extra.dedup();
    extra.retain(|e| !primary.iter().any(|p| p == e));
    (primary, extra)
}

const ALIASES: &[(&str, &[&str])] = &[
    (
        "validator",
        &[
            "validators",
            "staking",
            "staker",
            "bonded",
            "slashing",
            "valoper",
            "cometbft",
        ],
    ),
    ("terpd", &["validator", "staking", "consensus", "node"]),
    ("ibc", &["relayer", "channel", "client", "connection"]),
    ("governance", &["proposal", "vote", "gov"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn githem_pages_and_headings() {
        let text = "=== src/pages/guides/validators.mdx ===\n# Validators\n\n## Staking\n\nBonded set.\n\n## Slashing\n\nPenalties.\n";
        let pages = parse_pages("blob", "github:x", text);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].href, "/guides/validators");
        assert_eq!(pages[0].title, "Validators");
        assert!(pages[0].chunks.iter().any(|c| c.heading_path.contains("Staking")));
    }

    #[test]
    fn vocs_shell_detect() {
        assert!(looks_like_vocs_shell(
            "Skip to content\nSearch...\n⌘\n# Welcome\nThe Terp Mission\n"
        ));
        assert!(!looks_like_vocs_shell(
            "# Validators\n\nValidators participate in the bonded set by staking tokens.\n"
        ));
    }

    #[test]
    fn llms_list() {
        let items = parse_llms_txt("- [Validators](/guides/validators): run a node\n");
        assert_eq!(items[0].0, "/guides/validators");
    }

    #[test]
    fn bundle_full_splits_pages() {
        let full = "<!--\n- [Welcome](/index): x\n- [Overview](/overview/): y\n-->\n\n# Welcome\nhello\n\n# Overview\nworld\n";
        let b = bundle_llms_full(full).unwrap();
        assert!(b.contains("=== index ==="));
        assert!(b.contains("=== overview ==="));
        assert!(b.contains("hello"));
        assert!(b.contains("world"));
    }
}
