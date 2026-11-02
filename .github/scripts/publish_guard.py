#!/usr/bin/env python3
"""Fail closed on crates.io publish while terp-rs still patch-depends on siblings.

Verify mode exits 0 while a [patch.crates-io] entry uses path = "../".
That is the current tree: a downloaded crate cannot build those paths, so
this workflow must not publish. Publish mode always exits 1.
When the foreign path patches are gone, verify exits 1 so a real publisher
has to be wired before anyone uploads.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path


def foreign_path_patches(root: Path) -> list[str]:
    text = (root / "Cargo.toml").read_text()
    lines = text.splitlines()
    in_patch = False
    found = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("["):
            in_patch = stripped == "[patch.crates-io]"
            continue
        if in_patch and 'path = "../' in stripped:
            found.append(stripped)
    return found


def main() -> int:
    mode = os.environ.get("MODE", "verify")
    root = Path(__file__).resolve().parents[2]
    hits = foreign_path_patches(root)
    print(f"mode={mode} foreign_path_patches={len(hits)}")
    if mode == "publish":
        print("This workflow does not publish. The workspace still resolves forks by path.")
        return 1
    if hits:
        print("Publish stays closed: [patch.crates-io] still points at sibling checkouts.")
        return 0
    print("Foreign path patches are gone. Wire a publisher before uploading to crates.io.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
