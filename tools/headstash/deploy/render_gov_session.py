#!/usr/bin/env python3
"""Render the gov-roles mainnet artifacts (UNSIGNED, nothing is signed or broadcast).

usage: render_gov_session.py decisions.mainnet.json --circuit DIR --wasm FILE --fe-wasm FILE --page DIR --prover DIR
       --eph-grantee ADDR [--out DIR] [--proposer-acct N --proposer-seq N --fx-acct N --fx-seq N] [--terpd BIN]

Layout (gov = wasm admin + contract owner, terp1gnfq proposes, terp1fx grants claim fees directly):
  00-render/                    render.py --partial output (message bodies)
  10-session-grant/             Zigner doc: produce with gov-session/ bin gov-session-grant (see checklist)
  11-circuit-proposal.inner.json  MsgSubmitProposal{MsgStoreFullCircuit} proposer=granter, 5,000 TERP initial deposit
  12-circuit-exec.unsigned.json   MsgExec by the ephemeral key (on-device), fee via the session feegrant
  13-store-wasm.unsigned.json     normal MsgStoreCode signed by the eph key (sender=eph, instantiate=gov only), fee via session feegrant
  13b-store-frontend-wasm.unsigned.json  same for cw_headstash_frontend.wasm
  14-instantiate-proposal.unsigned.json  small proposal: pool->gov, Instantiate2 frontend (gov admin+owner), PublishManifest
                                  (activate), Instantiate2 cw-headstash (gov admin, funds, frontend_contract), set campaign, bind circuit
  18-fe-uploader-feegrant.unsigned.json  terp1fx (Zigner): capped, expiring BasicAllowance to a throwaway uploader key
  19-fe-chunks/NN-fF-iI.unsigned.json  UploadChunk txs signed by the uploader key (direct), fee granter terp1fx; >64 KiB, never Zigner
  20-fe-uploader-revoke.unsigned.json  terp1fx (Zigner): MsgRevokeAllowance once all chunks landed
  page-manifest.json               the manifest gov publishes (file + chunk sha256s)
  15-session-revoke.unsigned.json MsgExec revoke (gov_session.revoke_tx)
  16-feegrant-terp1fx.unsigned.json  MsgGrantAllowance template from terp1fx (relay issues one per claimer)
  17-deposit-topup.unsigned.json  MsgDeposit 5,000 TERP from the proposer (x2: one per proposal)
  sizes.json                     every doc's amino sign-doc bytes vs Zigner 65536
Placeholders kept as {{NAME}}: PROPOSAL_METADATA_URI, FORUM_URL,
plus later-step CODE_ID, NOTE_ZK_ID, PROPOSAL_ID_*.
"""
import base64, gzip, hashlib, io, json, os, re, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
import gov_session as gs

GOV = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
INIT_DEPOSIT, FULL_DEPOSIT = 5_000_000_000, 10_000_000_000
ZIGNER_MAX = 65536
SENTINELS = {  # values that pass render.py checks, swapped back to {{NAME}} after rendering
    "PROPOSAL_METADATA_URI": "https://placeholder.invalid/PROPOSAL_METADATA_URI",
    "FORUM_URL": "https://placeholder.invalid/FORUM_URL",
}
ALLOWED_LEFT = {"PROPOSAL_METADATA_URI", "FORUM_URL", "CODE_ID", "NOTE_ZK_ID",
                "PROPOSAL_ID_CIRCUIT", "PROPOSAL_ID_CONTRACT", "CLAIMER_ADDR", "FE_CODE_ID", "FE_STORE_GAS", "FE_STORE_FEE_UTHIOL",
                "FE_UPLOADER_ADDR", "FE_UPLOAD_EXPIRATION", "EXEC_FEE_UTHIOL", "EXEC_GAS", "STORE_GAS", "STORE_FEE_UTHIOL", "CLAIM_FEE_UTHIOL", "SEQ"}


