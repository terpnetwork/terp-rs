#!/usr/bin/env python3
"""
Dependency Switcher for the Abstract monorepo (v4 - full coverage).

Edits [patch.crates-io], [workspace.dependencies], and direct member deps
in workspace Cargo.tomls. Reads _devops/dependency-matrix.toml for fork
crate definitions. Non-matrix entries are preserved.

Targets:
  - patches:  [patch.crates-io] in workspace root
  - ws-deps:  [workspace.dependencies] in workspace root
  - members:  direct deps in member Cargo.tomls (not workspace=true)
  - all:      patches + ws-deps + members

Modes:
  - stable:    Remove patches / revert to crates.io version
  - local:     path = "../../fork/..."
  - git:       git = "...", branch = "..."
  - zk_local:  path = "../../zk-fork/..." (falls back to local)
  - zk_git:    git = "...", branch = "..." (ZK URLs, falls back to git)
  - dev:       git URLs in ws-deps/members + local paths via [patch.'<url>']
  - zk_dev:    zk_git URLs in ws-deps/members + zk_local paths via [patch.'<url>']
  - patch-local: version lines stay; [patch.crates-io] → path
  - patch-git:   version lines stay; [patch.crates-io] → git + full rev
                 (does not also write [patch.'<git-url>']; Cargo keeps the
                 first replacement and will not chain a path over that git rev)

Usage:
    python3 _scripts/dep-switch.py switch <mode> [--dry-run]
    python3 _scripts/dep-switch.py switch <mode> --target all
    python3 _scripts/dep-switch.py switch <mode> --workspace cw-plus --target all
    python3 _scripts/dep-switch.py switch <mode> --crate cosmwasm-std --target members
    python3 _scripts/dep-switch.py status
"""

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("ERROR: Python 3.11+ required (for tomllib), or install tomli")
        sys.exit(1)

from dep_common import (
    DEV_MODE_MAP,
    KNOWN_WORKSPACE_ROOTS,
    PATCH_MODE_MAP,
    REPO_ROOT,
    build_entry,
    build_ws_dep_entry,
    discover_projects,
    drop_unreferenced_git_patches,
    extract_workspace_deps,
    find_section,
    git_dependency_urls,
    is_repo_only,
    load_defaults,
    load_matrix,
    matrix_pkg_names,
    pkg_to_matrix,
    read_cargo_toml,
    rel_path,
    resolve_git_source,
    save_active_mode,
)


def _is_repo_only_by_name(name: str, crates: dict) -> bool:
    """Check if a crate name found in a Cargo.toml corresponds to a repo_only matrix entry.
    Looks up the name against matrix keys and package names."""
    for key, info in crates.items():
        if not is_repo_only(info):
            continue
        if key == name or info.get("package", key) == name:
            return True
    return False


def validate_toml(path):
    """Validate a TOML file parses correctly. Returns True if valid, prints error and returns False otherwise."""
    try:
        with open(path, "rb") as f:
            tomllib.load(f)
        return True
    except Exception as e:
        print(f"  ERROR: invalid TOML after write: {path}")
        print(f"         {e}")
        return False


def cargo_sort(path):
    """Run cargo-sort on a Cargo.toml to keep dependency tables sorted."""
    import subprocess
    try:
        subprocess.run(
            ["cargo", "sort", "--grouped", str(Path(path).parent)],
            capture_output=True, timeout=10,
        )
    except Exception:
        pass  # non-fatal — sorting is best-effort


# ── Resolution helpers ───────────────────────────────────────────────────────

def resolve_projects(ws_filter=None, all_projects=False):
    """Resolve which projects to operate on. Returns list of ProjectRoot objects.

    If all_projects is True, returns all discovered projects.
    If ws_filter is None (and not all_projects), returns projects matching
    KNOWN_WORKSPACE_ROOTS.
    Otherwise, matches by name or path against discovered projects.
    """
    projects = discover_projects()

    if all_projects and not ws_filter:
        return projects

    if not ws_filter:
        # Default: known workspace roots
        known_dirs = {str(Path(w).parent) for w in KNOWN_WORKSPACE_ROOTS}
        return [p for p in projects if p.rel_path in known_dirs]

    result = []
    for f in ws_filter:
        matched = False
        for p in projects:
            if f in (p.name, p.rel_path):
                result.append(p)
                matched = True
                break
        if not matched:
            print(f"WARNING: workspace '{f}' not found, skipping")
    return result


def resolve_crate_filter(crate_filter, crates):
    """Resolve --crate filter to a set of published package names."""
    if not crate_filter:
        return None  # No filter = all crates

    known = matrix_pkg_names(crates)
    key_to_pkg = {k: v.get("package", k) for k, v in crates.items()}

    result = set()
    for c in crate_filter:
        if c in known:
            result.add(c)
        elif c in key_to_pkg:
            result.add(key_to_pkg[c])
        else:
            print(f"WARNING: crate '{c}' not found in matrix, skipping")
    return result if result else None


# ── Patch Rewriting ──────────────────────────────────────────────────────────

def _absorb_dep_names(table, known, names):
    """Record matrix package names declared in one dependency table."""
    if not isinstance(table, dict):
        return
    for name, spec in table.items():
        if name in known:
            names.add(name)
        if isinstance(spec, dict):
            pkg = spec.get("package")
            if isinstance(pkg, str) and pkg in known:
                names.add(pkg)


def _declared_matrix_names(ws_toml, crates):
    """Matrix package names this workspace (and its members) actually depend on.

    Scans workspace.dependencies plus dependency tables. Member scans pick up
    crates that are not re-exported from the workspace table, such as
    dao-testing's cw1-whitelist.
    """
    import glob as glob_mod

    known = matrix_pkg_names(crates)
    names = set()
    cargo_path = REPO_ROOT / ws_toml

    def absorb(data):
        if not isinstance(data, dict):
            return
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            _absorb_dep_names(data.get(section), known, names)
        ws = data.get("workspace", {}).get("dependencies", {})
        _absorb_dep_names(ws, known, names)
        target = data.get("target")
        if isinstance(target, dict):
            for cfg in target.values():
                if not isinstance(cfg, dict):
                    continue
                for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                    _absorb_dep_names(cfg.get(section), known, names)

    try:
        data, _ = read_cargo_toml(cargo_path)
    except Exception:
        return names
    absorb(data)

    ws_dir = cargo_path.parent
    exclude = set()
    for exc in data.get("workspace", {}).get("exclude", []):
        for match in glob_mod.glob(str(ws_dir / exc)):
            exclude.add(Path(match).resolve())
    seen = set()
    for pattern in data.get("workspace", {}).get("members", []):
        for match in glob_mod.glob(str(ws_dir / pattern)):
            match_path = Path(match).resolve()
            if match_path in exclude or not match_path.is_dir():
                continue
            member_cargo = match_path / "Cargo.toml"
            if member_cargo in seen or not member_cargo.exists():
                continue
            seen.add(member_cargo)
            try:
                member_data, _ = read_cargo_toml(member_cargo)
            except Exception:
                continue
            absorb(member_data)
    return names


def _patch_pkg_name(declared_name, lookup):
    """crates.io package name a [patch.crates-io] key must use.

    Dependency keys can be aliases (`cw-multi-test` → `abstract-cw-multi-test`).
    The patch key is the published package name.
    """
    hit = lookup.get(declared_name)
    if not hit:
        return None
    key, info = hit
    if is_repo_only(info):
        return None
    return info.get("package", key), key, info


