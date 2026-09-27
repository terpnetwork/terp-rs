"""Self-contained HTML status report for the dependency control plane.

Renders the same visual language as terp.network/public/graphs/dependency-map.html
(shared tokens, panel layout, ok/warn/bad semantics, data-in-a-JSON-script-tag
contract) over gen-tools' own findings, so dependency status is browsable in a
browser instead of a markdown table nobody reads.

The page embeds no external references: no CDN, no remote font, no eval. It is
one file that works from file:// with the network off, which is the same rule
the offline confirmation page in the KMS spec follows.

Layout choice: the terp.network page uses a hand-rolled force simulation. This
report does not. At 157 crates the question is never "what does the topology
look like", it is "which repositories are broken and what is uncommitted", and a
force layout is actively bad at answering that. Nodes are placed in BFS layers
from a focus crate and coloured by worst-state, so the eye lands on failures
first. The same layering function the existing hierarchy preset uses.
"""

from __future__ import annotations

import html
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path

import dep_probe
from dep_common import (
    MATRIX_PATH,
    REPO_ROOT,
    discover_projects,
    is_repo_only,
    load_defaults,
    load_matrix,
    pkg_name,
    resolve_git_source,
    scan_project,
)

# Worst-state wins, so a repository is coloured by its most serious problem.
STATE_RANK = {"missing": 3, "auth": 2, "unreachable": 2, "invalid-url": 3, "present": 0}
GIT_RANK = {"errored": 3, "outgoing": 2, "prunable": 2, "dirty": 1, "detached": 1, "clean": 0}


def _as_dep(value):
    """Normalize a workspace_deps value to a DepSpec-like view.

    scan_project puts DepSpec objects in member_deps but plain dicts in
    workspace_deps, so both shapes have to be read.
    """
    if hasattr(value, "pkg_name"):
        return {
            "pkg": value.pkg_name,
            "source": value.source,
            "path": value.path,
            "git_url": value.git_url,
        }
    if isinstance(value, dict):
        return {
            "pkg": value.get("package") or value.get("name") or "",
            "source": value.get("source", ""),
            "path": value.get("path"),
            "git_url": value.get("git_url"),
        }
    return None


def _repo_key(url: str) -> str:
    """Normalize a git URL to a stable repo identity."""
    if not url:
        return ""
    trimmed = url.rstrip("/")
    if trimmed.endswith(".git"):
        trimmed = trimmed[:-4]
    return trimmed.rsplit("/", 1)[-1]


