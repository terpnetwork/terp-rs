"""
Shared library for dependency tooling in the Terp / Abstract monorepo.

Provides: config + monorepo root resolution, matrix loading, project discovery,
dep extraction, TOML writing helpers, strict matrix validation, heal, and
cargo-metadata verification used by dep-scrape, dep-overview, and dep-switch.
"""

import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Python 3.11+ required (for tomllib), or install tomli")
        sys.exit(1)


# ── Constants / config ───────────────────────────────────────────────────────

# Tooling home: gen-tools/ (parent of _scripts/)
TOOLING_ROOT = Path(__file__).resolve().parent.parent
DEVOPS_DIR = TOOLING_ROOT / "_devops"
CONFIG_PATH = DEVOPS_DIR / "dep-config.toml"
MIGRATION_PATH = DEVOPS_DIR / "migration-progress.yaml"

VALID_MODES = ("stable", "local", "git", "zk_local", "zk_git", "dev", "zk_dev")

# Composite mode decomposition: (ws_dep_mode, patch_mode, local_key)
DEV_MODE_MAP = {
    "dev":    ("git",    "local",    "local"),
    "zk_dev": ("zk_git", "zk_local", "zk_local"),
}

SKIP_DIRS = {
    ".git", "target", "_devops", "_scripts", ".claude", "node_modules",
    "__pycache__", "dist", "build",
}


def load_dep_config() -> dict:
    """Load _devops/dep-config.toml. Missing file → defaults."""
    defaults = {
        "paths": {
            "monorepo_root": "../../../..",
            "matrix": "dependency-matrix.toml",
            "mode_state": ".dep-mode",
        },
        "policy": {
            "forbid_absolute_paths": True,
            "strict_package_names": True,
            "verify_after_switch": False,
            "deprioritize_path_prefixes": {
                "prefixes": ["masp/vendor/", "build/"],
            },
        },
    }
    if not CONFIG_PATH.exists():
        return defaults
    with open(CONFIG_PATH, "rb") as f:
        data = tomllib.load(f)
    # shallow merge
    for section, vals in defaults.items():
        if section not in data:
            data[section] = vals
        elif isinstance(vals, dict):
            for k, v in vals.items():
                data[section].setdefault(k, v)
    return data


def _resolve_monorepo_root(cfg: dict) -> Path:
    """Resolve monorepo root from config, env, or fallbacks."""
    env = os.environ.get("TERP_DEP_ROOT") or os.environ.get("DEP_MONOREPO_ROOT")
    if env:
        return Path(env).expanduser().resolve()

    rel = cfg.get("paths", {}).get("monorepo_root", "../../../..")
    root = (DEVOPS_DIR / rel).resolve()
    if root.is_dir():
        return root

    # Fallbacks: walk up from tooling looking for cosmwasm/ or many Cargo workspaces
    for candidate in (
        TOOLING_ROOT.parent.parent.parent,  # crates/
        TOOLING_ROOT.parent.parent.parent.parent,  # terp-core/
    ):
        if (candidate / "cosmwasm").is_dir() or (candidate / "cw-plus").is_dir():
            return candidate.resolve()
    return TOOLING_ROOT


_DEP_CFG = load_dep_config()
REPO_ROOT = _resolve_monorepo_root(_DEP_CFG)
MATRIX_PATH = DEVOPS_DIR / _DEP_CFG.get("paths", {}).get("matrix", "dependency-matrix.toml")
MODE_STATE_PATH = DEVOPS_DIR / _DEP_CFG.get("paths", {}).get("mode_state", ".dep-mode")
DEP_POLICY = _DEP_CFG.get("policy", {})


def save_active_mode(mode: str):
    """Persist the current dep mode to _devops/.dep-mode."""
    MODE_STATE_PATH.write_text(mode.strip() + "\n")


def load_active_mode() -> Optional[str]:
    """Read the saved dep mode from _devops/.dep-mode, or None if not set."""
    if MODE_STATE_PATH.exists():
        mode = MODE_STATE_PATH.read_text().strip()
        if mode in VALID_MODES:
            return mode
    return None


def normalize_matrix_local(local: str) -> Optional[str]:
    """Normalize a matrix local path to monorepo-relative form, or None if empty/N/A."""
    if not local or local == "N/A":
        return None
    local = local.strip()
    if local.startswith("file://"):
        local = local[len("file://"):]
    # Absolute → try make relative to REPO_ROOT
    p = Path(local)
    if p.is_absolute():
        try:
            rel = p.resolve().relative_to(REPO_ROOT.resolve())
            local = str(rel)
        except ValueError:
            return local  # outside monorepo — caller treats as invalid
    local = local.lstrip("./")
    # If path points at Cargo.toml, use parent package dir
    if local.endswith("/Cargo.toml") or local.endswith("Cargo.toml"):
        local = str(Path(local).parent)
    # Collapse /./ segments
    local = str(Path(local))
    return local.replace("\\", "/")

# Default workspace roots for switch when no --workspace filter is given.
# Paths are relative to monorepo_root (terp-core/crates).
KNOWN_WORKSPACE_ROOTS = [
    "abstract/framework/Cargo.toml",
    "abstract/modules/Cargo.toml",
    "abstract/integrations/Cargo.toml",
    "abstract/interchain/Cargo.toml",
    "cw-orchestrator/Cargo.toml",
    "cw-plus/Cargo.toml",
    "cw-minus/Cargo.toml",
    "cosmwasm/Cargo.toml",
    "dao-contracts/Cargo.toml",
    "polytone/Cargo.toml",
    "cw-multi-test-fork/Cargo.toml",
    "clone-cw-multi-test/Cargo.toml",
]


# ── Data Classes ─────────────────────────────────────────────────────────────

@dataclass
class ProjectRoot:
    name: str
    rel_path: str
    cargo_toml: Path
    is_workspace: bool
    member_tomls: list  # list[Path]
    has_readme: bool

    @property
    def dir(self) -> Path:
        return self.cargo_toml.parent


@dataclass
class DepSpec:
    name: str
    package: Optional[str]  # renamed package name if any
    source: str  # "path" | "git" | "registry"
    version: Optional[str]
    path: Optional[str]
    git_url: Optional[str]
    git_branch: Optional[str]
    features: list  # list[str]
    default_features: bool
    dep_kind: str  # "normal" | "dev" | "build"
    workspace_inherited: bool
    declaring_file: str

    @property
    def pkg_name(self) -> str:
        return self.package or self.name


@dataclass
class PatchEntry:
    source_url: str  # e.g. "crates-io" or a git URL
    name: str
    package: Optional[str]
    path: Optional[str]
    git_url: Optional[str]
    git_branch: Optional[str]


@dataclass
class ProjectScanResult:
    project: ProjectRoot
    workspace_deps: dict  # name -> parsed dep info
    member_deps: list  # list[DepSpec]
    patches: list  # list[PatchEntry]
    path_dep_count: int = 0
    git_dep_count: int = 0
    registry_dep_count: int = 0
    patch_count: int = 0
    loc: int = 0
    diagnostics: list = field(default_factory=list)


# ── Matrix Functions ─────────────────────────────────────────────────────────

def load_matrix() -> dict:
    """Load the dependency matrix TOML. Returns the [crates] table.

    Normalizes legacy top-level url/branch fields into nested git tables.
    """
    with open(MATRIX_PATH, "rb") as f:
        crates = tomllib.load(f).get("crates", {})
    for _key, info in crates.items():
        if not isinstance(info, dict):
            continue
        # Legacy: url/branch at crate top-level instead of [crates.x.git]
        if info.get("url") and not isinstance(info.get("git"), dict):
            gi = {"url": info.pop("url")}
            if info.get("branch"):
                gi["branch"] = info.pop("branch")
            info["git"] = gi
        elif isinstance(info.get("git"), dict):
            info.pop("url", None)
            # keep top-level branch only if git.branch missing
            if info.get("branch") and not info["git"].get("branch"):
                info["git"]["branch"] = info.pop("branch")
            else:
                info.pop("branch", None)
    return crates


def load_defaults() -> dict:
    """Load [defaults] from the matrix. Returns {} if absent."""
    with open(MATRIX_PATH, "rb") as f:
        return tomllib.load(f).get("defaults", {})


def resolve_git_source(info: dict, mode: str, defaults: dict) -> tuple:
    """Resolve (url, branch) for a crate in git or zk_git mode.

    Resolution order for zk_git:
      url:    zk_git.url  → git.url
      branch: zk_git.branch → defaults.zk_git_branch → git.branch → defaults.git_branch

    Resolution order for git:
      url:    git.url (required)
      branch: git.branch → defaults.git_branch → "main"

    Returns (url, branch) or (None, None) if unresolvable.
    """
    gi = info.get("git", {})
    if not isinstance(gi, dict):
        gi = {}
    zg = info.get("zk_git", {})
    if not isinstance(zg, dict):
        zg = {}

    if mode == "git":
        url = gi.get("url")
        if not url or url == "N/A":
            return (None, None)
        branch = gi.get("branch") or defaults.get("git_branch", "main")
        return (url, branch)

    if mode == "zk_git":
        url = zg.get("url") or gi.get("url")
        if not url or url == "N/A":
            return (None, None)
        branch = (zg.get("branch")
                  or defaults.get("zk_git_branch")
                  or gi.get("branch")
                  or defaults.get("git_branch", "main"))
        return (url, branch)

    return (None, None)


def is_repo_only(info: dict) -> bool:
    """Check if a matrix entry is repo-level metadata (not a publishable crate).
    These entries exist for dep-graph push coverage but must NEVER be written
    to Cargo.toml files as dependency or patch entries."""
    return bool(info.get("repo_only"))


def pkg_name(key: str, info: dict) -> str:
    """Published crates.io package name for a matrix crate."""
    return info.get("package", key)


def matrix_pkg_names(crates: dict) -> set:
    """Set of all published package names + dep_aliases in the matrix."""
    names = set()
    for k, v in crates.items():
        if is_repo_only(v):
            continue
        names.add(pkg_name(k, v))
        for alias in v.get("dep_aliases", []):
            names.add(alias)
    return names


def pkg_to_matrix(crates: dict) -> dict:
    """Map published package name (and dep_aliases) -> (matrix_key, matrix_info)."""
    result = {}
    for k, v in crates.items():
        if is_repo_only(v):
            continue
        result[pkg_name(k, v)] = (k, v)
        for alias in v.get("dep_aliases", []):
            result[alias] = (k, v)
    return result


def matrix_key_to_pkg(crates: dict) -> dict:
    """Map matrix key -> published package name."""
    return {k: pkg_name(k, v) for k, v in crates.items()}


# ── Migration YAML Loader ───────────────────────────────────────────────────

