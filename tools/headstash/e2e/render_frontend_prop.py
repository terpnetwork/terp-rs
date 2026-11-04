#!/usr/bin/env python3
"""Render the Headstash claim-page frontend DAO proposal (generic; no addresses hard-coded).

One DAO proposal (via the DAO's pre-propose, submitted with mainnet_deploy dao-propose):
  1. instantiate2 cw-headstash-frontend (code --code-id, admin = owner = the DAO, max_chunk_bytes 128 KiB,
     salt --salt); the address is predicted here (cosmwasm instantiate2, fix_msg = false);
  2. publish_manifest {activate: true} of the deterministic bundle (gzip -9, mtime 0) on that address;
  3. set_frontend_contract {addr: predicted} on every --target (cw-headstash instances owned by the DAO).
Chunks are not in the proposal: after execution anyone uploads them with mainnet_deploy chunk-upload
(permissionless UploadChunk, sha-checked on chain) from the chunk plan written here.
The loader (not on chain) is rendered per target; its PIN is the sha256 of the manifest JSON exactly as
the contract returns it (field order of cw-headstash-frontend Manifest/FileEntry, compact).

Outputs in --out: <prefix>.propose.json, <prefix>.chunks.json, loader-<name>.html, <prefix>.report.json.
"""
import argparse, base64, gzip, hashlib, io, json, os, sys, urllib.request

CT = {".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html", ".json": "application/json", ".css": "text/css"}
PAGE_FILES = [("page", "app.html"), ("page", "app.js"), ("page", "crypto.js"), ("page", "v2lib.js"), ("page", "steps.json"), ("prover", "zk_headstash.js"), ("prover", "zk_headstash_bg.wasm")]
CHUNK = 128 * 1024
HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_PAGE = os.path.normpath(os.path.join(HERE, "..", "frontend"))

# ---- bech32 (BIP-173) + instantiate2 (cosmwasm_std::instantiate2_address, no msg)
_CS = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def _poly(v):
    g = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]; c = 1
    for x in v:
        b = c >> 25; c = (c & 0x1ffffff) << 5 ^ x
        for i in range(5): c ^= g[i] if (b >> i) & 1 else 0
    return c
def _hrpx(h): return [ord(x) >> 5 for x in h] + [0] + [ord(x) & 31 for x in h]
def _conv(data, f, t, pad):
    acc = bits = 0; out = []; mx = (1 << t) - 1
    for v in data:
        acc = (acc << f) | v; bits += f
        while bits >= t: bits -= t; out.append((acc >> bits) & mx)
    if pad and bits: out.append((acc << (t - bits)) & mx)
    elif not pad and (bits >= f or ((acc << (t - bits)) & mx)): raise ValueError("bad padding")
    return out
def bech32_encode(hrp, raw):
    d = _conv(raw, 8, 5, True); p = _poly(_hrpx(hrp) + d + [0] * 6) ^ 1
    return hrp + "1" + "".join(_CS[x] for x in d + [(p >> 5 * (5 - i)) & 31 for i in range(6)])
def bech32_decode(a):
    hrp, _, d = a.lower().rpartition("1"); v = [_CS.index(c) for c in d]
    if _poly(_hrpx(hrp) + v) != 1: raise ValueError("bad checksum: " + a)
    return hrp, bytes(_conv(v[:-6], 5, 8, False))
def instantiate2(checksum, creator, salt):
    L = lambda b: len(b).to_bytes(8, "big") + b
    key = b"wasm\x00" + L(checksum) + L(creator) + L(salt) + (0).to_bytes(8, "big")
    return hashlib.sha256(hashlib.sha256(b"module").digest() + key).digest()
def predict(hrp, checksum_hex, creator, salt):
    return bech32_encode(hrp, instantiate2(bytes.fromhex(checksum_hex), bech32_decode(creator)[1], salt))

# ---- bundle
def gz(raw):
    bio = io.BytesIO()
    with gzip.GzipFile(fileobj=bio, mode="wb", compresslevel=9, mtime=0, filename="") as f: f.write(raw)
    return bio.getvalue()
def bundle(page_dir, prover_dir, chunk=CHUNK):
    dirs = {"page": page_dir, "prover": prover_dir}; ents, chunks, fs = [], [], []
    for d, n in PAGE_FILES:
        raw = open(os.path.join(dirs[d], n), "rb").read(); st = gz(raw)
        cs = [st[i:i + chunk] for i in range(0, len(st), chunk)]; chunks.append(cs)
        fs.append({"path": n, "raw_size": len(raw), "gzip_size": len(st), "sha256": hashlib.sha256(raw).hexdigest(), "chunks": len(cs)})
        ents.append({"path": n, "content_type": CT.get(os.path.splitext(n)[1], "application/octet-stream"), "encoding": "gzip", "size": len(raw),
                     "sha256": hashlib.sha256(raw).hexdigest(), "stored_size": len(st), "chunks": [hashlib.sha256(c).hexdigest() for c in cs]})
    m = {"version": 1, "entry": "app.js", "files": ents, "note": "headstash claim page", "activate": True}
    return m, chunks, fs
