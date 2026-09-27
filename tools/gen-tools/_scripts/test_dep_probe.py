#!/usr/bin/env python3
"""Tests for the read-only probes in dep_probe.

No network and no cargo. Branch checks are exercised by replacing
`remote_branches`, so a missing branch is detected from a fixture rather than
from a live remote. Run:

    python3 _scripts/test_dep_probe.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

_SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))

import dep_probe  # noqa: E402
from dep_probe import (  # noqa: E402
    RepoState,
    check_branches_on_remote,
    find_git_repos,
    preflight_for_write,
    probe_repo,
    resolve_all_git_sources,
    summarize_branches,
    summarize_repos,
)

COSMWASM = "https://github.com/permissionlessweb/cosmwasm.git"
COSMOS_RUST = "https://github.com/permissionlessweb/cosmos-rust.git"


class ResolveAllGitSources(unittest.TestCase):
    def test_groups_keys_by_url_and_branch(self):
        matrix = {
            "cosmwasm-std": {"git": {"url": COSMWASM, "branch": "mvp"}},
            "cosmwasm-schema": {"git": {"url": COSMWASM, "branch": "mvp"}},
            "cosmos-sdk-proto": {"git": {"url": COSMOS_RUST, "branch": "v3.1.0-zk.0"}},
        }
        pairs = resolve_all_git_sources(matrix, {}, "git")
        self.assertEqual(sorted(pairs[(COSMWASM, "mvp")]), ["cosmwasm-schema", "cosmwasm-std"])
        self.assertEqual(pairs[(COSMOS_RUST, "v3.1.0-zk.0")], ["cosmos-sdk-proto"])

    def test_branch_falls_back_to_defaults(self):
        matrix = {"cosmos-sdk-proto": {"git": {"url": COSMOS_RUST}}}
        pairs = resolve_all_git_sources(matrix, {"git_branch": "v3.1.0-zk.0"}, "git")
        self.assertIn((COSMOS_RUST, "v3.1.0-zk.0"), pairs)

    def test_repo_only_entries_are_included(self):
        # repo_only entries are never written to Cargo files but still declare a
        # branch that push/bootstrap flows depend on, so they must be checked.
        matrix = {
            "some-repo": {
                "repo_only": True,
                "git": {"url": COSMWASM, "branch": "mvp"},
            }
        }
        pairs = resolve_all_git_sources(matrix, {}, "git")
        self.assertIn((COSMWASM, "mvp"), pairs)

    def test_all_mode_emits_both_git_and_zk_git(self):
        matrix = {
            "p": {
                "git": {"url": COSMWASM, "branch": "mvp"},
                "zk_git": {"url": COSMWASM, "branch": "zk-mvp"},
            }
        }
        pairs = resolve_all_git_sources(matrix, {}, "all")
        self.assertEqual(set(pairs), {(COSMWASM, "mvp"), (COSMWASM, "zk-mvp")})


class CheckBranchesOnRemote(unittest.TestCase):
    """The defect this whole probe exists to catch: a matrix naming a branch the
    remote does not publish. cosmwasm has mvp but never had v3.1.0-zk.0."""

    def setUp(self):
        self._real = dep_probe.remote_branches
        self._cache = dep_probe.BRANCH_CACHE
        tmp = tempfile.NamedTemporaryFile(suffix=".json", delete=False)
        tmp.close()
        self._cache_path = Path(tmp.name)
        dep_probe.BRANCH_CACHE = self._cache_path

        def fake(url, refresh=False):
            if url == COSMWASM:
                return {"main", "mvp", "feat/wasm-call-depth"}, "ok"
            if url == COSMOS_RUST:
                return {"main", "v3.1.0-zk.0", "zk-mvp"}, "ok"
            return set(), "unreachable"

        dep_probe.remote_branches = fake

    def tearDown(self):
        dep_probe.remote_branches = self._real
        dep_probe.BRANCH_CACHE = self._cache
        self._cache_path.unlink(missing_ok=True)

    def test_correct_branch_is_reported_ok(self):
        matrix = {"cosmwasm-std": {"git": {"url": COSMWASM, "branch": "mvp"}}}
        findings = check_branches_on_remote(matrix, {}, "git", refresh=True)
        self.assertEqual([f.state for f in findings], ["present"])
        self.assertTrue(findings[0].ok)

    def test_unpublished_branch_is_reported_missing(self):
        matrix = {"cosmwasm-std": {"git": {"url": COSMWASM, "branch": "v3.1.0-zk.0"}}}
        findings = check_branches_on_remote(matrix, {}, "git", refresh=True)
        self.assertEqual([f.state for f in findings], ["missing"])
        self.assertFalse(findings[0].ok)
        # the operator needs to know what IS there to pick a real branch
        self.assertIn("mvp", findings[0].detail)

    def test_unreachable_is_distinct_from_missing(self):
        matrix = {"x": {"git": {"url": "https://example.invalid/x.git", "branch": "main"}}}
        findings = check_branches_on_remote(matrix, {}, "git", refresh=True)
        self.assertEqual([f.state for f in findings], ["unreachable"])
        self.assertFalse(findings[0].ok)

    def test_missing_lists_every_affected_matrix_key(self):
        matrix = {
            "cosmwasm-std": {"git": {"url": COSMWASM, "branch": "v3.1.0-zk.0"}},
            "cosmwasm-schema": {"git": {"url": COSMWASM, "branch": "v3.1.0-zk.0"}},
        }
        findings = check_branches_on_remote(matrix, {}, "git", refresh=True)
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].keys, ["cosmwasm-schema", "cosmwasm-std"])

    def test_summarize_counts_states(self):
        matrix = {
            "a": {"git": {"url": COSMWASM, "branch": "mvp"}},
            "b": {"git": {"url": COSMWASM, "branch": "v3.1.0-zk.0"}},
        }
        counts = summarize_branches(check_branches_on_remote(matrix, {}, "git", refresh=True))
        self.assertEqual(counts["present"], 1)
        self.assertEqual(counts["missing"], 1)

    def test_cache_avoids_second_remote_call(self):
        calls = []

        def counting(url, refresh=False):
            calls.append(url)
            return {"mvp"}, "ok"

        dep_probe.remote_branches = counting
        matrix = {"cosmwasm-std": {"git": {"url": COSMWASM, "branch": "mvp"}}}
        check_branches_on_remote(matrix, {}, "git", refresh=True)
        check_branches_on_remote(matrix, {}, "git")
        self.assertEqual(calls.count(COSMWASM), 1)
        check_branches_on_remote(matrix, {}, "git", refresh=True)
        self.assertEqual(calls.count(COSMWASM), 2)


class WorktreeParsing(unittest.TestCase):
    def test_parses_records_and_prunable_flag(self):
        text = (
            "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n"
            "worktree /gone\nHEAD def\nbranch refs/heads/zk-mvp\n"
            "prunable gitdir file points to non-existent location\n\n"
            "worktree /det\nHEAD 123\n"
        )
        parsed = dep_probe._parse_worktrees(text)
        self.assertEqual(len(parsed), 3)
        self.assertEqual(parsed[0].branch, "main")
        self.assertEqual(parsed[0].prunable, "")
        self.assertEqual(parsed[1].prunable, "gitdir file points to non-existent location")
        self.assertEqual(parsed[2].branch, "")

    def test_empty_input_yields_nothing(self):
        self.assertEqual(dep_probe._parse_worktrees(""), [])
        self.assertEqual(dep_probe._parse_worktrees("\n\n"), [])


class Preflight(unittest.TestCase):
    def _state(self, rel, **kw):
        s = RepoState(rel)
        s.staged = kw.get("staged", 0)
        s.unstaged = kw.get("unstaged", 0)
        s.untracked = kw.get("untracked", 0)
        s.ahead = kw.get("ahead", 0)
        s.upstream = kw.get("upstream", "origin/main")
        s.worktrees = kw.get("worktrees", [])
        return s

    def test_clean_repo_does_not_block(self):
        self.assertEqual(preflight_for_write([self._state("a")]), [])

    def test_dirty_repo_blocks_with_counts(self):
        blockers = preflight_for_write([self._state("a", unstaged=3, untracked=1)])
        self.assertEqual(len(blockers), 1)
        self.assertIn("unstaged=3", blockers[0])
        self.assertIn("untracked=1", blockers[0])

    def test_staged_alone_blocks(self):
        self.assertTrue(preflight_for_write([self._state("a", staged=1)]))

    def test_outgoing_commits_block(self):
        blockers = preflight_for_write([self._state("a", ahead=2)])
        self.assertIn("2 outgoing commit(s)", blockers[0])

    def test_prunable_worktree_blocks(self):
        wt = dep_probe.WorktreeInfo("/gone", "zk-mvp", "missing")
        blockers = preflight_for_write([self._state("a", worktrees=[wt])])
        self.assertIn("prunable", blockers[0])

    def test_targets_filter_limits_scope(self):
        states = [self._state("a", unstaged=1), self._state("b", unstaged=1)]
        blockers = preflight_for_write(states, targets=["a"])
        self.assertEqual(len(blockers), 1)
        self.assertTrue(blockers[0].startswith("a:"))

    def test_detached_head_alone_does_not_block(self):
        s = self._state("a")
        s.detached = True
        self.assertEqual(preflight_for_write([s]), [])


class RealRepoProbe(unittest.TestCase):
    """Exercise probe_repo against a real throwaway git repo."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        self._git("init", "-q", "-b", "main")
        self._git("config", "user.email", "t@example.invalid")
        self._git("config", "user.name", "t")
        (self.repo / "a.txt").write_text("one\n")
        self._git("add", "a.txt")
        self._git("commit", "-q", "-m", "first")
        self._git("commit", "-q", "--allow-empty", "-m", "second")

    def tearDown(self):
        self.tmp.cleanup()

    def _git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, capture_output=True, text=True, check=True
        )

    def test_clean_repo_reports_clean(self):
        state = probe_repo(self.repo, self.repo.parent)
        self.assertFalse(state.dirty)
        self.assertEqual(state.branch, "main")
        self.assertEqual(state.staged, 0)
        self.assertEqual(state.unstaged, 0)
        self.assertEqual(state.ahead, 0)

    def test_unstaged_modification_detected(self):
        (self.repo / "a.txt").write_text("two\n")
        state = probe_repo(self.repo, self.repo.parent)
        self.assertTrue(state.dirty)
        self.assertEqual(state.unstaged, 1)

    def test_untracked_file_detected(self):
        (self.repo / "b.txt").write_text("new\n")
        state = probe_repo(self.repo, self.repo.parent)
        self.assertEqual(state.untracked, 1)
        self.assertTrue(state.dirty)

    def test_staged_file_detected(self):
        (self.repo / "a.txt").write_text("two\n")
        self._git("add", "a.txt")
        state = probe_repo(self.repo, self.repo.parent)
        self.assertEqual(state.staged, 1)

    def test_detached_head_detected(self):
        sha = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=self.repo, capture_output=True, text=True
        ).stdout.strip()
        self._git("checkout", "-q", sha)
        state = probe_repo(self.repo, self.repo.parent)
        self.assertTrue(state.detached)

    def test_summarize_counts(self):
        state = probe_repo(self.repo, self.repo.parent)
        summary = summarize_repos([state])
        self.assertEqual(summary["repos"], 1)
        self.assertEqual(summary["dirty"], 0)
        self.assertEqual(summary["errored"], 0)

    def test_find_git_repos_does_not_descend_into_a_repo(self):
        # A repo is a leaf. Descending would walk into vendored dependencies and
        # build trees, which is both slow and wrong: a vendored crate is not a
        # monorepo member whose work could be destroyed.
        nested = self.repo / "inner"
        nested.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=nested, check=True,
                       capture_output=True)
        found = [str(p.relative_to(self.repo.parent)) for p in find_git_repos(self.repo.parent)]
        self.assertIn("repo", found)
        self.assertNotIn("repo/inner", found)

    def test_find_git_repos_finds_repo_under_plain_directory(self):
        # The _worktrees shape: a linked worktree living under a directory that
        # is not itself a repository must still be discovered.
        plain = Path(self.tmp.name) / "_worktrees"
        plain.mkdir()
        linked = plain / "some-worktree"
        linked.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=linked, check=True,
                       capture_output=True)
        found = [str(p.relative_to(Path(self.tmp.name))) for p in find_git_repos(self.tmp.name)]
        self.assertIn("_worktrees/some-worktree", found)

    def test_find_git_repos_skips_target_and_node_modules(self):
        (self.repo / "target").mkdir()
        (self.repo / "node_modules").mkdir()
        for name in ("target", "node_modules"):
            subprocess.run(["git", "init", "-q"], cwd=self.repo / name, check=True,
                           capture_output=True)
        found = [str(p.relative_to(self.repo.parent)) for p in find_git_repos(self.repo.parent)]
        self.assertNotIn("repo/target", found)
        self.assertNotIn("repo/node_modules", found)


if __name__ == "__main__":
    unittest.main(verbosity=2)
