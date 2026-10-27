#!/usr/bin/env python3
"""Headstash v1 direct on-chain claim e2e (vk_id 3 notes, loopback only).

Store the note circuit (store-full-circuit), instantiate cw-headstash (zk-api build),
BindNoteCircuit, chunked RegisterNotes from fee-granted fresh wallets, accept anchors,
then ClaimNote from one fresh fee-granted wallet per note, with exact balance checks
and the negatives from the launch brief. Fee grants come from headstash-register-relay
on 127.0.0.1:18795 (granter = keyring name "relayer"). Test secrets stay in memory.
"""
import base64, hashlib, json, os, secrets, subprocess, sys, time
import requests
import run as b
from run import (R, log, step, save, b64, h2b64, pmul, kec, eip191, evm_sign, b58, pallas_rand, terpd, q, height,
                 wait_tx, tx, attr, smart, bal, rec, list_hash, gas_used, N, HOME, CHAIN_ID, NODE, RPC, REST, RUN,
                 RELAY_BIND, RELAY_BIN, ELIG_BIN, EX, WAVS_BIN, DENOM, NOTE_VK_HASH, DENOMS, NOTE_EX, CHUNK, TERPD)
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

WASM = os.environ.get("WASM", "/tmp/hsdirect/opt-out/cw_headstash.wasm")
CUT_AHEAD = int(os.environ.get("CUT_AHEAD", "600"))
SWEEP = os.environ.get("SWEEP", "1") == "1"
RELAY_URL = "http://" + RELAY_BIND
CLAIM_GAS = int(os.environ.get("HEADSTASH_CLAIM_GAS", "400000"))
REGISTER_GAS = int(os.environ.get("HEADSTASH_REGISTER_GAS", "12000000"))
GP = 0.025
CTX_TAG = b"terp-hs-direct-ctx-v1"

def direct_ctx(chain_id, contract):
    h = bytearray(hashlib.sha256(CTX_TAG + b"\0" + chain_id.encode() + b"\0" + contract.encode()).digest()); h[31] &= 0x3F
    return bytes(h)

def txg(frm, gas, fee, granter, *args):
    """Fee-granted tx: fee == the allowance spend limit, so one use exhausts it."""
    a = ["tx", *args, "--from", frm, "--home", HOME, "--keyring-backend", "test", "--chain-id", CHAIN_ID, "--node", NODE,
         "--gas", str(gas), "--fees", "%duterp" % fee, "-b", "sync", "-y", "-o", "json"]
    if granter: a += ["--fee-granter", granter]
    p = subprocess.run([TERPD, *a], capture_output=True, text=True)
    try: res = json.loads(p.stdout)
    except Exception: return {"code": -1, "raw_log": (p.stdout + p.stderr)[-600:]}
    if res.get("code", 0) != 0: return res
    return wait_tx(res["txhash"])

def neg(name, res, *needles):
    log_ = res.get("raw_log", "") if isinstance(res, dict) else str(res)
    R["negatives"][name] = "code %s %s" % (res.get("code") if isinstance(res, dict) else "-", log_[:260])
    ok = (res.get("code") != 0 if isinstance(res, dict) else True) and all(n in log_ for n in needles)
    step("neg_" + name, ok, R["negatives"][name])

def grant(grantee, kind, body):
    r = requests.post(RELAY_URL + "/headstash/feegrant", json={"grantee": grantee, kind: body}, timeout=120)
    try: j = r.json()
    except Exception: j = {"error": r.text[:200]}
    return r.status_code, j

def new_wallet(name):
    terpd("keys", "add", name, "--keyring-backend", "test", "--home", HOME, "--output", "json")  # mnemonic discarded
    return terpd("keys", "show", name, "-a", "--keyring-backend", "test", "--home", HOME).strip()

