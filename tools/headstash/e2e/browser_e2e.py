#!/usr/bin/env python3
"""Headstash claim page: headless browser e2e (loopback only).

Reuses run_direct.py setup (fresh contract, vk3 circuit, eligibility, relay) and replaces
its flow: publishes the page (app.html, app.js, crypto.js, prover js+wasm) to a fresh
cw-headstash-frontend, sets frontend_contract on cw-headstash, serves loader.html on
127.0.0.1:3921, then Playwright (chromium headless) loads the page from on-chain chunks,
generates the notes file from the test EVM key, registers all notes, (test accepts an
anchor), re-imports the notes file, claims every note, and the test checks balances.
Test key is generated in memory by run_direct and passed to the browser only via stdin.
"""
import re
import base64, functools, hashlib, http.server, json, os, subprocess, sys, threading, time
os.environ.setdefault("CUT_AHEAD", "7200")  # 43 browser proofs + rounds need > 600 blocks of claim window
os.environ.setdefault("HSFE_ASSET_DIR", "/tmp/hsfe/prover")
import run_direct as rd
import frontend_bench as fb
from run import tx, attr, smart, gas_used, REST

PAGE = os.environ.get("HSFE_PAGE_DIR", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "frontend"))
PROVER = os.environ.get("HSFE_PROVER_DIR", "/tmp/hsfe/prover")
PW_JS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "browser_e2e.mjs")
PW_NODE_PATH = os.environ.get("PW_NODE_PATH", os.path.expanduser("~/.npm/_npx/bbb8a2c4738e2b0c/node_modules"))
PORT = 3921
CHUNK = int(os.environ.get("HSFE_CHUNK_KIB", "128")) * 1024
OUT = os.environ.get("BROWSER_OUT", "/tmp/hsfe/browser-e2e.json")
R = rd.R

def page_files():
    names = [(PAGE, "app.html"), (PAGE, "app.js"), (PAGE, "crypto.js"), (PROVER, "zk_headstash.js"), (PROVER, "zk_headstash_bg.wasm")]
    out = []
    for d, n in names:
        raw = open(os.path.join(d, n), "rb").read()
        out.append({"path": n, "raw": raw, "stored": fb.gz(raw), "ct": fb.CT.get(os.path.splitext(n)[1], "application/octet-stream")})
    return out

def publish(contract):
    res = tx("owner", 20000000, "wasm", "store", fb.FE_WASM); code_id = attr(res, "code_id")
    rd.step("fe_store", res.get("code") == 0 and bool(code_id), "code_id %s" % code_id)
    owner = R["addresses"]["OWNER_ADDR"]
    res = tx("owner", 2000000, "wasm", "instantiate", code_id, json.dumps({"owner": owner, "max_chunk_bytes": CHUNK}), "--label", "hs-frontend", "--admin", owner)
    fe = attr(res, "_contract_address"); rd.step("fe_instantiate", res.get("code") == 0 and bool(fe), fe or res.get("raw_log", "")[:200])
    fs = page_files(); m, chunks = fb.manifest(fs, CHUNK); m["entry"] = "app.js"; m["note"] = "headstash claim page"
    res = tx("owner", 3000000, "wasm", "execute", fe, json.dumps({"publish_manifest": {"manifest": m}})); rd.step("fe_publish", res.get("code") == 0, res.get("raw_log", "")[:160])
    n = 0
    for fi, cs in enumerate(chunks):
        for ci, c in enumerate(cs):
            res = tx("stranger", 600000 + 64 * len(c), "wasm", "execute", fe, json.dumps({"upload_chunk": {"version": 1, "file": fi, "index": ci, "data": base64.b64encode(c).decode()}}))
            n += res.get("code") == 0
    st = smart(fe, {"status": {"version": 1}}); rd.step("fe_upload", st.get("complete") is True, "%d chunk txs %s" % (n, json.dumps(st)))
    res = tx("owner", 500000, "wasm", "execute", contract, json.dumps({"set_frontend_contract": {"addr": fe}}))
    rd.step("headstash_set_frontend_contract", res.get("code") == 0 and smart(contract, {"frontend_contract": {}}) == fe, res.get("raw_log", "")[:160])
    R["page"] = {"frontend": fe, "files": [{"path": f["path"], "raw": len(f["raw"]), "gzip": len(f["stored"]), "sha256": hashlib.sha256(f["raw"]).hexdigest()} for f in fs],
                 "total_gzip": sum(len(f["stored"]) for f in fs), "app_only_gzip": sum(len(f["stored"]) for f in fs if not f["path"].startswith("zk_headstash")), "chunks": sum(len(c) for c in chunks)}
    return fe

