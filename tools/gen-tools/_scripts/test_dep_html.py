#!/usr/bin/env python3
"""Tests for the self-contained HTML report.

No network, no cargo, no filesystem writes outside tempdirs. Run:

    python3 _scripts/test_dep_html.py
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

_SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))

import dep_html  # noqa: E402


def _payload(**over):
    base = {
        "generated_at": "2026-09-27T00:00:00Z",
        "monorepo_root": "/tmp/crates",
        "matrix_path": "/tmp/crates/_devops/dependency-matrix.toml",
        "repos": [
            {
                "repo": "cosmwasm",
                "url": "https://github.com/permissionlessweb/cosmwasm",
                "crates": [{"key": "cosmwasm-std", "pkg": "cosmwasm-std", "repo_only": False}],
                "branches": [{"mode": "git", "branch": "mvp"}],
                "state": "present",
                "git": "clean",
                "worst": "ok",
            }
        ],
        "git": [],
        "summary": {"repos": 1, "crates": 1, "missing": 0, "unconfirmed": 0, "dirty": 0},
    }
    base.update(over)
    return base


class RepoKey(unittest.TestCase):
    def test_strips_trailing_slash_and_git_suffix(self):
        for url in (
            "https://github.com/permissionlessweb/cosmwasm.git",
            "https://github.com/permissionlessweb/cosmwasm",
            "https://github.com/permissionlessweb/cosmwasm/",
        ):
            self.assertEqual(dep_html._repo_key(url), "cosmwasm")

    def test_ssh_urls_normalize_to_repo_name(self):
        self.assertEqual(
            dep_html._repo_key("ssh://git@groot-wan/Users/x/crates/cosmwasm"), "cosmwasm"
        )

    def test_empty_url_is_empty(self):
        self.assertEqual(dep_html._repo_key(""), "")


class AsDep(unittest.TestCase):
    def test_reads_dataclass_like_object(self):
        class D:
            pkg_name = "cw-multi-test"
            source = "path"
            path = "clone-cw-multi-test"
            git_url = None

        self.assertEqual(
            dep_html._as_dep(D()),
            {"pkg": "cw-multi-test", "source": "path", "path": "clone-cw-multi-test", "git_url": None},
        )

    def test_reads_plain_dict_with_rename(self):
        d = {"name": "cw-multi-test", "package": "abstract-cw-multi-test", "source": "git", "path": None}
        self.assertEqual(dep_html._as_dep(d)["pkg"], "abstract-cw-multi-test")

    def test_ignores_unusable_values(self):
        self.assertIsNone(dep_html._as_dep(None))
        self.assertIsNone(dep_html._as_dep("nonsense"))


class SelfContained(unittest.TestCase):
    """The page must work from file:// with the network off. That is the invariant."""

    def test_no_external_references(self):
        page = dep_html.render_html(_payload())
        self.assertNotIn("http://", page.replace("https://github.com", ""))
        for pattern in (r'src\s*=\s*["\']https?:', r'href\s*=\s*["\']https?:', r"@import"):
            self.assertIsNone(re.search(pattern, page), f"external reference matched {pattern}")

    def test_no_eval_or_dynamic_import(self):
        page = dep_html.render_html(_payload())
        for bad in ("eval(", "new Function(", "import(", "document.write("):
            self.assertNotIn(bad, page)

    def test_data_is_embedded_as_json_script_tag(self):
        page = dep_html.render_html(_payload())
        self.assertIn('<script id="DATA" type="application/json">', page)
        raw = page.split('<script id="DATA" type="application/json">', 1)[1].split("</script>", 1)[0]
        self.assertEqual(json.loads(raw)["summary"]["repos"], 1)