def rewrite_patches(ws_toml, crates, mode, crate_filter=None, dry_run=False, defaults=None, ensure=False):
    """Update [patch.crates-io] for one workspace Cargo.toml.

    ensure=True inserts entries for matrix crates this manifest declares, and
    creates the section when it is missing. Used by patch-local / patch-git.
    Other modes leave a missing section alone.
    """
    cargo_path = REPO_ROOT / ws_toml
    content = cargo_path.read_text()
    lines = content.split("\n")

    start, end = find_section(lines, "[patch.crates-io]")
    created = False
    if start is None:
        if not ensure:
            return False
        if lines and lines[-1] != "":
            lines.append("")
        start = len(lines)
        lines.append("[patch.crates-io]")
        end = len(lines)
        created = True

    known = matrix_pkg_names(crates)
    lookup = pkg_to_matrix(crates)

    new_section = ["[patch.crates-io]"]
    static_lines = []

    for line in lines[start + 1 : end]:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            static_lines.append(line)
            continue
        if "=" not in stripped:
            static_lines.append(line)
            continue

        name = stripped.split("=", 1)[0].strip()
        if _is_repo_only_by_name(name, crates):
            continue  # drop repo-level entries (not real crates)
        if name in known:
            if crate_filter and name not in crate_filter:
                static_lines.append(line)
                continue
            key, info = lookup[name]
            # Self-referential: preserve as-is
            if _is_self_referential(ws_toml, info, mode):
                static_lines.append(line)
            else:
                val = build_entry(key, info, mode, ws_toml, defaults=defaults)
                patch_name = name
                if ensure:
                    mapped = _patch_pkg_name(name, lookup)
                    if mapped:
                        patch_name = mapped[0]
                if val:
                    new_section.append(f"{patch_name} = {val}")
                elif mode == "git-rev":
                    print(f"  [WARN] {ws_toml}: skip patch {patch_name}: "
                          f"git.rev/git.commit is not a full 40-hex sha")
        else:
            static_lines.append(line)

    # Also add NEW patch entries for matrix crates that are marked as
    # patch_only=true (transitive deps that need patching but aren't direct).
    seen_names = {l.split("=", 1)[0].strip() for l in new_section[1:]}
    proj_name = str(Path(ws_toml).parent)
    for key, info in crates.items():
        if is_repo_only(info):
            continue
        if not info.get("patch_only"):
            continue
        consumers = info.get("consumers", [])
        pkg = info.get("package", key)
        if pkg in seen_names:
            continue
        if not any(proj_name == c or proj_name.endswith("/" + c) or c in proj_name for c in consumers):
            continue
        if crate_filter and pkg not in crate_filter:
            continue
        # Self-referential: never touch
        if _is_self_referential(ws_toml, info, mode):
            continue
        val = build_entry(key, info, mode, ws_toml, defaults=defaults)
        if val:
            new_section.append(f"{pkg} = {val}")
        elif mode == "git-rev":
            print(f"  [WARN] {ws_toml}: skip patch {pkg}: "
                  f"git.rev/git.commit is not a full 40-hex sha")

    if ensure:
        seen_names = {l.split("=", 1)[0].strip() for l in new_section[1:] if "=" in l}
        for declared in sorted(_declared_matrix_names(ws_toml, crates)):
            mapped = _patch_pkg_name(declared, lookup)
            if not mapped:
                continue
            pkg, key, info = mapped
            if pkg in seen_names:
                continue
            if crate_filter and pkg not in crate_filter and declared not in crate_filter:
                continue
            if _is_self_referential(ws_toml, info, mode):
                continue
            val = build_entry(key, info, mode, ws_toml, defaults=defaults)
            if val:
                new_section.append(f"{pkg} = {val}")
                seen_names.add(pkg)
            elif mode == "git-rev":
                print(f"  [WARN] {ws_toml}: skip patch {pkg}: "
                      f"git.rev/git.commit is not a full 40-hex sha")

    final_section = ["[patch.crates-io]"]
    for line in static_lines:
        final_section.append(line)
    matrix_lines = [l for l in new_section[1:]]
    if created and not matrix_lines:
        return False
    if matrix_lines:
        if final_section[-1].strip():
            final_section.append("")
        final_section.append("")
        final_section.extend(matrix_lines)

    new_content = "\n".join(lines[:start] + final_section + lines[end:])

    if new_content != content:
        if dry_run:
            print(f"  [DRY RUN] Would modify patches in {ws_toml}")
            for line in matrix_lines:
                print(f"    {line}")
        else:
            cargo_path.write_text(new_content)
            if not validate_toml(cargo_path):
                cargo_path.write_text(content)
                print(f"  REVERTED patches in {ws_toml} (invalid TOML)")
                return False
            cargo_sort(cargo_path)
            print(f"  Modified patches in {ws_toml}")
        return True
    return False


# ── Git URL Patch Rewriting (dev/zk_dev modes) ──────────────────────────────

def _find_all_patch_sections(lines):
    """Find all [patch.'<url>'] sections in lines.
    Returns list of (start, end, url) tuples."""
    sections = []
    # cargo-sort rewrites the header with double quotes. Both forms are the
    # same section, and a missed header leaves a second patch in place.
    PATCH_RE = re.compile(r"""^\[patch\.(?:'([^']+)'|"([^"]+)")\]\s*$""")
    for i, line in enumerate(lines):
        m = PATCH_RE.match(line.strip())
        if m:
            sections.append((i, m.group(1) or m.group(2)))

    result = []
    for idx, (start, url) in enumerate(sections):
        # End is next section header or EOF
        end = len(lines)
        for j in range(start + 1, len(lines)):
            if lines[j].strip().startswith("["):
                end = j
                break
        result.append((start, end, url))
    return result


def rewrite_git_url_patches(ws_toml, crates, mode, crate_filter=None, dry_run=False, defaults=None):
    """Generate [patch.'<git-url>'] sections with local path entries for dev/zk_dev modes.

    Groups matrix crates by their resolved git URL, then for each URL writes a
    patch section redirecting those crates to local paths. Skips patch_only crates
    (those only need [patch.crates-io]) and self-referential crates.
    """
    if defaults is None:
        defaults = {}

    ws_dep_mode, patch_mode, local_key = DEV_MODE_MAP[mode]

    cargo_path = REPO_ROOT / ws_toml
    content = cargo_path.read_text()
    lines = content.split("\n")

    known = matrix_pkg_names(crates)
    proj_name = str(Path(ws_toml).parent)

    # Scan [workspace.dependencies] for git URLs actually referenced by this workspace.
    # Only generate [patch.'<url>'] sections for URLs that the workspace depends on.
    ws_dep_urls = set()
    sec_start, sec_end = find_section(lines, "[workspace.dependencies]")
    if sec_start is not None:
        for line in lines[sec_start + 1 : sec_end]:
            stripped = line.strip()
            # Extract git URLs from entries like: crate = { git = "https://...", ... }
            m = re.search(r'git\s*=\s*["\']([^"\']+)["\']', stripped)
            if m:
                ws_dep_urls.add(m.group(1))

    # Also include git URLs for self-referential crates — transitive deps from other
    # git sources may depend on these, so [patch.'<url>'] must redirect them to local.
    for key, info in crates.items():
        if is_repo_only(info) or info.get("patch_only"):
            continue
        if _is_self_referential(ws_toml, info, mode):
            url, _ = resolve_git_source(info, ws_dep_mode, defaults)
            if url:
                ws_dep_urls.add(url)

    # Group crates by git URL -> list of (pkg_name, local_path_entry)
    url_crates = {}  # url -> [(pkg_name, path_entry_str)]
    for key, info in crates.items():
        # Skip repo-level entries (not published crates)
        if is_repo_only(info):
            continue

        # Skip patch_only — those go in [patch.crates-io] only
        if info.get("patch_only"):
            continue

        pkg = info.get("package", key)

        if crate_filter and pkg not in crate_filter:
            continue

        is_self_ref = _is_self_referential(ws_toml, info, mode)

        # Resolve git URL for this crate
        url, branch = resolve_git_source(info, ws_dep_mode, defaults)
        if not url:
            continue

        # Only patch URLs that the workspace actually references in [workspace.dependencies]
        # or that belong to self-referential crates (needed for transitive dep patching).
        if ws_dep_urls and url not in ws_dep_urls:
            continue

        # No per-crate consumer filter for [patch.'<url>'] — these must cover all
        # packages from a referenced URL, including transitive deps. The URL-level
        # filter already ensures we only patch repos the workspace depends on.

        # Resolve local path for this crate
        local = info.get(local_key) if local_key != "local" else None
        if not local or local == "N/A":
            local = info.get("local", "N/A")
        if local == "N/A":
            continue

        # Validate the local path points to a package (not a virtual manifest)
        local_abs = REPO_ROOT / local.lstrip("./")
        local_cargo = local_abs / "Cargo.toml" if local_abs.is_dir() else local_abs
        if local_cargo.exists():
            try:
                with open(local_cargo, "rb") as f:
                    crate_data = tomllib.load(f)
                if "package" not in crate_data and "workspace" in crate_data:
                    # Virtual manifest — skip (local path in matrix needs fixing)
                    print(f"  [WARN] Skipping {pkg}: local path '{local}' is a virtual manifest")
                    continue
            except Exception:
                pass

        path_str = rel_path(ws_toml, local)
        url_crates.setdefault(url, []).append((pkg, path_str))

    if not url_crates:
        return False

    # Find existing [patch.'<url>'] sections
    existing_sections = _find_all_patch_sections(lines)

    # Collect all matrix-managed git URLs
    matrix_urls = set(url_crates.keys())

    # Build new content: remove existing matrix-managed patch sections,
    # preserve manual entries within them
    # Process in reverse to maintain line indices
    preserved_manual = {}  # url -> [manual_lines]
    for start, end, url in reversed(existing_sections):
        if url not in matrix_urls:
            continue
        # Separate matrix-tracked vs manual entries in this section
        manual_lines = []
        for line in lines[start + 1 : end]:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                manual_lines.append(line)
                continue
            if "=" not in stripped:
                manual_lines.append(line)
                continue
            name = stripped.split("=", 1)[0].strip()
            if name not in known and not _is_repo_only_by_name(name, crates):
                manual_lines.append(line)
        # Only keep manual lines if there's actual content
        has_content = any(l.strip() and not l.strip().startswith("#") for l in manual_lines)
        if has_content:
            preserved_manual[url] = manual_lines
        # Remove the section
        lines[start:end] = []

    # Build new sections
    new_sections = []
    for url in sorted(url_crates.keys()):
        entries = url_crates[url]
        section_lines = [f"[patch.'{url}']"]
        # Collect matrix package names for dedup
        matrix_pkgs = {pkg for pkg, _ in entries}
        # Add preserved manual entries first (skip if matrix will provide it)
        if url in preserved_manual:
            for ml in preserved_manual[url]:
                stripped = ml.strip()
                if stripped and not stripped.startswith("#") and "=" in stripped:
                    name = stripped.split("=", 1)[0].strip()
                    if name in matrix_pkgs:
                        continue  # matrix entry takes precedence
                section_lines.append(ml)
        # Add matrix entries
        for pkg, path_str in sorted(entries):
            section_lines.append(f'{pkg} = {{ path = "{path_str}" }}')
        section_lines.append("")
        new_sections.extend(section_lines)

    # Find insertion point: before [profile.*] or at EOF
    insert_at = len(lines)
    for i, line in enumerate(lines):
        if line.strip().startswith("[profile"):
            insert_at = i
            break

    # Ensure blank line before new sections
    if insert_at > 0 and lines[insert_at - 1].strip():
        lines.insert(insert_at, "")
        insert_at += 1

    for i, sec_line in enumerate(new_sections):
        lines.insert(insert_at + i, sec_line)

    new_content = "\n".join(lines)

    if new_content != content:
        if dry_run:
            print(f"  [DRY RUN] Would add/update git-url patches in {ws_toml}")
            for url in sorted(url_crates):
                print(f"    [patch.'{url}']: {len(url_crates[url])} entries")
        else:
            cargo_path.write_text(new_content)
            if not validate_toml(cargo_path):
                cargo_path.write_text(content)
                print(f"  REVERTED git-url patches in {ws_toml} (invalid TOML)")
                return False
            cargo_sort(cargo_path)
            print(f"  Modified git-url patches in {ws_toml}")
        return True
    return False


