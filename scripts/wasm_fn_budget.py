#!/usr/bin/env python3
"""Detect the AArch64 singlepass i64.div_u branch that the chain rejects.

Wasmer singlepass 7.4.2, the compiler in libwasmvm v4.0.0-zk, lowers
``i64.div_u`` with a short ``cbz`` to that function's division-by-zero
trap. The trap is the function's first dynamic label and is emitted after
the body. ``i64.div_s`` and the i32 divisions use a long branch instead.

Dynasm encodes that ``cbz`` as a signed 19-bit, 4-byte-aligned offset
(``fits_signed_bitfield(byte_delta >> 2, 19)``). The machine-code distance
from the check to the trap must sit in [-1048576, 1048572]. Outside that
window finalization returns
``ImpossibleRelocation(Dynamic(DynamicLabel(0)))`` and the chain reports
``create wasm contract failed``.

This script cannot assemble ARM64. It applies two Wasm ceilings measured
on terp-core:local-arm64 (singlepass 7.4.2), both standing in for that
1 MiB machine-code window:

* ``i64.div_u`` tail. Longest tail that stored: 28062
  (cw_vesting-staking). gauge_orchestrator failed at 47699.
* Whole function body. Largest body that stored: 69396 (cw721_roles).
  dao_calendar's 100895-byte function contains no ``i64.div_u`` and
  still failed with the same relocation. A function with no
  ``i64.div_u`` can still compile past 1 MiB of ARM64.

CosmWasm's own limits (4096 operand-stack values, 1024 locals) are a
different check and do not produce this error.

Usage:
  python3 scripts/wasm_fn_budget.py artifacts/*.wasm
  python3 scripts/wasm_fn_budget.py --opt artifacts/cw721_svg.wasm
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

# Longest i64.div_u Wasm tail that stored on terp-core:local-arm64.
# cw_vesting-staking stored at 28062. gauge_orchestrator failed at 47699.
# This is not the assembler constant. That constant is CBZ_NATIVE_MAX,
# which counts ARM64 bytes, not Wasm bytes. Tails between these two
# measurements were not uploaded. The script rejects them.
DEFAULT_BUDGET = 28062
# Largest Wasm function body that stored on the same image.
# cw721_roles stored at 69396. dao_calendar failed at 100895.
DEFAULT_MAX_FUNCTION = 69396
# Signed 19-bit cbz, in bytes of machine code. Upper bound is one slot shorter.
CBZ_NATIVE_MIN = -1_048_576
CBZ_NATIVE_MAX = 1_048_572
I64_DIV_U = 0x80
VALTYPES = {0x7F, 0x7E, 0x7D, 0x7C, 0x7B, 0x70, 0x6F, 0x6E, 0x6D, 0x6C, 0x6B, 0x6A}
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


def sleb(data: bytes, i: int) -> tuple[int, int]:
    n = 0
    shift = 0
    while True:
        b = data[i]
        i += 1
        n |= (b & 0x7F) << shift
        if b < 128:
            return n, i
        shift += 7


def skip_blocktype(data: bytes, i: int) -> int:
    b = data[i]
    if b == 0x40 or b in VALTYPES:
        return i + 1
    _, i = sleb(data, i)
    return i


def skip_memarg(data: bytes, i: int) -> int:
    _, i = leb(data, i)
    _, i = leb(data, i)
    return i


def skip_simd(data: bytes, i: int) -> int:
    op, i = leb(data, i)
    # Memory lane ops carry a memarg. The rest carry a lane, a shuffle mask,
    # or nothing. A wrong skip desynchronizes the walker; callers check that
    # the body is fully consumed.
    memarg_ops = {0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59}
    if op in memarg_ops:
        return skip_memarg(data, i)
    if op == 0x0C:  # v128.const
        return i + 16
    if op == 0x0D:  # i8x16.shuffle
        return i + 16
    if op in {0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C} | set(range(0x21, 0x35)) | {0x36}:
        return i + 1  # lane index
    return i


def skip_immediate(data: bytes, op: int, i: int) -> int:
    if op in (0x02, 0x03, 0x04):
        return skip_blocktype(data, i)
    if op in (0x0C, 0x0D, 0x10):
        _, i = leb(data, i)
        return i
    if op == 0x0E:
        count, i = leb(data, i)
        for _ in range(count):
            _, i = leb(data, i)
        _, i = leb(data, i)
        return i
    if op == 0x11:
        _, i = leb(data, i)
        _, i = leb(data, i)
        return i
    if op == 0x1C:
        count, i = leb(data, i)
        return i + count
    if op in (0x20, 0x21, 0x22, 0x23, 0x24):
        _, i = leb(data, i)
        return i
    if 0x28 <= op <= 0x3E:
        return skip_memarg(data, i)
    if op in (0x3F, 0x40):
        _, i = leb(data, i)
        return i
    if op == 0x41:
        _, i = sleb(data, i)
        return i
    if op == 0x42:
        _, i = sleb(data, i)
        return i
    if op == 0x43:
        return i + 4
    if op == 0x44:
        return i + 8
    if op == 0xD0:
        return i + 1
    if op == 0xD2:
        _, i = leb(data, i)
        return i
    if op == 0xFC:
        sub, i = leb(data, i)
        if sub <= 7:
            return i
        if sub in (8, 12):
            _, i = leb(data, i)
            _, i = leb(data, i)
            return i
        if sub in (9, 13):
            _, i = leb(data, i)
            return i
        if sub in (10, 14):
            _, i = leb(data, i)
            _, i = leb(data, i)
            return i
        if sub in (11, 15, 16, 17):
            _, i = leb(data, i)
            return i
        return i
    if op == 0xFD:
        return skip_simd(data, i)
    return i


def udiv_tails(body: bytes) -> tuple[list[int], bool]:
    """Wasm bytes from each i64.div_u to the end of this function body.

    The boolean is false when the opcode walk does not consume the body.
    """
    j = 0
    nlocals, j = leb(body, j)
    for _ in range(nlocals):
        _, j = leb(body, j)
        j += 1
    ops_at = j
    depth = 1
    hits: list[int] = []
    while j < len(body):
        op = body[j]
        at = j
        j += 1
        try:
            j = skip_immediate(body, op, j)
        except IndexError:
            return hits, False
        if op == I64_DIV_U:
            hits.append(len(body) - at)
        if op in (0x02, 0x03, 0x04):
            depth += 1
        elif op == 0x0B:
            depth -= 1
            if depth == 0:
                return hits, j == len(body)
    return hits, False


def function_udiv_tails(path: Path) -> tuple[list[int], list[str]]:
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
        return [], []
    j = 0
    count, j = leb(code, j)
    tails: list[int] = []
    bad: list[str] = []
    for idx in range(count):
        body_len, j = leb(code, j)
        body = code[j : j + body_len]
        j += body_len
        hits, ok = udiv_tails(body)
        if not ok:
            bad.append(f"function {idx} opcode walk did not consume {body_len} bytes")
        tails.extend(hits)
    return tails, bad


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
    parser.add_argument(
        "--max-udiv-tail",
        type=int,
        default=DEFAULT_BUDGET,
        help="Wasm bytes from i64.div_u to the end of its function",
    )
    parser.add_argument(
        "--max-function-size",
        type=int,
        default=DEFAULT_MAX_FUNCTION,
        help="Wasm bytes in one function body",
    )
    parser.add_argument("--top", type=int, default=8)
    parser.add_argument("--check", type=Path, help="cosmwasm-check binary; runs the stack limiter")
    parser.add_argument("--capabilities", default=DEFAULT_CAPS)
    parser.add_argument("--out", type=Path, help="write a JSON report")
    args = parser.parse_args()

    report = {
        "machine": platform.platform(),
        "python": platform.python_version(),
        "wasm_opt": tool_version(["wasm-opt", "--version"]),
        "udiv_tail_budget": args.max_udiv_tail,
        "max_function_body": args.max_function_size,
        "cbz_native_min": CBZ_NATIVE_MIN,
        "cbz_native_max": CBZ_NATIVE_MAX,
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
            tails, walk_errors = function_udiv_tails(measured)
            tails_sorted = sorted(tails, reverse=True)
            longest_tail = tails_sorted[0] if tails_sorted else 0
            body_over = largest > args.max_function_size
            over = longest_tail > args.max_udiv_tail or body_over or bool(walk_errors)
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
                "i64_div_u": len(tails),
                "longest_udiv_tail": longest_tail,
                "udiv_tails": tails_sorted[: args.top],
                "walk_errors": walk_errors,
                "body_over_budget": body_over,
                "over_budget": over,
                "check": check,
            }
            report["contracts"].append(row)
            status = "FAIL" if over or (check and check["exit"] != 0) else "ok"
            div = f" i64.div_u={len(tails)} tail={longest_tail}" if tails or walk_errors else " i64.div_u=0"
            if body_over:
                div += " body_over=1"
            err = f" walk_errors={len(walk_errors)}" if walk_errors else ""
            print(
                f"{status} {src.name} largest={largest}{div}{err} functions={len(sizes)} bytes={row['measured_bytes']}"
            )

    if args.out:
        args.out.write_text(json.dumps(report, indent=2) + "\n")
        print(f"wrote {args.out}")
    return 1 if failed else 0




# ---------------------------------------------------------------------------
# Per-function attribution, name section, and the PASS / WARN / ERROR report.
# ---------------------------------------------------------------------------

# Measured failures on Wasmer 7.4.2 singlepass AArch64 (cosmwasm-check built
# from crates/cosmwasm on groot, 2026-10-26). Tails above DEFAULT_BUDGET but
# below these are WARN: terp721_account_manifold (tail 42787) and
# terp_bounty_escrow (42989) compile, cw_svg_minter (42521) does not. The Wasm
# tail is only a proxy for native distance, so only a real compile is ERROR.
KNOWN_FAIL_UDIV_TAIL = 42521
KNOWN_FAIL_FUNCTION = 100895
RELOC_MARKERS = ("ImpossibleRelocation", "Assembler failed", "Compilation error", "Could not compile")
SCAN_CAPS = DEFAULT_CAPS + ",zk,hash_blake"
# Relaxed limits so a static-validation limit does not hide the compile result.
RELAXED_LIMITS = (
    '{"max_function_params":1000,"max_total_function_params":1000000,'
    '"max_function_locals":100000,"max_total_function_locals":100000000}'
)


def sections(data: bytes) -> list[tuple[int, bytes]]:
    i = 8
    out = []
    while i < len(data):
        sid = data[i]
        i += 1
        n, i = leb(data, i)
        out.append((sid, data[i : i + n]))
        i += n
    return out


def imported_function_count(payload: bytes) -> int:
    j = 0
    count, j = leb(payload, j)
    funcs = 0
    for _ in range(count):
        for _ in range(2):
            ln, j = leb(payload, j)
            j += ln
        kind = payload[j]
        j += 1
        if kind == 0:
            funcs += 1
            _, j = leb(payload, j)
        elif kind == 1:
            j += 1
            flag = payload[j]
            j += 1
            _, j = leb(payload, j)
            if flag & 1:
                _, j = leb(payload, j)
        elif kind == 2:
            flag = payload[j]
            j += 1
            _, j = leb(payload, j)
            if flag & 1:
                _, j = leb(payload, j)
        elif kind == 3:
            j += 2
        else:
            j += 1
    return funcs


def function_names(payload: bytes) -> dict[int, str]:
    j = 0
    ln, j = leb(payload, j)
    if payload[j : j + ln] != b"name":
        return {}
    j += ln
    names: dict[int, str] = {}
    while j < len(payload):
        sub = payload[j]
        j += 1
        size, j = leb(payload, j)
        end = j + size
        if sub == 1:
            count, k = leb(payload, j)
            for _ in range(count):
                idx, k = leb(payload, k)
                nl, k = leb(payload, k)
                names[idx] = payload[k : k + nl].decode("utf-8", "replace")
                k += nl
        j = end
    return names


def per_function(path: Path) -> list[dict]:
    """One row per defined function: index, name, body size, longest i64.div_u tail."""
    data = path.read_bytes()
    if data[:4] != b"\0asm":
        raise SystemExit(f"{path} is not a wasm module")
    imports = 0
    names: dict[int, str] = {}
    code = None
    for sid, payload in sections(data):
        if sid == 2:
            imports = imported_function_count(payload)
        elif sid == 0:
            try:
                names.update(function_names(payload))
            except IndexError:
                pass
        elif sid == 10:
            code = payload
    rows: list[dict] = []
    if code is None:
        return rows
    j = 0
    count, j = leb(code, j)
    for n in range(count):
        body_len, j = leb(code, j)
        body = code[j : j + body_len]
        j += body_len
        hits, ok = udiv_tails(body)
        idx = imports + n
        rows.append(
            {
                "index": idx,
                "name": names.get(idx),
                "body": body_len,
                "udiv": len(hits),
                "tail": max(hits) if hits else 0,
                "walk_ok": ok,
            }
        )
    return rows


def fn_label(row: dict | None) -> str:
    if row is None:
        return "-"
    label = f"func[{row['index']}]"
    if row.get("name"):
        name = row["name"]
        if len(name) > 70:
            name = name[:67] + "..."
        label += f" {name}"
    return label


def run_check(check: Path, wasm: Path, limits: str | None) -> tuple[int, str]:
    cmd = [str(check), "--available-capabilities", SCAN_CAPS]
    if limits:
        cmd += ["--wasm-limits", limits]
    cmd.append(str(wasm))
    proc = subprocess.run(cmd, capture_output=True, text=True)
    text = (proc.stdout + proc.stderr).strip()
    lines = [l for l in text.splitlines() if l.startswith("Error")]
    return proc.returncode, (lines[0] if lines else text.splitlines()[-1] if text else "")


def stub_function(data: bytes, code_index: int) -> bytes:
    """Copy of the module with one defined function's body replaced by `unreachable`."""
    out = bytearray(data[:8])
    i = 8
    while i < len(data):
        sid = data[i]
        start = i
        i += 1
        n, i = leb(data, i)
        payload = data[i : i + n]
        i += n
        if sid != 10:
            out += data[start:i]
            continue
        j = 0
        count, j = leb(payload, j)
        body = bytearray(enc_leb(count))
        for k in range(count):
            blen, j2 = leb(payload, j)
            fn = payload[j:j2 + blen]
            j = j2 + blen
            body += enc_leb(3) + b"\x00\x00\x0b" if k == code_index else fn
        out += bytes([10]) + enc_leb(len(body)) + body
    return bytes(out)


