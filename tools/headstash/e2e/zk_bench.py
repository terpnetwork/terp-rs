#!/usr/bin/env python3
"""On-chain verify benchmark (measurement only). Local chain on 127.0.0.1 only.
Usage: zk_bench.py <terpd> <home> <chain-id> <rpc-port> <bench.wasm> <fixture-dir> <zkid> <out.json>
Keys are referenced by keyring name only; nothing secret is read."""
import json, subprocess, sys, time, base64, re
B, H, CID, PORT, WASM, FX, ZKID, OUT = sys.argv[1:9]
ZKID = int(ZKID); NODE = f"tcp://127.0.0.1:{PORT}"
COMMON = ["--home", H, "--keyring-backend", "test", "--chain-id", CID, "--node", NODE, "-o", "json"]

def sh(args):
    return subprocess.run(args, capture_output=True, text=True)

def tx(args, gas=400_000_000):
    r = sh([B, "tx"] + args + ["--from", "owner", "--gas", str(gas), "--fees", f"{gas//20+1}uterp", "-y"] + COMMON)
    d = json.loads(r.stdout or "{}") if r.stdout.strip().startswith("{") else {"code": -1, "raw_log": r.stderr[-400:]}
    if d.get("code", 0) != 0:
        return {"code": d.get("code"), "raw_log": d.get("raw_log"), "gas_used": None}
    h = d["txhash"]
    for _ in range(60):
        time.sleep(1)
        q = sh([B, "q", "tx", h, "--node", NODE, "-o", "json"])
        if q.returncode == 0 and q.stdout.strip():
            t = json.loads(q.stdout); t["_hash"] = h; return t
    return {"code": -2, "raw_log": "timeout"}

def attrs(t):
    out = {}
    for e in t.get("events", []):
        if e["type"] == "wasm":
            for a in e["attributes"]:
                out[a["key"]] = a["value"]
    return out

def block_time(height):
    # wall time between this block and the previous (proposal-to-commit not exposed).
    a = json.loads(sh(["curl", "-s", f"http://127.0.0.1:{PORT}/block?height={height}"]).stdout)["result"]["block"]["header"]["time"]
    b = json.loads(sh(["curl", "-s", f"http://127.0.0.1:{PORT}/block?height={int(height)-1}"]).stdout)["result"]["block"]["header"]["time"]
    return a, b

rows = json.load(open(f"{FX}/proofs.json"))
P = [base64.b64encode(bytes.fromhex(r["proof"])).decode() for r in rows]
I = [base64.b64encode(bytes.fromhex(r["instances"])).decode() for r in rows]
res = {"zkid": ZKID, "runs": []}

st = tx(["wasm", "store", WASM])
code_id = next(a["value"] for e in st["events"] if e["type"] == "store_code" for a in e["attributes"] if a["key"] == "code_id")
res["store_code_gas"] = int(st["gas_used"])
inst = tx(["wasm", "instantiate", code_id, "{}", "--label", "zk-bench", "--no-admin"])
addr = next(a["value"] for e in inst["events"] if e["type"] == "instantiate" for a in e["attributes"] if a["key"] == "_contract_address")
res["contract"] = addr

def run(name, msg, n):
    t = tx(["wasm", "execute", addr, json.dumps(msg)])
    r = {"name": name, "n": n, "code": t.get("code"), "gas_used": int(t["gas_used"]) if t.get("gas_used") else None,
         "height": t.get("height"), "attrs": attrs(t), "raw_log": (t.get("raw_log") or "")[:300],
         "tx_bytes": len(base64.b64decode(t["tx"]["value"])) if isinstance(t.get("tx"), dict) and "value" in t["tx"] else None}
    print(json.dumps({k: r[k] for k in ("name", "n", "code", "gas_used", "attrs")}), flush=True)
    res["runs"].append(r); return r

ONLY_NEG = len(sys.argv) > 9
if ONLY_NEG:
    addr = sys.argv[9]
for n in (() if ONLY_NEG else (1, 2, 4, 8, 16, 32)):
    run(f"baseline_{n}", {"baseline": {"zkid": ZKID, "proofs": P[:n], "instances": I[:n]}}, n)
if not ONLY_NEG: run("verify_1", {"verify": {"zkid": ZKID, "proof": P[0], "instance": I[0]}}, 1)
if not ONLY_NEG: run("verify_1_repeat", {"verify": {"zkid": ZKID, "proof": P[1], "instance": I[1]}}, 1)
for n in (() if ONLY_NEG else (1, 2, 4, 8, 16, 32)):
    run(f"batch_{n}", {"batch_verify": {"zkid": ZKID, "proofs": P[:n], "instances": I[:n]}}, n)

# negatives
pb = bytearray(bytes.fromhex(rows[0]["proof"])); pb[100] ^= 1
forged = base64.b64encode(bytes(pb)).decode()
ib = bytearray(bytes.fromhex(rows[0]["instances"]))
wr = bytearray(ib); wr[32:64] = bytes.fromhex(rows[1]["root"])          # limb 1 = root
# limbs: 0 ctx, 1 root, 2 nullifier, 3-4 destination, 5 amount
wa = bytearray(ib); wa[160:192] = bytes.fromhex(rows[1]["instances"])[160:192] # 1 TERP note claimed as 2 TERP (same set)
wn = bytearray(ib); wn[64:96] = bytes.fromhex(rows[1]["instances"])[64:96]     # another note nullifier
wrong_nf = base64.b64encode(bytes(wn)).decode()
wrong_root = base64.b64encode(bytes(wr)).decode(); wrong_pub = base64.b64encode(bytes(wa)).decode()
run("neg_forged_proof", {"verify": {"zkid": ZKID, "proof": forged, "instance": I[0]}}, 1)
run("neg_wrong_root", {"verify": {"zkid": ZKID, "proof": P[0], "instance": wrong_root}}, 1)
run("neg_amount_1to2_terp", {"verify": {"zkid": ZKID, "proof": P[0], "instance": wrong_pub}}, 1)
run("neg_wrong_nullifier", {"verify": {"zkid": ZKID, "proof": P[0], "instance": wrong_nf}}, 1)
run("neg_proof_swapped", {"verify": {"zkid": ZKID, "proof": P[1], "instance": I[0]}}, 1)
run("neg_batch8_one_forged", {"batch_verify": {"zkid": ZKID, "proofs": [forged] + P[1:8], "instances": I[:8]}}, 8)
run("neg_unknown_zkid", {"verify": {"zkid": 999, "proof": P[0], "instance": I[0]}}, 1)
json.dump(res, open(OUT, "w"), indent=1)