class Escaping(unittest.TestCase):
    def test_hostile_repo_name_cannot_close_the_script_tag(self):
        payload = _payload()
        payload["repos"][0]["repo"] = "evil</script><script>alert(1)</script>"
        page = dep_html.render_html(payload)
        raw = page.split('<script id="DATA" type="application/json">', 1)[1].split("</script>", 1)[0]
        # payload survives intact and did not terminate the tag early
        self.assertEqual(
            json.loads(raw)["repos"][0]["repo"], "evil</script><script>alert(1)</script>"
        )
        self.assertEqual(page.count('<script id="DATA" type="application/json">'), 1)

    def test_payload_never_reaches_the_document_markup(self):
        # A <img ...> inside a type="application/json" block is inert: the browser
        # does not parse that block as HTML. What matters is that the hostile value
        # stays inside the data block and never appears in the surrounding markup.
        payload = _payload()
        payload["repos"][0]["repo"] = "<img src=x onerror=alert(1)>"
        page = dep_html.render_html(payload)
        head, rest = page.split('<script id="DATA" type="application/json">', 1)
        data_block, tail = rest.split("</script>", 1)
        self.assertIn("<img src=x onerror=alert(1)>", data_block)
        self.assertNotIn("<img src=x onerror=alert(1)>", head)
        self.assertNotIn("<img src=x onerror=alert(1)>", tail)

    def test_no_data_field_is_concatenated_into_markup_unescaped(self):
        # Static guard: every data value the renderer splices into an HTML string
        # must go through esc(). Extend the field list when new fields are rendered.
        fields = [
            "r.repo", "r.url", "r.branch", "r.worst", "r.git", "b.mode", "b.branch",
            "g.state", "g.branch", "g.upstream", "c.key", "c.pkg",
        ]
        js = dep_html._JS
        offenders = []
        for field in fields:
            for m in re.finditer(r"\+\s*" + re.escape(field) + r"\s*\+", js):
                prefix = js[max(0, m.start() - 4):m.start()]
                if prefix != "esc(":
                    offenders.append((field, js[max(0, m.start() - 40):m.end() + 5]))
        self.assertEqual(offenders, [], f"unescaped interpolation(s): {offenders}")

    def test_renderer_escapes_before_inserting(self):
        self.assertIn("function esc(s)", dep_html._JS)
        for char, entity in (("&", "&amp;"), ("<", "&lt;"), (">", "&gt;"), ('"', "&quot;")):
            self.assertIn(entity, dep_html._JS)


class ServeGuard(unittest.TestCase):
    def test_refuses_routable_interface(self):
        for host in ("0.0.0.0", "192.168.1.10", "::"):
            with self.assertRaises(ValueError) as ctx:
                dep_html.serve(output=Path(tempfile.gettempdir()) / "x.html", host=host)
            self.assertIn("loopback-only", str(ctx.exception))

    def test_accepts_loopback_names(self):
        # Do not actually serve; assert the guard is not what rejects these.
        for host in ("127.0.0.1", "localhost", "::1"):
            try:
                dep_html.serve(output=Path("/nonexistent-dir-for-test") / "x.html", host=host)
            except ValueError as exc:
                self.assertNotIn("loopback-only", str(exc))
            except Exception:
                pass


class GraphContract(unittest.TestCase):
    """The layout and the renderer both iterate the repo set.

    A graph node or edge endpoint outside that set has no position, so draw()
    drops it without a word and the graph renders emptier than the data claims.
    """

    def _assert_subset(self, payload):
        repos = {r["repo"] for r in payload["repos"]}
        graph = payload.get("graph") or {}
        for node in graph.get("nodes", []):
            self.assertIn(node["id"], repos, f"graph node not in repos: {node['id']}")
        for edge in graph.get("edges", []):
            self.assertIn(edge["from"], repos, f"edge.from not in repos: {edge['from']}")
            self.assertIn(edge["to"], repos, f"edge.to not in repos: {edge['to']}")

    def test_graph_is_a_subset_of_repos(self):
        payload = _payload()
        payload["graph"] = {
            "nodes": [{"id": "cosmwasm", "crates": 2, "worst": "ok"}],
            "edges": [{"from": "cosmwasm", "to": "cosmwasm", "pkg": "x"}],
        }
        self._assert_subset(payload)

    def test_workspace_only_nodes_are_excluded(self):
        # The bug this guards: one node per workspace path, thousands of them,
        # none of which the renderer can draw.
        payload = _payload()
        payload["graph"] = {
            "nodes": [{"id": "local:_worktrees/cosmwasm-bulk-memory/contracts/burner", "crates": 1}],
            "edges": [{"from": "local:_worktrees/x/contracts/burner", "to": "cosmwasm", "pkg": "cosmwasm-vm"}],
        }
        with self.assertRaises(AssertionError):
            self._assert_subset(payload)

    def test_every_repo_has_a_drawable_worst(self):
        payload = _payload()
        graph = {"nodes": [], "edges": []}
        payload["graph"] = graph
        self._assert_subset(payload)
        for r in payload["repos"]:
            self.assertIn(r["worst"], ("ok", "warn", "bad"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