def remove_git_url_patches(ws_toml, crates, dry_run=False, defaults=None):
    """Remove matrix-managed [patch.'<git-url>'] sections when leaving dev mode.

    Preserves self-referential sections and any manual (non-matrix) entries.
    """
    if defaults is None:
        defaults = {}

    cargo_path = REPO_ROOT / ws_toml
    content = cargo_path.read_text()
    lines = content.split("\n")

    known = matrix_pkg_names(crates)

    # Collect all git URLs from matrix (both git and zk_git)
    matrix_urls = set()
    for key, info in crates.items():
        for m in ("git", "zk_git"):
            url, _ = resolve_git_source(info, m, defaults)
            if url:
                matrix_urls.add(url)

    # Build lookup: pkg_name -> (matrix_key, info) for self-ref checks
    lookup = pkg_to_matrix(crates)

    existing_sections = _find_all_patch_sections(lines)

    # Process in reverse to maintain indices
    changed = False
    for start, end, url in reversed(existing_sections):
        if url not in matrix_urls:
            continue

        # Check entries: classify as self-ref, manual, or matrix-removable
        # * names are always removable (repo-level, not real crates)
        manual_lines = []
        has_manual = False
        self_ref_lines = []
        has_self_ref = False
        for line in lines[start + 1 : end]:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                manual_lines.append(line)
                self_ref_lines.append(line)
                continue
            if "=" not in stripped:
                manual_lines.append(line)
                self_ref_lines.append(line)
                continue
            name = stripped.split("=", 1)[0].strip()
            if _is_repo_only_by_name(name, crates):
                continue  # always removable
            if name not in known:
                manual_lines.append(line)
                has_manual = True
                self_ref_lines.append(line)
            elif name in lookup:
                _, info = lookup[name]
                if _is_self_referential(ws_toml, info, "git"):
                    # Self-referential entry — preserve it
                    self_ref_lines.append(line)
                    has_self_ref = True

        if has_manual or has_self_ref:
            # Keep section header + manual + self-ref entries
            keep_lines = []
            for line in lines[start + 1 : end]:
                stripped = line.strip()
                if not stripped or stripped.startswith("#"):
                    keep_lines.append(line)
                    continue
                if "=" not in stripped:
                    keep_lines.append(line)
                    continue
                name = stripped.split("=", 1)[0].strip()
                if _is_repo_only_by_name(name, crates):
                    continue  # always removable
                if name not in known:
                    keep_lines.append(line)
                elif name in lookup:
                    _, info = lookup[name]
                    if _is_self_referential(ws_toml, info, "git"):
                        keep_lines.append(line)
            replacement = [f"[patch.'{url}']"] + keep_lines
            if replacement != lines[start:end]:
                lines[start:end] = replacement
                changed = True
        else:
            # Remove entire section
            # Also remove trailing blank line if present
            rm_end = end
            if rm_end < len(lines) and not lines[rm_end].strip():
                rm_end += 1
            lines[start:rm_end] = []
            changed = True

    if changed:
        new_content = "\n".join(lines)
        if dry_run:
            print(f"  [DRY RUN] Would remove git-url patches from {ws_toml}")
        else:
            cargo_path.write_text(new_content)
            if not validate_toml(cargo_path):
                cargo_path.write_text(content)
                print(f"  REVERTED git-url patch removal in {ws_toml} (invalid TOML)")
                return False
            cargo_sort(cargo_path)
            print(f"  Removed git-url patches from {ws_toml}")
        return True
    return False


# ── Workspace Dependencies Rewriting ─────────────────────────────────────────

def _is_self_referential(ws_toml, info, mode="git"):
    """Check if a crate is defined inside the same workspace (self-referential).
    E.g. wynddex/Cargo.toml should not point wyndex to git://wynddex."""
    # Resolve composite dev modes to their ws_dep_mode for local_key selection
    effective_mode = DEV_MODE_MAP[mode][0] if mode in DEV_MODE_MAP else mode
    local_key = "zk_local" if effective_mode == "zk_git" else "local"
    local_path = info.get(local_key, "") or info.get("local", "")
    if not local_path or local_path == "N/A":
        return False
    # Normalize both paths (strips ./ prefix, resolves ../, etc.)
    local_norm = str(Path(local_path))
    ws_dir_norm = str(Path(ws_toml).parent)
    return local_norm.startswith(ws_dir_norm + "/") or local_norm.startswith(ws_dir_norm + "\\")


def _skip_self_ref(ws_toml, info, mode):
    """Only skip self-referential deps for git/dev modes (avoids circular git refs).
    For local/zk_local/stable, rel_path() computes correct internal paths."""
    if mode in ("local", "zk_local", "stable"):
        return False
    # dev/zk_dev and git/zk_git all skip self-refs
    return _is_self_referential(ws_toml, info, mode)


def _self_ref_internal_path(ws_toml, info, mode):
    """For self-referential crates, compute the correct internal workspace path.
    Returns None if the crate is not self-referential."""
    if not _is_self_referential(ws_toml, info, mode):
        return None
    effective_mode = DEV_MODE_MAP[mode][0] if mode in DEV_MODE_MAP else mode
    local_key = "zk_local" if effective_mode == "zk_git" else "local"
    local_path = info.get(local_key, "") or info.get("local", "")
    # Compute relative path from workspace root to the crate
    ws_dir = Path(ws_toml).parent
    return str(Path(os.path.relpath(Path(local_path), ws_dir)))