def load_migration_progress() -> list:
    """Load migration-progress.yaml. Returns list of workspace dicts.
    Uses a simple line-based parser (no pyyaml dependency)."""
    if not MIGRATION_PATH.exists():
        return []

    try:
        import yaml
        with open(MIGRATION_PATH) as f:
            data = yaml.safe_load(f)
        return data.get("workspaces", []) if data else []
    except ImportError:
        pass

    # Simple fallback parser
    workspaces = []
    current = None
    in_blockers = False

    for line in MIGRATION_PATH.read_text().splitlines():
        stripped = line.strip()

        if stripped.startswith("#") or not stripped:
            in_blockers = False
            continue

        if stripped == "workspaces:":
            continue

        if stripped.startswith("- name:"):
            if current:
                workspaces.append(current)
            current = {"name": stripped.split(":", 1)[1].strip()}
            in_blockers = False
            continue

        if current and ":" in stripped and not stripped.startswith("-"):
            key, val = stripped.split(":", 1)
            key = key.strip()
            val = val.strip().strip('"').strip("'")
            if key == "blockers":
                in_blockers = True
                current["blockers"] = []
                # Check for inline empty list
                if val in ("[]", ""):
                    continue
            elif key in ("path", "cw_version", "cargo_check", "cargo_test", "notes"):
                current[key] = val
                in_blockers = False
            continue

        if in_blockers and current and stripped.startswith("- "):
            blocker = stripped[2:].strip().strip('"').strip("'")
            current.setdefault("blockers", []).append(blocker)
            continue

    if current:
        workspaces.append(current)

    return workspaces


# ── Project Discovery ────────────────────────────────────────────────────────

def discover_projects() -> list:
    """Discover all project roots under REPO_ROOT.
    Walks top-level dirs; when a dir has no root Cargo.toml, checks one
    level of subdirs (handles abstract/, ics23/rust, etc.).
    Also discovers excluded workspace members, standalone sub-projects
    (tools, examples, nested workspaces), and deeply nested projects."""
    import glob as glob_mod

    projects = []
    seen_paths = set()  # dedup by cargo_toml path

    def _add_project(proj):
        if proj and proj.cargo_toml not in seen_paths:
            seen_paths.add(proj.cargo_toml)
            projects.append(proj)

    # Top-level dirs
    for entry in sorted(REPO_ROOT.iterdir()):
        if not entry.is_dir():
            continue
        if entry.name in SKIP_DIRS or entry.name.startswith("."):
            continue

        cargo_toml = entry / "Cargo.toml"
        if cargo_toml.exists():
            proj = _build_project(cargo_toml)
            _add_project(proj)
            # Check excluded members — they have independent dep resolution
            if proj and proj.is_workspace:
                try:
                    with open(cargo_toml, "rb") as f:
                        data = tomllib.load(f)
                    for exc in data.get("workspace", {}).get("exclude", []):
                        for match in glob_mod.glob(str(entry / exc)):
                            match_path = Path(match)
                            exc_cargo = match_path / "Cargo.toml"
                            if exc_cargo.exists():
                                _add_project(_build_project(exc_cargo))
                            elif match_path.is_dir():
                                # Excluded dir has sub-projects (e.g. testing/contracts/*)
                                _discover_subdirs(match_path, projects, seen_paths, depth=2)
                except Exception:
                    pass
            # Also discover sub-projects (tools/, examples/, dependencies/)
            _discover_subdirs(entry, projects, seen_paths, depth=4)
            continue

        # No root Cargo.toml — recursively check subdirs up to 4 levels deep
        # (handles abstract/, ics23/rust, <project>/,
        #  terp-rs/crates/<project>/, etc.)
        _discover_subdirs(entry, projects, seen_paths, depth=4)

    return projects


def _discover_subdirs(directory: Path, projects: list, seen_paths: set, depth: int):
    """Recursively discover Cargo.toml projects in subdirectories."""
    if depth <= 0:
        return
    try:
        children = sorted(directory.iterdir())
    except PermissionError:
        return
    for sub in children:
        if not sub.is_dir() or sub.name in SKIP_DIRS or sub.name.startswith("."):
            continue
        sub_cargo = sub / "Cargo.toml"
        if sub_cargo.exists():
            proj = _build_project(sub_cargo)
            if proj and proj.cargo_toml not in seen_paths:
                seen_paths.add(proj.cargo_toml)
                projects.append(proj)
            # Also recurse to find sub-projects (codegen/, tools/, examples/)
            _discover_subdirs(sub, projects, seen_paths, depth - 1)
        else:
            # No Cargo.toml here — go deeper
            _discover_subdirs(sub, projects, seen_paths, depth - 1)


def _build_project(cargo_toml: Path) -> Optional[ProjectRoot]:
    """Build a ProjectRoot from a Cargo.toml path."""
    try:
        with open(cargo_toml, "rb") as f:
            data = tomllib.load(f)
    except Exception:
        return None

    proj_dir = cargo_toml.parent
    rel = os.path.relpath(proj_dir, REPO_ROOT)
    name = rel.replace("/", "-") if "/" in rel else proj_dir.name

    ws = data.get("workspace", {})
    is_workspace = "members" in ws
    member_tomls = []

    if is_workspace:
        members = ws.get("members", [])
        excludes = ws.get("exclude", [])
        member_tomls = resolve_workspace_members(proj_dir, members, excludes)

    has_readme = (proj_dir / "README.md").exists()

    return ProjectRoot(
        name=name,
        rel_path=rel,
        cargo_toml=cargo_toml,
        is_workspace=is_workspace,
        member_tomls=member_tomls,
        has_readme=has_readme,
    )


def resolve_workspace_members(project_dir: Path, members: list, excludes: list) -> list:
    """Resolve workspace member glob patterns to Cargo.toml paths."""
    import glob as glob_mod

    exclude_set = set()
    for exc in excludes:
        for match in glob_mod.glob(str(project_dir / exc)):
            exclude_set.add(Path(match).resolve())

    result = []
    for pattern in members:
        # Expand globs
        expanded = glob_mod.glob(str(project_dir / pattern))
        for match in sorted(expanded):
            match_path = Path(match).resolve()
            if match_path in exclude_set:
                continue
            cargo = match_path / "Cargo.toml" if match_path.is_dir() else match_path
            if cargo.exists() and cargo.name == "Cargo.toml":
                result.append(cargo)

    return result


# ── Dep Extraction ───────────────────────────────────────────────────────────

def extract_workspace_deps(data: dict) -> dict:
    """Extract [workspace.dependencies] table from parsed TOML data."""
    return data.get("workspace", {}).get("dependencies", {})


def extract_deps_from_table(table: dict, kind: str, ws_deps: dict,
                            declaring_file: str) -> list:
    """Extract deps from a [dependencies], [dev-dependencies], or [build-dependencies] table.

    Args:
        table: The dependency table dict
        kind: "normal" | "dev" | "build"
        ws_deps: The workspace.dependencies dict for resolving workspace=true
        declaring_file: Path of the Cargo.toml declaring these deps
    """
    deps = []
    for name, spec in table.items():
        dep = _parse_dep_spec(name, spec, kind, ws_deps, declaring_file)
        if dep:
            deps.append(dep)
    return deps


def _parse_dep_spec(name: str, spec, kind: str, ws_deps: dict,
                    declaring_file: str) -> Optional[DepSpec]:
    """Parse a single dependency specification."""
    if isinstance(spec, str):
        # Simple version string: name = "1.0"
        return DepSpec(
            name=name, package=None, source="registry",
            version=spec, path=None, git_url=None, git_branch=None,
            features=[], default_features=True, dep_kind=kind,
            workspace_inherited=False, declaring_file=declaring_file,
        )

    if isinstance(spec, dict):
        # Check for workspace = true
        if spec.get("workspace", False):
            # Resolve from workspace deps
            ws_spec = ws_deps.get(name, {})
            resolved = _parse_dep_spec(name, ws_spec, kind, {}, declaring_file) if ws_spec else None
            if resolved:
                resolved.workspace_inherited = True
                # Merge features from the member-level spec
                extra_features = spec.get("features", [])
                if extra_features:
                    resolved.features = list(set(resolved.features + extra_features))
                if "default-features" in spec:
                    resolved.default_features = spec["default-features"]
                return resolved
            # Workspace ref but not found in workspace deps
            return DepSpec(
                name=name, package=None, source="registry",
                version=None, path=None, git_url=None, git_branch=None,
                features=spec.get("features", []),
                default_features=spec.get("default-features", True),
                dep_kind=kind, workspace_inherited=True,
                declaring_file=declaring_file,
            )

        package = spec.get("package")
        version = spec.get("version")
        path = spec.get("path")
        git_url = spec.get("git")
        git_branch = spec.get("branch")
        features = spec.get("features", [])
        default_features = spec.get("default-features", True)

        if path:
            source = "path"
        elif git_url:
            source = "git"
        else:
            source = "registry"

        return DepSpec(
            name=name, package=package, source=source,
            version=version, path=path, git_url=git_url, git_branch=git_branch,
            features=features, default_features=default_features,
            dep_kind=kind, workspace_inherited=False,
            declaring_file=declaring_file,
        )

    return None


def extract_all_deps(data: dict, ws_deps: dict, declaring_file: str) -> list:
    """Extract all deps from a Cargo.toml's parsed data."""
    deps = []
    for section, kind in [
        ("dependencies", "normal"),
        ("dev-dependencies", "dev"),
        ("build-dependencies", "build"),
    ]:
        table = data.get(section, {})
        deps.extend(extract_deps_from_table(table, kind, ws_deps, declaring_file))

    # Also handle target-specific deps
    for target_key, target_data in data.get("target", {}).items():
        for section, kind in [
            ("dependencies", "normal"),
            ("dev-dependencies", "dev"),
            ("build-dependencies", "build"),
        ]:
            table = target_data.get(section, {})
            deps.extend(extract_deps_from_table(table, kind, ws_deps, declaring_file))

    return deps


def extract_patches(data: dict) -> list:
    """Extract all [patch.*] entries from parsed TOML data."""
    patches = []
    patch_tables = data.get("patch", {})
    for source_url, table in patch_tables.items():
        for name, spec in table.items():
            if isinstance(spec, dict):
                patches.append(PatchEntry(
                    source_url=source_url,
                    name=name,
                    package=spec.get("package"),
                    path=spec.get("path"),
                    git_url=spec.get("git"),
                    git_branch=spec.get("branch"),
                ))
            else:
                patches.append(PatchEntry(
                    source_url=source_url,
                    name=name,
                    package=None,
                    path=None,
                    git_url=None,
                    git_branch=None,
                ))
    return patches


# ── TOML Writing Helpers ─────────────────────────────────────────────────────

def find_section(lines: list, header: str) -> tuple:
    """Find start/end line indices for a TOML section.
    Returns (start_index, end_index) where end is the next section or EOF.
    Sub-sections (e.g. [workspace.dependencies.foo] under [workspace.dependencies])
    are included in the range."""
    # Derive prefix for sub-section matching: "[workspace.dependencies]" -> "[workspace.dependencies."
    prefix = header.rstrip("]") + "."
    start = end = None
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped == header:
            start = i
        elif start is not None and end is None and stripped.startswith("["):
            # Skip sub-sections like [workspace.dependencies.foo]
            if stripped.startswith(prefix):
                continue
            end = i
            break
    if start is not None and end is None:
        end = len(lines)
    return start, end


