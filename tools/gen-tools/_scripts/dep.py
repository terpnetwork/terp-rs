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
    VALID_MODES,
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

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
