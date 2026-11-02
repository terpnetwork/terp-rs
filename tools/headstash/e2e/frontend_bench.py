#!/usr/bin/env python3
"""cw-headstash-frontend local e2e + chunk-size benchmark (loopback only).

Needs a chain from bootstrap-chain.sh. Stores cw_headstash_frontend.wasm, and per
chunk size instantiates a fresh frontend contract (owner = keyring "owner"),
publishes a manifest of the real prover assets (deterministic gzip, mtime 0),
uploads every chunk from keyring "stranger" (uploads are permissionless), then
downloads each file back over REST smart queries and verifies chunk, stored and
raw sha256. Writes $BENCH_OUT (json). No secrets are read or printed.
"""
import base64, gzip, hashlib, io, json, os, sys, time
import requests
import run as b
from run import tx, attr, smart, gas_used, log, REST

FE_WASM = os.environ.get("FE_WASM", "/tmp/hsfe/opt-out/cw_headstash_frontend.wasm")
ASSETS = os.environ["HSFE_ASSET_DIR"]
EXTRA = [p for p in os.environ.get("HSFE_EXTRA", "").split(":") if p]
SIZES = [int(s) * 1024 for s in os.environ.get("BENCH_KIB", "16,32,64,96,128").split(",")]
OUT = os.environ.get("BENCH_OUT", "/tmp/hsfe/frontend-onchain-bench.json")
GAS_PRICE = 0.025
CT = {".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html", ".json": "application/json", ".css": "text/css"}

def gz(raw):
    bio = io.BytesIO()
    with gzip.GzipFile(fileobj=bio, mode="wb", compresslevel=9, mtime=0, filename="") as f: f.write(raw)
    return bio.getvalue()

def files():
    paths = [os.path.join(ASSETS, n) for n in ("zk_headstash.js", "zk_headstash_bg.wasm", "worker.js")] + EXTRA
    out = []
    for p in paths:
        raw = open(p, "rb").read()
        out.append({"path": os.path.basename(p), "raw": raw, "stored": gz(raw),
                    "ct": CT.get(os.path.splitext(p)[1], "application/octet-stream")})
    return out

def manifest(fs, chunk, version=1):
    ents, chunks = [], []
    for f in fs:
        cs = [f["stored"][i:i + chunk] for i in range(0, len(f["stored"]), chunk)]
        chunks.append(cs)
        ents.append({"path": f["path"], "content_type": f["ct"], "encoding": "gzip", "size": len(f["raw"]),
                     "sha256": hashlib.sha256(f["raw"]).hexdigest(), "stored_size": len(f["stored"]),
                     "chunks": [hashlib.sha256(c).hexdigest() for c in cs]})
    entry = next((f["path"] for f in fs if f["path"].endswith(".html")), fs[0]["path"])
    return {"version": version, "entry": entry, "files": ents, "note": "bench", "activate": True}, chunks

def rest_smart(contract, msg):
    q = base64.b64encode(json.dumps(msg).encode()).decode()
    r = requests.get("%s/cosmwasm/wasm/v1/contract/%s/smart/%s" % (REST, contract, q), timeout=60)
    r.raise_for_status(); return r.json()["data"]