def enc_leb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def bisect_offenders(check: Path, wasm: Path, rows: list[dict], top: int) -> list[dict]:
    """Stub the largest functions one at a time; those whose removal makes the
    module compile are the functions singlepass cannot assemble."""
    data = wasm.read_bytes()
    imports = rows[0]["index"] if rows else 0
    found = []
    with tempfile.TemporaryDirectory() as tmp:
        for r in sorted(rows, key=lambda r: r["body"], reverse=True)[:top]:
            probe = Path(tmp) / f"stub{r['index']}.wasm"
            probe.write_bytes(stub_function(data, r["index"] - imports))
            rc, msg = run_check(check, probe, RELAXED_LIMITS)
            if rc == 0:
                found.append(r)
    return found


def scan(src: Path, measured: Path, args) -> dict:
    rows = per_function(measured)
    by_tail = max(rows, key=lambda r: r["tail"], default=None)
    by_body = max(rows, key=lambda r: r["body"], default=None)
    walk_errors = [r for r in rows if not r["walk_ok"]]
    tail = by_tail["tail"] if by_tail else 0
    body = by_body["body"] if by_body else 0
    notes: list[str] = []
    arm = "PASS"
    offender = None
    if tail > args.max_udiv_tail:
        arm = "WARN"
        offender = by_tail
        notes.append(f"i64.div_u tail {tail} > {args.max_udiv_tail}")
    if body > args.max_function_size:
        arm = "WARN"
        offender = offender or by_body
        notes.append(f"body {body} > {args.max_function_size}")
    if tail >= KNOWN_FAIL_UDIV_TAIL or body >= KNOWN_FAIL_FUNCTION:
        notes.append("at/over a measured failure")
    if walk_errors:
        arm = "WARN"
        notes.append(f"opcode walk failed in {len(walk_errors)} fn")
    verified = None
    static = None
    if args.check:
        rc, msg = run_check(args.check, measured, None)
        if rc != 0 and not any(m in msg for m in RELOC_MARKERS):
            static = msg
            rc, msg = run_check(args.check, measured, RELAXED_LIMITS)
        if rc == 0:
            verified = "compiled"
            arm = "WARN" if static else "PASS"
        elif any(m in msg for m in RELOC_MARKERS):
            verified = "failed"
            arm = "ERROR"
            hits = bisect_offenders(args.check, measured, rows, args.bisect) if args.bisect else []
            if hits:
                offender = hits[0]
                if len(hits) > 1:
                    notes.append("also: " + ", ".join(fn_label(h) for h in hits[1:]))
                notes.append(f"confirmed by stubbing (body {offender['body']}, udiv tail {offender['tail']})")
            else:
                offender = offender or by_tail
            notes.insert(0, msg.replace("Error compiling Wasm: Could not compile: Compilation error: ", ""))
        else:
            arm = "WARN"
            notes.append(msg)
        if static:
            notes.append("default limits: " + static.replace("Error during static Wasm validation: ", ""))
    host = platform.machine().lower().replace("aarch64", "arm64")
    # Singlepass x86_64 branches are rel32 (+-2 GiB): no measured relocation limit.
    x86 = "PASS" if not walk_errors else "WARN"
    if args.check and host in ("x86_64", "amd64"):
        x86, arm_guess = arm, ("WARN" if tail > args.max_udiv_tail or body > args.max_function_size else "PASS")
        arm = arm_guess
    return {
        "path": str(src),
        "artifact": src.name,
        "bytes": measured.stat().st_size,
        "functions": len(rows),
        "largest": body,
        "largest_fn": fn_label(by_body),
        "longest_udiv_tail": tail,
        "longest_udiv_fn": fn_label(by_tail),
        "arm64": arm,
        "x86_64": x86,
        "verified_on": f"{host}:{verified}" if verified else None,
        "offender": fn_label(offender) if offender else None,
        "notes": notes,
        "top": sorted(rows, key=lambda r: max(r["tail"], r["body"]), reverse=True)[: args.top],
    }