def manifest_pin(m):  # == loader: sha256(JSON.stringify(<manifest as returned by the contract>))
    return hashlib.sha256(json.dumps(m, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
def bundle_sha(fs):  # one hash over every raw file (path + sha), in manifest order
    return hashlib.sha256("".join("%s %s\n" % (f["sha256"], f["path"]) for f in fs).encode()).hexdigest()

def b64j(o): return base64.b64encode(json.dumps(o, separators=(",", ":")).encode()).decode()
def proposal_msgs(dao, code_id, checksum_hex, salt, label, targets, m, hrp="terp", max_chunk=CHUNK):
    fe = predict(hrp, checksum_hex, dao, salt)
    msgs = [{"wasm": {"instantiate2": {"admin": dao, "code_id": int(code_id), "label": label,
                                       "msg": b64j({"owner": dao, "max_chunk_bytes": max_chunk}), "funds": [], "salt": base64.b64encode(salt).decode()}}},
            {"wasm": {"execute": {"contract_addr": fe, "msg": b64j({"publish_manifest": {"manifest": m}}), "funds": []}}}]
    msgs += [{"wasm": {"execute": {"contract_addr": t, "msg": b64j({"set_frontend_contract": {"addr": fe}}), "funds": []}}} for t in targets]
    return fe, msgs
def chunk_plan(fe, m, chunks):
    return {"contract": fe, "version": m["version"],
            "files": [{"path": e["path"], "chunks": [base64.b64encode(c).decode() for c in cs]} for e, cs in zip(m["files"], chunks)]}
def loader(page_dir, rest, relay, chain, headstash, denom, gp, cg, rg, pin, extra_connect="", fg=900000):
    lt = open(os.path.join(page_dir, "loader.html")).read()
    for k, v in {"__REST__": rest, "__RELAY__": relay, "__CHAIN__": chain, "__HEADSTASH__": headstash, "__DENOM__": denom,
                 "__GP__": str(gp), "__CG__": str(cg), "__RG__": str(rg), "__FG__": str(fg), "__EXTRA_CONNECT__": extra_connect, "__MANIFEST_SHA__": pin}.items():
        lt = lt.replace(k, v)
    assert "__" not in lt.split("<script")[1].split("const st")[0], "unfilled placeholder"
    return lt

def lcd(base, path):
    return json.loads(urllib.request.urlopen(base.rstrip("/") + path, timeout=20).read())
def smart(base, a, msg):
    return lcd(base, "/cosmwasm/wasm/v1/contract/%s/smart/%s" % (a, base64.b64encode(json.dumps(msg).encode()).decode()))["data"]

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dao", required=True); ap.add_argument("--code-id", type=int, required=True); ap.add_argument("--checksum", required=True)
    ap.add_argument("--target", action="append", required=True, help="NAME=ADDR of a cw-headstash instance (repeatable)")
    ap.add_argument("--salt", default="headstash-frontend:v1"); ap.add_argument("--label", default="headstash-frontend-v1")
    ap.add_argument("--page-dir", default=DEFAULT_PAGE); ap.add_argument("--prover-dir", required=True)
    ap.add_argument("--rest", required=True); ap.add_argument("--relay", required=True); ap.add_argument("--chain-id", required=True)
    ap.add_argument("--denom", required=True); ap.add_argument("--gas-price", required=True); ap.add_argument("--claim-gas", type=int, default=400000)
    ap.add_argument("--register-gas", type=int, default=12000000); ap.add_argument("--forward-gas", type=int, default=900000)
    ap.add_argument("--extra-connect", default="", help="space-separated extra connect-src origins (HAL host, IPFS gateways)"); ap.add_argument("--hrp", default="terp")
    ap.add_argument("--title", default="Headstash claim page: on-chain frontend (cw-headstash-frontend)")
    ap.add_argument("--out", required=True); ap.add_argument("--prefix", default="30-dao-prop-frontend")
    ap.add_argument("--verify-lcd", help="read-only LCD: check code checksum, target owners, frontend_contract unset, address free")
    a = ap.parse_args()
    if a.claim_gas != 400000: print("WARNING: claim gas %d != 400000 (deploy packet)" % a.claim_gas, file=sys.stderr)
    targets = [t.split("=", 1) for t in a.target]
    m, chunks, fs = bundle(a.page_dir, a.prover_dir)
    pin = manifest_pin(m); salt = a.salt.encode()
    fe, msgs = proposal_msgs(a.dao, a.code_id, a.checksum.lower(), salt, a.label, [t[1] for t in targets], m, a.hrp)
    checks = {}
    if a.verify_lcd:
        ci = lcd(a.verify_lcd, "/cosmwasm/wasm/v1/code/%d" % a.code_id)["code_info"]
        checks["code_checksum"] = ci["data_hash"].lower() == a.checksum.lower()
        for n, t in targets:
            checks[n + "_owner_is_dao"] = smart(a.verify_lcd, t, {"ownership": {}})["owner"] == a.dao
            checks[n + "_frontend_contract"] = smart(a.verify_lcd, t, {"frontend_contract": {}})
        try: lcd(a.verify_lcd, "/cosmwasm/wasm/v1/contract/" + fe); checks["predicted_address_free"] = False
        except Exception: checks["predicted_address_free"] = True
    files = "; ".join("%s %d B (gzip %d, %d chunk%s) sha256 %s" % (f["path"], f["raw_size"], f["gzip_size"], f["chunks"], "s" * (f["chunks"] > 1), f["sha256"]) for f in fs)
    desc = ("Publishes the Headstash claim page on chain and points both headstash instances at it. "
            "1) instantiate2 cw-headstash-frontend (code %d, checksum %s) at %s with admin and owner = this DAO, max_chunk_bytes %d, salt %r, label %r. "
            "2) publish_manifest v1 (activate on completion), manifest sha256 %s, files: %s. "
            "3) set_frontend_contract %s on %s. "
            "After execution the %d chunks are uploaded permissionlessly (UploadChunk is sha-checked against this manifest); "
            "version 1 becomes current when the last chunk lands. Nothing is paid out and no funds move."
            ) % (a.code_id, a.checksum.upper(), fe, CHUNK, a.salt, a.label, pin, files, fe, ", ".join("%s (%s)" % (n, t) for n, t in targets), sum(len(c) for c in chunks))
    body = {"propose": {"msg": {"propose": {"title": a.title, "description": desc, "msgs": msgs}}}}
    os.makedirs(a.out, exist_ok=True)
    pf = os.path.join(a.out, a.prefix + ".propose.json"); json.dump(body, open(pf, "w"), indent=1); open(pf, "a").write("\n")
    cf = os.path.join(a.out, a.prefix + ".chunks.json"); json.dump(chunk_plan(fe, m, chunks), open(cf, "w")); open(cf, "a").write("\n")
    loaders = {}
    for n, t in targets:
        lt = loader(a.page_dir, a.rest, a.relay, a.chain_id, t, a.denom, a.gas_price, a.claim_gas, a.register_gas, pin, a.extra_connect, a.forward_gas)
        lf = os.path.join(a.out, "loader-%s.html" % n); open(lf, "w").write(lt)
        loaders[n] = {"file": lf, "sha256": hashlib.sha256(lt.encode()).hexdigest(), "headstash": t}
    sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
    rep = {"dao": a.dao, "code_id": a.code_id, "checksum": a.checksum.upper(), "salt": a.salt, "salt_b64": base64.b64encode(salt).decode(), "label": a.label,
           "predicted_frontend": fe, "targets": dict(targets), "manifest_pin_sha256": pin, "bundle_sha256": bundle_sha(fs), "files": fs,
           "chunks_total": sum(len(c) for c in chunks), "stored_bytes": sum(e["stored_size"] for e in m["files"]),
           "propose_file": pf, "propose_sha256": sha(pf), "chunks_file": cf, "chunks_sha256": sha(cf), "loaders": loaders,
           "loader_cfg": {"rest": a.rest, "relay": a.relay, "chain_id": a.chain_id, "denom": a.denom, "gas_price": a.gas_price,
                          "claim_gas": a.claim_gas, "register_gas": a.register_gas}, "checks": checks}
    json.dump(rep, open(os.path.join(a.out, a.prefix + ".report.json"), "w"), indent=1)
    print(json.dumps({k: rep[k] for k in ("predicted_frontend", "manifest_pin_sha256", "bundle_sha256", "chunks_total", "propose_sha256", "chunks_sha256", "checks")}, indent=1))
    for n, l in loaders.items(): print("loader-%s.html sha256 %s" % (n, l["sha256"]))

if __name__ == "__main__":
    main()