def build_payload(include_git: bool = True, include_graph: bool = True) -> dict:
    """Collect every finding the report renders. Read-only.

    include_graph walks all discovered workspaces to derive real dependency
    edges. It is the slow part; the status tables do not need it.
    """
    matrix = load_matrix()
    defaults = load_defaults()
    findings = dep_probe.check_branches_on_remote(matrix, defaults, mode="all")

    # repo identity -> its matrix crates, and the branches each mode resolves to
    repos = {}

    def repo_slot(key):
        return repos.setdefault(
            key,
            {"repo": key, "url": "", "crates": [], "branches": [], "state": "present", "git": "clean"},
        )

    for key, info in sorted(matrix.items()):
        if not isinstance(info, dict):
            continue
        name = pkg_name(key, info)
        modes = {}
        for mode in ("git", "zk_git"):
            url, branch = resolve_git_source(info, mode, defaults)
            if url:
                modes[mode] = (url, branch)
        rkey = _repo_key(modes.get("git", modes.get("zk_git", ("", "")))[0])
        slot = repo_slot(rkey or f"local:{key}")
        if not slot["url"]:
            slot["url"] = (modes.get("git") or modes.get("zk_git") or ("", ""))[0]
        slot["crates"].append({"key": key, "pkg": name, "repo_only": bool(is_repo_only(info))})
        for mode, (url, branch) in sorted(modes.items()):
            entry = {"mode": mode, "branch": branch}
            if entry not in slot["branches"]:
                slot["branches"].append(entry)

    for f in findings:
        key = _repo_key(f.url)
        slot = repos.get(key)
        if slot is None:
            slot = repo_slot(key)
            slot["url"] = f.url
        if STATE_RANK.get(f.state, 2) > STATE_RANK.get(slot["state"], 0):
            slot["state"] = f.state
        slot.setdefault("findings", []).extend(
            {"key": k, "mode": "all", "detail": f.detail} for k in f.keys
        )

    git_by_rel = {}
    if include_git:
        for state in dep_probe.probe_git_state():
            rel = state.rel
            if state.error:
                git = "errored"
            elif state.outgoing:
                git = "outgoing"
            elif state.prunable:
                git = "prunable"
            elif state.dirty:
                git = "dirty"
            elif state.detached:
                git = "detached"
            else:
                git = "clean"
            git_by_rel[rel] = {
                "state": git,
                "branch": state.branch,
                "staged": state.staged,
                "unstaged": state.unstaged,
                "untracked": state.untracked,
                "ahead": state.ahead,
                "upstream": state.upstream,
            }

    # A matrix repo maps onto a monorepo path via its first crate's local dir.
    local_by_repo = {}
    for key, info in sorted(matrix.items()):
        if not isinstance(info, dict):
            continue
        rkey = _repo_key(resolve_git_source(info, "git", defaults)[0] or "")
        local = info.get("local")
        if rkey and local and rkey not in local_by_repo:
            local_by_repo[rkey] = local

    for rkey, slot in repos.items():
        local = local_by_repo.get(rkey)
        if local:
            g = git_by_rel.get(local)
            if g:
                slot["git"] = g["state"]
                slot["git_detail"] = g
        rank = max(STATE_RANK.get(slot["state"], 2), GIT_RANK.get(slot["git"], 0))
        if rank >= 3:
            slot["worst"] = "bad"
        elif rank >= 1:
            slot["worst"] = "warn"
        else:
            slot["worst"] = "ok"

    payload = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "monorepo_root": str(REPO_ROOT),
        "matrix_path": str(MATRIX_PATH),
        "repos": [repos[k] for k in sorted(repos)],
        "git": list(git_by_rel.values()),
        "summary": {
            "repos": len(repos),
            "crates": sum(len(r["crates"]) for r in repos.values()),
            "missing": sum(1 for f in findings if f.state == "missing"),
            "unconfirmed": sum(1 for f in findings if f.state in ("unreachable", "auth", "invalid-url")),
            "dirty": sum(1 for g in git_by_rel.values() if g["state"] in ("dirty", "outgoing", "prunable", "errored")),
        },
    }

    if include_graph:
        payload["graph"] = _build_graph(matrix, defaults, repos)
    return payload


def _build_graph(matrix: dict, defaults: dict, repos: dict) -> dict:
    """Derive real dependency edges between matrix repos from workspace scans."""
    pkg_to_repo = {}
    for slot in repos.values():
        for c in slot["crates"]:
            pkg_to_repo[c["pkg"]] = slot["repo"]

    # local path -> repo identity, so a path dep can be traced to its repo
    path_to_repo = {}
    for key, info in matrix.items():
        if not isinstance(info, dict):
            continue
        local = info.get("local")
        if not local:
            continue
        rkey = _repo_key(resolve_git_source(info, "git", defaults)[0] or "")
        if rkey:
            path_to_repo[local] = rkey

    nodes = {}
    edges = []
    seen = set()

    for project in discover_projects():
        try:
            scan = scan_project(project, matrix)
        except Exception:
            # A workspace that cannot be scanned is reported by diagnostics, not
            # invented here. Never fabricate an edge.
            continue
        owner = None
        for source in scan.workspace_deps.values():
            d = _as_dep(source)
            if d and d.get("path"):
                cand = path_to_repo.get(d["path"])
                if cand:
                    owner = cand
                    break
        if owner is None:
            local = project.rel_path
            owner = path_to_repo.get(local) or f"local:{local}"
        # Only matrix repos are drawable: the layout and the renderer both iterate
        # the repo set. A workspace with no repo identity would become a node with
        # no position, and every edge touching it would be silently dropped.
        if owner not in repos:
            continue
        nodes.setdefault(owner, {"id": owner, "crates": 0})
        nodes[owner]["crates"] += 1

        for dep in list(scan.member_deps) + list(scan.workspace_deps.values()):
            d = _as_dep(dep)
            if not d:
                continue
            target = None
            if d.get("path"):
                target = path_to_repo.get(d["path"])
            if not target and d.get("pkg") in pkg_to_repo:
                target = pkg_to_repo[d["pkg"]]
            if not target or target == owner:
                continue
            if target not in repos:
                continue
            pair = (owner, target)
            if pair in seen:
                continue
            seen.add(pair)
            edges.append({"from": owner, "to": target, "pkg": d["pkg"]})
            nodes.setdefault(target, {"id": target, "crates": 0})

    worst = {r["repo"]: r.get("worst", "ok") for r in repos.values()}
    for node in nodes.values():
        node["worst"] = worst.get(node["id"], "ok")
    return {"nodes": list(nodes.values()), "edges": edges}