def main():
    addrs = dict(l.strip().split("=", 1) for l in open(os.path.join(HOME, "addresses.env")) if "=" in l)
    OWNER = addrs["OWNER_ADDR"]
    RELAYER = terpd("keys", "show", "relayer", "-a", "--keyring-backend", "test", "--home", HOME).strip()
    R["mode"] = "direct"
    R["chain"] = {"chain_id": CHAIN_ID, "rpc": RPC, "rest": REST, "relay": RELAY_URL, "start_height": height()}
    R["addresses"] = dict(addrs, RELAYER_ADDR=RELAYER)

    # 1. identities + note eligibility (EVM 200008 TERP -> 43 notes, Sol 3.5 TERP -> 2 notes)
    evm_priv = secrets.randbelow(N - 1) + 1
    pt = pmul(evm_priv)
    evm_addr = kec(pt[0].to_bytes(32, "big") + pt[1].to_bytes(32, "big"))[12:]
    sol_sk = Ed25519PrivateKey.generate()
    sol_pk = sol_sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    ids = {"evm": (0x01, evm_addr, evm_addr.hex()), "sol": (0x02, sol_pk, "sol:" + sol_pk.hex())}
    comm = os.path.join(RUN, "communities", "mvk"); os.makedirs(comm, exist_ok=True)
    open(os.path.join(comm, "mvk.csv"), "w").write("addr,amount\n0x%s,200008\n%s,3.5\n" % (evm_addr.hex(), b58(sol_pk)))
    elig = os.path.join(RUN, "eligibility")
    subprocess.run([ELIG_BIN, os.path.join(RUN, "communities"), elig, "--decimals", "mvk=6", "--denominations"], check=True, capture_output=True)
    ntree = json.load(open(os.path.join(elig, "notes", "tree.json")))
    paths = [json.loads(l) for l in open(os.path.join(elig, "notes", "note_paths.jsonl"))]
    notes = {k: sorted([p for p in paths if p["identity"] == ids[k][2]], key=lambda p: p["note_index"]) for k in ids}
    step("note_eligibility_tree", ntree["depth"] == 17 and {k: len(v) for k, v in notes.items()} == {"evm": 43, "sol": 2},
         "root %s notes %s" % (ntree["root_hex"], ntree["notes"]))

    # 2. circuit store (vk_id 3 blob split params || vk_body)
    cdir = os.path.join(RUN, "circuit"); os.makedirs(cdir, exist_ok=True)
    subprocess.run([NOTE_EX + "/bench_note_proofs", cdir, "0"], check=True, capture_output=True)
    blob = open(os.path.join(cdir, "blob.bin"), "rb").read()
    step("circuit_blob_hash", hashlib.sha256(blob).hexdigest() == NOTE_VK_HASH, "blob %d B sha %s" % (len(blob), hashlib.sha256(blob).hexdigest()))
    res = rec("store_circuit", tx("owner", 60000000, "wasm", "store-full-circuit", os.path.join(cdir, "params.bin"),
              os.path.join(cdir, "vk_body.bin"), "--k", "11", "--circuit-type", "0", "--curve-type", "0"))
    zk_id = None
    for ev in res.get("events", []):
        for at in ev.get("attributes", []):
            if at["key"] in ("zk_id", "circuit_id", "zkid", "id") and at["value"].isdigit() and zk_id is None and "circuit" in ev["type"]:
                zk_id = int(at["value"])
    R["circuit_store_events"] = [ev for ev in res.get("events", []) if "circuit" in ev.get("type", "")]
    zk_id = zk_id or int(os.environ.get("ZK_ID", "1"))
    R["zk_id"] = zk_id; R["gas"] = {"store_circuit": gas_used(res)}
    step("store_circuit", res.get("code") == 0, "zk_id %s gas %d %s" % (zk_id, gas_used(res), res.get("raw_log", "")[:160]))

    # 3. store + instantiate the zk-api contract
    res = rec("store", tx("owner", 250000000, "wasm", "store", WASM))
    code_id = attr(res, "code_id"); R["code_id"] = code_id
    R["wasm_sha256"] = hashlib.sha256(open(WASM, "rb").read()).hexdigest()
    step("store_wasm", res.get("code") == 0 and code_id, "code_id %s wasm %s gas %d" % (code_id, R["wasm_sha256"], gas_used(res)))
    wavs = json.loads(subprocess.run([WAVS_BIN], capture_output=True, text=True, check=True).stdout)
    init = {"genesis_root": b64(bytes(32)), "genesis_label": "headstash-direct-e2e",
            "token_strategy": {"existing_fungible": {"proof": h2b64(DENOM), "raw": "uterp"}}, "wavs": wavs}
    res = rec("instantiate", tx("owner", 80000000, "wasm", "instantiate", code_id, json.dumps(init),
              "--label", "headstash-direct-e2e", "--admin", OWNER, amount="10030000000uterp"))  # two 5000 TERP note payouts (evm_0 + negatives note) + small notes
    contract = attr(res, "_contract_address"); R["contract"] = contract
    step("instantiate", res.get("code") == 0 and contract, contract or res.get("raw_log", ""))
    b.R = R

    ctx = direct_ctx(CHAIN_ID, contract); R["ctx_hex"] = ctx.hex()
    info = smart(contract, {"direct_claim": {}})
    step("direct_ctx_matches_contract", base64.b64decode(info["expected_ctx"]) == ctx, json.dumps(info)[:200])
    cut = height() + CUT_AHEAD; R["cutoff_height"] = cut
    classes = {}
    for k in ids:
        for p in notes[k]: classes[p["class"]] = classes.get(p["class"], 0) + 1
    amounts = [{"amount": DENOMS[c], "cap": n} for c, n in sorted(classes.items())]
    def campaign(c=ctx):
        return {"set_epoch_campaign": {"campaign": {
            "ctx": b64(c), "backend": "halo2", "cutoff_height": cut, "bech32_hrp": "terp", "denom": h2b64(DENOM),
            "amounts": amounts, "reserved": [], "trusted": [], "trusted_verify_threshold": 0, "trusted_sign_threshold": 0,
            "max_rows": 8, "allocation_root": b64(bytes(32)), "root_id": 0, "vk_id": 3, "vk_hash": h2b64(NOTE_VK_HASH),
            "sweep_recipient": OWNER,
            "registration": {"eligibility_root": h2b64(ntree["root_hex"]), "eligibility_depth": 17,
                             "min_anonymity_set": 2, "anchor_start_height": 1, "anchor_interval": 1}}}}
    res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(campaign())))
    if res.get("code") != 0:  # trusted set may be mandatory; fall back to a 1-of-1 placeholder (unused by the direct path)
        c = campaign(); cc = c["set_epoch_campaign"]["campaign"]
        cc["trusted"] = [b64(bytes(32))]; cc["trusted_verify_threshold"] = 1; cc["trusted_sign_threshold"] = 1
        R["set_campaign_first_error"] = res.get("raw_log", "")[:300]
        res = rec("set_campaign", tx("owner", 5000000, "wasm", "execute", contract, json.dumps(c)))
    step("set_campaign_vk3_direct_ctx", res.get("code") == 0, res.get("raw_log", "")[:200])

    # 4. BindNoteCircuit: negatives then the real bind
    res = tx("stranger", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
    neg("bind_by_non_owner", res)
    res = tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id + 99}}))
    neg("wrong_zk_id_unknown_circuit", res)
    res = rec("bind_circuit", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}})))
    R["gas"]["bind_circuit"] = gas_used(res)
    step("bind_note_circuit_hash_matches", res.get("code") == 0,
         "sha256(Circuit.data) == vk_hash on chain: %s; %s" % (res.get("code") == 0, res.get("raw_log", "")[:240]))
    if res.get("code") != 0: return
    res = tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"bind_note_circuit": {"zk_id": zk_id}}))
    neg("rebind_zk_id_swap", res, "circuit is fixed")

    # 5. relay (fee grants) on 127.0.0.1:18795
    env = dict(os.environ, HEADSTASH_RELAY_HOME=HOME, HEADSTASH_RELAY_FROM="relayer", HEADSTASH_KEYRING_BACKEND="test",
               HEADSTASH_REST=REST, HEADSTASH_CONTRACT=contract, HEADSTASH_CHAIN_ID=CHAIN_ID, HEADSTASH_RELAY_BIND=RELAY_BIND,
               HEADSTASH_GAS_PRICE="0.025uterp", HEADSTASH_CLAIM_GAS=str(CLAIM_GAS), HEADSTASH_REGISTER_GAS=str(REGISTER_GAS),
               TERPD=TERPD, PATH="/usr/local/bin:" + os.environ.get("PATH", ""))
    env.pop("HEADSTASH_ELIGIBILITY", None)
    rlog = open(os.path.join(RUN, "relay.log"), "w")
    relay = subprocess.Popen([RELAY_BIN], env=env, stdout=rlog, stderr=subprocess.STDOUT)
    open(os.path.join(RUN, "relay.pid"), "w").write(str(relay.pid))
    try:
        up = False
        for _ in range(60):
            time.sleep(1)
            if "relay_up" in open(os.path.join(RUN, "relay.log")).read(): up = True; break
            if relay.poll() is not None: break
        step("relay_up", up, open(os.path.join(RUN, "relay.log")).read()[-300:])
        if not up: return
        flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut)
    finally:
        relay.terminate()
        try: relay.wait(10)
        except Exception: relay.kill()

