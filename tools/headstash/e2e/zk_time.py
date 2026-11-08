#!/usr/bin/env python3
"""On-chain verify WALL-TIME benchmark (measurement only, 127.0.0.1 only).
Usage: zk_time.py <terpd> <home> <chain-id> <rpc-port> <prom-port> <api-port> <node-log> <fixture> <zkid> <out.json> <contract|new> [reps] [mode]
Per tx it records: CometBFT finalize_block time for the block holding the tx (sampled
from cometbft_abci_connection_method_timing_seconds{method=finalize_block}), the SDK
telemetry delta of wasm execute time, and the ZKTIME host-function lines that an
instrumented libwasmvm prints to stderr. Keys are referenced by keyring name only."""
import json, subprocess, sys, time, base64, threading, re, urllib.request
B, H, CID, PORT, PROM, API, LOG, FX, ZKID, OUT, ADDR = sys.argv[1:12]
REPS = int(sys.argv[12]) if len(sys.argv) > 12 else 5
MODE = sys.argv[13] if len(sys.argv) > 13 else "full"
ZKID = int(ZKID); NODE = f"tcp://127.0.0.1:{PORT}"
COMMON = ["--home", H, "--keyring-backend", "test", "--chain-id", CID, "--node", NODE, "-o", "json"]

def sh(a): return subprocess.run(a, capture_output=True, text=True)
def get(url):
    with urllib.request.urlopen(url, timeout=5) as r: return r.read().decode()

fb = []  # (t, latest_block_height, consensus_height, finalize_block seconds) per single-block increment
def sampler():
    last = None
    while True:
        try:
            t = get(f"http://127.0.0.1:{PROM}/metrics")
            s = c = h = ch = None
            for ln in t.splitlines():
                if ln.startswith("cometbft_abci_connection_method_timing_seconds_sum") and 'method="finalize_block"' in ln: s = float(ln.split()[-1])
                elif ln.startswith("cometbft_abci_connection_method_timing_seconds_count") and 'method="finalize_block"' in ln: c = int(float(ln.split()[-1]))
                elif ln.startswith("cometbft_consensus_latest_block_height"): h = int(float(ln.split()[-1]))
                elif ln.startswith("cometbft_consensus_height"): ch = int(float(ln.split()[-1]))
            if last and c == last[1] + 1: fb.append((time.time(), h, ch, s - last[0]))
            last = (s, c)
        except Exception: pass
        time.sleep(0.1)
threading.Thread(target=sampler, daemon=True).start()

def app_metrics():
    out = {}
    try:
        for ln in get(f"http://127.0.0.1:{API}/metrics?format=prometheus").splitlines():
            if ln.startswith("#") or "wasm" not in ln: continue
            k, v = ln.rsplit(" ", 1)
            if k.split("{")[0].endswith(("_sum", "_count")): out[k] = float(v)
    except Exception as e: out["err"] = str(e)
    return out

logpos = [0]
def new_zk_lines():
    with open(LOG, "rb") as f:
        f.seek(logpos[0]); d = f.read(); logpos[0] += len(d)
    return [l for l in d.decode(errors="replace").splitlines() if l.startswith("ZKTIME")]

def tx(args, gas=400_000_000):
    r = sh([B, "tx"] + args + ["--from", "owner", "--gas", str(gas), "--fees", f"{gas//20+1}uterp", "-y"] + COMMON)
    d = json.loads(r.stdout) if r.stdout.strip().startswith("{") else {"code": -1, "raw_log": r.stderr[-300:]}
    if d.get("code", 0) != 0: return d
    for _ in range(60):
        time.sleep(0.5)
        q = sh([B, "q", "tx", d["txhash"], "--node", NODE, "-o", "json"])
        if q.returncode == 0 and q.stdout.strip(): return json.loads(q.stdout)
    return {"code": -2}

rows = json.load(open(f"{FX}/proofs.json"))
P = [base64.b64encode(bytes.fromhex(r["proof"])).decode() for r in rows]
I = [base64.b64encode(bytes.fromhex(r["instances"])).decode() for r in rows]
res = {"zkid": ZKID, "runs": [], "uptime": []}
new_zk_lines()
if ADDR == "new":
    st = tx(["wasm", "store", "/tmp/hs-p2-opt-out/cw_zk_bench.wasm"])
    cid = next(a["value"] for e in st["events"] if e["type"] == "store_code" for a in e["attributes"] if a["key"] == "code_id")
    it = tx(["wasm", "instantiate", cid, "{}", "--label", "zk-time", "--no-admin"])
    ADDR = next(a["value"] for e in it["events"] if e["type"] == "instantiate" for a in e["attributes"] if a["key"] == "_contract_address")
res["contract"] = ADDR; new_zk_lines()

def fin(h):
    # finalize_block for block h is sampled while consensus_height == h (latest_block_height is still h-1).
    v = [x[3] for x in fb if x[2] == h]
    return round(v[0] * 1000, 3) if len(v) == 1 else None

def run(name, msg, n, rep):
    m0 = app_metrics(); t = tx(["wasm", "execute", ADDR, json.dumps(msg)]); time.sleep(1.2); m1 = app_metrics()
    h = int(t.get("height", 0) or 0)
    dm = {k: m1[k] - m0.get(k, 0) for k in m1 if k != "err" and m1[k] != m0.get(k)}
    r = {"name": name, "n": n, "rep": rep, "code": t.get("code"), "gas_used": int(t.get("gas_used") or 0), "height": h,
         "finalize_ms": fin(h), "wasm_metric_delta": dm, "zk": new_zk_lines()}
    print(json.dumps({k: r[k] for k in ("name", "rep", "code", "gas_used", "finalize_ms", "zk")}), flush=True)
    res["runs"].append(r)

if MODE == "single":
    run("verify_1_first", {"verify": {"zkid": ZKID, "proof": P[0], "instance": I[0]}}, 1, 0)
    run("verify_1_second", {"verify": {"zkid": ZKID, "proof": P[1], "instance": I[1]}}, 1, 0)
else:
    for rep in range(REPS):
        res["uptime"].append(sh(["uptime"]).stdout.strip())
        run("verify_1", {"verify": {"zkid": ZKID, "proof": P[rep % 40], "instance": I[rep % 40]}}, 1, rep)
        for n in (1, 2, 4, 8, 16, 32):
            run(f"baseline_{n}", {"baseline": {"zkid": ZKID, "proofs": P[:n], "instances": I[:n]}}, n, rep)
            run(f"batch_{n}", {"batch_verify": {"zkid": ZKID, "proofs": P[:n], "instances": I[:n]}}, n, rep)
    res["uptime"].append(sh(["uptime"]).stdout.strip())
res["fb_samples"] = fb
json.dump(res, open(OUT, "w"), indent=1)