def read_cargo_toml(path: Path) -> tuple:
    """Read a Cargo.toml: returns (parsed_dict, raw_text)."""
    raw = path.read_text()
    with open(path, "rb") as f:
        data = tomllib.load(f)
    return data, raw


def rewrite_section(lines: list, start: int, end: int, new_lines: list) -> list:
    """Replace lines[start:end] with new_lines, return updated list."""
    return lines[:start] + new_lines + lines[end:]


# ── Path Utilities ───────────────────────────────────────────────────────────

def rel_path(ws_toml: str, local: str) -> str:
    """Relative path from workspace Cargo.toml dir to a dependency."""
    ws_dir = (REPO_ROOT / ws_toml).parent
    dep = (REPO_ROOT / local).resolve()
    return os.path.relpath(dep, ws_dir).replace("\\", "/")


def build_entry(key: str, info: dict, mode: str, ws: str, defaults: dict = None) -> Optional[str]:
    """Build a patch entry value string for a crate in a given mode.
    Returns value string or None if mode source is unavailable."""
    if defaults is None:
        defaults = {}

    if mode == "stable":
        return None

    if mode == "local":
        local = info.get("local", "N/A")
        if local == "N/A":
            return None
        return f'{{ path = "{rel_path(ws, local)}" }}'

    if mode in ("git", "zk_git"):
        url, branch = resolve_git_source(info, mode, defaults)
        if not url:
            return None
        return f'{{ git = "{url}", branch = "{branch}" }}'

    if mode == "zk_local":
        zl = info.get("zk_local")
        if zl and zl != "N/A":
            return f'{{ path = "{rel_path(ws, zl)}" }}'
        local = info.get("local", "N/A")
        if local == "N/A":
            return None
        return f'{{ path = "{rel_path(ws, local)}" }}'

    return None


def build_ws_dep_entry(key: str, info: dict, mode: str, ws_toml: str,
                       existing_spec: dict, consumer_name: str = None,
                       defaults: dict = None, dep_name: str = None) -> Optional[str]:
    """Build a [workspace.dependencies] entry string for a crate.

    Preserves existing fields like features, default-features, etc.
    Merges required_features from matrix when consumer_name matches.
    Returns None if mode source unavailable.

    Args:
        key: matrix key
        info: matrix info dict
        mode: stable/local/git/zk_local/zk_git
        ws_toml: workspace Cargo.toml relative path
        existing_spec: the existing parsed dependency spec dict
        consumer_name: project/consumer name for required_features lookup
        defaults: global defaults from [defaults] section
    """
    if defaults is None:
        defaults = {}

    if mode == "stable":
        version = info.get("stable", "N/A")
        if version == "N/A":
            return None
        parts = [f'version = "{version}"']
    elif mode == "local":
        local = info.get("local", "N/A")
        if local == "N/A":
            return None
        parts = [f'path = "{rel_path(ws_toml, local)}"']
    elif mode in ("git", "zk_git"):
        url, branch = resolve_git_source(info, mode, defaults)
        if not url:
            return None
        parts = [f'git = "{url}"', f'branch = "{branch}"']
    elif mode == "zk_local":
        zl = info.get("zk_local")
        if zl and zl != "N/A":
            parts = [f'path = "{rel_path(ws_toml, zl)}"']
        else:
            local = info.get("local", "N/A")
            if local == "N/A":
                return None
            parts = [f'path = "{rel_path(ws_toml, local)}"']
    else:
        return None

    # Preserve extra fields from existing spec
    all_features = set()
    pkg = None
    if isinstance(existing_spec, dict):
        pkg = existing_spec.get("package")
        if pkg:
            parts.insert(0, f'package = "{pkg}"')
        features = existing_spec.get("features", [])
        if features:
            all_features.update(features)
        df = existing_spec.get("default-features")
        if df is not None and df is False:
            parts.append("default-features = false")
        if existing_spec.get("optional"):
            parts.append("optional = true")
    # Auto-add package when dep name differs from matrix package name
    if not pkg and dep_name:
        matrix_pkg = info.get("package", key)
        if matrix_pkg != dep_name:
            parts.insert(0, f'package = "{matrix_pkg}"')

    # Merge required_features from matrix for this consumer
    req_feats = info.get("required_features", {})
    req_df_off = info.get("required_default_features_off", [])
    # Build set of consumer name variants for matching
    consumer_names = set()
    if consumer_name:
        consumer_names.add(consumer_name)
        if "-" in consumer_name:
            consumer_names.add(consumer_name.split("-")[0])
    if ws_toml:
        ws_parent = str(Path(ws_toml).parent)
        consumer_names.add(ws_parent)  # e.g. "bme/climb"
        consumer_names.add(ws_parent.split("/")[0])  # e.g. "bme"

    if req_feats and consumer_names:
        for cname in consumer_names:
            if cname in req_feats:
                all_features.update(req_feats[cname])
                break

    # Apply required default-features = false from matrix
    if req_df_off and consumer_names:
        for cname in consumer_names:
            if cname in req_df_off:
                if "default-features = false" not in parts:
                    parts.append("default-features = false")
                break

    if all_features:
        feat_str = ", ".join(f'"{f}"' for f in sorted(all_features))
        parts.append(f"features = [{feat_str}]")

    return "{ " + ", ".join(parts) + " }"


# ── Scan Helpers ─────────────────────────────────────────────────────────────

def _discover_all_cargo_tomls(project_dir: Path) -> list:
    """Walk project_dir and find ALL Cargo.toml files (for deep scanning)."""
    found = []
    for root, dirs, files in os.walk(project_dir):
        # Skip common non-project dirs
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        if "Cargo.toml" in files:
            found.append(Path(root) / "Cargo.toml")
    return found


def _get_excluded_dirs(project_dir: Path, data: dict) -> set:
    """Get the set of resolved excluded directory paths from workspace config."""
    import glob as glob_mod
    ws = data.get("workspace", {})
    excludes = ws.get("exclude", [])
    excluded = set()
    for exc in excludes:
        for match in glob_mod.glob(str(project_dir / exc)):
            excluded.add(Path(match).resolve())
    return excluded


LOC_EXTENSIONS = {".rs", ".toml", ".ts", ".js", ".py", ".go", ".sol"}


def count_loc(project_dir: Path) -> int:
    """Count lines of source code in a project directory.
    Counts .rs, .toml, .ts, .js, .py, .go, .sol files, skipping target/ etc."""
    total = 0
    for root, dirs, files in os.walk(project_dir):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for f in files:
            ext = os.path.splitext(f)[1]
            if ext in LOC_EXTENSIONS:
                try:
                    # Fast: count newlines without reading full content into memory
                    fp = os.path.join(root, f)
                    with open(fp, "rb") as fh:
                        total += sum(1 for _ in fh)
                except (OSError, UnicodeDecodeError):
                    continue
    return total


def scan_project(project: ProjectRoot, matrix: dict) -> ProjectScanResult:
    """Scan a single project: extract all deps, patches, and compute stats."""
    data, _ = read_cargo_toml(project.cargo_toml)
    ws_deps = extract_workspace_deps(data)
    rel_file = os.path.relpath(project.cargo_toml, REPO_ROOT)

    # Root-level deps
    all_deps = extract_all_deps(data, ws_deps, rel_file)

    # Root-level patches
    patches = extract_patches(data)

    # Member deps
    toml_errors = []
    for member_toml in project.member_tomls:
        try:
            with open(member_toml, "rb") as f:
                member_data = tomllib.load(f)
            member_rel = os.path.relpath(member_toml, REPO_ROOT)
            member_deps = extract_all_deps(member_data, ws_deps, member_rel)
            all_deps.extend(member_deps)
        except Exception as e:
            rel = os.path.relpath(member_toml, REPO_ROOT)
            toml_errors.append(f"[WARN] {project.name}: broken TOML in member '{rel}': {e}")
            continue

    # Deep scan: find ALL Cargo.tomls under the project (including excluded/hidden ones)
    # These are stored separately for diagnostics but not counted in main stats
    proj_dir = project.cargo_toml.parent
    member_set = {t.resolve() for t in project.member_tomls}
    member_set.add(project.cargo_toml.resolve())
    excluded_dirs = _get_excluded_dirs(proj_dir, data)

    hidden_deps = []
    all_tomls = _discover_all_cargo_tomls(proj_dir)
    for toml_path in all_tomls:
        resolved = toml_path.resolve()
        if resolved in member_set:
            continue
        # Determine if this toml is in an excluded dir
        toml_dir = toml_path.parent.resolve()
        is_excluded = any(
            toml_dir == exc or str(toml_dir).startswith(str(exc) + os.sep)
            for exc in excluded_dirs
        )
        try:
            with open(toml_path, "rb") as f:
                hidden_data = tomllib.load(f)
            hidden_rel = os.path.relpath(toml_path, REPO_ROOT)
            deps = extract_all_deps(hidden_data, {}, hidden_rel)
            for dep in deps:
                dep._hidden = True
                dep._is_excluded = is_excluded
            hidden_deps.extend(deps)
        except Exception as e:
            rel = os.path.relpath(toml_path, REPO_ROOT)
            toml_errors.append(f"[WARN] {project.name}: broken TOML '{rel}': {e}")
            continue

    # Compute stats (main deps only, not hidden)
    path_count = sum(1 for d in all_deps if d.source == "path")
    git_count = sum(1 for d in all_deps if d.source == "git")
    reg_count = sum(1 for d in all_deps if d.source == "registry")

    result = ProjectScanResult(
        project=project,
        workspace_deps=ws_deps,
        member_deps=all_deps,
        patches=patches,
        path_dep_count=path_count,
        git_dep_count=git_count,
        registry_dep_count=reg_count,
        patch_count=len(patches),
        loc=count_loc(project.cargo_toml.parent),
    )
    # Seed diagnostics with TOML parse errors found during scan
    result.diagnostics = toml_errors
    # Attach hidden deps for diagnostics
    result._hidden_deps = hidden_deps
    return result