def _collect_member_pkg_names(ws_toml, data):
    """Collect package names of all workspace members (for collision detection)."""
    import glob as glob_mod
    ws_dir = (REPO_ROOT / ws_toml).parent
    members = data.get("workspace", {}).get("members", [])
    exclude = set()
    for exc in data.get("workspace", {}).get("exclude", []):
        for m in glob_mod.glob(str(ws_dir / exc)):
            exclude.add(Path(m).resolve())
    pkg_names = {}  # pkg_name -> member_path
    for pattern in members:
        for match in glob_mod.glob(str(ws_dir / pattern)):
            match_path = Path(match).resolve()
            if match_path in exclude:
                continue
            cargo = Path(match) / "Cargo.toml"
            if cargo.exists():
                try:
                    with open(cargo, "rb") as f:
                        mdata = tomllib.load(f)
                    name = mdata.get("package", {}).get("name")
                    if name:
                        pkg_names[name] = str(match)
                except Exception:
                    pass
    return pkg_names


def _patch_source_ready(key, info, ws_toml, source_mode, defaults):
    """True when this crate can supply the patch that matches a version line.

    patch-git without a full rev must not rewrite the dependency onto crates.io.
    The existing git or path line stays, and the patch pass reports the skip.
    """
    if not source_mode:
        return True
    return build_entry(key, info, source_mode, ws_toml, defaults=defaults) is not None


def rewrite_ws_deps(ws_toml, crates, mode, crate_filter=None, dry_run=False, defaults=None, require_source=None):
    """Update [workspace.dependencies] entries for matrix crates."""
    cargo_path = REPO_ROOT / ws_toml
    content = cargo_path.read_text()
    lines = content.split("\n")

    start, end = find_section(lines, "[workspace.dependencies]")
    # If no inline [workspace.dependencies] section, scan for expanded tables
    if start is None:
        WS_DEP_TABLE_PREFIX = "[workspace.dependencies."
        first = last = None
        for idx, line in enumerate(lines):
            if line.strip().startswith(WS_DEP_TABLE_PREFIX):
                if first is None:
                    first = idx
                last = idx
        if first is None:
            return False
        # Set range to cover from first expanded table to end of last one
        start = first - 1  # so start+1 = first
        # Find end: next non-ws-dep section header after last expanded table
        end = len(lines)
        for idx in range(last + 1, len(lines)):
            stripped = lines[idx].strip()
            if stripped.startswith("[") and not stripped.startswith(WS_DEP_TABLE_PREFIX):
                end = idx
                break

    known = matrix_pkg_names(crates)
    lookup = pkg_to_matrix(crates)
    consumer_name = str(Path(ws_toml).parent).split("/")[0]

    data, _ = read_cargo_toml(cargo_path)
    ws_deps_table = data.get("workspace", {}).get("dependencies", {})

    # Collect member package names for collision detection
    member_pkgs = _collect_member_pkg_names(ws_toml, data)

    changed = False
    preview = []
    new_lines = list(lines)

    # Regex for expanded table format: [workspace.dependencies.name]
    WS_DEP_TABLE_RE = re.compile(r'^\[workspace\.dependencies\.(.+)\]\s*$')

    # Pass 1: handle expanded table entries (must process in reverse to preserve indices)
    expanded_blocks = []  # [(start_line, end_line, name)]
    i = start + 1
    while i < end:
        m = WS_DEP_TABLE_RE.match(new_lines[i].strip())
        if m:
            dep_name = m.group(1)
            block_start = i
            # Find end of this table (next section header or blank line before next section)
            j = i + 1
            while j < end and not new_lines[j].strip().startswith("["):
                j += 1
            expanded_blocks.append((block_start, j, dep_name))
            i = j
        else:
            i += 1

    # Collect collapsed inline entries; remove expanded blocks in reverse
    collapsed_entries = []  # [(name, new_line)] in original order
    blocks_to_remove = []   # [(block_start, block_end)]
    for block_start, block_end, name in expanded_blocks:
        target_pkg = None
        if name in known:
            target_pkg = name
        else:
            spec = ws_deps_table.get(name, {})
            if isinstance(spec, dict):
                pkg_alias = spec.get("package")
                if pkg_alias and pkg_alias in known:
                    target_pkg = pkg_alias
        if not target_pkg:
            continue
        if crate_filter and target_pkg not in crate_filter:
            continue

        key, info = lookup[target_pkg]

        # Self-referential: never touch — these are internal workspace deps
        if _is_self_referential(ws_toml, info, mode):
            continue
        if not _patch_source_ready(key, info, ws_toml, require_source, defaults):
            continue

        pkg_name = info.get("package", key)
        if pkg_name in member_pkgs:
            dep_path = info.get("zk_local" if mode == "zk_local" else "local", "")
            member_path = member_pkgs[pkg_name]
            dep_resolved = str((REPO_ROOT / dep_path).resolve()) if dep_path else ""
            if dep_resolved != member_path:
                print(f"  [WARN] Skipping {name}: path dep '{pkg_name}' would collide "
                      f"with workspace member at {member_path}")
                continue

        existing_spec = ws_deps_table.get(name, {})
        new_val = build_ws_dep_entry(key, info, mode, ws_toml, existing_spec, consumer_name=consumer_name, defaults=defaults, dep_name=name)
        if new_val is None:
            continue

        new_line = f"{name} = {new_val}"
        collapsed_entries.append(new_line)
        blocks_to_remove.append((block_start, block_end))
        if dry_run:
            preview.append(new_line)

    if collapsed_entries:
        # Remove expanded blocks in reverse order to preserve indices
        for block_start, block_end in reversed(blocks_to_remove):
            new_lines[block_start:block_end] = []
            end -= (block_end - block_start)

        # Find or create [workspace.dependencies] section for inline entries
        ws_dep_header_idx = None
        for idx, line in enumerate(new_lines):
            if line.strip() == "[workspace.dependencies]":
                ws_dep_header_idx = idx
                break

        if ws_dep_header_idx is None:
            # Insert [workspace.dependencies] section before the first remaining
            # expanded table or at `end` (after last removed block)
            insert_at = end
            for idx, line in enumerate(new_lines):
                if line.strip().startswith("[workspace.dependencies."):
                    insert_at = idx
                    break
            new_lines.insert(insert_at, "")
            new_lines.insert(insert_at, "[workspace.dependencies]")
            ws_dep_header_idx = insert_at
            end += 2

        # Insert collapsed entries right after [workspace.dependencies] header
        for i, entry in enumerate(collapsed_entries):
            new_lines.insert(ws_dep_header_idx + 1 + i, entry)
            end += 1

        changed = True

    # Pass 2: handle inline entries (name = { ... })
    for i in range(start + 1, end):
        stripped = new_lines[i].strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue

        name = stripped.split("=", 1)[0].strip()

        target_pkg = None
        if name in known:
            target_pkg = name
        else:
            spec = ws_deps_table.get(name, {})
            if isinstance(spec, dict):
                pkg_alias = spec.get("package")
                if pkg_alias and pkg_alias in known:
                    target_pkg = pkg_alias

        if not target_pkg:
            continue
        if crate_filter and target_pkg not in crate_filter:
            continue

        key, info = lookup[target_pkg]

        # Self-referential: never touch — these are internal workspace deps
        if _is_self_referential(ws_toml, info, mode):
            continue
        if not _patch_source_ready(key, info, ws_toml, require_source, defaults):
            continue

        # Detect package name collision: external path dep vs workspace member
        pkg_name = info.get("package", key)
        if pkg_name in member_pkgs:
            dep_path = info.get("zk_local" if mode == "zk_local" else "local", "")
            member_path = member_pkgs[pkg_name]
            dep_resolved = str((REPO_ROOT / dep_path).resolve()) if dep_path else ""
            if dep_resolved != member_path:
                print(f"  [WARN] Skipping {name}: path dep '{pkg_name}' would collide "
                      f"with workspace member at {member_path}")
                continue

        existing_spec = ws_deps_table.get(name, {})
        new_val = build_ws_dep_entry(key, info, mode, ws_toml, existing_spec, consumer_name=consumer_name, defaults=defaults, dep_name=name)
        if new_val is None:
            continue

        new_line = f"{name} = {new_val}"
        if new_line != stripped:
            new_lines[i] = new_line
            changed = True
            if dry_run:
                preview.append(new_line)

    if changed:
        new_content = "\n".join(new_lines)
        if dry_run:
            print(f"  [DRY RUN] Would modify ws-deps in {ws_toml}")
            for line in preview:
                print(f"    {line}")
        else:
            cargo_path.write_text(new_content)
            if not validate_toml(cargo_path):
                cargo_path.write_text(content)
                print(f"  REVERTED ws-deps in {ws_toml} (invalid TOML)")
                return False
            cargo_sort(cargo_path)
            print(f"  Modified ws-deps in {ws_toml}")
        return True
    return False


