#!/usr/bin/env python3
"""Fail if matrix native_tools crates are hard-required in [dependencies] without optional=true.

Allows:
  - [workspace.dependencies] path pins
  - [dev-dependencies] (integration packages may need natives for cargo test)
  - providing crate tree and *test-tube* wrapper packages

Blocks:
  - Library [dependencies] that always pull Go c-shared into default cargo check
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # type: ignore
    except ImportError:
        print("need Python 3.11+ or tomli", file=sys.stderr)
        sys.exit(2)

HERE = Path(__file__).resolve().parent
MATRIX = HERE.parent / "_devops" / "dependency-matrix.toml"


def monorepo_root() -> Path:
    return HERE.parents[3]


def section_of(lines: list[str], idx: int) -> str:
    for j in range(idx, -1, -1):
        s = lines[j].strip()
        if s.startswith("[") and s.endswith("]"):
            return s
    return ""


def main() -> int:
    data = tomllib.loads(MATRIX.read_text())
    crates = data.get("crates") or {}
    native = {
        n: m for n, m in crates.items() if isinstance(m, dict) and m.get("native_tools")
    }
    if not native:
        print("no native_tools crates in matrix")
        return 0

    root = monorepo_root()
    errors: list[str] = []
    for name, meta in native.items():
        pkg = name.replace("_", "-")
        local = (meta.get("local") or "").replace("\\", "/").strip("/")
        for cargo in root.rglob("Cargo.toml"):
            parts = cargo.parts
            if any(x in parts for x in ("target", ".git", ".worktrees")):
                continue
            rel = cargo.relative_to(root).as_posix()
            if local and (rel == f"{local}/Cargo.toml" or rel.startswith(local + "/")):
                continue
            parent = Path(rel).parent.name
            if "test-tube" in parent:
                continue
            try:
                text = cargo.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            lines = text.splitlines()
            for i, line in enumerate(lines):
                s = line.strip()
                if not s or s.startswith("#"):
                    continue
                if not re.match(rf"^{re.escape(pkg)}\s*=", s):
                    continue
                sec = section_of(lines, i)
                if "workspace.dependencies" in sec:
                    continue
                if "dev-dependencies" in sec:
                    continue
                # only plain [dependencies] or target.*.dependencies (not dev)
                if "dependencies" not in sec:
                    continue
                if re.search(r"optional\s*=\s*true", s):
                    continue
                if not any(k in s for k in ("path", "workspace", "git", "version")):
                    continue
                errors.append(
                    f"{rel}:{i+1}: hard [dependencies] on native crate {pkg!r} "
                    f"(native_tools={meta.get('native_tools')}); "
                    f"use optional = true + feature (STABLE.md Native-build path deps)"
                )
    if errors:
        print("NATIVE OPTIONAL CONSUMER CHECK FAILED:")
        for e in errors[:40]:
            print(" ", e)
        return 1
    print(
        f"ok: {len(native)} native crate(s); no hard non-optional [dependencies] consumers"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
