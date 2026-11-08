"""Read-only probes for the dependency control plane.

Two probes, both strictly read-only. Neither writes a Cargo file, a matrix
file, or a git ref.

    check_branches_on_remote  Every git url/branch the matrix declares is
                              verified to exist on the remote. The matrix is
                              the declared source of truth for forked-crate
                              resolution, but nothing previously confirmed the
                              declared branch is actually published. A matrix
                              naming an unpublished branch silently produces
                              Cargo files that cannot resolve.

    probe_git_state           Per-repository working-tree state: staged,
                              unstaged, and untracked counts, commits ahead of
                              upstream, detached HEAD, and linked worktrees
                              (including prunable ones). Mode switching rewrites
                              Cargo files, so a writer must know which
                              repositories hold work it could destroy.

Both probes are the safety floor for the gated write paths. See
`preflight_for_write` for the guard that write paths are expected to call.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from dep_common import (
    DEVOPS_DIR,
    REPO_ROOT,
    is_repo_only,
    load_defaults,
    load_matrix,
    resolve_git_source,
)

BRANCH_CACHE = DEVOPS_DIR / ".branch-cache.json"
LS_REMOTE_TIMEOUT = 45
MAX_REPO_DEPTH = 3


# ── remote branch existence ───────────────────────────────────────────


class BranchFinding:
    """One (url, branch) pair declared by the matrix, checked against its remote."""

    def __init__(self, url: str, branch: str, keys: list, state: str, detail: str = ""):
        self.url = url
        self.branch = branch
        self.keys = sorted(keys)
        self.state = state
        self.detail = detail

    @property
    def ok(self) -> bool:
        return self.state == "present"

    def format(self) -> list:
        head = f"[{self.state.upper()}] {self.url} @ {self.branch}"
        if self.detail:
            head = f"{head} — {self.detail}"
        lines = [head]
        for key in self.keys:
            lines.append(f"    matrix key: {key}")
        return lines


def _run_git(args: list, cwd: Path = None, timeout: int = 30):
    """Run git with an argument list. Never a shell string. Returns (rc, stdout)."""
    try:
        proc = subprocess.run(
            ["git", *args],
            cwd=str(cwd) if cwd else None,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except (subprocess.TimeoutExpired, OSError) as exc:
        return 1, f"{type(exc).__name__}: {exc}"
    return proc.returncode, proc.stdout


def _load_cache() -> dict:
    if not BRANCH_CACHE.exists():
        return {}
    try:
        return json.loads(BRANCH_CACHE.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def _save_cache(cache: dict) -> None:
    try:
        BRANCH_CACHE.write_text(json.dumps(cache, indent=2, sort_keys=True) + "\n")
    except OSError:
        # A cache we cannot persist is a performance problem, not a correctness
        # problem. Never fail the probe over it.
        pass


def remote_branches(url: str, refresh: bool = False) -> tuple:
    """Return (set_of_branch_names, state) where state is one of:
    'ok', 'unreachable', 'auth', 'invalid-url'.
    """
    if url.startswith("ssh://") or "@" in url.split("//", 1)[0]:
        return set(), "unreachable"

    rc, out = _run_git(
        ["ls-remote", "--heads", url], timeout=LS_REMOTE_TIMEOUT
    )
    if rc != 0:
        lowered = out.lower()
        if "could not read" in lowered or "not found" in lowered or "does not appear" in lowered:
            return set(), "invalid-url"
        if "permission denied" in lowered or "authentication failed" in lowered or "terminal prompt" in lowered:
            return set(), "auth"
        return set(), "unreachable"

    names = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1].startswith("refs/heads/"):
            names.add(parts[1][len("refs/heads/"):])
    if not names:
        return names, "invalid-url"
    return names, "ok"


def resolve_all_git_sources(matrix: dict, defaults: dict, mode: str) -> dict:
    """Map (url, branch) -> [matrix keys] for every entry that resolves a git source.

    `mode` is 'git', 'zk_git', or 'all'. repo_only entries are included: they are
    never written to Cargo files (see is_repo_only) but they still declare a
    branch that push/bootstrap flows depend on, so a missing one is real drift.
    """
    modes = ("git", "zk_git") if mode == "all" else (mode,)
    grouped = {}
    for key, info in matrix.items():
        if not isinstance(info, dict):
            continue
        for m in modes:
            url, branch = resolve_git_source(info, m, defaults)
            if not url or not branch:
                continue
            # git and zk_git can resolve to the same pair; a key must be listed
            # once per pair, not once per mode that agreed on it.
            grouped.setdefault((url, branch), set()).add(key)
    return {pair: sorted(keys) for pair, keys in grouped.items()}


def check_branches_on_remote(
    matrix: dict, defaults: dict, mode: str = "all", refresh: bool = False
) -> list:
    """Verify every matrix (url, branch) against its remote. Returns BranchFinding list.

    One `ls-remote` per unique url, not per matrix entry. Results are cached by
    url; pass refresh=True to re-query. An unreachable remote is reported as its
    own state rather than being conflated with a missing branch, because "we
    could not ask" and "the branch is not there" demand different responses.
    """
    pairs = resolve_all_git_sources(matrix, defaults, mode)
    by_url = {}
    for url, branch in pairs:
        by_url.setdefault(url, set()).add(branch)

    cache = {} if refresh else _load_cache()
    cache_dirty = refresh
    heads = {}
    states = {}

    for url in sorted(by_url):
        cached = cache.get(url)
        if isinstance(cached, dict) and "heads" in cached and "state" in cached:
            heads[url] = set(cached["heads"])
            states[url] = cached["state"]
            continue
        found, state = remote_branches(url, refresh=refresh)
        heads[url] = found
        states[url] = state
        cache[url] = {"heads": sorted(found), "state": state}
        cache_dirty = True

    if cache_dirty:
        _save_cache(cache)

    findings = []
    for (url, branch), keys in sorted(pairs.items()):
        state = states[url]
        if state != "ok":
            detail = ""
            if state == "unreachable" and (url.startswith("ssh://") or "@" in url.split("//", 1)[0]):
                detail = "non-https url, not probed by design"
            findings.append(BranchFinding(url, branch, keys, state, detail))
        elif branch in heads[url]:
            findings.append(BranchFinding(url, branch, keys, "present"))
        else:
            avail = sorted(heads[url])
            shown = ", ".join(avail[:6]) + ("…" if len(avail) > 6 else "")
            findings.append(BranchFinding(url, branch, keys, "missing", f"available: {shown}"))
    return findings


# ── repository working-tree state ─────────────────────────────────────


class WorktreeInfo:
    def __init__(self, path: str, branch: str, prunable: str):
        self.path = path
        self.branch = branch
        self.prunable = prunable

    def format(self) -> list:
        flag = f" prunable({self.prunable})" if self.prunable else ""
        return [f"    worktree{flag}: {self.path} [{self.branch or 'detached'}]"]


class RepoState:
    """Working-tree and upstream state for one git repository."""

    def __init__(self, rel: str):
        self.rel = rel
        self.branch = ""
        self.detached = False
        self.staged = 0
        self.unstaged = 0
        self.untracked = 0
        self.ahead = 0
        self.upstream = ""
        self.worktrees = []
        self.error = ""

    @property
    def dirty(self) -> bool:
        return bool(self.staged or self.unstaged or self.untracked)

    @property
    def outgoing(self) -> bool:
        return self.ahead > 0

    @property
    def prunable(self) -> bool:
        return any(w.prunable for w in self.worktrees)

    def format(self) -> list:
        if self.error:
            return [f"  {self.rel}  [ERROR] {self.error}"]
        flags = []
        if self.dirty:
            flags.append(
                f"dirty staged={self.staged} unstaged={self.unstaged} untracked={self.untracked}"
            )
        if self.outgoing:
            flags.append(f"outgoing={self.ahead} -> {self.upstream}")
        if self.detached:
            flags.append("DETACHED")
        if self.prunable:
            flags.append("prunable-worktree")
        head = self.branch or "(no branch)"
        suffix = ("  " + "; ".join(flags)) if flags else "  clean"
        lines = [f"  {self.rel}  [{head}]{suffix}"]
        for wt in self.worktrees:
            lines.extend(wt.format())
        return lines


def find_git_repos(root: Path = None, max_depth: int = MAX_REPO_DEPTH) -> list:
    """Every git repository at or under root, as monorepo-relative paths.

    Handles both standalone repos (.git directory) and submodules (.git file).
    Skips anything inside target/ and node_modules/.

    A directory that is itself a repository is a leaf: the walk does not descend
    into it. Vendored dependencies and build trees live there, and a vendored
    crate is not a monorepo member whose work a write path could destroy.
    """
    base = Path(root) if root else Path(REPO_ROOT)
    found = []
    base_depth = len(base.parts)

    def walk(directory: Path, depth: int):
        if depth > max_depth:
            return
        try:
            entries = list(directory.iterdir())
        except OSError:
            return
        for entry in entries:
            if not entry.is_dir() or entry.is_symlink():
                continue
            if entry.name in ("target", "node_modules", ".git"):
                continue
            if (entry / ".git").exists():
                found.append(entry)
                continue
            walk(entry, depth + 1)

    walk(base, 0)
    return sorted(found, key=lambda p: str(p.relative_to(base)))


def _parse_worktrees(text: str) -> list:
    out = []
    for block in text.split("\n\n"):
        if not block.strip():
            continue
        path, branch, prunable = "", "", ""
        for line in block.splitlines():
            if line.startswith("worktree "):
                path = line[len("worktree "):]
            elif line.startswith("branch "):
                branch = line[len("branch "):].replace("refs/heads/", "")
            elif line.startswith("prunable"):
                prunable = line[len("prunable"):].strip() or "yes"
        if path:
            out.append(WorktreeInfo(path, branch, prunable))
    return out


def probe_repo(path: Path, root: Path = None) -> RepoState:
    """Read working-tree state for one repository. Read-only git plumbing only."""
    base = Path(root) if root else Path(REPO_ROOT)
    try:
        rel = str(path.relative_to(base))
    except ValueError:
        rel = str(path)
    state = RepoState(rel)

    rc, out = _run_git(["status", "--porcelain"], cwd=path)
    if rc != 0:
        state.error = out.strip() or f"git status exit {rc}"
        return state
    for line in out.splitlines():
        if len(line) < 3:
            continue
        xy, name = line[:2], line[3:]
        if xy == "??":
            state.untracked += 1
            continue
        if xy[0] not in (" ", "?"):
            state.staged += 1
        if xy[1] not in (" ", "?"):
            state.unstaged += 1

    rc, out = _run_git(["symbolic-ref", "--quiet", "--short", "HEAD"], cwd=path)
    if rc == 0 and out.strip():
        state.branch = out.strip()
    else:
        state.detached = True

    rc, out = _run_git(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], cwd=path)
    if rc == 0 and out.strip():
        state.upstream = out.strip()
        rc2, out2 = _run_git(["rev-list", "--count", f"{state.upstream}..HEAD"], cwd=path)
        if rc2 == 0 and out2.strip().isdigit():
            state.ahead = int(out2.strip())

    rc, out = _run_git(["worktree", "list", "--porcelain"], cwd=path)
    if rc == 0:
        state.worktrees = _parse_worktrees(out)

    return state


def probe_git_state(repos: list = None, root: Path = None) -> list:
    """RepoState for every repository, or for an explicit subset."""
    base = Path(root) if root else Path(REPO_ROOT)
    paths = [Path(p) if Path(p).is_absolute() else base / p for p in repos] if repos else find_git_repos(base)
    return [probe_repo(p, base) for p in paths]


# ── write-path preflight ──────────────────────────────────────────────


def preflight_for_write(states: list, targets: list = None) -> list:
    """Return human-readable blockers before a write path rewrites Cargo files.

    A write is only safe when every target repository is clean and has nothing
    outgoing. Callers must treat a non-empty result as a hard stop, not a
    warning: the whole purpose is to refuse destroying in-flight work.
    """
    blockers = []
    for state in states:
        if targets and state.rel not in targets:
            continue
        if state.error:
            blockers.append(f"{state.rel}: git state unreadable ({state.error})")
            continue
        if state.dirty:
            blockers.append(
                f"{state.rel}: dirty (staged={state.staged} unstaged={state.unstaged} "
                f"untracked={state.untracked})"
            )
        if state.outgoing:
            blockers.append(f"{state.rel}: {state.ahead} outgoing commit(s) to {state.upstream}")
        if state.prunable:
            blockers.append(f"{state.rel}: linked worktree is prunable")
    return blockers


def summarize_branches(findings: list) -> dict:
    counts = {}
    for f in findings:
        counts[f.state] = counts.get(f.state, 0) + 1
    return counts


def summarize_repos(states: list) -> dict:
    return {
        "repos": len(states),
        "dirty": sum(1 for s in states if s.dirty),
        "outgoing": sum(1 for s in states if s.outgoing),
        "detached": sum(1 for s in states if s.detached),
        "prunable": sum(1 for s in states if s.prunable),
        "errored": sum(1 for s in states if s.error),
    }