# ── Member Dependencies Rewriting ────────────────────────────────────────────

def _parse_dep_line(line):
    """Parse a Cargo.toml dep line into (name, rest_of_line, indent).
    Returns None if line is not a dep assignment."""
    stripped = line.strip()
    if not stripped or stripped.startswith("#") or stripped.startswith("[") or "=" not in stripped:
        return None
    # Avoid matching table headers like [dependencies.foo]
    if stripped.startswith("["):
        return None
    name = stripped.split("=", 1)[0].strip()
    indent = line[:len(line) - len(line.lstrip())]
    return name, indent


def _is_in_dep_section(lines, line_idx):
    """Check if a line is inside a [dependencies], [dev-dependencies],
    [build-dependencies], or [target.*.dependencies] section.
    Returns the section name string if found, or False."""
    # Walk backwards to find the last section header
    for i in range(line_idx - 1, -1, -1):
        stripped = lines[i].strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]").strip()
            # Match dependency sections. workspace.dependencies contains the
            # substring ".dependencies" and must not be rewritten here; the
            # ws-deps pass owns that table.
            if section in ("dependencies", "dev-dependencies", "build-dependencies"):
                return section
            if section.startswith("target.") and (
                section.endswith(".dependencies")
                or section.endswith(".dev-dependencies")
                or section.endswith(".build-dependencies")
            ):
                return section
            return False
    return False


def _resolve_dep_pkg(name, data, known):
    """Resolve a dep name to its published matrix package name, or None."""
    if name in known:
        return name
    # Check package = "..." alias in all dep sections
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        spec = data.get(section, {}).get(name, None)
        if spec is not None:
            if isinstance(spec, dict):
                pkg_alias = spec.get("package")
                if pkg_alias and pkg_alias in known:
                    return pkg_alias
            return None
    for _tk, td in data.get("target", {}).items():
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            spec = td.get(section, {}).get(name, None)
            if spec is not None:
                if isinstance(spec, dict):
                    pkg_alias = spec.get("package")
                    if pkg_alias and pkg_alias in known:
                        return pkg_alias
                return None
    return None


def _find_existing_spec(name, data, section_hint=None):
    """Find the existing parsed spec for a dep name.

    If section_hint is provided, look in that section first to get the
    correct spec (avoids copying optional=true from [dependencies] into
    [dev-dependencies] rewrites).
    """
    if section_hint:
        # Normalize target.*.xxx sections
        parts = section_hint.split(".")
        if parts[0] == "target" and len(parts) >= 3:
            target_key = parts[1]
            sub_section = ".".join(parts[2:])
            s = data.get("target", {}).get(target_key, {}).get(sub_section, {}).get(name)
            if s is not None:
                return s
        else:
            s = data.get(section_hint, {}).get(name)
            if s is not None:
                return s
    # Fallback: search all sections
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        s = data.get(section, {}).get(name)
        if s is not None:
            return s
    for _tk, td in data.get("target", {}).items():
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            s = td.get(section, {}).get(name)
            if s is not None:
                return s
    return None


def rewrite_member_file(member_toml, crates, mode, crate_filter, ws_deps, ws_toml=None, dry_run=False, defaults=None, require_source=None):
    """Rewrite direct matrix crate deps in a single member Cargo.toml.

    Skips deps that use workspace = true (those are handled by ws-deps target).
    ws_toml: workspace root Cargo.toml path (for self-referential detection).
    """
    consumer_name = ws_toml.split("/")[0] if ws_toml and "/" in ws_toml else None
    try:
        content = member_toml.read_text()
    except Exception:
        return False

    # Parse to identify workspace=true deps (skip those)
    try:
        with open(member_toml, "rb") as f:
            data = tomllib.load(f)
    except Exception:
        return False

    known = matrix_pkg_names(crates)
    lookup = pkg_to_matrix(crates)
    rel_toml = os.path.relpath(member_toml, REPO_ROOT)

    # Collect dep names that use workspace = true
    ws_inherited = set()
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for name, spec in data.get(section, {}).items():
            if isinstance(spec, dict) and spec.get("workspace"):
                ws_inherited.add(name)

    # Also check target-specific deps
    for _target_key, target_data in data.get("target", {}).items():
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, spec in target_data.get(section, {}).items():
                if isinstance(spec, dict) and spec.get("workspace"):
                    ws_inherited.add(name)

    lines = content.split("\n")
    changed = False

    # ── Pass 1: Inline deps (name = { ... } on one line) ────────────────
    skip_until = -1  # skip continuation lines from multiline replacements
    for i, line in enumerate(lines):
        if i < skip_until:
            continue

        parsed = _parse_dep_line(line)
        if parsed is None:
            continue

        name, indent = parsed

        if name in ws_inherited:
            continue

        target_pkg = _resolve_dep_pkg(name, data, known)
        if not target_pkg:
            continue
        dep_section = _is_in_dep_section(lines, i)
        if not dep_section:
            continue
        if crate_filter and target_pkg not in crate_filter:
            continue

        key, info = lookup[target_pkg]

        # Self-referential: never touch — these are internal workspace deps
        if ws_toml and _is_self_referential(ws_toml, info, mode):
            i += 1
            continue
        if not _patch_source_ready(key, info, rel_toml, require_source, defaults):
            i += 1
            continue

        existing_spec = _find_existing_spec(name, data, section_hint=dep_section)

        new_val = build_ws_dep_entry(key, info, mode, rel_toml, existing_spec or {}, consumer_name=consumer_name, defaults=defaults, dep_name=name)
        # dev-dependencies cannot be optional — strip if present
        if new_val and "dev-dependencies" in dep_section:
            new_val = re.sub(r',\s*optional\s*=\s*true', '', new_val)
            new_val = re.sub(r'optional\s*=\s*true,\s*', '', new_val)
        if new_val is None:
            continue

        new_line = f"{indent}{name} = {new_val}"

        # Detect multiline value: if line has unclosed { or [, find the end
        val_part = line.split("=", 1)[1] if "=" in line else ""
        open_braces = val_part.count("{") - val_part.count("}")
        open_brackets = val_part.count("[") - val_part.count("]")
        if open_braces > 0 or open_brackets > 0:
            # Find end of multiline value
            end = i + 1
            while end < len(lines):
                val_part += lines[end]
                open_braces += lines[end].count("{") - lines[end].count("}")
                open_brackets += lines[end].count("[") - lines[end].count("]")
                end += 1
                if open_braces <= 0 and open_brackets <= 0:
                    break
            # Replace lines[i:end] with single line
            lines[i:end] = [new_line]
            skip_until = i + 1
            changed = True
            if dry_run:
                print(f"    {new_line}")
        elif new_line != line:
            lines[i] = new_line
            changed = True
            if dry_run:
                print(f"    {new_line}")

    # ── Pass 2: Expanded table sections ([dependencies.name]) ───────────
    # Pattern: [dependencies.foo] or [dev-dependencies.foo] etc.
    DEP_TABLE_RE = re.compile(
        r'^\[((?:dev-|build-)?dependencies)\.(.+)\]\s*$'
    )
    # Also handle [target.'cfg(...)'.dependencies.foo]
    TARGET_DEP_TABLE_RE = re.compile(
        r'^\[target\.[^]]+\.((?:dev-|build-)?dependencies)\.(.+)\]\s*$'
    )

    i = 0
    while i < len(lines):
        stripped = lines[i].strip()
        m = DEP_TABLE_RE.match(stripped) or TARGET_DEP_TABLE_RE.match(stripped)
        if not m:
            i += 1
            continue

        dep_section = m.group(1)
        dep_name = m.group(2).strip().strip('"').strip("'")

        if dep_name in ws_inherited:
            i += 1
            continue

        target_pkg = _resolve_dep_pkg(dep_name, data, known)
        if not target_pkg:
            i += 1
            continue
        if crate_filter and target_pkg not in crate_filter:
            i += 1
            continue

        key, info = lookup[target_pkg]

        if not _patch_source_ready(key, info, rel_toml, require_source, defaults):
            i += 1
            continue
        # Self-referential: never touch — these are internal workspace deps
        if ws_toml and _is_self_referential(ws_toml, info, mode):
            # Skip past entire expanded table section
            j = i + 1
            while j < len(lines):
                s = lines[j].strip()
                if s.startswith("[") and s.endswith("]"):
                    break
                j += 1
            i = j
            continue

        existing_spec = _find_existing_spec(dep_name, data, section_hint=dep_section)

        new_val = build_ws_dep_entry(key, info, mode, rel_toml, existing_spec or {}, consumer_name=consumer_name, defaults=defaults, dep_name=dep_name)
        # dev-dependencies cannot be optional — strip if present
        if new_val and "dev-dependencies" in dep_section:
            new_val = re.sub(r',\s*optional\s*=\s*true', '', new_val)
            new_val = re.sub(r'optional\s*=\s*true,\s*', '', new_val)
        if new_val is None:
            i += 1
            continue

        # Find end of this table section (next [header] or EOF)
        section_start = i
        j = i + 1
        while j < len(lines):
            s = lines[j].strip()
            if s.startswith("[") and s.endswith("]"):
                break
            j += 1
        section_end = j

        # Replace entire section with inline form
        inline_line = f"{dep_name} = {new_val}"

        # We need to figure out which parent section this belongs to
        # so we can put the inline dep in the right place.
        # For now, just replace the expanded section with the inline form.
        old_section = "\n".join(lines[section_start:section_end])
        new_section_line = inline_line

        if old_section.strip() != new_section_line.strip():
            lines[section_start:section_end] = [new_section_line]
            changed = True
            if dry_run:
                print(f"    {new_section_line}")
            # Don't advance i since we collapsed lines
        else:
            i = section_end
            continue
        i += 1

    if changed:
        new_content = "\n".join(lines)
        if dry_run:
            print(f"    [DRY RUN] {rel_toml}")
        else:
            member_toml.write_text(new_content)
            if not validate_toml(member_toml):
                member_toml.write_text(content)
                print(f"    REVERTED {rel_toml} (invalid TOML)")
                return False
            cargo_sort(member_toml)
            print(f"    {rel_toml}")
        return True
    return False