def run_diagnostics(scan: ProjectScanResult, matrix: dict, mode: str = None) -> list:
    """Run diagnostic checks on a scanned project. Returns list of warning strings.
    If mode is specified, compares git branches/URLs against the mode-appropriate
    matrix section (e.g. zk_git when in zk_git mode)."""
    warnings = []
    lookup = pkg_to_matrix(matrix)
    known_pkgs = matrix_pkg_names(matrix)

    # Check git deps that should be local (matrix has a local path)
    seen_git_warns = set()
    for dep in scan.member_deps:
        dep_pkg = dep.pkg_name
        if dep.source == "git" and dep_pkg in lookup:
            _key, info = lookup[dep_pkg]
            local = info.get("local", "N/A")
            if local != "N/A" and dep.name not in seen_git_warns:
                seen_git_warns.add(dep.name)
                warnings.append(
                    f"[WARN] {scan.project.name}: git dep '{dep.name}' has local path in matrix"
                )

    # Check path deps not tracked in matrix
    seen_path_deps = set()
    for dep in scan.member_deps:
        if dep.source == "path" and dep.pkg_name not in known_pkgs:
            # Ignore internal workspace path deps (path starts with . or within same project)
            if dep.path and not dep.path.startswith(".."):
                continue
            seen_path_deps.add(dep.pkg_name)

    # Version mismatches: workspace.dependencies version != matrix stable
    for dep_name, ws_spec in scan.workspace_deps.items():
        if isinstance(ws_spec, dict):
            version = ws_spec.get("version")
        elif isinstance(ws_spec, str):
            version = ws_spec
        else:
            continue

        if dep_name in lookup and version:
            _key, info = lookup[dep_name]
            stable = info.get("stable", "N/A")
            if stable != "N/A" and version and version.lstrip("=^~") != stable:
                warnings.append(
                    f"[INFO] {scan.project.name}: '{dep_name}' version {version} vs matrix stable {stable}"
                )

    # Check git branch mismatch: workspace.dependencies vs matrix
    # Pick the best matrix git section based on mode
    git_keys = _mode_git_keys(mode)

    seen_branch_warns = set()
    for dep_name, ws_spec in scan.workspace_deps.items():
        if isinstance(ws_spec, dict) and ws_spec.get("git"):
            pkg = ws_spec.get("package", dep_name)
            if pkg in lookup:
                _key, info = lookup[pkg]
                # Find the first matching git section for this mode
                matrix_git = None
                for gk in git_keys:
                    mg = info.get(gk)
                    if mg and isinstance(mg, dict):
                        matrix_git = mg
                        break
                if not matrix_git:
                    continue
                matrix_branch = matrix_git.get("branch", "main")
                ws_branch = ws_spec.get("branch", "main")
                matrix_url = matrix_git.get("url", "")
                ws_url = ws_spec["git"]
                # Branch mismatch (same URL or close enough)
                if ws_branch != matrix_branch and dep_name not in seen_branch_warns:
                    seen_branch_warns.add(dep_name)
                    warnings.append(
                        f"[WARN] {scan.project.name}: ws dep '{dep_name}' branch "
                        f"'{ws_branch}' != matrix branch '{matrix_branch}'"
                    )
                # URL mismatch (pointing to different repo entirely)
                if matrix_url and ws_url:
                    m_id = "/".join(matrix_url.rstrip("/").split("/")[-2:]).lower()
                    w_id = "/".join(ws_url.rstrip("/").split("/")[-2:]).lower()
                    if m_id != w_id and dep_name not in seen_branch_warns:
                        seen_branch_warns.add(dep_name)
                        warnings.append(
                            f"[WARN] {scan.project.name}: ws dep '{dep_name}' git URL "
                            f"'{ws_url}' != matrix URL '{matrix_url}'"
                        )

    # Check git branch mismatch: patches vs matrix
    for patch in scan.patches:
        pkg = patch.package or patch.name
        if pkg in lookup and patch.git_url and patch.git_branch:
            _key, info = lookup[pkg]
            matrix_git = None
            for gk in git_keys:
                mg = info.get(gk)
                if mg and isinstance(mg, dict):
                    matrix_git = mg
                    break
            if not matrix_git:
                continue
            matrix_branch = matrix_git.get("branch", "main")
            if patch.git_branch != matrix_branch:
                warnings.append(
                    f"[WARN] {scan.project.name}: patch '{patch.name}' branch "
                    f"'{patch.git_branch}' != matrix branch '{matrix_branch}'"
                )

    # Check for self-referential git deps in workspace.dependencies
    # (workspace members pointing to their own repo's git URL)
    proj_data, _ = read_cargo_toml(scan.project.cargo_toml)
    ws_data = proj_data.get("workspace", {})
    ws_members_raw = ws_data.get("members", [])
    if ws_members_raw:
        # Try to detect the repo's own git URLs
        pkg_section = proj_data.get("package", {})
        ws_pkg = proj_data.get("workspace", {}).get("package", {})
        repo_url = pkg_section.get("repository", "") or ws_pkg.get("repository", "")
        # Normalize: extract github owner/repo
        own_repos = set()
        if repo_url:
            # e.g. "https://github.com/permissionlessweb/tendermint-rs" → "permissionlessweb/tendermint-rs"
            for part in repo_url.replace("https://", "").replace("http://", "").split("/")[1:]:
                pass
            parts = repo_url.rstrip("/").split("/")
            if len(parts) >= 2:
                own_repos.add("/".join(parts[-2:]).lower())

        for dep_name, ws_spec in scan.workspace_deps.items():
            if isinstance(ws_spec, dict) and ws_spec.get("git"):
                git_url = ws_spec["git"].rstrip("/")
                git_parts = git_url.split("/")
                if len(git_parts) >= 2:
                    git_id = "/".join(git_parts[-2:]).lower()
                    if git_id in own_repos:
                        warnings.append(
                            f"[WARN] {scan.project.name}: '{dep_name}' is a self-referential git dep "
                            f"(points to own repo '{git_url}', should use path)"
                        )

    # Check for features referencing non-optional dependencies
    # Runs on root Cargo.toml AND all member Cargo.tomls
    tomls_to_check = [(proj_data, scan.project.cargo_toml)]
    for member_toml in scan.project.member_tomls:
        try:
            with open(member_toml, "rb") as f:
                member_data = tomllib.load(f)
            tomls_to_check.append((member_data, member_toml))
        except Exception:
            continue

    seen_feat_warns = set()
    for check_data, check_path in tomls_to_check:
        check_features = check_data.get("features", {})
        check_deps = check_data.get("dependencies", {})
        check_rel = os.path.relpath(check_path, REPO_ROOT)
        for feat_name, feat_list in check_features.items():
            if not isinstance(feat_list, list):
                continue
            for entry in feat_list:
                if not isinstance(entry, str):
                    continue
                # Handle dep:crate syntax
                if entry.startswith("dep:"):
                    dep_ref = entry[4:]
                    if dep_ref in check_deps:
                        dep_spec = check_deps[dep_ref]
                        is_optional = isinstance(dep_spec, dict) and dep_spec.get("optional", False)
                        if not is_optional:
                            key = (check_rel, feat_name, dep_ref)
                            if key not in seen_feat_warns:
                                seen_feat_warns.add(key)
                                warnings.append(
                                    f"[WARN] {scan.project.name}: '{check_rel}' feature '{feat_name}' "
                                    f"uses 'dep:{dep_ref}' but '{dep_ref}' is not optional"
                                )
                # Handle plain crate name in features that matches a dep name
                # e.g. features: bank = ["osmosis-std"] where osmosis-std is a dep
                elif "/" not in entry and not entry.startswith("dep:"):
                    # Could be "crate/feature" (enabling feature of a dep) or just "crate"
                    dep_ref = entry
                    if dep_ref in check_deps:
                        dep_spec = check_deps[dep_ref]
                        is_optional = isinstance(dep_spec, dict) and dep_spec.get("optional", False)
                        if not is_optional:
                            key = (check_rel, feat_name, dep_ref)
                            if key not in seen_feat_warns:
                                seen_feat_warns.add(key)
                                warnings.append(
                                    f"[WARN] {scan.project.name}: '{check_rel}' feature '{feat_name}' "
                                    f"references '{dep_ref}' but it's not optional"
                                )

    # Check for missing required_features from matrix
    # Matrix entries can specify required_features = { consumer = ["feat1", "feat2"] }
    # to indicate that a consumer must enable those features on the dep
    proj_name = scan.project.name
    # Also match by consumer-style names (e.g. "abstract-framework" matches consumer "abstract")
    proj_consumer_names = {proj_name}
    if "-" in proj_name:
        proj_consumer_names.add(proj_name.split("-")[0])
    proj_consumer_names.add(scan.project.rel_path.split("/")[0])

    for dep_name, ws_spec in scan.workspace_deps.items():
        pkg = dep_name
        if isinstance(ws_spec, dict):
            pkg = ws_spec.get("package", dep_name)

        if pkg not in lookup:
            continue

        _key, info = lookup[pkg]
        req_feats = info.get("required_features", {})
        if not req_feats:
            continue

        # Find which consumer key matches this project
        matched_consumer = None
        for consumer_key in req_feats:
            if consumer_key in proj_consumer_names:
                matched_consumer = consumer_key
                break

        if not matched_consumer:
            continue

        needed = set(req_feats[matched_consumer])
        # Get actual features enabled on this dep
        actual = set()
        if isinstance(ws_spec, dict):
            actual = set(ws_spec.get("features", []))

        missing = needed - actual
        if missing:
            warnings.append(
                f"[WARN] {scan.project.name}: dep '{dep_name}' missing required features "
                f"{sorted(missing)} (needed per matrix required_features.{matched_consumer})"
            )

    # Check for external path deps (path resolves outside the git repo)
    # These break when the project is consumed via git
    seen_ext_paths = set()
    proj_dir = scan.project.rel_path
    # Git repo root: first path component (e.g., "abstract/interchain" → "abstract")
    # Cross-refs within the same git repo are fine (e.g., abstract/framework ↔ abstract/interchain)
    git_root = proj_dir.split("/")[0] if "/" in proj_dir else proj_dir

    def _is_external_path(dep_path, base_dir):
        """Check if a relative path resolves outside the git repo root."""
        if "/../" not in "/" + dep_path + "/" and not dep_path.startswith("../"):
            return False
        resolved = os.path.normpath(os.path.join(base_dir, dep_path))
        # Inside the project dir itself
        if resolved == proj_dir or resolved.startswith(proj_dir + "/"):
            return False
        # Inside the same git repo (e.g., abstract/framework and abstract/interchain)
        if resolved.startswith(git_root + "/") or resolved == git_root:
            return False
        return True

    # 1. Check [workspace.dependencies] entries directly (paths relative to project root)
    for dep_name, ws_spec in scan.workspace_deps.items():
        if isinstance(ws_spec, dict) and ws_spec.get("path"):
            dep_path = ws_spec["path"]
            if _is_external_path(dep_path, proj_dir):
                key = (dep_name, dep_path)
                if key not in seen_ext_paths:
                    seen_ext_paths.add(key)
                    warnings.append(
                        f"[WARN] {scan.project.name}: '{dep_name}' uses external path '{dep_path}' "
                        f"(breaks git resolution)"
                    )

    # 2. Check member deps (skip workspace-inherited since ws-deps check already covers those)
    for dep in scan.member_deps:
        if dep.source == "path" and dep.path and not dep.workspace_inherited:
            dep_path = dep.path
            declaring_dir = str(Path(dep.declaring_file).parent)
            if _is_external_path(dep_path, declaring_dir):
                key = (dep.pkg_name, dep_path)
                if key not in seen_ext_paths:
                    seen_ext_paths.add(key)
                    warnings.append(
                        f"[WARN] {scan.project.name}: '{dep.name}' uses external path '{dep_path}' "
                        f"(breaks git resolution)"
                    )

    # 3. Check hidden/excluded Cargo.tomls for external path deps
    # These are sub-Cargo.tomls not in workspace members — cargo auto-discovers them
    # if not properly excluded, and even excluded ones can cause issues
    hidden_deps = getattr(scan, "_hidden_deps", [])
    for dep in hidden_deps:
        if dep.source == "path" and dep.path:
            dep_path = dep.path
            declaring_dir = str(Path(dep.declaring_file).parent)
            if _is_external_path(dep_path, declaring_dir):
                is_excluded = getattr(dep, "_is_excluded", False)
                key = (dep.pkg_name, dep_path, dep.declaring_file)
                if key not in seen_ext_paths:
                    seen_ext_paths.add(key)
                    rel_file = dep.declaring_file
                    if is_excluded:
                        warnings.append(
                            f"[INFO] {scan.project.name}: excluded '{rel_file}' has external path dep "
                            f"'{dep.name}' → '{dep_path}'"
                        )
                    else:
                        warnings.append(
                            f"[WARN] {scan.project.name}: hidden '{rel_file}' has external path dep "
                            f"'{dep.name}' → '{dep_path}' (not excluded, breaks git resolution!)"
                        )

    scan.diagnostics.extend(warnings)
    return warnings


