#!/usr/bin/env python3
"""Headstash claim page v2: headless browser e2e (loopback only, two local chains + hermes).

Reuses run_direct.py setup (fresh contract, vk3 circuit, eligibility, relay with the
claim_forward grant kind) and replaces its flow:
  * chain B (second terpd, stand-in for Penumbra) + hermes + ICS-20 channel; set_forward_channel;
  * builds the HAL document (all eligibility rows + root) and serves it on 127.0.0.1:3921/hal/,
    set_hal_source {uri: "127.0.0.1:3921/hal/hal.json", sha256, raw CIDv1};
  * publishes the v2 page (app.html, app.js, crypto.js, v2lib.js, steps.json, prover) to a fresh
    cw-headstash-frontend, set_frontend_contract, serves loader.html on 127.0.0.1:3921.
Browser phases (browser_e2e_v2.mjs, chromium headless; injected test EIP-1193 + test Penumbra providers):
  v2a: load from chain, browse instances by code id, select, HAL fetch+verify, tampered local HAL refused,
       every step's info modal, MetaMask eligibility (no network request carries the address),
       derive notes (seed signature x2, deterministic), register all notes.
  (harness accepts an anchor)
  v2b: fresh browser, load notes file, 1 ephemeral claim, 1 forward to a Zafu-shaped penumbra1 address
       (chain B rejects it -> error ack -> refund to the per-forward fallback), 1 forward to a chain-B
       address (advanced receiver; received on B), reuse of a receiver refused locally.
Test key generated in memory by run_direct; passed to the Node driver only via stdin; never printed.
"""
import base64, hashlib, json, os, re, sys, time
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE); sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "tools", "headstash", "e2e"))
import browser_e2e as be           # sets CUT_AHEAD / HSFE_ASSET_DIR defaults, publish(), serve(), fund(), bal()
import run_forward as rf           # chain B + hermes helpers
import run_direct as rd
import frontend_bench as fb
from run import tx, smart, REST, height

PAGE, PROVER, PORT = be.PAGE, be.PROVER, be.PORT
PW_JS = os.path.join(HERE, "browser_e2e_v2.mjs")
OUT = os.environ.get("BROWSER_OUT", "/tmp/hsfe-v2/browser-e2e-v2.json")
SHOTS = os.environ.get("HSFE_SHOTS", "/tmp/hsfe-v2/shots")
R = rd.R

def page_files():
    names = [(PAGE, n) for n in ("app.html", "app.js", "crypto.js", "v2lib.js", "steps.json")] + [(PROVER, "zk_headstash.js"), (PROVER, "zk_headstash_bg.wasm")]
    return [{"path": n, "raw": open(os.path.join(d, n), "rb").read(), "stored": fb.gz(open(os.path.join(d, n), "rb").read()),
             "ct": fb.CT.get(os.path.splitext(n)[1], "application/octet-stream")} for d, n in names]
be.page_files = page_files

def raw_cid(digest):
    b = bytes([1, 0x55, 0x12, 32]) + digest
    return "b" + base64.b32encode(b).decode().lower().rstrip("=")

def build_hal(contract, notes, path):
    camp = smart(contract, {"epoch_campaign": {}}); reg = camp["registration"]
    root = base64.b64decode(reg["eligibility_root"]).hex(); depth = int(reg["eligibility_depth"])
    rows = [r for k, v in notes.items() if isinstance(v, list) for r in v if isinstance(r, dict) and "path" in r]
    keep = ("identity", "note_index", "leaf_index", "path", "class", "amount")
    hal = {"v": 1, "kind": "terp-headstash-hal", "eligibility_root": root, "depth": depth, "rows": [{k: r[k] for k in keep if k in r} for r in rows]}
    raw = json.dumps(hal, separators=(",", ":")).encode(); open(path, "wb").write(raw)
    d = hashlib.sha256(raw).digest()
    return {"sha256": d.hex(), "cid": raw_cid(d), "bytes": len(raw), "rows": len(rows), "depth": depth, "root": root,
            "depth_ok": all(len(r["path"]) == depth for r in rows)}

def browser(phase, payload):
    import subprocess
    env = dict(os.environ, NODE_PATH=be.PW_NODE_PATH)
    p = subprocess.run([os.environ.get("NODE_BIN", "node"), PW_JS, phase], input=json.dumps(payload), capture_output=True, text=True, env=env, timeout=5400)
    try: out = json.loads(p.stdout.strip().splitlines()[-1])
    except Exception: out = {"ok": False, "stdout": p.stdout[-800:], "stderr": p.stderr[-1500:]}
    out["stderr_tail"] = p.stderr[-1500:]
    return out

def keep(o, *drop): return {k: v for k, v in o.items() if k not in drop}

def flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut):
    os.makedirs(SHOTS, exist_ok=True)
    be.fund(contract)
    # chain B + channel + allowlist
    rf.chain_b(); ch_a, ch_b = rf.setup_ibc(OWNER)
    if not ch_a: return
    rf.hermes_start()
    res = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_forward_channel": {"channel_id": ch_a, "timeout_seconds": 600, "enabled": True}}))
    rd.step("v2_set_forward_channel", res.get("code") == 0, "%s %s" % (ch_a, res.get("raw_log", "")[:120]))
    recv_b = rf.bterpd("keys", "show", "recv", "-a", "--keyring-backend", "test", "--home", rf.HB).strip()
    work = os.path.join(rd.RUN, "browser-v2"); os.makedirs(os.path.join(work, "hal"), exist_ok=True)
    hal = build_hal(contract, notes, os.path.join(work, "hal", "hal.json"))
    R["hal"] = hal; rd.step("v2_hal_built", hal["depth_ok"] and hal["rows"] > 0, json.dumps(keep(hal, "root")))
    src = {"uri": "127.0.0.1:%d/hal/hal.json" % PORT, "sha256": hal["sha256"], "cid": hal["cid"]}
    res = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_hal_source": {"source": src}}))
    rd.step("v2_set_hal_source", res.get("code") == 0 and smart(contract, {"hal_source": {}})["uri"] == src["uri"], res.get("raw_log", "")[:160])
    tampered = os.path.join(work, "hal-tampered.json"); raw = bytearray(open(os.path.join(work, "hal", "hal.json"), "rb").read())
    i = raw.find(b'"leaf_index":') + 13; raw[i] = ord("9") if raw[i] != ord("9") else ord("8"); open(tampered, "wb").write(bytes(raw))
    be.publish(contract)
    lt = open(os.path.join(PAGE, "loader.html")).read()
    for k, v in {"__REST__": REST, "__RELAY__": rd.RELAY_URL, "__CHAIN__": rd.CHAIN_ID, "__HEADSTASH__": contract, "__DENOM__": "uterp",
                 "__GP__": str(rd.GP), "__CG__": str(rd.CLAIM_GAS), "__RG__": str(rd.REGISTER_GAS), "__FG__": "900000",
                 "__EXTRA_CONNECT__": "http://127.0.0.1:%d" % PORT, "__MANIFEST_SHA__": ""}.items():
        lt = lt.replace(k, v)
    open(os.path.join(work, "loader.html"), "w").write(lt)
    R["page"]["loader_bytes"] = len(lt.encode()); R["page"]["loader_csp"] = re.search(r'content="([^"]+)"', lt).group(1)
    srv = be.serve(work)
    try:
        url = "http://127.0.0.1:%d/loader.html" % PORT
        nr = os.path.join(work, "notes-after-register.json")
        a = browser("v2a", {"url": url, "key_hex": "%064x" % evm_priv, "evm_addr": "0x" + (evm_addr.hex() if isinstance(evm_addr, (bytes, bytearray)) else str(evm_addr).replace("0x", "")), "crypto": os.path.join(PAGE, "crypto.js"), "v2lib": os.path.join(PAGE, "v2lib.js"),
                            "tampered": tampered, "contract": contract, "out_derive": os.path.join(work, "notes-after-derive.json"), "out": nr, "shots": SHOTS})
        R["browser_v2a"] = keep(a, "log"); R["browser_v2a_log"] = a.get("log", "")[-6000:]
        rd.step("v2_page_loaded_from_chain", a.get("loaded") is True, a.get("error", "") or "manifest v%s, %d ms" % (a.get("version"), a.get("load_ms", 0)))
        rd.step("v2_browse_instances_by_code_id", (a.get("instances") or 0) >= 1 and a.get("selected") == contract, "code %s: %s instance(s)" % (a.get("code_ids"), a.get("instances")))
        rd.step("v2_hal_fetched_and_verified", a.get("hal_state") == "ok" and a.get("hal_sha") == hal["sha256"], a.get("hal_text", "")[:200])
        rd.step("v2_hal_tampered_local_file_refused", a.get("hal_tamper_state") == "error" and a.get("hal_restored") == "ok", a.get("hal_tamper_err", "")[:200])
        rd.step("v2_info_modal_every_step", a.get("modals_ok") == a.get("modals") and (a.get("modals") or 0) >= 9, "%s/%s" % (a.get("modals_ok"), a.get("modals")))
        rd.step("v2_eligibility_local_via_eip1193", a.get("elig_notes") == len(notes["evm"]) and a.get("leak") == [], "notes %s; requests during lookup %d, carrying the address %s" % (a.get("elig_notes"), a.get("lookup_requests", -1), a.get("leak")))
        ok, nf = be.canon_ok(nr) if a.get("saved") else (False, {})
        st = [n["st"] for n in nf.get("notes", [])]
        rd.step("v2_notes_derived_from_wallet_signature", nf.get("derivation") == "eip1193-seed-v2" and a.get("seed_signs") == 2 and "%064x" % evm_priv not in open(nr).read() if ok else False,
                "derivation %s, seed signatures %s, reward %s" % (nf.get("derivation"), a.get("seed_signs"), nf.get("reward_addr")))
        rd.step("v2_reward_default_ephemeral", bool(nf.get("reward_priv")) and nf.get("reward_addr", "").startswith("terp1"), nf.get("reward_addr", ""))
        rd.step("v2_page_registered_all_notes", ok and len(st) == len(notes["evm"]) and set(st) == {"registered"}, json.dumps(a.get("counts")))
        if set(st) != {"registered"}: return
        rt = smart(contract, {"registration_tree": {}})
        while height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
        res = tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}}))
        rd.step("v2_accept_anchor", res.get("code") == 0, res.get("raw_log", "")[:120])
        c0 = be.bal(contract); bb0 = rf.b_balances(recv_b)
        b = browser("v2b", {"url": url, "notes": nr, "recv_b": recv_b, "v2lib": os.path.join(PAGE, "v2lib.js"), "out": os.path.join(work, "notes-after-claim.json"), "shots": SHOTS})
        R["browser_v2b"] = keep(b, "log"); R["browser_v2b_log"] = b.get("log", "")[-8000:]
        ok2, nf2 = be.canon_ok(os.path.join(work, "notes-after-claim.json")) if b.get("saved") else (False, {})
        spent = [n for n in nf2.get("notes", []) if n["st"] == "spent"]
        eph = [n for n in spent if n.get("mode") == "ephemeral"]; fwd = [n for n in spent if n.get("mode") == "forward"]
        rd.step("v2_ephemeral_claim", len(eph) == 1 and be.bal(eph[0]["dest"]) == int(eph[0]["amt"]), "%s holds %s" % (eph[0]["dest"], be.bal(eph[0]["dest"])) if eph else json.dumps(b.get("error")))
        zf = [n for n in fwd if n["fwd"]["receiver"].startswith("penumbra1")]; bf = [n for n in fwd if n["fwd"]["receiver"] == recv_b]
        R["forwards"] = [n["fwd"] for n in fwd]
        okz = zf and rf.wait_for(lambda: be.bal(zf[0]["fwd"]["fallback"]) == int(zf[0]["amt"]), 150)
        rd.step("v2_forward_zafu_shaped_receiver_refunded_to_fresh_fallback", bool(okz), "receiver %s… (test Penumbra provider; chain B is terpd so it error-acks) fallback %s bal %s" % (zf[0]["fwd"]["receiver"][:30], zf[0]["fwd"]["fallback"], be.bal(zf[0]["fwd"]["fallback"])) if zf else "no zafu forward")
        okb = bf and rf.wait_for(lambda: sum(v - bb0.get(d, 0) for d, v in rf.b_balances(recv_b).items() if d.startswith("ibc/")) == int(bf[0]["amt"]), 150)
        rd.step("v2_forward_received_on_chain_b", bool(okb), "recv %s balances %s" % (recv_b, rf.b_balances(recv_b)))
        fbs = [n["fwd"]["fallback"] for n in fwd]; dsts = [n["dest"] for n in eph] + fbs + [n["fwd"]["receiver"] for n in fwd]
        rd.step("v2_all_destinations_distinct", len(dsts) == len(set(dsts)) and len(fbs) == 2, json.dumps(dsts)[:300])
        rd.step("v2_receiver_reuse_refused_locally", "already used" in (b.get("reuse_err") or ""), (b.get("reuse_err") or "")[:200])
        rd.step("v2_pending_forwards_cleared", rf.wait_for(lambda: len(smart(contract, {"pending_forwards": {}})) == 0, 90), "")
        R["prove_ms"] = [n.get("prove_ms") for n in spent]; R["escrow_delta"] = c0 - be.bal(contract)
    finally:
        srv.shutdown(); rf.hermes_stop()
        json.dump(R, open(OUT, "w"), indent=1)

rd.flow = flow
if __name__ == "__main__":
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    try: rd.main()
    except Exception as e:
        import traceback; traceback.print_exc(); rd.step("harness_exception", False, repr(e)[:300])
    finally:
        rf.hermes_stop(); rd.save()
        p = sum(s["pass"] for s in R["steps"]); f = len(R["steps"]) - p
        rd.log("E2E_RESULT passed=%d failed=%d report=%s" % (p, f, os.path.join(rd.RUN, "report.json")))
        sys.exit(1 if f else 0)