def flow(contract, ctx, ids, notes, evm_priv, sol_sk, sol_pk, evm_addr, RELAYER, OWNER, cut):
    claim_spend = int(CLAIM_GAS * GP + 0.999999); reg_spend = int(REGISTER_GAS * GP + 0.999999)
    R["spend"] = {"claim": claim_spend, "register": reg_spend, "claim_gas_limit": CLAIM_GAS, "register_gas_limit": REGISTER_GAS}
    sec = {k: [pallas_rand() for _ in notes[k]] for k in ids}
    kcs = {k: [subprocess.run([EX + "/membership_key_com"], input=s.hex(), capture_output=True, text=True, check=True).stdout.strip()
               for s in sec[k]] for k in ids}
    lists = {k: [(p["class"], kcs[k][i]) for i, p in enumerate(notes[k])] for k in ids}
    def wire(lst): return [{"class": c, "key_com": h2b64(kc)} for c, kc in lst]
    def signer(k): return {"evm": {"address": b64(evm_addr)}} if k == "evm" else {"sol": {"pubkey": b64(sol_pk)}}
    def sign(k, lst):
        lh = list_hash(ctx, ids[k][0], ids[k][1], lst).hex()
        if k == "evm":
            return evm_sign(evm_priv, eip191("headstash-register-notes-v2:%s:%s:%d" % (ctx.hex(), lh, len(lst))))
        return sol_sk.sign(("headstash-register-notes-sol-v2:%s:%s:%d" % (ctx.hex(), lh, len(lst))).encode())
    def reg_body(k, lst, start, sig, rows):
        return {"signer": signer(k), "signature": b64(sig) if sig else None, "list": wire(lst), "start": start,
                "eligibility": [{"index": r["leaf_index"], "path": [h2b64(x) for x in r["path"]]} for r in rows]}

    def granted_register(label, k, lst, start, sig, rows):
        w = new_wallet("dreg_%s" % label)
        body = reg_body(k, lst, start, sig, rows)
        code, j = grant(w, "register", body)
        step("grant_register_%s" % label, code == 200 and j.get("granted"), json.dumps(j)[:240])
        if code != 200: return None
        res = rec("register_%s" % label, txg("dreg_%s" % label, REGISTER_GAS, reg_spend, RELAYER, "wasm", "execute", contract,
                                           json.dumps({"register_notes": body})))
        R["gas"]["register_%s" % label] = {"notes": len(rows), "gas_used": gas_used(res)}
        step("register_notes_%s" % label, res.get("code") == 0, "%d notes gas %d %s" % (len(rows), gas_used(res), res.get("raw_log", "")[:160]))
        step("register_wallet_unfunded_%s" % label, bal(w) == 0, "fee paid by granter; wallet balance %d" % bal(w))
        return res

    # NEG: grant refused for a bad registration signature
    w = new_wallet("dreg_bad")
    badsig = bytearray(sign("evm", lists["evm"])); badsig[5] ^= 1
    code, j = grant(w, "register", reg_body("evm", lists["evm"], 0, bytes(badsig), notes["evm"][:CHUNK]))
    R["negatives"]["grant_refused_bad_register_sig"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_bad_register_sig", code != 200 and not j.get("granted"), R["negatives"]["grant_refused_bad_register_sig"])

    # 6. EVM registration in two chunks (32 + 11), each from a fresh fee-granted wallet
    sig = sign("evm", lists["evm"])
    if not granted_register("evm0", "evm", lists["evm"], 0, sig, notes["evm"][:CHUNK]): return
    if not granted_register("evm1", "evm", lists["evm"], CHUNK, None, notes["evm"][CHUNK:]): return

    # 7. anchor A1 (43 EVM notes)
    def anchor_now(label, want):
        rt = smart(contract, {"registration_tree": {}})
        while height() < int(rt.get("next_anchor_height", 0)): time.sleep(1)
        res = rec("accept_anchor_" + label, tx("stranger", 5000000, "wasm", "execute", contract, json.dumps({"accept_anchor": {}})))
        rt = smart(contract, {"registration_tree": {}})
        a = rt["anchors"][-1]
        step("accept_anchor_" + label, res.get("code") == 0 and a["leaf_count"] == want,
             "root %s leaf_count %s height %s gas %d" % (base64.b64decode(a["root"]).hex(), a["leaf_count"], a.get("height"), gas_used(res)))
        R["gas"]["accept_anchor_" + label] = gas_used(res)
        return base64.b64decode(a["root"]), rt
    A1, _ = anchor_now("A1", 43)

    # 8. Sol notes registered after A1
    if not granted_register("sol", "sol", lists["sol"], 0, sign("sol", lists["sol"]), notes["sol"]): return
    rt = smart(contract, {"registration_tree": {}})
    live = base64.b64decode(rt["root"])

    def witness(k, i, dest, root, amount=None):
        pth = smart(contract, {"registration_path": {"key_com": h2b64(kcs[k][i]), "anchor": b64(root)}})
        return json.dumps({"secret_hex": sec[k][i].hex(), "ctx_hex": ctx.hex(), "root_hex": root.hex(), "destination": dest,
                           "amount": str(amount if amount is not None else pth["amount"]), "index": pth["index"],
                           "siblings_hex": pth["siblings"]}), pth
    def prove(k, i, dest, root, amount=None):
        wit, pth = witness(k, i, dest, root, amount)
        t0 = time.time()
        out = subprocess.run([NOTE_EX + "/prove_note_witness", "/dev/stdin"], input=wit, capture_output=True, text=True)
        if out.returncode != 0: return None, out.stderr.strip()[-200:]
        pr = json.loads(out.stdout); p = pr["body"].split(":")  # headstash-note-claim-v2:ctx:root:nullifier:dest:amount
        pr.update({"root": b64(root), "nullifier": h2b64(p[3]), "amount": int(p[5]), "dest": p[4],
                   "proof": h2b64(pr["proof_hex"]), "secs": round(time.time() - t0, 1)})
        return pr, ""
    def claim_msg(pr, **o):
        m = {"root": pr["root"], "nullifier": pr["nullifier"], "amount": pr["amount"], "proof": pr["proof"]}; m.update(o)
        return {"claim_note": m}

    # NEG: a note registered after A1 has no path in A1 (cannot claim against A1)
    try:
        smart(contract, {"registration_path": {"key_com": h2b64(kcs["sol"][0]), "anchor": b64(A1)}}); r_ = "path returned"
    except Exception as e: r_ = str(e)[-200:]
    R["negatives"]["late_note_not_in_older_anchor"] = r_
    step("neg_late_note_not_in_older_anchor", r_ != "path returned", r_)

    # NEG: live root (not an accepted anchor) -> grant refused and tx rejected. The contract serves
    # paths only under accepted anchors, so the live-root claim carries a real proof (evm_1 under A1)
    # with root = live; the anchor check runs before verify. After A2 snapshots the same root, a
    # proof under it is accepted (live_root_becomes_claimable_once_anchored).
    wl = new_wallet("dclaim_live")
    prl, err = prove("evm", 1, wl, A1)
    if prl:
        code, j = grant(wl, "claim", claim_msg(prl, root=b64(live))["claim_note"])
        res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prl, root=b64(live))))
        R["negatives"]["live_root_grant"] = "%d %s" % (code, json.dumps(j)[:160])
        neg("claim_against_live_root", res, "not an accepted anchor")
        step("neg_grant_refused_live_root", code != 200, R["negatives"]["live_root_grant"])
    else:
        step("prove_live_root", False, err)
    # NEG: fabricated root
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prl, root=b64(pallas_rand()))))
    neg("claim_against_fabricated_root", res, "not an accepted anchor")

    A2, rt2 = anchor_now("A2", 45)
    R["anchors"] = [{"root_hex": base64.b64decode(a["root"]).hex(), "leaf_count": a["leaf_count"], "height": a.get("height")} for a in rt2["anchors"]]
    step("anchor_growth_item_vec", len(rt2["anchors"]) >= 2, "ANCHORS entries %d (Item<Vec>, flagged)" % len(rt2["anchors"]))

    # NEG: a note registered after A1 cannot claim against A1 (forced root swap on a valid A2 proof)
    ws = new_wallet("dclaim_late")
    prs, err = prove("sol", 0, ws, A2)
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prs, root=b64(A1))))
    neg("late_note_claim_against_older_anchor", res, "invalid note proof")

    # 9. positive claims: one fresh fee-granted wallet per note
    step("live_root_becomes_claimable_once_anchored", A2 == live, "live root at rejection %s == A2 %s" % (live.hex()[:16], A2.hex()[:16]))
    picks = [("evm", 0, A2), ("evm", 41, A1), ("evm", 42, A2), ("sol", 1, A2)]
    R["claims"] = []
    granter0 = bal(RELAYER)
    for j_, (k, i, root) in enumerate(picks):
        name = "dclaim%d" % j_
        wlt = new_wallet(name)
        pr, err = prove(k, i, wlt, root)
        if not pr: step("prove_%s_%d" % (k, i), False, err); return
        assert pr["dest"] == wlt
        c0, g0 = bal(contract), bal(RELAYER)
        code, j = grant(wlt, "claim", claim_msg(pr)["claim_note"])
        step("grant_claim_%s_%d" % (k, i), code == 200 and j.get("granted"), json.dumps(j)[:200])
        if code != 200: return
        g1 = bal(RELAYER)
        res = rec("claim_%s_%d" % (k, i), txg(name, CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(pr))))
        c1, g2, w1 = bal(contract), bal(RELAYER), bal(wlt)
        ok = res.get("code") == 0 and w1 == pr["amount"] and c0 - c1 == pr["amount"] and g1 - g2 == claim_spend
        R["claims"].append({"note": "%s_%d" % (k, i), "anchor": "A1" if root == A1 else "A2", "wallet": wlt, "amount": pr["amount"],
                            "gas_used": gas_used(res), "gas_wanted": int(res.get("gas_wanted", 0) or 0), "fee": claim_spend,
                            "grant_tx_fee": g0 - g1, "prove_secs": pr["secs"], "proof_bytes": pr["proof_bytes"]})
        step("direct_claim_%s_%d%s" % (k, i, "_older_anchor_A1" if root == A1 else ""), ok,
             "paid %d to fresh wallet (bal %d) contract -%d granter -%d (fee) gas %d" % (pr["amount"], w1, c0 - c1, g1 - g2, gas_used(res)))
        if j_ == 0: first = (name, wlt, pr, k, i)
    gs = [c["gas_used"] for c in R["claims"]]
    R["gas"]["claim"] = {"min": min(gs), "max": max(gs), "mean": sum(gs) // len(gs)}
    R["gas"]["grant_tx_fee"] = [c["grant_tx_fee"] for c in R["claims"]]
    st = smart(contract, {"direct_claim": {}})
    step("direct_claim_counter", st.get("claimed") == len(picks), json.dumps(st)[:200])

    # 10. negatives against real proofs
    name0, w0, pr0, k0, i0 = first
    # allowance is pruned after its one fee
    res = txg(name0, CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps({"accept_anchor": {}}))
    neg("allowance_single_use", res)
    # replayed nullifier: fresh proof of an already claimed note to a new wallet
    wr = new_wallet("dclaim_replay")
    prr, err = prove(k0, i0, wr, A2)
    code, j = grant(wr, "claim", claim_msg(prr)["claim_note"])
    R["negatives"]["grant_refused_replayed_nullifier"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_replayed_nullifier", code != 200, R["negatives"]["grant_refused_replayed_nullifier"])
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prr)))
    neg("replayed_nullifier", res, "already")
    # fresh unclaimed note for the remaining negatives
    wn = new_wallet("dclaim_neg")
    paid_n = {}
    for c in R["claims"]: paid_n[int(c["amount"])] = paid_n.get(int(c["amount"]), 0) + 1
    camp_amounts = smart(contract, {"epoch_campaign": {}})["amounts"]
    room = lambda amt: next(int(a["cap"]) for a in camp_amounts if int(a["amount"]) == int(amt)) - paid_n.get(int(amt), 0)
    used = {(c["note"].split("_")[0], int(c["note"].split("_")[1])) for c in R["claims"]}
    ni = next(i for i in range(len(notes["evm"])) if ("evm", i) not in used and i != 1 and room(DENOMS[notes["evm"][i]["class"]]) >= 1)
    R["neg_note"] = "evm_%d" % ni
    prn, err = prove("evm", ni, wn, A2)
    forged = bytearray(base64.b64decode(prn["proof"])); forged[100] ^= 1
    code, j = grant(wn, "claim", claim_msg(prn, proof=b64(bytes(forged)))["claim_note"])
    R["negatives"]["grant_refused_forged_proof"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_refused_invalid_proof", code != 200 and not j.get("granted"), R["negatives"]["grant_refused_forged_proof"])
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, proof=b64(bytes(forged)))))
    neg("forged_proof", res, "invalid note proof")
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, root=b64(A1))))
    neg("wrong_root_other_anchor", res, "invalid note proof")
    other = [a["amount"] for a in json.loads(json.dumps(smart(contract, {"epoch_campaign": {}})["amounts"])) if a["amount"] != prn["amount"] and room(a["amount"]) >= 1][0]
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, amount=other)))
    neg("wrong_amount_same_set_swap", res, "invalid note proof")
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn, amount=prn["amount"] + 1)))
    neg("wrong_amount_not_denomination", res)
    res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prn)))
    neg("wrong_sender_dest", res, "invalid note proof")
    # grant scope: a granted wallet cannot spend the allowance on another message type
    code, j = grant(wn, "claim", claim_msg(prn)["claim_note"])
    step("grant_claim_neg_wallet", code == 200 and j.get("granted"), json.dumps(j)[:200])
    res = txg("dclaim_neg", 200000, claim_spend, RELAYER, "bank", "send", wn, OWNER, "1uterp")
    neg("allowance_rejects_bank_send", res)
    res = txg("dclaim_neg", CLAIM_GAS, claim_spend + 1, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(prn)))
    neg("allowance_rejects_fee_above_spend_limit", res)
    code, j = grant(wn, "claim", claim_msg(prn)["claim_note"])
    R["negatives"]["grant_second_for_same_nullifier"] = "%d %s" % (code, json.dumps(j)[:200])
    step("neg_grant_dedupe_same_nullifier", code != 200, R["negatives"]["grant_second_for_same_nullifier"])
    c0 = bal(contract)
    res = rec("claim_neg_wallet_ok", txg("dclaim_neg", CLAIM_GAS, claim_spend, RELAYER, "wasm", "execute", contract, json.dumps(claim_msg(prn))))
    step("direct_claim_after_negatives", res.get("code") == 0 and bal(wn) == prn["amount"] and c0 - bal(contract) == prn["amount"],
         "gas %d" % gas_used(res))
    R["claims"].append({"note": R["neg_note"], "anchor": "A2", "wallet": wn, "amount": prn["amount"], "gas_used": gas_used(res), "fee": claim_spend})
    save()

    # 11. window close + sweep
    if SWEEP:
        log("waiting for cutoff", cut, "now", height())
        while height() <= cut + 1: time.sleep(2)
        wc = new_wallet("dclaim_late2")
        prc, err = prove("sol", 0, wc, A2)
        code, j = grant(wc, "claim", claim_msg(prc)["claim_note"])
        step("neg_grant_refused_after_cutoff", code != 200 and "window" in json.dumps(j) or code != 200, "%d %s" % (code, json.dumps(j)[:160]))
        res = tx("stranger", CLAIM_GAS, "wasm", "execute", contract, json.dumps(claim_msg(prc)))
        neg("claim_after_cutoff", res, "claim window closed")
        c0, o0 = bal(contract), bal(OWNER)
        res = rec("sweep", tx("owner", 3000000, "wasm", "execute", contract, json.dumps({"sweep_unclaimed": {}})))
        fee = int(3000000 * GP + 0.999999)
        step("sweep_unclaimed", res.get("code") == 0 and bal(contract) == 0 and bal(OWNER) - o0 == c0 - fee,
             "swept %d contract now %d %s" % (c0, bal(contract), res.get("raw_log", "")[:160]))

if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        import traceback; traceback.print_exc(); step("harness_exception", False, repr(e)[:300])
    finally:
        save()
        p = sum(s["pass"] for s in R["steps"]); f = len(R["steps"]) - p
        log("E2E_RESULT passed=%d failed=%d report=%s" % (p, f, os.path.join(RUN, "report.json")))
        sys.exit(1 if f else 0)
