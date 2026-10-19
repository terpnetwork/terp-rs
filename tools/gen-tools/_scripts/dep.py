#!/usr/bin/env python3
"""
Stable dependency control plane CLI (Phase 0/1).

Single entrypoint for matrix integrity + mode switching + verification.

Usage:
    python3 _scripts/dep.py config
    python3 _scripts/dep.py validate
    python3 _scripts/dep.py heal [--dry-run]
    python3 _scripts/dep.py status
    python3 _scripts/dep.py switch <mode> [options]
    python3 _scripts/dep.py pin-revs --workspace PATH [--dry-run]
    python3 _scripts/dep.py verify [--mode MODE] [--workspace PATH]
    python3 _scripts/dep.py ensure-branches <mode> [--force]
    python3 _scripts/dep.py branch-check [--mode MODE] [--refresh] [--all]
    python3 _scripts/dep.py git-status [--repo PATH] [--all]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

_SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))


def _load_script(name: str):
    """Load a hyphenated sibling script (e.g. dep-switch.py) as a module."""
    path = _SCRIPTS / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), path)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


from dep_common import (
    CONFIG_PATH,
    DEP_POLICY,
    DEVOPS_DIR,
    MATRIX_PATH,
    REPO_ROOT,
    TOOLING_ROOT,
    KNOWN_WORKSPACE_ROOTS,
    VALID_MODES,
    pin_floating_git_deps,
    same_source_git_patches,
    lint_unused_patches,
    unused_patch_warnings,
    heal_matrix_locals,
    load_active_mode,
    load_defaults,
    load_dep_config,
    load_matrix,
    matrix_is_clean,
    validate_matrix_strict,
    verify_mode_resolution,
    write_healed_matrix,
)

import dep_probe


def cmd_config(_args):
    cfg = load_dep_config()
    print("Dependency control plane")
    print(f"  tooling_root : {TOOLING_ROOT}")
    print(f"  devops_dir   : {DEVOPS_DIR}")
    print(f"  config       : {CONFIG_PATH} ({'ok' if CONFIG_PATH.exists() else 'missing'})")
    print(f"  matrix       : {MATRIX_PATH} ({'ok' if MATRIX_PATH.exists() else 'missing'})")
    print(f"  monorepo_root: {REPO_ROOT}")
    print(f"  active_mode  : {load_active_mode() or '(unset)'}")
    print(f"  policy       : {json.dumps(DEP_POLICY, indent=2)}")
    print(f"  config.paths : {cfg.get('paths')}")
    # quick sanity
    for probe in ("cosmwasm/packages/std", "cw-plus", "cw-orchestrator"):
        p = REPO_ROOT / probe
        print(f"  probe {probe}: {'OK' if p.exists() else 'MISSING'}")


def cmd_validate(args):
    matrix = load_matrix()
    issues = validate_matrix_strict(matrix, mode=args.mode)
    errors = [i for i in issues if i.level == "error"]
    warns = [i for i in issues if i.level != "error"]

    print(f"Matrix validation  root={REPO_ROOT}")
    print(f"  crates: {len(matrix)}  errors: {len(errors)}  warnings: {len(warns)}")
    for i in errors:
        print(f"  {i.format()}")
    if args.verbose:
        for i in warns:
            print(f"  {i.format()}")
    elif warns:
        print(f"  ({len(warns)} warnings hidden; pass -v)")

    if errors:
        print("\nFAIL: matrix is not clean. Run: python3 _scripts/dep.py heal")
        sys.exit(1)
    print("\nOK: matrix is clean")
    sys.exit(0)


def cmd_heal(args):
    matrix = load_matrix()
    print(f"Healing matrix paths  root={REPO_ROOT}")
    print(f"  matrix: {MATRIX_PATH}")

    report = heal_matrix_locals(matrix, dry_run=args.dry_run)

    print(f"  packages indexed: {report['packages_indexed']}")
    print(f"  fixed: {len(report['fixed'])}")
    print(f"  unchanged: {len(report['unchanged'])}")
    print(f"  unresolved: {len(report['unresolved'])}")
    print(f"  repo_only normalized: {len(report['repo_only_normalized'])}")

    if args.verbose or args.dry_run:
        for item in report["fixed"][:50]:
            print(f"  FIX {item['key']}.{item['field']}: {item['from']} -> {item['to']}")
        if len(report["fixed"]) > 50:
            print(f"  ... {len(report['fixed']) - 50} more fixes")
        for item in report["unresolved"][:30]:
            print(f"  UNRESOLVED {item['key']}.{item['field']}: {item['current']} "
                  f"(want package {item['expected_package']})")

    report_path = DEVOPS_DIR / "matrix-heal-report.json"
    if not args.dry_run:
        report_path.write_text(json.dumps(report, indent=2))
        write_healed_matrix(matrix, MATRIX_PATH)
        print(f"  wrote {MATRIX_PATH}")
        print(f"  wrote {report_path}")

        ok, errors, warns = matrix_is_clean(load_matrix())
        print(f"  post-heal: errors={len(errors)} warnings={len(warns)}")
        for e in errors[:40]:
            print(f"    {e}")
        if not ok:
            print("\nWARN: matrix still has errors after heal (unresolved packages?)")
            sys.exit(2)
        print("\nOK: matrix healed and clean")
    else:
        print("\n[DRY RUN] no files written")
        # still show would-be error count if we applied
        # re-run on a copy
        import copy
        m2 = copy.deepcopy(load_matrix())
        heal_matrix_locals(m2, dry_run=False)
        ok, errors, warns = matrix_is_clean(m2)
        print(f"  would-be post-heal: errors={len(errors)} warnings={len(warns)}")
        for e in errors[:30]:
            print(f"    {e}")


def _workspace_manifests(root: Path):
    """Root manifest plus concrete workspace members. Excludes are skipped."""
    import tomllib

    root_toml = root / "Cargo.toml"
    if not root_toml.is_file():
        return
    yield root_toml
    with root_toml.open("rb") as handle:
        data = tomllib.load(handle)
    workspace = data.get("workspace") or {}
    excluded = set()
    for pattern in workspace.get("exclude") or []:
        for match in root.glob(pattern):
            excluded.add(match.resolve())
    for pattern in workspace.get("members") or []:
        for match in root.glob(pattern):
            if match.resolve() in excluded:
                continue
            cargo = match / "Cargo.toml"
            if cargo.is_file():
                yield cargo


def cmd_pin_revs(args):
    """Rewrite matrix git branches to the matrix full rev."""
    if args.core and args.workspace:
        print("REFUSING pin-revs: pass either --core or --workspace.")
        sys.exit(1)
    if not args.core and not args.workspace:
        print("REFUSING pin-revs: pass --workspace or --core.")
        sys.exit(1)

    crates = load_matrix()
    if args.core:
        roots = [REPO_ROOT / Path(path).parent for path in KNOWN_WORKSPACE_ROOTS]
    else:
        roots = []
        for name in args.workspace:
            candidate = REPO_ROOT / name
            if not (candidate / "Cargo.toml").is_file():
                candidate = Path(name)
            if not (candidate / "Cargo.toml").is_file():
                print(f"SKIP {name}: no Cargo.toml")
                continue
            roots.append(candidate)

    wrote = 0
    same_source = []
    if args.dry_run:
        print("[DRY RUN]")
    for root in roots:
        for cargo in _workspace_manifests(root):
            text = cargo.read_text()
            new_text, changes, warnings = pin_floating_git_deps(text, crates)
            rel = cargo
            try:
                rel = cargo.relative_to(REPO_ROOT)
            except ValueError:
                pass
            for warning in warnings:
                print(f"  [WARN] {rel}: {warning}")
            for hit in same_source_git_patches(new_text):
                message = f"{rel}: {hit}"
                same_source.append(message)
                print(f"  [ERROR] {message}")
            if not changes:
                continue
            print(f"  {rel}")
            for change in changes:
                print(f"    {change}")
            if not args.dry_run:
                cargo.write_text(new_text)
                wrote += 1
    if same_source:
        print(f"pin-revs: {len(same_source)} same-source patch(es); Cargo will reject them")
        sys.exit(1)
    if args.dry_run:
        print("pin-revs: dry-run wrote nothing")
    else:
        print(f"pin-revs: updated {wrote} manifest(s)")


def _lint_workspace_roots(args):
    if args.core and args.workspace:
        print("REFUSING lint-patches: pass either --core or --workspace.")
        sys.exit(1)
    if not args.core and not args.workspace:
        print("REFUSING lint-patches: pass --workspace or --core.")
        sys.exit(1)
    if args.core:
        return [REPO_ROOT / Path(path).parent for path in KNOWN_WORKSPACE_ROOTS]
    roots = []
    for name in args.workspace:
        candidate = REPO_ROOT / name
        if not (candidate / "Cargo.toml").is_file():
            candidate = Path(name)
        if not (candidate / "Cargo.toml").is_file():
            print(f"SKIP {name}: no Cargo.toml")
            continue
        roots.append(candidate)
    return roots


def cmd_lint_patches(args):
    """Drop patch entries Cargo reports as unused. Keep the rest in order."""
    import subprocess

    roots = _lint_workspace_roots(args)
    if args.dry_run:
        print("[DRY RUN]")
    wrote = 0
    for root in roots:
        cargo = root / "Cargo.toml"
        proc = subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=str(root),
            capture_output=True,
            text=True,
            timeout=300,
        )
        rel = cargo
        try:
            rel = cargo.relative_to(REPO_ROOT)
        except ValueError:
            pass
        if proc.returncode != 0:
            err = (proc.stderr or "").strip().splitlines()
            tail = err[-1] if err else "cargo metadata failed"
            print(f"  [ERROR] {rel}: {tail}")
            sys.exit(1)
        warnings = unused_patch_warnings(proc.stderr or "")
        if not warnings:
            print(f"  {rel}: no unused patches")
            continue
        new_text, removed, dropped_sections, unmatched = lint_unused_patches(
            cargo.read_text(), root, warnings
        )
        print(f"  {rel}")
        for item in removed:
            print(f"    remove {item}")
        for section in dropped_sections:
            print(f"    drop empty [{section}]")
        for name, source in unmatched:
            print(f"    [WARN] unused patch not matched: {name} ({source})")
        if new_text == cargo.read_text():
            continue
        if not args.dry_run:
            cargo.write_text(new_text)
            wrote += 1
    if args.dry_run:
        print("lint-patches: dry-run wrote nothing")
    else:
        print(f"lint-patches: updated {wrote} manifest(s)")


def cmd_verify(args):
    matrix = load_matrix()
    mode = args.mode or load_active_mode() or "local"
    defaults = load_defaults()

    ok, errors, _warns = matrix_is_clean(matrix, mode=mode)
    if not ok:
        print("Matrix is not clean; refusing verify. Run heal/validate first.")
        for e in errors[:20]:
            print(f"  {e}")
        sys.exit(1)

    from dep_common import discover_projects

    dep_switch = _load_script("dep-switch")
    resolve_projects = dep_switch.resolve_projects

    if args.workspace:
        projects = resolve_projects(args.workspace, all_projects=True)
    else:
        projects = resolve_projects(None, all_projects=False)
        if not projects:
            projects = [p for p in discover_projects() if p.is_workspace][: args.limit]

    print(f"Verify mode={mode}  workspaces={len(projects)}  root={REPO_ROOT}")
    report = verify_mode_resolution(
        matrix,
        mode,
        projects=projects,
        defaults=defaults,
        max_workspaces=args.limit,
    )
    print(f"  checked: {len(report['checked'])}")
    for p in report["checked"]:
        print(f"    - {p}")
    if report["errors"]:
        print(f"  ERRORS: {len(report['errors'])}")
        for e in report["errors"]:
            print(f"    {e}")
        sys.exit(1)
    print("  OK: resolution matches mode")
    sys.exit(0)


def cmd_switch(args):
    # Fail closed on dirty matrix unless --force
    if args.core:
        if args.workspace:
            print("REFUSING switch: pass either --core or --workspace.")
            sys.exit(1)
        args.workspace = [str(Path(path).parent) for path in KNOWN_WORKSPACE_ROOTS]
    matrix = load_matrix()
    ok, errors, warns = matrix_is_clean(matrix, mode=args.mode)
    if not ok and not args.force:
        print("REFUSING switch: matrix has integrity errors. Run `dep.py heal` or pass --force.")
        for e in errors[:30]:
            print(f"  {e}")
        sys.exit(1)
    if warns and args.verbose:
        for w in warns[:20]:
            print(f"  {w}")

    # Delegate to dep-switch implementation
    dep_switch = _load_script("dep-switch")

    # Build namespace compatible with dep_switch.cmd_switch
    ns = argparse.Namespace(
        mode=args.mode,
        dry_run=args.dry_run,
        workspace=args.workspace,
        crate=args.crate,
        target=args.target,
    )
    dep_switch.cmd_switch(ns)

    if args.verify or DEP_POLICY.get("verify_after_switch"):
        print("\n-- verify after switch --")
        report = verify_mode_resolution(
            load_matrix(),
            args.mode,
            projects=dep_switch.resolve_projects(args.workspace, all_projects=bool(args.workspace)),
            defaults=load_defaults(),
            max_workspaces=args.verify_limit,
        )
        if report["errors"]:
            print(f"VERIFY FAILED ({len(report['errors'])} errors)")
            for e in report["errors"]:
                print(f"  {e}")
            sys.exit(1)
        print("VERIFY OK")


def cmd_status(args):
    dep_switch = _load_script("dep-switch")
    # Prefer workspace roots only (full member dump is huge under monorepo_root=crates/)
    if args.workspace:
        projects = dep_switch.resolve_projects(args.workspace, all_projects=True)
    else:
        from dep_common import discover_projects
        projects = [p for p in discover_projects() if p.is_workspace]
        projects.sort(key=lambda p: p.rel_path)

    crates = load_matrix()
    from dep_common import matrix_pkg_names, find_section

    known = matrix_pkg_names(crates)
    print(f"Dependency status  root={REPO_ROOT}  workspaces={len(projects)}\n")
    for proj in projects:
        ws_toml = proj.rel_path + "/Cargo.toml"
        ws_path = REPO_ROOT / ws_toml
        if not ws_path.exists():
            continue
        content = ws_path.read_text()
        lines = content.split("\n")
        print(f"  {ws_toml}")
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
            print(f"    patches: {total} entries ({matrix_count} matrix) [{mode_str}]")
        else:
            print("    patches: (none)")
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
            print("    ws-deps: (none)")
        print()


def cmd_ensure_branches(args):
    # Gated: creating a remote branch to satisfy a matrix entry publishes a
    # ref under a name that nothing consumes when the matrix itself is wrong.
    # Refuse unless the matrix's branches are confirmed present, or the
    # operator overrides explicitly.
    if not args.force:
        matrix = load_matrix()
        defaults = load_defaults()
        findings = dep_probe.check_branches_on_remote(matrix, defaults, mode="all")
        missing = [f for f in findings if f.state == "missing"]
        if missing:
            print("REFUSING ensure-branches: the matrix names branches the remote does not publish.")
            print()
            for f in missing:
                for line in f.format():
                    print(line)
            print()
            print("ensure-branches would publish these names from whatever HEAD is pointed at,")
            print("which is how a wrong matrix entry becomes permanent. Fix the matrix first.")
            print("Run 'branch-check' for the full report, or pass --force to override.")
            sys.exit(1)
    dep_switch = _load_script("dep-switch")
    ns = argparse.Namespace(mode=args.mode, dry_run=args.dry_run, base=args.base)
    dep_switch.cmd_ensure_branches(ns)


def cmd_branch_check(args):
    matrix = load_matrix()
    defaults = load_defaults()
    findings = dep_probe.check_branches_on_remote(
        matrix, defaults, mode=args.mode, refresh=args.refresh
    )
    counts = dep_probe.summarize_branches(findings)
    print(f"Branch check  matrix_crates={len(matrix)}  url_branch_pairs={len(findings)}")
    print("  " + "  ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    print()
    for f in findings:
        if not args.all and f.ok:
            continue
        for line in f.format():
            print(line)
    missing = [f for f in findings if f.state == "missing"]
    if missing:
        print()
        print(f"{len(missing)} matrix (url, branch) pair(s) name a branch the remote does not publish.")
        print("The matrix is the source of truth. Correct the matrix; do not create the branch.")
        sys.exit(1)


def cmd_git_status(args):
    states = dep_probe.probe_git_state(repos=args.repo)
    summary = dep_probe.summarize_repos(states)
    print(
        f"Git state  repos={summary['repos']}  dirty={summary['dirty']}  "
        f"outgoing={summary['outgoing']}  detached={summary['detached']}  "
        f"prunable={summary['prunable']}  errored={summary['errored']}"
    )
    print()
    for state in states:
        if not args.all and not (state.dirty or state.outgoing or state.detached or state.prunable or state.error):
            continue
        for line in state.format():
            print(line)
    blockers = dep_probe.preflight_for_write(states, targets=None)
    if blockers and not args.all:
        print()
        print("WRITE PREFLIGHT — these repositories block any Cargo-file write path:")
        for b in blockers:
            print(f"  - {b}")


def cmd_html(args):
    dep_html = _load_script("dep_html")
    if args.serve:
        dep_html.serve(output=args.output, port=args.port, host=args.host)
        return
    path = dep_html.write_report(
        output=args.output,
        include_git=not args.skip_git,
        include_graph=not args.skip_graph,
    )
    print(f"wrote {path}  ({path.stat().st_size} bytes, self-contained)")


def main():
    parser = argparse.ArgumentParser(
        description="Stable dependency control plane",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("config", help="Show resolved roots and policy")
    p.set_defaults(func=cmd_config)

    p = sub.add_parser("validate", help="Strict matrix integrity check (exit 1 on errors)")
    p.add_argument("--mode", choices=list(VALID_MODES), default=None)
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(func=cmd_validate)

    p = sub.add_parser("heal", help="Rewrite matrix local/zk_local to valid monorepo-relative paths")
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(func=cmd_heal)

    p = sub.add_parser("status", help="Show current dep modes across workspaces")
    p.add_argument("--workspace", action="append")
    p.set_defaults(func=cmd_status)

    p = sub.add_parser("switch", help="Switch dependency mode (fail-closed on dirty matrix)")
    p.add_argument("mode", choices=list(VALID_MODES))
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--workspace", action="append")
    p.add_argument(
        "--core",
        action="store_true",
        help="Limit the switch to the known core workspaces (not the whole monorepo)",
    )
    p.add_argument("--crate", action="append")
    p.add_argument(
        "--target",
        choices=["patches", "ws-deps", "members", "both", "all"],
        default="all",
    )
    p.add_argument("--force", action="store_true", help="Allow switch with matrix errors")
    p.add_argument("--verify", action="store_true", help="cargo metadata verify after switch")
    p.add_argument("--verify-limit", type=int, default=10)
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(func=cmd_switch)

    p = sub.add_parser(
        "pin-revs",
        help="Replace matrix git branches with the matrix full rev",
    )
    p.add_argument("--workspace", action="append")
    p.add_argument(
        "--core",
        action="store_true",
        help="Limit the rewrite to the known core workspaces",
    )
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(func=cmd_pin_revs)

    p = sub.add_parser(
        "lint-patches",
        help="Remove unused [patch] entries Cargo ignored, keeping order",
    )
    p.add_argument("--workspace", action="append")
    p.add_argument(
        "--core",
        action="store_true",
        help="Limit the lint to the known core workspaces",
    )
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(func=cmd_lint_patches)

    p = sub.add_parser("verify", help="Assert cargo metadata sources match mode")
    p.add_argument("--mode", choices=list(VALID_MODES), default=None)
    p.add_argument("--workspace", action="append")
    p.add_argument("--limit", type=int, default=15)
    p.set_defaults(func=cmd_verify)

    p = sub.add_parser("ensure-branches", help="Create missing remote branches from matrix")
    p.add_argument("mode", choices=["git", "zk_git", "all"])
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--base", default=None)
    p.add_argument("--force", action="store_true", help="Create branches even if the matrix names unpublished ones")
    p.set_defaults(func=cmd_ensure_branches)

    p = sub.add_parser(
        "branch-check",
        help="Verify every matrix git url/branch exists on the remote (read-only)",
    )
    p.add_argument("--mode", choices=["git", "zk_git", "all"], default="all")
    p.add_argument("--refresh", action="store_true", help="Ignore the cache and re-query remotes")
    p.add_argument("--all", action="store_true", help="Show present branches too, not only problems")
    p.set_defaults(func=cmd_branch_check)

    p = sub.add_parser(
        "git-status",
        help="Per-repository dirty/outgoing/worktree state and write preflight (read-only)",
    )
    p.add_argument("--repo", action="append", help="Monorepo-relative path; repeatable")
    p.add_argument("--all", action="store_true", help="Include clean repositories")
    p.set_defaults(func=cmd_git_status)

    p = sub.add_parser(
        "html",
        help="Render the dependency status report as one self-contained HTML file",
    )
    p.add_argument("--output", help="Output path (default: <monorepo>/_devops/dep-status.html)")
    p.add_argument("--serve", action="store_true", help="Serve on loopback instead of writing only")
    p.add_argument("--port", type=int, default=8787)
    p.add_argument("--host", default="127.0.0.1", help="Loopback only; anything else is refused")
    p.add_argument("--skip-git", action="store_true", help="Omit the per-repository git state walk")
    p.add_argument("--skip-graph", action="store_true", help="Omit the dependency edge scan (faster)")
    p.set_defaults(func=cmd_html)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