def serve(dirpath):
    h = functools.partial(http.server.SimpleHTTPRequestHandler, directory=dirpath)
    h.log_message = lambda *a: None
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), h)
    threading.Thread(target=srv.serve_forever, daemon=True).start(); return srv

def browser(phase, payload):
    env = dict(os.environ, NODE_PATH=PW_NODE_PATH)
    p = subprocess.run([os.environ.get("NODE_BIN", "node"), PW_JS, phase], input=json.dumps(payload), capture_output=True, text=True, env=env, timeout=3600)
    try: out = json.loads(p.stdout.strip().splitlines()[-1])
    except Exception: out = {"ok": False, "stdout": p.stdout[-800:], "stderr": p.stderr[-800:]}
    return out

def bal(a):
    r = rd.requests.get("%s/cosmos/bank/v1beta1/balances/%s/by_denom" % (REST, a), params={"denom": "uterp"}, timeout=10).json()
    return int(r.get("balance", {}).get("amount", 0))

def canon_ok(path):
    o = json.load(open(path)); want = o.pop("sha256")
    return hashlib.sha256(json.dumps(o, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest() == want, o

OWED_UTERP = int(os.environ.get("HSFE_OWED_UTERP", str(200008 * 10**6 + 3500000)))  # mvk.csv: EVM 200008 + Sol 3.5 TERP

def fund(contract):
    have = bal(contract); need = OWED_UTERP - have
    if need > 0:
        res = tx("owner", 200000, "bank", "send", "owner", contract, "%duterp" % need)
        rd.step("fund_campaign_to_owed_total", res.get("code") == 0 and bal(contract) >= OWED_UTERP, "contract %d -> %d (owed %d)" % (have, bal(contract), OWED_UTERP))
    R["campaign_funded_uterp"] = bal(contract)

def flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut):
    fund(contract)
    fe = publish(contract)
    work = os.path.join(rd.RUN, "browser"); os.makedirs(work, exist_ok=True)
    lt = open(os.path.join(PAGE, "loader.html")).read()
    for k, v in {"__REST__": REST, "__RELAY__": rd.RELAY_URL, "__CHAIN__": rd.CHAIN_ID, "__HEADSTASH__": contract, "__DENOM__": "uterp",
                 "__GP__": str(rd.GP), "__CG__": str(rd.CLAIM_GAS), "__RG__": str(rd.REGISTER_GAS), "__MANIFEST_SHA__": ""}.items():
        lt = lt.replace(k, v)
    open(os.path.join(work, "loader.html"), "w").write(lt)
    R["page"]["loader_bytes"] = len(lt.encode()); R["page"]["loader_sha256"] = hashlib.sha256(lt.encode()).hexdigest()
    elig = os.path.join(work, "my_rows.jsonl")
    open(elig, "w").write("".join(json.dumps(r) + "\n" for r in notes["evm"]))
    srv = serve(work)
    try:
        reward = rd.new_wallet("page_reward")
        url = "http://127.0.0.1:%d/loader.html" % PORT
        # phase A: load from chain, generate notes file, register all notes
        a = browser("register", {"url": url, "key_hex": "%064x" % evm_priv, "elig": elig, "reward": reward, "out": os.path.join(work, "notes-after-register.json")})
        R["browser_register"] = {k: v for k, v in a.items() if k != "log"}; R["browser_register_log"] = a.get("log", "")[-4000:]
        rd.step("page_loaded_from_chain", a.get("loaded") is True, a.get("error", "") or "manifest v%s" % a.get("version"))
        ok, nf = canon_ok(os.path.join(work, "notes-after-register.json")) if a.get("saved") else (False, {})
        st = [n["st"] for n in nf.get("notes", [])]
        rd.step("notes_file_checksum_and_no_key", ok and "%064x" % evm_priv not in open(os.path.join(work, "notes-after-register.json")).read(), "sha256 ok %s" % ok)
        rd.step("page_registered_all_notes", len(st) == len(notes["evm"]) and set(st) == {"registered"}, json.dumps(a.get("counts")))
        if set(st) != {"registered"}: return
        # anchor (permissionless) between the two browser sessions
        rt = smart(contract, {"registration_tree": {}})
        while rd.height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
        res = tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}}))
        rd.step("accept_anchor", res.get("code") == 0, res.get("raw_log", "")[:120])
        # phase B: fresh browser, re-import notes file, claim every note
        b = browser("claim", {"url": url, "notes": os.path.join(work, "notes-after-register.json"), "out": os.path.join(work, "notes-after-claim.json"), "n": len(st)})
        R["browser_claim"] = {k: v for k, v in b.items() if k != "log"}; R["browser_claim_log"] = b.get("log", "")[-6000:]
        ok2, nf2 = canon_ok(os.path.join(work, "notes-after-claim.json")) if b.get("saved") else (False, {})
        st2 = [n["st"] for n in nf2.get("notes", [])]
        rd.step("page_claimed_all_notes", ok2 and set(st2) == {"spent"} and len(st2) == len(notes["evm"]), json.dumps(b.get("counts")))
        tot = 0; good = 0
        for n in nf2.get("notes", []):
            got = bal(n["dest"]); tot += got; good += got == int(n.get("amt") or -1)
        rd.step("claim_balances_match", good == len(st2) and good > 0, "%d/%d dest wallets hold exactly their note amount; total %d uterp" % (good, len(st2), tot))
        R["prove_ms"] = [n.get("prove_ms") for n in nf2.get("notes", [])]
        # replay: claim again from the same file must not mint (nullifier spent)
        c = browser("replay", {"url": url, "notes": os.path.join(work, "notes-after-register.json"), "out": os.path.join(work, "notes-replay.json"), "n": 1})
        tot2 = sum(bal(n["dest"]) for n in nf2.get("notes", []))
        rlog = c.get("log", ""); R["browser_replay_log"] = rlog[-20000:]; R["browser_replay"] = {k: c.get(k) for k in ("ok", "error", "counts")}
        # per note outcome of the replay: must be refused by the relay, or rejected on chain as spent; never a code-0 claim
        refused = len(re.findall(r"fee grant refused", rlog)); failed = len(re.findall(r"claim failed code [1-9]", rlog))
        codes = []
        for h in re.findall(r"claimed .*? tx ([0-9A-Fa-f]{64})", rlog):
            tr = rd.requests.get(REST + "/cosmos/tx/v1beta1/txs/" + h, timeout=10).json().get("tx_response", {})
            codes.append([tr.get("code"), (tr.get("raw_log") or "")[:160]])
        R["replay_tx_codes"] = codes
        oks = sum(1 for code, _ in codes if code == 0); rejected = refused + failed + sum(1 for code, _ in codes if code)
        rd.step("neg_replay_rejected_spent_nullifier", rejected == len(st2) and oks == 0,
                "%d/%d replays rejected (grant refused %d, tx failed %d, tx rejected-as-spent %d, tx ok %d)" % (rejected, len(st2), refused, failed, rejected - refused - failed, oks))
        rd.step("neg_replay_from_old_file_no_double_mint", tot2 == tot and tot > 0, "balances %d -> %d after re-claim attempt" % (tot, tot2))
    finally:
        srv.shutdown()
        json.dump(R, open(OUT, "w"), indent=1)

rd.flow = flow
if __name__ == "__main__":
    rd.main()
