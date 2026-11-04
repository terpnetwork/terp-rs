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
    names = [(PAGE, n) for n in ("app.html", "app.js", "crypto.js", "v2lib.js", "steps.json", "zafu-client.js")] + [(PROVER, "zk_headstash.js"), (PROVER, "zk_headstash_bg.wasm")]
    return [{"path": n, "raw": open(os.path.join(d, n), "rb").read(), "stored": fb.gz(open(os.path.join(d, n), "rb").read()),
             "ct": fb.CT.get(os.path.splitext(n)[1], "application/octet-stream")} for d, n in names]
be.page_files = page_files
import render_frontend_prop as rfp
MANIFEST_THEME = rfp.load_theme(rfp.DEFAULT_THEME)       # R12: manifest default = Azure Calcium
_fb_manifest = fb.manifest
def _manifest_with_theme(fs, chunk, version=1):
    m, chunks = _fb_manifest(fs, chunk, version); m["theme"] = MANIFEST_THEME; return m, chunks
fb.manifest = _manifest_with_theme
# headstash theme for the e2e: Azure Calcium with two tokens changed, so its source is distinguishable from the manifest default
HS_THEME = rfp.canon_theme({"light": dict(MANIFEST_THEME["light"], surface="#F4F9FF", accent="#3A6E9E"), "dark": dict(MANIFEST_THEME["dark"], surface="#0B1A2A"), "radius_px": 12})
MALICIOUS = {"light": {"bg": "red;}body{background:url(http://127.0.0.1:1/x.png)}"}, "dark": {"bg": "#000"}}
def rgb(h): return "rgb(%d, %d, %d)" % tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))
def theme_checks(contract, url):
    th = lambda tag, **kw: browser("theme", dict({"url": url, "tag": tag, "shots": SHOTS}, **kw))
    fakes = ["chrome-extension://" + "a" * 32, "chrome-extension://" + "b" * 16 + "c" * 16]
    a = th("manifest", fake_wallets=fakes); R["theme_manifest"] = keep(a, "log")
    wo = a.get("wallet_options") or []
    rd.step("v2_wallet_any_announced_provider_user_picks", len(wo) == 2 and all(f in " ".join(wo) for f in fakes) and a.get("wallet_select_visible") and fakes[1] in (a.get("wallet_detect") or ""),
            "%d options; picked: %s" % (len(wo), (a.get("wallet_detect") or "")[:120]))
    ok = a.get("ok") and all(a.get(m, {}).get("source") == "manifest" and a[m].get("mode") == m and a[m].get("body_bg") == rgb(MANIFEST_THEME[m]["bg"]) for m in ("light", "dark"))
    rd.step("v2_theme_fallback_manifest_default", bool(ok), "light %s / dark %s, source %s" % (a.get("light", {}).get("body_bg"), a.get("dark", {}).get("body_bg"), a.get("light", {}).get("source")) if a.get("ok") else a.get("error", ""))
    bad_tx = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_theme": {"theme": MALICIOUS}}))
    other = tx("stranger", 400000, "wasm", "execute", contract, json.dumps({"set_theme": {"theme": HS_THEME}}))
    rd.step("v2_theme_contract_rejects_malicious_and_non_owner", bad_tx.get("code") != 0 and other.get("code") != 0 and smart(contract, {"theme": {}}) is None,
            "malicious: %s | non-owner: %s" % (bad_tx.get("raw_log", "")[:100], other.get("raw_log", "")[:100]))
    res = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_theme": {"theme": HS_THEME}}))
    rd.step("v2_theme_set_on_headstash", res.get("code") == 0 and smart(contract, {"theme": {}}) == HS_THEME, res.get("raw_log", "")[:120])
    b = th("headstash", toggle=True); R["theme_headstash"] = keep(b, "log")
    ok = b.get("ok") and all(b.get(m, {}).get("source") == "headstash" and b[m].get("surface").upper() == HS_THEME[m]["surface"] and b[m].get("body_bg") == rgb(HS_THEME[m]["bg"]) and b[m].get("radius") == "12px" for m in ("light", "dark"))
    rd.step("v2_theme_loaded_from_headstash_contract", bool(ok), "light surface %s, dark surface %s, radius %s" % (b.get("light", {}).get("surface"), b.get("dark", {}).get("surface"), b.get("light", {}).get("radius")) if b.get("ok") else b.get("error", ""))
    rd.step("v2_theme_light_and_dark_render", bool(b.get("ok")) and b["light"]["body_bg"] != b["dark"]["body_bg"] and b["light"]["btn_bg"] == rgb(HS_THEME["light"]["accent"]) and b["dark"]["btn_bg"] == rgb(HS_THEME["dark"]["accent"]),
            "light body %s btn %s | dark body %s btn %s" % tuple(b.get(m, {}).get(k) for m in ("light", "dark") for k in ("body_bg", "btn_bg")))
    t1, t2, t3 = b.get("after_toggle", {}), b.get("after_reload", {}), b.get("after_toggle2", {})
    rd.step("v2_theme_toggle_persists", t1.get("mode") == "light" and t1.get("stored") == "light" and t2.get("mode") == "light" and t3.get("mode") == "dark" and t3.get("stored") == "dark",
            "dark pref -> toggle %s (stored %s) -> reload %s -> toggle %s" % (t1.get("mode"), t1.get("stored"), t2.get("mode"), t3.get("mode")))
    c = th("malicious", tamper=MALICIOUS); R["theme_malicious"] = keep(c, "log")
    l = c.get("light", {})
    ok = c.get("ok") and l.get("source") == "manifest" and l.get("rejected") == "headstash" and "url(" not in l.get("css", "x") and "red" not in l.get("css", "x") and l.get("body_bg") == rgb(MANIFEST_THEME["light"]["bg"])
    rd.step("v2_theme_malicious_rejected_in_page", bool(ok), "source %s rejected %s; %s" % (l.get("source"), l.get("rejected"), (c.get("log_rejects") or [""])[0][:140]))
    import shutil
    for n in ("light", "dark"):  # canonical screenshot names; the landing in pure Azure Calcium (manifest default)
        s = os.path.join(SHOTS, "theme-headstash-%s.png" % n)
        if os.path.exists(s): shutil.copyfile(s, os.path.join(SHOTS, "theme-%s.png" % n))
        s = os.path.join(SHOTS, "theme-manifest-%s.png" % n)
        if os.path.exists(s): shutil.copyfile(s, os.path.join(SHOTS, "azure-landing-%s.png" % n))
    # the claim phases (and their screenshots) run on the manifest default: put Azure Calcium back on the headstash
    res = tx("owner", 400000, "wasm", "execute", contract, json.dumps({"set_theme": {"theme": MANIFEST_THEME}}))
    rd.step("v2_theme_reset_to_azure", res.get("code") == 0 and smart(contract, {"theme": {}}) == MANIFEST_THEME, res.get("raw_log", "")[:120])

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
        theme_checks(contract, url)
        nr = os.path.join(work, "notes-after-register.json")
        a = browser("v2a", {"url": url, "key_hex": "%064x" % evm_priv, "evm_addr": "0x" + (evm_addr.hex() if isinstance(evm_addr, (bytes, bytearray)) else str(evm_addr).replace("0x", "")), "crypto": os.path.join(PAGE, "crypto.js"), "v2lib": os.path.join(PAGE, "v2lib.js"),
                            "tampered": tampered, "contract": contract, "out_derive": os.path.join(work, "notes-after-derive.json"), "out": nr, "shots": SHOTS})
        R["browser_v2a"] = keep(a, "log"); R["browser_v2a_log"] = a.get("log", "")[-6000:]
        la = a.get("landing") or {}
        rd.step("v2_landing_hides_claim_ui", a.get("loaded") is True and la.get("sheet_open") is False and la.get("visible") == [] and "none" in (a.get("hs_cur_before") or ""),
                "sheet open %s; visible claim tools %s; selected '%s'" % (la.get("sheet_open"), la.get("visible"), a.get("hs_cur_before")))
        sh = a.get("sheet") or {}
        rd.step("v2_sheet_open_close_focus", sh.get("open") is True and sh.get("aria_modal") == "true" and sh.get("focus_in") is True and sh.get("tab_escapes") == 0 and sh.get("bottom") == 0
                and sh.get("esc_closed") is True and sh.get("focus_back") is True and sh.get("close_btn_closed") is True and sh.get("focus_back_resume") is True and sh.get("anim") == "hs-up" and sh.get("anim_reduced") == "none",
                json.dumps(sh)[:300])
        exp = [("sections_open", {"eligibility": "current+open", "register": "locked", "save": "locked", "wallet": "locked", "claim": "locked"}),
               ("sections_eligible", {"eligibility": "done", "register": "current+open", "save": "locked", "wallet": "locked", "claim": "locked"}),
               ("sections_derived", {"eligibility": "done", "register": "current+open", "save": "done", "wallet": "locked", "claim": "locked"}),
               ("sections_registered", {"eligibility": "done", "register": "done", "save": "done", "wallet": "current+open", "claim": "locked"})]
        bad = [(k, a.get(k)) for k, v in exp if a.get(k) != v]
        rd.step("v2_progressive_reveal", not bad, "open→eligible→derived→registered as expected" if not bad else json.dumps(bad)[:300])
        def chain_priv():
            rt = smart(contract, {"registration_tree": {}}); dc = smart(contract, {"direct_claim": {}})
            return int(rt["count"]), int(dc["claimed"])
        pa = a.get("privacy") or {}; cr, cc = chain_priv()
        rd.step("v2_privacy_set_matches_chain_after_register", pa.get("registered") == str(cr) and pa.get("claimed") == str(cc) and pa.get("set") == str(cr - cc) and str(cr - cc) in (a.get("reg_privacy") or ""),
                "page %s/%s/%s vs chain registered %d claimed %d; '%s'" % (pa.get("registered"), pa.get("claimed"), pa.get("set"), cr, cc, (a.get("reg_privacy") or "")[:120]))
        rd.step("v2_page_loaded_from_chain", a.get("loaded") is True, a.get("error", "") or "manifest v%s, %d ms" % (a.get("version"), a.get("load_ms", 0)))
        rd.step("v2_browse_instances_by_code_id", (a.get("instances") or 0) >= 1 and a.get("selected") == contract, "code %s: %s instance(s)" % (a.get("code_ids"), a.get("instances")))
        rd.step("v2_hal_fetched_and_verified", a.get("hal_state") == "ok" and a.get("hal_sha") == hal["sha256"], a.get("hal_text", "")[:200])
        rd.step("v2_hal_tampered_local_file_refused", a.get("hal_tamper_state") == "error" and a.get("hal_restored") == "ok", a.get("hal_tamper_err", "")[:200])
        rd.step("v2_info_modal_every_step", a.get("modals_ok") == a.get("modals") and (a.get("modals") or 0) >= 10 and a.get("sheet_open_after_modals") is True, "%s/%s" % (a.get("modals_ok"), a.get("modals")))
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
        b = browser("v2b", {"url": url, "contract": contract, "notes": nr, "recv_b": recv_b, "v2lib": os.path.join(PAGE, "v2lib.js"), "out": os.path.join(work, "notes-after-claim.json"), "shots": SHOTS,
                              "zafu_ext": os.environ.get("HSFE_ZAFU_EXT", "/tmp/zafu-ext/unpacked"), "zafu_profile": os.environ.get("HSFE_ZAFU_PROFILE", "/tmp/zafu-e2e/profile-onboarded"),
                              "zafu_pass": os.environ.get("HSFE_ZAFU_PASS", "hs-e2e-pass-1")})
        R["browser_v2b"] = keep(b, "log"); R["browser_v2b_log"] = b.get("log", "")[-8000:]
        ok2, nf2 = be.canon_ok(os.path.join(work, "notes-after-claim.json")) if b.get("saved") else (False, {})
        spent = [n for n in nf2.get("notes", []) if n["st"] == "spent"]
        eph = [n for n in spent if n.get("mode") == "ephemeral"]; fwd = [n for n in spent if n.get("mode") == "forward"]
        rd.step("v2_ephemeral_claim", len(eph) == 1 and be.bal(eph[0]["dest"]) == int(eph[0]["amt"]), "%s holds %s" % (eph[0]["dest"], be.bal(eph[0]["dest"])) if eph else json.dumps(b.get("error")))
        zf = [n for n in fwd if n["fwd"]["receiver"].startswith("penumbra1")]; bf = [n for n in fwd if n["fwd"]["receiver"] == recv_b]
        R["forwards"] = [n["fwd"] for n in fwd]
        rd.step("v2_real_zafu_connected", b.get("real_zafu") and b.get("zafu_state") == "connected" and b.get("zafu_approvals", 0) >= 1, "%s approvals=%s: %s" % (b.get("zafu_state"), b.get("zafu_approvals"), (b.get("zafu_text") or "")[:120]))
        bk = b.get("zafu_book") or []; nz = [e for e in bk if e.get("k", "").split(":")[-1].strip("0")]
        rd.step("v2_zafu_unpacked_non_store_id_connects", b.get("zafu_ext_id_unpacked") is True and b.get("zafu_state") == "connected" and (b.get("zafu_ext_id") or "-") in (b.get("zafu_text") or ""),
                "ext id %s (= unpacked path hash: %s); %s" % (b.get("zafu_ext_id"), b.get("zafu_ext_id_unpacked"), (b.get("zafu_text") or "")[:100]))
        rd.step("v2_zafu_fresh_nonzero_indices", len(bk) == 3 and len(nz) == 3 and len({e["k"] for e in bk}) == 3 and b.get("zafu_addrs_distinct") and zf and zf[0]["fwd"]["receiver"] == bk[0]["a"],
                "%d indices %s; addrs %s" % (len(bk), [e["k"] for e in bk], b.get("zafu_addr_prefixes")))
        rd.step("v2_zafu_disconnect", b.get("zafu_after_disconnect") == "found", str(b.get("zafu_after_disconnect")))
        okz = zf and rf.wait_for(lambda: be.bal(zf[0]["fwd"]["fallback"]) == int(zf[0]["amt"]), 150)
        rd.step("v2_forward_zafu_receiver_refunded_to_fresh_fallback", bool(okz), "receiver %s… (real Zafu AddressByIndex; chain B is terpd so it error-acks) fallback %s bal %s" % (zf[0]["fwd"]["receiver"][:30], zf[0]["fwd"]["fallback"], be.bal(zf[0]["fwd"]["fallback"])) if zf else "no zafu forward")
        okb = bf and rf.wait_for(lambda: sum(v - bb0.get(d, 0) for d, v in rf.b_balances(recv_b).items() if d.startswith("ibc/")) == int(bf[0]["amt"]), 150)
        rd.step("v2_forward_received_on_chain_b", bool(okb), "recv %s balances %s" % (recv_b, rf.b_balances(recv_b)))
        fbs = [n["fwd"]["fallback"] for n in fwd]; dsts = [n["dest"] for n in eph] + fbs + [n["fwd"]["receiver"] for n in fwd]
        rd.step("v2_all_destinations_distinct", len(dsts) == len(set(dsts)) and len(fbs) == 2, json.dumps(dsts)[:300])
        rd.step("v2_receiver_reuse_refused_locally", "already used" in (b.get("reuse_err") or ""), (b.get("reuse_err") or "")[:200])
        g = b.get("gate") or {}
        rd.step("v2_privacy_confirm_gate", g.get("small") == "true" and g.get("claim_disabled") is True and g.get("ack_visible") is True and "privacy set" in (g.get("forced_err") or "") and g.get("forced_posts") == []
                and g.get("log_delta") == 0 and g.get("disabled_after") is True and g.get("enabled_after_ack") is True and g.get("relay_hidden") is True and b.get("claim_locked_before_dest") is True and b.get("claim_relocked_on_dest_change") is True,
                "set %s < %s; forced: '%s'; posts %s; advice '%s'" % (g.get("set"), g.get("threshold"), (g.get("forced_err") or "")[:90], g.get("forced_posts"), (g.get("advice") or "")[:90]))
        pe = b.get("privacy_end") or {}; cr, cc = chain_priv()
        rd.step("v2_privacy_set_matches_chain_after_claims", pe.get("registered") == str(cr) and pe.get("claimed") == str(cc) and pe.get("set") == str(cr - cc) and cc >= 3,
                "page %s/%s/%s vs chain registered %d claimed %d" % (pe.get("registered"), pe.get("claimed"), pe.get("set"), cr, cc))
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