def run_cross_project_diagnostics(scans: list, matrix: dict) -> list:
    """Run diagnostics that compare across multiple projects.
    Detects when the same package is sourced from divergent git repos/branches.
    Returns list of warning strings."""
    warnings = []

    # Collect all git sources by package name (or alias) across projects
    # Key: effective package name, Value: list of (project, toml_key, git_url, branch)
    git_sources = {}

    for scan in scans:
        # From workspace.dependencies
        for dep_name, ws_spec in scan.workspace_deps.items():
            if isinstance(ws_spec, dict) and ws_spec.get("git"):
                pkg = ws_spec.get("package", dep_name)
                git_url = ws_spec["git"]
                branch = ws_spec.get("branch")
                git_sources.setdefault(pkg, []).append(
                    (scan.project.name, dep_name, git_url, branch)
                )

        # From patches
        for patch in scan.patches:
            if patch.git_url:
                pkg = patch.package or patch.name
                git_sources.setdefault(pkg, []).append(
                    (scan.project.name, patch.name, patch.git_url, patch.git_branch)
                )

    # Find divergent sources: same package, different git URL or branch
    for pkg, sources in git_sources.items():
        # Normalize URLs for comparison
        url_branches = set()
        for proj, key, url, branch in sources:
            norm_url = "/".join(url.rstrip("/").split("/")[-2:]).lower()
            url_branches.add((norm_url, branch or "(default)"))

        if len(url_branches) > 1:
            details = "; ".join(
                f"{proj}: {key}→{url}@{branch or '(default)'}"
                for proj, key, url, branch in sources
            )
            warnings.append(
                f"[WARN] cross-project: package '{pkg}' has divergent git sources: {details}"
            )

    return warnings


def validatetomls() -> list:
    """Walk all local repo clones and validate every Cargo.toml parses correctly.
    Returns list of warning strings for broken files."""
    warnings = []

    for entry in sorted(REPO_ROOT.iterdir()):
        if not entry.is_dir() or entry.name in SKIP_DIRS or entry.name.startswith("."):
            continue

        # For the 'abstract' dir, walk sub-workspaces
        if entry.name == "abstract":
            dirs_to_walk = [sub for sub in sorted(entry.iterdir()) if sub.is_dir()]
        else:
            dirs_to_walk = [entry]

        for walk_dir in dirs_to_walk:
            for root, dirs, files in os.walk(walk_dir):
                dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
                if "Cargo.toml" in files:
                    toml_path = Path(root) / "Cargo.toml"
                    try:
                        with open(toml_path, "rb") as f:
                            tomllib.load(f)
                    except Exception as e:
                        rel = os.path.relpath(toml_path, REPO_ROOT)
                        # Extract just the first line of the error for brevity
                        err_msg = str(e).split("\n")[0]
                        warnings.append(
                            f"[WARN] broken TOML: '{rel}': {err_msg}"
                        )

    return warnings


def validate_feature_optional_deps() -> list:
    """Walk all local repo Cargo.tomls and check for features referencing
    non-optional dependencies. Cargo rejects this at parse time so catching
    it here prevents 'cargo update' and 'cargo check' from failing.

    Checks both dep:crate and plain crate name syntax in [features]."""
    warnings = []

    for entry in sorted(REPO_ROOT.iterdir()):
        if not entry.is_dir() or entry.name in SKIP_DIRS or entry.name.startswith("."):
            continue

        if entry.name == "abstract":
            dirs_to_walk = [sub for sub in sorted(entry.iterdir()) if sub.is_dir()]
        else:
            dirs_to_walk = [entry]

        for walk_dir in dirs_to_walk:
            for root, dirs, files in os.walk(walk_dir):
                dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
                if "Cargo.toml" not in files:
                    continue
                toml_path = Path(root) / "Cargo.toml"
                try:
                    with open(toml_path, "rb") as f:
                        data = tomllib.load(f)
                except Exception:
                    continue

                features = data.get("features", {})
                deps = data.get("dependencies", {})
                if not features or not deps:
                    continue

                rel = os.path.relpath(toml_path, REPO_ROOT)
                for feat_name, feat_list in features.items():
                    if not isinstance(feat_list, list):
                        continue
                    for entry_str in feat_list:
                        if not isinstance(entry_str, str):
                            continue
                        # dep:crate syntax
                        if entry_str.startswith("dep:"):
                            dep_ref = entry_str[4:]
                        elif "/" not in entry_str:
                            dep_ref = entry_str
                        else:
                            continue
                        if dep_ref in deps:
                            dep_spec = deps[dep_ref]
                            is_optional = isinstance(dep_spec, dict) and dep_spec.get("optional", False)
                            if not is_optional:
                                warnings.append(
                                    f"[WARN] '{rel}': feature '{feat_name}' references "
                                    f"'{dep_ref}' but it is not optional"
                                )

    return warnings


def validate_cargo_locks() -> list:
    """Scan all Cargo.lock files for duplicate crate instances from different
    sources (e.g., same crate from registry AND git, or two git branches).
    This catches the ics23-style two-instance type-incompatibility bug.

    Returns list of warning strings."""
    warnings = []

    for entry in sorted(REPO_ROOT.iterdir()):
        if not entry.is_dir() or entry.name in SKIP_DIRS or entry.name.startswith("."):
            continue

        if entry.name == "abstract":
            dirs_to_scan = [sub for sub in sorted(entry.iterdir()) if sub.is_dir()]
        else:
            dirs_to_scan = [entry]

        for scan_dir in dirs_to_scan:
            # Only check top-level Cargo.lock (not wasm/ subprojects)
            lock_path = scan_dir / "Cargo.lock"
            if not lock_path.exists():
                continue

            rel = os.path.relpath(lock_path, REPO_ROOT)
            try:
                content = lock_path.read_text()
            except Exception:
                continue

            # Parse [[package]] entries: collect (name, version, source) tuples
            packages = {}  # name -> list of (version, source)
            current_name = None
            current_version = None
            current_source = None

            for line in content.splitlines():
                stripped = line.strip()
                if stripped == "[[package]]":
                    if current_name:
                        packages.setdefault(current_name, []).append(
                            (current_version or "?", current_source or "local")
                        )
                    current_name = current_version = current_source = None
                elif stripped.startswith('name = "'):
                    current_name = stripped.split('"')[1]
                elif stripped.startswith('version = "'):
                    current_version = stripped.split('"')[1]
                elif stripped.startswith('source = "'):
                    current_source = stripped.split('"')[1]

            # Don't forget last entry
            if current_name:
                packages.setdefault(current_name, []).append(
                    (current_version or "?", current_source or "local")
                )

            # Flag duplicates with different sources
            for pkg_name, entries in packages.items():
                if len(entries) <= 1:
                    continue
                # Group by source type (registry vs git vs path)
                sources = set()
                for ver, src in entries:
                    if src.startswith("git+"):
                        # Normalize: extract url+branch
                        sources.add(src.split("#")[0])
                    else:
                        sources.add(src)
                if len(sources) > 1:
                    src_list = ", ".join(
                        f"v{ver} from {src[:80]}" for ver, src in entries
                    )
                    warnings.append(
                        f"[WARN] '{rel}': duplicate crate '{pkg_name}' "
                        f"from different sources: {src_list}"
                    )

    return warnings


def _mode_path_keys(mode: str = None) -> list:
    """Return which path keys (local/zk_local) are relevant for a mode."""
    if mode in ("zk_local", "zk_git"):
        return ["zk_local", "local"]  # zk_local with local fallback
    if mode in ("local", "git"):
        return ["local"]
    return ["local", "zk_local"]  # None = check all


def _mode_git_keys(mode: str = None) -> list:
    """Return which git keys (git/zk_git) are relevant for a mode."""
    if mode in ("zk_git", "zk_local"):
        return ["zk_git", "git"]  # zk_git with git fallback
    if mode in ("git", "local"):
        return ["git"]
    return ["git", "zk_git"]  # None = check all


def validate_matrix_branches(matrix: dict, mode: str = None) -> list:
    """Check that git and zk_git branches in the matrix exist on their remotes.
    Groups by URL to minimize network calls (one git ls-remote per URL).
    If mode is specified, only checks branches relevant to that mode.
    Returns list of warning strings for missing branches."""
    import subprocess

    git_keys = _mode_git_keys(mode)

    # Collect all (url, branch, key, field) tuples
    checks = []
    for key, info in sorted(matrix.items()):
        for gk in git_keys:
            gi = info.get(gk)
            if gi and isinstance(gi, dict):
                url = gi.get("url")
                branch = gi.get("branch")
                if url and branch:
                    checks.append((url, branch, key, gk))

    # Group by URL → { url: set((branch, key, field)) }
    url_groups = {}
    for url, branch, key, field in checks:
        url_groups.setdefault(url, set()).add((branch, key, field))

    warnings = []
    for url in sorted(url_groups):
        entries = url_groups[url]
        # Collect unique branches needed from this URL
        needed_branches = {branch for branch, _, _ in entries}
        try:
            r = subprocess.run(
                ["git", "ls-remote", "--heads", url],
                capture_output=True, text=True, timeout=30,
            )
            if r.returncode != 0:
                warnings.append(
                    f"[WARN] git ls-remote failed for {url}: {r.stderr.strip()}"
                )
                continue

            # Parse refs/heads/<branch> from output
            remote_branches = set()
            for line in r.stdout.strip().split("\n"):
                if not line:
                    continue
                parts = line.split("\t")
                if len(parts) == 2 and parts[1].startswith("refs/heads/"):
                    remote_branches.add(parts[1][len("refs/heads/"):])

            # Check each needed branch
            for branch, key, field in sorted(entries):
                if branch not in remote_branches:
                    warnings.append(
                        f"[WARN] matrix '{key}': {field} branch '{branch}' "
                        f"not found on {url}"
                    )
        except subprocess.TimeoutExpired:
            warnings.append(f"[WARN] git ls-remote timed out for {url}")
        except Exception as e:
            warnings.append(f"[WARN] git ls-remote error for {url}: {e}")

    return warnings