PAGE_FILES = (("page", "app.html"), ("page", "app.js"), ("page", "crypto.js"), ("prover", "zk_headstash.js"), ("prover", "zk_headstash_bg.wasm"))
CT = {".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html"}
UPLOAD_GAS = lambda n: 600000 + 64 * n   # e2e/browser_e2e.py; measured ~45 gas/B (spec §10)
GAS_PRICE = 0.05                          # mainnet global-fee minimum, uthiol/gas


def gz(raw):  # deterministic gzip, identical to e2e/frontend_bench.gz
    bio = io.BytesIO()
    with gzip.GzipFile(fileobj=bio, mode="wb", compresslevel=9, mtime=0, filename="") as f: f.write(raw)
    return bio.getvalue()


def page_manifest(dirs, chunk):
    ents, chunks = [], []
    for d, n in PAGE_FILES:
        raw = open(os.path.join(dirs[d], n), "rb").read(); st = gz(raw)
        cs = [st[i:i + chunk] for i in range(0, len(st), chunk)]; chunks.append(cs)
        ents.append({"path": n, "content_type": CT[os.path.splitext(n)[1]], "encoding": "gzip", "size": len(raw),
                     "sha256": hashlib.sha256(raw).hexdigest(), "stored_size": len(st), "chunks": [hashlib.sha256(c).hexdigest() for c in cs]})
    return {"version": 1, "entry": "app.js", "files": ents, "note": "headstash claim page", "activate": True}, chunks


def unsentinel(text):
    for k in ("PROPOSAL_METADATA_URI", "FORUM_URL"):
        text = text.replace(SENTINELS[k], "{{%s}}" % k)
    return text


def amino_doc(chain_id, acct, seq, tx):
    """Approximate amino-JSON StdSignDoc (Zigner signs this form): sorted, compact."""
    def am(m):
        m = dict(m); t = m.pop("@type"); return {"type": t, "value": m}
    f = tx["auth_info"]["fee"]
    d = {"account_number": str(acct), "chain_id": chain_id, "fee": {"amount": f["amount"], "gas": f["gas_limit"]},
         "memo": tx["body"]["memo"], "msgs": [am(m) for m in tx["body"]["messages"]], "sequence": str(seq)}
    if f.get("granter"): d["fee"]["granter"] = f["granter"]
    return json.dumps(d, sort_keys=True, separators=(",", ":"))


def validate_rendered_proposal(name, obj, proposer):
    """Fail-loud checks on a rendered proposal / tx. Returns list of errors."""
    errs, s = [], unsentinel(json.dumps(obj))
    left = set(re.findall(r"\{\{([A-Z0-9_]+)\}\}", s)) - ALLOWED_LEFT
    if left: errs.append("unexpected placeholders %s" % sorted(left))
    if "placeholder.invalid" in s or "__placeholder__" in s or "987654321987" in s: errs.append("sentinel leaked")
    if "EXAMPLE" in s.upper() and "rendered-example" not in name: errs.append("EXAMPLE value")
    def walk(m):
        if isinstance(m, dict):
            t = m.get("@type", "")
            if t == "/cosmos.gov.v1.MsgSubmitProposal":
                if m.get("proposer") != proposer: errs.append("proposer %s" % m.get("proposer"))
                dep = m.get("initial_deposit", [])
                if dep != [{"denom": "uterp", "amount": str(INIT_DEPOSIT)}]: errs.append("initial deposit %s" % dep)
            if t.startswith("/cosmwasm.wasm.v1.MsgInstantiateContract"):
                if m.get("admin") != GOV or m.get("sender") != GOV: errs.append("instantiate admin/sender not gov")
                if (m.get("msg") or {}).get("owner") not in (None, GOV): errs.append("owner not gov")
                if "token_strategy" in (m.get("msg") or {}) and not (m.get("msg") or {}).get("frontend_contract"): errs.append("cw-headstash instantiate lacks frontend_contract")
            if t == "/cosmwasm.wasm.v1.MsgStoreCode":
                ip = m.get("instantiate_permission") or {}
                if ip.get("addresses") != [GOV]: errs.append("store code instantiate_permission not gov-only")
            if t in ("/cosmos.authz.v1beta1.MsgGrant", "/cosmos.feegrant.v1beta1.MsgGrantAllowance") and "session" in name:
                pass
            for v in m.values(): walk(v)
        elif isinstance(m, list):
            for v in m: walk(v)
    walk(obj)
    return errs