_CSS = """
:root {
  --bg:#000; --panel:#0c100e; --panel2:#101612; --border:rgba(152,232,193,.22);
  --border-subtle:rgba(152,232,193,.10); --text:#f8f8f2; --muted:#6272a4;
  --text-muted:#d2d3d8; --accent:#98e8c1; --ok:#98e8c1; --warn:#ffb86c; --bad:#fe7d7d;
  --side-w:min(340px,34vw); --radius:12px;
  --mono:'Satoshi',system-ui,-apple-system,"Segoe UI",sans-serif;
}
*{box-sizing:border-box}
html,body{height:100%;margin:0}
body{font-family:var(--mono);background:var(--bg);color:var(--text);display:flex;flex-direction:column;overflow:hidden;-webkit-font-smoothing:subpixel-antialiased}
.topbar{display:flex;align-items:center;gap:16px;padding:12px 18px;border-bottom:1px solid var(--border);flex:0 0 auto}
.topbar h1{font-size:14px;font-weight:600;letter-spacing:.02em;margin:0}
.topbar .sub{color:var(--muted);font-size:11px}
.graph-toolbar-row{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
button{font-family:inherit;font-size:11px;color:var(--text-muted);background:var(--panel2);
  border:1px solid var(--border);border-radius:8px;padding:5px 10px;cursor:pointer}
button:hover{border-color:var(--accent);color:var(--accent)}
button[aria-pressed="true"]{border-color:var(--accent);color:var(--accent)}
main{flex:1 1 auto;display:flex;min-height:0}
#graph{flex:1 1 auto;position:relative;overflow:hidden;background:
  radial-gradient(1200px 600px at 30% -10%, rgba(152,232,193,.06), transparent 60%)}
svg{width:100%;height:100%;display:block}
.links line{stroke:var(--border-subtle);stroke-width:1}
.node circle{stroke-width:1.5;cursor:pointer}
.node.ok circle{fill:rgba(152,232,193,.14);stroke:var(--ok)}
.node.warn circle{fill:rgba(255,184,108,.16);stroke:var(--warn)}
.node.bad circle{fill:rgba(254,125,125,.18);stroke:var(--bad)}
.node text{fill:var(--text-muted);font-size:10px;pointer-events:none}
.node.sel circle{stroke-width:3}
aside{flex:0 0 var(--side-w);border-left:1px solid var(--border);background:var(--panel);
  overflow:auto;padding:14px 16px}
aside h2{font-size:12px;margin:0 0 4px;letter-spacing:.02em}
aside .side-count{color:var(--muted);font-size:11px;margin-bottom:12px}
table{width:100%;border-collapse:collapse;font-size:11px}
th,td{text-align:left;padding:5px 6px;border-bottom:1px solid var(--border-subtle);vertical-align:top}
th{color:var(--muted);font-weight:500}
td.mono,code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:10.5px}
.table-scroll{max-height:230px;overflow:auto;border:1px solid var(--border-subtle);border-radius:8px}
.pill{display:inline-block;padding:1px 7px;border-radius:999px;font-size:10px;border:1px solid var(--border)}
.pill.ok{color:var(--ok);border-color:rgba(152,232,193,.4)}
.pill.warn{color:var(--warn);border-color:rgba(255,184,108,.4)}
.pill.bad{color:var(--bad);border-color:rgba(254,125,125,.45)}
.muted{color:var(--muted)}
"""

