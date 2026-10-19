#!/usr/bin/env python3
"""Function-size budget for CosmWasm contracts.

The chain compiles a contract with a stack-height limiter, then Wasmer
singlepass. wasm-opt -Oz inlines every one-caller function into the exports.
On aarch64 that fused body can fail with
ImpossibleRelocation(Dynamic(DynamicLabel(0))).

Stored artifacts that pass the limiter include dao-dao-core at 64835
bytes and cw_infuser at 59744 bytes. cw721_svg fails at 82187 bytes.
The default budget is 68000 bytes, between those two measurements.

This is the contract-size workload. It is not a CPU profile. CPU sampling
belongs on a host benchmark once a contract has a proven hot loop.

Usage:
  python3 scripts/wasm_fn_budget.py --opt artifacts/cw721_svg.wasm
  python3 scripts/wasm_fn_budget.py --check path/to/cosmwasm-check artifacts/*.wasm
"""

from __future__ import annotations

import argparse
import json
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_BUDGET = 68000
DEFAULT_CAPS = (
    "ibc2,iterator,staking,stargate,cosmwasm_1_1,cosmwasm_1_2,cosmwasm_1_3,"
    "cosmwasm_1_4,cosmwasm_2_0,cosmwasm_2_1,cosmwasm_2_2,cosmwasm_3_0,bulk_memory"
)


def leb(data: bytes, i: int) -> tuple[int, int]:
    n = 0
    shift = 0
    while True:
        b = data[i]
        i += 1
        n |= (b & 0x7F) << shift
        if b < 128:
            return n, i
        shift += 7


def function_sizes(path: Path) -> list[int]:
    data = path.read_bytes()
    if data[:4] != b"\0asm":
        raise SystemExit(f"{path} is not a wasm module")
    i = 8
    code = None
    while i < len(data):
        sid = data[i]
        i += 1
        n, i = leb(data, i)
        payload = data[i : i + n]
        i += n
        if sid == 10:
            code = payload
    if code is None:
        return []
    j = 0
    count, j = leb(code, j)
    sizes = []
    for _ in range(count):
        body, j = leb(code, j)
        j += body
        sizes.append(body)
    return sizes


def tool_version(cmd: list[str]) -> str:
    try:
        out = subprocess.run(cmd, check=False, capture_output=True, text=True)
    except FileNotFoundError:
        return "missing"
    text = (out.stdout or out.stderr).strip().splitlines()
    return text[0] if text else "unknown"


def optimize(src: Path, dst: Path) -> None:
    wasm_opt = shutil.which("wasm-opt")
    if wasm_opt is None:
        raise SystemExit("wasm-opt is not on PATH")
    cmd = [
        wasm_opt,
        "-Oz",
        "--enable-bulk-memory",
        "--enable-sign-ext",
        "--enable-nontrapping-float-to-int",
        "--enable-reference-types",
        "--enable-multivalue",
        "--enable-mutable-globals",
        "-o",
        str(dst),
        str(src),
    ]
    subprocess.run(cmd, check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wasm", nargs="+", type=Path)
    parser.add_argument("--opt", action="store_true", help="run wasm-opt -Oz before measuring")
    parser.add_argument("--max-function-size", type=int, default=DEFAULT_BUDGET)
    parser.add_argument("--top", type=int, default=8)
    parser.add_argument("--check", type=Path, help="cosmwasm-check binary; runs the stack limiter")
    parser.add_argument("--capabilities", default=DEFAULT_CAPS)
    parser.add_argument("--out", type=Path, help="write a JSON report")
    args = parser.parse_args()

    report = {
        "machine": platform.platform(),
        "python": platform.python_version(),
        "wasm_opt": tool_version(["wasm-opt", "--version"]),
        "budget": args.max_function_size,
        "contracts": [],
    }
    failed = False
    for src in args.wasm:
        with tempfile.TemporaryDirectory() as tmp:
            measured = src
            if args.opt:
                measured = Path(tmp) / (src.stem + ".opt.wasm")
                optimize(src, measured)
            sizes = function_sizes(measured)
            sizes_sorted = sorted(sizes, reverse=True)
            largest = sizes_sorted[0] if sizes_sorted else 0
            over = largest > args.max_function_size
            check = None
            if args.check:
                proc = subprocess.run(
                    [
                        str(args.check),
                        "--available-capabilities",
                        args.capabilities,
                        str(measured),
                    ],
                    capture_output=True,
                    text=True,
                )
                check = {
                    "exit": proc.returncode,
                    "stdout": proc.stdout[-2000:],
                    "stderr": proc.stderr[-2000:],
                }
                if proc.returncode != 0:
                    failed = True
            if over:
                failed = True
            row = {
                "path": str(src),
                "measured_bytes": measured.stat().st_size,
                "functions": len(sizes),
                "largest": largest,
                "top": sizes_sorted[: args.top],
                "over_budget": over,
                "check": check,
            }
            report["contracts"].append(row)
            status = "FAIL" if over or (check and check["exit"] != 0) else "ok"
            print(
                f"{status} {src.name} largest={largest} functions={len(sizes)} bytes={row['measured_bytes']}"
            )

    if args.out:
        args.out.write_text(json.dumps(report, indent=2) + "\n")
        print(f"wrote {args.out}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
