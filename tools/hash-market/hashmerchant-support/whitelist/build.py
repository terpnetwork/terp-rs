#!/usr/bin/env python3
"""
Loyalty eligibility whitelist: members → Merkle tree + proofs.

Leaf codec (v1):
  leaf = sha256( b"hm-loyalty/v1|" + address_utf8 )

Interior nodes: sha256( sorted_pair(left, right) )  # order-independent
Odd node at a level is promoted unchanged.

Usage:
  python3 build.py --in members.json --out tree/
  python3 build.py --in members.json --out tree/ --proof-for terp1...
  python3 build.py --self-test
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any


DOMAIN = b"hm-loyalty/v1|"
CODEC = "hm-loyalty/v1"


def sha256(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def leaf_hash(address: str) -> bytes:
    addr = address.strip()
    if not addr:
        raise ValueError("empty address")
    return sha256(DOMAIN + addr.encode("utf-8"))


def node_hash(a: bytes, b: bytes) -> bytes:
    left, right = (a, b) if a <= b else (b, a)
    return sha256(left + right)


def load_members(path: Path) -> list[dict[str, Any]]:
    raw = json.loads(path.read_text())
    if isinstance(raw, list):
        return raw
    if isinstance(raw, dict) and "members" in raw:
        return list(raw["members"])
    raise SystemExit(f"unsupported members format: {path}")


def extract_address(m: dict[str, Any]) -> str:
    for k in ("address", "addr"):
        if m.get(k):
            return str(m[k]).strip()
    raise ValueError(f"member missing address: {m!r}")


def build_tree(leaves: list[bytes]) -> tuple[bytes, dict[bytes, list[bytes]]]:
    """
    Return (root, proofs) where proofs[leaf] = list of sibling hashes (bytes) bottom-up.
    """
    if not leaves:
        empty = sha256(b"hm-loyalty/empty")
        return empty, {}

    # unique sorted leaves
    level = sorted(set(leaves))
    # proofs accumulate siblings as we climb
    proofs: dict[bytes, list[bytes]] = {leaf: [] for leaf in level}
    # track which original leaves are under each current node
    under: dict[bytes, set[bytes]] = {n: {n} for n in level}

    while len(level) > 1:
        nxt: list[bytes] = []
        i = 0
        while i < len(level):
            if i + 1 < len(level):
                a, b = level[i], level[i + 1]
                parent = node_hash(a, b)
                nxt.append(parent)
                under[parent] = under[a] | under[b]
                for leaf in under[a]:
                    proofs[leaf].append(b)
                for leaf in under[b]:
                    proofs[leaf].append(a)
                i += 2
            else:
                # promote odd node — no sibling
                solo = level[i]
                nxt.append(solo)
                i += 1
        level = nxt

    root = level[0]
    return root, proofs


def verify(leaf: bytes, proof: list[bytes], root: bytes) -> bool:
    cur = leaf
    for sib in proof:
        cur = node_hash(cur, sib)
    return cur == root


def write_tree(
    out: Path,
    members: list[dict[str, Any]],
    root: bytes,
    proofs: dict[bytes, list[bytes]],
) -> None:
    out.mkdir(parents=True, exist_ok=True)
    entries = []
    for m in members:
        addr = extract_address(m)
        leaf = leaf_hash(addr)
        if leaf not in proofs:
            raise RuntimeError(f"leaf missing from tree for {addr}")
        proof_hex = [s.hex() for s in proofs[leaf]]
        if not verify(leaf, proofs[leaf], root):
            raise RuntimeError(f"internal proof verify failed for {addr}")
        entries.append(
            {
                "address": addr,
                "name": m.get("name"),
                "leaf": leaf.hex(),
                "proof": proof_hex,
            }
        )

    meta = {
        "codec": CODEC,
        "domain": DOMAIN.decode("utf-8"),
        "hash": "sha256",
        "pair": "sorted-concat",
        "root": root.hex(),
        "count": len(entries),
        "members": entries,
    }
    (out / "tree.json").write_text(json.dumps(meta, indent=2) + "\n")
    (out / "root.txt").write_text(root.hex() + "\n")
    proofs_dir = out / "proofs"
    proofs_dir.mkdir(exist_ok=True)
    for e in entries:
        (proofs_dir / f"{e['address']}.json").write_text(
            json.dumps(
                {
                    "codec": CODEC,
                    "address": e["address"],
                    "leaf": e["leaf"],
                    "proof": e["proof"],
                    "root": root.hex(),
                },
                indent=2,
            )
            + "\n"
        )


def self_test() -> None:
    members = [
        {"name": "alice", "address": "terp1alice000000000000000000000000000001"},
        {"name": "bob", "address": "terp1bob000000000000000000000000000000002"},
        {"name": "carol", "address": "terp1carol000000000000000000000000000003"},
        {"name": "dave", "address": "terp1dave00000000000000000000000000000004"},
        {"name": "eve", "address": "terp1eve000000000000000000000000000000005"},
    ]
    leaves = [leaf_hash(extract_address(m)) for m in members]
    root, proofs = build_tree(leaves)
    for m in members:
        leaf = leaf_hash(extract_address(m))
        assert verify(leaf, proofs[leaf], root), m

    # non-member should not verify with someone else's proof
    stranger = leaf_hash("terp1notamember000000000000000000000099")
    stolen = proofs[leaves[0]]
    assert not verify(stranger, stolen, root)

    # empty tree
    r0, p0 = build_tree([])
    assert len(r0) == 32 and p0 == {}

    # single leaf
    one = [leaf_hash("terp1only000000000000000000000000000001")]
    r1, p1 = build_tree(one)
    assert verify(one[0], p1[one[0]], r1)
    assert p1[one[0]] == []

    print("self-test OK root=", root.hex())


def main() -> None:
    ap = argparse.ArgumentParser(description="Build loyalty whitelist Merkle tree")
    ap.add_argument("--in", dest="inp", type=Path, help="members.json")
    ap.add_argument("--out", dest="out", type=Path, help="output directory")
    ap.add_argument("--proof-for", dest="proof_for", help="print proof JSON for address")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        self_test()
        return

    if not args.inp or not args.out:
        ap.error("--in and --out required unless --self-test")

    members = load_members(args.inp)
    leaves = [leaf_hash(extract_address(m)) for m in members]
    root, proofs = build_tree(leaves)
    write_tree(args.out, members, root, proofs)
    print(f"root={root.hex()}")
    print(f"wrote {args.out}/tree.json ({len(members)} members)")

    if args.proof_for:
        leaf = leaf_hash(args.proof_for)
        if leaf not in proofs:
            raise SystemExit(f"address not in tree: {args.proof_for}")
        print(
            json.dumps(
                {
                    "address": args.proof_for,
                    "leaf": leaf.hex(),
                    "proof": [s.hex() for s in proofs[leaf]],
                    "root": root.hex(),
                    "codec": CODEC,
                },
                indent=2,
            )
        )


if __name__ == "__main__":
    main()