def validate_matrix_paths(matrix: dict, mode: str = None) -> list:
    """Check that local and zk_local paths in the matrix actually exist on disk.
    If mode is specified, only checks paths relevant to that mode.
    Returns list of warning strings for missing paths."""
    warnings = []
    path_keys = _mode_path_keys(mode)
    forbid_abs = DEP_POLICY.get("forbid_absolute_paths", True)

    for key, info in sorted(matrix.items()):
        for pk in path_keys:
            local = info.get(pk)
            if local and local != "N/A":
                if forbid_abs and (local.startswith("/") or local.startswith("file://")):
                    warnings.append(
                        f"[WARN] matrix '{key}': {pk} path is absolute '{local}'"
                    )
                    break
                norm = normalize_matrix_local(local) or local
                resolved = REPO_ROOT / norm.lstrip("./")
                if not resolved.exists():
                    warnings.append(
                        f"[WARN] matrix '{key}': {pk} path '{local}' does not exist"
                    )
                break  # first found path is sufficient (fallback order)

    return warnings


def validate_matrix_packages(matrix: dict, mode: str = None) -> list:
    """Verify that Cargo.toml at each matrix path has the expected package name.

    For each matrix entry with a local or zk_local path, reads the Cargo.toml
    at that path and checks the [package] name matches the matrix's expected
    package name. Also checks that dep_aliases are not stale.

    If mode is specified, only checks paths relevant to that mode.
    Returns list of warning/info strings."""
    warnings = []
    path_keys = _mode_path_keys(mode)

    for key, info in sorted(matrix.items()):
        if is_repo_only(info):
            continue

        expected_pkg = info.get("package", key)
        aliases = set(info.get("dep_aliases", []))

        for path_key in path_keys:
            local = info.get(path_key)
            if not local or local == "N/A":
                continue

            resolved = REPO_ROOT / local.lstrip("./")
            cargo_toml = resolved / "Cargo.toml" if resolved.is_dir() else resolved
            if not cargo_toml.exists():
                continue  # validate_matrix_paths already flags this

            try:
                import tomllib as _toml
            except ImportError:
                try:
                    import tomli as _toml
                except ImportError:
                    continue

            try:
                with open(cargo_toml, "rb") as f:
                    data = _toml.load(f)
            except Exception:
                continue

            actual_name = data.get("package", {}).get("name")
            if not actual_name:
                # Workspace root (no [package]) or virtual manifest — skip
                continue

            # Check package name matches
            if actual_name != expected_pkg:
                warnings.append(
                    f"[WARN] matrix '{key}': {path_key} '{local}' has package "
                    f"name '{actual_name}', expected '{expected_pkg}'"
                )
                # Suggest dep_aliases if the actual name is commonly used as a dep name
                if actual_name not in aliases and actual_name != key:
                    warnings.append(
                        f"  [HINT] Consider adding dep_aliases = [\"{actual_name}\"] "
                        f"to matrix entry '{key}' if consumers import it as '{actual_name}'"
                    )

            # Check dep_aliases consistency: each alias should differ from package name
            for alias in aliases:
                if alias == expected_pkg:
                    warnings.append(
                        f"[WARN] matrix '{key}': dep_alias '{alias}' is same as "
                        f"package name (redundant)"
                    )

    return warnings


# ── Strict matrix validation / package index / heal / verify ─────────────────

@dataclass
class MatrixIssue:
    level: str  # "error" | "warn" | "info"
    key: str
    field: str
    message: str

    def format(self) -> str:
        return f"[{self.level.upper()}] matrix '{self.key}': {self.field}: {self.message}"


def build_package_index(root: Path = None, max_depth: int = 8) -> dict:
    """Map package name -> list of monorepo-relative directory paths."""
    root = root or REPO_ROOT
    index = {}
    root = root.resolve()

    for dirpath, dirnames, filenames in os.walk(root):
        # prune
        dirnames[:] = [
            d for d in dirnames
            if d not in SKIP_DIRS and not d.startswith(".")
        ]
        rel_dir = Path(dirpath).relative_to(root)
        if len(rel_dir.parts) > max_depth:
            dirnames.clear()
            continue
        if "Cargo.toml" not in filenames:
            continue
        cargo = Path(dirpath) / "Cargo.toml"
        try:
            with open(cargo, "rb") as f:
                data = tomllib.load(f)
        except Exception:
            continue
        name = data.get("package", {}).get("name")
        if not name:
            continue
        rel = str(rel_dir).replace("\\", "/")
        if rel == ".":
            rel = ""
        index.setdefault(name, []).append(rel if rel else ".")
    return index


def _package_name_at(local_path: str) -> Optional[str]:
    """Read [package].name at a monorepo-relative path, or None."""
    norm = normalize_matrix_local(local_path)
    if not norm:
        return None
    resolved = REPO_ROOT / norm
    cargo = resolved / "Cargo.toml" if resolved.is_dir() else resolved
    if not cargo.exists():
        return None
    try:
        with open(cargo, "rb") as f:
            data = tomllib.load(f)
        return data.get("package", {}).get("name")
    except Exception:
        return None


def _deprioritized(path: str) -> bool:
    prefixes = (
        DEP_POLICY.get("deprioritize_path_prefixes", {}) or {}
    ).get("prefixes", ["masp/vendor/", "build/"])
    return any(path.startswith(p) or f"/{p}" in f"/{path}" for p in prefixes)


def pick_local_path(key: str, info: dict, pkg_index: dict) -> Optional[str]:
    """Choose best monorepo-relative local path for a matrix crate."""
    expected = info.get("package", key)
    candidates = list(pkg_index.get(expected, []))

    # Also try key if different
    if key != expected:
        for c in pkg_index.get(key, []):
            if c not in candidates:
                candidates.append(c)

    if not candidates:
        # Keep existing if it already has the right package name
        for field in ("local", "zk_local"):
            cur = info.get(field)
            if not cur or cur == "N/A":
                continue
            norm = normalize_matrix_local(cur)
            if not norm:
                continue
            actual = _package_name_at(norm)
            if actual == expected:
                return norm
        return None

    def score(path: str) -> tuple:
        # lower is better
        pen = 0
        if _deprioritized(path):
            pen += 100
        # prefer paths that look like the matrix key / package
        key_hit = key.replace("_", "-") in path or key in path
        pkg_hit = expected.replace("_", "-") in path or expected in path
        if not key_hit and not pkg_hit:
            pen += 10
        # prefer shorter
        pen += path.count("/")
        # prefer non-absolute-looking
        return (pen, len(path), path)

    # Prefer current local if still valid
    for field in ("local", "zk_local"):
        cur = info.get(field)
        if not cur or cur == "N/A":
            continue
        norm = normalize_matrix_local(cur)
        if norm and norm in candidates and _package_name_at(norm) == expected:
            return norm
        if norm and _package_name_at(norm) == expected and not Path(cur).is_absolute():
            return norm

    candidates.sort(key=score)
    return candidates[0] if candidates else None


def validate_matrix_strict(matrix: dict, mode: str = None) -> list:
    """Hard validation of matrix integrity. Returns list[MatrixIssue].

    Errors: absolute paths, missing paths, package name mismatches, virtual manifests.
    """
    issues = []
    path_keys = _mode_path_keys(mode)
    forbid_abs = DEP_POLICY.get("forbid_absolute_paths", True)
    strict_pkg = DEP_POLICY.get("strict_package_names", True)

    for key, info in sorted(matrix.items()):
        if is_repo_only(info):
            # repo_only still needs a resolvable local for graph/push, but not package match
            for pk in path_keys:
                local = info.get(pk)
                if not local or local == "N/A":
                    continue
                if forbid_abs and (str(local).startswith("/") or str(local).startswith("file://")):
                    issues.append(MatrixIssue("error", key, pk, f"absolute path '{local}'"))
                else:
                    norm = normalize_matrix_local(local)
                    if norm and not (REPO_ROOT / norm).exists():
                        issues.append(MatrixIssue("warn", key, pk, f"path '{local}' does not exist"))
                break
            continue

        expected = info.get("package", key)
        saw_path = False
        for pk in path_keys:
            local = info.get(pk)
            if not local or local == "N/A":
                continue
            saw_path = True
            raw = str(local)
            if forbid_abs and (raw.startswith("/") or raw.startswith("file://")):
                issues.append(MatrixIssue(
                    "error", key, pk, f"absolute path forbidden: '{local}'"
                ))
                break

            norm = normalize_matrix_local(local)
            if not norm:
                issues.append(MatrixIssue("error", key, pk, "empty after normalize"))
                break

            resolved = REPO_ROOT / norm
            cargo = resolved / "Cargo.toml" if resolved.is_dir() else resolved
            if not cargo.exists():
                issues.append(MatrixIssue(
                    "error", key, pk, f"path '{norm}' does not exist under {REPO_ROOT}"
                ))
                break

            try:
                with open(cargo, "rb") as f:
                    data = tomllib.load(f)
            except Exception as e:
                issues.append(MatrixIssue("error", key, pk, f"unreadable Cargo.toml: {e}"))
                break

            if "package" not in data:
                if "workspace" in data:
                    issues.append(MatrixIssue(
                        "error", key, pk,
                        f"'{norm}' is a virtual workspace manifest, not a package"
                    ))
                else:
                    issues.append(MatrixIssue(
                        "error", key, pk, f"'{norm}' has no [package] table"
                    ))
                break

            actual = data["package"].get("name")
            if strict_pkg and actual and actual != expected:
                issues.append(MatrixIssue(
                    "error", key, pk,
                    f"package name '{actual}' != expected '{expected}' (path '{norm}')"
                ))
            break

        if not saw_path and not info.get("patch_only"):
            # local optional for pure-git crates, warn only
            if not info.get("git") and not info.get("stable"):
                issues.append(MatrixIssue(
                    "warn", key, "local", "no local/git/stable source configured"
                ))

    return issues


def matrix_is_clean(matrix: dict, mode: str = None) -> tuple:
    """Return (ok: bool, errors: list[str], warnings: list[str])."""
    issues = validate_matrix_strict(matrix, mode=mode)
    errors = [i.format() for i in issues if i.level == "error"]
    warns = [i.format() for i in issues if i.level != "error"]
    return (len(errors) == 0, errors, warns)