def print_table(results: list[dict], check: bool) -> None:
    head = ("artifact", "arm64", "x86_64", "largest", "udiv_tail", "offending function / notes")
    rows = []
    for r in results:
        detail = r["offender"] or ""
        if r["notes"]:
            detail = (detail + "  " if detail else "") + "; ".join(r["notes"])
        rows.append((r["artifact"], r["arm64"], r["x86_64"], str(r["largest"]), str(r["longest_udiv_tail"]), detail))
    w = [max(len(h), *(len(x[i]) for x in rows)) if rows else len(h) for i, h in enumerate(head[:5])]
    line = "  ".join(h.ljust(w[i]) for i, h in enumerate(head[:5])) + "  " + head[5]
    print("=== wasm arch scan (Wasmer 7.4.2 singlepass) ===")
    print(f"mode: {'compile-verified (cosmwasm-check) + ' if check else ''}heuristic "
          f"(udiv tail <= {DEFAULT_BUDGET}, body <= {DEFAULT_MAX_FUNCTION})")
    print(line)
    print("-" * min(len(line) + 40, 160))
    for x in rows:
        print("  ".join(x[i].ljust(w[i]) for i in range(5)) + "  " + x[5])
    counts = {s: sum(1 for r in results if r["arm64"] == s) for s in ("PASS", "WARN", "ERROR")}
    print("-" * min(len(line) + 40, 160))
    print(f"arm64: {counts['PASS']} PASS, {counts['WARN']} WARN, {counts['ERROR']} ERROR")