_JS = r"""
(function () {
  const DATA = JSON.parse(document.getElementById('DATA').textContent);
  const repos = DATA.repos || [];
  const graph = DATA.graph || { nodes: [], edges: [] };

  const worstRank = { ok: 0, warn: 1, bad: 2 };
  const order = repos.slice().sort(function (a, b) {
    const d = (worstRank[b.worst] || 0) - (worstRank[a.worst] || 0);
    if (d) return d;
    const e = (a.crates.length) - (b.crates.length);
    return e || a.repo.localeCompare(b.repo);
  });

  // BFS layers from the most severe repo: failures land in the first column.
  const focus = (order[0] && order[0].repo) || null;
  const adj = {};
  graph.edges.forEach(function (e) {
    (adj[e.from] = adj[e.from] || []).push(e.to);
    (adj[e.to] = adj[e.to] || []).push(e.from);
  });
  const layer = {};
  if (focus) {
    layer[focus] = 0;
    let q = [focus];
    while (q.length) {
      const nq = [];
      q.forEach(function (n) {
        (adj[n] || []).forEach(function (m) {
          if (layer[m] === undefined) { layer[m] = layer[n] + 1; nq.push(m); }
        });
      });
      q = nq;
    }
  }
  const maxLayer = Object.keys(layer).reduce(function (m, k) { return Math.max(m, layer[k]); }, 0);
  repos.forEach(function (r) { if (layer[r.repo] === undefined) layer[r.repo] = maxLayer + 1; });

  const byLayer = {};
  repos.forEach(function (r) {
    (byLayer[layer[r.repo]] = byLayer[layer[r.repo]] || []).push(r);
  });

  const svg = document.getElementById('svg');
  const side = document.getElementById('side');
  let pos = {};

  function layout() {
    const w = svg.clientWidth || 900, h = svg.clientHeight || 600;
    const keys = Object.keys(byLayer).map(Number).sort(function (a, b) { return a - b; });
    const colW = Math.max(150, w / Math.max(3, keys.length + 1));
    keys.forEach(function (L) {
      const col = byLayer[L].slice().sort(function (a, b) {
        return (worstRank[b.worst] || 0) - (worstRank[a.worst] || 0) || a.repo.localeCompare(b.repo);
      });
      const colH = h - 60;
      const step = col.length > 1 ? colH / (col.length - 1) : 0;
      col.forEach(function (r, i) {
        const y = col.length > 1 ? 30 + i * step : h / 2;
        pos[r.repo] = { x: (L + 0.5) * colW, y: y, r: 5 + Math.min(11, Math.sqrt(r.crates.length) * 2.4) };
      });
    });
    draw();
  }

  function draw() {
    const parts = ['<g class="links">'];
    graph.edges.forEach(function (e) {
      const a = pos[e.from], b = pos[e.to];
      if (!a || !b) return;
      parts.push('<line x1="' + a.x + '" y1="' + a.y + '" x2="' + b.x + '" y2="' + b.y + '"/>');
    });
    parts.push('</g><g class="nodes">');
    repos.forEach(function (r) {
      const p = pos[r.repo];
      if (!p) return;
      parts.push('<g class="node ' + esc(r.worst) + '" data-repo="' + esc(r.repo) + '">' +
        '<circle cx="' + p.x + '" cy="' + p.y + '" r="' + p.r + '"/>' +
        '<text x="' + (p.x + p.r + 5) + '" y="' + (p.y + 3) + '">' + esc(r.repo) + '</text></g>');
    });
    parts.push('</g>');
    svg.innerHTML = parts.join('');
    Array.prototype.forEach.call(svg.querySelectorAll('.node'), function (g) {
      g.addEventListener('click', function () { select(g.getAttribute('data-repo')); });
    });
  }

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  function select(repo) {
    Array.prototype.forEach.call(svg.querySelectorAll('.node'), function (g) {
      g.classList.toggle('sel', g.getAttribute('data-repo') === repo);
    });
    render(repo);
  }

  function render(repo) {
    const r = repos.find(function (x) { return x.repo === repo; });
    if (!r) { side.innerHTML = ''; return; }
    const s = DATA.summary;
    const h = [];
    h.push('<h2>' + esc(r.repo) + '</h2>');
    h.push('<div class="side-count">' + r.crates.length + ' crates · ' +
      '<span class="pill ' + esc(r.worst) + '">' + esc(r.worst) + '</span></div>');
    if (r.url) h.push('<div class="muted mono" style="word-break:break-all;margin-bottom:10px">' + esc(r.url) + '</div>');
    h.push('<table><tr><th>mode</th><th>branch</th></tr>');
    r.branches.forEach(function (b) { h.push('<tr><td>' + esc(b.mode) + '</td><td class="mono">' + esc(b.branch) + '</td></tr>'); });
    h.push('</table>');
    h.push('<h2 style="margin-top:14px">git</h2>');
    const g = r.git_detail;
    if (g) {
      h.push('<div class="side-count"><span class="pill ' + (g.state === 'clean' ? 'ok' : 'warn') + '">' +
        esc(g.state) + '</span> ' + esc(g.branch || 'detached') + '</div>');
      h.push('<table>' +
        '<tr><th>staged</th><td>' + g.staged + '</td></tr>' +
        '<tr><th>unstaged</th><td>' + g.unstaged + '</td></tr>' +
        '<tr><th>untracked</th><td>' + g.untracked + '</td></tr>' +
        '<tr><th>ahead</th><td>' + g.ahead + (g.upstream ? ' → ' + esc(g.upstream) : '') + '</td></tr>' +
        '</table>');
    } else { h.push('<div class="muted">not a monorepo checkout</div>'); }
    h.push('<h2 style="margin-top:14px">crates</h2><div class="table-scroll"><table><tr><th>key</th><th>package</th></tr>');
    r.crates.forEach(function (c) { h.push('<tr><td class="mono">' + esc(c.key) + '</td><td class="mono">' + esc(c.pkg) + '</td></tr>'); });
    h.push('</table></div>');
    side.innerHTML = h.join('');
  }

  function renderAll() {
    const s = DATA.summary;
    const h = [];
    h.push('<h2>summary</h2><div class="side-count">' + s.repos + ' repos · ' + s.crates + ' crates</div>');
    h.push('<table>' +
      '<tr><th>missing branches</th><td><span class="pill ' + (s.missing ? 'bad' : 'ok') + '">' + s.missing + '</span></td></tr>' +
      '<tr><th>unconfirmed</th><td>' + s.unconfirmed + '</td></tr>' +
      '<tr><th>dirty repos</th><td>' + s.dirty + '</td></tr>' +
      '</table>');
    h.push('<h2 style="margin-top:14px">repositories</h2><div class="table-scroll"><table><tr><th>repo</th><th>crates</th><th>state</th><th>git</th></tr>');
    order.forEach(function (r) {
      h.push('<tr><td class="mono"><a href="#" data-repo="' + esc(r.repo) + '" style="color:var(--accent);text-decoration:none">' +
        esc(r.repo) + '</a></td><td>' + r.crates.length + '</td>' +
        '<td><span class="pill ' + esc(r.worst) + '">' + esc(r.worst) + '</span></td>' +
        '<td class="muted">' + esc(r.git) + '</td></tr>');
    });
    h.push('</table></div>');
    side.innerHTML = h.join('');
    Array.prototype.forEach.call(side.querySelectorAll('a[data-repo]'), function (a) {
      a.addEventListener('click', function (ev) { ev.preventDefault(); select(a.getAttribute('data-repo')); });
    });
  }

  Array.prototype.forEach.call(document.querySelectorAll('button[data-focus]'), function (b) {
    b.addEventListener('click', function () { renderAll(); });
  });
  window.addEventListener('resize', layout);
  layout();
  renderAll();
})();
"""