def heal_matrix_locals(matrix: dict, dry_run: bool = False) -> dict:
    """Heal local/zk_local paths in-memory. Returns report dict.

    Does not write the matrix file — use write_healed_matrix for that.
    """
    pkg_index = build_package_index()
    report = {
        "repo_root": str(REPO_ROOT),
        "packages_indexed": len(pkg_index),
        "fixed": [],
        "unchanged": [],
        "unresolved": [],
        "repo_only_normalized": [],
    }

    for key, info in sorted(matrix.items()):
        if not isinstance(info, dict):
            continue
        expected = info.get("package", key)

        for field in ("local", "zk_local"):
            cur = info.get(field)
            if cur is None or cur == "N/A":
                continue

            if is_repo_only(info):
                norm = normalize_matrix_local(cur)
                if norm and norm != cur:
                    report["repo_only_normalized"].append(
                        {"key": key, "field": field, "from": cur, "to": norm}
                    )
                    if not dry_run:
                        info[field] = norm
                continue

            picked = pick_local_path(key, info, pkg_index)
            norm_cur = normalize_matrix_local(cur)

            if picked is None:
                # try normalize-only if package already matches
                if norm_cur and _package_name_at(norm_cur) == expected:
                    if norm_cur != cur:
                        report["fixed"].append(
                            {"key": key, "field": field, "from": cur, "to": norm_cur, "reason": "normalize"}
                        )
                        if not dry_run:
                            info[field] = norm_cur
                    else:
                        report["unchanged"].append({"key": key, "field": field, "path": cur})
                elif norm_cur and (REPO_ROOT / norm_cur).exists():
                    # Path exists but package name differs — adopt actual package name
                    actual = _package_name_at(norm_cur)
                    if actual:
                        report["fixed"].append(
                            {
                                "key": key,
                                "field": field,
                                "from": cur,
                                "to": norm_cur,
                                "reason": "normalize+package",
                                "package_from": expected,
                                "package_to": actual,
                            }
                        )
                        if not dry_run:
                            info[field] = norm_cur
                            if field == "local":
                                # Only set package when matrix key is not the dep alias case
                                if info.get("package", key) == expected:
                                    info["package"] = actual
                    else:
                        # virtual manifest or unreadable
                        report["unresolved"].append(
                            {"key": key, "field": field, "current": cur, "expected_package": expected}
                        )
                else:
                    report["unresolved"].append(
                        {"key": key, "field": field, "current": cur, "expected_package": expected}
                    )
                continue

            # For zk_local, only rewrite if broken; prefer keeping distinct zk path when valid
            if field == "zk_local":
                if norm_cur and _package_name_at(norm_cur) == expected and not str(cur).startswith("/"):
                    if norm_cur != cur:
                        report["fixed"].append(
                            {"key": key, "field": field, "from": cur, "to": norm_cur, "reason": "normalize"}
                        )
                        if not dry_run:
                            info[field] = norm_cur
                    else:
                        report["unchanged"].append({"key": key, "field": field, "path": cur})
                    continue

            if picked != cur:
                report["fixed"].append(
                    {
                        "key": key,
                        "field": field,
                        "from": cur,
                        "to": picked,
                        "reason": "package-index",
                        "package": expected,
                    }
                )
                if not dry_run:
                    info[field] = picked
            else:
                report["unchanged"].append({"key": key, "field": field, "path": cur})

        # Ensure local is set when we found a package path and local was missing/N/A
        if not is_repo_only(info):
            local = info.get("local")
            if not local or local == "N/A":
                picked = pick_local_path(key, info, pkg_index)
                if picked:
                    report["fixed"].append(
                        {
                            "key": key,
                            "field": "local",
                            "from": local or "N/A",
                            "to": picked,
                            "reason": "fill-missing",
                            "package": expected,
                        }
                    )
                    if not dry_run:
                        info["local"] = picked

    return report


def write_healed_matrix(matrix: dict, path: Path = None) -> Path:
    """Rewrite dependency-matrix.toml from healed matrix dict.

    Preserves a short header comment. Uses a deterministic TOML layout.
    """
    path = path or MATRIX_PATH
    # Load defaults from existing file if present
    defaults = {}
    if path.exists():
        with open(path, "rb") as f:
            raw = tomllib.load(f)
        defaults = raw.get("defaults", {})

    lines = [
        "# dependency-matrix.toml — source of truth for forked crate resolution",
        "# Healed by dep tooling: paths are monorepo-relative to REPO_ROOT.",
        "# Do not store absolute paths. Package names must match Cargo.toml [package].name.",
        "",
    ]

    if defaults or True:
        lines.append("[defaults]")
        lines.append(f'git_branch = "{defaults.get("git_branch", "main")}"')
        lines.append(f'zk_git_branch = "{defaults.get("zk_git_branch", "zk-mvp")}"')
        lines.append("")

    def _fmt_str_list(vals):
        if not vals:
            return "[]"
        inner = ", ".join(f'"{v}"' for v in vals)
        return f"[{inner}]"

    def _write_git_table(crate_key: str, section: str, gi: dict):
        if not isinstance(gi, dict):
            return
        lines.append(f"[crates.{crate_key}.{section}]")
        if gi.get("url"):
            lines.append(f'url = "{gi["url"]}"')
        if gi.get("branch"):
            lines.append(f'branch = "{gi["branch"]}"')
        if gi.get("origin"):
            lines.append(f'origin = "{gi["origin"]}"')
        lines.append("")

    for key in sorted(matrix.keys()):
        info = matrix[key]
        if not isinstance(info, dict):
            continue
        lines.append(f"[crates.{key}]")

        # Scalar / simple fields in stable order
        order = [
            "description", "package", "stable", "local", "zk_local",
            "repo_only", "patch_only",
        ]
        for field in order:
            if field not in info:
                continue
            val = info[field]
            if val is None:
                continue
            if isinstance(val, bool):
                lines.append(f"{field} = {'true' if val else 'false'}")
            elif isinstance(val, str):
                # escape quotes
                esc = val.replace("\\", "\\\\").replace('"', '\\"')
                lines.append(f'{field} = "{esc}"')
            else:
                lines.append(f"{field} = {val}")

        if info.get("dep_aliases"):
            lines.append(f"dep_aliases = {_fmt_str_list(info['dep_aliases'])}")
        if info.get("consumers"):
            cons = info["consumers"]
            if len(cons) <= 3:
                lines.append(f"consumers = {_fmt_str_list(cons)}")
            else:
                lines.append("consumers = [")
                for c in cons:
                    lines.append(f'    "{c}",')
                lines.append("]")
        if info.get("required_default_features_off"):
            lines.append(
                f"required_default_features_off = {_fmt_str_list(info['required_default_features_off'])}"
            )

        lines.append("")

        # git / zk_git tables
        if isinstance(info.get("git"), dict):
            _write_git_table(key, "git", info["git"])
        if isinstance(info.get("zk_git"), dict):
            _write_git_table(key, "zk_git", info["zk_git"])

        # required_features nested table
        req = info.get("required_features")
        if isinstance(req, dict) and req:
            lines.append(f"[crates.{key}.required_features]")
            for consumer, feats in sorted(req.items()):
                lines.append(f'"{consumer}" = {_fmt_str_list(feats)}')
            lines.append("")

        # features nested table (matrix feature hints)
        feats = info.get("features")
        if isinstance(feats, dict) and feats:
            lines.append(f"[crates.{key}.features]")
            for fname, flist in sorted(feats.items()):
                if isinstance(flist, list):
                    lines.append(f'"{fname}" = {_fmt_str_list(flist)}')
            lines.append("")

    path.write_text("\n".join(lines) + "\n")
    return path


def cargo_metadata(workspace_dir: Path, timeout: int = 180) -> Optional[dict]:
    """Run cargo metadata in workspace_dir; return parsed JSON or None."""
    try:
        r = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=str(workspace_dir),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if r.returncode != 0:
            # retry with deps (some old cargos) — still try full metadata
            r = subprocess.run(
                ["cargo", "metadata", "--format-version", "1"],
                cwd=str(workspace_dir),
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        if r.returncode != 0:
            return None
        return json.loads(r.stdout)
    except Exception:
        return None


def _source_kind(source: Optional[str]) -> str:
    if not source:
        return "path"
    if source.startswith("registry+"):
        return "registry"
    if source.startswith("git+"):
        return "git"
    return "other"


def verify_workspace_resolution(
    workspace_dir: Path,
    matrix: dict,
    mode: str,
    defaults: dict = None,
) -> list:
    """Verify cargo metadata sources for matrix packages in one workspace.

    Returns list of error strings (empty = ok). Uses --no-deps metadata for
    packages that are members; for full resolution of deps, prefers full metadata.
    """
    defaults = defaults or {}
    errors = []

    # Full metadata so dependencies appear
    try:
        r = subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=str(workspace_dir),
            capture_output=True,
            text=True,
            timeout=300,
        )
    except Exception as e:
        return [f"{workspace_dir}: cargo metadata failed: {e}"]

    if r.returncode != 0:
        err = (r.stderr or r.stdout or "").strip().split("\n")[-1]
        return [f"{workspace_dir}: cargo metadata exit {r.returncode}: {err}"]

    try:
        meta = json.loads(r.stdout)
    except json.JSONDecodeError as e:
        return [f"{workspace_dir}: invalid metadata JSON: {e}"]

    # package name -> list of packages
    by_name = {}
    for pkg in meta.get("packages", []):
        by_name.setdefault(pkg["name"], []).append(pkg)

    lookup = pkg_to_matrix(matrix)
    known = matrix_pkg_names(matrix)

    # Dual/multi-identity: same crate name from multiple sources (classic freeze/matrix drift).
    # e.g. cosmwasm-std as path + git@freeze + git@mvp → type mismatches in dao-testing.
    critical_unity = {
        "cosmwasm-std",
        "cosmwasm-schema",
        "cosmwasm-core",
        "cosmwasm-crypto",
        "cw-schema",
        "cw-storage-plus",
        "cw-controllers",
        "cw-utils",
        "cw2",
    }
    for name, pkgs in sorted(by_name.items()):
        if len(pkgs) <= 1:
            continue
        if name not in critical_unity and name not in known and name not in lookup:
            continue
        sources = sorted(
            {(pkg.get("source") or f"path:{pkg.get('manifest_path', '')}") for pkg in pkgs}
        )
        if len(sources) > 1:
            errors.append(
                f"{workspace_dir.name}: DUAL-IDENTITY '{name}' has {len(sources)} sources "
                f"(matrix mode drift — prefer `dep.py switch dev` or unify "
                f"[patch.'git-url'] → single path tip): {sources[:4]}"
            )

    # Determine expected source kind for mode
    is_dev = mode in DEV_MODE_MAP
    if is_dev:
        ws_mode, _patch_mode, _ = DEV_MODE_MAP[mode]
        # overlay: path wins for resolved packages that are matrix-managed
        expect_kind = "path"  # local patch should win
    else:
        ws_mode = mode
        if mode in ("local", "zk_local"):
            expect_kind = "path"
        elif mode in ("git", "zk_git"):
            expect_kind = "git"
        elif mode == "stable":
            expect_kind = "registry"
        else:
            expect_kind = None

    # Only check packages that appear in this workspace's package set
    for name, pkgs in sorted(by_name.items()):
        if name not in known and name not in lookup:
            continue
        entry = lookup.get(name)
        if not entry:
            continue
        key, info = entry
        if is_repo_only(info):
            continue

        for pkg in pkgs:
            kind = _source_kind(pkg.get("source"))
            src = pkg.get("source") or pkg.get("manifest_path", "")

            if expect_kind == "path":
                if kind != "path":
                    errors.append(
                        f"{workspace_dir.name}: '{name}' expected path ({mode}), got {kind}: {src}"
                    )
                else:
                    # manifest_path should live under matrix local
                    local = info.get("zk_local") if mode in ("zk_local", "zk_dev") else info.get("local")
                    if mode == "zk_dev":
                        local = info.get("zk_local") or info.get("local")
                    if mode == "dev":
                        local = info.get("local")
                    norm = normalize_matrix_local(local) if local and local != "N/A" else None
                    if norm:
                        expected_dir = str((REPO_ROOT / norm).resolve())
                        manifest = str(Path(pkg["manifest_path"]).resolve().parent)
                        if manifest != expected_dir and not manifest.startswith(expected_dir + os.sep):
                            # allow if package path equals expected
                            if Path(manifest) != Path(expected_dir):
                                errors.append(
                                    f"{workspace_dir.name}: '{name}' path '{manifest}' "
                                    f"!= matrix '{expected_dir}'"
                                )
            elif expect_kind == "git":
                if kind != "git":
                    errors.append(
                        f"{workspace_dir.name}: '{name}' expected git ({mode}), got {kind}: {src}"
                    )
                else:
                    url, branch = resolve_git_source(info, ws_mode if ws_mode in ("git", "zk_git") else "git", defaults)
                    if url and src and url.rstrip("/") not in src.replace(".git", ""):
                        # soft: URL host/path should appear
                        id_m = "/".join(url.rstrip("/").split("/")[-2:]).lower()
                        if id_m not in src.lower():
                            errors.append(
                                f"{workspace_dir.name}: '{name}' git source '{src}' "
                                f"does not match matrix url '{url}'"
                            )
            elif expect_kind == "registry":
                if kind not in ("registry", "path"):
                    # path can still appear for workspace members; only flag git
                    if kind == "git":
                        errors.append(
                            f"{workspace_dir.name}: '{name}' expected registry ({mode}), got git: {src}"
                        )

            # Dual instances of same package name with different sources
        if len(pkgs) > 1:
            kinds = {_source_kind(p.get("source")) for p in pkgs}
            if len(kinds) > 1:
                errors.append(
                    f"{workspace_dir.name}: '{name}' has dual instances: "
                    + ", ".join(
                        f"{_source_kind(p.get('source'))}:{p.get('source') or p.get('manifest_path')}"
                        for p in pkgs
                    )
                )

    return errors