def rewrite_members(project, crates, mode, crate_filter=None, dry_run=False, defaults=None, require_source=None):
    """Rewrite direct matrix deps in all member Cargo.tomls of a project."""
    ws_toml = project.rel_path + "/Cargo.toml"

    # Load workspace deps for identifying inherited ones
    try:
        data, _ = read_cargo_toml(project.cargo_toml)
    except Exception as e:
        print(f"  SKIP {project.name}: invalid TOML ({e})")
        return
    ws_deps = extract_workspace_deps(data)

    # Also rewrite the root Cargo.toml's own [dependencies] (if it has any outside workspace.deps)
    tomls_to_check = [project.cargo_toml] + list(project.member_tomls)

    count = 0
    for member_toml in tomls_to_check:
        if rewrite_member_file(member_toml, crates, mode, crate_filter, ws_deps, ws_toml=ws_toml, dry_run=dry_run, defaults=defaults, require_source=require_source):
            count += 1

    if count > 0:
        print(f"  Members: {count} file(s) in {project.name}")
    else:
        print(f"  Members: no changes in {project.name}")

    return count > 0


# ── patch-local closure ──────────────────────────────────────────────────────

def _effective_dep(name, raw, ws_deps):
    """Dependency spec Cargo will resolve, and whether its path is workspace-relative."""
    if isinstance(raw, str):
        return {"version": raw}, False
    if not isinstance(raw, dict):
        return None, False
    if raw.get("workspace") and isinstance(ws_deps, dict) and name in ws_deps:
        base = ws_deps[name]
        if isinstance(base, str):
            spec = {"version": base}
        elif isinstance(base, dict):
            spec = dict(base)
        else:
            return None, False
        if raw.get("package"):
            spec["package"] = raw["package"]
        return spec, True
    return dict(raw), False


def _workspace_context(pkg_dir: Path):
    """Workspace root and its [workspace.dependencies] for a package directory."""
    root = REPO_ROOT.resolve()
    current = pkg_dir.resolve()
    while True:
        cargo = current / "Cargo.toml"
        if cargo.is_file():
            try:
                data, _ = read_cargo_toml(cargo)
            except Exception:
                data = {}
            workspace = data.get("workspace") or {}
            members = workspace.get("members") or []
            if members and _dir_is_member(current, members, workspace.get("exclude") or [], pkg_dir.resolve()):
                return current, workspace.get("dependencies") or {}
        if current == root or current.parent == current:
            break
        current = current.parent
    return None, {}


def _dir_is_member(ws_dir: Path, members, exclude, pkg_dir: Path) -> bool:
    import glob as glob_mod

    pkg_dir = pkg_dir.resolve()
    ws_dir = ws_dir.resolve()
    excluded = set()
    for pattern in exclude:
        for match in glob_mod.glob(str(ws_dir / pattern)):
            excluded.add(Path(match).resolve())
    if pkg_dir in excluded:
        return False
    if pkg_dir == ws_dir and "." in members:
        return True
    for pattern in members:
        for match in glob_mod.glob(str(ws_dir / pattern)):
            if Path(match).resolve() == pkg_dir:
                return True
    return False


def _member_dirs(ws_dir: Path, data: dict) -> list:
    import glob as glob_mod

    found = []
    excluded = set()
    for pattern in data.get("workspace", {}).get("exclude") or []:
        for match in glob_mod.glob(str(ws_dir / pattern)):
            excluded.add(Path(match).resolve())
    for pattern in data.get("workspace", {}).get("members") or []:
        for match in glob_mod.glob(str(ws_dir / pattern)):
            path = Path(match).resolve()
            if path in excluded or not (path / "Cargo.toml").is_file():
                continue
            found.append(path)
    return found


def _inside_repo(path: Path):
    try:
        path.resolve().relative_to(REPO_ROOT.resolve())
    except ValueError:
        return None
    if (path / "Cargo.toml").is_file():
        return path.resolve()
    return None


def _in_this_workspace(pkg_dir: Path, ws_toml: str) -> bool:
    ws_dir = (REPO_ROOT / ws_toml).parent.resolve()
    try:
        pkg_dir.resolve().relative_to(ws_dir)
    except ValueError:
        return False
    return True


def _queue_registry_patch(spec, name, lookup, crates, ws_toml, defaults, seen_patch, to_add, queue, pkg_dir):
    """A crates.io edge for a matrix crate needs a local [patch.crates-io] entry.

    A member of this workspace is not patched from its own manifests. An
    outside crate that version-depends on that member still needs the patch,
    because the member does not satisfy a crates.io edge by itself.
    """
    if spec.get("path") or spec.get("git") or not spec.get("version"):
        return
    pkg = spec.get("package") or name
    hit = lookup.get(pkg) or lookup.get(name)
    if not hit:
        return
    key, info = hit
    if is_repo_only(info):
        return
    if _is_self_referential(ws_toml, info, "registry") and _in_this_workspace(pkg_dir, ws_toml):
        return
    mapped = _patch_pkg_name(pkg, lookup) or _patch_pkg_name(name, lookup)
    if not mapped:
        return
    published, _key, _info = mapped
    if published not in seen_patch:
        val = build_entry(key, info, "local", ws_toml, defaults=defaults)
        if val:
            to_add.append(f"{published} = {val}")
            seen_patch.add(published)
    local = info.get("local")
    if local and local != "N/A":
        dest = _inside_repo(REPO_ROOT / local)
        if dest is not None:
            queue.append((dest, False, False))