def main():
    R = {"wasm_sha256": hashlib.sha256(open(FE_WASM, "rb").read()).hexdigest(), "wasm_bytes": os.path.getsize(FE_WASM),
         "gas_price_uterp": GAS_PRICE, "runs": []}
    res = tx("owner", 20000000, "wasm", "store", FE_WASM)
    code_id = attr(res, "code_id"); assert res.get("code") == 0 and code_id, res.get("raw_log")
    R["code_id"] = code_id; R["store_gas"] = gas_used(res)
    fs = files()
    R["files"] = [{"path": f["path"], "raw": len(f["raw"]), "gzip": len(f["stored"]),
                   "sha256": hashlib.sha256(f["raw"]).hexdigest()} for f in fs]
    owner = b.terpd("keys", "show", "owner", "-a", "--keyring-backend", "test", "--home", b.HOME).strip()
    ok_all = True
    # chunk cap: wasmvm rejects storage values > 128 KiB, so the contract refuses max_chunk_bytes above it
    res = tx("owner", 2000000, "wasm", "instantiate", code_id, json.dumps({"owner": owner, "max_chunk_bytes": 256 * 1024}),
             "--label", "hsfe-256k", "--admin", owner)
    R["neg_instantiate_256k_rejected"] = res.get("code", 0) != 0
    R["measured_wasmvm_value_limit"] = "pre-fix build (cap 512 KiB): a 262144-byte chunk failed with Value too big. Tried to write 262144 bytes to storage, limit is 131072"
    ok_all &= R["neg_instantiate_256k_rejected"]
    for chunk in SIZES:
        res = tx("owner", 2000000, "wasm", "instantiate", code_id,
                 json.dumps({"owner": owner, "max_chunk_bytes": chunk}), "--label", "hsfe-%d" % chunk, "--admin", owner)
        fe = attr(res, "_contract_address"); assert res.get("code") == 0 and fe, res.get("raw_log")
        m, chunks = manifest(fs, chunk)
        # negative: a stranger may not publish
        neg = tx("stranger", 3000000, "wasm", "execute", fe, json.dumps({"publish_manifest": {"manifest": m}}))
        res = tx("owner", 3000000, "wasm", "execute", fe, json.dumps({"publish_manifest": {"manifest": m}}))
        assert res.get("code") == 0, res.get("raw_log")
        pub_gas = gas_used(res)
        # negative: tampered chunk rejected
        bad = bytearray(chunks[0][0]); bad[-1] ^= 1
        tamper = tx("stranger", 3000000, "wasm", "execute", fe, json.dumps(
            {"upload_chunk": {"version": 1, "file": 0, "index": 0, "data": base64.b64encode(bytes(bad)).decode()}}))
        gases, t0, h0 = [], time.time(), b.height()
        for fi, cs in enumerate(chunks):
            for ci, c in enumerate(cs):
                g = 600000 + 64 * len(c)  # generous; actual gas_used is recorded
                res = tx("stranger", g, "wasm", "execute", fe, json.dumps(
                    {"upload_chunk": {"version": 1, "file": fi, "index": ci, "data": base64.b64encode(c).decode()}}))
                assert res.get("code") == 0, (chunk, fi, ci, res.get("raw_log", "")[:300])
                gases.append(gas_used(res))
        up_s, blocks = time.time() - t0, b.height() - h0
        st = smart(fe, {"status": {"version": 1}}); cfg = smart(fe, {"config": {}})
        # download over REST and verify everything like loader.html does
        t1, ver = time.time(), True
        man = rest_smart(fe, {"manifest": {}})
        for fi, e in enumerate(man["files"]):
            buf = b""
            for ci, h in enumerate(e["chunks"]):
                d = base64.b64decode(rest_smart(fe, {"chunk": {"version": man["version"], "file": fi, "index": ci}}))
                ver &= hashlib.sha256(d).hexdigest() == h; buf += d
            ver &= len(buf) == e["stored_size"] and hashlib.sha256(gzip.decompress(buf)).hexdigest() == e["sha256"]
            ver &= e["sha256"] == hashlib.sha256(fs[fi]["raw"]).hexdigest()
        dl_s = time.time() - t1
        total_gas = sum(gases) + pub_gas
        run = {"chunk_kib": chunk // 1024, "contract": fe, "upload_txs": len(gases), "publish_gas": pub_gas,
               "upload_gas_total": sum(gases), "upload_gas_max": max(gases), "upload_gas_min": min(gases),
               "total_gas": total_gas, "fee_uterp_at_0.025": int(total_gas * GAS_PRICE),
               "upload_wall_s": round(up_s, 1), "upload_blocks": blocks, "download_verify_s": round(dl_s, 2),
               "status": st, "current": cfg.get("current"), "verified": bool(ver),
               "neg_stranger_publish_rejected": neg.get("code", 0) != 0,
               "neg_tampered_chunk_rejected": tamper.get("code", 0) != 0}
        ok = ver and st["complete"] and cfg.get("current") == 1 and run["neg_stranger_publish_rejected"] and run["neg_tampered_chunk_rejected"]
        ok_all &= ok
        log(json.dumps(run)); R["runs"].append(run)
        json.dump(R, open(OUT, "w"), indent=1)
    R["ok"] = bool(ok_all); json.dump(R, open(OUT, "w"), indent=1)
    log("BENCH_DONE ok=%s" % ok_all)

if __name__ == "__main__":
    main()