def render_html(payload: dict) -> str:
    data = json.dumps(payload, sort_keys=True).replace("</", "<\\/")
    s = payload["summary"]
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Dependency status · gen-tools</title>
<style>{_CSS}</style>
</head>
<body>
<div class="topbar">
  <h1>Dependency status</h1>
  <span class="sub">{html.escape(str(s['repos']))} repos · {html.escape(str(s['crates']))} crates ·
    <span class="pill bad">{html.escape(str(s['missing']))} missing</span>
    <span class="pill warn">{html.escape(str(s['unconfirmed']))} unconfirmed</span>
    <span class="pill warn">{html.escape(str(s['dirty']))} dirty</span>
  </span>
  <div class="graph-toolbar-row">
    <button data-focus="all">All repositories</button>
  </div>
  <span class="sub" style="margin-left:auto">{html.escape(payload['generated_at'])}</span>
</div>
<main>
  <div id="graph"><svg id="svg"></svg></div>
  <aside id="side"></aside>
</main>
<script id="DATA" type="application/json">{data}</script>
<script>{_JS}</script>
</body>
</html>
"""


def write_report(output: Path = None, include_git: bool = True, include_graph: bool = True) -> Path:
    path = Path(output) if output else REPO_ROOT / "_devops" / "dep-status.html"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_html(build_payload(include_git=include_git, include_graph=include_graph)))
    return path


def serve(output: Path = None, port: int = 8787, host: str = "127.0.0.1") -> None:
    """Serve the report on loopback only.

    Loopback is not a default, it is the design: this page summarizes dependency
    state for a machine that holds unpublished work. Binding a routable
    interface would publish that inventory.
    """
    import http.server
    import socketserver

    if host not in ("127.0.0.1", "localhost", "::1"):
        raise ValueError(f"refusing to bind {host!r}: the report is loopback-only by design")

    path = write_report(output)
    page = path.read_text()

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            if self.path not in ("/", "/index.html"):
                self.send_error(404)
                return
            body = page.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    with socketserver.TCPServer((host, port), Handler) as httpd:
        print(f"dependency status: http://{host}:{port}/  (ctrl-c to stop)")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nstopped")