def scan_main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="PASS/WARN/ERROR arch scan of wasm artifacts")
    parser.add_argument("wasm", nargs="+", type=Path)
    parser.add_argument("--max-udiv-tail", type=int, default=DEFAULT_BUDGET)
    parser.add_argument("--max-function-size", type=int, default=DEFAULT_MAX_FUNCTION)
    parser.add_argument("--top", type=int, default=5)
    parser.add_argument("--check", type=Path, help="cosmwasm-check built from crates/cosmwasm (Wasmer 7.4.2)")
    parser.add_argument("--fail-on", choices=("never", "error", "warn"), default="never")
    parser.add_argument("--bisect", type=int, default=12,
                        help="on ERROR, stub the N largest functions to find the one that fails (0 = off)")
    parser.add_argument("--functions", action="store_true", help="print the top functions per artifact")
    parser.add_argument("--out", type=Path, help="write a JSON report")
    args = parser.parse_args(argv)
    if args.check and not args.check.exists():
        print(f"warn: --check {args.check} not found; heuristic only", file=sys.stderr)
        args.check = None
    results = [scan(p, p, args) for p in args.wasm if p.suffix == ".wasm"]
    print_table(results, bool(args.check))
    if args.functions:
        for r in results:
            print(f"\n{r['artifact']}: top functions")
            for f in r["top"]:
                print(f"  {fn_label(f)}  body={f['body']} udiv={f['udiv']} tail={f['tail']}")
    if args.out:
        args.out.write_text(json.dumps({"machine": platform.platform(), "artifacts": results}, indent=2) + "\n")
    worst = {"PASS": 0, "WARN": 1, "ERROR": 2}
    level = max((worst[r["arm64"]] for r in results), default=0)
    level = max(level, max((worst[r["x86_64"]] for r in results), default=0))
    if args.fail_on == "error" and level >= 2:
        return 1
    if args.fail_on == "warn" and level >= 1:
        return 1
    return 0


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "scan":
        sys.exit(scan_main(sys.argv[2:]))
    sys.exit(main())