def main():
    a = sys.argv[1:]
    def opt(k, d=None):
        return a[a.index(k) + 1] if k in a else d
    dec_path, circ, wasm, eph = a[0], opt("--circuit"), opt("--wasm"), opt("--eph-grantee")
    fe_wasm, page_dir, prover_dir = opt("--fe-wasm"), opt("--page"), opt("--prover")
    if not (fe_wasm and page_dir and prover_dir): sys.exit("FATAL --fe-wasm, --page and --prover are required")
    out = opt("--out", os.path.join(HERE, "rendered-mainnet"))
    p_acct, p_seq = int(opt("--proposer-acct", 0)), int(opt("--proposer-seq", 116))
    fx_acct, fx_seq = int(opt("--fx-acct", 19)), int(opt("--fx-seq", 256))
    terpd = opt("--terpd", "terpd")
    dec = json.load(open(dec_path))
    proposer, fx, chain = dec["PROPOSER"], dec["OPERATORS"][0]["addr"], dec["CHAIN_ID"]
    os.makedirs(out, exist_ok=True)

    # 1. contract address is predictable: instantiate2(creator=gov, checksum, salt)
    wb = open(wasm, "rb").read(); wsha = hashlib.sha256(wb).hexdigest()
    if dec["WASM_SHA256"] != wsha: sys.exit("FATAL WASM_SHA256 %s != %s" % (dec["WASM_SHA256"], wsha))
    salt = hashlib.sha256(("headstash:" + dec["CAMPAIGN_LABEL"]).encode()).hexdigest()
    contract = subprocess.run([terpd, "q", "wasm", "build-address", wsha, GOV, salt], capture_output=True, text=True, check=True).stdout.strip().split()[-1]
    fb_ = open(fe_wasm, "rb").read(); fsha = hashlib.sha256(fb_).hexdigest()
    if dec["FE_WASM_SHA256"] != fsha: sys.exit("FATAL FE_WASM_SHA256 %s != %s" % (dec["FE_WASM_SHA256"], fsha))
    fe_salt = hashlib.sha256(("headstash-frontend:" + dec["CAMPAIGN_LABEL"]).encode()).hexdigest()
    fe_addr = subprocess.run([terpd, "q", "wasm", "build-address", fsha, GOV, fe_salt], capture_output=True, text=True, check=True).stdout.strip().split()[-1]
    chunk = int(dec.get("FE_MAX_CHUNK_BYTES", 131072))
    if not 0 < chunk <= 131072: sys.exit("FATAL FE_MAX_CHUNK_BYTES over the 128 KiB wasmvm value limit")
    manifest, chunks = page_manifest({"page": page_dir, "prover": prover_dir}, chunk)

    # 2. render.py --partial with sentinels, CONTRACT_ADDR filled
    tmp = dict(dec); tmp["CONTRACT_ADDR"] = contract
    for k, v in SENTINELS.items():
        if str(dec.get(k, "")).startswith("{{"): tmp[k] = v
    tdec = os.path.join(out, ".decisions.sentinel.json"); json.dump(tmp, open(tdec, "w"))
    rdir = os.path.join(out, "00-render")
    r = subprocess.run([sys.executable, os.path.join(HERE, "render.py"), tdec, "--circuit", circ, "--partial", "--out", rdir], capture_output=True, text=True)
    os.remove(tdec)
    if r.returncode: sys.exit("FATAL render.py: " + r.stderr[-800:])
    raw = {}
    for f in os.listdir(rdir):
        p = os.path.join(rdir, f); raw[f] = open(p).read(); open(p, "w").write(unsentinel(raw[f]))
    rd = lambda f: raw[f]  # parse sentinel text; placeholders are restored when files are written
    meta_uri = dec["PROPOSAL_METADATA_URI"] if not str(dec["PROPOSAL_METADATA_URI"]).startswith("{{") else "{{PROPOSAL_METADATA_URI}}"
    sub = json.loads(rd("submit-proposal.json"))
    circuit_msg = [m for m in sub["messages"] if m["@type"] == "/cosmwasm.wasm.v1.MsgStoreFullCircuit"][0]
    funding = dec["FUNDING_AMOUNT_UTERP"]

    def submit(msgs, title, summary):
        return {"@type": "/cosmos.gov.v1.MsgSubmitProposal", "messages": msgs,
                "initial_deposit": [{"denom": "uterp", "amount": str(INIT_DEPOSIT)}], "proposer": proposer,
                "metadata": meta_uri, "title": title, "summary": summary, "expedited": False}

    files = {}
    # 11/12 circuit proposal via the ephemeral session key
    p_circ = submit([circuit_msg], "Headstash 2026 (1/2): store the note circuit",
                    "Stores the Headstash vk_id 3 note circuit (vk hash %s) with gov as sender. Forum: {{FORUM_URL}}" % circuit_msg.get("vk_body", "")[:0] + dec.get("_vk", ""))
    p_circ["summary"] = "Stores the Headstash vk_id 3 note circuit (sha256(params||vk) 34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398) as gov. Forum: {{FORUM_URL}}"
    files["11-circuit-proposal.inner.json"] = (p_circ, None)
    files["12-circuit-exec.unsigned.json"] = (gs.exec_tx(eph, proposer, [p_circ], "{{EXEC_GAS}}", "{{EXEC_FEE_UTHIOL}}", "uthiol"), ("eph", None, None))
    # 13 wasm store (normal tx, sender = granter) via the session: > 64 KiB so Zigner cannot sign it directly
    store = {"@type": "/cosmwasm.wasm.v1.MsgStoreCode", "sender": eph,
             "wasm_byte_code": base64.b64encode(gzip.compress(wb, 9, mtime=0)).decode(),
             "instantiate_permission": {"permission": "AnyOfAddresses", "addresses": [GOV]}}
    files["13-store-wasm.unsigned.json"] = (gs._tx([store], "{{STORE_GAS}}", "{{STORE_FEE_UTHIOL}}", "uthiol", proposer, "headstash-gov-session:store-code"), ("eph", None, None))
    fstore = dict(store, wasm_byte_code=base64.b64encode(gzip.compress(fb_, 9, mtime=0)).decode())
    files["13b-store-frontend-wasm.unsigned.json"] = (gs._tx([fstore], "{{FE_STORE_GAS}}", "{{FE_STORE_FEE_UTHIOL}}", "uthiol", proposer, "headstash-gov-session:store-code"), ("eph", None, None))
    # 14 instantiate proposal (small, proposer signs on Zigner)
    init = json.loads(rd("01-instantiate.json")); init["owner"] = GOV; init["frontend_contract"] = fe_addr
    camp = json.loads(rd("02-set-epoch-campaign-open.json")) if "02-set-epoch-campaign-open.json" in raw else "{{SET_EPOCH_CAMPAIGN}}"
    msgs = [{"@type": "/cosmos.distribution.v1beta1.MsgCommunityPoolSpend", "authority": GOV, "recipient": GOV,
             "amount": [{"denom": "uterp", "amount": funding}]},
            {"@type": "/cosmwasm.wasm.v1.MsgInstantiateContract2", "sender": GOV, "admin": GOV, "code_id": "{{FE_CODE_ID}}",
             "label": dec["CAMPAIGN_LABEL"] + "-frontend", "msg": {"owner": GOV, "max_chunk_bytes": chunk}, "funds": [],
             "salt": base64.b64encode(bytes.fromhex(fe_salt)).decode(), "fix_msg": False},
            {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": fe_addr, "msg": {"publish_manifest": {"manifest": manifest}}, "funds": []},
            {"@type": "/cosmwasm.wasm.v1.MsgInstantiateContract2", "sender": GOV, "admin": GOV, "code_id": "{{CODE_ID}}",
             "label": dec["CAMPAIGN_LABEL"], "msg": init, "funds": [{"denom": "uterp", "amount": funding}],
             "salt": base64.b64encode(bytes.fromhex(salt)).decode(), "fix_msg": False},
            {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": contract, "msg": camp, "funds": []},
            {"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": GOV, "contract": contract,
             "msg": {"bind_note_circuit": {"zk_id": "{{NOTE_ZK_ID}}"}}, "funds": []}]
    p_inst = submit(msgs, "Headstash 2026 (2/2): instantiate and fund cw-headstash",
                    "Instantiates cw-headstash code {{CODE_ID}} (sha256 %s) at %s with gov as admin and owner, funds it with %s uterp from the community pool, opens the campaign and binds note circuit {{NOTE_ZK_ID}}. Also instantiates the claim page host cw-headstash-frontend code {{FE_CODE_ID}} (sha256 %s) at %s (gov admin and owner) and publishes page manifest v1 (sha256 %s, activates when every chunk is uploaded). Forum: {{FORUM_URL}}" % (wsha, contract, funding, fsha, fe_addr, hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()))
    files["14-instantiate-proposal.unsigned.json"] = (gs._tx([p_inst], 3000000, 75000, "uthiol", "", "headstash-gov:instantiate"), ("proposer", p_acct, p_seq + 1))
    files["15-session-revoke.unsigned.json"] = (gs.revoke_tx(eph, proposer, gs.SESSION_TYPES_MAINNET, 600000, 30000, "uthiol", use_feegrant=True), ("eph", None, None))
    files["16-feegrant-terp1fx.unsigned.json"] = (gs._tx([{"@type": "/cosmos.feegrant.v1beta1.MsgGrantAllowance", "granter": fx, "grantee": "{{CLAIMER_ADDR}}",
        "allowance": {"@type": "/cosmos.feegrant.v1beta1.BasicAllowance", "spend_limit": [{"denom": "uthiol", "amount": "{{CLAIM_FEE_UTHIOL}}"}], "expiration": None}}],
        200000, 10000, "uthiol", ""), ("terp1fx", fx_acct, fx_seq))
    files["17-deposit-topup.unsigned.json"] = (gs._tx([{"@type": "/cosmos.gov.v1.MsgDeposit", "proposal_id": "{{PROPOSAL_ID_CIRCUIT}}", "depositor": proposer,
        "amount": [{"denom": "uterp", "amount": str(FULL_DEPOSIT - INIT_DEPOSIT)}]}], 30000000, 750000, "uthiol", ""), ("proposer", p_acct, None))

    # 18-20: chunk uploads (permissionless, content-addressed) by a throwaway key; fees granted by terp1fx
    up_fee = [int(UPLOAD_GAS(len(c)) * GAS_PRICE) + 1 for cs in chunks for c in cs]
    cap = sum(up_fee) + 1_000_000  # +1 THIOL slack for a retry
    files["18-fe-uploader-feegrant.unsigned.json"] = (gs._tx([{"@type": "/cosmos.feegrant.v1beta1.MsgGrantAllowance", "granter": fx, "grantee": "{{FE_UPLOADER_ADDR}}",
        "allowance": {"@type": "/cosmos.feegrant.v1beta1.BasicAllowance", "spend_limit": [{"denom": "uthiol", "amount": str(cap)}], "expiration": "{{FE_UPLOAD_EXPIRATION}}"}}],
        200000, 10000, "uthiol", "", "headstash-frontend:uploader-feegrant"), ("terp1fx", fx_acct, fx_seq))
    k = 0
    for fi, cs in enumerate(chunks):
        for ci, c in enumerate(cs):
            g = UPLOAD_GAS(len(c))
            files["19-fe-chunks/%02d-f%d-i%d.unsigned.json" % (k, fi, ci)] = (gs._tx([{"@type": "/cosmwasm.wasm.v1.MsgExecuteContract", "sender": "{{FE_UPLOADER_ADDR}}", "contract": fe_addr,
                "msg": {"upload_chunk": {"version": 1, "file": fi, "index": ci, "data": base64.b64encode(c).decode()}}, "funds": []}],
                g, int(g * GAS_PRICE) + 1, "uthiol", fx, "headstash-frontend:upload"), ("uploader", None, None)); k += 1
    files["20-fe-uploader-revoke.unsigned.json"] = (gs._tx([{"@type": "/cosmos.feegrant.v1beta1.MsgRevokeAllowance", "granter": fx, "grantee": "{{FE_UPLOADER_ADDR}}"}],
        100000, 5000, "uthiol", "", "headstash-frontend:uploader-revoke"), ("terp1fx", fx_acct, None))
    os.makedirs(os.path.join(out, "19-fe-chunks"), exist_ok=True)
    for f_ in os.listdir(os.path.join(out, "19-fe-chunks")): os.remove(os.path.join(out, "19-fe-chunks", f_))
    open(os.path.join(out, "page-manifest.json"), "w").write(json.dumps(manifest, indent=1) + "\n")

    sizes, errs = {}, {}
    for n, (obj, signer) in files.items():
        if obj is None: continue
        s = unsentinel(json.dumps(obj, indent=1)); open(os.path.join(out, n), "w").write(s + "\n")
        e = validate_rendered_proposal(n, obj, proposer)  # checked on sentinel values (valid JSON)
        if e: errs[n] = e
        ent = {"tx_json_bytes": len(unsentinel(json.dumps(obj, separators=(",", ":")))), "signer": signer[0] if signer else None}
        if signer and signer[0] not in ("eph", "uploader") and "messages" in obj.get("body", {}):
            doc = amino_doc(chain, signer[1], signer[2] if signer[2] is not None else "{{SEQ}}", obj)
            ent.update(amino_sign_doc_bytes=len(doc), zigner_ok=len(doc) <= ZIGNER_MAX)
        elif signer and signer[0] == "eph":
            ent.update(signed_by="on-device ephemeral key (direct mode, not Zigner)")
        elif signer and signer[0] == "uploader":
            ent.update(signed_by="throwaway uploader key (direct mode, not Zigner); fee granter terp1fx", gas=int(obj["auth_info"]["fee"]["gas_limit"]), fee_uthiol=int(obj["auth_info"]["fee"]["amount"][0]["amount"]))
        sizes[n] = ent
    g = os.path.join(out, "10-session-grant", "meta.json")
    if os.path.exists(g):
        m = json.load(open(g)); sizes["10-session-grant/sign_doc.json"] = {"amino_sign_doc_bytes": m["sign_doc_bytes"], "zigner_ok": m["fits_zigner"],
                                                                           "qr_frames": m["qr_frames"], "signer": "proposer (Zigner)", "types": m["type_urls"]}
    rep = {"contract_addr_predicted": contract, "salt_hex": salt, "wasm_sha256": wsha, "wasm_bytes": len(wb),
           "frontend": {"addr_predicted": fe_addr, "salt_hex": fe_salt, "wasm_sha256": fsha, "wasm_bytes": len(fb_), "wasm_gzip_bytes": len(gzip.compress(fb_, 9, mtime=0)),
                        "max_chunk_bytes": chunk, "chunks": sum(map(len, chunks)), "upload_gas_total": sum(UPLOAD_GAS(len(c)) for cs in chunks for c in cs),
                        "upload_fee_uthiol_total": sum(up_fee), "uploader_allowance_uthiol": cap,
                        "manifest_sha256": hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                        "files": [{"path": e["path"], "raw": e["size"], "gzip": e["stored_size"], "sha256": e["sha256"], "chunks": len(e["chunks"])} for e in manifest["files"]]},
           "wasm_gzip_bytes": len(gzip.compress(wb, 9, mtime=0)), "files": sizes, "validate_rendered_proposal": errs or "ok",
           "placeholders_left": sorted({k for n in files if files[n][0] is not None for k in re.findall(r"\{\{([A-Z0-9_]+)\}\}", open(os.path.join(out, n)).read())})}
    json.dump(rep, open(os.path.join(out, "sizes.json"), "w"), indent=2)
    print(json.dumps(rep, indent=2))
    if errs: sys.exit(1)


if __name__ == "__main__":
    main()