def _closure_patches(ws_toml, crates, defaults):
    """Local patches for registry edges reached through this workspace's path graph.

    Also returns git URLs still named by those manifests. A URL that survives
    here still needs its `[patch.'<url>']` section.
    """
    cargo_path = REPO_ROOT / ws_toml
    try:
        data, _ = read_cargo_toml(cargo_path)
    except Exception:
        return [], set()
    text = cargo_path.read_text()
    lines = text.split("\n")
    lookup = pkg_to_matrix(crates)
    ws_dir = cargo_path.parent.resolve()
    seen = set()
    seen_patch = set()
    urls = set()
    to_add = []
    queue = [(ws_dir, True, True)]
    for member in _member_dirs(ws_dir, data):
        queue.append((member, True, False))

    start, end = find_section(lines, "[patch.crates-io]")
    if start is not None:
        for line in lines[start + 1 : end]:
            name = line.split("=", 1)[0].strip() if "=" in line else ""
            if name and not line.strip().startswith("#"):
                seen_patch.add(name)
            match = re.search(r'path\s*=\s*"([^"]+)"', line)
            if not match:
                continue
            dest = _inside_repo(ws_dir / match.group(1))
            if dest is not None:
                queue.append((dest, False, False))

    while queue:
        pkg_dir, include_dev, scan_workspace_table = queue.pop()
        pkg_dir = pkg_dir.resolve()
        if pkg_dir in seen:
            continue
        cargo = pkg_dir / "Cargo.toml"
        if not cargo.is_file() or _inside_repo(pkg_dir) is None:
            continue
        seen.add(pkg_dir)
        try:
            manifest_text = cargo.read_text()
            manifest, _ = read_cargo_toml(cargo)
        except Exception:
            continue
        urls.update(git_dependency_urls(manifest_text))
        _ws_root, ws_deps = _workspace_context(pkg_dir)

        def consider(table, force_workspace):
            if not isinstance(table, dict):
                return
            for dep_name, raw in table.items():
                spec, from_workspace = _effective_dep(dep_name, raw, ws_deps)
                if spec is None:
                    continue
                if force_workspace:
                    from_workspace = True
                source_path = spec.get("path")
                if source_path:
                    base = _ws_root if from_workspace and _ws_root is not None else pkg_dir
                    dest = _inside_repo(base / source_path)
                    if dest is not None:
                        queue.append((dest, False, False))
                    continue
                _queue_registry_patch(
                    spec, dep_name, lookup, crates, ws_toml, defaults,
                    seen_patch, to_add, queue, pkg_dir,
                )

        for section in ("dependencies", "build-dependencies"):
            consider(manifest.get(section), False)
        if include_dev:
            consider(manifest.get("dev-dependencies"), False)
        if scan_workspace_table:
            consider(manifest.get("workspace", {}).get("dependencies"), True)
        target = manifest.get("target")
        if isinstance(target, dict):
            for cfg in target.values():
                if not isinstance(cfg, dict):
                    continue
                for section in ("dependencies", "build-dependencies"):
                    consider(cfg.get(section), False)
                if include_dev:
                    consider(cfg.get("dev-dependencies"), False)
    return to_add, urls


def unify_local_patches(ws_toml, crates, defaults, dry_run):
    """Fill `[patch.crates-io]` for the path closure and drop unused git-url sections."""
    cargo_path = REPO_ROOT / ws_toml
    added, urls = _closure_patches(ws_toml, crates, defaults)
    if added:
        content = cargo_path.read_text()
        lines = content.split("\n")
        start, end = find_section(lines, "[patch.crates-io]")
        if start is None:
            if lines and lines[-1] != "":
                lines.append("")
            lines.extend(["[patch.crates-io]", ""])
            lines.extend(added)
        else:
            lines[end:end] = added
        new_content = "\n".join(lines)
        if dry_run:
            print(f"  [DRY RUN] Would extend [patch.crates-io] in {ws_toml}")
            for line in added:
                print(f"    {line}")
        else:
            cargo_path.write_text(new_content)
            if not validate_toml(cargo_path):
                cargo_path.write_text(content)
                print(f"  REVERTED closure patches in {ws_toml} (invalid TOML)")
            else:
                cargo_sort(cargo_path)
                print(f"  Extended [patch.crates-io] in {ws_toml} ({len(added)} entries)")

    content = cargo_path.read_text()
    new_content, dropped = drop_unreferenced_git_patches(content, urls)
    if not dropped:
        return
    if dry_run:
        print(f"  [DRY RUN] Would drop git-url patches from {ws_toml}")
        for url in dropped:
            print(f"    {url}")
        return
    cargo_path.write_text(new_content)
    if not validate_toml(cargo_path):
        cargo_path.write_text(content)
        print(f"  REVERTED git-url patch removal in {ws_toml} (invalid TOML)")
        return
    cargo_sort(cargo_path)
    print(f"  Dropped {len(dropped)} git-url patch section(s) from {ws_toml}")


# ── Commands ─────────────────────────────────────────────────────────────────

TARGET_DESC = {
    "patches": "[patch.crates-io]",
    "ws-deps": "[workspace.dependencies]",
    "members": "member Cargo.tomls",
    "both": "patches + ws-deps",
    "all": "patches + ws-deps + members",
}


def cmd_switch(args):
    """Switch fork deps to the specified mode."""
    crates = load_matrix()
    defaults = load_defaults()
    projects = resolve_projects(args.workspace, all_projects=(args.target in ("members", "all")))
    crate_filter = resolve_crate_filter(args.crate, crates)
    target = args.target
    mode = args.mode

    # Decompose composite modes into the dependency-line mode and the patch mode.
    is_dev = mode in DEV_MODE_MAP
    is_patch = mode in PATCH_MODE_MAP
    if is_dev:
        ws_dep_mode, patch_mode, _ = DEV_MODE_MAP[mode]
    elif is_patch:
        ws_dep_mode, patch_mode = PATCH_MODE_MAP[mode]
    else:
        ws_dep_mode = patch_mode = mode

    filter_desc = ""
    if args.workspace:
        filter_desc += f" workspace={','.join(args.workspace)}"
    if args.crate:
        filter_desc += f" crate={','.join(args.crate)}"

    if is_dev:
        print(f"Switching to mode: {mode} (ws-deps/members={ws_dep_mode}, patches={patch_mode}, git-url-patches=local) | target: {TARGET_DESC[target]}{filter_desc}")
    elif is_patch:
        print(f"Switching to mode: {mode} (ws-deps/members={ws_dep_mode}, patches={patch_mode}, one [patch.crates-io]) | target: {TARGET_DESC[target]}{filter_desc}")
    else:
        print(f"Switching to mode: {mode} | target: {TARGET_DESC[target]}{filter_desc}")
    if args.dry_run:
        print("  [DRY RUN]")
    print()

    for proj in projects:
        ws_toml = proj.rel_path + "/Cargo.toml"

        if not (REPO_ROOT / ws_toml).exists():
            print(f"  SKIP {ws_toml}: file not found")
            continue

        if target in ("patches", "both", "all"):
            rewrite_patches(
                ws_toml, crates, patch_mode, crate_filter, args.dry_run,
                defaults=defaults, ensure=is_patch,
            )

        if target in ("ws-deps", "both", "all"):
            rewrite_ws_deps(
                ws_toml, crates, ws_dep_mode, crate_filter, args.dry_run,
                defaults=defaults, require_source=patch_mode if is_patch else None,
            )

        if target in ("members", "all"):
            rewrite_members(
                proj, crates, ws_dep_mode, crate_filter, args.dry_run,
                defaults=defaults, require_source=patch_mode if is_patch else None,
            )

        # Git URL patches: only on workspace roots (not standalone member crates)
        if target in ("patches", "both", "all") and proj.is_workspace:
            if is_dev:
                rewrite_git_url_patches(ws_toml, crates, mode, crate_filter, args.dry_run, defaults=defaults)
            elif mode in ("local", "zk_local", "stable"):
                remove_git_url_patches(ws_toml, crates, args.dry_run, defaults=defaults)
            # patch-git keeps an existing [patch.'<git-url>']. A crates.io→git
            # replacement cannot be chained, and a git edge inside a patched
            # crate still needs that section. patch-local retires it below,
            # once the path closure no longer names the URL.

    if is_patch and patch_mode == "local":
        for proj in projects:
            ws_toml = proj.rel_path + "/Cargo.toml"
            if not (REPO_ROOT / ws_toml).exists():
                continue
            unify_local_patches(ws_toml, crates, defaults, args.dry_run)

    # Persist active mode (skip on dry-run or partial crate/workspace filters)
    if not args.dry_run and not args.crate and not args.workspace:
        save_active_mode(mode)