def verify_mode_resolution(
    matrix: dict,
    mode: str,
    projects: list = None,
    defaults: dict = None,
    max_workspaces: int = 20,
) -> dict:
    """Verify resolution across workspace projects. Returns report dict."""
    defaults = defaults or load_defaults()
    if projects is None:
        projects = discover_projects()

    # Prefer real workspaces
    workspaces = [p for p in projects if p.is_workspace]
    if not workspaces:
        workspaces = projects

    report = {
        "mode": mode,
        "repo_root": str(REPO_ROOT),
        "checked": [],
        "errors": [],
        "skipped": [],
    }

    for proj in workspaces[:max_workspaces]:
        ws_dir = proj.cargo_toml.parent
        # Skip if no Cargo.lock and never built — still try metadata
        errs = verify_workspace_resolution(ws_dir, matrix, mode, defaults=defaults)
        report["checked"].append(proj.rel_path)
        report["errors"].extend(errs)

    return report


# ── Nix / monorepo bootstrap (complements dep-switch; does not replace it) ───

NIX_DIR = DEVOPS_DIR / "nix"
REPOS_JSON_PATH = NIX_DIR / "repos.json"


def github_repo_name(url: str) -> str:
    """Extract repo name from a git URL."""
    u = url.rstrip("/")
    if u.endswith(".git"):
        u = u[:-4]
    return u.split("/")[-1]


def collect_bootstrap_repos(matrix: dict = None, defaults: dict = None) -> list:
    """Collapse matrix crates into unique git clones for monorepo bootstrap.

    Returns list of dicts:
      {
        id, url, branch, dest,  # dest = top-level dir under REPO_ROOT
        package_paths,          # matrix local paths living inside dest
        crate_keys,             # matrix keys covered
        zk_branch,              # optional
      }
    """
    from collections import Counter, defaultdict

    matrix = matrix if matrix is not None else load_matrix()
    defaults = defaults if defaults is not None else load_defaults()

    # url -> accum
    acc = defaultdict(lambda: {
        "branches": Counter(),
        "zk_branches": Counter(),
        "tops": Counter(),
        "package_paths": set(),
        "keys": [],
    })

    for key, info in matrix.items():
        if not isinstance(info, dict):
            continue

        url, branch = resolve_git_source(info, "git", defaults)
        zk_url, zk_branch = resolve_git_source(info, "zk_git", defaults)

        # Prefer primary git URL; zk may share same URL with different branch
        primary = url or zk_url
        if not primary or primary == "N/A":
            continue

        e = acc[primary]
        e["keys"].append(key)
        if branch:
            e["branches"][branch] += 1
        if zk_url == primary and zk_branch:
            e["zk_branches"][zk_branch] += 1
        elif zk_url and zk_url != primary:
            # separate zk remote — rare; track as its own entry via recursion-like insert
            ze = acc[zk_url]
            ze["keys"].append(key + "#zk")
            if zk_branch:
                ze["branches"][zk_branch] += 1

        for field in ("local", "zk_local"):
            loc = info.get(field)
            if not loc or loc == "N/A":
                continue
            norm = normalize_matrix_local(loc)
            if not norm:
                continue
            e["package_paths"].add(norm)
            top = norm.split("/")[0]
            if top:
                e["tops"][top] += 1

    repos = []
    for url, e in sorted(acc.items(), key=lambda kv: github_repo_name(kv[0]).lower()):
        if e["branches"]:
            branch = e["branches"].most_common(1)[0][0]
        else:
            branch = defaults.get("git_branch", "main")
        zk_branch = None
        if e["zk_branches"]:
            zk_branch = e["zk_branches"].most_common(1)[0][0]

        if e["tops"]:
            dest = e["tops"].most_common(1)[0][0]
        else:
            dest = github_repo_name(url)

        # Special cases: avoid collapsing unrelated tops
        # If tops disagree heavily, prefer github name if present in tops
        gname = github_repo_name(url)
        if gname in e["tops"]:
            dest = gname
        # cw-multi-test-fork clones often map to clone-cw-multi-test or cw-multi-test-fork
        if "cw-multi-test-fork" in url and "clone-cw-multi-test" in e["tops"]:
            dest = "clone-cw-multi-test"
        if "cw-multi-test-fork" in url and "cw-multi-test-fork" in e["tops"]:
            dest = "cw-multi-test-fork"

        repos.append({
            "id": gname,
            "url": url,
            "branch": branch,
            "zk_branch": zk_branch,
            "dest": dest,
            "package_paths": sorted(e["package_paths"]),
            "crate_keys": sorted(set(e["keys"])),
        })

    # Dedup by dest: prefer more package_paths
    by_dest = {}
    for r in repos:
        d = r["dest"]
        if d not in by_dest or len(r["package_paths"]) > len(by_dest[d]["package_paths"]):
            by_dest[d] = r
    return sorted(by_dest.values(), key=lambda r: r["dest"])


def export_nix_repos(path: Path = None, matrix: dict = None) -> Path:
    """Write repos.json for Nix bootstrap + documentation."""
    path = path or REPOS_JSON_PATH
    path.parent.mkdir(parents=True, exist_ok=True)
    repos = collect_bootstrap_repos(matrix=matrix)
    payload = {
        "schema_version": 1,
        "description": (
            "Unique git forks to clone under monorepo_root for first-time bootstrap. "
            "Generated from dependency-matrix.toml — do not hand-edit; run dep.py nix-export."
        ),
        "monorepo_root_hint": "terp-core/crates (see dep-config.toml monorepo_root)",
        "generated_by": "dep.py nix-export",
        "repo_count": len(repos),
        "repos": repos,
    }
    path.write_text(json.dumps(payload, indent=2) + "\n")
    return path


def bootstrap_status(repos: list = None) -> dict:
    """Report which bootstrap dests exist under REPO_ROOT."""
    repos = repos if repos is not None else collect_bootstrap_repos()
    present = []
    missing = []
    for r in repos:
        dest = REPO_ROOT / r["dest"]
        git_ok = (dest / ".git").exists() or (dest / ".git").is_file()
        if dest.is_dir() and (git_ok or any(dest.iterdir())):
            present.append({**r, "path": str(dest), "has_git": git_ok})
        else:
            missing.append({**r, "path": str(dest)})
    return {
        "repo_root": str(REPO_ROOT),
        "total": len(repos),
        "present": present,
        "missing": missing,
    }


def bootstrap_clone_repos(
    repos: list = None,
    *,
    dry_run: bool = False,
    shallow: bool = True,
    update: bool = False,
    only_missing: bool = True,
    jobs: int = 1,
) -> dict:
    """Clone matrix forks into REPO_ROOT/<dest>.

    Complements dep-switch: gets the *source trees* onto disk so path mode works.
    Does not rewrite Cargo.toml (use dep.py switch for that).
    """
    repos = repos if repos is not None else collect_bootstrap_repos()
    report = {
        "repo_root": str(REPO_ROOT),
        "cloned": [],
        "skipped": [],
        "updated": [],
        "failed": [],
        "dry_run": dry_run,
    }

    def _run(cmd, cwd=None):
        return subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, timeout=600,
        )

    for r in repos:
        dest = REPO_ROOT / r["dest"]
        url = r["url"]
        branch = r["branch"]

        if dest.exists() and any(dest.iterdir()):
            if only_missing and not update:
                report["skipped"].append({"dest": r["dest"], "reason": "exists"})
                continue
            if update and (dest / ".git").exists():
                if dry_run:
                    report["updated"].append({"dest": r["dest"], "action": "fetch+checkout"})
                    continue
                # fetch branch tip
                r1 = _run(["git", "-C", str(dest), "fetch", "origin", branch])
                if r1.returncode != 0:
                    report["failed"].append({
                        "dest": r["dest"], "step": "fetch",
                        "stderr": (r1.stderr or "")[-400:],
                    })
                    continue
                r2 = _run(["git", "-C", str(dest), "checkout", branch])
                r3 = _run(["git", "-C", str(dest), "pull", "--ff-only", "origin", branch])
                if r2.returncode != 0 or r3.returncode != 0:
                    report["failed"].append({
                        "dest": r["dest"], "step": "checkout/pull",
                        "stderr": ((r2.stderr or "") + (r3.stderr or ""))[-400:],
                    })
                else:
                    report["updated"].append({"dest": r["dest"]})
                continue
            report["skipped"].append({"dest": r["dest"], "reason": "exists-no-update"})
            continue

        if dry_run:
            report["cloned"].append({"dest": r["dest"], "url": url, "branch": branch})
            continue

        dest.parent.mkdir(parents=True, exist_ok=True)
        cmd = ["git", "clone", "--branch", branch]
        if shallow:
            cmd += ["--depth", "1", "--single-branch"]
        cmd += [url, str(dest)]
        result = _run(cmd)
        if result.returncode != 0:
            # retry without branch (default branch) then checkout
            cmd2 = ["git", "clone"]
            if shallow:
                cmd2 += ["--depth", "1"]
            cmd2 += [url, str(dest)]
            result2 = _run(cmd2)
            if result2.returncode != 0:
                report["failed"].append({
                    "dest": r["dest"],
                    "url": url,
                    "branch": branch,
                    "stderr": (result.stderr or result2.stderr or "")[-500:],
                })
                continue
            _run(["git", "-C", str(dest), "checkout", branch])
        report["cloned"].append({"dest": r["dest"], "url": url, "branch": branch})

    return report
