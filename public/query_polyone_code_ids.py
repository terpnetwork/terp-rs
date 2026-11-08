#!/usr/bin/env python3
"""Query a Juno LCD for polytone contract code_ids from polyone.json.

Default LCD: https://juno-api.polkachu.com
Override: JUNO_LCD=https://juno-api.publicnode.com python3 query_polyone_code_ids.py

Does not broadcast. Prints a table and exits 1 if any live instance
diverges from meta.code_ids.live_daodao.
"""

from __future__ import annotations

import json
import os
import ssl
import sys
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
POLYONE = HERE / "polyone.json"
LCD = os.environ.get("JUNO_LCD", "https://juno-api.polkachu.com").rstrip("/")
CTX = ssl.create_default_context()


def get(path: str) -> dict:
    url = f"{LCD}{path}"
    req = urllib.request.Request(url, headers={"User-Agent": "terp-polyone/1.0"})
    try:
        with urllib.request.urlopen(req, timeout=25, context=CTX) as r:
            return json.loads(r.read().decode())
    except urllib.error.URLError as e:
        # macOS python.org builds often lack certs; curl-backed retry
        import subprocess

        out = subprocess.check_output(["curl", "-fsS", "--max-time", "25", url], text=True)
        return json.loads(out)


def contract_info(addr: str) -> dict:
    data = get(f"/cosmwasm/wasm/v1/contract/{addr}")
    return data.get("contract_info", data)


def main() -> int:
    blob = json.loads(POLYONE.read_text())
    live = blob["meta"]["code_ids"]["live_daodao"]
    expected = {
        "listener": str(live["listener"]),
        "note": str(live["note"]),
        "voice": str(live["voice"]),
    }

    rows = []
    diverged = False
    for chain, conn in blob["juno-1"].items():
        for role in ("listener", "note"):
            addr = conn[role]
            info = contract_info(addr)
            cid = str(info.get("code_id"))
            ok = cid == expected[role]
            if not ok:
                diverged = True
            rows.append((chain, role, cid, info.get("label"), addr, ok))
    for chain, voice in blob.get("juno_inbound_voices", {}).items():
        addr = voice["voice"]
        info = contract_info(addr)
        cid = str(info.get("code_id"))
        ok = cid == expected["voice"]
        if not ok:
            diverged = True
        rows.append((chain, "voice", cid, info.get("label"), addr, ok))

    print(f"lcd={LCD}")
    print(f"{'counterparty':16} {'role':10} {'code':6} {'ok':3} label")
    for chain, role, cid, label, _addr, ok in rows:
        print(f"{chain:16} {role:10} {cid:6} {'Y' if ok else 'N':3} {label}")

    if diverged:
        print("DIVERGED from meta.code_ids.live_daodao", file=sys.stderr)
        return 1
    print("all live instances match live_daodao pins")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