def cmd_status(args):
    """Show current dependency status for each workspace."""
    crates = load_matrix()
    known = matrix_pkg_names(crates)
    projects = resolve_projects(args.workspace if hasattr(args, "workspace") else None, all_projects=True)

    print("Dependency status:\n")

    for proj in projects:
        ws_toml = proj.rel_path + "/Cargo.toml"
        ws_path = REPO_ROOT / ws_toml
        if not ws_path.exists():
            print(f"  {ws_toml}: file not found")
            continue

        content = ws_path.read_text()
        lines = content.split("\n")

        print(f"  {ws_toml}")

        # Patch section
        start, end = find_section(lines, "[patch.crates-io]")
        if start is not None:
            total = matrix_count = 0
            modes = set()
            for line in lines[start + 1 : end]:
                stripped = line.strip()
                if not stripped or stripped.startswith("#") or "=" not in stripped:
                    continue
                total += 1
                name = stripped.split("=", 1)[0].strip()
                if name in known:
                    matrix_count += 1
                if "path =" in stripped:
                    modes.add("local")
                elif "git =" in stripped:
                    modes.add("git")
            mode_str = ", ".join(sorted(modes)) or "stable/empty"
            print(f"    patches: {total} entries ({matrix_count} matrix, {total - matrix_count} static) [{mode_str}]")
        else:
            print(f"    patches: (none)")

        # Workspace deps section
        ws_start, ws_end = find_section(lines, "[workspace.dependencies]")
        if ws_start is not None:
            path_count = git_count = ver_count = matrix_ws = 0
            for line in lines[ws_start + 1 : ws_end]:
                stripped = line.strip()
                if not stripped or stripped.startswith("#") or "=" not in stripped:
                    continue
                name = stripped.split("=", 1)[0].strip()
                if name in known:
                    matrix_ws += 1
                if "path =" in stripped:
                    path_count += 1
                elif "git =" in stripped:
                    git_count += 1
                else:
                    ver_count += 1
            print(f"    ws-deps: {path_count} path, {git_count} git, {ver_count} version ({matrix_ws} matrix)")
        else:
            print(f"    ws-deps: (none)")

        # Member direct deps
        member_path = member_git = member_ver = member_total = 0
        for mt in proj.member_tomls:
            try:
                with open(mt, "rb") as f:
                    md = tomllib.load(f)
            except Exception:
                continue
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                for name, spec in md.get(section, {}).items():
                    if isinstance(spec, dict) and spec.get("workspace"):
                        continue  # skip inherited
                    if name not in known:
                        # Check package alias
                        if isinstance(spec, dict):
                            pkg = spec.get("package", "")
                            if pkg not in known:
                                continue
                        else:
                            continue
                    member_total += 1
                    if isinstance(spec, dict):
                        if spec.get("path"):
                            member_path += 1
                        elif spec.get("git"):
                            member_git += 1
                        else:
                            member_ver += 1
                    elif isinstance(spec, str):
                        member_ver += 1

        if member_total > 0:
            print(f"    members: {member_path} path, {member_git} git, {member_ver} version ({member_total} matrix direct)")
        else:
            print(f"    members: (no direct matrix deps)")

        print()


# ── Branch Ensure ─────────────────────────────────────────────────────────

def _git(repo, *cmd, timeout=60):
    """Run a git command in repo, return (ok, stdout, stderr)."""
    r = subprocess.run(
        ["git", "-C", str(repo)] + list(cmd),
        capture_output=True, text=True, timeout=timeout,
    )
    return r.returncode == 0, r.stdout.strip(), r.stderr.strip()


def root_for(local_path):
    """Resolve git repo root from a matrix local path."""
    resolved = REPO_ROOT / local_path.lstrip("./")
    if not resolved.exists():
        return None
    ok, root, _ = _git(resolved, "rev-parse", "--show-toplevel", timeout=5)
    return Path(root) if ok else None


def _find_remote(repo, url):
    """Find the remote name in repo that matches url."""
    ok, out, _ = _git(repo, "remote", "-v", timeout=5)
    if not ok:
        return None

    def norm(u):
        u = u.rstrip("/")
        if u.endswith(".git"):
            u = u[:-4]
        if u.startswith("git@github.com:"):
            u = "https://github.com/" + u[len("git@github.com:"):]
        return u.lower()

    target = norm(url)
    for line in out.split("\n"):
        parts = line.split()
        if len(parts) >= 2 and norm(parts[1]) == target:
            return parts[0]
    return None


def _ls_remote_branches(url):
    """Return set of branch names on remote, or None on error."""
    try:
        r = subprocess.run(
            ["git", "ls-remote", "--heads", url],
            capture_output=True, text=True, timeout=30,
        )
        if r.returncode != 0:
            return None
        branches = set()
        for line in r.stdout.strip().split("\n"):
            if line and "\t" in line:
                ref = line.split("\t")[1]
                if ref.startswith("refs/heads/"):
                    branches.add(ref[len("refs/heads/"):])
        return branches
    except Exception:
        return None


def cmd_ensure_branches(args):
    """Create missing branches on remotes for the specified mode(s)."""
    crates = load_matrix()
    defaults = load_defaults()
    mode = args.mode

    # Collect needed (url, branch) grouped by url, with local paths for repo resolution
    # url_map: { url: { "branches": {branch: field}, "locals": [path], "base_branches": set } }
    url_map = {}
    for key, info in sorted(crates.items()):
        local = info.get("local", "N/A")

        if mode in ("git", "all"):
            url, branch = resolve_git_source(info, "git", defaults)
            if url and branch:
                e = url_map.setdefault(url, {"branches": {}, "locals": [], "base_branches": set()})
                e["branches"].setdefault(branch, "git")
                if local and local != "N/A":
                    e["locals"].append(local)

        if mode in ("zk_git", "all"):
            zk_url, zk_branch = resolve_git_source(info, "zk_git", defaults)
            if zk_url and zk_branch:
                e = url_map.setdefault(zk_url, {"branches": {}, "locals": [], "base_branches": set()})
                e["branches"].setdefault(zk_branch, "zk_git")
                if local and local != "N/A":
                    e["locals"].append(local)
                # The corresponding git branch is a good base for zk branches
                git_url, git_branch = resolve_git_source(info, "git", defaults)
                if git_branch:
                    e["base_branches"].add(git_branch)

    created = skipped = 0
    for url in sorted(url_map):
        info = url_map[url]
        remote_branches = _ls_remote_branches(url)
        if remote_branches is None:
            print(f"  SKIP {url}: ls-remote failed")
            skipped += len(info["branches"])
            continue

        missing = {b: f for b, f in info["branches"].items() if b not in remote_branches}
        if not missing:
            continue

        # Find local repo
        repo = None
        for lp in info["locals"]:
            repo = root_for(lp)
            if repo:
                break
        if not repo:
            print(f"  SKIP {url}: no local clone")
            skipped += len(missing)
            continue

        remote_name = _find_remote(repo, url)
        if not remote_name:
            print(f"  SKIP {url}: no matching remote in {repo.name}")
            skipped += len(missing)
            continue

        # Fetch latest
        _git(repo, "fetch", remote_name)

        for branch in sorted(missing):
            # Pick base: prefer the git.branch (for zk), then main/master/first available
            base = None
            if args.base:
                base = args.base
            else:
                for candidate in list(info["base_branches"]) + ["main", "master"]:
                    if candidate in remote_branches:
                        base = candidate
                        break
                if not base and remote_branches:
                    base = sorted(remote_branches)[0]

            if not base:
                print(f"  SKIP {url} '{branch}': no base branch available")
                skipped += 1
                continue

            if args.dry_run:
                print(f"  [DRY RUN] {url}  {base} -> {branch}")
                continue

            # Create branch from remote base and push
            ref = f"refs/remotes/{remote_name}/{base}"
            ok, _, err = _git(repo, "push", remote_name, f"{ref}:refs/heads/{branch}")
            if ok:
                print(f"  CREATED {url}  {base} -> {branch}")
                created += 1
            else:
                print(f"  FAILED  {url}  {branch}: {err}")
                skipped += 1

    print(f"\nDone: {created} created, {skipped} skipped")


def main():
    parser = argparse.ArgumentParser(
        description="Switch fork dependency modes in workspace Cargo.tomls"
    )
    sub = parser.add_subparsers(dest="command")

    # Switch command
    sw = sub.add_parser("switch", help="Switch dependency mode")
    sw.add_argument("mode", choices=["stable", "git", "local", "zk_git", "zk_local", "dev", "zk_dev"])
    sw.add_argument("--dry-run", action="store_true", help="Preview without writing")
    sw.add_argument(
        "--workspace", action="append",
        help="Target workspace(s) by name or path (repeatable)",
    )
    sw.add_argument(
        "--crate", action="append",
        help="Target crate(s) by name or matrix key (repeatable)",
    )
    sw.add_argument(
        "--target", choices=["patches", "ws-deps", "members", "both", "all"],
        default="all",
        help="Which sections to modify (default: all)",
    )
    sw.set_defaults(func=cmd_switch)

    # Ensure-branches command
    eb = sub.add_parser("ensure-branches",
                        help="Create missing branches on remotes for a mode")
    eb.add_argument("mode", choices=["git", "zk_git", "all"])
    eb.add_argument("--dry-run", action="store_true", help="Preview without pushing")
    eb.add_argument("--base", help="Override base branch (default: auto-detect)")
    eb.set_defaults(func=cmd_ensure_branches)

    # Status command
    st = sub.add_parser("status", help="Show current dependency status")
    st.add_argument(
        "--workspace", action="append",
        help="Target workspace(s) by name or path (repeatable)",
    )
    st.set_defaults(func=cmd_status)

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(1)

    args.func(args)


if __name__ == "__main__":
    main()
